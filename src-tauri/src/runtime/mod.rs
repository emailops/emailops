//! Transport-agnostic application runtime.
//!
//! This module is the seam between "EmailOps the mail engine" and "whatever is driving
//! it" — a Tauri window, the CLI, or an HTTP server. Nothing in here knows about Tauri.
//!
//! * [`core::AppCore`] — one user's mailbox universe: database, data dir, task queues.
//! * [`ctx::UserCtx`] — the ambient per-user context that lets a single process serve
//!   several users without the event/log/secret seams crossing wires.
//! * [`spawn`] — context-preserving replacements for `tokio::spawn`.

pub mod core;
pub mod ctx;
pub mod spawn;

pub use core::AppCore;
pub use ctx::{UserCtx, UserId};
