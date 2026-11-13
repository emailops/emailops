//! Upcoming-meeting notification planning. The planner is pure; the executor
//! loop lives in `services::sync_scheduler` and is deliberately thin: send the
//! OS notification, emit the `meeting-reminder` event (the in-app banner with
//! the Join button), mark the row notified.

use std::collections::HashSet;

use crate::db::Database;
use crate::models::error::Result;
use crate::models::{Calendar, CalendarEvent};

/// Default reminder lead time (minutes before the meeting starts).
pub const DEFAULT_NOTIFY_MINUTES: i64 = 10;
const MIN_NOTIFY_MINUTES: i64 = 1;
const MAX_NOTIFY_MINUTES: i64 = 120;

/// Pure planner: which events deserve a reminder right now.
///
/// An event qualifies when it has not been notified yet, is not all-day
/// (all-day events have no meaningful "starts in N minutes"), has not started
/// yet, starts within the lead window, `own_email` has not declined it, and
/// it is not on a calendar another person shared with the account (see
/// [`is_own_calendar`]). An event whose calendar is not in `calendars` counts
/// as the account's own.
///
/// A meeting on several calendars of an account is one row per calendar with
/// the same provider id, and gets one reminder: the first copy planned marks
/// the meeting, and a copy already notified keeps the others quiet. An
/// invitation that also shows on a shared calendar is therefore reminded once,
/// from the copy on the user's own calendar.
pub fn plan_meeting_notifications<'a>(
    events: &'a [CalendarEvent],
    calendars: &[Calendar],
    now: i64,
    lead_secs: i64,
    own_email: &str,
) -> Vec<&'a CalendarEvent> {
    let shared: HashSet<&str> = calendars
        .iter()
        .filter(|c| !is_own_calendar(c, own_email))
        .map(|c| c.provider_calendar_id.as_str())
        .collect();
    let meeting = |e: &'a CalendarEvent| (e.provider_event_id.as_str(), e.start_time);
    let mut reminded: HashSet<(&str, i64)> = events.iter().filter(|e| e.notified_at.is_some()).map(meeting).collect();
    events
        .iter()
        .filter(|e| !e.is_all_day)
        .filter(|e| e.status != "cancelled")
        .filter(|e| e.start_time > now && e.start_time - now <= lead_secs)
        .filter(|e| !declined_by(e, own_email))
        .filter(|e| !shared.contains(e.calendar_id.as_str()))
        .filter(|e| reminded.insert(meeting(e)))
        .collect()
}

/// The account's own calendar: its primary one, an ICS feed the user
/// subscribed to, or one whose provider-reported owner is `own_email`.
/// Anything else was shared by another person — including another person's
/// Google primary calendar, which reports no owner at all.
fn is_own_calendar(calendar: &Calendar, own_email: &str) -> bool {
    calendar.is_primary
        || calendar.feed_host.is_some()
        || calendar
            .owner_email
            .as_deref()
            .is_some_and(|owner| owner.eq_ignore_ascii_case(own_email))
}

fn declined_by(event: &CalendarEvent, email: &str) -> bool {
    event
        .attendees
        .iter()
        .any(|a| a.email.eq_ignore_ascii_case(email) && a.response == "declined")
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
    const OWN: &str = "me@example.com";

    #[test]
    fn event_starting_inside_lead_window_is_planned() {
        let events = vec![event("soon", NOW + 300)];
        let planned = plan_meeting_notifications(&events, &[], NOW, LEAD, OWN);
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].id, "soon");
    }

    #[test]
    fn event_starting_after_lead_window_is_not_planned_yet() {
        let events = vec![event("later", NOW + LEAD + 1)];
        assert!(plan_meeting_notifications(&events, &[], NOW, LEAD, OWN).is_empty());
    }

    #[test]
    fn event_already_started_is_not_planned() {
        let events = vec![event("started", NOW), event("past", NOW - 60)];
        assert!(plan_meeting_notifications(&events, &[], NOW, LEAD, OWN).is_empty());
    }

    #[test]
    fn already_notified_event_is_not_planned_again() {
        let mut e = event("done", NOW + 300);
        e.notified_at = Some(NOW - 60);
        assert!(plan_meeting_notifications(&[e], &[], NOW, LEAD, OWN).is_empty());
    }

    #[test]
    fn all_day_events_never_notify() {
        let mut e = event("allday", NOW + 300);
        e.is_all_day = true;
        assert!(plan_meeting_notifications(&[e], &[], NOW, LEAD, OWN).is_empty());
    }

    #[test]
    fn cancelled_events_never_notify() {
        let mut e = event("cancelled", NOW + 300);
        e.status = "cancelled".to_string();
        assert!(plan_meeting_notifications(&[e], &[], NOW, LEAD, OWN).is_empty());
    }

    #[test]
    fn tentative_events_do_notify() {
        let mut e = event("tentative", NOW + 300);
        e.status = "tentative".to_string();
        assert_eq!(plan_meeting_notifications(&[e], &[], NOW, LEAD, OWN).len(), 1);
    }

    fn attendee(email: &str, response: &str) -> crate::models::CalendarAttendee {
        crate::models::CalendarAttendee {
            email: email.to_string(),
            response: response.to_string(),
        }
    }

    #[test]
    fn a_meeting_the_user_declined_is_not_planned() {
        let mut e = event("declined", NOW + 300);
        e.attendees = vec![
            attendee("Me@Example.com", "declined"),
            attendee("a@example.com", "accepted"),
        ];
        assert!(plan_meeting_notifications(&[e], &[], NOW, LEAD, OWN).is_empty());
    }

    #[test]
    fn a_meeting_someone_else_declined_is_planned() {
        let mut e = event("theirs", NOW + 300);
        e.attendees = vec![
            attendee("me@example.com", "accepted"),
            attendee("a@example.com", "declined"),
        ];
        assert_eq!(plan_meeting_notifications(&[e], &[], NOW, LEAD, OWN).len(), 1);
    }

    #[test]
    fn a_meeting_on_two_calendars_is_planned_once() {
        let mut other = event("same", NOW + 300);
        other.calendar_id = "team".to_string();
        let events = vec![event("same", NOW + 300), other];
        assert_eq!(plan_meeting_notifications(&events, &[], NOW, LEAD, OWN).len(), 1);
    }

    #[test]
    fn the_other_copy_stays_quiet_once_one_copy_was_notified() {
        let mut notified = event("same", NOW + 300);
        notified.notified_at = Some(NOW - 60);
        let mut other = event("same", NOW + 300);
        other.calendar_id = "team".to_string();
        assert!(plan_meeting_notifications(&[notified, other], &[], NOW, LEAD, OWN).is_empty());
    }

    #[test]
    fn two_occurrences_of_one_id_are_two_meetings() {
        let events = vec![event("series", NOW + 300), event("series", NOW + 500)];
        assert_eq!(plan_meeting_notifications(&events, &[], NOW, LEAD, OWN).len(), 2);
    }

    #[test]
    fn boundary_exactly_lead_seconds_before_start_is_planned() {
        let events = vec![event("edge", NOW + LEAD)];
        assert_eq!(plan_meeting_notifications(&events, &[], NOW, LEAD, OWN).len(), 1);
    }

    // ── whose calendar reminds ─────────────────────────────────────────────

    fn calendar(provider_calendar_id: &str) -> Calendar {
        Calendar {
            id: format!("acc1:{provider_calendar_id}"),
            account_id: "acc1".to_string(),
            provider_calendar_id: provider_calendar_id.to_string(),
            name: String::new(),
            color: String::new(),
            is_primary: false,
            access_role: "reader".to_string(),
            is_visible: true,
            sort_order: 0,
            owner_email: None,
            feed_host: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn event_on(calendar_id: &str, id: &str, start: i64) -> CalendarEvent {
        let mut e = event(id, start);
        e.calendar_id = calendar_id.to_string();
        e
    }

    #[test]
    fn a_meeting_on_another_persons_shared_primary_calendar_is_not_planned() {
        // Google lists another person's primary calendar under their address
        // and without a dataOwner.
        let events = vec![event_on("colleague@example.com", "theirs", NOW + 300)];
        let calendars = vec![calendar("colleague@example.com")];
        assert!(plan_meeting_notifications(&events, &calendars, NOW, LEAD, OWN).is_empty());
    }

    #[test]
    fn a_meeting_on_a_calendar_someone_else_owns_is_not_planned() {
        let mut team = calendar("team@group.calendar.google.com");
        team.owner_email = Some("colleague@example.com".to_string());
        team.access_role = "writer".to_string();
        let events = vec![event_on("team@group.calendar.google.com", "standup", NOW + 300)];
        assert!(plan_meeting_notifications(&events, &[team], NOW, LEAD, OWN).is_empty());
    }

    #[test]
    fn a_meeting_on_a_secondary_calendar_the_user_owns_is_planned() {
        let mut gym = calendar("gym@group.calendar.google.com");
        gym.owner_email = Some("Me@Example.com".to_string());
        let events = vec![event_on("gym@group.calendar.google.com", "class", NOW + 300)];
        assert_eq!(plan_meeting_notifications(&events, &[gym], NOW, LEAD, OWN).len(), 1);
    }

    #[test]
    fn a_meeting_on_the_primary_calendar_is_planned() {
        let mut primary = calendar("primary");
        primary.is_primary = true;
        let events = vec![event_on("primary", "mine", NOW + 300)];
        assert_eq!(plan_meeting_notifications(&events, &[primary], NOW, LEAD, OWN).len(), 1);
    }

    #[test]
    fn a_meeting_on_a_subscribed_feed_is_planned() {
        let mut feed = calendar("feed-2");
        feed.feed_host = Some("calendar.example.com".to_string());
        let events = vec![event_on("feed-2", "dentist", NOW + 300)];
        assert_eq!(plan_meeting_notifications(&events, &[feed], NOW, LEAD, OWN).len(), 1);
    }

    #[test]
    fn an_invitation_also_on_a_shared_calendar_is_planned_once_from_the_users_copy() {
        let mut primary = calendar("primary");
        primary.is_primary = true;
        let calendars = vec![primary, calendar("colleague@example.com")];
        let events = vec![
            event_on("colleague@example.com", "review", NOW + 300),
            event_on("primary", "review", NOW + 300),
        ];

        let planned = plan_meeting_notifications(&events, &calendars, NOW, LEAD, OWN);

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].calendar_id, "primary");
    }

    #[test]
    fn a_meeting_on_a_calendar_not_registered_yet_is_planned() {
        let events = vec![event_on("unregistered", "new", NOW + 300)];
        assert_eq!(plan_meeting_notifications(&events, &[], NOW, LEAD, OWN).len(), 1);
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
