//! Koharu's Tauri-managed application state, commands, and lifecycle.

mod app;
mod batch;
mod commands;

pub use app::run;
pub use batch::{BatchFailure, BatchOptions, BatchReport, run_batch};
pub use commands::bindings;
