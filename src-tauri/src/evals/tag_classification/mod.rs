// Tag-classification harness.
//
// `services::classification` assigns every synced email an intent, a topic
// and an urgency, and those tags drive the sidebar filters, the chat query
// planner's tag mapping and the priority ordering. Until now nothing measured
// them: `email_classification_eval` probes a different model on a different
// 10-way taxonomy with no ground truth, so a prompt edit or a decoding change
// could move every tag in the mailbox unnoticed.
//
// This harness runs the real `classify_with_provider` against a synthetic
// labelled corpus with the rule engine switched off, so the number it reports
// is the model's, not a regex's.

pub mod case_loader;
pub mod metrics;
pub mod report;
pub mod runner;
