// Pooled-recall eval harness for `services::agent_search::run_agent_search`.
//
// For each query in `cases.yaml`:
//   1. Run every configured `AgentSearchMode` against the production DB.
//   2. Pool the returned email IDs into a single deduplicated set (top-K per
//      mode, union).
//   3. Send each pool member to an OpenRouter LLM judge that rates the email's
//      relevance to the query on a 0/1/2 scale, with a brief rationale.
//   4. From the judgments, compute per-mode Precision@K, Recall@K, F1@K and
//      mean wall-clock latency.
//   5. Emit a self-contained HTML report.
//
// The point is to measure how much value the smart mode adds over the baseline
// FTS on real user mail without manual labelling — pool-based qrels are the
// standard IR trick for this.

pub mod cases;
pub mod judge;
pub mod report;
pub mod runner;

pub use runner::{run as run_agent_search_eval, RunConfig};
