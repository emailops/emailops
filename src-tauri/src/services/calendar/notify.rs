//! Upcoming-meeting notification planning. The planner is pure; the executor
//! loop lives in `services::sync_scheduler` and is deliberately thin: send the
//! OS notification, emit the `meeting-reminder` event (the in-app banner with
//! the Join button), mark the row notified.

use crate::db::Database;
use crate::models::error::Result;
use crate::models::CalendarEvent;

/// Default reminder lead time (minutes before the meeting starts).
pub const DEFAULT_NOTIFY_MINUTES: i64 = 10;
const MIN_NOTIFY_MINUTES: i64 = 1;
const MAX_NOTIFY_MINUTES: i64 = 120;

/// Pure planner: which events deserve a reminder right now.
///
/// An event qualifies when it has not been notified yet, is not all-day
/// (all-day events have no meaningful "starts in N minutes"), has not started
/// yet, and starts within the lead window.
pub fn plan_meeting_notifications(events: &[CalendarEvent], now: i64, lead_secs: i64) -> Vec<&CalendarEvent> {
    events
        .iter()
        .filter(|e| e.notified_at.is_none())
        .filter(|e| !e.is_all_day)
        .filter(|e| e.status != "cancelled")
        .filter(|e| e.start_time > now && e.start_time - now <= lead_secs)
        .collect()
}

/// Reminder lead time in seconds, or `None` when meeting notifications are
/// disabled. Reads `calendar_notifications_enabled` (default: on) and
/// `calendar_notify_minutes` (default 10, clamped to [1, 120]).
pub fn notification_lead_secs(db: &Database) -> Result<Option<i64>> {
    let enabled = db
        .get_preference("calendar_notifications_enabled")?
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    if !enabled {
        return Ok(None);
    }
    let minutes = db
        .get_preference("calendar_notify_minutes")?
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(DEFAULT_NOTIFY_MINUTES)
        .clamp(MIN_NOTIFY_MINUTES, MAX_NOTIFY_MINUTES);
    Ok(Some(minutes * 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, start: i64) -> CalendarEvent {
        CalendarEvent {
            id: id.to_string(),
            account_id: "acc1".to_string(),
            provider_event_id: id.to_string(),
            calendar_id: "primary".to_string(),
            title: "Standup".to_string(),
            description: String::new(),
            location: String::new(),
            start_time: start,
            end_time: start + 1_800,
            is_all_day: false,
            timezone: String::new(),
            organizer: String::new(),
            attendees: Vec::new(),
            meeting_link: None,
            meeting_platform: None,
            status: "confirmed".to_string(),
            html_link: None,
            notified_at: None,
            recurring_event_id: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    const NOW: i64 = 100_000;
    const LEAD: i64 = 600; // 10 minutes

    #[test]
    fn event_starting_inside_lead_window_is_planned() {
        let events = vec![event("soon", NOW + 300)];
        let planned = plan_meeting_notifications(&events, NOW, LEAD);
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].id, "soon");
    }

    #[test]
    fn event_starting_after_lead_window_is_not_planned_yet() {
        let events = vec![event("later", NOW + LEAD + 1)];
        assert!(plan_meeting_notifications(&events, NOW, LEAD).is_empty());
    }

    #[test]
    fn event_already_started_is_not_planned() {
        let events = vec![event("started", NOW), event("past", NOW - 60)];
        assert!(plan_meeting_notifications(&events, NOW, LEAD).is_empty());
    }

    #[test]
    fn already_notified_event_is_not_planned_again() {
        let mut e = event("done", NOW + 300);
        e.notified_at = Some(NOW - 60);
        assert!(plan_meeting_notifications(&[e], NOW, LEAD).is_empty());
    }

    #[test]
    fn all_day_events_never_notify() {
        let mut e = event("allday", NOW + 300);
        e.is_all_day = true;
        assert!(plan_meeting_notifications(&[e], NOW, LEAD).is_empty());
    }

    #[test]
    fn cancelled_events_never_notify() {
        let mut e = event("cancelled", NOW + 300);
        e.status = "cancelled".to_string();
        assert!(plan_meeting_notifications(&[e], NOW, LEAD).is_empty());
    }

    #[test]
    fn tentative_events_do_notify() {
        let mut e = event("tentative", NOW + 300);
        e.status = "tentative".to_string();
        assert_eq!(plan_meeting_notifications(&[e], NOW, LEAD).len(), 1);
    }

    #[test]
    fn boundary_exactly_lead_seconds_before_start_is_planned() {
        let events = vec![event("edge", NOW + LEAD)];
        assert_eq!(plan_meeting_notifications(&events, NOW, LEAD).len(), 1);
    }

    // ── notification_lead_secs (prefs) ─────────────────────────────────────

    #[test]
    fn lead_defaults_to_ten_minutes_when_unset() {
        let db = Database::new_for_testing().expect("db");
        assert_eq!(notification_lead_secs(&db).expect("read"), Some(600));
    }

    #[test]
    fn lead_reads_configured_minutes() {
        let db = Database::new_for_testing().expect("db");
        db.set_preference("calendar_notify_minutes", "30").expect("set");
        assert_eq!(notification_lead_secs(&db).expect("read"), Some(1_800));
    }

    #[test]
    fn lead_clamps_out_of_range_values() {
        let db = Database::new_for_testing().expect("db");
        db.set_preference("calendar_notify_minutes", "0").expect("set");
        assert_eq!(
            notification_lead_secs(&db).expect("read"),
            Some(60),
            "clamped to 1 minute"
        );
        db.set_preference("calendar_notify_minutes", "9999").expect("set");
        assert_eq!(
            notification_lead_secs(&db).expect("read"),
            Some(7_200),
            "clamped to 120 minutes"
        );
    }

    #[test]
    fn disabled_notifications_return_none() {
        let db = Database::new_for_testing().expect("db");
        db.set_preference("calendar_notifications_enabled", "false")
            .expect("set");
        assert_eq!(notification_lead_secs(&db).expect("read"), None);
    }

    #[test]
    fn garbage_minutes_pref_falls_back_to_default() {
        let db = Database::new_for_testing().expect("db");
        db.set_preference("calendar_notify_minutes", "soon™").expect("set");
        assert_eq!(notification_lead_secs(&db).expect("read"), Some(600));
    }
}
