//! Calendar event storage. Rows are provider-expanded recurrence instances
//! keyed by `(account_id, provider_event_id)`; all reads are account-scoped
//! (the calendar surface is per-account by decision — docs/DECISIONS.md).

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
    /// is `(account_id, provider_event_id)` — a re-synced instance updates in
    /// place and keeps its row id and `notified_at` marker (so an event whose
    /// details change after the reminder fired is not re-notified).
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
                 ON CONFLICT (account_id, provider_event_id) DO UPDATE SET
                     calendar_id = excluded.calendar_id,
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

    /// Delete one event by its provider id (incremental sync saw a cancellation).
    pub fn delete_calendar_event(&self, account_id: &str, provider_event_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM calendar_events WHERE account_id = ?1 AND provider_event_id = ?2",
            params![account_id, provider_event_id],
        )?;
        Ok(())
    }

    /// One event by its provider id — the delete command resolves the clicked
    /// instance into its series linkage this way.
    pub fn get_calendar_event_by_provider_id(
        &self,
        account_id: &str,
        provider_event_id: &str,
    ) -> Result<Option<CalendarEvent>> {
        let conn = self.reader();
        let sql =
            format!("SELECT {EVENT_COLUMNS} FROM calendar_events WHERE account_id = ?1 AND provider_event_id = ?2");
        let mut stmt = conn.prepare(&sql)?;
        match stmt.query_row(params![account_id, provider_event_id], row_to_event) {
            Ok(event) => Ok(Some(event)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete every stored instance of a recurring series (whole-series delete).
    /// Matches instances linked via `recurring_event_id` AND a possible master
    /// row stored under the master id itself.
    pub fn delete_calendar_events_for_master(&self, account_id: &str, master_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM calendar_events
             WHERE account_id = ?1 AND (recurring_event_id = ?2 OR provider_event_id = ?2)",
            params![account_id, master_id],
        )?;
        Ok(())
    }

    /// Delete a series' stored instances starting at or after `from_start`
    /// ("this and following events").
    pub fn delete_calendar_events_for_master_from(
        &self,
        account_id: &str,
        master_id: &str,
        from_start: i64,
    ) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM calendar_events
             WHERE account_id = ?1 AND recurring_event_id = ?2 AND start_time >= ?3",
            params![account_id, master_id, from_start],
        )?;
        Ok(())
    }

    /// Delete events starting inside `[window_start, window_end)` whose
    /// `updated_at` predates the current sync run. Full-window resyncs upsert
    /// every fetched instance with `updated_at = run_started_at`, so anything
    /// older inside the window was removed upstream. Upsert-then-sweep (rather
    /// than delete-then-insert) keeps `notified_at` on surviving rows.
    pub fn delete_stale_calendar_events(
        &self,
        account_id: &str,
        window_start: i64,
        window_end: i64,
        run_started_at: i64,
    ) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM calendar_events
             WHERE account_id = ?1 AND start_time >= ?2 AND start_time < ?3 AND updated_at < ?4",
            params![account_id, window_start, window_end, run_started_at],
        )?;
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
        CalendarEvent {
            id: format!("{account_id}:{provider_event_id}"),
            account_id: account_id.to_string(),
            provider_event_id: provider_event_id.to_string(),
            calendar_id: "primary".to_string(),
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
            "conflict on (account, provider_event) must not duplicate"
        );
        assert_eq!(events[0].title, "Standup (moved)");
        assert_eq!(events[0].start_time, 150);
        assert_eq!(events[0].id, "acc1:ev1", "row id survives updates");
    }

    #[test]
    fn upsert_preserves_notified_at_when_event_changes() {
        let db = test_db();
        db.upsert_calendar_events(&[event("acc1", "ev1", 100, 200)])
            .expect("insert");
        db.mark_calendar_event_notified("acc1:ev1", 90).expect("mark notified");

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

        db.delete_calendar_event("acc1", "ev1").expect("delete");

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

        db.delete_stale_calendar_events("acc1", 0, 300, 5_000)
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
        let found = db.get_calendar_event_by_provider_id("acc1", "ev1").expect("get");
        assert_eq!(found.expect("some").provider_event_id, "ev1");
        assert!(db
            .get_calendar_event_by_provider_id("acc1", "missing")
            .expect("get")
            .is_none());
        assert!(
            db.get_calendar_event_by_provider_id("acc2", "ev1")
                .expect("get")
                .is_none(),
            "account-scoped"
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

        db.delete_calendar_events_for_master("acc1", "m")
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

        db.delete_calendar_events_for_master_from("acc1", "m", 200)
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
