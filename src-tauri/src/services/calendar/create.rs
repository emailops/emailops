//! Create a calendar event on the provider and mirror it locally. The stored
//! row goes through the same `plan_event_rows` pipeline as sync, so meeting
//! links (including a freshly created Google Meet) are extracted identically.

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::CalendarEvent;
use crate::services::calendar::sync::plan_event_rows;
use crate::sync::calendar_provider::{CalendarProvider, NewCalendarEvent};

/// Longest event we accept from the create dialog — 14 days. Anything longer
/// is almost certainly a slipped date picker, not intent.
const MAX_EVENT_SECS: i64 = 14 * 86_400;

/// Invitee ceiling — matches Google's practical attendee limits and keeps a
/// paste accident from fanning out hundreds of invitations.
const MAX_ATTENDEES: usize = 100;

/// Minimal structural check for an invitee address: one `@` with non-empty
/// sides, no whitespace. Deliverability is the provider's problem.
fn looks_like_email(address: &str) -> bool {
    let mut parts = address.split('@');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(local), Some(domain), None) => {
            !local.is_empty() && domain.contains('.') && !address.chars().any(char::is_whitespace)
        }
        _ => false,
    }
}

/// Pure validation of a new-event request. Separated so the rules are
/// unit-testable without a provider or DB.
pub fn validate_new_event(input: &NewCalendarEvent) -> Result<()> {
    if input.title.trim().is_empty() {
        return Err(AppError::InvalidInput("event title must not be empty".to_string()));
    }
    if input.end_time <= input.start_time {
        return Err(AppError::InvalidInput("event end must be after its start".to_string()));
    }
    if input.end_time - input.start_time > MAX_EVENT_SECS {
        return Err(AppError::InvalidInput("event is longer than 14 days".to_string()));
    }
    if input.attendees.len() > MAX_ATTENDEES {
        return Err(AppError::InvalidInput(format!(
            "too many invitees (max {MAX_ATTENDEES})"
        )));
    }
    if let Some(bad) = input.attendees.iter().find(|a| !looks_like_email(a)) {
        return Err(AppError::InvalidInput(format!(
            "'{bad}' is not a valid invitee email address"
        )));
    }
    Ok(())
}

/// Create the event on the provider's primary calendar, store the returned
/// instance locally, and hand it back for immediate rendering.
pub async fn create_calendar_event(
    db: &Database,
    account_id: &str,
    provider: &dyn CalendarProvider,
    input: NewCalendarEvent,
    now: i64,
) -> Result<CalendarEvent> {
    validate_new_event(&input)?;
    let created = provider.create_event(&input).await?;
    let rows = plan_event_rows(account_id, std::slice::from_ref(&created), now);
    let row = rows
        .into_iter()
        .next()
        .ok_or_else(|| AppError::SyncError("provider returned an unusable created event".to_string()))?;
    db.upsert_calendar_events(std::slice::from_ref(&row))?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::calendar_provider::FakeCalendarProvider;

    fn input(title: &str, start: i64, end: i64) -> NewCalendarEvent {
        NewCalendarEvent {
            title: title.to_string(),
            description: String::new(),
            attendees: Vec::new(),
            start_time: start,
            end_time: end,
            time_zone: "Europe/Madrid".to_string(),
            recurrence: crate::sync::calendar_provider::EventRecurrence::None,
            request_meet_link: false,
        }
    }

    // ── validate_new_event ─────────────────────────────────────────────────

    #[test]
    fn rejects_empty_title() {
        let err = validate_new_event(&input("   ", 100, 200)).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[test]
    fn rejects_end_not_after_start() {
        assert!(validate_new_event(&input("Sync", 200, 200)).is_err());
        assert!(validate_new_event(&input("Sync", 200, 100)).is_err());
    }

    #[test]
    fn rejects_absurdly_long_events() {
        let err = validate_new_event(&input("Vacation?", 0, 15 * 86_400)).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[test]
    fn accepts_a_normal_meeting() {
        assert!(validate_new_event(&input("Standup", 100, 1_900)).is_ok());
    }

    #[test]
    fn accepts_valid_invitee_addresses() {
        let mut req = input("Sync", 100, 1_900);
        req.attendees = vec!["ana@example.com".to_string(), "b.c+tag@sub.example.org".to_string()];
        assert!(validate_new_event(&req).is_ok());
    }

    #[test]
    fn rejects_malformed_invitee_addresses() {
        for bad in [
            "not-an-email",
            "two@@example.com",
            "a b@example.com",
            "@example.com",
            "user@nodot",
        ] {
            let mut req = input("Sync", 100, 1_900);
            req.attendees = vec![bad.to_string()];
            let err = validate_new_event(&req).unwrap_err();
            assert!(matches!(err, AppError::InvalidInput(_)), "should reject {bad}");
        }
    }

    #[test]
    fn rejects_more_than_100_invitees() {
        let mut req = input("All hands", 100, 1_900);
        req.attendees = (0..101).map(|i| format!("user{i}@example.com")).collect();
        assert!(validate_new_event(&req).is_err());
    }

    #[tokio::test]
    async fn attendees_and_description_land_on_the_stored_row() {
        let db = test_db();
        let provider = FakeCalendarProvider::with_events(vec![]);
        let mut req = input("Kickoff", 1_000, 2_800);
        req.description = "Agenda: scope, dates".to_string();
        req.attendees = vec!["ana@example.com".to_string()];

        let event = create_calendar_event(&db, "acc1", &provider, req, 500)
            .await
            .expect("create");

        assert_eq!(event.description, "Agenda: scope, dates");
        assert_eq!(event.attendees.len(), 1);
        assert_eq!(event.attendees[0].email, "ana@example.com");
        assert_eq!(event.attendees[0].response, "needsAction");
    }

    // ── create_calendar_event (executor against fakes) ─────────────────────

    fn test_db() -> Database {
        let db = Database::new_for_testing().expect("test db");
        db.seed_test_account("acc1");
        db
    }

    #[tokio::test]
    async fn creates_on_provider_and_stores_locally() {
        let db = test_db();
        let provider = FakeCalendarProvider::with_events(vec![]);

        let event = create_calendar_event(&db, "acc1", &provider, input("Planning", 1_000, 2_800), 500)
            .await
            .expect("create");

        assert_eq!(event.title, "Planning");
        assert_eq!(event.account_id, "acc1");
        let sent = provider.created.lock().expect("created lock");
        assert_eq!(sent.len(), 1, "the provider must receive exactly one create call");
        let stored = db.list_calendar_events("acc1", 0, 10_000).expect("list");
        assert_eq!(stored.len(), 1, "the created event must be queryable immediately");
        assert_eq!(stored[0].provider_event_id, event.provider_event_id);
    }

    #[tokio::test]
    async fn meet_link_from_provider_is_extracted_onto_the_row() {
        let db = test_db();
        let provider = FakeCalendarProvider::with_events(vec![]);
        let mut request = input("Meet sync", 1_000, 2_800);
        request.request_meet_link = true;

        let event = create_calendar_event(&db, "acc1", &provider, request, 500)
            .await
            .expect("create");

        assert_eq!(event.meeting_platform.as_deref(), Some("meet"));
        assert!(event
            .meeting_link
            .as_deref()
            .unwrap_or_default()
            .starts_with("https://meet.google.com/"));
    }

    #[tokio::test]
    async fn provider_failure_propagates_and_stores_nothing() {
        let db = test_db();
        let provider = FakeCalendarProvider::failing("[UNAVAILABLE] create rejected");

        let result = create_calendar_event(&db, "acc1", &provider, input("Doomed", 1_000, 2_000), 500).await;

        assert!(result.is_err());
        assert!(db.list_calendar_events("acc1", 0, 10_000).expect("list").is_empty());
    }

    #[tokio::test]
    async fn invalid_input_never_reaches_the_provider() {
        let db = test_db();
        let provider = FakeCalendarProvider::with_events(vec![]);

        let result = create_calendar_event(&db, "acc1", &provider, input("", 1_000, 2_000), 500).await;

        assert!(matches!(result, Err(AppError::InvalidInput(_))));
        assert!(
            provider.created.lock().expect("lock").is_empty(),
            "no provider call on invalid input"
        );
    }
}
