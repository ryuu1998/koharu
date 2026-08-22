//! Koharu's Tauri-managed application state, commands, and lifecycle.

mod app;
mod batch;
mod commands;

pub use app::run;
pub use batch::{
    BatchFailure, BatchOptions, BatchReport, DEFAULT_REMOTE_TRANSLATION_CONCURRENCY, run_batch,
};
pub use commands::bindings;
