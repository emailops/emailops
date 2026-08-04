//! Calendar registry: the set of calendars each account can see (its own,
//! shared-with-me, subscribed). Rows are refreshed from the provider on every
//! sync; the user's `is_visible` toggle is local state and is never clobbered
//! by a refresh — see `migrations/V022__calendars.sql`.

use crate::db::Database;
use crate::models::error::Result;
use crate::models::Calendar;
use rusqlite::params;

const CALENDAR_COLUMNS: &str = "id, account_id, provider_calendar_id, name, color, is_primary, \
     access_role, is_visible, sort_order, created_at, updated_at";

/// Stable row id for a calendar. Opaque — never parsed apart.
pub fn calendar_row_id(account_id: &str, provider_calendar_id: &str) -> String {
    format!("{account_id}:{provider_calendar_id}")
}

fn row_to_calendar(row: &rusqlite::Row) -> rusqlite::Result<Calendar> {
    Ok(Calendar {
        id: row.get(0)?,
        account_id: row.get(1)?,
        provider_calendar_id: row.get(2)?,
        name: row.get(3)?,
        color: row.get(4)?,
        is_primary: row.get::<_, i32>(5)? != 0,
        access_role: row.get(6)?,
        is_visible: row.get::<_, i32>(7)? != 0,
        sort_order: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

impl Database {
    /// Insert or refresh a batch of calendars in one transaction. On conflict
    /// the provider-owned fields update in place but `is_visible` is left
    /// alone: hiding a calendar is a local decision and must survive every
    /// subsequent sync.
    pub fn upsert_calendars(&self, calendars: &[Calendar]) -> Result<()> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO calendars (
                     id, account_id, provider_calendar_id, name, color, is_primary,
                     access_role, is_visible, sort_order, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (account_id, provider_calendar_id) DO UPDATE SET
                     name = excluded.name,
                     color = excluded.color,
                     is_primary = excluded.is_primary,
                     access_role = excluded.access_role,
                     sort_order = excluded.sort_order,
                     updated_at = excluded.updated_at",
            )?;
            for calendar in calendars {
                stmt.execute(params![
                    calendar.id,
                    calendar.account_id,
                    calendar.provider_calendar_id,
                    calendar.name,
                    calendar.color,
                    calendar.is_primary as i32,
                    calendar.access_role,
                    calendar.is_visible as i32,
                    calendar.sort_order,
                    calendar.created_at,
                    calendar.updated_at,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Every calendar known for an account, in the provider's own list order
    /// (primary first), then by name so the order is stable.
    pub fn list_calendars(&self, account_id: &str) -> Result<Vec<Calendar>> {
        let conn = self.reader();
        let sql = format!(
            "SELECT {CALENDAR_COLUMNS} FROM calendars
             WHERE account_id = ?1
             ORDER BY is_primary DESC, sort_order ASC, name ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![account_id], row_to_calendar)?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Show or hide one calendar (Settings → Calendar). Hiding filters it out
    /// of the calendar view without stopping its sync, so re-showing it is
    /// instant rather than waiting for the next poll.
    pub fn set_calendar_visible(&self, account_id: &str, provider_calendar_id: &str, visible: bool) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE calendars SET is_visible = ?3 WHERE account_id = ?1 AND provider_calendar_id = ?2",
            params![account_id, provider_calendar_id, visible as i32],
        )?;
        Ok(())
    }

    /// Drop calendars the provider no longer lists (unsubscribed, or access
    /// revoked). Called only after a successful list fetch, so a failed sync
    /// can never wipe the registry.
    pub fn delete_calendars_not_in(&self, account_id: &str, live_provider_ids: &[String]) -> Result<()> {
        let conn = self.connection();
        if live_provider_ids.is_empty() {
            conn.execute("DELETE FROM calendars WHERE account_id = ?1", params![account_id])?;
            return Ok(());
        }
        let placeholders = (0..live_provider_ids.len())
            .map(|i| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let sql =
            format!("DELETE FROM calendars WHERE account_id = ?1 AND provider_calendar_id NOT IN ({placeholders})");
        let mut args: Vec<&dyn rusqlite::ToSql> = vec![&account_id];
        for id in live_provider_ids {
            args.push(id);
        }
        conn.execute(&sql, args.as_slice())?;
        Ok(())
    }
}

/// Exercises the V022 `calendar_events` rebuild against a pre-V022 table.
///
/// The normal migration tests all start from an empty DB, so the
/// `INSERT … SELECT` that carries existing rows across the rebuild is never
/// executed with data in it — exactly the statement that would silently lose a
/// user's synced calendar. This runs the real migration file against a
/// hand-built old-shape table.
#[cfg(test)]
mod v022_rebuild_tests {
    use rusqlite::Connection;

    const V022: &str = include_str!("../../migrations/V022__calendars.sql");

    /// `accounts` (for the FK) plus `calendar_events` in its V011+V012 shape.
    fn pre_v022_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            "CREATE TABLE accounts (id TEXT PRIMARY KEY);
             INSERT INTO accounts (id) VALUES ('acc1');
             CREATE TABLE calendar_events (
                 id TEXT PRIMARY KEY,
                 account_id TEXT NOT NULL,
                 provider_event_id TEXT NOT NULL,
                 calendar_id TEXT NOT NULL DEFAULT 'primary',
                 title TEXT NOT NULL DEFAULT '',
                 description TEXT NOT NULL DEFAULT '',
                 location TEXT NOT NULL DEFAULT '',
                 start_time INTEGER NOT NULL,
                 end_time INTEGER NOT NULL,
                 is_all_day INTEGER NOT NULL DEFAULT 0,
                 timezone TEXT NOT NULL DEFAULT '',
                 organizer TEXT NOT NULL DEFAULT '',
                 attendees_json TEXT NOT NULL DEFAULT '[]',
                 meeting_link TEXT,
                 meeting_platform TEXT,
                 status TEXT NOT NULL DEFAULT 'confirmed'
                     CHECK (status IN ('confirmed', 'tentative', 'cancelled')),
                 html_link TEXT,
                 notified_at INTEGER,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 recurring_event_id TEXT,
                 FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE,
                 UNIQUE (account_id, provider_event_id)
             );
             CREATE INDEX idx_calendar_events_account_start
                 ON calendar_events(account_id, start_time);
             CREATE INDEX idx_calendar_events_account_master
                 ON calendar_events(account_id, recurring_event_id)
                 WHERE recurring_event_id IS NOT NULL;",
        )
        .expect("pre-V022 schema");
        conn
    }

    fn seed_event(conn: &Connection, provider_event_id: &str, notified_at: Option<i64>) {
        conn.execute(
            "INSERT INTO calendar_events
                 (id, account_id, provider_event_id, calendar_id, title, start_time, end_time,
                  attendees_json, notified_at, created_at, updated_at)
             VALUES (?1, 'acc1', ?2, 'primary', 'Standup', 100, 200, '[]', ?3, 10, 10)",
            rusqlite::params![format!("acc1:{provider_event_id}"), provider_event_id, notified_at],
        )
        .expect("seed");
    }

    #[test]
    fn existing_events_survive_the_rebuild_with_rewritten_ids() {
        let conn = pre_v022_db();
        seed_event(&conn, "ev1", Some(90));
        seed_event(&conn, "ev2", None);

        conn.execute_batch(V022).expect("apply V022");

        let mut stmt = conn
            .prepare("SELECT id, provider_event_id, calendar_id, notified_at FROM calendar_events ORDER BY id")
            .expect("prepare");
        let rows: Vec<(String, String, String, Option<i64>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("rows");

        assert_eq!(
            rows,
            vec![
                (
                    "acc1:primary:ev1".to_string(),
                    "ev1".to_string(),
                    "primary".to_string(),
                    Some(90)
                ),
                (
                    "acc1:primary:ev2".to_string(),
                    "ev2".to_string(),
                    "primary".to_string(),
                    None
                ),
            ],
            "the rebuild must carry every row across, notified_at included"
        );
    }

    #[test]
    fn the_rebuilt_table_accepts_one_event_id_in_two_calendars() {
        let conn = pre_v022_db();
        seed_event(&conn, "shared-ev", None);
        conn.execute_batch(V022).expect("apply V022");

        conn.execute(
            "INSERT INTO calendar_events
                 (id, account_id, provider_event_id, calendar_id, title, start_time, end_time,
                  attendees_json, created_at, updated_at)
             VALUES ('acc1:team:shared-ev', 'acc1', 'shared-ev', 'team@group.calendar.google.com',
                     'Standup', 100, 200, '[]', 10, 10)",
            [],
        )
        .expect("the old UNIQUE(account, event) would have rejected this");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM calendar_events", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 2);
    }

    #[test]
    fn the_rebuild_recreates_the_indexes_it_dropped() {
        let conn = pre_v022_db();
        conn.execute_batch(V022).expect("apply V022");

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'calendar_events'")
            .expect("prepare");
        let names: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("rows");

        for required in [
            "idx_calendar_events_account_start",
            "idx_calendar_events_account_master",
        ] {
            assert!(
                names.iter().any(|n| n == required),
                "missing {required}, have {names:?}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calendar(account_id: &str, provider_calendar_id: &str, name: &str, color: &str) -> Calendar {
        Calendar {
            id: calendar_row_id(account_id, provider_calendar_id),
            account_id: account_id.to_string(),
            provider_calendar_id: provider_calendar_id.to_string(),
            name: name.to_string(),
            color: color.to_string(),
            is_primary: provider_calendar_id == "primary",
            access_role: "owner".to_string(),
            is_visible: true,
            sort_order: 0,
            created_at: 1_000,
            updated_at: 1_000,
        }
    }

    fn test_db() -> Database {
        let db = Database::new_for_testing().expect("create test db");
        db.seed_test_account("acc1");
        db.seed_test_account("acc2");
        db
    }

    #[test]
    fn upsert_then_list_roundtrips_all_fields() {
        let db = test_db();
        let mut cal = calendar("acc1", "team@group.calendar.google.com", "Team", "#33b679");
        cal.access_role = "reader".to_string();
        cal.sort_order = 3;

        db.upsert_calendars(std::slice::from_ref(&cal)).expect("upsert");
        let listed = db.list_calendars("acc1").expect("list");

        assert_eq!(listed, vec![cal]);
    }

    #[test]
    fn list_is_scoped_to_the_account() {
        let db = test_db();
        db.upsert_calendars(&[
            calendar("acc1", "primary", "Mine", "#039be5"),
            calendar("acc2", "primary", "Theirs", "#d50000"),
        ])
        .expect("upsert");

        let listed = db.list_calendars("acc1").expect("list");

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Mine");
    }

    #[test]
    fn list_puts_the_primary_calendar_first() {
        let db = test_db();
        let mut shared = calendar("acc1", "shared@group.calendar.google.com", "Shared", "#0b8043");
        shared.sort_order = 0;
        let mut primary = calendar("acc1", "primary", "Personal", "#039be5");
        primary.sort_order = 9; // provider order would otherwise put it last
        db.upsert_calendars(&[shared, primary]).expect("upsert");

        let listed = db.list_calendars("acc1").expect("list");

        assert_eq!(listed[0].provider_calendar_id, "primary");
    }

    #[test]
    fn resync_refreshes_provider_fields() {
        let db = test_db();
        db.upsert_calendars(&[calendar("acc1", "primary", "Old name", "#039be5")])
            .expect("first sync");

        let mut renamed = calendar("acc1", "primary", "New name", "#d50000");
        renamed.updated_at = 2_000;
        db.upsert_calendars(&[renamed]).expect("resync");

        let listed = db.list_calendars("acc1").expect("list");
        assert_eq!(listed.len(), 1, "a resync updates in place, never duplicates");
        assert_eq!(listed[0].name, "New name");
        assert_eq!(listed[0].color, "#d50000");
    }

    #[test]
    fn resync_preserves_the_users_hidden_toggle() {
        // The whole point of a local visibility flag: syncing the calendar
        // list every 5 minutes must not keep un-hiding what the user hid.
        let db = test_db();
        db.upsert_calendars(&[calendar(
            "acc1",
            "holidays@group.v.calendar.google.com",
            "Holidays",
            "#0b8043",
        )])
        .expect("first sync");
        db.set_calendar_visible("acc1", "holidays@group.v.calendar.google.com", false)
            .expect("hide");

        db.upsert_calendars(&[calendar(
            "acc1",
            "holidays@group.v.calendar.google.com",
            "Holidays",
            "#0b8043",
        )])
        .expect("resync");

        let listed = db.list_calendars("acc1").expect("list");
        assert!(!listed[0].is_visible, "sync must not reset the user's hide toggle");
    }

    #[test]
    fn set_calendar_visible_is_scoped_to_the_account() {
        let db = test_db();
        db.upsert_calendars(&[
            calendar("acc1", "primary", "Mine", "#039be5"),
            calendar("acc2", "primary", "Theirs", "#d50000"),
        ])
        .expect("upsert");

        db.set_calendar_visible("acc1", "primary", false).expect("hide");

        assert!(!db.list_calendars("acc1").expect("list")[0].is_visible);
        assert!(db.list_calendars("acc2").expect("list")[0].is_visible);
    }

    #[test]
    fn delete_calendars_not_in_drops_only_the_missing_ones() {
        let db = test_db();
        db.upsert_calendars(&[
            calendar("acc1", "primary", "Personal", "#039be5"),
            calendar("acc1", "team@group.calendar.google.com", "Team", "#33b679"),
            calendar("acc2", "primary", "Other account", "#d50000"),
        ])
        .expect("upsert");

        db.delete_calendars_not_in("acc1", &["primary".to_string()])
            .expect("sweep");

        let acc1 = db.list_calendars("acc1").expect("list");
        assert_eq!(acc1.len(), 1);
        assert_eq!(acc1[0].provider_calendar_id, "primary");
        assert_eq!(
            db.list_calendars("acc2").expect("list").len(),
            1,
            "other accounts untouched"
        );
    }

    #[test]
    fn delete_calendars_not_in_with_no_live_ids_clears_the_account() {
        let db = test_db();
        db.upsert_calendars(&[calendar("acc1", "primary", "Personal", "#039be5")])
            .expect("upsert");

        db.delete_calendars_not_in("acc1", &[]).expect("sweep");

        assert!(db.list_calendars("acc1").expect("list").is_empty());
    }
}
