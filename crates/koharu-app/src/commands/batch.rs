use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, ensure};
use async_trait::async_trait;
use koharu_desktop::Desktop;
use koharu_pipeline::{
    Committer, Operation, Pipeline, Progress, Request, RunStatus, Scope, Stage, StageOutput,
    StopToken,
};
use koharu_scene::{
    AssetInput, AssetMetadata, AssetRole, At, EntityId, PageDraft, Session, Snapshot,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{State, ipc::Channel};

use super::{
    Error, batch_source, import,
    output::cbz,
    processing::{JobId, Processing},
};

#[derive(Clone, Debug, Deserialize, Type)]
pub struct BatchRequest {
    pub chapters: Vec<PathBuf>,
    pub output: PathBuf,
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
    ChapterStarted {
        #[specta(type = f64)]
        index: usize,
        name: String,
        #[specta(type = f64)]
        pages: usize,
    },
    ChapterProgress {
        #[specta(type = f64)]
        index: usize,
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
        #[specta(type = f64)]
        index: usize,
    },
    ChapterFinished {
        #[specta(type = f64)]
        index: usize,
        output: PathBuf,
    },
    ChapterSkipped {
        #[specta(type = f64)]
        index: usize,
        output: PathBuf,
    },
    ChapterFailed {
        #[specta(type = f64)]
        index: usize,
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
    stop: Mutex<Option<StopToken>>,
}

struct BatchGuard<'a> {
    batch: &'a BatchState,
    processing: &'a Processing,
    job: JobId,
}
impl Drop for BatchGuard<'_> {
    fn drop(&mut self) {
        self.batch.stop.lock().take();
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
    validate(&request)?;
    let job = JobId::new();
    let stop = StopToken::default();
    {
        let mut stops = processing.stops.lock();
        if !stops.is_empty() {
            return Err(anyhow::anyhow!("another process is already running").into());
        }
        *batch.stop.lock() = Some(stop.clone());
        stops.insert(job, stop.clone());
    }
    let _guard = BatchGuard {
        batch: &batch,
        processing: &processing,
        job,
    };
    let sink = Arc::new(move |event| {
        let _ = on_event.send(event);
    });
    Ok(run_chapters(request, pipeline.inner().clone(), &desktop, stop, sink).await?)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn stop_batch(batch: State<'_, BatchState>) -> std::result::Result<(), Error> {
    if let Some(stop) = batch.stop.lock().as_ref() {
        stop.stop();
    }
    Ok(())
}

pub(super) fn validate(request: &BatchRequest) -> Result<()> {
    ensure!(!request.chapters.is_empty(), "select at least one chapter");
    ensure!(
        (1..=100).contains(&request.jpeg_quality),
        "JPEG quality must be between 1 and 100"
    );
    for chapter in &request.chapters {
        ensure!(
            chapter.is_file() && batch_source::is_cbz(chapter),
            "input must be a CBZ archive: {}",
            chapter.display()
        );
        let output = request
            .output
            .join(chapter.file_name().context("chapter has no filename")?);
        if output.exists() {
            ensure!(
                std::fs::canonicalize(chapter)? != std::fs::canonicalize(&output)?,
                "choose a different output folder; source archives cannot be replaced"
            );
        }
    }
    Ok(())
}

pub(super) async fn run_chapters(
    request: BatchRequest,
    pipeline: Pipeline,
    desktop: &Desktop,
    stop: StopToken,
    events: EventSink,
) -> Result<BatchReport> {
    validate(&request)?;
    tokio::fs::create_dir_all(&request.output).await?;
    let mut report = BatchReport::default();
    events(BatchEvent::Started {
        total_chapters: request.chapters.len(),
    });
    for (index, chapter) in request.chapters.iter().enumerate() {
        if stop.stopped() {
            report.stopped = true;
            break;
        }
        let output = request
            .output
            .join(chapter.file_name().context("chapter has no filename")?);
        if output.exists() && !request.overwrite {
            report.skipped += 1;
            events(BatchEvent::ChapterSkipped { index, output });
            continue;
        }
        match process_chapter(
            chapter,
            output.clone(),
            request.jpeg_quality,
            request.overwrite,
            &pipeline,
            desktop,
            stop.clone(),
            Arc::clone(&events),
            index,
        )
        .await
        {
            Ok(()) => {
                report.completed += 1;
                events(BatchEvent::ChapterFinished { index, output });
            }
            Err(_) if stop.stopped() => {
                report.stopped = true;
                break;
            }
            Err(error) => {
                let message = format!("{error:#}");
                events(BatchEvent::ChapterFailed {
                    index,
                    name: chapter
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    error: message.clone(),
                });
                report.failures.push(BatchFailure {
                    chapter: chapter.clone(),
                    error: message,
                });
            }
        }
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
    stop: StopToken,
    events: EventSink,
    index: usize,
) -> Result<()> {
    let path = chapter.to_owned();
    let pages = tokio_rayon::spawn(move || import::import(vec![path])).await?;
    let count = pages.len();
    events(BatchEvent::ChapterStarted {
        index,
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
    let session = Arc::new(tokio::sync::Mutex::new(session));
    let progress = Arc::new(Mutex::new(ChapterProgress::default()));
    for stage in Stage::ALL {
        if stop.stopped() {
            anyhow::bail!("batch stopped");
        }
        progress.lock().phase_pages = 0;
        let progress_state = Arc::clone(&progress);
        let progress_events = Arc::clone(&events);
        let callback = Arc::new(move |event| {
            let mut current = progress_state.lock();
            current.record(event);
            progress_events(BatchEvent::ChapterProgress {
                index,
                completed_pages: current.phase_pages,
                total_pages: count,
                completed_steps: current.finished.len(),
                total_steps: count * Stage::ALL.len(),
                stage,
            });
        });
        let snapshot = session.lock().await.snapshot();
        let report = pipeline
            .execute(
                snapshot,
                Request {
                    operation: Operation::Only { stage },
                    scope: Scope::Project,
                    stop: stop.clone(),
                    progress: Some(callback),
                    inpainting_mask: None,
                },
                &mut SessionCommitter(Arc::clone(&session)),
            )
            .await?;
        ensure!(
            report.status == RunStatus::Completed && !stop.stopped(),
            "batch stopped"
        );
    }
    events(BatchEvent::ChapterExporting { index });
    let path = chapter.to_owned();
    let metadata = tokio_rayon::spawn(move || batch_source::comic_info(&path)).await?;
    cbz::export(
        session.lock().await.snapshot(),
        desktop.renderer(),
        desktop.rasterizer().await?,
        output,
        quality,
        metadata,
        overwrite,
    )
    .await?;
    Ok(())
}

#[derive(Default)]
struct ChapterProgress {
    finished: BTreeSet<(EntityId, Stage)>,
    phase_pages: usize,
}
impl ChapterProgress {
    fn record(&mut self, progress: Progress) {
        if let Progress::Finished { page, stage, .. } | Progress::Skipped { page, stage } = progress
        {
            if self.finished.insert((page, stage)) {
                self.phase_pages += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_to_overwrite_an_original_archive() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chapter.cbz");
        std::fs::write(&path, b"archive").unwrap();
        let request = BatchRequest {
            chapters: vec![path],
            output: directory.path().to_owned(),
            jpeg_quality: 95,
            overwrite: true,
        };
        assert!(
            validate(&request)
                .unwrap_err()
                .to_string()
                .contains("source archives")
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
        progress.phase_pages = 0;
        progress.record(Progress::Skipped {
            page,
            stage: Stage::Ocr,
        });
        assert_eq!(progress.finished.len(), 2);
        assert_eq!(progress.phase_pages, 1);
    }
}
