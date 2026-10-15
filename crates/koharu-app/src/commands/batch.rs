use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use anyhow::{Context as _, Result, ensure};
use async_trait::async_trait;
use futures::stream;
use koharu_desktop::Desktop;
use koharu_pipeline::{
    Committer, Operation, Pipeline, Progress, Request, RunControl, RunStatus, Scope, Stage,
    StageOutput,
};
use koharu_scene::{
    AssetInput, AssetMetadata, AssetRole, At, EntityId, PageDraft, Session, Snapshot,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{State, ipc::Channel};

use super::{
    Error,
    batch_queue::{BatchEntry, Queue},
    batch_source, import,
    output::cbz,
    processing::{JobId, Processing},
};

#[derive(Clone, Debug, Deserialize, Type)]
pub struct BatchRequest {
    pub id: String,
    pub chapters: Vec<BatchEntry>,
    pub jpeg_quality: u8,
    pub overwrite: bool,
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
        stage: Stage,
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

pub(super) type EventSink = Arc<dyn Fn(BatchEvent) + Send + Sync>;

#[derive(Default)]
pub(crate) struct BatchState {
    active: Mutex<Option<Arc<BatchRun>>>,
}

pub(super) struct BatchRun {
    id: String,
    queue: Mutex<Queue>,
    stop: RunControl,
    events: EventSink,
}

struct BatchGuard<'a> {
    batch: &'a BatchState,
    processing: &'a Processing,
    job: JobId,
}
impl Drop for BatchGuard<'_> {
    fn drop(&mut self) {
        self.batch.active.lock().take();
        self.processing.stops.lock().remove(&self.job);
    }
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn process_batch(
    request: BatchRequest,
    on_event: Channel<BatchEvent>,
    pipeline: State<'_, Pipeline>,
    batch: State<'_, BatchState>,
    processing: State<'_, Processing>,
) -> std::result::Result<BatchReport, Error> {
    let stop = RunControl::default();
    let sink: EventSink = Arc::new(move |event| {
        let _ = on_event.send(event);
    });
    let run = prepare(&request, stop.clone(), sink)?;
    let job = JobId::new();
    {
        let mut stops = processing.stops.lock();
        let mut active = batch.active.lock();
        if !stops.is_empty() || active.is_some() {
            return Err(anyhow::anyhow!("another process is already running").into());
        }
        *active = Some(run.clone());
        stops.insert(job, stop);
    }
    let _guard = BatchGuard {
        batch: &batch,
        processing: &processing,
        job,
    };
    Ok(run_chapters(request, pipeline.inner().clone(), run).await?)
}

fn current(batch: &BatchState, id: &str) -> Result<Arc<BatchRun>> {
    let active = batch.active.lock();
    let run = active.as_ref().context("no batch is running")?;
    ensure!(run.id == id, "the batch has changed");
    Ok(run.clone())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn append_batch(
    id: String,
    chapters: Vec<BatchEntry>,
    batch: State<'_, BatchState>,
) -> std::result::Result<(), Error> {
    let run = current(&batch, &id)?;
    if run.stop.stopped() {
        return Err(anyhow::anyhow!("the batch is stopping").into());
    }
    let mut queue = run.queue.lock();
    queue.append(chapters)?;
    (run.events)(BatchEvent::QueueChanged {
        paths: queue.paths(),
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn reorder_batch(
    id: String,
    path: PathBuf,
    before: Option<PathBuf>,
    batch: State<'_, BatchState>,
) -> std::result::Result<(), Error> {
    let run = current(&batch, &id)?;
    let mut queue = run.queue.lock();
    queue.reorder(&path, before.as_deref())?;
    (run.events)(BatchEvent::QueueChanged {
        paths: queue.paths(),
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn remove_batch(
    id: String,
    path: PathBuf,
    batch: State<'_, BatchState>,
) -> std::result::Result<(), Error> {
    let run = current(&batch, &id)?;
    if run.stop.stopped() {
        return Err(anyhow::anyhow!("the batch is stopping").into());
    }
    let mut queue = run.queue.lock();
    queue.remove(&path)?;
    (run.events)(BatchEvent::QueueChanged {
        paths: queue.paths(),
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn retry_batch(
    id: String,
    path: PathBuf,
    batch: State<'_, BatchState>,
) -> std::result::Result<(), Error> {
    let run = current(&batch, &id)?;
    if run.stop.stopped() {
        return Err(anyhow::anyhow!("the batch is stopping").into());
    }
    let mut queue = run.queue.lock();
    queue.retry(&path)?;
    (run.events)(BatchEvent::ChapterQueued { path });
    (run.events)(BatchEvent::QueueChanged {
        paths: queue.paths(),
    });
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn pause_batch(
    id: String,
    batch: State<'_, BatchState>,
) -> std::result::Result<bool, Error> {
    let run = current(&batch, &id)?;
    run.stop.pause();
    Ok(run.stop.drained().await)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn resume_batch(
    id: String,
    batch: State<'_, BatchState>,
) -> std::result::Result<(), Error> {
    current(&batch, &id)?.stop.resume();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn stop_batch(batch: State<'_, BatchState>) -> std::result::Result<(), Error> {
    if let Some(run) = batch.active.lock().as_ref() {
        run.stop.stop();
    }
    Ok(())
}

pub(super) fn prepare(
    request: &BatchRequest,
    stop: RunControl,
    events: EventSink,
) -> Result<Arc<BatchRun>> {
    ensure!(!request.id.is_empty(), "batch identity is required");
    ensure!(!request.chapters.is_empty(), "select at least one chapter");
    ensure!(
        (1..=100).contains(&request.jpeg_quality),
        "JPEG quality must be between 1 and 100"
    );
    let mut queue = Queue::default();
    queue.append(request.chapters.clone())?;
    Ok(Arc::new(BatchRun {
        id: request.id.clone(),
        queue: Mutex::new(queue),
        stop,
        events,
    }))
}

pub(super) async fn run_chapters(
    request: BatchRequest,
    pipeline: Pipeline,
    run: Arc<BatchRun>,
) -> Result<BatchReport> {
    // Batch render resources belong to this run, not the open editor. Dropping
    // this owner releases its rasterizer, decoded images, nodes, and font cache.
    let desktop = Desktop::new()?;
    let events = &run.events;
    let stop = &run.stop;
    let mut report = BatchReport::default();
    {
        let queue = run.queue.lock();
        events(BatchEvent::Started {
            total_chapters: queue.paths().len(),
        });
        events(BatchEvent::QueueChanged {
            paths: queue.paths(),
        });
    }
    loop {
        if !stop.ready().await {
            report.stopped = true;
            break;
        }
        // Claim and append use the same lock: work is either accepted or the run is closed.
        let Some(entry) = run.queue.lock().claim() else {
            break;
        };
        let chapter = entry.path;
        let output = entry.output;
        if output.exists() && !request.overwrite {
            report.skipped += 1;
            events(BatchEvent::ChapterSkipped {
                path: chapter,
                output,
            });
            run.queue.lock().finish(false);
            continue;
        }
        let result = async {
            tokio::fs::create_dir_all(output.parent().context("output needs a parent")?).await?;
            process_chapter(
                &chapter,
                output.clone(),
                request.jpeg_quality,
                request.overwrite,
                &pipeline,
                &desktop,
                stop.clone(),
                Arc::clone(events),
            )
            .await
        }
        .await;
        match result {
            Ok(()) => {
                run.queue.lock().finish(false);
                report.completed += 1;
                events(BatchEvent::ChapterFinished {
                    path: chapter,
                    output,
                });
            }
            Err(_) if stop.stopped() => {
                report.stopped = true;
                break;
            }
            Err(error) => {
                let message = format!("{error:#}");
                run.queue.lock().finish(true);
                tracing::error!(batch_id = %run.id, chapter = %chapter.display(), error = %message, "batch chapter failed");
                events(BatchEvent::ChapterFailed {
                    path: chapter.clone(),
                    name: chapter
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    error: message.clone(),
                });
                report.failures.retain(|failure| failure.chapter != chapter);
                report.failures.push(BatchFailure {
                    chapter,
                    error: message,
                });
            }
        }
    }
    report
        .failures
        .retain(|failure| run.queue.lock().failed(&failure.chapter));
    drop(desktop);
    pipeline.unload().await;
    tracing::info!(batch_id = %run.id, "batch resources released");
    tracing::info!(batch_id = %run.id, completed = report.completed, skipped = report.skipped,
        failed = report.failures.len(), stopped = report.stopped, "batch finished");
    events(BatchEvent::Finished {
        report: report.clone(),
    });
    Ok(report)
}

struct SessionCommitter(Arc<tokio::sync::Mutex<Session>>);
#[async_trait]
impl Committer for SessionCommitter {
    async fn commit(&mut self, output: StageOutput) -> Result<Snapshot> {
        let mut session = self.0.lock().await;
        let patch = output.patch.rebase_on(&session.snapshot())?;
        Ok(session.commit(patch).await?.snapshot)
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_chapter(
    chapter: &Path,
    output: PathBuf,
    quality: u8,
    overwrite: bool,
    pipeline: &Pipeline,
    desktop: &Desktop,
    stop: RunControl,
    events: EventSink,
) -> Result<()> {
    let started = Instant::now();
    let path = chapter.to_owned();
    let Some(import_activity) = stop.admit().await else {
        anyhow::bail!("batch stopped");
    };
    let pages = tokio_rayon::spawn(move || import::import(vec![path])).await?;
    drop(import_activity);
    let count = pages.len();
    events(BatchEvent::ChapterStarted {
        path: chapter.to_owned(),
        name: chapter
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        pages: count,
    });
    let mut session = Session::memory().await?;
    let patch = session.snapshot().patch(move |edit| {
        for page in pages {
            let id = edit.add_page(
                PageDraft::new(page.name, f64::from(page.width), f64::from(page.height)),
                At::End,
            )?;
            edit.set_asset(
                id,
                &AssetRole::new("source")?,
                AssetInput::new(
                    page.bytes,
                    page.format.to_mime_type(),
                    AssetMetadata {
                        width: Some(page.width),
                        height: Some(page.height),
                        attributes: Default::default(),
                    },
                ),
            )?;
        }
        Ok(())
    })?;
    session.commit(patch).await?;
    let import_elapsed = started.elapsed();
    let processing_started = Instant::now();
    let session = Arc::new(tokio::sync::Mutex::new(session));
    let progress = Arc::new(Mutex::new(ChapterProgress::default()));
    let progress_state = Arc::clone(&progress);
    let progress_events = Arc::clone(&events);
    let progress_path = chapter.to_owned();
    let (ready_pages, ready_receiver) = tokio::sync::mpsc::unbounded_channel();
    let callback = Arc::new(move |event| {
        let stage = match &event {
            Progress::Started { .. } => return,
            Progress::Loading { stage, .. }
            | Progress::Running { stage, .. }
            | Progress::Finished { stage, .. }
            | Progress::Skipped { stage, .. } => *stage,
        };
        let mut current = progress_state.lock();
        let ready = current.record(event);
        progress_events(BatchEvent::ChapterProgress {
            path: progress_path.clone(),
            completed_pages: current.completed(stage),
            active_pages: current.active(stage),
            total_pages: count,
            completed_steps: current.finished.len(),
            total_steps: count * Stage::ALL.len(),
            stage,
        });
        if let Some(page) = ready {
            let _ = ready_pages.send(page);
        }
    });
    // The pipeline commits each page before its readiness notification. Only
    // page IDs wait in this queue; rendered JPEGs are staged on disk.
    let snapshot = session.lock().await.snapshot();
    let mut committer = SessionCommitter(Arc::clone(&session));
    let processing = pipeline.execute(
        snapshot.clone(),
        Request {
            operation: Operation::Full,
            scope: Scope::Project,
            stop: stop.clone(),
            progress: Some(callback),
            inpainting_mask: None,
        },
        &mut committer,
    );
    let render_session = session.clone();
    let ready = stream::unfold(ready_receiver, move |mut receiver| {
        let session = render_session.clone();
        async move {
            let page = receiver.recv().await?;
            let snapshot = session.lock().await.snapshot();
            Some((Ok((page, snapshot)), receiver))
        }
    });
    let render_events = events.clone();
    let render_path = chapter.to_owned();
    let rendering = async {
        let rasterizer = desktop.rasterizer().await?;
        cbz::prepare(
            snapshot,
            ready,
            desktop.renderer(),
            rasterizer,
            quality,
            stop.clone(),
            Some(Arc::new(move |completed_pages| {
                render_events(BatchEvent::ChapterRendered {
                    path: render_path.clone(),
                    completed_pages,
                    total_pages: count,
                });
            })),
        )
        .await
    };
    // Neither a rendering error nor a pipeline error may drop native work that
    // was already admitted. Publication waits for both sides to finish.
    let (report, prepared) = tokio::join!(processing, rendering);
    let report = report?;
    ensure!(
        report.status == RunStatus::Completed && !stop.stopped(),
        "batch stopped"
    );
    let processing_elapsed = processing_started.elapsed();
    let prepared = prepared?;
    let Some(_export_activity) = stop.admit().await else {
        anyhow::bail!("batch stopped");
    };
    let export_started = Instant::now();
    events(BatchEvent::ChapterExporting {
        path: chapter.to_owned(),
    });
    let path = chapter.to_owned();
    let metadata = tokio_rayon::spawn(move || batch_source::comic_info(&path)).await?;
    prepared.write(output, metadata, overwrite).await?;
    tracing::info!(chapter = %chapter.display(), import_secs = import_elapsed.as_secs_f64(),
        processing_secs = processing_elapsed.as_secs_f64(), export_secs = export_started.elapsed().as_secs_f64(),
        total_secs = started.elapsed().as_secs_f64(), "batch chapter completed");
    Ok(())
}

#[derive(Default)]
struct ChapterProgress {
    finished: BTreeSet<(EntityId, Stage)>,
    active: BTreeSet<(EntityId, Stage)>,
}
impl ChapterProgress {
    fn record(&mut self, progress: Progress) -> Option<EntityId> {
        match progress {
            Progress::Loading { page, stage, .. } | Progress::Running { page, stage, .. } => {
                if !self.finished.contains(&(page, stage)) {
                    self.active.insert((page, stage));
                }
                None
            }
            Progress::Finished { page, stage, .. } | Progress::Skipped { page, stage } => {
                self.active.remove(&(page, stage));
                (self.finished.insert((page, stage))
                    && Stage::ALL
                        .iter()
                        .all(|stage| self.finished.contains(&(page, *stage))))
                .then_some(page)
            }
            Progress::Started { .. } => None,
        }
    }

    fn active(&self, stage: Stage) -> usize {
        self.active
            .iter()
            .filter(|(_, active)| *active == stage)
            .count()
    }

    fn completed(&self, stage: Stage) -> usize {
        self.finished
            .iter()
            .filter(|(_, completed)| *completed == stage)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_page_becomes_renderable_once_after_its_own_stages_finish() {
        let first = EntityId::new();
        let second = EntityId::new();
        let mut progress = ChapterProgress::default();
        for page in [first, second] {
            for stage in [Stage::Detection, Stage::Ocr, Stage::Inpainting] {
                assert!(progress.record(Progress::Skipped { page, stage }).is_none());
            }
        }
        assert_eq!(
            progress.record(Progress::Skipped {
                page: second,
                stage: Stage::Translation,
            }),
            Some(second)
        );
        assert!(
            progress
                .record(Progress::Skipped {
                    page: second,
                    stage: Stage::Translation,
                })
                .is_none()
        );
        assert_eq!(
            progress.record(Progress::Skipped {
                page: first,
                stage: Stage::Translation,
            }),
            Some(first)
        );
    }

    #[test]
    fn progress_stays_monotonic_across_phase_changes_and_duplicate_events() {
        let page = EntityId::new();
        let mut progress = ChapterProgress::default();
        progress.record(Progress::Skipped {
            page,
            stage: Stage::Detection,
        });
        progress.record(Progress::Skipped {
            page,
            stage: Stage::Detection,
        });
        assert_eq!(progress.finished.len(), 1);
        progress.record(Progress::Skipped {
            page,
            stage: Stage::Ocr,
        });
        assert_eq!(progress.finished.len(), 2);
        assert_eq!(progress.completed(Stage::Ocr), 1);
        assert_eq!(progress.completed(Stage::Detection), 1);
        progress.record(Progress::Running {
            page,
            stage: Stage::Detection,
            model: "detection".to_owned(),
        });
        assert_eq!(progress.finished.len(), 2);
        assert_eq!(progress.completed(Stage::Detection), 1);
        assert_eq!(progress.active(Stage::Detection), 0);
    }

    #[test]
    fn pending_translations_are_visible_before_any_page_finishes() {
        let mut progress = ChapterProgress::default();
        let pages = [EntityId::new(), EntityId::new()];
        for page in pages {
            for _ in 0..2 {
                progress.record(Progress::Running {
                    page,
                    stage: Stage::Translation,
                    model: "cloud".to_owned(),
                });
            }
        }
        assert_eq!(progress.active(Stage::Translation), 2);
        assert_eq!(progress.completed(Stage::Translation), 0);
        progress.record(Progress::Skipped {
            page: pages[0],
            stage: Stage::Translation,
        });
        assert_eq!(progress.active(Stage::Translation), 1);
        assert_eq!(progress.completed(Stage::Translation), 1);
    }
}
