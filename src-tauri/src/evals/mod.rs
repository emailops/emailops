// Eval harness for the chat service.
//
// Compiled only when the `eval` cargo feature is on so the production binary
// stays lean. The entry point is `src/bin/chat_eval.rs`, which drives the
// `runner` module below.
//
// Architecture:
//   case_loader  — parse YAML cases into typed structs
//   harness      — run a single case through `services::chat::run_chat_turn`
//   metrics      — deterministic (heuristic) assertions against the trace/answer
//   judge        — OpenRouter-backed LLM-as-a-judge metrics (optional)
//   report       — render an HTML report with per-case results; also writes JSON
//   runner       — orchestrate: load env, copy DB, build mock app, iterate cases
//   json_report  — standardised machine-readable JSON schema for all eval runs

pub mod agent_search;
pub mod case_loader;
pub mod db_source;
pub mod email_classification;
pub mod extraction;
pub mod harness;
pub mod json_report;
pub mod judge;
pub mod metrics;
pub mod report;
pub mod runner;
pub mod shared;
pub mod shortcuts;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvalError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("App error: {0}")]
    App(#[from] crate::models::error::AppError),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),

    #[error("Template error: {0}")]
    Template(#[from] tera::Error),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Aborted: {0}")]
    Aborted(String),
}

pub type EvalResult<T> = std::result::Result<T, EvalError>;
