// YAML case loader.
//
// Parses `*.yaml` files under `src-tauri/evals/chat/cases/` into typed
// `EvalCase` structs. The YAML format is documented inline in
// `user_queries.yaml`.

use std::path::Path;

use chrono::NaiveDate;
use serde::Deserialize;

use crate::evals::{EvalError, EvalResult};
use crate::models::RouteMode;

/// What "today" means for a case at run time. The chat path reads
/// `services::clock::current()` to format date strings into the system prompt
/// and to compute the search window for the today/this-week shortcuts; the
/// harness installs a `FixedClock` derived from this value so cases that
/// depend on "today" can run deterministically against a static fixture (the
/// demo DB) without drifting as wall-clock advances.
///
/// YAML shapes:
/// ```yaml
/// as_of: "2024-01-15"   # pin to a literal UTC date
/// as_of: latest         # resolve to MAX(received_at) in the case's account inbox
/// ```
/// `as_of` is optional — absent means "use the system clock", preserving the
/// pre-existing behaviour for every case that didn't opt in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsOf {
    /// Pin to an explicit UTC date.
    Date(NaiveDate),
    /// Resolve to the newest email's date in the case's account inbox at run
    /// start. Keeps cases robust against demo-DB regeneration where the
    /// dataset's "today" floats with wall-clock at generation time.
    Latest,
}

impl<'de> serde::Deserialize<'de> for AsOf {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        let trimmed = raw.trim();
        if trimmed.eq_ignore_ascii_case("latest") {
            Ok(AsOf::Latest)
        } else {
            NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
                .map(AsOf::Date)
                .map_err(|e| serde::de::Error::custom(format!("invalid as_of {trimmed:?}: {e}")))
        }
    }
}

/// Metrics the judge can run for a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    AnswerRelevancy,
    Faithfulness,
    ContextualRelevancy,
    ContextualRecall,
}

impl MetricKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MetricKind::AnswerRelevancy => "answer_relevancy",
            MetricKind::Faithfulness => "faithfulness",
            MetricKind::ContextualRelevancy => "contextual_relevancy",
            MetricKind::ContextualRecall => "contextual_recall",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub question: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default = "default_tier")]
    pub tier: String,
    #[serde(default)]
    pub model: Option<String>,

    /// Account override for this case — either the account id or the account's
    /// email address. When set, overrides the CLI --account flag and the
    /// auto-resolved default. Used so cases can target the mailbox where the
    /// test data actually lives.
    #[serde(default)]
    pub account: Option<String>,

    /// Thread id to seed the conversation with. When set, the harness creates
    /// the conversation via `create_conversation_with_thread` (instead of an
    /// empty chat), so `run_chat_turn` takes the thread-bound short-circuit —
    /// the path that grounds the answer in this single thread and exposes only
    /// the `generate_email_draft` tool. Use this to eval thread-bound chat
    /// (e.g. "draft a reply") rather than the RAG/tools pipeline.
    #[serde(default)]
    pub thread_id: Option<String>,

    /// Thread the "main view" has open, passed as *ambient* context for this
    /// turn — the chat panel's removable context chip, not a seeded
    /// conversation. Distinct from `thread_id`: that one binds the whole
    /// conversation up front (`ChatTurnMode::ConversationThread`), while this
    /// exercises `ChatTurnMode::AmbientThread`, which resolves per turn and is
    /// never persisted.
    #[serde(default)]
    pub ambient_thread_id: Option<String>,

    /// Account owning `ambient_thread_id`. Set it when the thread belongs to a
    /// *different* account than `account` — that is the interesting case, since
    /// the panel runs on the first enabled account in unified ("All accounts")
    /// mode while the user can be reading any account's thread. Leaving this
    /// unset makes the turn fall back to its own account.
    #[serde(default)]
    pub ambient_account: Option<String>,

    /// Expected router mode. Absence skips the route check.
    #[serde(default)]
    pub expected_route: Option<RouteMode>,

    /// Tool names that must appear in `trace.tool_calls` (in any order).
    #[serde(default)]
    pub expected_tools_called: Vec<String>,

    /// Tool names that must NOT appear in `trace.tool_calls`. For turns whose
    /// correct behaviour is a plain text answer — e.g. thread-bound
    /// translate/summarise requests, which must not reach for
    /// `generate_email_draft` and save a draft instead of answering.
    #[serde(default)]
    pub expected_tools_not_called: Vec<String>,

    /// Case-insensitive substrings that must appear in the final assistant content.
    #[serde(default)]
    pub expected_answer_contains: Vec<String>,

    /// Case-insensitive substrings that must NOT appear in the final assistant
    /// content. Guards against failure-mode phrasings ("I couldn't access…")
    /// that a positive anchor cannot distinguish from a real answer.
    #[serde(default)]
    pub expected_answer_not_contains: Vec<String>,

    /// Case-insensitive substrings that must appear in the serialized JSON
    /// arguments of at least one traced tool call — pins what a tool was asked
    /// (e.g. the exact sender address), not just that it ran.
    #[serde(default)]
    pub expected_tool_args_contains: Vec<String>,

    /// Regex pattern the auto-derived conversation title must match.
    #[serde(default)]
    pub expected_title_pattern: Option<String>,

    /// Golden reference answer used by the LLM judge (optional).
    #[serde(default)]
    pub expected_output: Option<String>,

    /// Metrics the judge should run for this case.
    #[serde(default)]
    pub metrics: Vec<MetricKind>,

    /// Pin "today" for this case so time-dependent prompts (e.g. "summarize
    /// today's emails") resolve deterministically against a static fixture.
    /// See [`AsOf`] for the YAML shapes.
    #[serde(default)]
    pub as_of: Option<AsOf>,
}

fn default_category() -> String {
    "general".into()
}

fn default_tier() -> String {
    "smoke".into()
}

/// Load every `*.yaml` file in `dir` into a flat `Vec<EvalCase>`.
pub fn load_cases(dir: &Path) -> EvalResult<Vec<EvalCase>> {
    if !dir.exists() {
        return Err(EvalError::Config(format!(
            "cases directory does not exist: {}",
            dir.display()
        )));
    }

    let mut out: Vec<EvalCase> = Vec::new();
    let mut yaml_paths: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("yaml"))
        .collect();
    yaml_paths.sort();

    for path in yaml_paths {
        let text = std::fs::read_to_string(&path)?;
        let cases: Vec<EvalCase> =
            serde_yaml::from_str(&text).map_err(|e| EvalError::Config(format!("{}: {}", path.display(), e)))?;
        out.extend(cases);
    }

    // Sanity: unique ids.
    let mut seen = std::collections::HashSet::new();
    for c in &out {
        if !seen.insert(c.id.clone()) {
            return Err(EvalError::Config(format!("duplicate case id: {}", c.id)));
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_case(yaml: &str) -> EvalCase {
        let cases: Vec<EvalCase> = serde_yaml::from_str(yaml).expect("parse yaml");
        cases.into_iter().next().expect("at least one case")
    }

    #[test]
    fn as_of_defaults_to_none() {
        let c = parse_case(
            r#"
- id: t
  question: "q"
"#,
        );
        assert_eq!(c.as_of, None);
    }

    #[test]
    fn as_of_accepts_iso_date() {
        let c = parse_case(
            r#"
- id: t
  question: "q"
  as_of: "2024-01-15"
"#,
        );
        assert_eq!(c.as_of, Some(AsOf::Date(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap())));
    }

    #[test]
    fn as_of_accepts_latest_sentinel() {
        let c = parse_case(
            r#"
- id: t
  question: "q"
  as_of: latest
"#,
        );
        assert_eq!(c.as_of, Some(AsOf::Latest));
    }

    /// Regression guard: the public `user_queries.yaml` must parse with the
    /// AsOf-extended schema. If someone reformats the YAML in a way that
    /// breaks deserialisation (e.g. mistyping `as_of: latest` as `as_of:
    /// :latest`), this fires at build time instead of at `make cli-eval` time.
    #[test]
    fn public_user_queries_yaml_parses_with_as_of_field() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("evals/chat/cases");
        let cases = load_cases(&dir).expect("public user_queries.yaml must parse");
        let demo_daily = cases
            .iter()
            .find(|c| c.id == "demo_daily_summary")
            .expect("demo_daily_summary case must be present in the public YAML");
        assert_eq!(
            demo_daily.as_of,
            Some(AsOf::Latest),
            "demo_daily_summary must pin as_of: latest so today resolves against the demo fixture"
        );
    }

    #[test]
    fn as_of_rejects_garbage() {
        let result: std::result::Result<Vec<EvalCase>, _> = serde_yaml::from_str(
            r#"
- id: t
  question: "q"
  as_of: "not-a-date"
"#,
        );
        assert!(result.is_err(), "expected parse failure for garbage as_of value");
    }
}
