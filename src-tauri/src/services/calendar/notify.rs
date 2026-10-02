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

/// Preference: show the meeting title in the OS notification (`"true"`).
/// Off by default — OS notifications appear on the lock screen and in
/// Notification Center, outside the app's main-password lock (CASA/DASA
/// 1.10.2). The in-app reminder banner always shows the full event.
pub const SHOW_TITLE_PREF: &str = "calendar_notification_show_title";

pub fn notification_shows_title(db: &Database) -> Result<bool> {
    Ok(db
        .get_preference(SHOW_TITLE_PREF)?
        .is_some_and(|v| v.eq_ignore_ascii_case("true")))
}

/// Pure planner: the (title, body) of the OS notification for `event`.
pub fn notification_text(event: &CalendarEvent, minutes_left: i64, show_title: bool) -> (String, String) {
    let body = match &event.meeting_platform {
        Some(platform) => format!("Starts in {minutes_left} min · join via {platform}"),
        None => format!("Starts in {minutes_left} min"),
    };
    let title = if show_title && !event.title.is_empty() {
        event.title.clone()
    } else {
        "Upcoming meeting".to_string()
    };
    (title, body)
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

    #[test]
    fn notification_hides_the_meeting_title_by_default() {
        let db = Database::new_for_testing().expect("db");
        assert!(!notification_shows_title(&db).expect("read"));
    }

    #[test]
    fn notification_shows_the_title_when_the_user_opts_in() {
        let db = Database::new_for_testing().expect("db");
        db.set_preference(SHOW_TITLE_PREF, "true").expect("set");
        assert!(notification_shows_title(&db).expect("read"));
    }

    #[test]
    fn hidden_title_notification_carries_no_event_text() {
        let mut e = event("e1", NOW + 300);
        e.title = "Board review: acquisition terms".to_string();
        e.meeting_platform = Some("Zoom".to_string());
        let (title, body) = notification_text(&e, 5, false);
        assert_eq!(title, "Upcoming meeting");
        assert_eq!(body, "Starts in 5 min · join via Zoom");
    }

    #[test]
    fn shown_title_notification_uses_the_event_title() {
        let mut e = event("e1", NOW + 300);
        e.title = "Design sync".to_string();
        let (title, body) = notification_text(&e, 5, true);
        assert_eq!(title, "Design sync");
        assert_eq!(body, "Starts in 5 min");
    }

    #[test]
    fn shown_title_falls_back_when_the_event_has_none() {
        let mut e = event("e1", NOW + 300);
        e.title = String::new();
        assert_eq!(notification_text(&e, 5, true).0, "Upcoming meeting");
    }
}
