//! Koharu's Tauri-managed application state, commands, and lifecycle.

mod app;
mod commands;

pub use app::run;
pub use commands::batch::{BatchFailure, BatchReport};
pub use commands::batch_cli::{BatchOptions, run_batch};
pub use commands::bindings;
