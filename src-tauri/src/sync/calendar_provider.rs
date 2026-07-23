//! Provider seam for calendar sync. Gmail (Google Calendar v3) and Outlook
//! (Microsoft Graph calendarView) implement [`CalendarProvider`]; IMAP has no
//! calendar. Tests use [`FakeCalendarProvider`].
//!
//! v1 contract is deliberately simple: one call returns every event instance
//! overlapping a rolling window, recurrences already expanded by the provider
//! (Google `singleEvents=true`, Graph `calendarView`). The sync service does a
//! full-window upsert-then-sweep on each run — no sync tokens, no delta
//! bookkeeping, no 410-invalidation handling. `calendar_sync_state.sync_token`
//! stays reserved for a future incremental upgrade.

use async_trait::async_trait;

use crate::models::error::Result;

/// Whether a provider (by its `accounts.provider` string) has a calendar we
/// can sync. IMAP does not (CalDAV is out of scope for v1).
pub fn provider_supports_calendar(provider: &str) -> bool {
    matches!(provider, "gmail" | "outlook")
}

/// A provider-neutral calendar event instance, already expanded (one value per
/// occurrence). Times are UTC epoch seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderCalendarEvent {
    pub provider_event_id: String,
    pub title: String,
    pub description: String,
    pub location: String,
    pub start_time: i64,
    /// Exclusive end.
    pub end_time: i64,
    pub is_all_day: bool,
    /// Original IANA timezone as reported by the provider; empty when unknown.
    pub timezone: String,
    pub organizer: String,
    pub attendees: Vec<crate::models::CalendarAttendee>,
    /// Structured conference/join URLs in provider priority order. The
    /// meeting-link extractor consumes these before falling back to text.
    pub structured_meeting_urls: Vec<String>,
    /// "confirmed" | "tentative" | "cancelled"
    pub status: String,
    /// Provider web UI link for the event.
    pub html_link: Option<String>,
    /// Series master id for expanded recurring instances (Google
    /// `recurringEventId` / Graph `seriesMasterId`); `None` when not recurring.
    pub recurring_event_id: Option<String>,
}

/// Recurrence presets for created events — the Google Calendar-style dropdown
/// set, not arbitrary RRULEs. Each provider client translates these into its
/// native shape (`google_rrule` / `graph_recurrence`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventRecurrence {
    None,
    Daily,
    /// Weekly on the start date's weekday.
    Weekly,
    /// Monday through Friday.
    Weekdays,
    /// Monthly on the start date's day-of-month.
    Monthly,
    /// Yearly on the start date.
    Yearly,
}

impl EventRecurrence {
    /// Parse the wire value from the frontend dialog. `None` for unknown input
    /// so the command boundary can reject it explicitly.
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "weekdays" => Some(Self::Weekdays),
            "monthly" => Some(Self::Monthly),
            "yearly" => Some(Self::Yearly),
            _ => None,
        }
    }
}

/// Input for creating an event on the provider's primary calendar.
#[derive(Debug, Clone, PartialEq)]
pub struct NewCalendarEvent {
    pub title: String,
    pub description: String,
    /// Invitee email addresses. Providers send invitations when non-empty.
    pub attendees: Vec<String>,
    /// UTC epoch seconds.
    pub start_time: i64,
    /// UTC epoch seconds (exclusive end). Must be after `start_time`.
    pub end_time: i64,
    /// The user's IANA timezone (e.g. "Europe/Madrid"). Required by Google for
    /// recurring events so occurrences track local wall time across DST.
    pub time_zone: String,
    pub recurrence: EventRecurrence,
    /// Ask the provider to attach a generated conference link (Google Meet).
    /// Providers without an equivalent (Graph, for now) ignore it.
    pub request_meet_link: bool,
}

#[async_trait]
pub trait CalendarProvider: Send + Sync {
    /// Every event instance overlapping `[window_start, window_end)`,
    /// recurrences expanded. Cancelled instances may be included (callers
    /// filter on `status`).
    async fn list_events(&self, window_start: i64, window_end: i64) -> Result<Vec<ProviderCalendarEvent>>;

    /// Create an event on the primary calendar and return it as the provider
    /// now sees it (id assigned, conference link attached when requested).
    async fn create_event(&self, event: &NewCalendarEvent) -> Result<ProviderCalendarEvent>;

    /// Delete (cancel) an event. When `notify` is true, attendees receive a
    /// cancellation; `message` rides along where the provider supports it
    /// (Graph `/cancel` comment — Google's API only sends its standard
    /// cancellation email, so the message is ignored there).
    async fn delete_event(&self, provider_event_id: &str, notify: bool, message: &str) -> Result<()>;

    /// End a recurring series just before `first_removed_start` ("delete this
    /// and following events"): occurrences from that instant on stop existing,
    /// earlier ones survive.
    async fn truncate_recurring_event(&self, master_id: &str, first_removed_start: i64, notify: bool) -> Result<()>;

    /// RSVP to an invitation identified by its iCalendar UID (from the
    /// invite's .ics). `self_email` is the account's own address — the
    /// attendee whose response changes. The organizer is notified.
    async fn rsvp_by_ical_uid(&self, ical_uid: &str, response: RsvpResponse, self_email: &str) -> Result<()>;
}

/// The three invite answers. Wire values match the frontend buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RsvpResponse {
    Accepted,
    Declined,
    Tentative,
}

impl RsvpResponse {
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "accepted" => Some(Self::Accepted),
            "declined" => Some(Self::Declined),
            "tentative" => Some(Self::Tentative),
            _ => None,
        }
    }

    /// Google `responseStatus` value (also our normalized attendee value).
    pub fn as_google_status(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Declined => "declined",
            Self::Tentative => "tentative",
        }
    }

    /// Graph event action segment (`POST /me/events/{id}/<action>`).
    pub fn as_graph_action(self) -> &'static str {
        match self {
            Self::Accepted => "accept",
            Self::Declined => "decline",
            Self::Tentative => "tentativelyAccept",
        }
    }
}

/// In-memory fake for service tests. Returns the configured events (filtered
/// to the requested window) or the configured error, and records created
/// events so tests can assert on them.
#[cfg(any(test, debug_assertions))]
pub struct FakeCalendarProvider {
    pub events: Vec<ProviderCalendarEvent>,
    pub error: Option<String>,
    pub created: std::sync::Mutex<Vec<NewCalendarEvent>>,
    /// `(provider_event_id, notify, message)` per delete call.
    pub deleted: std::sync::Mutex<Vec<(String, bool, String)>>,
    /// `(master_id, first_removed_start, notify)` per truncate call.
    pub truncated: std::sync::Mutex<Vec<(String, i64, bool)>>,
    /// `(ical_uid, google_status, self_email)` per RSVP call.
    pub rsvps: std::sync::Mutex<Vec<(String, String, String)>>,
}

#[cfg(any(test, debug_assertions))]
impl FakeCalendarProvider {
    pub fn with_events(events: Vec<ProviderCalendarEvent>) -> Self {
        Self {
            events,
            error: None,
            created: std::sync::Mutex::new(Vec::new()),
            deleted: std::sync::Mutex::new(Vec::new()),
            truncated: std::sync::Mutex::new(Vec::new()),
            rsvps: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn failing(message: &str) -> Self {
        Self {
            events: Vec::new(),
            error: Some(message.to_string()),
            created: std::sync::Mutex::new(Vec::new()),
            deleted: std::sync::Mutex::new(Vec::new()),
            truncated: std::sync::Mutex::new(Vec::new()),
            rsvps: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[cfg(any(test, debug_assertions))]
#[async_trait]
impl CalendarProvider for FakeCalendarProvider {
    async fn list_events(&self, window_start: i64, window_end: i64) -> Result<Vec<ProviderCalendarEvent>> {
        if let Some(message) = &self.error {
            return Err(crate::models::error::AppError::SyncError(message.clone()));
        }
        Ok(self
            .events
            .iter()
            .filter(|e| e.start_time < window_end && e.end_time > window_start)
            .cloned()
            .collect())
    }

    async fn create_event(&self, event: &NewCalendarEvent) -> Result<ProviderCalendarEvent> {
        if let Some(message) = &self.error {
            return Err(crate::models::error::AppError::SyncError(message.clone()));
        }
        let mut created = self.created.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        created.push(event.clone());
        Ok(ProviderCalendarEvent {
            provider_event_id: format!("created-{}", created.len()),
            title: event.title.clone(),
            description: event.description.clone(),
            location: String::new(),
            start_time: event.start_time,
            end_time: event.end_time,
            is_all_day: false,
            timezone: "UTC".to_string(),
            organizer: String::new(),
            attendees: event
                .attendees
                .iter()
                .map(|email| crate::models::CalendarAttendee {
                    email: email.clone(),
                    response: "needsAction".to_string(),
                })
                .collect(),
            structured_meeting_urls: if event.request_meet_link {
                vec!["https://meet.google.com/fak-efak-efk".to_string()]
            } else {
                Vec::new()
            },
            status: "confirmed".to_string(),
            html_link: None,
            recurring_event_id: None,
        })
    }

    async fn delete_event(&self, provider_event_id: &str, notify: bool, message: &str) -> Result<()> {
        if let Some(error) = &self.error {
            return Err(crate::models::error::AppError::SyncError(error.clone()));
        }
        self.deleted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((provider_event_id.to_string(), notify, message.to_string()));
        Ok(())
    }

    async fn truncate_recurring_event(&self, master_id: &str, first_removed_start: i64, notify: bool) -> Result<()> {
        if let Some(error) = &self.error {
            return Err(crate::models::error::AppError::SyncError(error.clone()));
        }
        self.truncated
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((master_id.to_string(), first_removed_start, notify));
        Ok(())
    }

    async fn rsvp_by_ical_uid(&self, ical_uid: &str, response: RsvpResponse, self_email: &str) -> Result<()> {
        if let Some(error) = &self.error {
            return Err(crate::models::error::AppError::SyncError(error.clone()));
        }
        self.rsvps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((
                ical_uid.to_string(),
                response.as_google_status().to_string(),
                self_email.to_string(),
            ));
        Ok(())
    }
}

/// Map a failed calendar HTTP response to a typed, user-presentable error.
/// Pure so both clients share one classification and it stays unit-testable.
///
/// - 401, or 403 whose body carries a token/permission marker → `NeedsReauth`
///   (the frontend renders this as a friendly banner with a re-auth button —
///   never the raw JSON).
/// - 403 for a disabled API (`SERVICE_DISABLED` / `accessNotConfigured`) is a
///   *developer-side* problem re-auth cannot fix → friendly `SyncError`.
/// - Anything else → `SyncError` with the body truncated so a provider JSON
///   dump never lands in the UI banner.
pub(crate) fn classify_calendar_fetch_error(
    provider_label: &str,
    status: u16,
    body: &str,
    account_id: Option<&str>,
) -> crate::models::error::AppError {
    use crate::models::error::AppError;

    // 403 + these markers = the token was never granted calendar access
    // (scope unchecked on the consent screen / token predates the calendar
    // scopes). Drives the scheduler's auto-disable of the integration.
    const SCOPE_DENIED_MARKERS: &[&str] = &[
        "ACCESS_TOKEN_SCOPE_INSUFFICIENT",
        "insufficientPermissions",
        "insufficient authentication scopes",
        "ErrorAccessDenied",
        "Authorization_RequestDenied",
    ];
    // Token-level failures: re-auth fixes them, and they must NOT flip the
    // user's calendar toggle.
    const REAUTH_MARKERS: &[&str] = &["InvalidAuthenticationToken"];
    const API_DISABLED_MARKERS: &[&str] = &["SERVICE_DISABLED", "accessNotConfigured"];

    if status == 403 && API_DISABLED_MARKERS.iter().any(|m| body.contains(m)) {
        return AppError::SyncError(format!(
            "{provider_label} calendar is not enabled for this app's API project — enable the Calendar API in the developer console"
        ));
    }
    if status == 403 && SCOPE_DENIED_MARKERS.iter().any(|m| body.contains(m)) {
        return AppError::CalendarPermissionDenied {
            account_id: account_id.unwrap_or_default().to_string(),
        };
    }
    if status == 401 || (status == 403 && REAUTH_MARKERS.iter().any(|m| body.contains(m))) {
        return AppError::NeedsReauth {
            account_id: account_id.unwrap_or_default().to_string(),
        };
    }
    let mut detail = body.trim().to_string();
    if detail.len() > 200 {
        // Keep the boundary on a char to avoid panicking on multibyte content.
        let cut = detail
            .char_indices()
            .take_while(|(i, _)| *i <= 200)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        detail.truncate(cut);
        detail.push('…');
    }
    AppError::SyncError(format!(
        "{provider_label} calendar sync failed (HTTP {status}): {detail}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::error::AppError;

    #[test]
    fn gmail_and_outlook_support_calendar_imap_does_not() {
        assert!(provider_supports_calendar("gmail"));
        assert!(provider_supports_calendar("outlook"));
        assert!(!provider_supports_calendar("imap"));
        assert!(!provider_supports_calendar(""));
    }

    // ── classify_calendar_fetch_error ──────────────────────────────────────

    #[test]
    fn google_scope_insufficient_403_maps_to_permission_denied_with_account() {
        // The exact failure from the wild: token predates the calendar scope
        // (or the user unchecked calendar on the consent screen). This is the
        // signal the scheduler uses to auto-disable the integration.
        let body = r#"{ "error": { "code": 403, "message": "Request had insufficient authentication scopes.", "status": "PERMISSION_DENIED", "details": [ { "reason": "ACCESS_TOKEN_SCOPE_INSUFFICIENT" } ] } }"#;
        let err = classify_calendar_fetch_error("Google", 403, body, Some("acc-1"));
        assert!(
            matches!(&err, AppError::CalendarPermissionDenied { account_id } if account_id == "acc-1"),
            "got: {err:?}"
        );
    }

    #[test]
    fn graph_access_denied_403_maps_to_permission_denied() {
        let body =
            r#"{"error":{"code":"ErrorAccessDenied","message":"Access is denied. Check credentials and try again."}}"#;
        let err = classify_calendar_fetch_error("Outlook", 403, body, Some("acc-2"));
        assert!(matches!(err, AppError::CalendarPermissionDenied { .. }), "got: {err:?}");
    }

    #[test]
    fn plain_401_maps_to_needs_reauth_not_permission_denied() {
        // An expired/invalid token is a transient auth problem — it must NOT
        // auto-disable the calendar integration.
        let err = classify_calendar_fetch_error("Google", 401, "", Some("acc-1"));
        assert!(matches!(err, AppError::NeedsReauth { .. }), "got: {err:?}");
    }

    #[test]
    fn invalid_token_403_maps_to_needs_reauth_not_permission_denied() {
        let body = r#"{"error":{"code":"InvalidAuthenticationToken","message":"Access token has expired."}}"#;
        let err = classify_calendar_fetch_error("Outlook", 403, body, Some("acc-2"));
        assert!(matches!(err, AppError::NeedsReauth { .. }), "got: {err:?}");
    }

    #[test]
    fn api_disabled_403_is_a_sync_error_not_reauth() {
        // Re-auth cannot fix a disabled API — don't send the user in circles.
        let body = r#"{"error":{"status":"PERMISSION_DENIED","details":[{"reason":"SERVICE_DISABLED"}]}}"#;
        let err = classify_calendar_fetch_error("Google", 403, body, Some("acc-1"));
        match err {
            AppError::SyncError(msg) => assert!(msg.contains("enable the Calendar API"), "got: {msg}"),
            other => panic!("expected SyncError, got {other:?}"),
        }
    }

    #[test]
    fn generic_failure_truncates_the_body() {
        let body = "x".repeat(500);
        let err = classify_calendar_fetch_error("Google", 500, &body, None);
        match err {
            AppError::SyncError(msg) => {
                assert!(
                    msg.len() < 300,
                    "raw provider payloads must not reach the banner: {}",
                    msg.len()
                );
                assert!(msg.contains("HTTP 500"));
                assert!(msg.ends_with('…'));
            }
            other => panic!("expected SyncError, got {other:?}"),
        }
    }

    #[test]
    fn generic_403_without_markers_stays_a_sync_error() {
        let err = classify_calendar_fetch_error("Google", 403, "rate limit exceeded for project", Some("acc-1"));
        assert!(matches!(err, AppError::SyncError(_)), "got: {err:?}");
    }
}
