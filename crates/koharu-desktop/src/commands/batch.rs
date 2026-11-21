use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context as _, Result, ensure};
use koharu_pipeline::{Event, Outcome, Request, RunId, Stage, Target};
use koharu_scene::PageId;
use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{State, ipc::Channel};
use tokio::sync::Notify;

use super::{
    Error,
    batch_archive::Chapter,
    batch_queue::{BatchEntry, Queue},
};
use crate::app::App;

#[derive(Deserialize, Type)]
pub struct BatchRequest {
    pub id: String,
    pub chapters: Vec<BatchEntry>,
    pub jpeg_quality: u8,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct BatchFailure {
    pub chapter: PathBuf,
    pub error: String,
}

#[derive(Clone, Debug, Default, Serialize, Type)]
pub struct BatchReport {
    #[specta(type = f64)]
    pub completed: usize,
    #[specta(type = f64)]
    pub skipped: usize,
    pub failures: Vec<BatchFailure>,
    pub stopped: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum BatchStage {
    Detection,
    Ocr,
    Inpainting,
    Translation,
}

impl From<Stage> for BatchStage {
    fn from(value: Stage) -> Self {
        match value {
            Stage::Detect => Self::Detection,
            Stage::Recognize => Self::Ocr,
            Stage::Inpaint => Self::Inpainting,
            Stage::Translate => Self::Translation,
        }
    }
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum BatchEvent {
    Started {
        #[specta(type = f64)]
        total_chapters: usize,
    },
    QueueChanged {
        paths: Vec<PathBuf>,
    },
    ChapterQueued {
        path: PathBuf,
    },
    ChapterStarted {
        path: PathBuf,
        name: String,
        #[specta(type = f64)]
        pages: usize,
    },
    ChapterProgress {
        path: PathBuf,
        #[specta(type = f64)]
        completed_pages: usize,
        #[specta(type = f64)]
        active_pages: usize,
        #[specta(type = f64)]
        total_pages: usize,
        #[specta(type = f64)]
        completed_steps: usize,
        #[specta(type = f64)]
        total_steps: usize,
        stage: BatchStage,
    },
    ChapterExporting {
        path: PathBuf,
    },
    ChapterRendered {
        path: PathBuf,
        #[specta(type = f64)]
        completed_pages: usize,
        #[specta(type = f64)]
        total_pages: usize,
    },
    ChapterFinished {
        path: PathBuf,
        output: PathBuf,
    },
    ChapterSkipped {
        path: PathBuf,
        output: PathBuf,
    },
    ChapterFailed {
        path: PathBuf,
        name: String,
        error: String,
    },
    Finished {
        report: BatchReport,
    },
}

#[derive(Default)]
pub struct BatchState {
    active: Mutex<Option<Arc<BatchRun>>>,
}

#[derive(Default)]
struct Control {
    paused: bool,
    run: Option<RunId>,
}

struct BatchRun {
    id: String,
    queue: Mutex<Queue>,
    control: Mutex<Control>,
    stopped: Arc<AtomicBool>,
    wake: Notify,
    export_wake: Condvar,
    events: Channel<BatchEvent>,
}

impl BatchRun {
    fn tell(&self, event: BatchEvent) {
        let _ = self.events.send(event);
    }
    async fn ready(&self) -> bool {
        loop {
            let notified = self.wake.notified();
            if self.stopped.load(Ordering::Acquire) {
                return false;
            }
            if !self.control.lock().paused {
                return true;
            }
            notified.await;
        }
    }
    fn ready_export(&self) -> Result<()> {
        let mut control = self.control.lock();
        while control.paused && !self.stopped.load(Ordering::Acquire) {
            self.export_wake.wait(&mut control);
        }
        ensure!(!self.stopped.load(Ordering::Acquire), "batch stopped");
        Ok(())
    }

    fn publish(
        &self,
        temporary: tempfile::NamedTempFile,
        destination: &std::path::Path,
        overwrite: bool,
    ) -> Result<()> {
        let mut control = self.control.lock();
        while control.paused && !self.stopped.load(Ordering::Acquire) {
            self.export_wake.wait(&mut control);
        }
        ensure!(!self.stopped.load(Ordering::Acquire), "batch stopped");
        if overwrite {
            temporary.persist(destination)?;
        } else {
            temporary.persist_noclobber(destination)?;
        }
        Ok(())
    }
}

struct Guard<'a>(&'a BatchState);
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.active.lock().take();
    }
}

fn current(state: &BatchState, id: &str) -> Result<Arc<BatchRun>> {
    let run = state.active.lock().clone().context("no batch is running")?;
    ensure!(run.id == id, "the batch has changed");
    ensure!(
        !run.stopped.load(Ordering::Acquire),
        "the batch is stopping"
    );
    Ok(run)
}

#[tauri::command]
#[specta::specta]
pub async fn process_batch(
    app: State<'_, Arc<App>>,
    state: State<'_, BatchState>,
    request: BatchRequest,
    on_event: Channel<BatchEvent>,
) -> std::result::Result<BatchReport, Error> {
    if !(1..=100).contains(&request.jpeg_quality) {
        return Err(anyhow::anyhow!("JPEG quality must be between 1 and 100").into());
    }
    let mut queue = Queue::default();
    queue.append(request.chapters)?;
    let run = Arc::new(BatchRun {
        id: request.id,
        queue: Mutex::new(queue),
        control: Mutex::new(Control::default()),
        stopped: Arc::new(AtomicBool::new(false)),
        wake: Notify::new(),
        export_wake: Condvar::new(),
        events: on_event,
    });
    {
        let mut active = state.active.lock();
        if active.is_some() {
            return Err(anyhow::anyhow!("another batch is already running").into());
        }
        *active = Some(run.clone());
    }
    let _guard = Guard(&state);
    let paths = run.queue.lock().paths();
    run.tell(BatchEvent::Started {
        total_chapters: paths.len(),
    });
    run.tell(BatchEvent::QueueChanged { paths });
    let mut report = BatchReport::default();
    loop {
        if !run.ready().await {
            report.stopped = true;
            break;
        }
        let Some(entry) = run.queue.lock().claim() else {
            break;
        };
        if entry.output.exists() && !entry.overwrite {
            report.skipped += 1;
            run.tell(BatchEvent::ChapterSkipped {
                path: entry.path.clone(),
                output: entry.output,
            });
            run.queue.lock().finish(false);
            continue;
        }
        let path = entry.path.clone();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let result = run_chapter(
            &app,
            &run,
            &entry.path,
            &entry.output,
            entry.overwrite,
            request.jpeg_quality,
        )
        .await;
        let stopped = run.stopped.load(Ordering::Acquire);
        match result {
            Ok(()) if !stopped => {
                report.completed += 1;
                report.failures.retain(|failure| failure.chapter != path);
                run.tell(BatchEvent::ChapterFinished {
                    path,
                    output: entry.output,
                });
                run.queue.lock().finish(false);
            }
            _ if stopped => {
                report.stopped = true;
                run.queue.lock().finish(false);
                break;
            }
            Err(error) => {
                let message = format!("{error:#}");
                report.failures.push(BatchFailure {
                    chapter: path.clone(),
                    error: message.clone(),
                });
                run.tell(BatchEvent::ChapterFailed {
                    path,
                    name,
                    error: message,
                });
                run.queue.lock().finish(true);
            }
            Ok(()) => unreachable!(),
        }
    }
    report
        .failures
        .retain(|failure| run.queue.lock().failed(&failure.chapter));
    app.pipeline.unload_when_idle();
    run.tell(BatchEvent::Finished {
        report: report.clone(),
    });
    Ok(report)
}

async fn run_chapter(
    app: &Arc<App>,
    batch: &Arc<BatchRun>,
    path: &Path,
    output: &Path,
    overwrite: bool,
    quality: u8,
) -> Result<()> {
    let input = path.to_path_buf();
    let chapter = Arc::new(tokio::task::spawn_blocking(move || Chapter::open(&input)).await??);
    if !batch.ready().await {
        anyhow::bail!("batch stopped");
    }
    let page_ids: Vec<_> = chapter
        .session
        .lock()
        .project()
        .pages()
        .map(|page| page.id)
        .collect();
    let pages = page_ids.len();
    let indices: HashMap<_, _> = page_ids
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect();
    let staging = tempfile::tempdir().context("create batch render staging")?;
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    batch.tell(BatchEvent::ChapterStarted {
        path: path.to_path_buf(),
        name,
        pages,
    });
    let (id, initial_pause) = {
        let mut control = batch.control.lock();
        ensure!(!batch.stopped.load(Ordering::Acquire), "batch stopped");
        let id = app.pipeline.run(
            chapter.session.clone(),
            Request {
                target: Target::Project,
                stages: BTreeSet::from([
                    Stage::Detect,
                    Stage::Recognize,
                    Stage::Inpaint,
                    Stage::Translate,
                ]),
                translation_order: koharu_pipeline::TranslationOrder::Independent,
                config: None,
            },
        )?;
        control.run = Some(id);
        (id, control.paused.then(|| app.pipeline.pause(id, true)))
    };
    if let Some(wait) = initial_pause {
        let _ = wait.await;
    }
    let mut cursor = 0;
    let mut active: HashMap<Stage, HashSet<PageId>> = HashMap::new();
    let mut finished: HashMap<Stage, HashSet<PageId>> = HashMap::new();
    let mut rendered = HashSet::new();
    let mut render_error = None;
    loop {
        let events = app.pipeline.history(id, Duration::from_millis(500)).await?;
        for event in &events[cursor..] {
            let changed = match event {
                Event::Started { page, stage, .. } => {
                    active.entry(*stage).or_default().insert(*page);
                    Some(*stage)
                }
                Event::Finished { page, stage, .. } => {
                    active.entry(*stage).or_default().remove(page);
                    finished.entry(*stage).or_default().insert(*page);
                    Some(*stage)
                }
                Event::Ended { outcome, .. } => {
                    batch.control.lock().run = None;
                    match outcome {
                        Outcome::Done => {}
                        Outcome::Stopped => {
                            if let Some(error) = render_error.take() {
                                return Err(error);
                            }
                            anyhow::bail!("batch stopped")
                        }
                        Outcome::Failed { reason, .. } => anyhow::bail!("{reason}"),
                    }
                    break;
                }
                Event::Queued { .. } => None,
            };
            if let Some(stage) = changed {
                batch.tell(BatchEvent::ChapterProgress {
                    path: path.to_path_buf(),
                    completed_pages: finished.get(&stage).map_or(0, HashSet::len),
                    active_pages: active.get(&stage).map_or(0, HashSet::len),
                    total_pages: pages,
                    completed_steps: finished.values().map(HashSet::len).sum(),
                    total_steps: pages * 4,
                    stage: stage.into(),
                });
            }
            if let Event::Finished { page, .. } = event
                && render_error.is_none()
                && !rendered.contains(page)
                && [
                    Stage::Detect,
                    Stage::Recognize,
                    Stage::Inpaint,
                    Stage::Translate,
                ]
                .iter()
                .all(|stage| {
                    finished
                        .get(stage)
                        .is_some_and(|pages| pages.contains(page))
                })
            {
                let page = *page;
                let index = *indices
                    .get(&page)
                    .context("finished page is not in the chapter")?;
                let render_chapter = chapter.clone();
                let render_batch = batch.clone();
                let renderer = app.renderer.clone();
                let stage_path = staging.path().to_owned();
                let result = tokio::task::spawn_blocking(move || {
                    render_batch.ready_export()?;
                    render_chapter.render_page(&renderer, page, index, quality, &stage_path)
                })
                .await;
                match result {
                    Ok(Ok(())) => {
                        rendered.insert(page);
                        batch.tell(BatchEvent::ChapterRendered {
                            path: path.to_path_buf(),
                            completed_pages: rendered.len(),
                            total_pages: pages,
                        });
                    }
                    Ok(Err(error)) => {
                        render_error = Some(error);
                        app.pipeline.stop_run(id);
                    }
                    Err(error) => {
                        render_error = Some(error.into());
                        app.pipeline.stop_run(id);
                    }
                }
            }
        }
        cursor = events.len();
        if matches!(
            events.last(),
            Some(Event::Ended {
                outcome: Outcome::Done,
                ..
            })
        ) {
            break;
        }
    }
    if let Some(error) = render_error {
        return Err(error);
    }
    ensure!(
        rendered.len() == pages,
        "not all chapter pages were rendered"
    );
    if !batch.ready().await {
        anyhow::bail!("batch stopped");
    }
    batch.tell(BatchEvent::ChapterExporting {
        path: path.to_path_buf(),
    });
    let destination = output.to_path_buf();
    let export_batch = batch.clone();
    let publish_batch = batch.clone();
    tokio::task::spawn_blocking(move || {
        chapter.publish(
            staging.path(),
            &destination,
            overwrite,
            || export_batch.ready_export(),
            |temporary, destination, overwrite| {
                publish_batch.publish(temporary, destination, overwrite)
            },
        )
    })
    .await??;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn append_batch(
    state: State<'_, BatchState>,
    id: String,
    chapters: Vec<BatchEntry>,
) -> std::result::Result<(), Error> {
    let run = current(&state, &id)?;
    let mut queue = run.queue.lock();
    queue.append(chapters)?;
    run.tell(BatchEvent::QueueChanged {
        paths: queue.paths(),
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn reorder_batch(
    state: State<'_, BatchState>,
    id: String,
    path: PathBuf,
    before: Option<PathBuf>,
) -> std::result::Result<(), Error> {
    let run = current(&state, &id)?;
    let mut queue = run.queue.lock();
    queue.reorder(&path, before.as_deref())?;
    run.tell(BatchEvent::QueueChanged {
        paths: queue.paths(),
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn remove_batch(
    state: State<'_, BatchState>,
    id: String,
    path: PathBuf,
) -> std::result::Result<(), Error> {
    let run = current(&state, &id)?;
    let mut queue = run.queue.lock();
    queue.remove(&path)?;
    run.tell(BatchEvent::QueueChanged {
        paths: queue.paths(),
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn retry_batch(
    state: State<'_, BatchState>,
    id: String,
    path: PathBuf,
) -> std::result::Result<(), Error> {
    let run = current(&state, &id)?;
    let mut queue = run.queue.lock();
    queue.retry(&path)?;
    run.tell(BatchEvent::ChapterQueued { path });
    run.tell(BatchEvent::QueueChanged {
        paths: queue.paths(),
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn pause_batch(
    app: State<'_, Arc<App>>,
    state: State<'_, BatchState>,
    id: String,
) -> std::result::Result<bool, Error> {
    let run = current(&state, &id)?;
    let wait = {
        let mut control = run.control.lock();
        control.paused = true;
        control.run.map(|id| app.pipeline.pause(id, true))
    };
    if let Some(wait) = wait {
        let _ = wait.await;
    }
    Ok(!run.stopped.load(Ordering::Acquire) && run.control.lock().paused)
}

#[tauri::command]
#[specta::specta]
pub async fn resume_batch(
    app: State<'_, Arc<App>>,
    state: State<'_, BatchState>,
    id: String,
) -> std::result::Result<(), Error> {
    let run = current(&state, &id)?;
    let wait = {
        let mut control = run.control.lock();
        control.paused = false;
        control.run.map(|id| app.pipeline.pause(id, false))
    };
    if let Some(wait) = wait {
        let _ = wait.await;
    }
    run.wake.notify_one();
    run.export_wake.notify_all();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn stop_batch(app: State<'_, Arc<App>>, state: State<'_, BatchState>) {
    if let Some(run) = state.active.lock().clone() {
        let control = run.control.lock();
        run.stopped.store(true, Ordering::Release);
        if let Some(id) = control.run {
            app.pipeline.stop_run(id);
        }
        drop(control);
        run.wake.notify_one();
        run.export_wake.notify_all();
    }
}
