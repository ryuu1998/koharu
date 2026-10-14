use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use anyhow::{Context as _, Result, ensure};
use async_trait::async_trait;
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
    desktop: State<'_, Desktop>,
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
    Ok(run_chapters(request, pipeline.inner().clone(), &desktop, run).await?)
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
    desktop: &Desktop,
    run: Arc<BatchRun>,
) -> Result<BatchReport> {
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
            run.queue.lock().finish();
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
                desktop,
                stop.clone(),
                Arc::clone(events),
            )
            .await
        }
        .await;
        match result {
            Ok(()) => {
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
                events(BatchEvent::ChapterFailed {
                    path: chapter.clone(),
                    name: chapter
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    error: message.clone(),
                });
                report.failures.push(BatchFailure {
                    chapter,
                    error: message,
                });
            }
        }
        run.queue.lock().finish();
    }
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
    let callback = Arc::new(move |event| {
        let stage = match &event {
            Progress::Started { .. } => return,
            Progress::Loading { stage, .. }
            | Progress::Running { stage, .. }
            | Progress::Finished { stage, .. }
            | Progress::Skipped { stage, .. } => *stage,
        };
        let mut current = progress_state.lock();
        current.record(event);
        progress_events(BatchEvent::ChapterProgress {
            path: progress_path.clone(),
            completed_pages: current.completed(stage),
            total_pages: count,
            completed_steps: current.finished.len(),
            total_steps: count * Stage::ALL.len(),
            stage,
        });
    });
    // One execution preserves the bounded page window and decoded images across stages.
    let snapshot = session.lock().await.snapshot();
    let mut committer = SessionCommitter(Arc::clone(&session));
    let processing = pipeline.execute(
        snapshot,
        Request {
            operation: Operation::Full,
            scope: Scope::Project,
            stop: stop.clone(),
            progress: Some(callback),
            inpainting_mask: None,
        },
        &mut committer,
    );
    let preparation = desktop.rasterizer();
    tokio::pin!(processing, preparation);
    // Renderer setup can overlap processing, but a renderer failure must not drop
    // in-flight model jobs before the pipeline has drained them.
    let (report, prepared) = tokio::select! {
        report = &mut processing => (report?, None),
        prepared = &mut preparation => (processing.await?, Some(prepared)),
    };
    ensure!(
        report.status == RunStatus::Completed && !stop.stopped(),
        "batch stopped"
    );
    let processing_elapsed = processing_started.elapsed();
    let Some(_export_activity) = stop.admit().await else {
        anyhow::bail!("batch stopped");
    };
    let export_started = Instant::now();
    events(BatchEvent::ChapterExporting {
        path: chapter.to_owned(),
    });
    let rasterizer = match prepared {
        Some(result) => result?,
        None => preparation.await?,
    };
    let path = chapter.to_owned();
    let metadata = tokio_rayon::spawn(move || batch_source::comic_info(&path)).await?;
    let snapshot = session.lock().await.snapshot();
    cbz::export(
        snapshot,
        desktop.renderer(),
        rasterizer,
        output,
        quality,
        metadata,
        overwrite,
    )
    .await?;
    eprintln!(
        "Batch {}: import {:.2}s, pipeline {:.2}s, export {:.2}s, total {:.2}s",
        chapter.file_name().unwrap_or_default().to_string_lossy(),
        import_elapsed.as_secs_f64(),
        processing_elapsed.as_secs_f64(),
        export_started.elapsed().as_secs_f64(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

#[derive(Default)]
struct ChapterProgress {
    finished: BTreeSet<(EntityId, Stage)>,
}
impl ChapterProgress {
    fn record(&mut self, progress: Progress) {
        if let Progress::Finished { page, stage, .. } | Progress::Skipped { page, stage } = progress
        {
            self.finished.insert((page, stage));
        }
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
    }
}
