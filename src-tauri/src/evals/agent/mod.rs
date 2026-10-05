// Email agent harness.
//
// The agent makes two decisions per email or event: which rules and panels
// match (one structured call), and what each matched rule does (one call per
// rule). Both run through `services::agent::runner::decide`, the function the
// app uses, so a prompt or planner change shows up here unchanged. A case
// checks the matched set exactly and the actions it must / must not take.

pub mod runner;
