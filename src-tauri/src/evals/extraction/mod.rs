// Eval harness for per-email memory extraction (tasks + facts).
//
// Two flavors share this module:
//   - ExtractionKind::Tasks — scores whether the extractor surfaces the right
//     action items from an email.
//   - ExtractionKind::Facts — scores whether the extracted memory facts are
//     durable, useful, and grounded.
//
// For each case we render a 3-column row:
//   left:    source email (subject, sender, sanitized body snippet)
//   middle:  extracted tasks (or facts) as structured list
//   right:   LLM-as-judge verdict (score 0–1, rationale, flags)

pub mod judge;
pub mod report;
pub mod runner;

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionKind {
    Tasks,
    Facts,
}

impl ExtractionKind {
    pub fn label(&self) -> &'static str {
        match self {
            ExtractionKind::Tasks => "tasks",
            ExtractionKind::Facts => "facts",
        }
    }
}

impl fmt::Display for ExtractionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}
