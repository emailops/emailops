//! Calendar sync: full-window fetch → upsert → stale sweep.
//!
//! Planner (`sync_window`, `plan_event_rows`) is pure; the executor
//! (`sync_account_calendar`) does the I/O against the [`CalendarProvider`]
//! seam and the DB. No sync tokens in v1 — see `sync/calendar_provider.rs`.

use crate::db::calendars::calendar_row_id;
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Calendar, CalendarEvent};
use crate::services::calendar::meeting_link::extract_meeting_link;
use crate::sync::calendar_provider::{CalendarProvider, ProviderCalendar, ProviderCalendarEvent};

/// Rolling sync window: 30 days back (recent context) / 90 days forward
/// (planning horizon). ~120 days of expanded instances is a few hundred
/// events for a busy calendar — cheap to re-fetch on every cycle.
pub const CALENDAR_WINDOW_PAST_SECS: i64 = 30 * 86_400;
pub const CALENDAR_WINDOW_FUTURE_SECS: i64 = 90 * 86_400;

/// The `[start, end)` window to sync around `now`.
pub fn sync_window(now: i64) -> (i64, i64) {
    (now - CALENDAR_WINDOW_PAST_SECS, now + CALENDAR_WINDOW_FUTURE_SECS)
}

/// Pure planner: provider calendars → DB rows. `is_visible` is seeded from the
/// provider's own "shown in my UI" flag, which only matters on first insert —
/// the upsert never overwrites it afterwards, so the user's toggle wins from
/// then on.
pub fn plan_calendar_rows(account_id: &str, fetched: &[ProviderCalendar], now: i64) -> Vec<Calendar> {
    fetched
        .iter()
        .enumerate()
        .map(|(index, c)| Calendar {
            id: calendar_row_id(account_id, &c.provider_calendar_id),
            account_id: account_id.to_string(),
            provider_calendar_id: c.provider_calendar_id.clone(),
            name: c.name.clone(),
            color: c.color.clone(),
            is_primary: c.is_primary,
            access_role: c.access_role.clone(),
            is_visible: c.selected,
            sort_order: index as i64,
            created_at: now,
            updated_at: now,
        })
        .collect()
}

/// Pure planner: provider events → DB rows for one calendar. Cancelled
/// instances are dropped; the stale sweep removes their previously-synced
/// rows. Meeting links are extracted here (structured URLs first, then
/// location/description text).
pub fn plan_event_rows(
    account_id: &str,
    calendar_id: &str,
    fetched: &[ProviderCalendarEvent],
    now: i64,
) -> Vec<CalendarEvent> {
    fetched
        .iter()
        .filter(|e| e.status != "cancelled")
        .map(|e| {
            let link = extract_meeting_link(&e.structured_meeting_urls, &e.location, &e.description);
            CalendarEvent {
                // The calendar segment is load-bearing: providers reuse one
                // event id across every calendar an event is visible in.
                id: format!("{account_id}:{calendar_id}:{}", e.provider_event_id),
                account_id: account_id.to_string(),
                provider_event_id: e.provider_event_id.clone(),
                calendar_id: calendar_id.to_string(),
                title: e.title.clone(),
                description: e.description.clone(),
                location: e.location.clone(),
                start_time: e.start_time,
                end_time: e.end_time,
                is_all_day: e.is_all_day,
                timezone: e.timezone.clone(),
                organizer: e.organizer.clone(),
                attendees: e.attendees.clone(),
                meeting_link: link.as_ref().map(|l| l.url.clone()),
                meeting_platform: link.as_ref().map(|l| l.platform.to_string()),
                status: e.status.clone(),
                html_link: e.html_link.clone(),
                notified_at: None, // preserved on conflict update — see upsert
                recurring_event_id: e.recurring_event_id.clone(),
                created_at: now,
                updated_at: now,
            }
        })
        .collect()
}

/// Prefer an account-wide auth/permission failure over an incidental one, so
/// the scheduler still sees the signal it auto-disables the integration on.
fn most_actionable(errors: Vec<AppError>) -> Option<AppError> {
    let mut fallback = None;
    for error in errors {
        match error {
            e @ (AppError::CalendarPermissionDenied { .. } | AppError::NeedsReauth { .. }) => return Some(e),
            other => fallback.get_or_insert(other),
        };
    }
    fallback
}

/// One full sync cycle for one account, across every calendar it can see.
/// Returns the number of events stored.
///
/// A single calendar that fails (sharing revoked, provider hiccup on one
/// shared calendar) is logged and skipped: its stored events are left exactly
/// as they were rather than swept, and the other calendars still sync. Only a
/// failure that affects *every* calendar propagates — that is the account-wide
/// problem the scheduler acts on.
pub async fn sync_account_calendar(
    db: &Database,
    account_id: &str,
    provider: &dyn CalendarProvider,
    now: i64,
) -> Result<u32> {
    let (window_start, window_end) = sync_window(now);

    // The calendar list is the one fetch that must succeed: everything below
    // is scoped by it, and treating a failed list as "no calendars" would
    // delete the entire local mirror.
    let fetched_calendars = provider.list_calendars().await?;
    db.upsert_calendars(&plan_calendar_rows(account_id, &fetched_calendars, now))?;
    let live_ids: Vec<String> = fetched_calendars
        .iter()
        .map(|c| c.provider_calendar_id.clone())
        .collect();
    db.delete_calendars_not_in(account_id, &live_ids)?;
    db.delete_calendar_events_not_in(account_id, &live_ids)?;

    let mut rows = Vec::new();
    let mut synced_ids: Vec<String> = Vec::new();
    let mut failures: Vec<AppError> = Vec::new();
    for calendar in &fetched_calendars {
        match provider
            .list_events(&calendar.provider_calendar_id, window_start, window_end)
            .await
        {
            Ok(fetched) => {
                rows.extend(plan_event_rows(
                    account_id,
                    &calendar.provider_calendar_id,
                    &fetched,
                    now,
                ));
                synced_ids.push(calendar.provider_calendar_id.clone());
            }
            Err(e) => {
                crate::services::logger::log(
                    "error",
                    "sync",
                    format!("calendar: skipping \"{}\" this cycle: {e}", calendar.name),
                );
                failures.push(e);
            }
        }
    }

    // Every calendar failed → the cause is account-wide (expired token, scope
    // never granted), not a per-calendar quirk. Surface it so the scheduler
    // can re-auth or auto-disable instead of silently reporting 0 events.
    if synced_ids.is_empty() && !failures.is_empty() {
        if let Some(error) = most_actionable(failures) {
            return Err(error);
        }
    }

    db.upsert_calendar_events(&rows)?;
    db.delete_stale_calendar_events(account_id, &synced_ids, window_start, window_end, now)?;
    db.set_calendar_sync_token(account_id, None, now)?;
    Ok(rows.len() as u32)
}

/// Build the calendar client for an OAuth account. Errors for providers
/// without a calendar (IMAP) — callers gate on `provider_supports_calendar`.
pub fn build_calendar_provider(account_id: &str, provider_name: &str) -> Result<Box<dyn CalendarProvider>> {
    // Reject unsupported providers before touching the keychain.
    if !crate::sync::calendar_provider::provider_supports_calendar(provider_name) {
        return Err(AppError::InvalidInput(format!(
            "Provider {provider_name} has no calendar support"
        )));
    }
    let tokens = crate::services::accounts::get_tokens(account_id)?;
    match provider_name {
        "gmail" => Ok(Box::new(crate::sync::gmail_calendar::GoogleCalendarClient::new(
            tokens.access_token,
            tokens.refresh_token,
            Some(account_id.to_string()),
        ))),
        _ => Ok(Box::new(crate::sync::outlook_calendar::OutlookCalendarClient::new(
            tokens.access_token,
            tokens.refresh_token,
            Some(account_id.to_string()),
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::calendar_provider::FakeCalendarProvider;

    fn provider_event(id: &str, start: i64, end: i64) -> ProviderCalendarEvent {
        ProviderCalendarEvent {
            provider_event_id: id.to_string(),
            title: format!("Event {id}"),
            description: String::new(),
            location: String::new(),
            start_time: start,
            end_time: end,
            is_all_day: false,
            timezone: "UTC".to_string(),
            organizer: "org@example.com".to_string(),
            attendees: vec![crate::models::CalendarAttendee {
                email: "a@example.com".to_string(),
                response: "accepted".to_string(),
            }],
            structured_meeting_urls: Vec::new(),
            status: "confirmed".to_string(),
            html_link: None,
            recurring_event_id: None,
        }
    }

    // ── sync_window ────────────────────────────────────────────────────────

    #[test]
    fn window_spans_30_days_back_and_90_forward() {
        let (start, end) = sync_window(1_000_000_000);
        assert_eq!(start, 1_000_000_000 - 30 * 86_400);
        assert_eq!(end, 1_000_000_000 + 90 * 86_400);
    }

    // ── plan_event_rows ────────────────────────────────────────────────────

    #[test]
    fn plans_rows_with_stable_ids_and_run_timestamps() {
        let rows = plan_event_rows("acc1", "primary", &[provider_event("ev1", 100, 200)], 5_000);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "acc1:primary:ev1");
        assert_eq!(rows[0].account_id, "acc1");
        assert_eq!(rows[0].updated_at, 5_000);
        assert_eq!(rows[0].notified_at, None);
    }

    #[test]
    fn cancelled_events_are_dropped_from_the_plan() {
        let mut cancelled = provider_event("gone", 100, 200);
        cancelled.status = "cancelled".to_string();
        let rows = plan_event_rows("acc1", "primary", &[cancelled, provider_event("kept", 300, 400)], 5_000);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider_event_id, "kept");
    }

    #[test]
    fn plan_extracts_meeting_link_from_structured_urls() {
        let mut event = provider_event("ev1", 100, 200);
        event.structured_meeting_urls = vec!["https://meet.google.com/abc-defg-hij".to_string()];
        let rows = plan_event_rows("acc1", "primary", &[event], 5_000);
        assert_eq!(
            rows[0].meeting_link.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(rows[0].meeting_platform.as_deref(), Some("meet"));
    }

    #[test]
    fn plan_extracts_meeting_link_from_location_text() {
        let mut event = provider_event("ev1", 100, 200);
        event.location = "https://acme.webex.com/meet/jdoe".to_string();
        let rows = plan_event_rows("acc1", "primary", &[event], 5_000);
        assert_eq!(rows[0].meeting_platform.as_deref(), Some("webex"));
    }

    #[test]
    fn plan_leaves_meeting_link_none_when_nothing_found() {
        let rows = plan_event_rows("acc1", "primary", &[provider_event("ev1", 100, 200)], 5_000);
        assert_eq!(rows[0].meeting_link, None);
        assert_eq!(rows[0].meeting_platform, None);
    }

    // ── sync_account_calendar (executor against fakes) ─────────────────────

    fn test_db() -> Database {
        let db = Database::new_for_testing().expect("test db");
        db.seed_test_account("acc1");
        db
    }

    #[tokio::test]
    async fn full_cycle_stores_fetched_events() {
        let db = test_db();
        let now = 1_000_000;
        let provider = FakeCalendarProvider::with_events(vec![
            provider_event("ev1", now + 3_600, now + 7_200),
            provider_event("ev2", now + 10_000, now + 13_600),
        ]);

        let stored = sync_account_calendar(&db, "acc1", &provider, now).await.expect("sync");

        assert_eq!(stored, 2);
        let events = db.list_calendar_events("acc1", now, now + 100_000).expect("list");
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn resync_removes_events_deleted_upstream_but_keeps_notified_marker() {
        let db = test_db();
        let first_run = 1_000_000;
        let provider = FakeCalendarProvider::with_events(vec![
            provider_event("keeps", first_run + 3_600, first_run + 7_200),
            provider_event("deleted-upstream", first_run + 9_000, first_run + 9_600),
        ]);
        sync_account_calendar(&db, "acc1", &provider, first_run)
            .await
            .expect("first sync");
        db.mark_calendar_event_notified("acc1:primary:keeps", first_run)
            .expect("mark");

        // Second run: upstream deleted one event.
        let second_run = first_run + 300;
        let provider =
            FakeCalendarProvider::with_events(vec![provider_event("keeps", first_run + 3_600, first_run + 7_200)]);
        sync_account_calendar(&db, "acc1", &provider, second_run)
            .await
            .expect("second sync");

        let events = db.list_calendar_events("acc1", 0, i64::MAX).expect("list");
        assert_eq!(events.len(), 1, "upstream-deleted event must disappear locally");
        assert_eq!(events[0].provider_event_id, "keeps");
        assert_eq!(
            events[0].notified_at,
            Some(first_run),
            "notified marker survives resync"
        );
    }

    #[tokio::test]
    async fn provider_error_propagates_and_leaves_existing_rows_untouched() {
        let db = test_db();
        let now = 1_000_000;
        let provider = FakeCalendarProvider::with_events(vec![provider_event("ev1", now + 100, now + 200)]);
        sync_account_calendar(&db, "acc1", &provider, now)
            .await
            .expect("seed sync");

        let failing = FakeCalendarProvider::failing("[UNAVAILABLE] calendar backend down");
        let result = sync_account_calendar(&db, "acc1", &failing, now + 300).await;

        assert!(result.is_err(), "provider failure must propagate, never be swallowed");
        let events = db.list_calendar_events("acc1", 0, i64::MAX).expect("list");
        assert_eq!(events.len(), 1, "a failed fetch must not wipe local events");
    }

    // ── multi-calendar ─────────────────────────────────────────────────────

    fn multi_calendar_provider(now: i64) -> FakeCalendarProvider {
        FakeCalendarProvider::with_calendars(vec![
            (
                FakeCalendarProvider::primary_calendar(),
                vec![provider_event("mine", now + 3_600, now + 7_200)],
            ),
            (
                FakeCalendarProvider::calendar("team@group.calendar.google.com", "Team", "#33b679"),
                vec![provider_event("theirs", now + 10_000, now + 13_600)],
            ),
        ])
    }

    #[tokio::test]
    async fn sync_stores_the_calendar_registry() {
        let db = test_db();
        let now = 1_000_000;

        sync_account_calendar(&db, "acc1", &multi_calendar_provider(now), now)
            .await
            .expect("sync");

        let calendars = db.list_calendars("acc1").expect("list calendars");
        assert_eq!(calendars.len(), 2);
        assert_eq!(calendars[0].provider_calendar_id, "primary");
        assert_eq!(calendars[1].name, "Team");
        assert_eq!(calendars[1].color, "#33b679");
    }

    #[tokio::test]
    async fn sync_stores_events_from_every_calendar_tagged_with_their_own() {
        let db = test_db();
        let now = 1_000_000;

        let stored = sync_account_calendar(&db, "acc1", &multi_calendar_provider(now), now)
            .await
            .expect("sync");

        assert_eq!(stored, 2);
        let events = db.list_calendar_events("acc1", 0, i64::MAX).expect("list");
        let mut tagged: Vec<(&str, &str)> = events
            .iter()
            .map(|e| (e.provider_event_id.as_str(), e.calendar_id.as_str()))
            .collect();
        tagged.sort_unstable();
        assert_eq!(
            tagged,
            vec![("mine", "primary"), ("theirs", "team@group.calendar.google.com")]
        );
    }

    #[tokio::test]
    async fn a_calendar_hidden_by_the_user_still_syncs() {
        // Visibility is a render-time filter, so re-showing a calendar is
        // instant instead of waiting for the next 5-minute poll.
        let db = test_db();
        let now = 1_000_000;
        sync_account_calendar(&db, "acc1", &multi_calendar_provider(now), now)
            .await
            .expect("first sync");
        db.set_calendar_visible("acc1", "team@group.calendar.google.com", false)
            .expect("hide");

        sync_account_calendar(&db, "acc1", &multi_calendar_provider(now), now + 300)
            .await
            .expect("second sync");

        let events = db.list_calendar_events("acc1", 0, i64::MAX).expect("list");
        assert_eq!(events.len(), 2, "hidden calendars keep syncing in the background");
        assert!(!db
            .list_calendars("acc1")
            .expect("list")
            .iter()
            .any(|c| c.provider_calendar_id == "team@group.calendar.google.com" && c.is_visible));
    }

    #[tokio::test]
    async fn one_failing_calendar_does_not_sweep_its_events_or_block_the_others() {
        let db = test_db();
        let now = 1_000_000;
        sync_account_calendar(&db, "acc1", &multi_calendar_provider(now), now)
            .await
            .expect("seed sync");

        // Next cycle: the shared calendar errors, the primary still works.
        let flaky = multi_calendar_provider(now).failing_calendar("team@group.calendar.google.com");
        let stored = sync_account_calendar(&db, "acc1", &flaky, now + 300)
            .await
            .expect("a single bad calendar must not fail the whole sync");

        assert_eq!(stored, 1, "only the healthy calendar contributed rows");
        let events = db.list_calendar_events("acc1", 0, i64::MAX).expect("list");
        assert_eq!(
            events.len(),
            2,
            "the failing calendar's events are left alone, not swept"
        );
    }

    #[tokio::test]
    async fn when_every_calendar_fails_the_permission_error_propagates() {
        // The scheduler auto-disables the integration on
        // CalendarPermissionDenied — swallowing it would strand the user with
        // a calendar that silently reports zero events forever.
        let db = test_db();
        let now = 1_000_000;
        let provider = multi_calendar_provider(now)
            .failing_calendar("primary")
            .failing_calendar("team@group.calendar.google.com");

        let result = sync_account_calendar(&db, "acc1", &provider, now).await;

        assert!(result.is_err(), "an account-wide failure must propagate");
    }

    #[tokio::test]
    async fn losing_access_to_a_calendar_drops_it_and_its_events() {
        let db = test_db();
        let now = 1_000_000;
        sync_account_calendar(&db, "acc1", &multi_calendar_provider(now), now)
            .await
            .expect("seed sync");

        // The shared calendar disappears from the provider's list entirely.
        let only_primary = FakeCalendarProvider::with_events(vec![provider_event("mine", now + 3_600, now + 7_200)]);
        sync_account_calendar(&db, "acc1", &only_primary, now + 300)
            .await
            .expect("sync");

        assert_eq!(db.list_calendars("acc1").expect("list").len(), 1);
        let events = db.list_calendar_events("acc1", 0, i64::MAX).expect("list");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].calendar_id, "primary");
    }

    #[tokio::test]
    async fn the_same_event_in_two_calendars_is_kept_as_two_rows() {
        // Google hands out one id per meeting, repeated in every calendar it
        // shows up in. Both copies must survive so hiding one calendar hides
        // exactly one of them.
        let db = test_db();
        let now = 1_000_000;
        let provider = FakeCalendarProvider::with_calendars(vec![
            (
                FakeCalendarProvider::primary_calendar(),
                vec![provider_event("shared-ev", now + 3_600, now + 7_200)],
            ),
            (
                FakeCalendarProvider::calendar("team@group.calendar.google.com", "Team", "#33b679"),
                vec![provider_event("shared-ev", now + 3_600, now + 7_200)],
            ),
        ]);

        let stored = sync_account_calendar(&db, "acc1", &provider, now).await.expect("sync");

        assert_eq!(stored, 2);
        assert_eq!(db.list_calendar_events("acc1", 0, i64::MAX).expect("list").len(), 2);
    }

    #[tokio::test]
    async fn a_calendar_the_provider_hides_starts_hidden() {
        let db = test_db();
        let now = 1_000_000;
        let mut unselected =
            FakeCalendarProvider::calendar("holidays@group.v.calendar.google.com", "Holidays", "#0b8043");
        unselected.selected = false;
        let provider = FakeCalendarProvider::with_calendars(vec![
            (FakeCalendarProvider::primary_calendar(), vec![]),
            (unselected, vec![]),
        ]);

        sync_account_calendar(&db, "acc1", &provider, now).await.expect("sync");

        let calendars = db.list_calendars("acc1").expect("list");
        let holidays = calendars
            .iter()
            .find(|c| c.provider_calendar_id == "holidays@group.v.calendar.google.com")
            .expect("holidays calendar");
        assert!(
            !holidays.is_visible,
            "a calendar the user already hid in the provider's UI should not appear unbidden"
        );
    }

    #[tokio::test]
    async fn sync_records_last_sync_timestamp() {
        let db = test_db();
        let provider = FakeCalendarProvider::with_events(vec![]);
        sync_account_calendar(&db, "acc1", &provider, 42_000)
            .await
            .expect("sync");

        let last: Option<i64> = db
            .connection()
            .query_row(
                "SELECT last_sync_at FROM calendar_sync_state WHERE account_id = 'acc1'",
                [],
                |r| r.get(0),
            )
            .expect("read state");
        assert_eq!(last, Some(42_000));
    }

    #[test]
    fn build_provider_rejects_imap() {
        let Err(error) = build_calendar_provider("acc-imap", "imap") else {
            panic!("imap has no calendar and must be rejected");
        };
        assert!(error.to_string().contains("no calendar support"));
    }
}
