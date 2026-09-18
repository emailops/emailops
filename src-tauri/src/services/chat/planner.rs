//! Model-backed query planner: turn ONE tools-first chat question into a single
//! `search_emails` filter (or defer to the normal tool loop) via a tiny, focused
//! completion — so the chat model skips the slow, error-prone tool-choice round.
//!
//! Split per the repo's planner/executor rule:
//!   - **pure** `parse_plan` (model text → [`Plan`]) and `SearchPlan::into_tool_call`
//!     ([`Plan`] → the pre-seeded tool call) — exhaustively unit-tested, no I/O.
//!   - **thin** [`plan_search`] executor — renders the prompt, calls the provider,
//!     parses. Reuses the already-loaded chat provider/model (no model swap; see
//!     the single-runtime cache in `services::ai`), so by default it runs on the
//!     configured chat model — `qwen3.5-4b-q4_k_m` out of the box.
//!
//! The planner only ever pre-seeds a search or defers; it never invents an answer.
//! Any uncertainty (unparseable output, an empty filter, a provider error, a
//! non-search ask) falls through to `Plan::Defer` so the normal loop still runs —
//! the fast path can only ever *save* a round, never break a turn.

use crate::ai::provider::{AIProvider, AiToolCall, AiToolCallFunction, CompletionOptions};
use crate::services::classification::TagGlossary;

/// The planner's decision for a turn.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// Pre-seed `search_emails` with these filters as the turn's round-0 call.
    /// Boxed: eight optional strings make the variant far larger than `Defer`.
    Search(Box<SearchPlan>),
    /// Not a single email search — let the normal model tool loop handle it.
    Defer,
}

/// The subset of `search_emails` arguments the planner can fill. All optional;
/// at least one selective field must be present for the plan to be a `Search`
/// (the tool rejects a filter-less call).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchPlan {
    pub query: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    /// Classifier tags — the planner's way to express a concept ("prospects")
    /// the mailbox never spells out.
    pub intent: Option<String>,
    pub topic: Option<String>,
    /// `"semantic"` to rank `query` by meaning instead of exact keywords —
    /// the planner's way to search for a description no tag captures.
    pub mode: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub limit: Option<i64>,
    /// `"oldest"` to surface the FIRST matching email ("primer correo"),
    /// `"newest"` (or unset) for the default most-recent-first.
    pub order: Option<String>,
    /// `Some(true)` = only mail the user has not read. `false` is no filter
    /// and is never stored.
    pub unread: Option<bool>,
}

impl SearchPlan {
    fn wants_oldest(&self) -> bool {
        self.order.as_deref() == Some("oldest")
    }
}

impl SearchPlan {
    /// True when no selective filter is set — `search_emails` would reject this,
    /// so the planner defers instead of pre-seeding a useless call.
    fn is_empty(&self) -> bool {
        self.query.is_none()
            && self.from.is_none()
            && self.to.is_none()
            && self.subject.is_none()
            && self.since.is_none()
            && self.until.is_none()
            && self.intent.is_none()
            && self.topic.is_none()
            && self.unread.is_none()
    }

    /// The plan without the classifier tags the planner guessed.
    ///
    /// `intent` / `topic` only match what the classifier tagged, and the model
    /// adds them even when the question named no kind of mail — its own prompt
    /// forbids it, and it does it anyway. On a turn the keyword heuristic did
    /// not recognise, that guess is the part most likely to return nothing
    /// ("¿qué correos de BorgBase tengo sin leer?" planned with
    /// intent=notification over invoices tagged billing → zero rows), so the
    /// hard filters are kept and the guess is dropped.
    pub fn without_classifier_tags(mut self) -> Self {
        self.intent = None;
        self.topic = None;
        self
    }

    /// Whether the plan names a FILTER (sender, recipient, subject, date
    /// window, classifier tag, unread) rather than just words to match.
    ///
    /// This is the line between the two retrieval mechanisms. A filter is
    /// something only `search_emails` can express, so a plan that carries one
    /// is worth taking off the RAG route. A keyword-only plan ("qué opina el
    /// equipo sobre el proyecto" → `query: "proyecto"`) is exactly what the
    /// embeddings index ranks better, so it stays on RAG.
    pub fn has_structural_filter(&self) -> bool {
        self.from.is_some()
            || self.to.is_some()
            || self.subject.is_some()
            || self.since.is_some()
            || self.until.is_some()
            || self.intent.is_some()
            || self.topic.is_some()
            || self.unread == Some(true)
    }

    /// Convert the plan into the `search_emails` tool call fed into the loop as
    /// the virtual round-0. `include_bodies` is set so the synthesis pass has the
    /// content in one shot and never needs a follow-up `get_email_body` round
    /// (mirrors the today/week summary shortcuts).
    pub fn into_tool_call(self) -> AiToolCall {
        // Capture order/limit before the field-by-field moves below. "first /
        // oldest" with no explicit count means THE single first email; otherwise
        // default to 25.
        let oldest = self.wants_oldest();
        let limit = self.limit.unwrap_or(if oldest { 1 } else { 25 });
        let unread = self.unread == Some(true);

        let mut args = serde_json::Map::new();
        let mut put = |k: &str, v: Option<String>| {
            if let Some(s) = v {
                args.insert(k.to_string(), serde_json::Value::String(s));
            }
        };
        put("query", self.query);
        put("from", self.from);
        put("to", self.to);
        put("subject", self.subject);
        put("intent", self.intent);
        put("topic", self.topic);
        put("mode", self.mode);
        put("since", self.since);
        put("until", self.until);
        if oldest {
            args.insert("order".to_string(), serde_json::Value::String("oldest".to_string()));
        }
        if unread {
            args.insert("unread".to_string(), serde_json::Value::Bool(true));
        }
        args.insert("limit".to_string(), serde_json::json!(limit));
        args.insert("include_bodies".to_string(), serde_json::json!(true));
        AiToolCall {
            function: AiToolCallFunction {
                name: "search_emails".to_string(),
                arguments: serde_json::Value::Object(args),
            },
        }
    }
}

/// Parse the planner model's reply into a [`Plan`]. Lenient by design — the model
/// may wrap the JSON in prose or ``` fences. Anything ambiguous (no JSON, an
/// explicit `{"defer": true}`, or an all-empty filter) becomes [`Plan::Defer`] so
/// the turn falls back to the normal loop rather than running a broken search.
impl SearchPlan {
    /// Deterministic clean-up of what the model planned, applied before the
    /// plan becomes a tool call:
    ///
    /// 1. A sender/recipient name copied into `query` is dropped. The planner
    ///    prompt forbids it, the model does it anyway ("acmenews" in
    ///    both `from` and `query`), and the extra full-text match over bodies
    ///    cost ~8 s per question for no gain.
    /// 2. A bare first name in `from` ("alex") asks for 5 rows instead of 1
    ///    so namesakes surface: "último email de alex" returned one
    ///    newsletter and the model never learned there was another Alex to
    ///    ask about. "oldest" keeps a single row — "the first email" is one.
    fn normalised(mut self) -> Self {
        let same = |a: &Option<String>, b: &Option<String>| match (a, b) {
            (Some(x), Some(y)) => x.trim().eq_ignore_ascii_case(y.trim()),
            _ => false,
        };
        if same(&self.query, &self.from) || same(&self.query, &self.to) {
            self.query = None;
        }
        let bare_name = self
            .from
            .as_deref()
            .map(|f| {
                let f = f.trim();
                !f.is_empty() && !f.contains('@') && !f.contains('.') && !f.contains(char::is_whitespace)
            })
            .unwrap_or(false);
        if bare_name && !self.wants_oldest() && self.limit.unwrap_or(25) < 5 {
            self.limit = Some(5);
        }
        self
    }
}

pub fn parse_plan(text: &str) -> Plan {
    let Some(obj) = extract_json_object(text) else {
        return Plan::Defer;
    };
    if obj.get("defer").and_then(|v| v.as_bool()) == Some(true) {
        return Plan::Defer;
    }
    let str_field = |key: &str| {
        obj.get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    // Accept the limit as a JSON number or a numeric string; clamp to the tool's
    // 1..=25 range so a hallucinated `1000` can't blow the result set.
    let limit = obj
        .get("limit")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
        })
        .map(|n| n.clamp(1, 25));
    // Only honour a recognised direction; anything else (or absent) → newest.
    let order = str_field("order")
        .map(|o| o.to_lowercase())
        .filter(|o| o == "oldest" || o == "newest");
    let plan = SearchPlan {
        query: str_field("query"),
        from: str_field("from"),
        to: str_field("to"),
        subject: str_field("subject"),
        intent: str_field("intent").map(|v| v.to_lowercase()),
        topic: str_field("topic").map(|v| v.to_lowercase()),
        // Only the semantic switch is meaningful; "keyword" is the default
        // and anything else is noise.
        mode: str_field("mode").map(|m| m.to_lowercase()).filter(|m| m == "semantic"),
        since: str_field("since"),
        until: str_field("until"),
        limit,
        order,
        unread: obj.get("unread").and_then(|v| v.as_bool()).filter(|u| *u),
    };
    if plan.is_empty() {
        return Plan::Defer;
    }
    Plan::Search(Box::new(plan.normalised()))
}

/// Lenient JSON-object extraction: drop ``` fences, then parse the first
/// balanced-looking `{...}` slice into a map. Returns `None` when there is no
/// parseable object.
fn extract_json_object(text: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let cleaned = text.replace("```json", "").replace("```", "");
    let start = cleaned.find('{')?;
    let end = cleaned.rfind('}')?;
    if end <= start {
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(&cleaned[start..=end]) {
        Ok(serde_json::Value::Object(map)) => Some(map),
        _ => None,
    }
}

/// Monday-anchored week boundaries (ISO `YYYY-MM-DD`, end-exclusive) derived
/// deterministically from `today`. "This week" is the calendar week starting
/// Monday and containing `today`; "last week" is the preceding one. Injected
/// into the planner prompt so week math never depends on the model counting
/// weekdays from a bare date (which it gets wrong).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WeekBounds {
    pub this_since: String,
    pub this_until: String,
    pub last_since: String,
    pub last_until: String,
}

/// Compute Monday-anchored week bounds from an ISO `today` string. Returns
/// `None` when `today` is not a valid `YYYY-MM-DD` date.
pub(crate) fn week_bounds(today: &str) -> Option<WeekBounds> {
    use chrono::{Datelike, Duration, NaiveDate};
    let d = NaiveDate::parse_from_str(today.trim(), "%Y-%m-%d").ok()?;
    // Monday = 0 … Sunday = 6.
    let offset = d.weekday().num_days_from_monday() as i64;
    let this_monday = d - Duration::days(offset);
    let next_monday = this_monday + Duration::days(7);
    let last_monday = this_monday - Duration::days(7);
    let fmt = |dt: NaiveDate| dt.format("%Y-%m-%d").to_string();
    Some(WeekBounds {
        this_since: fmt(this_monday),
        this_until: fmt(next_monday),
        last_since: fmt(last_monday),
        last_until: fmt(this_monday),
    })
}

/// Render the planner prompt from its registry template, substituting the
/// per-turn variables. Pure (no DB / no I/O) so it is unit-testable; the executor
/// fetches the template via `prompts::get_template`.
pub(crate) fn render_planner_prompt(
    template: &str,
    user_email: &str,
    today: &str,
    query: &str,
    glossary: &TagGlossary,
) -> String {
    let mut vars = std::collections::HashMap::new();
    vars.insert("user_email", user_email.to_string());
    vars.insert("today", today.to_string());
    vars.insert("query", query.to_string());
    // The classifier's tags with their meanings, so a concept in the question
    // ("prospects", "quote requests") maps onto a tag by definition — the
    // vocabulary follows Settings, not a hard-coded list.
    vars.insert("intent_definitions", TagGlossary::render_lines(&glossary.intents));
    vars.insert("topic_definitions", TagGlossary::render_lines(&glossary.topics));
    // Deterministic Monday-anchored week ranges so "this week" / "last week"
    // never rely on the model's weekday arithmetic. Empty on an unparseable
    // date — the template's generic relative-date rule still applies.
    let wb = week_bounds(today);
    vars.insert(
        "this_week_since",
        wb.as_ref().map(|w| w.this_since.clone()).unwrap_or_default(),
    );
    vars.insert(
        "this_week_until",
        wb.as_ref().map(|w| w.this_until.clone()).unwrap_or_default(),
    );
    vars.insert(
        "last_week_since",
        wb.as_ref().map(|w| w.last_since.clone()).unwrap_or_default(),
    );
    vars.insert(
        "last_week_until",
        wb.as_ref().map(|w| w.last_until.clone()).unwrap_or_default(),
    );
    crate::services::prompts::render(template, &vars)
}

/// Thin executor: render the prompt, run ONE completion on the (already-loaded)
/// chat provider, and parse the reply into a [`Plan`]. Never errors — a provider
/// failure degrades to [`Plan::Defer`] so the turn proceeds normally.
pub(crate) async fn plan_search(
    provider: &dyn AIProvider,
    template: &str,
    user_email: &str,
    today: &str,
    query: &str,
    glossary: &TagGlossary,
) -> Plan {
    let prompt = render_planner_prompt(template, user_email, today, query, glossary);
    let opts = CompletionOptions {
        temperature: Some(0.0),
        max_tokens: Some(128),
        think: Some(false),
    };
    match provider.complete(&prompt, opts).await {
        Ok(result) => parse_plan(&result.text),
        Err(_) => Plan::Defer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::classification::ClassificationConfig;

    fn search(text: &str) -> SearchPlan {
        match parse_plan(text) {
            Plan::Search(p) => *p,
            Plan::Defer => panic!("expected Search, got Defer for: {text}"),
        }
    }

    #[test]
    fn week_bounds_anchors_on_monday() {
        // 2026-06-30 is a Tuesday; its week starts Monday 2026-06-29.
        let w = week_bounds("2026-06-30").expect("valid date");
        assert_eq!(w.this_since, "2026-06-29");
        assert_eq!(w.this_until, "2026-07-06", "end-exclusive: next Monday");
        assert_eq!(w.last_since, "2026-06-22");
        assert_eq!(w.last_until, "2026-06-29");
    }

    #[test]
    fn week_bounds_on_monday_and_sunday() {
        // Monday: the week starts on that day.
        let mon = week_bounds("2026-06-29").expect("valid");
        assert_eq!(mon.this_since, "2026-06-29");
        assert_eq!(mon.this_until, "2026-07-06");
        // Sunday: still the same week starting the prior Monday.
        let sun = week_bounds("2026-07-05").expect("valid");
        assert_eq!(sun.this_since, "2026-06-29");
        assert_eq!(sun.this_until, "2026-07-06");
    }

    #[test]
    fn week_bounds_rejects_bad_date() {
        assert!(week_bounds("not-a-date").is_none());
        assert!(week_bounds("").is_none());
    }

    #[test]
    fn render_planner_prompt_injects_week_ranges() {
        let tmpl = "this={{this_week_since}}..{{this_week_until}} last={{last_week_since}}..{{last_week_until}}";
        let out = render_planner_prompt(tmpl, "me@x.com", "2026-06-30", "this week", &TagGlossary::defaults());
        assert_eq!(out, "this=2026-06-29..2026-07-06 last=2026-06-22..2026-06-29");
    }

    #[test]
    fn render_planner_prompt_lists_the_tag_glossary() {
        let g = TagGlossary::from_config(&ClassificationConfig {
            enabled: true,
            classify_previous: false,
            intents: vec!["request".into(), "escalation".into()],
            topics: vec!["wine".into()],
            categories: vec![],
        });
        let out = render_planner_prompt(
            "I:\n{{intent_definitions}}\nT:\n{{topic_definitions}}",
            "me@x.com",
            "2026-06-30",
            "q",
            &g,
        );
        assert!(out.contains("  request: "), "{out}");
        assert!(out.contains("  escalation\n"), "custom tag listed bare: {out}");
        assert!(out.contains("T:\n  wine"), "{out}");
    }

    #[test]
    fn parses_semantic_mode_and_ignores_unknown_modes() {
        let p = search(r#"{"query":"pido presupuesto a un proveedor","mode":"semantic"}"#);
        assert_eq!(p.mode.as_deref(), Some("semantic"));
        let call = p.into_tool_call();
        assert_eq!(call.function.arguments["mode"], "semantic");
        let p = search(r#"{"query":"x","mode":"fuzzy"}"#);
        assert_eq!(p.mode, None);
        let call = p.into_tool_call();
        assert!(call.function.arguments.get("mode").is_none());
    }

    #[test]
    fn mode_alone_is_not_a_filter() {
        assert_eq!(parse_plan(r#"{"mode":"semantic"}"#), Plan::Defer);
    }

    #[test]
    fn parses_self_sent_from_filter() {
        let p = search(r#"{"from":"me@acme.com","limit":3}"#);
        assert_eq!(p.from.as_deref(), Some("me@acme.com"));
        assert_eq!(p.limit, Some(3));
        assert!(p.to.is_none());
    }

    #[test]
    fn parses_recipient_and_query_fields() {
        let p = search(r#"{"to":"alex","query":"budget","subject":"Q3"}"#);
        assert_eq!(p.to.as_deref(), Some("alex"));
        assert_eq!(p.query.as_deref(), Some("budget"));
        assert_eq!(p.subject.as_deref(), Some("Q3"));
    }

    #[test]
    fn a_plan_with_a_real_filter_is_structural() {
        for json in [
            r#"{"from": "nadia"}"#,
            r#"{"to": "billing@acme.com"}"#,
            r#"{"subject": "invoice"}"#,
            r#"{"since": "2026-03-01"}"#,
            r#"{"intent": "introduction"}"#,
            r#"{"topic": "billing"}"#,
            r#"{"unread": true}"#,
        ] {
            match parse_plan(json) {
                Plan::Search(p) => assert!(p.has_structural_filter(), "expected structural: {json}"),
                Plan::Defer => panic!("expected a plan for {json}"),
            }
        }
    }

    #[test]
    fn a_keyword_only_plan_is_not_structural() {
        // "qué opina el equipo sobre el proyecto" plans as a bare keyword
        // search. Retrieval ranks that kind of question better than an FTS
        // filter does, so it must not pull the turn off the RAG route.
        for json in [
            r#"{"query": "proyecto"}"#,
            r#"{"query": "proyecto", "mode": "semantic"}"#,
        ] {
            match parse_plan(json) {
                Plan::Search(p) => assert!(!p.has_structural_filter(), "expected keyword-only: {json}"),
                Plan::Defer => panic!("expected a plan for {json}"),
            }
        }
    }

    #[test]
    fn dropping_the_guessed_tags_keeps_the_hard_filters() {
        // The planner adds a classifier tag the question never named
        // ("¿qué correos de BorgBase tengo sin leer?" → intent=notification,
        // while those invoices are tagged billing), and the search returns
        // nothing. On a turn the keyword list did not recognise, the tag is the
        // guessed half of the plan: drop it, keep from/unread.
        let Plan::Search(plan) = parse_plan(r#"{"from": "BorgBase", "unread": true, "intent": "notification"}"#) else {
            panic!("expected a plan");
        };
        let plan = plan.without_classifier_tags();
        assert_eq!(plan.intent, None);
        assert_eq!(plan.topic, None);
        assert_eq!(plan.from.as_deref(), Some("BorgBase"));
        assert_eq!(plan.unread, Some(true));
        assert!(plan.has_structural_filter());
    }

    #[test]
    fn a_plan_that_was_only_a_tag_stops_being_structural() {
        // "¿quién es Janos?" planned as a semantic query plus intent=question.
        // Without the tag there is no filter left, so the turn belongs to
        // retrieval — which is where it answered correctly before.
        let Plan::Search(plan) = parse_plan(r#"{"query": "Janos", "mode": "semantic", "intent": "question"}"#) else {
            panic!("expected a plan");
        };
        let plan = plan.without_classifier_tags();
        assert!(!plan.has_structural_filter());
        assert_eq!(plan.query.as_deref(), Some("Janos"));
    }

    #[test]
    fn explicit_defer_is_defer() {
        assert_eq!(parse_plan(r#"{"defer": true}"#), Plan::Defer);
    }

    #[test]
    fn non_search_asks_that_emit_defer_fall_through() {
        // The model is told to emit {"defer": true} for write/summarize/etc.
        assert_eq!(parse_plan("Sure! {\"defer\": true}"), Plan::Defer);
    }

    #[test]
    fn unparseable_output_defers_not_panics() {
        assert_eq!(parse_plan("I cannot help with that"), Plan::Defer);
        assert_eq!(parse_plan(""), Plan::Defer);
        assert_eq!(parse_plan("```\nnot json\n```"), Plan::Defer);
    }

    #[test]
    fn empty_filter_defers() {
        // No selective field (only a limit, or all nulls) → search_emails would
        // reject it, so defer rather than pre-seed a broken call.
        assert_eq!(parse_plan(r#"{"limit": 25}"#), Plan::Defer);
        assert_eq!(
            parse_plan(r#"{"query":null,"from":null,"to":null,"subject":null,"since":null,"until":null}"#),
            Plan::Defer
        );
    }

    #[test]
    fn strips_fences_and_leading_prose() {
        let p = search("Here you go:\n```json\n{\"from\":\"me@x.com\"}\n```");
        assert_eq!(p.from.as_deref(), Some("me@x.com"));
    }

    #[test]
    fn blank_string_fields_are_dropped() {
        // A model that fills "" for unused fields must not turn them into filters.
        let p = search(r#"{"from":"me@x.com","to":"  ","query":""}"#);
        assert_eq!(p.from.as_deref(), Some("me@x.com"));
        assert!(p.to.is_none());
        assert!(p.query.is_none());
    }

    #[test]
    fn limit_is_clamped_and_accepts_numeric_string() {
        // An address, so the bare-first-name widening does not apply here.
        assert_eq!(search(r#"{"from":"a@x.com","limit":1000}"#).limit, Some(25));
        assert_eq!(search(r#"{"from":"a@x.com","limit":0}"#).limit, Some(1));
        assert_eq!(search(r#"{"from":"a@x.com","limit":"3"}"#).limit, Some(3));
    }

    #[test]
    fn classification_filters_reach_the_tool_call() {
        // "últimos correos de prospects" → the planner maps the concept onto
        // the intent filter instead of a literal keyword.
        match parse_plan(r#"{"intent": "introduction", "limit": 5}"#) {
            Plan::Search(p) => {
                assert_eq!(p.intent.as_deref(), Some("introduction"));
                let call = (*p).into_tool_call();
                assert_eq!(call.function.arguments["intent"], "introduction");
                assert!(call.function.arguments.get("query").is_none());
            }
            other => panic!("expected search, got {other:?}"),
        }
        match parse_plan(r#"{"topic": "sales", "from": "acme"}"#) {
            Plan::Search(p) => assert_eq!(p.topic.as_deref(), Some("sales")),
            other => panic!("expected search, got {other:?}"),
        }
    }

    #[test]
    fn duplicated_sender_in_query_is_dropped() {
        match parse_plan(r#"{"from": "acmenews", "query": "acmenews", "limit": 5}"#) {
            Plan::Search(p) => {
                assert_eq!(p.query, None);
                assert_eq!(p.from.as_deref(), Some("acmenews"));
            }
            other => panic!("expected search, got {other:?}"),
        }
        // A genuine keyword next to the sender is kept.
        match parse_plan(r#"{"from": "acme", "query": "invoice"}"#) {
            Plan::Search(p) => assert_eq!(p.query.as_deref(), Some("invoice")),
            other => panic!("expected search, got {other:?}"),
        }
    }

    #[test]
    fn bare_first_name_lookup_asks_for_five_rows() {
        let limit = |json: &str| match parse_plan(json) {
            Plan::Search(p) => p.limit,
            other => panic!("expected search, got {other:?}"),
        };
        assert_eq!(limit(r#"{"from": "alex", "limit": 1}"#), Some(5));
        // An address, a full name or a domain identifies one sender: untouched.
        assert_eq!(limit(r#"{"from": "alex.smith@example.com", "limit": 1}"#), Some(1));
        assert_eq!(limit(r#"{"from": "ana de acme", "limit": 1}"#), Some(1));
        assert_eq!(limit(r#"{"from": "example.com", "limit": 1}"#), Some(1));
        // "the first email from alex" is one email, however many Alexes.
        assert_eq!(limit(r#"{"from": "alex", "order": "oldest", "limit": 1}"#), Some(1));
        // An explicit larger count is kept.
        assert_eq!(limit(r#"{"from": "alex", "limit": 10}"#), Some(10));
    }

    #[test]
    fn into_tool_call_drops_nulls_and_sets_defaults() {
        let call = SearchPlan {
            from: Some("me@x.com".into()),
            limit: Some(3),
            ..Default::default()
        }
        .into_tool_call();
        assert_eq!(call.function.name, "search_emails");
        let args = call.function.arguments.as_object().expect("object");
        assert_eq!(args.get("from").and_then(|v| v.as_str()), Some("me@x.com"));
        assert_eq!(args.get("limit").and_then(|v| v.as_i64()), Some(3));
        assert_eq!(args.get("include_bodies").and_then(|v| v.as_bool()), Some(true));
        assert!(!args.contains_key("to"), "null fields must be omitted");
        assert!(!args.contains_key("query"));
    }

    #[test]
    fn parses_oldest_order_for_first_email() {
        // "primer correo que envié a X" → to=X, order=oldest. The first email is
        // structurally unreachable without ascending sort, so this is the fix's
        // load-bearing parse.
        let p = search(r#"{"to":"acme","order":"oldest"}"#);
        assert_eq!(p.to.as_deref(), Some("acme"));
        assert_eq!(p.order.as_deref(), Some("oldest"));
    }

    #[test]
    fn unread_is_a_filter_on_its_own_and_reaches_the_tool_call() {
        let p = search(r#"{"unread": true, "order": "oldest"}"#);
        assert_eq!(p.unread, Some(true));
        let call = p.into_tool_call();
        assert_eq!(call.function.arguments["unread"], serde_json::json!(true));
        assert_eq!(call.function.arguments["limit"], serde_json::json!(1));
        // `unread: false` is not a filter: alone it leaves nothing to search.
        assert_eq!(parse_plan(r#"{"unread": false}"#), Plan::Defer);
        assert!(!search(r#"{"from": "a", "unread": false}"#)
            .into_tool_call()
            .function
            .arguments
            .as_object()
            .expect("object")
            .contains_key("unread"));
    }

    #[test]
    fn unknown_order_is_dropped() {
        assert_eq!(search(r#"{"from":"a","order":"sideways"}"#).order, None);
        // Case-insensitive accept.
        assert_eq!(
            search(r#"{"from":"a","order":"Oldest"}"#).order.as_deref(),
            Some("oldest")
        );
    }

    #[test]
    fn oldest_emits_order_and_defaults_limit_to_one() {
        let call = SearchPlan {
            to: Some("acme".into()),
            order: Some("oldest".into()),
            ..Default::default()
        }
        .into_tool_call();
        let args = call.function.arguments.as_object().expect("object");
        assert_eq!(args.get("order").and_then(|v| v.as_str()), Some("oldest"));
        assert_eq!(args.get("to").and_then(|v| v.as_str()), Some("acme"));
        assert_eq!(
            args.get("limit").and_then(|v| v.as_i64()),
            Some(1),
            "oldest + no explicit limit → THE single first email"
        );
    }

    #[test]
    fn newest_omits_order_arg() {
        let call = SearchPlan {
            from: Some("me@x.com".into()),
            ..Default::default()
        }
        .into_tool_call();
        let args = call.function.arguments.as_object().expect("object");
        assert!(!args.contains_key("order"), "default newest must not emit an order arg");
    }

    #[test]
    fn into_tool_call_defaults_limit_to_25() {
        let call = SearchPlan {
            query: Some("invoices".into()),
            ..Default::default()
        }
        .into_tool_call();
        let args = call.function.arguments.as_object().expect("object");
        assert_eq!(args.get("limit").and_then(|v| v.as_i64()), Some(25));
    }

    #[test]
    fn render_substitutes_all_per_turn_vars() {
        let out = render_planner_prompt(
            "addr={{user_email}} day={{today}} q={{query}}",
            "me@x.com",
            "2026-06-17",
            "emails I sent",
            &TagGlossary::defaults(),
        );
        assert_eq!(out, "addr=me@x.com day=2026-06-17 q=emails I sent");
    }
}
