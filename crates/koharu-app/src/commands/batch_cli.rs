use std::path::PathBuf;

use anyhow::Result;
use koharu_desktop::Desktop;
use koharu_pipeline::{Pipeline, StopToken};

use super::{
    batch::{BatchEvent, BatchReport, BatchRequest, run_chapters, validate},
    batch_source,
};

#[derive(Clone, Debug)]
pub struct BatchOptions {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub jpeg_quality: u8,
    pub overwrite: bool,
    pub cpu: bool,
}

pub async fn run_batch(options: BatchOptions) -> Result<BatchReport> {
    let output = options.output.unwrap_or_else(|| {
        if options.input.is_dir() {
            options.input.join("Translated")
        } else {
            options
                .input
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("Translated")
        }
    });
    let request = BatchRequest {
        chapters: batch_source::discover(&options.input)?,
        output,
        jpeg_quality: options.jpeg_quality,
        overwrite: options.overwrite,
    };
    validate(&request)?;
    koharu_ml::init().await?;
    let pipeline = Pipeline::load(koharu_ml::device(options.cpu))?;
    let desktop = Desktop::new()?;
    let stop = StopToken::default();
    let signal_stop = stop.clone();
    let signal = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_stop.stop();
        }
    });
    let events = std::sync::Arc::new(|event| match event {
        BatchEvent::ChapterStarted { name, .. } => eprintln!("Translating {name}"),
        BatchEvent::ChapterFinished { output, .. } => eprintln!("Saved {}", output.display()),
        BatchEvent::ChapterSkipped { output, .. } => {
            eprintln!("Skipped existing {}", output.display())
        }
        BatchEvent::ChapterFailed { name, error, .. } => eprintln!("Failed {name}: {error}"),
        _ => {}
    });
    let result = run_chapters(request, pipeline, &desktop, stop, events).await;
    signal.abort();
    result
}
