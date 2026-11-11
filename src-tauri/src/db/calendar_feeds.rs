//! Subscribed ICS feeds (read-only calendars for accounts whose provider has
//! no calendar API). The feed URL is a credential and lives in the secrets
//! vault; this table only knows the feed exists — see
//! `migrations/V039__calendar_feeds.sql`.

use rusqlite::{params, OptionalExtension};

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::CalendarFeed;

const FEED_COLUMNS: &str = "id, account_id, label, host, created_at";

fn row_to_feed(row: &rusqlite::Row) -> rusqlite::Result<CalendarFeed> {
    Ok(CalendarFeed {
        id: row.get(0)?,
        account_id: row.get(1)?,
        label: row.get(2)?,
        host: row.get(3)?,
        created_at: row.get(4)?,
    })
}

impl Database {
    pub fn insert_calendar_feed(&self, feed: &CalendarFeed) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO calendar_feeds (id, account_id, label, host, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![feed.id, feed.account_id, feed.label, feed.host, feed.created_at],
        )?;
        Ok(())
    }

    /// Feeds of one account, oldest first.
    pub fn list_calendar_feeds(&self, account_id: &str) -> Result<Vec<CalendarFeed>> {
        let conn = self.reader();
        let sql =
            format!("SELECT {FEED_COLUMNS} FROM calendar_feeds WHERE account_id = ?1 ORDER BY created_at ASC, id ASC");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![account_id], row_to_feed)?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn get_calendar_feed(&self, feed_id: &str) -> Result<Option<CalendarFeed>> {
        let conn = self.reader();
        let sql = format!("SELECT {FEED_COLUMNS} FROM calendar_feeds WHERE id = ?1");
        Ok(conn.query_row(&sql, params![feed_id], row_to_feed).optional()?)
    }

    pub fn count_calendar_feeds(&self, account_id: &str) -> Result<usize> {
        let conn = self.reader();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM calendar_feeds WHERE account_id = ?1",
            params![account_id],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Remove a feed together with the calendar and event rows it was mirrored
    /// into, in one transaction. `NotFound` when the account has no such feed
    /// (nothing is touched then). The vault entry is the service's job.
    pub fn delete_calendar_feed(&self, account_id: &str, feed_id: &str) -> Result<()> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let removed = tx.execute(
            "DELETE FROM calendar_feeds WHERE id = ?1 AND account_id = ?2",
            params![feed_id, account_id],
        )?;
        if removed == 0 {
            return Err(AppError::NotFound(format!("calendar feed '{feed_id}' not found")));
        }
        tx.execute(
            "DELETE FROM calendar_events WHERE account_id = ?1 AND calendar_id = ?2",
            params![account_id, feed_id],
        )?;
        tx.execute(
            "DELETE FROM calendars WHERE account_id = ?1 AND provider_calendar_id = ?2",
            params![account_id, feed_id],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::calendars::calendar_row_id;
    use crate::models::{Calendar, CalendarEvent, CalendarFeed};

    fn feed(id: &str, account_id: &str, label: &str) -> CalendarFeed {
        CalendarFeed {
            id: id.to_string(),
            account_id: account_id.to_string(),
            label: label.to_string(),
            host: "calendar.example.com".to_string(),
            created_at: 1_000,
        }
    }

    fn calendar(account_id: &str, provider_calendar_id: &str) -> Calendar {
        Calendar {
            id: calendar_row_id(account_id, provider_calendar_id),
            account_id: account_id.to_string(),
            provider_calendar_id: provider_calendar_id.to_string(),
            name: provider_calendar_id.to_string(),
            color: String::new(),
            is_primary: false,
            access_role: "reader".to_string(),
            is_visible: true,
            sort_order: 0,
            feed_host: None,
            created_at: 1_000,
            updated_at: 1_000,
        }
    }

    fn event(account_id: &str, calendar_id: &str, provider_event_id: &str) -> CalendarEvent {
        CalendarEvent {
            id: format!("{account_id}:{calendar_id}:{provider_event_id}"),
            account_id: account_id.to_string(),
            provider_event_id: provider_event_id.to_string(),
            calendar_id: calendar_id.to_string(),
            title: "Event".to_string(),
            description: String::new(),
            location: String::new(),
            start_time: 10_000,
            end_time: 13_600,
            is_all_day: false,
            timezone: String::new(),
            organizer: String::new(),
            attendees: vec![],
            meeting_link: None,
            meeting_platform: None,
            status: "confirmed".to_string(),
            html_link: None,
            notified_at: None,
            recurring_event_id: None,
            created_at: 1_000,
            updated_at: 1_000,
        }
    }

    #[test]
    fn insert_and_list_round_trip_in_creation_order() {
        let db = Database::new_for_testing().expect("db");
        db.seed_test_account_with_provider("acc1", "imap");
        let mut second = feed("feed-b", "acc1", "Personal");
        second.created_at = 2_000;
        db.insert_calendar_feed(&feed("feed-a", "acc1", "Work"))
            .expect("insert a");
        db.insert_calendar_feed(&second).expect("insert b");

        let feeds = db.list_calendar_feeds("acc1").expect("list");
        assert_eq!(feeds, vec![feed("feed-a", "acc1", "Work"), second]);
        assert_eq!(db.get_calendar_feed("feed-b").expect("get"), Some(feeds[1].clone()));
    }

    #[test]
    fn count_is_zero_for_an_account_without_feeds_and_scoped_by_account() {
        let db = Database::new_for_testing().expect("db");
        db.seed_test_account_with_provider("acc1", "imap");
        db.seed_test_account_with_provider("acc2", "imap");
        db.insert_calendar_feed(&feed("feed-a", "acc1", "Work"))
            .expect("insert");

        assert_eq!(db.count_calendar_feeds("acc1").expect("count"), 1);
        assert_eq!(db.count_calendar_feeds("acc2").expect("count"), 0);
        assert!(db.list_calendar_feeds("acc2").expect("list").is_empty());
    }

    #[test]
    fn delete_feed_removes_its_calendar_and_events_and_nothing_else() {
        let db = Database::new_for_testing().expect("db");
        db.seed_test_account_with_provider("acc1", "imap");
        db.insert_calendar_feed(&feed("feed-a", "acc1", "Work"))
            .expect("insert a");
        db.insert_calendar_feed(&feed("feed-b", "acc1", "Personal"))
            .expect("insert b");
        db.upsert_calendars(&[calendar("acc1", "feed-a"), calendar("acc1", "feed-b")])
            .expect("calendars");
        db.upsert_calendar_events(&[
            event("acc1", "feed-a", "e1"),
            event("acc1", "feed-a", "e2"),
            event("acc1", "feed-b", "e3"),
        ])
        .expect("events");

        db.delete_calendar_feed("acc1", "feed-a").expect("delete");

        assert_eq!(db.list_calendar_feeds("acc1").expect("list").len(), 1);
        let calendars = db.list_calendars("acc1").expect("calendars");
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].provider_calendar_id, "feed-b");
        let events = db.list_calendar_events("acc1", 0, 100_000).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].calendar_id, "feed-b");
    }

    #[test]
    fn delete_feed_of_another_account_is_not_found_and_leaves_it_intact() {
        let db = Database::new_for_testing().expect("db");
        db.seed_test_account_with_provider("acc1", "imap");
        db.seed_test_account_with_provider("acc2", "imap");
        db.insert_calendar_feed(&feed("feed-a", "acc1", "Work"))
            .expect("insert");

        let err = db.delete_calendar_feed("acc2", "feed-a").expect_err("must not delete");
        assert!(matches!(err, crate::models::error::AppError::NotFound(_)), "{err:?}");
        assert_eq!(db.count_calendar_feeds("acc1").expect("count"), 1);
    }

    #[test]
    fn list_calendars_reports_the_feed_host_only_for_feed_calendars() {
        let db = Database::new_for_testing().expect("db");
        db.seed_test_account_with_provider("acc1", "imap");
        db.insert_calendar_feed(&feed("feed-a", "acc1", "Work"))
            .expect("insert");
        db.upsert_calendars(&[calendar("acc1", "feed-a"), calendar("acc1", "primary")])
            .expect("calendars");

        let calendars = db.list_calendars("acc1").expect("list");
        let by_id: std::collections::HashMap<_, _> = calendars
            .into_iter()
            .map(|c| (c.provider_calendar_id.clone(), c))
            .collect();
        assert_eq!(by_id["feed-a"].feed_host.as_deref(), Some("calendar.example.com"));
        assert_eq!(by_id["primary"].feed_host, None);
    }

    #[test]
    fn deleting_the_account_cascades_to_its_feeds() {
        let db = Database::new_for_testing().expect("db");
        db.seed_test_account_with_provider("acc1", "imap");
        db.insert_calendar_feed(&feed("feed-a", "acc1", "Work"))
            .expect("insert");

        db.connection()
            .execute("DELETE FROM accounts WHERE id = 'acc1'", [])
            .expect("delete account");

        assert_eq!(db.get_calendar_feed("feed-a").expect("get"), None);
    }
}
