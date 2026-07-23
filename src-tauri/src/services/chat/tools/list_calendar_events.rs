//! Chat tool: list the user's calendar events from the locally-synced
//! calendar (Gmail / Outlook accounts). Deterministic — reads the DB, never
//! the network — so answers reflect exactly what the calendar view shows.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolCtx, ToolError, ToolOutput};
use crate::db::Database;
use crate::services::chat::parse_iso_date_to_ts;

pub struct ListCalendarEventsTool;

const DEFAULT_DAYS: i64 = 7;
const MAX_DAYS: i64 = 60;
const MAX_EVENTS: usize = 30;

/// Pure window resolution: explicit `since`/`until` ISO dates win; otherwise
/// `[now, now + days]` with `days` clamped to `[1, 60]` (default 7).
pub(crate) fn resolve_window(args: &Value, now: i64) -> (i64, i64) {
    let since = args
        .get("since")
        .and_then(|v| v.as_str())
        .and_then(parse_iso_date_to_ts);
    let until = args
        .get("until")
        .and_then(|v| v.as_str())
        .and_then(parse_iso_date_to_ts);
    match (since, until) {
        (Some(start), Some(end)) if end > start => (start, end),
        (Some(start), _) => (start, start + DEFAULT_DAYS * 86_400),
        _ => {
            let days = args
                .get("days")
                .and_then(|v| v.as_i64())
                .unwrap_or(DEFAULT_DAYS)
                .clamp(1, MAX_DAYS);
            (now, now + days * 86_400)
        }
    }
}

/// One event as a compact, model-friendly line in local time.
fn format_event_line(event: &crate::models::CalendarEvent) -> String {
    use chrono::TimeZone;
    let when = if event.is_all_day {
        match chrono::Local.timestamp_opt(event.start_time, 0).single() {
            Some(dt) => format!("{} all-day", dt.format("%Y-%m-%d")),
            None => "unknown-date".to_string(),
        }
    } else {
        let start = chrono::Local.timestamp_opt(event.start_time, 0).single();
        let end = chrono::Local.timestamp_opt(event.end_time, 0).single();
        match (start, end) {
            (Some(s), Some(e)) => format!("{}\u{2013}{}", s.format("%Y-%m-%d %H:%M"), e.format("%H:%M")),
            _ => "unknown-time".to_string(),
        }
    };
    let title = if event.title.is_empty() {
        "(untitled)"
    } else {
        &event.title
    };
    let mut line = format!("- {when} \"{title}\"");
    if !event.organizer.is_empty() {
        line.push_str(&format!(" organizer={}", event.organizer));
    }
    if let Some(platform) = &event.meeting_platform {
        line.push_str(&format!(" meeting={platform}"));
    }
    if event.recurring_event_id.is_some() {
        line.push_str(" recurring=yes");
    }
    line.push('\n');
    line
}

#[async_trait]
impl Tool for ListCalendarEventsTool {
    fn name(&self) -> &'static str {
        "list_calendar_events"
    }

    fn description(&self) -> &'static str {
        "List the user's calendar events (meetings) from their connected calendar. Use for 'what meetings do I have', 'my agenda this week', 'mis reuniones', 'mi agenda'. Times are local."
    }

    fn prompt_summary(&self) -> &'static str {
        "list the user's calendar events (meetings) for a date range."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "since": { "type": "string", "description": "ISO date (YYYY-MM-DD) — start of the range. Defaults to now." },
                "until": { "type": "string", "description": "ISO date (YYYY-MM-DD) — exclusive end of the range." },
                "days": { "type": "integer", "description": "Days ahead from now when since/until are omitted (default 7, max 60)." }
            },
            "required": []
        })
    }

    fn is_available(&self, db: &Database) -> bool {
        // Only advertise the tool when at least one connected account has
        // calendar integration enabled (per-account opt-in on a Gmail /
        // Outlook account) — setups without it never see the tool, so it
        // doesn't tax their prompt budget.
        db.list_accounts()
            .map(|accounts| {
                accounts.iter().any(|a| {
                    a.enabled
                        && crate::sync::calendar_provider::provider_supports_calendar(&a.provider)
                        && db.calendar_enabled(&a.id).unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }

    async fn execute(&self, ctx: &ToolCtx<'_>, args: Value) -> Result<ToolOutput, ToolError> {
        // Errors surface as tool-result text (the registry's convention — see
        // list_pending_tasks) so the model can react instead of the turn dying.
        let account = match ctx.db.get_account(ctx.account_id) {
            Ok(Some(account)) => account,
            Ok(None) => {
                return Ok(ToolOutput::text(format!(
                    "Calendar error: account '{}' not found.",
                    ctx.account_id
                )))
            }
            Err(e) => return Ok(ToolOutput::text(format!("Calendar error: {e}"))),
        };
        if !crate::sync::calendar_provider::provider_supports_calendar(&account.provider)
            || !ctx.db.calendar_enabled(&account.id).unwrap_or(false)
        {
            return Ok(ToolOutput::text(
                "This account has no calendar integration enabled (enable it in Settings → Calendar; only Gmail and Outlook calendars are supported)."
                    .to_string(),
            ));
        }
        let (start, end) = resolve_window(&args, chrono::Utc::now().timestamp());
        let events = match ctx.db.list_calendar_events(ctx.account_id, start, end) {
            Ok(events) => events,
            Err(e) => return Ok(ToolOutput::text(format!("Calendar error: {e}"))),
        };
        if events.is_empty() {
            return Ok(ToolOutput::text("No calendar events in this period.".to_string()));
        }
        let mut out = String::new();
        for event in events.iter().take(MAX_EVENTS) {
            out.push_str(&format_event_line(event));
        }
        if events.len() > MAX_EVENTS {
            out.push_str(&format!("(+{} more events not shown)\n", events.len() - MAX_EVENTS));
        }
        Ok(ToolOutput::text(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CalendarEvent;

    const NOW: i64 = 1_785_000_000;

    // ── resolve_window (pure) ──────────────────────────────────────────────

    #[test]
    fn window_defaults_to_seven_days_from_now() {
        assert_eq!(resolve_window(&json!({}), NOW), (NOW, NOW + 7 * 86_400));
    }

    #[test]
    fn window_days_is_clamped() {
        assert_eq!(resolve_window(&json!({"days": 0}), NOW), (NOW, NOW + 86_400));
        assert_eq!(resolve_window(&json!({"days": 500}), NOW), (NOW, NOW + 60 * 86_400));
    }

    #[test]
    fn window_uses_explicit_since_until() {
        let (start, end) = resolve_window(&json!({"since": "2026-07-27", "until": "2026-08-03"}), NOW);
        assert_eq!(end - start, 7 * 86_400);
        assert!(start > 0);
    }

    #[test]
    fn window_since_without_until_spans_a_week() {
        let (start, end) = resolve_window(&json!({"since": "2026-07-27"}), NOW);
        assert_eq!(end - start, 7 * 86_400);
    }

    #[test]
    fn window_inverted_until_falls_back_to_week_after_since() {
        let (start, end) = resolve_window(&json!({"since": "2026-07-27", "until": "2026-07-20"}), NOW);
        assert_eq!(end - start, 7 * 86_400);
    }

    // ── execute (against the in-memory DB) ─────────────────────────────────

    fn event(account_id: &str, id: &str, start: i64) -> CalendarEvent {
        CalendarEvent {
            id: format!("{account_id}:{id}"),
            account_id: account_id.to_string(),
            provider_event_id: id.to_string(),
            calendar_id: "primary".to_string(),
            title: format!("Meeting {id}"),
            description: String::new(),
            location: String::new(),
            start_time: start,
            end_time: start + 1_800,
            is_all_day: false,
            timezone: String::new(),
            organizer: "boss@example.com".to_string(),
            attendees: Vec::new(),
            meeting_link: Some("https://meet.google.com/abc-defg-hij".to_string()),
            meeting_platform: Some("meet".to_string()),
            status: "confirmed".to_string(),
            html_link: None,
            notified_at: None,
            recurring_event_id: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn seed_gmail_account(db: &Database, id: &str) {
        db.insert_account(&crate::models::Account {
            id: id.to_string(),
            provider: "gmail".to_string(),
            email: format!("{id}@example.com"),
            name: id.to_string(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
        })
        .expect("seed account");
    }

    fn disable_calendar(db: &Database, id: &str) {
        db.set_preference(&crate::db::calendar::calendar_enabled_pref_key(id), "false")
            .expect("disable calendar pref");
    }

    #[tokio::test]
    async fn lists_upcoming_events_with_organizer_and_platform() {
        let db = std::sync::Arc::new(Database::new_for_testing().expect("db"));
        seed_gmail_account(&db, "acc1");
        let now = chrono::Utc::now().timestamp();
        db.upsert_calendar_events(&[event("acc1", "ev1", now + 3_600)])
            .expect("seed");

        let ctx = ToolCtx {
            db: &db,
            account_id: "acc1",
            categories: &[],
        };
        let out = ListCalendarEventsTool.execute(&ctx, json!({})).await.expect("execute");
        let text = out.text;
        assert!(text.contains("Meeting ev1"), "got: {text}");
        assert!(text.contains("organizer=boss@example.com"), "got: {text}");
        assert!(text.contains("meeting=meet"), "got: {text}");
    }

    #[tokio::test]
    async fn empty_calendar_yields_friendly_notice() {
        let db = std::sync::Arc::new(Database::new_for_testing().expect("db"));
        seed_gmail_account(&db, "acc1");
        let ctx = ToolCtx {
            db: &db,
            account_id: "acc1",
            categories: &[],
        };
        let out = ListCalendarEventsTool.execute(&ctx, json!({})).await.expect("execute");
        assert_eq!(out.text, "No calendar events in this period.");
    }

    #[tokio::test]
    async fn imap_account_gets_no_integration_notice() {
        let db = std::sync::Arc::new(Database::new_for_testing().expect("db"));
        db.insert_account(&crate::models::Account {
            id: "imap1".to_string(),
            provider: "imap".to_string(),
            email: "imap1@example.com".to_string(),
            name: "imap1".to_string(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
        })
        .expect("seed account");
        let ctx = ToolCtx {
            db: &db,
            account_id: "imap1",
            categories: &[],
        };
        let out = ListCalendarEventsTool.execute(&ctx, json!({})).await.expect("execute");
        assert!(out.text.contains("no calendar integration"));
    }

    #[test]
    fn availability_requires_a_calendar_enabled_account() {
        let db = Database::new_for_testing().expect("db");
        assert!(!ListCalendarEventsTool.is_available(&db), "no accounts → hidden");
        seed_gmail_account(&db, "acc1");
        assert!(
            ListCalendarEventsTool.is_available(&db),
            "capable account → visible (integration is on by default)"
        );
        disable_calendar(&db, "acc1");
        assert!(
            !ListCalendarEventsTool.is_available(&db),
            "integration switched off → hidden"
        );
    }

    #[tokio::test]
    async fn account_with_calendar_disabled_gets_no_integration_notice() {
        let db = std::sync::Arc::new(Database::new_for_testing().expect("db"));
        seed_gmail_account(&db, "acc1");
        // Calendar-capable provider, but integration switched off (by the user
        // or the permission-denied auto-disable) — the tool must not read
        // events for it.
        disable_calendar(&db, "acc1");
        let now = chrono::Utc::now().timestamp();
        db.upsert_calendar_events(&[event("acc1", "ev1", now + 3_600)])
            .expect("seed");
        let ctx = ToolCtx {
            db: &db,
            account_id: "acc1",
            categories: &[],
        };
        let out = ListCalendarEventsTool.execute(&ctx, json!({})).await.expect("execute");
        assert!(out.text.contains("no calendar integration"), "got: {}", out.text);
        assert!(!out.text.contains("Meeting ev1"), "must not leak events: {}", out.text);
    }
}
