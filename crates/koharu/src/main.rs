#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod mallinfo;

use clap::Parser as _;
use koharu::panic;
use koharu::sentry;
use koharu_app as app;
use tracing_subscriber::{Layer as _, filter::filter_fn, layer::SubscriberExt as _};

#[derive(clap::Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Translate CBZ chapters without opening the desktop editor.
    Batch(BatchArgs),
}

#[derive(clap::Args)]
struct BatchArgs {
    /// A CBZ archive or a directory containing top-level CBZ chapters.
    input: std::path::PathBuf,
    #[arg(short, long)]
    output: Option<std::path::PathBuf>,
    #[arg(long, default_value_t = 95, value_parser = clap::value_parser!(u8).range(1..=100))]
    jpeg_quality: u8,
    #[arg(long)]
    overwrite: bool,
    #[arg(long)]
    cpu: bool,
}

#[tokio::main]
#[tauri_runtime_cef::cef_entry_point]
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
    let filter = filter_fn(|metadata| metadata.target() != "koharu_metrics");
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::filter::EnvFilter::builder()
                    .with_default_directive(tracing::Level::INFO.into())
                    .from_env_lossy(),
            )
            .with(sentry::tracing_layer().with_filter(filter.clone()))
            .with(koharu_metrics::layer())
            .with(koharu::tracing::TimingLayer::new().with_filter(filter)),
    )
    .expect("failed to set the global tracing subscriber");
    if let Some(Command::Batch(args)) = cli.command {
        match app::run_batch(app::BatchOptions {
            input: args.input,
            output: args.output,
            jpeg_quality: args.jpeg_quality,
            overwrite: args.overwrite,
            cpu: args.cpu,
        })
        .await
        {
            Ok(report) => {
                eprintln!(
                    "{} translated, {} skipped, {} failed",
                    report.completed,
                    report.skipped,
                    report.failures.len()
                );
                if report.stopped {
                    std::process::exit(130);
                }
                if !report.failures.is_empty() {
                    std::process::exit(1);
                }
            }
            Err(error) => {
                eprintln!("Batch failed: {error:#}");
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
    use super::*;

    #[test]
    fn parses_batch_arguments_and_bounds_jpeg_quality() {
        let cli = Cli::try_parse_from([
            "koharu",
            "batch",
            "chapters",
            "--output",
            "translated",
            "--cpu",
        ])
        .unwrap();
        let Some(Command::Batch(args)) = cli.command else {
            panic!("expected batch command");
        };
        assert_eq!(args.input, std::path::PathBuf::from("chapters"));
        assert_eq!(args.jpeg_quality, 95);
        assert!(args.cpu);
        assert!(
            Cli::try_parse_from(["koharu", "batch", "chapters", "--jpeg-quality", "0"]).is_err()
        );
    }
}
