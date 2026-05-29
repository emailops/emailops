// Shortcut-variant eval harness.
//
// Where the chat eval (evals/runner.rs) asks *does our chat pipeline answer
// a given user query well?*, this harness asks the complementary question:
// *which shortcut prompt produces the best answer for the same model/mailbox?*
//
// For each shortcut (5 hardcoded ones in ChatView.tsx today) we run N prompt
// variants through the real chat pipeline against a frozen time window, then
// score each variant on four split dimensions (structure / faithfulness /
// usefulness / tone) plus a deterministic structural rubric. Output is an
// HTML report with variants laid out side by side so you can eyeball which
// prompt wins per shortcut.
//
// Module layout mirrors `evals::runner`:
//   case_loader  — YAML → ShortcutCase (with Vec<ShortcutVariant>)
//   metrics      — deterministic structural rubric (table? columns? rows? es?)
//   judge        — split-rubric OpenRouter judge
//   runner       — orchestrate: run every (shortcut, variant) pair, render report
//   report       — side-by-side HTML

pub mod case_loader;
pub mod judge;
pub mod metrics;
pub mod report;
pub mod runner;
