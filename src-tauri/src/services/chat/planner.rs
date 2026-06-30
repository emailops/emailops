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

/// The planner's decision for a turn.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// Pre-seed `search_emails` with these filters as the turn's round-0 call.
    Search(SearchPlan),
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
    pub since: Option<String>,
    pub until: Option<String>,
    pub limit: Option<i64>,
    /// `"oldest"` to surface the FIRST matching email ("primer correo"),
    /// `"newest"` (or unset) for the default most-recent-first.
    pub order: Option<String>,
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
        put("since", self.since);
        put("until", self.until);
        if oldest {
            args.insert("order".to_string(), serde_json::Value::String("oldest".to_string()));
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
        since: str_field("since"),
        until: str_field("until"),
        limit,
        order,
    };
    if plan.is_empty() {
        return Plan::Defer;
    }
    Plan::Search(plan)
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
pub(crate) fn render_planner_prompt(template: &str, user_email: &str, today: &str, query: &str) -> String {
    let mut vars = std::collections::HashMap::new();
    vars.insert("user_email", user_email.to_string());
    vars.insert("today", today.to_string());
    vars.insert("query", query.to_string());
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
) -> Plan {
    let prompt = render_planner_prompt(template, user_email, today, query);
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

    fn search(text: &str) -> SearchPlan {
        match parse_plan(text) {
            Plan::Search(p) => p,
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
        let out = render_planner_prompt(tmpl, "me@x.com", "2026-06-30", "this week");
        assert_eq!(out, "this=2026-06-29..2026-07-06 last=2026-06-22..2026-06-29");
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
        assert_eq!(search(r#"{"from":"a","limit":1000}"#).limit, Some(25));
        assert_eq!(search(r#"{"from":"a","limit":0}"#).limit, Some(1));
        assert_eq!(search(r#"{"from":"a","limit":"3"}"#).limit, Some(3));
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
        );
        assert_eq!(out, "addr=me@x.com day=2026-06-17 q=emails I sent");
    }
}
