// Query-planner harness.
//
// `chat.query_plan` turns one mailbox question into a single `search_emails`
// filter, and everything downstream inherits its mistakes: a date window the
// question never asked for costs the turn two extra rounds, a guessed
// classifier tag returns zero rows, a copied `limit: 5` hides matches. Those
// are cheap to measure directly — one small completion per case — so they get
// their own harness instead of being inferred from full chat answers.

pub mod case_loader;
pub mod metrics;
pub mod report;
pub mod runner;
