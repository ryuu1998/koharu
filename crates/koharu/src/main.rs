#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use clap::Parser as _;
use koharu::panic;
use koharu::sentry;
use koharu_app as app;
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

#[derive(clap::Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Translate one CBZ or every top-level CBZ in a directory.
    Batch(BatchArguments),
}

#[derive(clap::Args)]
struct BatchArguments {
    /// A CBZ archive or directory containing CBZ chapters.
    #[arg(short, long)]
    input: PathBuf,

    /// Output directory. Defaults to a Translated directory beside the input.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// JPEG quality used for compact CBZ pages.
    #[arg(long, default_value_t = 95, value_parser = clap::value_parser!(u8).range(1..=100))]
    jpeg_quality: u8,

    /// Replace output archives that already exist.
    #[arg(long)]
    overwrite: bool,

    /// Run model inference on the CPU.
    #[arg(long)]
    cpu: bool,
}

impl From<BatchArguments> for app::BatchOptions {
    fn from(value: BatchArguments) -> Self {
        Self {
            input: value.input,
            output: value.output,
            jpeg_quality: value.jpeg_quality,
            overwrite: value.overwrite,
            cpu: value.cpu,
        }
    }
}

#[tokio::main]
#[tauri::cef_entry_point]
async fn main() {
    #[cfg(target_os = "windows")]
    {
        // SAFETY: This only requests the existing parent console. It does not allocate one.
        let _ = unsafe {
            windows::Win32::System::Console::AttachConsole(
                windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
            )
        };
    }

    let cli = Cli::parse();
    let _guard = sentry::initialize();
    panic::install();
    let filter = tracing_subscriber::filter::EnvFilter::builder()
        .with_default_directive(tracing::Level::INFO.into())
        .from_env_lossy();
    tracing_subscriber::registry()
        .with(filter)
        .with(sentry::tracing_layer())
        .with(koharu::tracing::TimingLayer::new())
        .init();

    if let Some(Command::Batch(arguments)) = cli.command {
        match app::run_batch(arguments.into()).await {
            Ok(report) => {
                eprintln!(
                    "batch finished: {} completed, {} skipped, {} failed",
                    report.completed,
                    report.skipped,
                    report.failures.len()
                );
                if !report.failures.is_empty() {
                    for failure in report.failures {
                        eprintln!("  {}: {}", failure.chapter.display(), failure.error);
                    }
                    std::process::exit(1);
                }
            }
            Err(error) => {
                eprintln!("batch failed: {error:#}");
                std::process::exit(1);
            }
        }
        return;
    }

    tokio::task::block_in_place(|| app::run(tauri::generate_context!()))
        .expect("failed to run the desktop application");
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn parses_batch_arguments() {
        let cli = Cli::try_parse_from([
            "koharu",
            "batch",
            "--input",
            "Manga Project",
            "--jpeg-quality",
            "92",
            "--overwrite",
        ])
        .unwrap();
        let Some(Command::Batch(arguments)) = cli.command else {
            panic!("batch command was not parsed");
        };
        assert_eq!(arguments.input, Path::new("Manga Project"));
        assert_eq!(arguments.jpeg_quality, 92);
        assert!(arguments.overwrite);
    }
}
