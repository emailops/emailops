//! Delete (cancel) a calendar event on the provider and drop the local mirror.
//! Provider-first, local-second: if the provider call fails, the local rows
//! stay so the user still sees events that, in reality, still exist.

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::sync::calendar_provider::CalendarProvider;

/// How much of a recurring series a delete affects. Non-recurring events only
/// ever use `Instance`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteScope {
    /// Just the clicked occurrence (or the whole event when non-recurring).
    Instance,
    /// The clicked occurrence and everything after it (series truncated).
    Following,
    /// The entire series, past occurrences included.
    All,
}

impl DeleteScope {
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "instance" => Some(Self::Instance),
            "following" => Some(Self::Following),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

#[allow(clippy::too_many_arguments)] // one argument per provider call parameter
pub async fn delete_calendar_event(
    db: &Database,
    account_id: &str,
    provider: &dyn CalendarProvider,
    calendar_id: &str,
    provider_event_id: &str,
    scope: DeleteScope,
    notify_attendees: bool,
    cancellation_message: &str,
) -> Result<()> {
    let event = db
        .get_calendar_event_by_provider_id(account_id, calendar_id, provider_event_id)?
        .ok_or_else(|| AppError::NotFound(format!("calendar event '{provider_event_id}' not found")))?;

    match (scope, event.recurring_event_id.as_deref()) {
        // Non-recurring events: every scope collapses to a plain delete.
        (_, None) | (DeleteScope::Instance, Some(_)) => {
            provider
                .delete_event(calendar_id, provider_event_id, notify_attendees, cancellation_message)
                .await?;
            db.delete_calendar_event(account_id, calendar_id, provider_event_id)?;
        }
        (DeleteScope::All, Some(master_id)) => {
            provider
                .delete_event(calendar_id, master_id, notify_attendees, cancellation_message)
                .await?;
            db.delete_calendar_events_for_master(account_id, calendar_id, master_id)?;
        }
        (DeleteScope::Following, Some(master_id)) => {
            provider
                .truncate_recurring_event(calendar_id, master_id, event.start_time, notify_attendees)
                .await?;
            db.delete_calendar_events_for_master_from(account_id, calendar_id, master_id, event.start_time)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CalendarEvent;
    use crate::sync::calendar_provider::FakeCalendarProvider;

    fn base_event(id: &str, start: i64) -> CalendarEvent {
        CalendarEvent {
            id: format!("acc1:primary:{id}"),
            account_id: "acc1".to_string(),
            provider_event_id: id.to_string(),
            calendar_id: "primary".to_string(),
            title: "Doomed".to_string(),
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

    fn seeded_db_with_event() -> Database {
        let db = Database::new_for_testing().expect("test db");
        db.seed_test_account("acc1");
        db.upsert_calendar_events(&[base_event("ev1", 1_000)])
            .expect("seed event");
        db
    }

    fn seeded_db_with_series() -> Database {
        let db = Database::new_for_testing().expect("test db");
        db.seed_test_account("acc1");
        let mut instances = Vec::new();
        for (id, start) in [("m_1", 1_000), ("m_2", 2_000), ("m_3", 3_000)] {
            let mut e = base_event(id, start);
            e.recurring_event_id = Some("m".to_string());
            instances.push(e);
        }
        db.upsert_calendar_events(&instances).expect("seed series");
        db
    }

    #[tokio::test]
    async fn deletes_on_provider_then_locally_with_cancellation_details() {
        let db = seeded_db_with_event();
        let provider = FakeCalendarProvider::with_events(vec![]);

        delete_calendar_event(
            &db,
            "acc1",
            &provider,
            "primary",
            "ev1",
            DeleteScope::Instance,
            true,
            "Moving to next week, sorry!",
        )
        .await
        .expect("delete");

        let deleted = provider.deleted.lock().expect("lock");
        assert_eq!(
            *deleted,
            vec![(
                "primary".to_string(),
                "ev1".to_string(),
                true,
                "Moving to next week, sorry!".to_string()
            )]
        );
        assert!(db.list_calendar_events("acc1", 0, 10_000).expect("list").is_empty());
    }

    #[tokio::test]
    async fn silent_delete_passes_notify_false() {
        let db = seeded_db_with_event();
        let provider = FakeCalendarProvider::with_events(vec![]);

        delete_calendar_event(
            &db,
            "acc1",
            &provider,
            "primary",
            "ev1",
            DeleteScope::Instance,
            false,
            "",
        )
        .await
        .expect("delete");

        let deleted = provider.deleted.lock().expect("lock");
        assert!(!deleted[0].2, "notify flag must pass through as false");
    }

    #[tokio::test]
    async fn provider_failure_keeps_the_local_event() {
        let db = seeded_db_with_event();
        let provider = FakeCalendarProvider::failing("[UNAVAILABLE] delete rejected");

        let result = delete_calendar_event(
            &db,
            "acc1",
            &provider,
            "primary",
            "ev1",
            DeleteScope::Instance,
            true,
            "",
        )
        .await;

        assert!(result.is_err());
        assert_eq!(
            db.list_calendar_events("acc1", 0, 10_000).expect("list").len(),
            1,
            "a failed provider delete must not silently drop the local mirror"
        );
    }

    #[tokio::test]
    async fn instance_scope_on_a_series_removes_only_that_occurrence() {
        let db = seeded_db_with_series();
        let provider = FakeCalendarProvider::with_events(vec![]);

        delete_calendar_event(
            &db,
            "acc1",
            &provider,
            "primary",
            "m_2",
            DeleteScope::Instance,
            false,
            "",
        )
        .await
        .expect("delete");

        let deleted = provider.deleted.lock().expect("lock");
        assert_eq!(
            deleted[0].1, "m_2",
            "the instance id, not the master, is deleted upstream"
        );
        let remaining: Vec<String> = db
            .list_calendar_events("acc1", 0, 10_000)
            .expect("list")
            .into_iter()
            .map(|e| e.provider_event_id)
            .collect();
        assert_eq!(remaining, vec!["m_1", "m_3"]);
    }

    #[tokio::test]
    async fn all_scope_deletes_the_master_and_every_local_instance() {
        let db = seeded_db_with_series();
        let provider = FakeCalendarProvider::with_events(vec![]);

        delete_calendar_event(&db, "acc1", &provider, "primary", "m_2", DeleteScope::All, true, "")
            .await
            .expect("delete");

        let deleted = provider.deleted.lock().expect("lock");
        assert_eq!(deleted[0].1, "m", "whole-series delete targets the master id");
        assert!(db.list_calendar_events("acc1", 0, 10_000).expect("list").is_empty());
    }

    #[tokio::test]
    async fn following_scope_truncates_the_series_at_the_clicked_instance() {
        let db = seeded_db_with_series();
        let provider = FakeCalendarProvider::with_events(vec![]);

        delete_calendar_event(
            &db,
            "acc1",
            &provider,
            "primary",
            "m_2",
            DeleteScope::Following,
            true,
            "",
        )
        .await
        .expect("delete");

        let truncated = provider.truncated.lock().expect("lock");
        assert_eq!(*truncated, vec![("primary".to_string(), "m".to_string(), 2_000, true)]);
        assert!(
            provider.deleted.lock().expect("lock").is_empty(),
            "no plain delete on Following"
        );
        let remaining: Vec<String> = db
            .list_calendar_events("acc1", 0, 10_000)
            .expect("list")
            .into_iter()
            .map(|e| e.provider_event_id)
            .collect();
        assert_eq!(remaining, vec!["m_1"], "the clicked and later occurrences are gone");
    }

    #[tokio::test]
    async fn unknown_event_is_a_not_found_error() {
        let db = seeded_db_with_event();
        let provider = FakeCalendarProvider::with_events(vec![]);
        let result = delete_calendar_event(
            &db,
            "acc1",
            &provider,
            "primary",
            "ghost",
            DeleteScope::Instance,
            false,
            "",
        )
        .await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }
}
