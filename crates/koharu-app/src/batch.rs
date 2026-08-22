use std::{
    collections::VecDeque,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use futures::future::try_join_all;
use koharu_desktop::Desktop;
use koharu_pipeline::{
    Committer, Operation, Pipeline, PipelineConfig, Progress, ProgressSink, Request, RunStatus,
    Scope, Stage, StageOutput, StopToken,
};
use koharu_rasterizer::Rasterizer;
use koharu_renderer::Renderer;
use koharu_scene::{
    AssetInput, AssetMetadata, AssetRole, At, EntityId, PageDraft, Session, Snapshot,
};
use koharu_translator::Provider;
use parking_lot::Mutex;
use rayon::prelude::*;
use serde::Serialize;
use specta::Type;
use tauri::{Cef, State, WebviewWindow, ipc::Channel};

use crate::commands::{Error, lifecycle::load_archive, output};

const COMIC_INFO_LIMIT: u64 = 2 * 1024 * 1024;
const THUMBNAIL_ENTRY_LIMIT: u64 = 512 * 1024 * 1024;
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];
pub const DEFAULT_REMOTE_TRANSLATION_CONCURRENCY: u8 = 4;
const MAX_REMOTE_TRANSLATION_CONCURRENCY: u8 = 8;

type EventSink = Arc<dyn Fn(BatchEvent) + Send + Sync>;

#[derive(Clone, Debug)]
pub struct BatchOptions {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub jpeg_quality: u8,
    pub translation_concurrency: u8,
    pub overwrite: bool,
    pub cpu: bool,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct BatchSource {
    pub path: PathBuf,
    pub default_output: PathBuf,
    pub chapters: Vec<BatchChapter>,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct BatchChapter {
    pub path: PathBuf,
    pub name: String,
    #[specta(type = f64)]
    pub size: u64,
    #[specta(type = f64)]
    pub pages: usize,
    pub thumbnail: Option<String>,
    pub error: Option<String>,
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
        #[specta(type = f64)]
        total_chapters: usize,
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
        stage: Option<Stage>,
    },
    ChapterExporting {
        #[specta(type = f64)]
        index: usize,
        output: PathBuf,
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
        #[specta(type = f64)]
        completed: usize,
        #[specta(type = f64)]
        skipped: usize,
        #[specta(type = f64)]
        failed: usize,
        stopped: bool,
    },
}

#[derive(Default)]
pub(crate) struct BatchState {
    stop: Mutex<Option<StopToken>>,
    translation_lanes: Mutex<Vec<Pipeline>>,
}

struct BatchGuard<'a>(&'a BatchState);

impl BatchState {
    fn begin(&self) -> Result<(BatchGuard<'_>, StopToken)> {
        let mut current = self.stop.lock();
        if current.is_some() {
            bail!("a batch is already running");
        }
        let stop = StopToken::default();
        *current = Some(stop.clone());
        Ok((BatchGuard(self), stop))
    }

    pub(crate) fn stop(&self) {
        if let Some(stop) = self.stop.lock().clone() {
            stop.stop();
        }
    }

    fn translation_lanes(&self, concurrency: u8) -> Result<Vec<Pipeline>> {
        validate_translation_concurrency(concurrency)?;
        if !remote_translation_selected()? {
            return Ok(Vec::new());
        }
        let mut lanes = self.translation_lanes.lock();
        while lanes.len() < usize::from(concurrency) {
            lanes.push(create_translation_lane()?);
        }
        Ok(lanes[..usize::from(concurrency)].to_vec())
    }
}

impl Drop for BatchGuard<'_> {
    fn drop(&mut self) {
        self.0.stop.lock().take();
    }
}

struct BatchRuntime {
    pipeline: Pipeline,
    translation_lanes: Vec<Pipeline>,
    renderer: Renderer,
    rasterizer: Arc<Rasterizer>,
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

#[tauri::command]
#[specta::specta]
pub(crate) async fn browse_batch_source(
    window: WebviewWindow<Cef>,
) -> std::result::Result<Option<BatchSource>, Error> {
    let Some(path) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .pick_folder()
        .await
        .map(|folder| folder.path().to_owned())
    else {
        return Ok(None);
    };
    Ok(Some(
        tokio::task::spawn_blocking(move || scan_batch_source(path))
            .await
            .context("batch source scanner stopped unexpectedly")??,
    ))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn browse_batch_output(
    window: WebviewWindow<Cef>,
) -> std::result::Result<Option<PathBuf>, Error> {
    Ok(rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .pick_folder()
        .await
        .map(|folder| folder.path().to_owned()))
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn process_batch(
    chapters: Vec<PathBuf>,
    output: PathBuf,
    jpeg_quality: u8,
    overwrite: bool,
    translation_concurrency: u8,
    on_event: Channel<BatchEvent>,
    pipeline: State<'_, Pipeline>,
    desktop: State<'_, Desktop>,
    batch: State<'_, BatchState>,
) -> std::result::Result<BatchReport, Error> {
    if chapters.is_empty() {
        return Err(anyhow::anyhow!("select at least one chapter").into());
    }
    validate_translation_concurrency(translation_concurrency)?;
    let (_guard, stop) = batch.begin()?;
    let sink: EventSink = Arc::new(move |event| {
        let _ = on_event.send(event);
    });
    let runtime = BatchRuntime {
        pipeline: pipeline.inner().clone(),
        translation_lanes: batch.translation_lanes(translation_concurrency)?,
        renderer: desktop.renderer(),
        rasterizer: desktop.rasterizer().await?,
    };
    Ok(run_chapters(
        chapters,
        output,
        jpeg_quality,
        overwrite,
        &runtime,
        stop,
        Some(sink),
    )
    .await?)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn stop_batch(batch: State<'_, BatchState>) -> std::result::Result<(), Error> {
    batch.stop();
    Ok(())
}

pub async fn run_batch(options: BatchOptions) -> Result<BatchReport> {
    validate_options(&options)?;
    let chapters = discover_chapters(&options.input)?;
    let output = options
        .output
        .clone()
        .unwrap_or_else(|| default_output_directory(&options.input));
    koharu_ml::init()
        .await
        .context("failed to initialize Koharu's model runtime")?;
    let runtime = BatchRuntime {
        pipeline: Pipeline::load(koharu_ml::device(options.cpu))?,
        translation_lanes: if remote_translation_selected()? {
            create_translation_lanes(options.translation_concurrency)?
        } else {
            Vec::new()
        },
        renderer: Renderer::new()?,
        rasterizer: Arc::new(Rasterizer::new()?),
    };
    run_chapters(
        chapters,
        output,
        options.jpeg_quality,
        options.overwrite,
        &runtime,
        StopToken::default(),
        None,
    )
    .await
}

fn remote_translation_selected() -> Result<bool> {
    let config = PipelineConfig::load()?;
    let remote = config.read()?.translation.model.provider != Provider::Local;
    Ok(remote)
}

fn create_translation_lane() -> Result<Pipeline> {
    Pipeline::load(koharu_ml::device(true))
}

fn create_translation_lanes(concurrency: u8) -> Result<Vec<Pipeline>> {
    validate_translation_concurrency(concurrency)?;
    (0..concurrency)
        .map(|_| create_translation_lane())
        .collect()
}

async fn run_chapters(
    chapters: Vec<PathBuf>,
    output_directory: PathBuf,
    jpeg_quality: u8,
    overwrite: bool,
    runtime: &BatchRuntime,
    stop: StopToken,
    events: Option<EventSink>,
) -> Result<BatchReport> {
    validate_chapters(&chapters, jpeg_quality)?;
    fs::create_dir_all(&output_directory).with_context(|| {
        format!(
            "failed to create output directory {}",
            output_directory.display()
        )
    })?;
    emit(
        &events,
        BatchEvent::Started {
            total_chapters: chapters.len(),
        },
    );

    let total_chapters = chapters.len();
    let mut report = BatchReport::default();
    for (index, chapter) in chapters.into_iter().enumerate() {
        if stop.stopped() {
            report.stopped = true;
            break;
        }
        let output = output_directory.join(
            chapter
                .file_name()
                .context("chapter path has no file name")?,
        );
        ensure_distinct_paths(&chapter, &output)?;
        if output.exists() && !overwrite {
            eprintln!("skip {}: output already exists", chapter.display());
            report.skipped += 1;
            emit(
                &events,
                BatchEvent::ChapterSkipped {
                    index,
                    output: output.clone(),
                },
            );
            continue;
        }

        eprintln!(
            "[{}/{}] translating {}",
            index + 1,
            total_chapters,
            chapter.display()
        );
        match process_chapter(
            &chapter,
            &output,
            jpeg_quality,
            runtime,
            stop.clone(),
            events.clone(),
            index,
            total_chapters,
        )
        .await
        {
            Ok(elapsed) => {
                report.completed += 1;
                emit(
                    &events,
                    BatchEvent::ChapterFinished {
                        index,
                        output: output.clone(),
                    },
                );
                eprintln!(
                    "finished {} in {:.1}s",
                    output.display(),
                    elapsed.as_secs_f64()
                );
            }
            Err(_) if stop.stopped() => {
                report.stopped = true;
                break;
            }
            Err(error) => {
                tracing::error!(chapter = %chapter.display(), %error, "batch chapter failed");
                let message = format!("{error:#}");
                emit(
                    &events,
                    BatchEvent::ChapterFailed {
                        index,
                        name: chapter_name(&chapter),
                        error: message.clone(),
                    },
                );
                eprintln!("failed {}: {message}", chapter.display());
                report.failures.push(BatchFailure {
                    chapter,
                    error: message,
                });
            }
        }
    }

    emit(
        &events,
        BatchEvent::Finished {
            completed: report.completed,
            skipped: report.skipped,
            failed: report.failures.len(),
            stopped: report.stopped,
        },
    );
    Ok(report)
}

async fn process_chapter(
    chapter: &Path,
    output_path: &Path,
    jpeg_quality: u8,
    runtime: &BatchRuntime,
    stop: StopToken,
    events: Option<EventSink>,
    index: usize,
    total_chapters: usize,
) -> Result<std::time::Duration> {
    let started = Instant::now();
    let archive_path = chapter.to_owned();
    let pages = tokio::task::spawn_blocking(move || load_archive(&archive_path))
        .await
        .context("CBZ import worker stopped unexpectedly")??;
    let page_count = pages.len();
    emit(
        &events,
        BatchEvent::ChapterStarted {
            index,
            total_chapters,
            name: chapter_name(chapter),
            pages: page_count,
        },
    );

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
    let page_ids = session
        .lock()
        .await
        .snapshot()
        .pages()
        .map(|page| page.id())
        .collect::<Vec<_>>();

    let progress = Arc::new(Mutex::new(ChapterProgress::new(
        page_count,
        Stage::ALL.len(),
    )));
    let progress_state = Arc::clone(&progress);
    let progress_events = events.clone();
    let progress_callback = Arc::new(move |event| {
        let mut progress = progress_state.lock();
        let stage = progress.record(event);
        emit(
            &progress_events,
            BatchEvent::ChapterProgress {
                index,
                completed_pages: progress.completed_pages,
                total_pages: progress.total_pages,
                completed_steps: progress.completed_steps,
                total_steps: progress.total_steps,
                stage,
            },
        );
    });
    for stage in Stage::ALL {
        let stage_started = Instant::now();
        {
            let mut progress = progress.lock();
            progress.start_stage(page_count);
            emit(
                &events,
                BatchEvent::ChapterProgress {
                    index,
                    completed_pages: progress.completed_pages,
                    total_pages: progress.total_pages,
                    completed_steps: progress.completed_steps,
                    total_steps: progress.total_steps,
                    stage: Some(stage),
                },
            );
        }
        if stage == Stage::Translation && !runtime.translation_lanes.is_empty() {
            execute_remote_translation(
                runtime,
                Arc::clone(&session),
                &page_ids,
                stop.clone(),
                progress_callback.clone(),
            )
            .await?;
        } else {
            execute_project_stage(
                &runtime.pipeline,
                Arc::clone(&session),
                stage,
                stop.clone(),
                progress_callback.clone(),
                page_count,
            )
            .await?;
        }
        tracing::info!(
            chapter = %chapter.display(),
            %stage,
            pages = page_count,
            remote_requests = runtime.translation_lanes.len(),
            elapsed_seconds = stage_started.elapsed().as_secs_f64(),
            "batch chapter stage finished"
        );
    }

    emit(
        &events,
        BatchEvent::ChapterExporting {
            index,
            output: output_path.to_owned(),
        },
    );
    let metadata_path = chapter.to_owned();
    let comic_info = tokio::task::spawn_blocking(move || read_comic_info(&metadata_path))
        .await
        .context("ComicInfo reader stopped unexpectedly")??;
    output::export_snapshot_cbz(
        session.lock().await.snapshot(),
        runtime.renderer.clone(),
        Arc::clone(&runtime.rasterizer),
        output_path.to_owned(),
        jpeg_quality,
        comic_info,
    )
    .await?;
    Ok(started.elapsed())
}

async fn execute_project_stage(
    pipeline: &Pipeline,
    session: Arc<tokio::sync::Mutex<Session>>,
    stage: Stage,
    stop: StopToken,
    progress: ProgressSink,
    page_count: usize,
) -> Result<()> {
    let snapshot = session.lock().await.snapshot();
    let mut committer = SessionCommitter(session);
    let report = pipeline
        .execute(
            snapshot,
            Request {
                operation: Operation::Only { stage },
                scope: Scope::Project,
                stop,
                progress: Some(progress),
                ..Request::default()
            },
            &mut committer,
        )
        .await?;
    if report.status != RunStatus::Completed {
        bail!("translation stopped during {stage} before all {page_count} pages completed");
    }
    Ok(())
}

async fn execute_remote_translation(
    runtime: &BatchRuntime,
    session: Arc<tokio::sync::Mutex<Session>>,
    pages: &[EntityId],
    stop: StopToken,
    progress: ProgressSink,
) -> Result<()> {
    let queue = Arc::new(tokio::sync::Mutex::new(
        pages.iter().copied().enumerate().collect::<VecDeque<_>>(),
    ));
    let workers = runtime.translation_lanes.iter().map(|pipeline| {
        let queue = Arc::clone(&queue);
        let session = Arc::clone(&session);
        let stop = stop.clone();
        let progress = progress.clone();
        async move {
            loop {
                let Some((page_index, page)) = queue.lock().await.pop_front() else {
                    break;
                };
                let snapshot = session.lock().await.snapshot();
                let mut committer = SessionCommitter(Arc::clone(&session));
                let report = pipeline
                    .execute(
                        snapshot,
                        Request {
                            operation: Operation::Only {
                                stage: Stage::Translation,
                            },
                            scope: Scope::Pages(vec![page]),
                            stop: stop.clone(),
                            progress: Some(progress.clone()),
                            ..Request::default()
                        },
                        &mut committer,
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "remote translation failed on page {} of {}",
                            page_index + 1,
                            pages.len()
                        )
                    })?;
                if report.status != RunStatus::Completed {
                    bail!(
                        "remote translation stopped on page {} of {}",
                        page_index + 1,
                        pages.len()
                    );
                }
            }
            Ok::<_, anyhow::Error>(())
        }
    });
    try_join_all(workers).await?;
    Ok(())
}

struct ChapterProgress {
    completed_pages: usize,
    total_pages: usize,
    completed_steps: usize,
    total_steps: usize,
}

impl ChapterProgress {
    fn new(total_pages: usize, stages_per_page: usize) -> Self {
        Self {
            completed_pages: 0,
            total_pages,
            completed_steps: 0,
            total_steps: total_pages.saturating_mul(stages_per_page),
        }
    }

    fn start_stage(&mut self, total_pages: usize) {
        self.completed_pages = 0;
        self.total_pages = total_pages;
    }

    fn record(&mut self, event: Progress) -> Option<Stage> {
        match event {
            Progress::Started { stages, .. } => stages.first().copied(),
            Progress::Loading { stage, .. } | Progress::Running { stage, .. } => Some(stage),
            Progress::Finished { stage, .. } | Progress::Skipped { stage, .. } => {
                self.completed_steps = self.completed_steps.saturating_add(1);
                self.completed_pages = self.completed_pages.saturating_add(1);
                Some(stage)
            }
        }
    }
}

fn emit(events: &Option<EventSink>, event: BatchEvent) {
    if let Some(events) = events {
        events(event);
    }
}

fn scan_batch_source(path: PathBuf) -> Result<BatchSource> {
    let chapters = discover_chapters(&path)?;
    let chapters = chapters
        .par_iter()
        .map(|chapter| chapter_card(chapter))
        .collect();
    let default_output = path.join("Translated");
    Ok(BatchSource {
        path,
        default_output,
        chapters,
    })
}

fn chapter_card(path: &Path) -> BatchChapter {
    let size = fs::metadata(path).map_or(0, |metadata| metadata.len());
    match inspect_chapter(path) {
        Ok((pages, thumbnail)) => BatchChapter {
            path: path.to_owned(),
            name: chapter_name(path),
            size,
            pages,
            thumbnail,
            error: None,
        },
        Err(error) => BatchChapter {
            path: path.to_owned(),
            name: chapter_name(path),
            size,
            pages: 0,
            thumbnail: None,
            error: Some(format!("{error:#}")),
        },
    }
}

fn inspect_chapter(path: &Path) -> Result<(usize, Option<String>)> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open archive {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("failed to read archive {}", path.display()))?;
    let mut images = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if entry.is_file() && has_any_extension(Path::new(entry.name()), IMAGE_EXTENSIONS) {
            images.push((index, entry.name().to_owned()));
        }
    }
    alphanumeric_sort::sort_slice_by_str_key(&mut images, |(_, name)| name);
    let Some((index, name)) = images.first() else {
        bail!("archive contains no supported images");
    };
    let mut entry = archive
        .by_index(*index)
        .with_context(|| format!("failed to read cover {name}"))?;
    if entry.size() > THUMBNAIL_ENTRY_LIMIT {
        bail!("cover image exceeds the 512 MiB page limit");
    }
    let mut bytes = Vec::with_capacity(usize::try_from(entry.size())?);
    entry
        .by_ref()
        .take(THUMBNAIL_ENTRY_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > THUMBNAIL_ENTRY_LIMIT {
        bail!("cover image exceeds the 512 MiB page limit");
    }
    let image = image::load_from_memory(&bytes).context("failed to decode cover image")?;
    let image = image.thumbnail(360, 480).to_rgba8();
    let encoder = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height());
    let thumbnail = encoder.encode(78.0);
    Ok((
        images.len(),
        Some(format!(
            "data:image/webp;base64,{}",
            BASE64.encode(&*thumbnail)
        )),
    ))
}

fn validate_options(options: &BatchOptions) -> Result<()> {
    if !options.input.exists() {
        bail!("input path does not exist: {}", options.input.display());
    }
    validate_translation_concurrency(options.translation_concurrency)?;
    validate_chapters(&discover_chapters(&options.input)?, options.jpeg_quality)
}

fn validate_translation_concurrency(concurrency: u8) -> Result<()> {
    if !(1..=MAX_REMOTE_TRANSLATION_CONCURRENCY).contains(&concurrency) {
        bail!(
            "remote translation concurrency must be between 1 and {}",
            MAX_REMOTE_TRANSLATION_CONCURRENCY
        );
    }
    Ok(())
}

fn validate_chapters(chapters: &[PathBuf], jpeg_quality: u8) -> Result<()> {
    if !(1..=100).contains(&jpeg_quality) {
        bail!("JPEG quality must be between 1 and 100");
    }
    if chapters.is_empty() {
        bail!("select at least one chapter");
    }
    for chapter in chapters {
        if !chapter.is_file() || !has_extension(chapter, "cbz") {
            bail!("batch input must be a CBZ file: {}", chapter.display());
        }
    }
    Ok(())
}

fn discover_chapters(input: &Path) -> Result<Vec<PathBuf>> {
    let mut chapters = if input.is_file() {
        if !has_extension(input, "cbz") {
            bail!("input file must use the .cbz extension");
        }
        vec![input.to_owned()]
    } else if input.is_dir() {
        fs::read_dir(input)
            .with_context(|| format!("failed to read {}", input.display()))?
            .filter_map(|entry| match entry {
                Ok(entry)
                    if entry.file_type().is_ok_and(|kind| kind.is_file())
                        && has_extension(&entry.path(), "cbz") =>
                {
                    Some(Ok(entry.path()))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<std::io::Result<Vec<_>>>()?
    } else {
        bail!("input must be a CBZ file or directory");
    };
    alphanumeric_sort::sort_slice_by_path_key(&mut chapters, PathBuf::as_path);
    if chapters.is_empty() {
        bail!("no CBZ chapters were found in {}", input.display());
    }
    Ok(chapters)
}

fn default_output_directory(input: &Path) -> PathBuf {
    if input.is_dir() {
        input.join("Translated")
    } else {
        input
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("Translated")
    }
}

fn ensure_distinct_paths(source: &Path, output: &Path) -> Result<()> {
    if output.exists() && fs::canonicalize(source)? == fs::canonicalize(output)? {
        bail!("refusing to replace source archive {}", source.display());
    }
    Ok(())
}

fn read_comic_info(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open archive {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("failed to read archive {}", path.display()))?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let is_comic_info = Path::new(entry.name())
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("ComicInfo.xml"));
        if !entry.is_file() || !is_comic_info {
            continue;
        }
        if entry.size() > COMIC_INFO_LIMIT {
            bail!("ComicInfo.xml exceeds the 2 MiB metadata limit");
        }
        let mut bytes = Vec::with_capacity(usize::try_from(entry.size())?);
        entry
            .by_ref()
            .take(COMIC_INFO_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > COMIC_INFO_LIMIT {
            bail!("ComicInfo.xml exceeds the 2 MiB metadata limit");
        }
        return Ok(Some(bytes));
    }
    Ok(None)
}

fn chapter_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("chapter.cbz")
        .to_owned()
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

fn has_any_extension(path: &Path, extensions: &[&str]) -> bool {
    extensions
        .iter()
        .any(|extension| has_extension(path, extension))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write as _};

    use image::{DynamicImage, ImageFormat, RgbaImage};
    use koharu_scene::EntityId;
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;

    #[test]
    fn discovers_top_level_chapters_in_natural_order() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("Chapter 10.cbz"), []).unwrap();
        fs::write(directory.path().join("Chapter 2.cbz"), []).unwrap();
        fs::write(directory.path().join("notes.txt"), []).unwrap();
        fs::create_dir(directory.path().join("nested")).unwrap();
        fs::write(directory.path().join("nested/Chapter 1.cbz"), []).unwrap();

        let names = discover_chapters(directory.path())
            .unwrap()
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(names, ["Chapter 2.cbz", "Chapter 10.cbz"]);
    }

    #[test]
    fn scans_cover_thumbnail_and_page_count() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chapter.cbz");
        let mut archive = ZipWriter::new(fs::File::create(&path).unwrap());
        for name in ["002.png", "001.png"] {
            archive
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            archive.write_all(&png(4, 6)).unwrap();
        }
        archive.finish().unwrap();

        let (pages, thumbnail) = inspect_chapter(&path).unwrap();
        assert_eq!(pages, 2);
        assert!(thumbnail.unwrap().starts_with("data:image/webp;base64,"));
    }

    #[test]
    fn preserves_small_comic_info_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chapter.cbz");
        let mut archive = ZipWriter::new(fs::File::create(&path).unwrap());
        archive
            .start_file("metadata/ComicInfo.xml", SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"<ComicInfo />").unwrap();
        archive.finish().unwrap();

        assert_eq!(read_comic_info(&path).unwrap().unwrap(), b"<ComicInfo />");
    }

    #[test]
    fn defaults_output_beside_a_single_chapter() {
        let input = Path::new("Manga/Chapter 1.cbz");
        assert_eq!(
            default_output_directory(input),
            Path::new("Manga/Translated")
        );
    }

    #[test]
    fn bounds_remote_translation_concurrency() {
        assert!(validate_translation_concurrency(1).is_ok());
        assert!(validate_translation_concurrency(4).is_ok());
        assert!(validate_translation_concurrency(8).is_ok());
        assert!(validate_translation_concurrency(0).is_err());
        assert!(validate_translation_concurrency(9).is_err());
    }

    #[test]
    fn keeps_chapter_progress_across_stage_passes() {
        let page = EntityId::new();
        let mut progress = ChapterProgress::new(1, 2);

        for stage in [Stage::Detection, Stage::Ocr] {
            progress.start_stage(1);
            progress.record(Progress::Started {
                pages: vec![page],
                stages: vec![stage],
            });
            assert_eq!(progress.completed_pages, 0);
            progress.record(Progress::Skipped { page, stage });
            assert_eq!(progress.completed_pages, 1);
        }

        assert_eq!(progress.completed_steps, 2);
        assert_eq!(progress.total_steps, 2);
    }

    #[tokio::test]
    async fn shared_committer_rebases_disjoint_page_results() {
        let mut session = Session::memory().await.unwrap();
        let mut pages = Vec::new();
        let patch = session
            .snapshot()
            .patch(|edit| {
                pages.push(edit.add_page(PageDraft::new("one", 10.0, 10.0), At::End)?);
                pages.push(edit.add_page(PageDraft::new("two", 10.0, 10.0), At::End)?);
                Ok(())
            })
            .unwrap();
        session.commit(patch).await.unwrap();
        let base = session.snapshot();
        let patches = pages
            .iter()
            .enumerate()
            .map(|(index, page)| {
                base.patch(|edit| {
                    edit.set_page(
                        *page,
                        PageDraft::new(format!("translated {}", index + 1), 10.0, 10.0),
                    )
                })
                .unwrap()
            })
            .collect::<Vec<_>>();
        let session = Arc::new(tokio::sync::Mutex::new(session));

        for (page, patch) in pages.iter().zip(patches) {
            SessionCommitter(Arc::clone(&session))
                .commit(StageOutput {
                    page: *page,
                    stage: Stage::Translation,
                    patch,
                })
                .await
                .unwrap();
        }

        let snapshot = session.lock().await.snapshot();
        assert_eq!(
            snapshot.page(pages[0]).unwrap().page().unwrap().label,
            "translated 1"
        );
        assert_eq!(
            snapshot.page(pages[1]).unwrap().page().unwrap().label,
            "translated 2"
        );
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::new(width, height))
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }
}
