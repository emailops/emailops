//! Calendar event storage. Rows are provider-expanded recurrence instances
//! keyed by `(account_id, calendar_id, provider_event_id)`; all reads are
//! account-scoped (the calendar surface is per-account by decision —
//! docs/DECISIONS.md). One account can hold several calendars: its own, plus
//! any shared with it — see `db::calendars` for the registry.

use crate::db::Database;
use crate::models::error::Result;
use crate::models::{CalendarAttendee, CalendarEvent};
use rusqlite::params;

/// Attendees were first persisted as plain email-string arrays; they now carry
/// RSVP state. Accept both shapes so pre-upgrade rows keep their attendee list
/// (with "needsAction") until the next sync refreshes them.
fn parse_attendees_json(raw: &str) -> Vec<CalendarAttendee> {
    if let Ok(list) = serde_json::from_str::<Vec<CalendarAttendee>>(raw) {
        return list;
    }
    serde_json::from_str::<Vec<String>>(raw)
        .map(|emails| {
            emails
                .into_iter()
                .map(|email| CalendarAttendee {
                    email,
                    response: "needsAction".to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

const EVENT_COLUMNS: &str = "id, account_id, provider_event_id, calendar_id, title, description, location, \
     start_time, end_time, is_all_day, timezone, organizer, attendees_json, meeting_link, \
     meeting_platform, status, html_link, notified_at, created_at, updated_at, recurring_event_id";

/// Preference key for the per-account calendar-integration opt-in. The
/// frontend writes the same composite key through the generic prefs commands
/// (`calendar.enabled:<account_id>` — see `src/lib/api.ts`).
pub fn calendar_enabled_pref_key(account_id: &str) -> String {
    format!("calendar.enabled:{account_id}")
}

fn row_to_event(row: &rusqlite::Row) -> rusqlite::Result<CalendarEvent> {
    let attendees_json: String = row.get(12)?;
    Ok(CalendarEvent {
        id: row.get(0)?,
        account_id: row.get(1)?,
        provider_event_id: row.get(2)?,
        calendar_id: row.get(3)?,
        title: row.get(4)?,
        description: row.get(5)?,
        location: row.get(6)?,
        start_time: row.get(7)?,
        end_time: row.get(8)?,
        is_all_day: row.get::<_, i32>(9)? != 0,
        timezone: row.get(10)?,
        organizer: row.get(11)?,
        attendees: parse_attendees_json(&attendees_json),
        meeting_link: row.get(13)?,
        meeting_platform: row.get(14)?,
        status: row.get(15)?,
        html_link: row.get(16)?,
        notified_at: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
        recurring_event_id: row.get(20)?,
    })
}

impl Database {
    /// Whether calendar integration is on for this account. On by default —
    /// only an explicit `"false"` disables it, written either by the user's
    /// Settings → Calendar toggle or by the scheduler when the provider
    /// reports the account never granted calendar permission.
    pub fn calendar_enabled(&self, account_id: &str) -> Result<bool> {
        Ok(self.get_preference(&calendar_enabled_pref_key(account_id))?.as_deref() != Some("false"))
    }

    /// Insert or update a batch of events in one transaction. Conflict target
    /// is `(account_id, calendar_id, provider_event_id)` — a re-synced instance
    /// updates in place and keeps its row id and `notified_at` marker (so an
    /// event whose details change after the reminder fired is not re-notified).
    /// The calendar is part of the key because providers reuse one event id
    /// across every calendar the event is visible in.
    pub fn upsert_calendar_events(&self, events: &[CalendarEvent]) -> Result<()> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO calendar_events (
                     id, account_id, provider_event_id, calendar_id, title, description, location,
                     start_time, end_time, is_all_day, timezone, organizer, attendees_json,
                     meeting_link, meeting_platform, status, html_link, notified_at, created_at, updated_at,
                     recurring_event_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
                 ON CONFLICT (account_id, calendar_id, provider_event_id) DO UPDATE SET
                     title = excluded.title,
                     description = excluded.description,
                     location = excluded.location,
                     start_time = excluded.start_time,
                     end_time = excluded.end_time,
                     is_all_day = excluded.is_all_day,
                     timezone = excluded.timezone,
                     organizer = excluded.organizer,
                     attendees_json = excluded.attendees_json,
                     meeting_link = excluded.meeting_link,
                     meeting_platform = excluded.meeting_platform,
                     status = excluded.status,
                     html_link = excluded.html_link,
                     updated_at = excluded.updated_at,
                     recurring_event_id = excluded.recurring_event_id",
            )?;
            for event in events {
                let attendees_json = serde_json::to_string(&event.attendees)
                    .map_err(|e| crate::models::error::AppError::IoError(e.to_string()))?;
                stmt.execute(params![
                    event.id,
                    event.account_id,
                    event.provider_event_id,
                    event.calendar_id,
                    event.title,
                    event.description,
                    event.location,
                    event.start_time,
                    event.end_time,
                    event.is_all_day as i32,
                    event.timezone,
                    event.organizer,
                    attendees_json,
                    event.meeting_link,
                    event.meeting_platform,
                    event.status,
                    event.html_link,
                    event.notified_at,
                    event.created_at,
                    event.updated_at,
                    event.recurring_event_id,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Events overlapping `[range_start, range_end)` for one account, ordered
    /// by start time. Cancelled events are excluded — they are kept only until
    /// the next window replace and never rendered.
    pub fn list_calendar_events(
        &self,
        account_id: &str,
        range_start: i64,
        range_end: i64,
    ) -> Result<Vec<CalendarEvent>> {
        let conn = self.reader();
        let sql = format!(
            "SELECT {EVENT_COLUMNS} FROM calendar_events
             WHERE account_id = ?1 AND start_time < ?3 AND end_time > ?2 AND status != 'cancelled'
             ORDER BY start_time ASC, end_time ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![account_id, range_start, range_end], row_to_event)?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Like [`Self::list_calendar_events`], but excluding calendars the user
    /// hid in Settings → Calendar.
    ///
    /// This is the listing for anything that *acts* on events — meeting
    /// reminders, the chat calendar tool — because a hidden calendar must not
    /// pop up a notification or turn up in a chat answer. The calendar view
    /// deliberately uses the unfiltered listing instead and filters client-side,
    /// so toggling a calendar back on is instant rather than a refetch.
    ///
    /// Events whose calendar has no registry row yet (synced before the
    /// registry existed) count as visible — never silently drop them.
    pub fn list_visible_calendar_events(
        &self,
        account_id: &str,
        range_start: i64,
        range_end: i64,
    ) -> Result<Vec<CalendarEvent>> {
        let conn = self.reader();
        let sql = format!(
            "SELECT {EVENT_COLUMNS} FROM calendar_events
             WHERE account_id = ?1 AND start_time < ?3 AND end_time > ?2 AND status != 'cancelled'
               AND calendar_id NOT IN (
                   SELECT provider_calendar_id FROM calendars
                   WHERE account_id = ?1 AND is_visible = 0
               )
             ORDER BY start_time ASC, end_time ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![account_id, range_start, range_end], row_to_event)?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Delete one event from one calendar (the user deleted it, or an
    /// incremental sync saw a cancellation).
    pub fn delete_calendar_event(&self, account_id: &str, calendar_id: &str, provider_event_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM calendar_events WHERE account_id = ?1 AND calendar_id = ?2 AND provider_event_id = ?3",
            params![account_id, calendar_id, provider_event_id],
        )?;
        Ok(())
    }

    /// One event by its calendar + provider id — the delete command resolves
    /// the clicked instance into its series linkage this way. The calendar is
    /// part of the lookup because the same event id can exist in several.
    pub fn get_calendar_event_by_provider_id(
        &self,
        account_id: &str,
        calendar_id: &str,
        provider_event_id: &str,
    ) -> Result<Option<CalendarEvent>> {
        let conn = self.reader();
        let sql = format!(
            "SELECT {EVENT_COLUMNS} FROM calendar_events
             WHERE account_id = ?1 AND calendar_id = ?2 AND provider_event_id = ?3"
        );
        let mut stmt = conn.prepare(&sql)?;
        match stmt.query_row(params![account_id, calendar_id, provider_event_id], row_to_event) {
            Ok(event) => Ok(Some(event)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete every stored instance of a recurring series (whole-series delete).
    /// Matches instances linked via `recurring_event_id` AND a possible master
    /// row stored under the master id itself.
    pub fn delete_calendar_events_for_master(
        &self,
        account_id: &str,
        calendar_id: &str,
        master_id: &str,
    ) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM calendar_events
             WHERE account_id = ?1 AND calendar_id = ?2
               AND (recurring_event_id = ?3 OR provider_event_id = ?3)",
            params![account_id, calendar_id, master_id],
        )?;
        Ok(())
    }

    /// Delete a series' stored instances starting at or after `from_start`
    /// ("this and following events").
    pub fn delete_calendar_events_for_master_from(
        &self,
        account_id: &str,
        calendar_id: &str,
        master_id: &str,
        from_start: i64,
    ) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM calendar_events
             WHERE account_id = ?1 AND calendar_id = ?2 AND recurring_event_id = ?3 AND start_time >= ?4",
            params![account_id, calendar_id, master_id, from_start],
        )?;
        Ok(())
    }

    /// Delete events starting inside `[window_start, window_end)` whose
    /// `updated_at` predates the current sync run. Full-window resyncs upsert
    /// every fetched instance with `updated_at = run_started_at`, so anything
    /// older inside the window was removed upstream. Upsert-then-sweep (rather
    /// than delete-then-insert) keeps `notified_at` on surviving rows.
    ///
    /// Scoped to `calendar_ids` — the calendars whose fetch actually succeeded
    /// this run. A calendar that errored is left completely untouched instead
    /// of having its events blanked by a sweep that never saw them.
    pub fn delete_stale_calendar_events(
        &self,
        account_id: &str,
        calendar_ids: &[String],
        window_start: i64,
        window_end: i64,
        run_started_at: i64,
    ) -> Result<()> {
        if calendar_ids.is_empty() {
            return Ok(());
        }
        let conn = self.connection();
        let placeholders = (0..calendar_ids.len())
            .map(|i| format!("?{}", i + 5))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "DELETE FROM calendar_events
             WHERE account_id = ?1 AND start_time >= ?2 AND start_time < ?3 AND updated_at < ?4
               AND calendar_id IN ({placeholders})"
        );
        let mut args: Vec<&dyn rusqlite::ToSql> = vec![&account_id, &window_start, &window_end, &run_started_at];
        for id in calendar_ids {
            args.push(id);
        }
        conn.execute(&sql, args.as_slice())?;
        Ok(())
    }

    /// Drop every event belonging to a calendar the account no longer has
    /// (unsubscribed, or sharing revoked). Called only after a successful
    /// calendar-list fetch — otherwise a transient list failure would delete
    /// the whole mirror.
    pub fn delete_calendar_events_not_in(&self, account_id: &str, live_calendar_ids: &[String]) -> Result<()> {
        let conn = self.connection();
        if live_calendar_ids.is_empty() {
            conn.execute("DELETE FROM calendar_events WHERE account_id = ?1", params![account_id])?;
            return Ok(());
        }
        let placeholders = (0..live_calendar_ids.len())
            .map(|i| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("DELETE FROM calendar_events WHERE account_id = ?1 AND calendar_id NOT IN ({placeholders})");
        let mut args: Vec<&dyn rusqlite::ToSql> = vec![&account_id];
        for id in live_calendar_ids {
            args.push(id);
        }
        conn.execute(&sql, args.as_slice())?;
        Ok(())
    }

    /// Record that the upcoming-meeting notification fired for this event.
    pub fn mark_calendar_event_notified(&self, event_id: &str, notified_at: i64) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE calendar_events SET notified_at = ?2 WHERE id = ?1",
            params![event_id, notified_at],
        )?;
        Ok(())
    }

    /// Incremental-sync cursor for an account (`None` before the first sync
    /// or after an invalidated token was cleared).
    pub fn get_calendar_sync_token(&self, account_id: &str) -> Result<Option<String>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT sync_token FROM calendar_sync_state WHERE account_id = ?1",
            params![account_id],
            |row| row.get::<_, Option<String>>(0),
        );
        match result {
            Ok(token) => Ok(token),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Store (or clear, with `None`) the incremental-sync cursor.
    pub fn set_calendar_sync_token(&self, account_id: &str, sync_token: Option<&str>, last_sync_at: i64) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO calendar_sync_state (account_id, sync_token, last_sync_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (account_id) DO UPDATE SET sync_token = excluded.sync_token, last_sync_at = excluded.last_sync_at",
            params![account_id, sync_token, last_sync_at],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(account_id: &str, provider_event_id: &str, start: i64, end: i64) -> CalendarEvent {
        event_in(account_id, "primary", provider_event_id, start, end)
    }

    fn event_in(account_id: &str, calendar_id: &str, provider_event_id: &str, start: i64, end: i64) -> CalendarEvent {
        CalendarEvent {
            id: format!("{account_id}:{calendar_id}:{provider_event_id}"),
            account_id: account_id.to_string(),
            provider_event_id: provider_event_id.to_string(),
            calendar_id: calendar_id.to_string(),
            title: "Standup".to_string(),
            description: String::new(),
            location: String::new(),
            start_time: start,
            end_time: end,
            is_all_day: false,
            timezone: "Europe/Madrid".to_string(),
            organizer: "organizer@example.com".to_string(),
            attendees: vec![
                CalendarAttendee {
                    email: "a@example.com".to_string(),
                    response: "accepted".to_string(),
                },
                CalendarAttendee {
                    email: "b@example.com".to_string(),
                    response: "needsAction".to_string(),
                },
            ],
            meeting_link: Some("https://meet.google.com/abc-defg-hij".to_string()),
            meeting_platform: Some("meet".to_string()),
            status: "confirmed".to_string(),
            html_link: None,
            notified_at: None,
            recurring_event_id: None,
            created_at: 1_000,
            updated_at: 1_000,
        }
    }

    fn series_instance(account_id: &str, id: &str, master: &str, start: i64) -> CalendarEvent {
        let mut e = event(account_id, id, start, start + 1_800);
        e.recurring_event_id = Some(master.to_string());
        e
    }

    fn test_db() -> Database {
        let db = Database::new_for_testing().expect("create test db");
        db.seed_test_account("acc1");
        db.seed_test_account("acc2");
        db
    }

    #[test]
    fn calendar_enabled_defaults_to_true() {
        let db = test_db();
        assert!(
            db.calendar_enabled("acc1").expect("read"),
            "calendar integration is on by default — only an explicit opt-out disables it"
        );
    }

    #[test]
    fn calendar_enabled_reflects_per_account_preference() {
        let db = test_db();
        db.set_preference(&calendar_enabled_pref_key("acc1"), "false")
            .expect("set pref");

        assert!(!db.calendar_enabled("acc1").expect("read"));
        assert!(db.calendar_enabled("acc2").expect("read"), "pref is per-account");

        db.set_preference(&calendar_enabled_pref_key("acc1"), "true")
            .expect("set pref");
        assert!(db.calendar_enabled("acc1").expect("read"));
    }

    #[test]
    fn visible_listing_skips_events_from_hidden_calendars() {
        // Meeting reminders and the chat calendar tool must not surface a
        // calendar the user switched off in Settings → Calendar.
        let db = test_db();
        db.upsert_calendars(&[
            crate::models::Calendar {
                id: "acc1:primary".to_string(),
                account_id: "acc1".to_string(),
                provider_calendar_id: "primary".to_string(),
                name: "Personal".to_string(),
                color: "#039be5".to_string(),
                is_primary: true,
                access_role: "owner".to_string(),
                is_visible: true,
                sort_order: 0,
                created_at: 0,
                updated_at: 0,
            },
            crate::models::Calendar {
                id: "acc1:holidays".to_string(),
                account_id: "acc1".to_string(),
                provider_calendar_id: "holidays".to_string(),
                name: "Holidays".to_string(),
                color: "#0b8043".to_string(),
                is_primary: false,
                access_role: "reader".to_string(),
                is_visible: false,
                sort_order: 1,
                created_at: 0,
                updated_at: 0,
            },
        ])
        .expect("seed calendars");
        db.upsert_calendar_events(&[
            event_in("acc1", "primary", "mine", 100, 200),
            event_in("acc1", "holidays", "bank-holiday", 100, 200),
        ])
        .expect("upsert");

        let all = db.list_calendar_events("acc1", 0, 1_000).expect("list all");
        let visible = db.list_visible_calendar_events("acc1", 0, 1_000).expect("list visible");

        assert_eq!(
            all.len(),
            2,
            "the calendar view still gets every event to filter itself"
        );
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].provider_event_id, "mine");
    }

    #[test]
    fn visible_listing_keeps_events_whose_calendar_is_not_registered_yet() {
        // Events synced before the registry exists (or by an older build) must
        // not silently vanish from reminders.
        let db = test_db();
        db.upsert_calendar_events(&[event_in("acc1", "primary", "ev1", 100, 200)])
            .expect("upsert");

        let visible = db.list_visible_calendar_events("acc1", 0, 1_000).expect("list");

        assert_eq!(visible.len(), 1);
    }

    #[test]
    fn the_same_event_id_in_two_calendars_is_stored_twice() {
        // Google reuses one event id across every copy of a meeting you are an
        // attendee on, so a meeting visible in both your own calendar and a
        // calendar shared with you arrives twice with the same id. Keying on
        // (account, event) alone made the second copy overwrite the first.
        let db = test_db();
        let mine = event_in("acc1", "primary", "shared-ev", 100, 200);
        let theirs = event_in("acc1", "team@group.calendar.google.com", "shared-ev", 100, 200);

        db.upsert_calendar_events(&[mine, theirs]).expect("upsert");

        let events = db.list_calendar_events("acc1", 0, 1_000).expect("list");
        assert_eq!(events.len(), 2, "one row per (calendar, event), not one per event");
        let mut calendars: Vec<&str> = events.iter().map(|e| e.calendar_id.as_str()).collect();
        calendars.sort_unstable();
        assert_eq!(calendars, vec!["primary", "team@group.calendar.google.com"]);
    }

    #[test]
    fn stale_sweep_spares_calendars_that_were_not_synced_this_run() {
        // A calendar whose fetch failed (access revoked mid-run, provider 5xx)
        // is skipped, not swept — otherwise one flaky shared calendar would
        // blank its events on every failed poll.
        let db = test_db();
        let fresh = event_in("acc1", "primary", "ev1", 100, 200);
        let mut untouched = event_in("acc1", "team@group.calendar.google.com", "ev2", 100, 200);
        untouched.updated_at = 1_000;
        db.upsert_calendar_events(&[fresh, untouched]).expect("upsert");

        // Only "primary" synced successfully in a run that started at 5_000.
        db.delete_stale_calendar_events("acc1", &["primary".to_string()], 0, 1_000, 5_000)
            .expect("sweep");

        let events = db.list_calendar_events("acc1", 0, 1_000).expect("list");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].calendar_id, "team@group.calendar.google.com");
    }

    #[test]
    fn stale_sweep_removes_events_of_the_synced_calendar() {
        let db = test_db();
        let mut stale = event_in("acc1", "primary", "ev1", 100, 200);
        stale.updated_at = 1_000;
        db.upsert_calendar_events(&[stale]).expect("upsert");

        db.delete_stale_calendar_events("acc1", &["primary".to_string()], 0, 1_000, 5_000)
            .expect("sweep");

        assert!(db.list_calendar_events("acc1", 0, 1_000).expect("list").is_empty());
    }

    #[test]
    fn events_of_a_calendar_the_user_no_longer_has_are_dropped() {
        let db = test_db();
        db.upsert_calendar_events(&[
            event_in("acc1", "primary", "ev1", 100, 200),
            event_in("acc1", "gone@group.calendar.google.com", "ev2", 100, 200),
        ])
        .expect("upsert");

        db.delete_calendar_events_not_in("acc1", &["primary".to_string()])
            .expect("sweep orphans");

        let events = db.list_calendar_events("acc1", 0, 1_000).expect("list");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].calendar_id, "primary");
    }

    #[test]
    fn upsert_then_list_roundtrips_all_fields() {
        let db = test_db();
        let e = event("acc1", "ev1", 100, 200);

        db.upsert_calendar_events(std::slice::from_ref(&e)).expect("upsert");
        let events = db.list_calendar_events("acc1", 0, 1_000).expect("list");

        assert_eq!(events.len(), 1);
        let got = &events[0];
        assert_eq!(got.provider_event_id, "ev1");
        let emails: Vec<&str> = got.attendees.iter().map(|a| a.email.as_str()).collect();
        assert_eq!(emails, vec!["a@example.com", "b@example.com"]);
        assert_eq!(got.attendees[0].response, "accepted");
        assert_eq!(got.attendees[1].response, "needsAction");
        assert_eq!(
            got.meeting_link.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(got.meeting_platform.as_deref(), Some("meet"));
        assert_eq!(got.timezone, "Europe/Madrid");
    }

    #[test]
    fn upsert_same_provider_event_updates_in_place() {
        let db = test_db();
        db.upsert_calendar_events(&[event("acc1", "ev1", 100, 200)])
            .expect("insert");

        let mut updated = event("acc1", "ev1", 150, 250);
        updated.id = "different-row-id".to_string(); // conflict path must keep the original row
        updated.title = "Standup (moved)".to_string();
        db.upsert_calendar_events(std::slice::from_ref(&updated))
            .expect("upsert");

        let events = db.list_calendar_events("acc1", 0, 1_000).expect("list");
        assert_eq!(
            events.len(),
            1,
            "conflict on (account, calendar, provider_event) must not duplicate"
        );
        assert_eq!(events[0].title, "Standup (moved)");
        assert_eq!(events[0].start_time, 150);
        assert_eq!(events[0].id, "acc1:primary:ev1", "row id survives updates");
    }

    #[test]
    fn upsert_preserves_notified_at_when_event_changes() {
        let db = test_db();
        db.upsert_calendar_events(&[event("acc1", "ev1", 100, 200)])
            .expect("insert");
        db.mark_calendar_event_notified("acc1:primary:ev1", 90)
            .expect("mark notified");

        db.upsert_calendar_events(&[event("acc1", "ev1", 110, 210)])
            .expect("re-sync");

        let events = db.list_calendar_events("acc1", 0, 1_000).expect("list");
        assert_eq!(
            events[0].notified_at,
            Some(90),
            "re-sync must not reset the notified marker"
        );
    }

    #[test]
    fn list_is_scoped_to_the_account() {
        let db = test_db();
        db.upsert_calendar_events(&[event("acc1", "ev1", 100, 200), event("acc2", "ev2", 100, 200)])
            .expect("upsert");

        let events = db.list_calendar_events("acc1", 0, 1_000).expect("list");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].account_id, "acc1");
    }

    #[test]
    fn list_returns_events_overlapping_the_range() {
        let db = test_db();
        db.upsert_calendar_events(&[
            event("acc1", "before", 0, 50),     // ends before range
            event("acc1", "spanning", 50, 150), // straddles range start
            event("acc1", "inside", 110, 190),  // fully inside
            event("acc1", "after", 200, 300),   // starts at range end (exclusive)
        ])
        .expect("upsert");

        let events = db.list_calendar_events("acc1", 100, 200).expect("list");
        let ids: Vec<&str> = events.iter().map(|e| e.provider_event_id.as_str()).collect();
        assert_eq!(ids, vec!["spanning", "inside"]);
    }

    #[test]
    fn list_excludes_cancelled_events() {
        let db = test_db();
        let mut cancelled = event("acc1", "ev1", 100, 200);
        cancelled.status = "cancelled".to_string();
        db.upsert_calendar_events(&[cancelled, event("acc1", "ev2", 100, 200)])
            .expect("upsert");

        let events = db.list_calendar_events("acc1", 0, 1_000).expect("list");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].provider_event_id, "ev2");
    }

    #[test]
    fn delete_event_removes_only_that_provider_event() {
        let db = test_db();
        db.upsert_calendar_events(&[event("acc1", "ev1", 100, 200), event("acc1", "ev2", 300, 400)])
            .expect("upsert");

        db.delete_calendar_event("acc1", "primary", "ev1").expect("delete");

        let events = db.list_calendar_events("acc1", 0, 1_000).expect("list");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].provider_event_id, "ev2");
    }

    #[test]
    fn delete_stale_removes_window_events_not_touched_by_this_sync_run() {
        // Full-window resync: everything fetched this run was upserted with
        // updated_at >= run_started_at; anything older inside the window no
        // longer exists upstream and must go. Events outside the window and
        // fresh events survive — with their notified_at intact.
        let db = test_db();
        let mut stale = event("acc1", "stale", 100, 200);
        stale.updated_at = 1_000;
        let mut fresh = event("acc1", "fresh", 150, 250);
        fresh.updated_at = 5_000;
        let mut outside = event("acc1", "outside", 900, 950);
        outside.updated_at = 1_000;
        db.upsert_calendar_events(&[stale, fresh, outside]).expect("upsert");

        db.delete_stale_calendar_events("acc1", &["primary".to_string()], 0, 300, 5_000)
            .expect("delete stale");

        let events = db.list_calendar_events("acc1", 0, 1_000).expect("list");
        let ids: Vec<&str> = events.iter().map(|e| e.provider_event_id.as_str()).collect();
        assert_eq!(ids, vec!["fresh", "outside"]);
    }

    #[test]
    fn legacy_string_array_attendees_still_load() {
        // Rows written before RSVP tracking stored `["a@example.com", …]`.
        let db = test_db();
        db.upsert_calendar_events(&[event("acc1", "ev1", 100, 200)])
            .expect("insert");
        db.connection()
            .execute(
                "UPDATE calendar_events SET attendees_json = '[\"legacy@example.com\"]' WHERE provider_event_id = 'ev1'",
                [],
            )
            .expect("write legacy shape");

        let events = db.list_calendar_events("acc1", 0, 1_000).expect("list");
        assert_eq!(events[0].attendees.len(), 1);
        assert_eq!(events[0].attendees[0].email, "legacy@example.com");
        assert_eq!(events[0].attendees[0].response, "needsAction");
    }

    #[test]
    fn recurring_event_id_roundtrips() {
        let db = test_db();
        db.upsert_calendar_events(&[series_instance("acc1", "m_1", "m", 100)])
            .expect("upsert");
        let events = db.list_calendar_events("acc1", 0, 1_000).expect("list");
        assert_eq!(events[0].recurring_event_id.as_deref(), Some("m"));
    }

    #[test]
    fn get_by_provider_id_finds_the_row_or_none() {
        let db = test_db();
        db.upsert_calendar_events(&[event("acc1", "ev1", 100, 200)])
            .expect("upsert");
        let found = db
            .get_calendar_event_by_provider_id("acc1", "primary", "ev1")
            .expect("get");
        assert_eq!(found.expect("some").provider_event_id, "ev1");
        assert!(db
            .get_calendar_event_by_provider_id("acc1", "primary", "missing")
            .expect("get")
            .is_none());
        assert!(
            db.get_calendar_event_by_provider_id("acc2", "primary", "ev1")
                .expect("get")
                .is_none(),
            "account-scoped"
        );
        assert!(
            db.get_calendar_event_by_provider_id("acc1", "other@group.calendar.google.com", "ev1")
                .expect("get")
                .is_none(),
            "calendar-scoped: the same event id in another calendar is a different row"
        );
    }

    #[test]
    fn delete_for_master_removes_all_instances_and_the_master_row() {
        let db = test_db();
        db.upsert_calendar_events(&[
            series_instance("acc1", "m_1", "m", 100),
            series_instance("acc1", "m_2", "m", 200),
            event("acc1", "m", 100, 200),             // master row stored under its own id
            event("acc1", "other", 300, 400),         // unrelated event survives
            series_instance("acc2", "m_9", "m", 100), // other account survives
        ])
        .expect("upsert");

        db.delete_calendar_events_for_master("acc1", "primary", "m")
            .expect("delete series");

        let acc1: Vec<String> = db
            .list_calendar_events("acc1", 0, 1_000)
            .expect("list")
            .into_iter()
            .map(|e| e.provider_event_id)
            .collect();
        assert_eq!(acc1, vec!["other"]);
        assert_eq!(db.list_calendar_events("acc2", 0, 1_000).expect("list").len(), 1);
    }

    #[test]
    fn delete_for_master_from_removes_only_later_instances() {
        let db = test_db();
        db.upsert_calendar_events(&[
            series_instance("acc1", "m_1", "m", 100),
            series_instance("acc1", "m_2", "m", 200),
            series_instance("acc1", "m_3", "m", 300),
        ])
        .expect("upsert");

        db.delete_calendar_events_for_master_from("acc1", "primary", "m", 200)
            .expect("truncate");

        let remaining: Vec<String> = db
            .list_calendar_events("acc1", 0, 1_000)
            .expect("list")
            .into_iter()
            .map(|e| e.provider_event_id)
            .collect();
        assert_eq!(remaining, vec!["m_1"], "only instances before the cutoff survive");
    }

    #[test]
    fn sync_token_roundtrip_and_clear() {
        let db = test_db();
        assert_eq!(db.get_calendar_sync_token("acc1").expect("read"), None);

        db.set_calendar_sync_token("acc1", Some("tok-1"), 1_000).expect("set");
        assert_eq!(
            db.get_calendar_sync_token("acc1").expect("read"),
            Some("tok-1".to_string())
        );

        db.set_calendar_sync_token("acc1", Some("tok-2"), 2_000)
            .expect("update");
        assert_eq!(
            db.get_calendar_sync_token("acc1").expect("read"),
            Some("tok-2".to_string())
        );

        db.set_calendar_sync_token("acc1", None, 3_000).expect("clear");
        assert_eq!(db.get_calendar_sync_token("acc1").expect("read"), None);
    }

    #[test]
    fn deleting_an_account_cascades_calendar_rows() {
        let db = test_db();
        db.upsert_calendar_events(&[event("acc1", "ev1", 100, 200)])
            .expect("upsert");
        db.set_calendar_sync_token("acc1", Some("tok"), 1_000)
            .expect("set token");

        db.delete_account("acc1").expect("delete account");

        let remaining: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM calendar_events", [], |r| r.get(0))
            .expect("count events");
        assert_eq!(remaining, 0, "calendar_events must cascade on account delete");
        let tokens: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM calendar_sync_state", [], |r| r.get(0))
            .expect("count sync state");
        assert_eq!(tokens, 0, "calendar_sync_state must cascade on account delete");
    }
}
