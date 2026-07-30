//! Eval harness for the junk detector (spam / phishing / graymail).
//!
//! Fully synthetic and fully deterministic: cases live in
//! `src-tauri/evals/junk/cases/` and contain no personal-mailbox content, and
//! scoring is a confusion matrix over the detector's own bands — no LLM judge.
//!
//! This suite is the measurement gate for the whole feature. It ships before
//! the detector does, running against the `judge()` stub, so every later stage
//! is a diff on a report that already exists rather than a fresh set of numbers
//! with no baseline.

pub mod cases;
pub mod metrics;
pub mod runner;
