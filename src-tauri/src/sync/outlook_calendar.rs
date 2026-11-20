//! Microsoft Graph calendar client. Fetches expanded event instances via
//! `/me/calendarView` (recurrences pre-expanded by Graph) over a time window.
//! Requests `Prefer: outlook.timezone="UTC"` so every dateTime arrives in UTC.
//! Parsing is a pure function ([`parse_graph_event`]) for HTTP-free tests.

use async_trait::async_trait;
use reqwest::{Client, Response, StatusCode};
use std::time::Duration;
use tokio::time::sleep;

use crate::models::error::{AppError, Result};
use crate::services::calendar::import::{EventTime, ImportableEvent};
use crate::sync::calendar_provider::{
    CalendarProvider, FoundEvent, ProviderCalendar, ProviderCalendarEvent, RsvpResponse,
};
use crate::sync::http_retry::{classify_attempt, Attempt, RetryDecision, RetryPolicy};

const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0";
const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 1_000;

/// Fields fetched for every event — enough to build a `ProviderCalendarEvent`
/// without a second round-trip.
const EVENT_SELECT_FIELDS: &str = "id,subject,body,bodyPreview,location,start,end,isAllDay,isCancelled,\
    organizer,attendees,onlineMeeting,onlineMeetingUrl,webLink,originalStartTimeZone";

/// Fields fetched for every calendar in `/me/calendars`.
const CALENDAR_SELECT_FIELDS: &str = "id,name,hexColor,color,isDefaultCalendar,canEdit,owner";

pub struct OutlookCalendarClient {
    client: Client,
    access_token: std::sync::Mutex<String>,
    refresh_token: Option<String>,
    account_id: Option<String>,
    base_url: String,
}

impl OutlookCalendarClient {
    pub fn new(access_token: String, refresh_token: Option<String>, account_id: Option<String>) -> Self {
        Self {
            client: crate::sync::http_client::provider_http_client(crate::sync::http_client::API_REQUEST_TIMEOUT),
            access_token: std::sync::Mutex::new(access_token),
            refresh_token,
            account_id,
            base_url: GRAPH_API_BASE.to_string(),
        }
    }

    /// Test-only base URL override (wiremock).
    #[allow(dead_code)]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    async fn refresh_access_token(&self) -> Result<()> {
        let Some(refresh_token) = &self.refresh_token else {
            return Err(AppError::AuthError(
                "Outlook session expired and no refresh token is stored. Please re-authenticate.".to_string(),
            ));
        };
        let Some(account_id) = &self.account_id else {
            return Err(AppError::AuthError(
                "Outlook calendar token refresh failed: account ID unknown.".to_string(),
            ));
        };
        let config = crate::sync::oauth::OAuthConfig::for_provider("outlook");
        let mut new_tokens = crate::sync::oauth::refresh_oauth_token(&config, refresh_token).await?;
        crate::services::accounts::store_tokens(account_id, &new_tokens)?;
        *self
            .access_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = std::mem::take(&mut new_tokens.access_token);
        Ok(())
    }

    /// The provider error for a non-success response.
    async fn failure(&self, response: Response) -> AppError {
        let status = response.status().as_u16();
        let error_text = response.text().await.unwrap_or_default();
        crate::sync::calendar_provider::classify_calendar_fetch_error(
            "Outlook",
            status,
            &error_text,
            self.account_id.as_deref(),
        )
    }

    /// The id of the first event of the primary mailbox matching `filter`.
    async fn first_event_id(&self, filter: &str) -> Result<Option<String>> {
        let url = format!(
            "{}/me/events?$filter={}&$select=id&$top=1",
            self.base_url,
            urlencoding::encode(filter)
        );
        let response = self.send_get_with_retry(&url, "find event by uid").await?;
        if !response.status().is_success() {
            return Err(self.failure(response).await);
        }
        let page: serde_json::Value = response.json().await?;
        Ok(page
            .get("value")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|e| e.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string))
    }

    /// Delete the instances an EXDATE removes and patch the ones an override
    /// replaces, on a series that was just created.
    async fn apply_exceptions(&self, series_id: &str, event: &ImportableEvent) -> Result<()> {
        for exdate in &event.exdates {
            let instance_id = self.find_instance_id(series_id, exdate).await?;
            self.delete_event("primary", &instance_id, false, "").await?;
        }
        for over in &event.overrides {
            let original = over.recurrence_id.as_ref().ok_or_else(|| {
                AppError::InvalidInput("an override without a RECURRENCE-ID cannot be imported".to_string())
            })?;
            let instance_id = self.find_instance_id(series_id, original).await?;
            let body = build_graph_override_patch(over)?;
            let url = format!("{}/me/events/{}", self.base_url, urlencoding::encode(&instance_id));
            let response = self
                .send_request_with_retry("patch instance", |client, token| {
                    client
                        .patch(&url)
                        .bearer_auth(token)
                        .header("Prefer", "outlook.timezone=\"UTC\"")
                        .json(&body)
                })
                .await?;
            if !response.status().is_success() {
                return Err(self.failure(response).await);
            }
        }
        Ok(())
    }

    /// The id of the series instance whose original start is `original`.
    async fn find_instance_id(&self, series_id: &str, original: &EventTime) -> Result<String> {
        let (from, to) = match original {
            EventTime::AllDay(date) => (
                date.and_time(chrono::NaiveTime::MIN).and_utc().timestamp() - SECONDS_PER_DAY,
                date.and_time(chrono::NaiveTime::MIN).and_utc().timestamp() + 2 * SECONDS_PER_DAY,
            ),
            EventTime::Timed { .. } => {
                let at = original.to_epoch();
                (at - SECONDS_PER_DAY, at + SECONDS_PER_DAY)
            }
        };
        let url = format!(
            "{}/me/events/{}/instances?startDateTime={}&endDateTime={}",
            self.base_url,
            urlencoding::encode(series_id),
            iso_utc(from)?,
            iso_utc(to)?
        );
        let response = self.send_get_with_retry(&url, "list instances").await?;
        if !response.status().is_success() {
            return Err(self.failure(response).await);
        }
        let page: serde_json::Value = response.json().await?;
        page.get("value")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .find(|instance| {
                instance
                    .get("originalStart")
                    .and_then(|v| v.as_str())
                    .is_some_and(|start| instance_starts_at(start, original))
            })
            .and_then(|instance| instance.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| AppError::SyncError("Outlook has no instance at the original start of an exception".into()))
    }

    async fn send_get_with_retry(&self, url: &str, operation: &str) -> Result<Response> {
        self.send_request_with_retry(operation, |client, token| {
            client
                .get(url)
                .bearer_auth(token)
                .header("Prefer", "outlook.timezone=\"UTC\"")
        })
        .await
    }

    /// POST for creates and actions that email attendees (create, cancel,
    /// RSVP): never re-sent after the server may have acted on it.
    async fn send_post_json_no_resend(&self, url: &str, body: &serde_json::Value, operation: &str) -> Result<Response> {
        self.send_request_with_policy(operation, RetryPolicy::NoRetryAfterSend, |client, token| {
            client
                .post(url)
                .bearer_auth(token)
                .header("Prefer", "outlook.timezone=\"UTC\"")
                .json(body)
        })
        .await
    }

    /// Shared retry loop: transparent 401 refresh (once), exponential backoff
    /// on 429/5xx and transport errors. The builder closure re-creates the
    /// request each attempt so retried requests carry the refreshed token.
    async fn send_request_with_retry<F>(&self, operation: &str, build: F) -> Result<Response>
    where
        F: Fn(&Client, &str) -> reqwest::RequestBuilder,
    {
        self.send_request_with_policy(operation, RetryPolicy::Idempotent, build)
            .await
    }

    /// The retry loop, under an explicit [`RetryPolicy`]: creates and sends go
    /// through [`RetryPolicy::NoRetryAfterSend`] so a 5xx never duplicates them.
    async fn send_request_with_policy<F>(&self, operation: &str, policy: RetryPolicy, build: F) -> Result<Response>
    where
        F: Fn(&Client, &str) -> reqwest::RequestBuilder,
    {
        let mut backoff_ms = INITIAL_BACKOFF_MS;
        let mut refreshed = false;
        // Carried so the give-up error names the real cause instead of a bare
        // "failed after N retries".
        let mut last_cause = String::from("no attempt was made");

        for attempt in 0..=MAX_RETRIES {
            let token = self
                .access_token
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let response = build(&self.client, &token).send().await;

            let outcome = match &response {
                Ok(resp) => Attempt::Status(resp.status().as_u16()),
                Err(e) if e.is_connect() => Attempt::ConnectError,
                Err(_) => Attempt::TransportError,
            };
            match &response {
                Ok(resp) => last_cause = format!("HTTP {}", resp.status()),
                Err(e) => last_cause = e.to_string(),
            }

            match classify_attempt(outcome, attempt, MAX_RETRIES, refreshed, policy) {
                // Only a genuine success reaches the caller. A throttled or 5xx
                // response must never be handed back as a valid payload.
                RetryDecision::Return => {
                    return response
                        .map_err(|e| AppError::SyncError(format!("Outlook calendar {operation} failed: {e}")))
                }
                RetryDecision::RefreshAndRetry => {
                    refreshed = true;
                    self.refresh_access_token().await?;
                }
                RetryDecision::Backoff => {
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms *= 2;
                }
                RetryDecision::GiveUp => break,
            }
        }

        Err(AppError::SyncError(format!(
            "Outlook calendar {operation} failed after {} attempts: {last_cause}",
            MAX_RETRIES + 1
        )))
    }
}

#[async_trait]
impl CalendarProvider for OutlookCalendarClient {
    async fn list_calendars(&self) -> Result<Vec<ProviderCalendar>> {
        let mut calendars = Vec::new();
        let mut url = format!(
            "{}/me/calendars?$top=100&$select={}",
            self.base_url, CALENDAR_SELECT_FIELDS
        );
        loop {
            let response = self.send_get_with_retry(&url, "list calendars").await?;
            if !response.status().is_success() {
                let status = response.status().as_u16();
                let error_text = response.text().await.unwrap_or_default();
                return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                    "Outlook",
                    status,
                    &error_text,
                    self.account_id.as_deref(),
                ));
            }
            let page: serde_json::Value = response.json().await?;
            if let Some(items) = page.get("value").and_then(|v| v.as_array()) {
                calendars.extend(items.iter().filter_map(parse_graph_calendar));
            }
            match page.get("@odata.nextLink").and_then(|v| v.as_str()) {
                Some(next) => url = next.to_string(),
                None => break,
            }
        }
        Ok(calendars)
    }

    async fn list_events(
        &self,
        calendar_id: &str,
        window_start: i64,
        window_end: i64,
    ) -> Result<Vec<ProviderCalendarEvent>> {
        let start = epoch_to_rfc3339(window_start)?;
        let end = epoch_to_rfc3339(window_end)?;
        let mut events = Vec::new();
        let mut url = format!(
            "{}/me/calendars/{}/calendarView?startDateTime={}&endDateTime={}&$top=100&$select={}",
            self.base_url,
            urlencoding::encode(calendar_id),
            urlencoding::encode(&start),
            urlencoding::encode(&end),
            EVENT_SELECT_FIELDS,
        );
        loop {
            let response = self.send_get_with_retry(&url, "list events").await?;
            if !response.status().is_success() {
                let status = response.status().as_u16();
                let error_text = response.text().await.unwrap_or_default();
                return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                    "Outlook",
                    status,
                    &error_text,
                    self.account_id.as_deref(),
                ));
            }
            let page: serde_json::Value = response.json().await?;
            if let Some(items) = page.get("value").and_then(|v| v.as_array()) {
                events.extend(items.iter().filter_map(parse_graph_event));
            }
            match page.get("@odata.nextLink").and_then(|v| v.as_str()) {
                Some(next) => url = next.to_string(),
                None => break,
            }
        }
        Ok(events)
    }

    async fn create_event(
        &self,
        event: &crate::sync::calendar_provider::NewCalendarEvent,
    ) -> Result<ProviderCalendarEvent> {
        let url = format!("{}/me/events", self.base_url);
        let body = build_graph_create_request(event)?;
        let response = self.send_post_json_no_resend(&url, &body, "create event").await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                "Outlook",
                status,
                &error_text,
                self.account_id.as_deref(),
            ));
        }
        let raw: serde_json::Value = response.json().await?;
        parse_graph_event(&raw)
            .ok_or_else(|| AppError::SyncError("Outlook returned an unparseable created event".to_string()))
    }

    async fn delete_event(
        &self,
        // Graph addresses every event under `/me/events/{id}` regardless of
        // which calendar holds it, so the calendar is not part of the URL.
        _calendar_id: &str,
        provider_event_id: &str,
        notify: bool,
        message: &str,
    ) -> Result<()> {
        // With attendees to notify, Graph's `/cancel` action sends the
        // cancellation and supports an organizer comment; a plain DELETE
        // removes silently.
        let response = if notify {
            let url = format!(
                "{}/me/events/{}/cancel",
                self.base_url,
                urlencoding::encode(provider_event_id)
            );
            let body = serde_json::json!({ "comment": message });
            self.send_post_json_no_resend(&url, &body, "cancel event").await?
        } else {
            let url = format!("{}/me/events/{}", self.base_url, urlencoding::encode(provider_event_id));
            self.send_request_with_retry("delete event", |client, token| client.delete(&url).bearer_auth(token))
                .await?
        };
        let status = response.status();
        // 404/410: already gone upstream — the local mirror should still drop it.
        if status.is_success() || status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
            return Ok(());
        }
        let error_text = response.text().await.unwrap_or_default();
        Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
            "Outlook",
            status.as_u16(),
            &error_text,
            self.account_id.as_deref(),
        ))
    }

    async fn truncate_recurring_event(
        &self,
        // See `delete_event`: Graph event ids are calendar-independent.
        _calendar_id: &str,
        master_id: &str,
        first_removed_start: i64,
        _notify: bool,
    ) -> Result<()> {
        // Graph sends series-change updates to attendees itself; there is no
        // per-call notify switch on PATCH (`_notify` intentionally unused).
        let master_url = format!("{}/me/events/{}", self.base_url, urlencoding::encode(master_id));
        let response = self
            .send_get_with_retry(&format!("{master_url}?$select=recurrence"), "get recurring master")
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                "Outlook",
                status,
                &error_text,
                self.account_id.as_deref(),
            ));
        }
        let master: serde_json::Value = response.json().await?;
        let recurrence = master
            .get("recurrence")
            .and_then(|r| graph_recurrence_with_end(r, first_removed_start))
            .ok_or_else(|| AppError::SyncError("event series has no recurrence rule to truncate".to_string()))?;
        let body = serde_json::json!({ "recurrence": recurrence });
        let response = self
            .send_request_with_retry("truncate series", |client, token| {
                client
                    .patch(&master_url)
                    .bearer_auth(token)
                    .header("Prefer", "outlook.timezone=\"UTC\"")
                    .json(&body)
            })
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                "Outlook",
                status,
                &error_text,
                self.account_id.as_deref(),
            ));
        }
        Ok(())
    }

    async fn rsvp_by_ical_uid(
        &self,
        ical_uid: &str,
        response: crate::sync::calendar_provider::RsvpResponse,
        _self_email: &str,
    ) -> Result<()> {
        // Graph's accept/decline/tentativelyAccept actions operate on "me" —
        // no attendee surgery needed (`_self_email` unused by design).
        let lookup_url = format!(
            "{}/me/events?$filter={}&$select=id&$top=1",
            self.base_url,
            urlencoding::encode(&outlook_uid_filter(ical_uid))
        );
        let lookup = self.send_get_with_retry(&lookup_url, "find invite event").await?;
        if !lookup.status().is_success() {
            let status = lookup.status().as_u16();
            let error_text = lookup.text().await.unwrap_or_default();
            return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                "Outlook",
                status,
                &error_text,
                self.account_id.as_deref(),
            ));
        }
        let page: serde_json::Value = lookup.json().await?;
        let event_id = page
            .get("value")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .and_then(|e| e.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AppError::NotFound(
                    "This invitation hasn't reached your calendar yet — try again in a moment.".to_string(),
                )
            })?
            .to_string();

        let action_url = format!(
            "{}/me/events/{}/{}",
            self.base_url,
            urlencoding::encode(&event_id),
            response.as_graph_action()
        );
        let body = serde_json::json!({ "sendResponse": true });
        let acted = self.send_post_json_no_resend(&action_url, &body, "rsvp").await?;
        if !acted.status().is_success() {
            let status = acted.status().as_u16();
            let error_text = acted.text().await.unwrap_or_default();
            return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                "Outlook",
                status,
                &error_text,
                self.account_id.as_deref(),
            ));
        }
        Ok(())
    }

    async fn find_event_by_uid(&self, ical_uid: &str) -> Result<Option<FoundEvent>> {
        // Our own copy first: an event we imported is what a re-import must
        // recognise even if its iCalUId differs from the file's UID.
        if let Some(id) = self.first_event_id(&outlook_imported_uid_filter(ical_uid)).await? {
            return Ok(Some(FoundEvent {
                provider_event_id: id,
                imported_by_emailops: true,
            }));
        }
        Ok(self
            .first_event_id(&outlook_uid_filter(ical_uid))
            .await?
            .map(|id| FoundEvent {
                provider_event_id: id,
                imported_by_emailops: false,
            }))
    }

    async fn import_event(&self, event: &ImportableEvent) -> Result<String> {
        let body = build_graph_import_request(event)?;
        let url = format!("{}/me/events", self.base_url);
        let response = self.send_post_json_no_resend(&url, &body, "import event").await?;
        if !response.status().is_success() {
            return Err(self.failure(response).await);
        }
        let created: serde_json::Value = response.json().await?;
        let series_id = created
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::SyncError("Outlook returned an imported event without an id".to_string()))?
            .to_string();
        if let Err(error) = self.apply_exceptions(&series_id, event).await {
            // All or nothing: drop the half-imported series. The rollback is
            // best effort; the import error is what the caller needs.
            if let Err(rollback) = self.delete_event("primary", &series_id, false, "").await {
                // Code only: the error text can embed a provider response body.
                crate::services::logger::log(
                    "error",
                    "sync",
                    format!(
                        "Outlook Calendar import rollback failed ({}); an orphan event may remain",
                        rollback.code()
                    ),
                );
            }
            return Err(error);
        }
        Ok(series_id)
    }

    async fn respond_to_event(
        &self,
        // Graph ids are mailbox-global, so the calendar is not part of the URL.
        _calendar_id: &str,
        provider_event_id: &str,
        response: RsvpResponse,
        _self_email: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/me/events/{}/{}",
            self.base_url,
            urlencoding::encode(provider_event_id),
            response.as_graph_action()
        );
        let body = serde_json::json!({ "sendResponse": true });
        let acted = self.send_post_json_no_resend(&url, &body, "rsvp").await?;
        if !acted.status().is_success() {
            return Err(self.failure(acted).await);
        }
        Ok(())
    }
}

const SECONDS_PER_DAY: i64 = 86_400;

fn iso_utc(epoch: i64) -> Result<String> {
    chrono::DateTime::from_timestamp(epoch, 0)
        .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .ok_or_else(|| AppError::InvalidInput("event time out of range".to_string()))
}

/// Whether a Graph `originalStart` (UTC, with or without fraction and `Z`) is
/// the occurrence `original` names: the same instant, or the same date for an
/// all-day occurrence.
fn instance_starts_at(original_start: &str, original: &EventTime) -> bool {
    let Some(stamp) = original_start.get(..19) else {
        return false;
    };
    let Ok(start) = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%dT%H:%M:%S") else {
        return false;
    };
    match original {
        EventTime::AllDay(date) => start.date() == *date,
        EventTime::Timed { .. } => start.and_utc().timestamp() == original.to_epoch(),
    }
}

/// The Windows zone name Graph expects for an event's IANA zone (or "UTC").
fn graph_zone_name(tz: &str) -> Result<&'static str> {
    if tz == "UTC" {
        return Ok("UTC");
    }
    crate::services::calendar::windows_zones::windows_name(tz)
        .ok_or_else(|| AppError::InvalidInput(format!("no Windows time zone for {tz}")))
}

fn graph_time(time: &EventTime) -> Result<serde_json::Value> {
    let (local, zone) = match time {
        EventTime::AllDay(date) => (date.and_time(chrono::NaiveTime::MIN), "UTC"),
        EventTime::Timed { local, tz } => (*local, graph_zone_name(tz)?),
    };
    Ok(serde_json::json!({"dateTime": local.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": zone}))
}

/// Body of `POST /me/events` for an import. Never carries attendees (Graph
/// would email them an invitation); the UID goes in an extended property so a
/// later import finds the event again.
pub(crate) fn build_graph_import_request(event: &ImportableEvent) -> Result<serde_json::Value> {
    if !event.rdates.is_empty() {
        return Err(AppError::InvalidInput(
            "Outlook cannot import events with RDATE".to_string(),
        ));
    }
    let mut body = serde_json::json!({
        "subject": event.summary,
        "start": graph_time(&event.start)?,
        "end": graph_time(&event.end)?,
        "isAllDay": matches!(event.start, EventTime::AllDay(_)),
        "singleValueExtendedProperties": [{"id": EMAILOPS_UID_PROPERTY, "value": event.uid}],
    });
    if !event.description.is_empty() {
        body["body"] = serde_json::json!({"contentType": "text", "content": event.description});
    }
    if !event.location.is_empty() {
        body["location"] = serde_json::json!({"displayName": event.location});
    }
    if let Some(rrule) = &event.rrule {
        // The same zone the planner checked the rule in.
        let (first_start, tz) = match &event.start {
            EventTime::AllDay(date) => (date.and_time(chrono::NaiveTime::MIN), event.fallback_tz.as_str()),
            EventTime::Timed { local, tz } => (*local, tz.as_str()),
        };
        let mut recurrence = graph_recurrence_from_rrule(rrule, first_start, tz)
            .map_err(|part| AppError::InvalidInput(format!("Outlook cannot represent the recurrence ({part})")))?;
        // The converter works in IANA (it converts UNTIL); Graph wants Windows.
        recurrence["range"]["recurrenceTimeZone"] = serde_json::json!(graph_zone_name(tz)?);
        body["recurrence"] = recurrence;
    }
    Ok(body)
}

/// `PATCH` body that makes one instance look like its override. Empty text is
/// sent as empty so it replaces the series' text; attendees are never sent.
fn build_graph_override_patch(over: &ImportableEvent) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "subject": over.summary,
        "body": {"contentType": "text", "content": over.description},
        "location": {"displayName": over.location},
        "start": graph_time(&over.start)?,
        "end": graph_time(&over.end)?,
        "isAllDay": matches!(over.start, EventTime::AllDay(_)),
    }))
}

/// Extended property EmailOps stamps on events it imports, so a later import
/// of the same `.ics` UID finds the event again.
pub(crate) const EMAILOPS_UID_PROPERTY: &str = "String {6f5a6c1e-8d2b-4f0e-9a3c-2b7e1d4c5a90} Name EmailOpsIcalUid";

/// Hex of the MS-OXOCAL `PidLidGlobalObjectId` class id.
const GLOBAL_OBJECT_ID_CLASS: &str = "040000008200E00074C5B7101A82E008";

/// Exchange exposes `iCalUId` for events organized outside Exchange as the hex
/// of a GlobalObjectId wrapping the original UID: class id, instance date,
/// creation time and reserved bytes (all zero here), a little-endian size and
/// `"vCal-Uid" 01 00 00 00 <uid> 00`. A UID that already is such an id passes
/// through (uppercased).
pub(crate) fn exchange_global_object_id(ical_uid: &str) -> String {
    let is_hex = ical_uid.bytes().all(|b| b.is_ascii_hexdigit());
    if is_hex && ical_uid.to_ascii_uppercase().starts_with(&GLOBAL_OBJECT_ID_CLASS[..16]) {
        return ical_uid.to_ascii_uppercase();
    }
    let mut data = b"vCal-Uid\x01\x00\x00\x00".to_vec();
    data.extend_from_slice(ical_uid.as_bytes());
    data.push(0);
    let mut bytes = Vec::with_capacity(40 + data.len());
    bytes.extend_from_slice(&[
        0x04, 0x00, 0x00, 0x00, 0x82, 0x00, 0xE0, 0x00, 0x74, 0xC5, 0xB7, 0x10, 0x1A, 0x82, 0xE0, 0x08,
    ]);
    bytes.extend_from_slice(&[0u8; 20]); // instance date, creation time, reserved
    bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&data);
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// OData `$filter` matching an invite event by its iCalendar UID: the plain
/// `iCalUId` or the Exchange-wrapped one.
pub(crate) fn outlook_uid_filter(ical_uid: &str) -> String {
    let plain = ical_uid.replace('\'', "''");
    let wrapped = exchange_global_object_id(ical_uid).replace('\'', "''");
    format!("iCalUId eq '{plain}' or iCalUId eq '{wrapped}'")
}

/// OData `$filter` matching an event EmailOps imported, by the UID stored in
/// its extended property. Meant as a separate query from [`outlook_uid_filter`].
pub(crate) fn outlook_imported_uid_filter(ical_uid: &str) -> String {
    let plain = ical_uid.replace('\'', "''");
    format!("singleValueExtendedProperties/Any(ep: ep/id eq '{EMAILOPS_UID_PROPERTY}' and ep/value eq '{plain}')")
}

/// Recurrence preset → Graph `patternedRecurrence`. Pattern anchors (weekday,
/// day-of-month, month) derive from the start's UTC date — for late-evening
/// events near a zone boundary this can differ from the local date; acceptable
/// for v1 (Google, the primary provider, recurs in the user's named zone).
pub(crate) fn graph_recurrence(
    recurrence: crate::sync::calendar_provider::EventRecurrence,
    start_time: i64,
) -> Option<serde_json::Value> {
    use crate::sync::calendar_provider::EventRecurrence as R;
    use chrono::Datelike;

    let start = chrono::DateTime::from_timestamp(start_time, 0)?;
    let start_date = start.format("%Y-%m-%d").to_string();
    let weekday = match start.weekday() {
        chrono::Weekday::Mon => "monday",
        chrono::Weekday::Tue => "tuesday",
        chrono::Weekday::Wed => "wednesday",
        chrono::Weekday::Thu => "thursday",
        chrono::Weekday::Fri => "friday",
        chrono::Weekday::Sat => "saturday",
        chrono::Weekday::Sun => "sunday",
    };
    let pattern = match recurrence {
        R::None => return None,
        R::Daily => serde_json::json!({"type": "daily", "interval": 1}),
        R::Weekly => serde_json::json!({"type": "weekly", "interval": 1, "daysOfWeek": [weekday]}),
        R::Weekdays => serde_json::json!({
            "type": "weekly", "interval": 1,
            "daysOfWeek": ["monday", "tuesday", "wednesday", "thursday", "friday"]
        }),
        R::Monthly => serde_json::json!({"type": "absoluteMonthly", "interval": 1, "dayOfMonth": start.day()}),
        R::Yearly => serde_json::json!({
            "type": "absoluteYearly", "interval": 1,
            "dayOfMonth": start.day(), "month": start.month()
        }),
    };
    Some(serde_json::json!({
        "pattern": pattern,
        "range": {"type": "noEnd", "startDate": start_date}
    }))
}

/// Build the `POST /me/events` body. Pure and deterministic for tests.
/// `request_meet_link` is Google-only; Graph events are created plain (Teams
/// meeting creation needs a work/school tenant and is out of scope).
pub(crate) fn build_graph_create_request(
    event: &crate::sync::calendar_provider::NewCalendarEvent,
) -> Result<serde_json::Value> {
    let mut body = serde_json::json!({
        "subject": event.title,
        "start": {"dateTime": epoch_to_graph_datetime(event.start_time)?, "timeZone": "UTC"},
        "end": {"dateTime": epoch_to_graph_datetime(event.end_time)?, "timeZone": "UTC"},
    });
    if !event.description.is_empty() {
        body["body"] = serde_json::json!({"contentType": "text", "content": event.description});
    }
    if !event.attendees.is_empty() {
        body["attendees"] = serde_json::json!(event
            .attendees
            .iter()
            .map(|a| serde_json::json!({"emailAddress": {"address": a}, "type": "required"}))
            .collect::<Vec<_>>());
    }
    if let Some(recurrence) = graph_recurrence(event.recurrence, event.start_time) {
        body["recurrence"] = recurrence;
    }
    Ok(body)
}

/// Graph day name for an iCalendar weekday code (`MO`..`SU`).
fn graph_day_name(code: &str) -> Option<&'static str> {
    Some(match code {
        "MO" => "monday",
        "TU" => "tuesday",
        "WE" => "wednesday",
        "TH" => "thursday",
        "FR" => "friday",
        "SA" => "saturday",
        "SU" => "sunday",
        _ => return None,
    })
}

/// Parse one `BYDAY` item (`TU`, `2TU`, `-1FR`) into (ordinal, Graph day).
/// Ordinals Graph cannot express (0, 5+, below -1) are rejected.
fn parse_byday_item(item: &str) -> Option<(Option<i32>, &'static str)> {
    let split = item.len().checked_sub(2)?;
    if !item.is_char_boundary(split) {
        return None;
    }
    let (ordinal, code) = item.split_at(split);
    let day = graph_day_name(code)?;
    if ordinal.is_empty() {
        return Some((None, day));
    }
    match ordinal.parse::<i32>().ok()? {
        n @ (-1 | 1..=4) => Some((Some(n), day)),
        _ => None,
    }
}

fn graph_index_name(ordinal: i32) -> &'static str {
    match ordinal {
        1 => "first",
        2 => "second",
        3 => "third",
        4 => "fourth",
        _ => "last",
    }
}

/// `UNTIL` value → the last day of the series in `tz`, as Graph's inclusive
/// `endDate`. A trailing `Z` is converted from UTC; a floating date-time is
/// local already. A date-time earlier in its day than the series' start time
/// of day (`start_time`) excludes that day's occurrence, so the series ends
/// the day before. A date value is the last day.
fn rrule_until_date(value: &str, tz: chrono_tz::Tz, start_time: chrono::NaiveTime) -> Option<chrono::NaiveDate> {
    use chrono::TimeZone;
    let local = if let Some(utc) = value.strip_suffix('Z') {
        let naive = chrono::NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        tz.from_utc_datetime(&naive).naive_local()
    } else if let Ok(local) = chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S") {
        local
    } else {
        let date_part = value.get(..8)?;
        return chrono::NaiveDate::parse_from_str(date_part, "%Y%m%d").ok();
    };
    if local.time() < start_time {
        local.date().pred_opt()
    } else {
        Some(local.date())
    }
}

/// iCalendar `RRULE` → Graph `patternedRecurrence` (`{pattern, range}`).
/// `first_start` is the series' first start as a wall time in `tz` (an IANA
/// zone name or `"UTC"`). Rules Graph cannot express return
/// `Err(<offending rule part>)`, e.g. `"FREQ=HOURLY"` or `"BYMONTHDAY=1,15"`.
///
/// Where Graph reads a rule differently from RFC 5545, the rule is spelled out
/// for Graph or rejected, never passed through:
/// - Week start: RFC 5545 defaults `WKST` to Monday and Graph defaults
///   `firstDayOfWeek` to Sunday, so a weekly pattern always names it.
/// - Several days in a relative pattern (`BYDAY=1MO,1TU`): RFC 5545 means each
///   day, Graph only the first one that matches. Rejected.
/// - `BYMONTHDAY` in a yearly rule without `BYMONTH`: RFC 5545 repeats it in
///   every month, Graph would keep a single month. Rejected.
pub(crate) fn graph_recurrence_from_rrule(
    rrule: &str,
    first_start: chrono::NaiveDateTime,
    tz: &str,
) -> std::result::Result<serde_json::Value, String> {
    use chrono::Datelike;

    let first_day = first_start.date();
    let zone: chrono_tz::Tz = tz.parse().map_err(|_| format!("TZID={tz}"))?;
    let mut freq = None;
    let (mut interval, mut count, mut until) = (1u32, None, None);
    let (mut byday, mut bymonthday, mut bymonth, mut wkst) = (None, None, None, None);
    for part in rrule.split(';').filter(|p| !p.is_empty()) {
        let (key, value) = part.split_once('=').ok_or_else(|| part.to_string())?;
        let key = key.to_ascii_uppercase();
        let bad = || format!("{key}={value}");
        match key.as_str() {
            "FREQ" => freq = Some(value.to_ascii_uppercase()),
            "INTERVAL" => interval = value.parse().ok().filter(|n| *n >= 1).ok_or_else(bad)?,
            "COUNT" => count = Some(value.parse::<u32>().ok().filter(|n| *n >= 1).ok_or_else(bad)?),
            "UNTIL" => until = Some(rrule_until_date(value, zone, first_start.time()).ok_or_else(bad)?),
            "BYDAY" => byday = Some(value.to_ascii_uppercase()),
            "BYMONTHDAY" => {
                bymonthday = Some(
                    value
                        .parse::<u32>()
                        .ok()
                        .filter(|d| (1..=31).contains(d))
                        .ok_or_else(bad)?,
                )
            }
            "BYMONTH" => {
                bymonth = Some(
                    value
                        .parse::<u32>()
                        .ok()
                        .filter(|m| (1..=12).contains(m))
                        .ok_or_else(bad)?,
                )
            }
            "WKST" => wkst = Some(graph_day_name(&value.to_ascii_uppercase()).ok_or_else(bad)?),
            // BYSETPOS, BYWEEKNO, BYYEARDAY, BYHOUR, ...: not expressible.
            _ => return Err(key),
        }
    }
    let freq = freq.ok_or_else(|| "FREQ".to_string())?;

    // RFC 5545 forbids COUNT together with UNTIL.
    if count.is_some() && until.is_some() {
        return Err("UNTIL".to_string());
    }

    // BYDAY items: (ordinal, day).
    let by_day_text = byday.unwrap_or_default();
    let mut days: Vec<(Option<i32>, &'static str)> = Vec::new();
    if !by_day_text.is_empty() {
        for item in by_day_text.split(',') {
            days.push(parse_byday_item(item).ok_or_else(|| format!("BYDAY={by_day_text}"))?);
        }
    }
    let by_day_err = || format!("BYDAY={by_day_text}");
    // Relative patterns (monthly/yearly) take exactly one day: Graph reads
    // several as "the first one that matches", RFC 5545 as each of them.
    let ordinal = match days.as_slice() {
        [(Some(n), _)] => Some(*n),
        _ if days.iter().any(|(o, _)| o.is_some()) => return Err(by_day_err()),
        _ => None,
    };
    let day_names: Vec<&str> = days.iter().map(|(_, d)| *d).collect();
    let anchor_day = match first_day.weekday() {
        chrono::Weekday::Mon => "monday",
        chrono::Weekday::Tue => "tuesday",
        chrono::Weekday::Wed => "wednesday",
        chrono::Weekday::Thu => "thursday",
        chrono::Weekday::Fri => "friday",
        chrono::Weekday::Sat => "saturday",
        chrono::Weekday::Sun => "sunday",
    };
    // Parts the frequency does not use must not be silently dropped.
    let reject_month = |m: Option<u32>| m.map_or(Ok(()), |m| Err(format!("BYMONTH={m}")));
    let reject_monthday = |d: Option<u32>| d.map_or(Ok(()), |d| Err(format!("BYMONTHDAY={d}")));
    // RFC 5545 skips months lacking the day; Graph moves it to the last day.
    let absolute_day = || {
        let day = bymonthday.unwrap_or_else(|| first_day.day());
        if day > 28 {
            Err(format!("BYMONTHDAY={day}"))
        } else {
            Ok(day)
        }
    };

    let mut pattern = match freq.as_str() {
        "DAILY" => {
            if !days.is_empty() {
                return Err(by_day_err());
            }
            reject_month(bymonth)?;
            reject_monthday(bymonthday)?;
            serde_json::json!({"type": "daily", "interval": interval})
        }
        "WEEKLY" => {
            if days.iter().any(|(o, _)| o.is_some()) {
                return Err(by_day_err());
            }
            reject_month(bymonth)?;
            reject_monthday(bymonthday)?;
            let names = if day_names.is_empty() {
                vec![anchor_day]
            } else {
                day_names
            };
            serde_json::json!({
                "type": "weekly", "interval": interval, "daysOfWeek": names,
                "firstDayOfWeek": wkst.unwrap_or("monday")
            })
        }
        "MONTHLY" => {
            reject_month(bymonth)?;
            match ordinal {
                Some(n) => {
                    reject_monthday(bymonthday)?;
                    serde_json::json!({
                        "type": "relativeMonthly", "interval": interval,
                        "daysOfWeek": day_names, "index": graph_index_name(n)
                    })
                }
                None if days.is_empty() => serde_json::json!({
                    "type": "absoluteMonthly", "interval": interval,
                    "dayOfMonth": absolute_day()?
                }),
                None => return Err(by_day_err()),
            }
        }
        "YEARLY" => match ordinal {
            Some(n) => {
                reject_monthday(bymonthday)?;
                // Without BYMONTH the rule means "nth weekday of the year".
                let month = bymonth.ok_or_else(by_day_err)?;
                serde_json::json!({
                    "type": "relativeYearly", "interval": interval, "month": month,
                    "daysOfWeek": day_names, "index": graph_index_name(n)
                })
            }
            None if days.is_empty() => {
                // Without BYMONTH, RFC 5545 repeats BYMONTHDAY in every month.
                if bymonth.is_none() {
                    reject_monthday(bymonthday)?;
                }
                serde_json::json!({
                    "type": "absoluteYearly", "interval": interval,
                    "month": bymonth.unwrap_or_else(|| first_day.month()),
                    "dayOfMonth": absolute_day()?
                })
            }
            None => return Err(by_day_err()),
        },
        _ => return Err(format!("FREQ={freq}")),
    };
    if let Some(week_start) = wkst {
        pattern["firstDayOfWeek"] = serde_json::json!(week_start);
    }

    let start_date = first_day.format("%Y-%m-%d").to_string();
    let range = match (count, until) {
        (Some(n), _) => serde_json::json!({
            "type": "numbered", "startDate": start_date, "numberOfOccurrences": n
        }),
        (None, Some(end)) => serde_json::json!({
            "type": "endDate", "startDate": start_date,
            "endDate": end.format("%Y-%m-%d").to_string(), "recurrenceTimeZone": tz
        }),
        (None, None) => serde_json::json!({"type": "noEnd", "startDate": start_date}),
    };
    Ok(serde_json::json!({"pattern": pattern, "range": range}))
}

/// Rewrite a Graph `recurrence` object so the series ends before
/// `first_removed_start`: the pattern is preserved, the range becomes
/// `endDate` with the UTC day *before* the first removed occurrence (Graph's
/// endDate is inclusive). Returns `None` when the input has no pattern.
pub(crate) fn graph_recurrence_with_end(
    existing: &serde_json::Value,
    first_removed_start: i64,
) -> Option<serde_json::Value> {
    let pattern = existing.get("pattern")?.clone();
    let start_date = existing
        .get("range")
        .and_then(|r| r.get("startDate"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let end_date = chrono::DateTime::from_timestamp(first_removed_start.saturating_sub(86_400), 0)?
        .format("%Y-%m-%d")
        .to_string();
    Some(serde_json::json!({
        "pattern": pattern,
        "range": {"type": "endDate", "startDate": start_date, "endDate": end_date}
    }))
}

/// Graph RSVP → normalized set ("accepted" | "declined" | "tentative" |
/// "needsAction" | "organizer").
fn normalize_graph_response(graph_response: &str) -> String {
    match graph_response {
        "accepted" => "accepted",
        "declined" => "declined",
        "tentativelyAccepted" => "tentative",
        "organizer" => "organizer",
        _ => "needsAction", // "none" | "notResponded" | unknown
    }
    .to_string()
}

/// Graph wants a zone-less wall time paired with a `timeZone` field.
fn epoch_to_graph_datetime(epoch_seconds: i64) -> Result<String> {
    chrono::DateTime::from_timestamp(epoch_seconds, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S").to_string())
        .ok_or_else(|| AppError::InvalidInput(format!("timestamp {epoch_seconds} out of range")))
}

/// Parse one `/me/calendars` entry. Returns `None` for entries with no id.
///
/// Graph exposes a calendar's colour two ways: `hexColor` (an explicit
/// "#rrggbb", present when the user picked a custom colour) and `color` (a
/// named preset whose exact rendering is Outlook's, not a documented hex). We
/// take `hexColor` when it is there and otherwise leave the colour empty so
/// the UI assigns a palette slot — inventing hexes for the presets would only
/// produce colours that don't match Outlook anyway.
///
/// Pure so it is unit-testable without HTTP.
fn parse_graph_calendar(raw: &serde_json::Value) -> Option<ProviderCalendar> {
    let provider_calendar_id = raw.get("id").and_then(|v| v.as_str())?.to_string();
    let is_primary = raw.get("isDefaultCalendar").and_then(|v| v.as_bool()).unwrap_or(false);
    let can_edit = raw.get("canEdit").and_then(|v| v.as_bool()).unwrap_or(false);
    Some(ProviderCalendar {
        provider_calendar_id,
        name: raw.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        color: raw
            .get("hexColor")
            .and_then(|v| v.as_str())
            .filter(|c| c.starts_with('#'))
            .unwrap_or_default()
            .to_string(),
        is_primary,
        access_role: if is_primary {
            "owner".to_string()
        } else if can_edit {
            "writer".to_string()
        } else {
            "reader".to_string()
        },
        owner_email: raw
            .pointer("/owner/address")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        // Graph has no "shown in my UI" flag, so every calendar starts visible.
        selected: true,
    })
}

fn epoch_to_rfc3339(epoch_seconds: i64) -> Result<String> {
    chrono::DateTime::from_timestamp(epoch_seconds, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .ok_or_else(|| AppError::InvalidInput(format!("timestamp {epoch_seconds} out of range")))
}

/// Parse one Graph `event` resource into the provider-neutral shape. Returns
/// `None` when id or times are missing/unparseable.
pub(crate) fn parse_graph_event(raw: &serde_json::Value) -> Option<ProviderCalendarEvent> {
    let id = raw.get("id")?.as_str()?.to_string();
    let start_time = parse_graph_time(raw.get("start")?)?;
    let end_time = parse_graph_time(raw.get("end")?)?;
    let is_cancelled = raw.get("isCancelled").and_then(|v| v.as_bool()).unwrap_or(false);

    let mut structured_meeting_urls = Vec::new();
    if let Some(join_url) = raw
        .get("onlineMeeting")
        .and_then(|m| m.get("joinUrl"))
        .and_then(|v| v.as_str())
    {
        structured_meeting_urls.push(join_url.to_string());
    }
    if let Some(legacy) = raw.get("onlineMeetingUrl").and_then(|v| v.as_str()) {
        structured_meeting_urls.push(legacy.to_string());
    }

    let attendees = raw
        .get("attendees")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    let email = a
                        .get("emailAddress")
                        .and_then(|e| e.get("address"))
                        .and_then(|v| v.as_str())?
                        .to_string();
                    let response = normalize_graph_response(
                        a.get("status")
                            .and_then(|s| s.get("response"))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default(),
                    );
                    Some(crate::models::CalendarAttendee { email, response })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(ProviderCalendarEvent {
        provider_event_id: id,
        title: raw
            .get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        // Graph bodies are HTML; the meeting-link extractor and the UI's
        // sanitizer both treat this as untrusted rich text.
        description: raw
            .get("body")
            .and_then(|b| b.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        location: raw
            .get("location")
            .and_then(|l| l.get("displayName"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        start_time,
        end_time,
        is_all_day: raw.get("isAllDay").and_then(|v| v.as_bool()).unwrap_or(false),
        timezone: raw
            .get("originalStartTimeZone")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        organizer: raw
            .get("organizer")
            .and_then(|o| o.get("emailAddress"))
            .and_then(|e| e.get("address"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        attendees,
        structured_meeting_urls,
        status: if is_cancelled { "cancelled" } else { "confirmed" }.to_string(),
        html_link: raw.get("webLink").and_then(|v| v.as_str()).map(|s| s.to_string()),
        recurring_event_id: raw
            .get("seriesMasterId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

/// Graph start/end object: `{"dateTime": "2026-07-22T10:00:00.0000000", "timeZone": "UTC"}`.
/// The dateTime has no offset — with the `Prefer: outlook.timezone="UTC"`
/// request header it is always UTC wall time.
fn parse_graph_time(time: &serde_json::Value) -> Option<i64> {
    let date_time = time.get("dateTime").and_then(|v| v.as_str())?;
    chrono::NaiveDateTime::parse_from_str(date_time, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|naive| naive.and_utc().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_graph_calendar ───────────────────────────────────────────────

    #[test]
    fn parses_the_default_calendar_as_primary_and_owned() {
        let raw = serde_json::json!({
            "id": "AAMkAGI2primary=",
            "name": "Calendar",
            "hexColor": "",
            "color": "auto",
            "isDefaultCalendar": true,
            "canEdit": true,
        });

        let calendar = parse_graph_calendar(&raw).expect("parse");

        assert!(calendar.is_primary);
        assert_eq!(calendar.access_role, "owner");
        assert!(calendar.selected, "Graph has no per-calendar visibility flag");
    }

    #[test]
    fn a_shared_read_only_calendar_parses_as_reader() {
        let raw = serde_json::json!({
            "id": "AAMkAGI2shared=",
            "name": "Team calendar",
            "isDefaultCalendar": false,
            "canEdit": false,
        });

        let calendar = parse_graph_calendar(&raw).expect("parse");

        assert_eq!(calendar.access_role, "reader");
        assert!(!calendar.is_primary);
        assert_eq!(calendar.name, "Team calendar");
    }

    #[test]
    fn a_calendar_carries_its_owner_address() {
        // Graph sets `owner` to the person who shared a calendar with the
        // user, and to the user for a calendar they created.
        let raw = serde_json::json!({
            "id": "AAMkAGI2shared=",
            "name": "Team calendar",
            "isDefaultCalendar": false,
            "canEdit": false,
            "owner": { "name": "A Colleague", "address": "colleague@example.com" },
        });

        let calendar = parse_graph_calendar(&raw).expect("parse");

        assert_eq!(calendar.owner_email.as_deref(), Some("colleague@example.com"));
    }

    #[test]
    fn a_calendar_without_an_owner_has_no_owner() {
        let raw = serde_json::json!({
            "id": "AAMkAGI2shared=",
            "name": "Team calendar",
            "isDefaultCalendar": false,
        });

        assert_eq!(parse_graph_calendar(&raw).expect("parse").owner_email, None);
    }

    #[test]
    fn a_shared_editable_calendar_parses_as_writer() {
        let raw = serde_json::json!({
            "id": "AAMkAGI2writable=",
            "name": "Project",
            "isDefaultCalendar": false,
            "canEdit": true,
        });

        assert_eq!(parse_graph_calendar(&raw).expect("parse").access_role, "writer");
    }

    #[test]
    fn a_custom_hex_colour_is_carried_through() {
        let raw = serde_json::json!({ "id": "AAMkAGI2=", "name": "Red one", "hexColor": "#a4373a" });

        assert_eq!(parse_graph_calendar(&raw).expect("parse").color, "#a4373a");
    }

    #[test]
    fn a_named_preset_colour_leaves_the_colour_for_the_app_to_pick() {
        // Graph's `color` presets have no documented hex; guessing one would
        // not match Outlook, so the UI assigns a palette slot instead.
        let raw = serde_json::json!({ "id": "AAMkAGI2=", "name": "Preset", "hexColor": "", "color": "lightGreen" });

        assert_eq!(parse_graph_calendar(&raw).expect("parse").color, "");
    }

    #[test]
    fn an_entry_without_an_id_is_skipped() {
        assert!(parse_graph_calendar(&serde_json::json!({ "name": "Nameless" })).is_none());
    }

    #[test]
    fn parses_teams_event_with_online_meeting() {
        let raw = serde_json::json!({
            "id": "ev-graph-1",
            "subject": "Quarterly review",
            "bodyPreview": "Join below",
            "body": {"contentType": "html", "content": "<a href=\"https://teams.microsoft.com/l/meetup-join/19%3am%40thread.v2/0\">Join</a>"},
            "location": {"displayName": "Teams meeting"},
            "start": {"dateTime": "2026-07-22T08:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-07-22T09:00:00.0000000", "timeZone": "UTC"},
            "isAllDay": false,
            "isCancelled": false,
            "originalStartTimeZone": "Romance Standard Time",
            "organizer": {"emailAddress": {"address": "organizer@example.com", "name": "Organizer"}},
            "attendees": [
                {"emailAddress": {"address": "a@example.com"}, "type": "required", "status": {"response": "accepted"}},
                {"emailAddress": {"address": "b@example.com"}, "type": "optional", "status": {"response": "tentativelyAccepted"}},
                {"emailAddress": {"address": "c@example.com"}, "type": "required", "status": {"response": "declined"}},
                {"emailAddress": {"address": "d@example.com"}, "type": "required"}
            ],
            "onlineMeeting": {"joinUrl": "https://teams.microsoft.com/l/meetup-join/19%3am%40thread.v2/0"},
            "webLink": "https://outlook.office365.com/calendar/item/xyz"
        });

        let event = parse_graph_event(&raw).expect("parse");
        assert_eq!(event.provider_event_id, "ev-graph-1");
        assert_eq!(event.title, "Quarterly review");
        assert_eq!(event.start_time, 1784707200); // 2026-07-22T08:00:00Z
        assert_eq!(event.end_time - event.start_time, 3_600);
        assert!(!event.is_all_day);
        assert_eq!(event.timezone, "Romance Standard Time");
        assert_eq!(event.organizer, "organizer@example.com");
        let rsvps: Vec<(&str, &str)> = event
            .attendees
            .iter()
            .map(|a| (a.email.as_str(), a.response.as_str()))
            .collect();
        assert_eq!(
            rsvps,
            vec![
                ("a@example.com", "accepted"),
                ("b@example.com", "tentative"),
                ("c@example.com", "declined"),
                ("d@example.com", "needsAction"),
            ]
        );
        assert_eq!(
            event.structured_meeting_urls,
            vec!["https://teams.microsoft.com/l/meetup-join/19%3am%40thread.v2/0"]
        );
        assert_eq!(event.status, "confirmed");
        assert_eq!(
            event.html_link.as_deref(),
            Some("https://outlook.office365.com/calendar/item/xyz")
        );
        assert!(event.description.contains("meetup-join"));
    }

    #[test]
    fn legacy_online_meeting_url_is_second_priority() {
        let raw = serde_json::json!({
            "id": "ev-legacy",
            "start": {"dateTime": "2026-07-22T08:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-07-22T09:00:00.0000000", "timeZone": "UTC"},
            "onlineMeetingUrl": "https://acme.webex.com/meet/jdoe"
        });
        let event = parse_graph_event(&raw).expect("parse");
        assert_eq!(event.structured_meeting_urls, vec!["https://acme.webex.com/meet/jdoe"]);
    }

    #[test]
    fn cancelled_event_maps_to_cancelled_status() {
        let raw = serde_json::json!({
            "id": "ev-cancelled",
            "isCancelled": true,
            "start": {"dateTime": "2026-07-22T08:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-07-22T09:00:00.0000000", "timeZone": "UTC"}
        });
        let event = parse_graph_event(&raw).expect("parse");
        assert_eq!(event.status, "cancelled");
    }

    #[test]
    fn all_day_flag_carries_through() {
        let raw = serde_json::json!({
            "id": "ev-allday",
            "isAllDay": true,
            "start": {"dateTime": "2026-07-23T00:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-07-24T00:00:00.0000000", "timeZone": "UTC"}
        });
        let event = parse_graph_event(&raw).expect("parse");
        assert!(event.is_all_day);
        assert_eq!(event.start_time, 1784764800);
    }

    #[test]
    fn event_without_times_is_skipped() {
        let raw = serde_json::json!({"id": "ev-broken"});
        assert!(parse_graph_event(&raw).is_none());
    }

    // ── graph_recurrence / build_graph_create_request ──────────────────────

    const WED_0800Z: i64 = 1784707200; // 2026-07-22T08:00:00Z, a Wednesday

    fn new_event() -> crate::sync::calendar_provider::NewCalendarEvent {
        crate::sync::calendar_provider::NewCalendarEvent {
            title: "Planning".to_string(),
            description: String::new(),
            attendees: Vec::new(),
            start_time: WED_0800Z,
            end_time: WED_0800Z + 3_600,
            time_zone: "Europe/Madrid".to_string(),
            recurrence: crate::sync::calendar_provider::EventRecurrence::None,
            request_meet_link: false,
        }
    }

    #[test]
    fn recurrence_none_yields_no_pattern() {
        use crate::sync::calendar_provider::EventRecurrence as R;
        assert!(graph_recurrence(R::None, WED_0800Z).is_none());
    }

    #[test]
    fn weekly_recurrence_anchors_on_the_start_weekday() {
        use crate::sync::calendar_provider::EventRecurrence as R;
        let rec = graph_recurrence(R::Weekly, WED_0800Z).expect("pattern");
        assert_eq!(rec["pattern"]["type"], "weekly");
        assert_eq!(rec["pattern"]["daysOfWeek"][0], "wednesday");
        assert_eq!(rec["range"]["type"], "noEnd");
        assert_eq!(rec["range"]["startDate"], "2026-07-22");
    }

    #[test]
    fn weekdays_recurrence_lists_monday_through_friday() {
        use crate::sync::calendar_provider::EventRecurrence as R;
        let rec = graph_recurrence(R::Weekdays, WED_0800Z).expect("pattern");
        let days: Vec<&str> = rec["pattern"]["daysOfWeek"]
            .as_array()
            .expect("array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(days, vec!["monday", "tuesday", "wednesday", "thursday", "friday"]);
    }

    #[test]
    fn monthly_and_yearly_anchor_on_day_and_month() {
        use crate::sync::calendar_provider::EventRecurrence as R;
        let monthly = graph_recurrence(R::Monthly, WED_0800Z).expect("pattern");
        assert_eq!(monthly["pattern"]["type"], "absoluteMonthly");
        assert_eq!(monthly["pattern"]["dayOfMonth"], 22);
        let yearly = graph_recurrence(R::Yearly, WED_0800Z).expect("pattern");
        assert_eq!(yearly["pattern"]["type"], "absoluteYearly");
        assert_eq!(yearly["pattern"]["month"], 7);
    }

    #[test]
    fn parses_series_master_link_on_occurrences() {
        let raw = serde_json::json!({
            "id": "occurrence-1",
            "seriesMasterId": "master-1",
            "start": {"dateTime": "2026-07-28T05:30:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-07-28T06:30:00.0000000", "timeZone": "UTC"}
        });
        let event = parse_graph_event(&raw).expect("parse");
        assert_eq!(event.recurring_event_id.as_deref(), Some("master-1"));
    }

    // ── graph_recurrence_from_rrule ────────────────────────────────────────

    fn from_rrule(rrule: &str) -> std::result::Result<serde_json::Value, String> {
        // 2026-12-01 is a Tuesday; the series starts at 10:00.
        let first = chrono::NaiveDate::from_ymd_opt(2026, 12, 1)
            .and_then(|d| d.and_hms_opt(10, 0, 0))
            .expect("date");
        graph_recurrence_from_rrule(rrule, first, "Europe/Madrid")
    }

    #[test]
    fn weekly_byday_list_maps_to_days_of_week() {
        let rec = from_rrule("FREQ=WEEKLY;BYDAY=TU,TH").expect("rule");
        assert_eq!(rec["pattern"]["type"], "weekly");
        assert_eq!(rec["pattern"]["daysOfWeek"], serde_json::json!(["tuesday", "thursday"]));
        assert_eq!(rec["pattern"]["interval"], 1);
        assert_eq!(rec["range"]["type"], "noEnd");
        assert_eq!(rec["range"]["startDate"], "2026-12-01");
    }

    #[test]
    fn interval_and_count_map_to_numbered_range() {
        let rec = from_rrule("FREQ=DAILY;INTERVAL=2;COUNT=10").expect("rule");
        assert_eq!(rec["pattern"]["type"], "daily");
        assert_eq!(rec["pattern"]["interval"], 2);
        assert_eq!(rec["range"]["type"], "numbered");
        assert_eq!(rec["range"]["numberOfOccurrences"], 10);
    }

    #[test]
    fn until_maps_to_end_date_in_event_zone() {
        let rec = from_rrule("FREQ=WEEKLY;UNTIL=20261231T225959Z").expect("rule");
        assert_eq!(rec["range"]["type"], "endDate");
        assert_eq!(rec["range"]["endDate"], "2026-12-31");
        assert_eq!(rec["range"]["recurrenceTimeZone"], "Europe/Madrid");
    }

    #[test]
    fn second_tuesday_monthly_is_relative_monthly() {
        let rec = from_rrule("FREQ=MONTHLY;BYDAY=2TU").expect("rule");
        assert_eq!(rec["pattern"]["type"], "relativeMonthly");
        assert_eq!(rec["pattern"]["index"], "second");
        assert_eq!(rec["pattern"]["daysOfWeek"], serde_json::json!(["tuesday"]));
    }

    #[test]
    fn last_friday_is_index_last() {
        let rec = from_rrule("FREQ=MONTHLY;BYDAY=-1FR").expect("rule");
        assert_eq!(rec["pattern"]["index"], "last");
    }

    #[test]
    fn monthday_maps_to_absolute_monthly() {
        let rec = from_rrule("FREQ=MONTHLY;BYMONTHDAY=15").expect("rule");
        assert_eq!(rec["pattern"]["type"], "absoluteMonthly");
        assert_eq!(rec["pattern"]["dayOfMonth"], 15);
    }

    #[test]
    fn yearly_with_month_and_day() {
        let rec = from_rrule("FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=9").expect("rule");
        assert_eq!(rec["pattern"]["type"], "absoluteYearly");
        assert_eq!(rec["pattern"]["month"], 3);
        assert_eq!(rec["pattern"]["dayOfMonth"], 9);
    }

    #[test]
    fn wkst_sets_first_day_of_week() {
        let rec = from_rrule("FREQ=WEEKLY;BYDAY=MO;WKST=SU").expect("rule");
        assert_eq!(rec["pattern"]["firstDayOfWeek"], "sunday");
    }

    #[test]
    fn missing_by_parts_anchor_on_first_start() {
        let weekly = from_rrule("FREQ=WEEKLY").expect("rule");
        assert_eq!(weekly["pattern"]["daysOfWeek"], serde_json::json!(["tuesday"]));
        let monthly = from_rrule("FREQ=MONTHLY").expect("rule");
        assert_eq!(monthly["pattern"]["dayOfMonth"], 1);
        let yearly = from_rrule("FREQ=YEARLY").expect("rule");
        assert_eq!(yearly["pattern"]["month"], 12);
        assert_eq!(yearly["pattern"]["dayOfMonth"], 1);
    }

    #[test]
    fn unsupported_parts_are_named() {
        for (rule, part) in [
            ("FREQ=HOURLY", "FREQ=HOURLY"),
            ("FREQ=MONTHLY;BYDAY=MO;BYSETPOS=-1", "BYSETPOS"),
            ("FREQ=MONTHLY;BYMONTHDAY=1,15", "BYMONTHDAY=1,15"),
            ("FREQ=YEARLY;BYWEEKNO=20", "BYWEEKNO"),
            ("FREQ=YEARLY;BYYEARDAY=100", "BYYEARDAY"),
            ("FREQ=MONTHLY;BYDAY=5MO", "BYDAY=5MO"),
        ] {
            assert_eq!(from_rrule(rule), Err(part.to_string()), "{rule}");
        }
    }

    #[test]
    fn parts_graph_cannot_express_exactly_are_rejected() {
        for (rule, part) in [
            ("FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR", "BYDAY=MO,TU,WE,TH,FR"),
            ("FREQ=WEEKLY;BYDAY=TU,2TH", "BYDAY=TU,2TH"),
            ("FREQ=WEEKLY;BYDAY=2TU", "BYDAY=2TU"),
            ("FREQ=MONTHLY;BYDAY=1MO,2TU", "BYDAY=1MO,2TU"),
            ("FREQ=MONTHLY;BYDAY=MO,2TU", "BYDAY=MO,2TU"),
            ("FREQ=MONTHLY;BYDAY=MO", "BYDAY=MO"),
            ("FREQ=MONTHLY;BYMONTH=3", "BYMONTH=3"),
            ("FREQ=DAILY;BYMONTH=1", "BYMONTH=1"),
            ("FREQ=DAILY;BYMONTHDAY=5", "BYMONTHDAY=5"),
            ("FREQ=WEEKLY;BYMONTHDAY=15", "BYMONTHDAY=15"),
            ("FREQ=YEARLY;BYDAY=2TU", "BYDAY=2TU"),
            ("FREQ=WEEKLY;COUNT=3;UNTIL=20261231T000000Z", "UNTIL"),
            ("FREQ=MONTHLY;BYMONTHDAY=29", "BYMONTHDAY=29"),
            ("FREQ=MONTHLY;BYMONTHDAY=31", "BYMONTHDAY=31"),
            ("FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30", "BYMONTHDAY=30"),
            ("FREQ=MONTHLY;INTERVAL=0", "INTERVAL=0"),
            ("FREQ=MONTHLY;INTERVAL=x", "INTERVAL=x"),
        ] {
            assert_eq!(from_rrule(rule), Err(part.to_string()), "{rule}");
        }
    }

    #[test]
    fn implicit_anchor_day_above_28_is_rejected() {
        let first = chrono::NaiveDate::from_ymd_opt(2026, 12, 31)
            .and_then(|d| d.and_hms_opt(10, 0, 0))
            .expect("date");
        for rule in ["FREQ=MONTHLY", "FREQ=YEARLY"] {
            assert_eq!(
                graph_recurrence_from_rrule(rule, first, "UTC"),
                Err("BYMONTHDAY=31".to_string()),
                "{rule}"
            );
        }
    }

    #[test]
    fn lowercase_keys_and_values_are_accepted() {
        let rec = from_rrule("freq=weekly;byday=tu,th;interval=2").expect("rule");
        assert_eq!(rec["pattern"]["interval"], 2);
        assert_eq!(rec["pattern"]["daysOfWeek"], serde_json::json!(["tuesday", "thursday"]));
    }

    #[test]
    fn until_as_date_value_is_the_end_date() {
        let rec = from_rrule("FREQ=WEEKLY;UNTIL=20261231").expect("rule");
        assert_eq!(rec["range"]["type"], "endDate");
        assert_eq!(rec["range"]["endDate"], "2026-12-31");
    }

    #[test]
    fn relative_yearly_with_month_maps_to_relative_yearly() {
        let rec = from_rrule("FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU").expect("rule");
        assert_eq!(rec["pattern"]["type"], "relativeYearly");
        assert_eq!(rec["pattern"]["month"], 3);
        assert_eq!(rec["pattern"]["index"], "last");
    }

    #[test]
    fn yearly_monthday_without_month_is_rejected() {
        // RFC 5545 repeats BYMONTHDAY in every month; Graph would keep one.
        assert_eq!(
            from_rrule("FREQ=YEARLY;BYMONTHDAY=15"),
            Err("BYMONTHDAY=15".to_string())
        );
        assert!(from_rrule("FREQ=YEARLY").is_ok());
    }

    #[test]
    fn multi_day_relative_patterns_are_rejected() {
        // Graph keeps only the first matching day of several.
        for (rule, part) in [
            ("FREQ=MONTHLY;BYDAY=1MO,1TU", "BYDAY=1MO,1TU"),
            ("FREQ=YEARLY;BYMONTH=3;BYDAY=-1SA,-1SU", "BYDAY=-1SA,-1SU"),
        ] {
            assert_eq!(from_rrule(rule), Err(part.to_string()), "{rule}");
        }
    }

    #[test]
    fn weekly_weeks_start_on_monday_unless_wkst_says_otherwise() {
        // RFC 5545 defaults WKST to MO; Graph defaults firstDayOfWeek to sunday.
        let rec = from_rrule("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,SU").expect("rule");
        assert_eq!(rec["pattern"]["daysOfWeek"], serde_json::json!(["monday", "sunday"]));
        assert_eq!(rec["pattern"]["firstDayOfWeek"], "monday");
        let rec = from_rrule("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,SU;WKST=SU").expect("rule");
        assert_eq!(rec["pattern"]["firstDayOfWeek"], "sunday");
    }

    // ── graph_recurrence_with_end ──────────────────────────────────────────

    #[test]
    fn truncation_preserves_pattern_and_ends_the_day_before() {
        let existing = serde_json::json!({
            "pattern": {"type": "weekly", "interval": 1, "daysOfWeek": ["tuesday"]},
            "range": {"type": "noEnd", "startDate": "2026-06-02"}
        });
        // First removed occurrence: 2026-07-28T05:30:00Z → series ends 2026-07-27.
        let rewritten = graph_recurrence_with_end(&existing, 1785216600).expect("rewrite");
        assert_eq!(rewritten["pattern"]["daysOfWeek"][0], "tuesday");
        assert_eq!(rewritten["range"]["type"], "endDate");
        assert_eq!(rewritten["range"]["startDate"], "2026-06-02");
        assert_eq!(rewritten["range"]["endDate"], "2026-07-27");
    }

    #[test]
    fn truncation_without_pattern_returns_none() {
        assert!(graph_recurrence_with_end(&serde_json::json!({}), 1785216600).is_none());
    }

    #[test]
    fn minimal_create_body_has_no_optional_fields() {
        let body = build_graph_create_request(&new_event()).expect("build");
        assert_eq!(body["subject"], "Planning");
        assert_eq!(body["start"]["timeZone"], "UTC");
        assert!(body.get("body").is_none());
        assert!(body.get("attendees").is_none());
        assert!(body.get("recurrence").is_none());
    }

    #[test]
    fn create_body_carries_description_attendees_and_recurrence() {
        use crate::sync::calendar_provider::EventRecurrence as R;
        let mut event = new_event();
        event.description = "Agenda: budget".to_string();
        event.attendees = vec!["ana@example.com".to_string()];
        event.recurrence = R::Daily;
        let body = build_graph_create_request(&event).expect("build");
        assert_eq!(body["body"]["content"], "Agenda: budget");
        assert_eq!(body["body"]["contentType"], "text");
        assert_eq!(body["attendees"][0]["emailAddress"]["address"], "ana@example.com");
        assert_eq!(body["attendees"][0]["type"], "required");
        assert_eq!(body["recurrence"]["pattern"]["type"], "daily");
    }

    // ── UID lookup ─────────────────────────────────────────────────────────

    #[test]
    fn global_object_id_wraps_vcal_uid() {
        let uid = "abc123@example.com";
        let id = exchange_global_object_id(uid);
        let uid_hex: String = uid.bytes().map(|b| format!("{b:02X}")).collect();

        assert!(id.starts_with("040000008200E00074C5B7101A82E008"));
        assert_eq!(id, id.to_uppercase());
        // 16-byte class id + 4 instance + 8 creation + 8 reserved = 36 bytes.
        let size = (("vCal-Uid".len() + 4 + uid.len() + 1) as u32).to_le_bytes();
        let expected_size: String = size.iter().map(|b| format!("{b:02X}")).collect();
        assert_eq!(&id[72..80], expected_size);
        assert_eq!(
            &id[32..72],
            "0".repeat(40),
            "instance, creation time and reserved are zero"
        );
        assert_eq!(&id[80..], format!("7643616C2D55696401000000{uid_hex}00"));
    }

    #[test]
    fn hex_global_object_ids_pass_through() {
        let already = "040000008200e00074c5b7101a82e008000000000000000000000000000000001234";
        assert_eq!(exchange_global_object_id(already), already.to_uppercase());
    }

    #[test]
    fn uid_filter_matches_plain_and_encoded() {
        let filter = outlook_uid_filter("abc123@example.com");
        let encoded = exchange_global_object_id("abc123@example.com");

        assert!(filter.contains("iCalUId eq 'abc123@example.com'"));
        assert!(filter.contains(&format!("iCalUId eq '{encoded}'")));
        assert!(!filter.contains("singleValueExtendedProperties"));
    }

    #[test]
    fn uid_filter_doubles_single_quotes() {
        let filter = outlook_uid_filter("o'brien@example.com");
        assert!(filter.contains("iCalUId eq 'o''brien@example.com'"));
    }

    #[test]
    fn imported_uid_filter_targets_the_emailops_property() {
        assert_eq!(
            outlook_imported_uid_filter("abc123@example.com"),
            format!(
                "singleValueExtendedProperties/Any(ep: ep/id eq '{EMAILOPS_UID_PROPERTY}' and ep/value eq 'abc123@example.com')"
            )
        );
        assert!(outlook_imported_uid_filter("o'brien@example.com").contains("ep/value eq 'o''brien@example.com'"));
    }

    #[tokio::test]
    async fn rsvp_finds_an_invite_organized_outside_exchange() {
        use crate::sync::calendar_provider::{CalendarProvider, RsvpResponse};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, Request, ResponseTemplate};

        let encoded = exchange_global_object_id("abc123@example.com");
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/me/events"))
            .and(move |req: &Request| req.url.query().unwrap_or("").contains(encoded.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"value":[{"id":"evt1"}]}"#, "application/json"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"value":[]}"#, "application/json"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/me/events/evt1/accept"))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;

        let client = OutlookCalendarClient::new("tok".into(), None, None).with_base_url(server.uri());
        client
            .rsvp_by_ical_uid("abc123@example.com", RsvpResponse::Accepted, "me@example.com")
            .await
            .expect("rsvp");
    }

    // ── ICS import ─────────────────────────────────────────────────────────

    fn import_timed(uid: &str) -> ImportableEvent {
        use crate::services::calendar::import::EventTime;
        let day = |d, h| {
            chrono::NaiveDate::from_ymd_opt(2026, 12, d)
                .unwrap()
                .and_hms_opt(h, 0, 0)
                .unwrap()
        };
        ImportableEvent {
            uid: uid.into(),
            summary: "Planning sync".into(),
            description: "Agenda".into(),
            location: "Room 4".into(),
            start: EventTime::Timed {
                local: day(1, 10),
                tz: "Europe/Madrid".into(),
            },
            end: EventTime::Timed {
                local: day(1, 11),
                tz: "Europe/Madrid".into(),
            },
            rrule: None,
            exdates: vec![],
            rdates: vec![],
            recurrence_id: None,
            overrides: vec![],
            organizer: Some("boss@example.com".into()),
            attendees: vec![],
            sequence: 0,
            fallback_tz: "Europe/Madrid".into(),
        }
    }

    fn weekly_with_exdate() -> ImportableEvent {
        use crate::services::calendar::import::EventTime;
        let mut event = import_timed("series@example.com");
        event.rrule = Some("FREQ=WEEKLY;COUNT=4".into());
        event.exdates = vec![EventTime::Timed {
            local: chrono::NaiveDate::from_ymd_opt(2026, 12, 8)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap(),
            tz: "Europe/Madrid".into(),
        }];
        event
    }

    #[test]
    fn graph_import_body_has_no_attendees() {
        let mut event = import_timed("a@example.com");
        event.attendees = (1..=3)
            .map(|n| crate::models::CalendarAttendee {
                email: format!("p{n}@example.com"),
                response: "needsAction".into(),
            })
            .collect();
        let body = build_graph_import_request(&event).expect("body");
        assert!(body.get("attendees").is_none());
    }

    #[test]
    fn graph_import_body_uses_the_windows_zone_of_the_event() {
        let mut event = import_timed("a@example.com");
        event.rrule = Some("FREQ=WEEKLY;COUNT=3".into());
        let body = build_graph_import_request(&event).expect("body");
        assert_eq!(
            body["start"],
            serde_json::json!({"dateTime": "2026-12-01T10:00:00", "timeZone": "Romance Standard Time"})
        );
        assert_eq!(
            body["end"],
            serde_json::json!({"dateTime": "2026-12-01T11:00:00", "timeZone": "Romance Standard Time"})
        );
        assert_eq!(body["isAllDay"], false);
        assert_eq!(body["subject"], "Planning sync");
        assert_eq!(body["location"]["displayName"], "Room 4");
        assert_eq!(body["body"]["contentType"], "text");
        assert_eq!(body["body"]["content"], "Agenda");
        assert_eq!(
            body["singleValueExtendedProperties"][0],
            serde_json::json!({"id": EMAILOPS_UID_PROPERTY, "value": "a@example.com"})
        );
        let mut expected = graph_recurrence_from_rrule(
            "FREQ=WEEKLY;COUNT=3",
            chrono::NaiveDate::from_ymd_opt(2026, 12, 1)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap(),
            "Europe/Madrid",
        )
        .expect("recurrence");
        assert_eq!(expected["pattern"]["firstDayOfWeek"], "monday");
        expected["range"]["recurrenceTimeZone"] = serde_json::json!("Romance Standard Time");
        assert_eq!(body["recurrence"], expected);
    }

    #[test]
    fn graph_end_date_drops_a_last_day_whose_occurrence_is_after_until() {
        // A daily series at 10:00 in Madrid; Graph's endDate is inclusive.
        for (until, end_date) in [
            ("20261231T000000Z", "2026-12-30"), // 01:00 in Madrid, before 10:00
            ("20261231T090000Z", "2026-12-31"), // 10:00 in Madrid: the last occurrence
            ("20261231T083000", "2026-12-30"),  // floating: already local
            ("20261231", "2026-12-31"),         // a date: the last day
        ] {
            let mut event = import_timed("a@example.com");
            event.rrule = Some(format!("FREQ=DAILY;UNTIL={until}"));
            let body = build_graph_import_request(&event).expect("body");
            assert_eq!(body["recurrence"]["range"]["endDate"], end_date, "UNTIL={until}");
        }
    }

    #[test]
    fn graph_import_utc_event_keeps_utc_and_empty_fields_are_omitted() {
        use crate::services::calendar::import::EventTime;
        let mut event = import_timed("a@example.com");
        event.description.clear();
        event.location.clear();
        if let EventTime::Timed { tz, .. } = &mut event.start {
            *tz = "UTC".into();
        }
        let body = build_graph_import_request(&event).expect("body");
        assert_eq!(body["start"]["timeZone"], "UTC");
        assert!(body.get("body").is_none());
        assert!(body.get("location").is_none());
    }

    #[test]
    fn graph_all_day_import() {
        use crate::services::calendar::import::EventTime;
        let mut event = import_timed("a@example.com");
        event.start = EventTime::AllDay(chrono::NaiveDate::from_ymd_opt(2026, 12, 5).unwrap());
        event.end = EventTime::AllDay(chrono::NaiveDate::from_ymd_opt(2026, 12, 6).unwrap());
        let body = build_graph_import_request(&event).expect("body");
        assert_eq!(body["isAllDay"], true);
        assert_eq!(body["start"]["dateTime"], "2026-12-05T00:00:00");
        assert_eq!(body["end"]["dateTime"], "2026-12-06T00:00:00");
    }

    #[test]
    fn graph_all_day_series_use_the_fallback_zone() {
        // The planner checks an all-day rule in `fallback_tz`; the body must agree.
        use crate::services::calendar::import::EventTime;
        let mut event = import_timed("a@example.com");
        event.start = EventTime::AllDay(chrono::NaiveDate::from_ymd_opt(2026, 12, 5).unwrap());
        event.end = EventTime::AllDay(chrono::NaiveDate::from_ymd_opt(2026, 12, 6).unwrap());
        event.rrule = Some("FREQ=DAILY;UNTIL=20261231T233000Z".into());
        let body = build_graph_import_request(&event).expect("body");
        // 23:30 UTC on 31/12 is 00:30 on 01/01 in Madrid.
        assert_eq!(body["recurrence"]["range"]["endDate"], "2027-01-01");
        assert_eq!(
            body["recurrence"]["range"]["recurrenceTimeZone"],
            "Romance Standard Time"
        );
    }

    #[test]
    fn graph_import_rejects_a_zone_without_a_windows_name() {
        use crate::services::calendar::import::EventTime;
        let mut event = import_timed("a@example.com");
        if let EventTime::Timed { tz, .. } = &mut event.start {
            *tz = "Nowhere/Land".into();
        }
        let error = build_graph_import_request(&event).expect_err("must fail");
        assert!(matches!(error, AppError::InvalidInput(ref m) if m.contains("no Windows time zone for Nowhere/Land")));
    }

    #[test]
    fn graph_import_rejects_rdates_instead_of_dropping_them() {
        let mut event = import_timed("a@example.com");
        event.rdates = vec!["RDATE:20261210T100000Z".into()];
        assert!(build_graph_import_request(&event).is_err());
    }

    fn instances_page(id: &str, original_start: &str) -> serde_json::Value {
        serde_json::json!({"value": [
            {"id": "other", "originalStart": "2026-12-01T09:00:00Z"},
            {"id": id, "originalStart": original_start}
        ]})
    }

    #[tokio::test]
    async fn outlook_import_deletes_exdate_instances() {
        use crate::sync::calendar_provider::CalendarProvider;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/events"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": "series-1"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/me/events/series-1/instances"))
            .and(query_param("startDateTime", "2026-12-07T09:00:00Z"))
            .and(query_param("endDateTime", "2026-12-09T09:00:00Z"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(instances_page("inst-2", "2026-12-08T09:00:00.0000000Z")),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/me/events/inst-2"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let client = OutlookCalendarClient::new("tok".into(), None, None).with_base_url(server.uri());
        let id = client.import_event(&weekly_with_exdate()).await.expect("import");
        assert_eq!(id, "series-1");
    }

    #[tokio::test]
    async fn outlook_import_patches_overrides() {
        use crate::services::calendar::import::EventTime;
        use crate::sync::calendar_provider::CalendarProvider;
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let at = |d, h| {
            chrono::NaiveDate::from_ymd_opt(2026, 12, d)
                .unwrap()
                .and_hms_opt(h, 0, 0)
                .unwrap()
        };
        let mut event = import_timed("series@example.com");
        event.rrule = Some("FREQ=WEEKLY;COUNT=4".into());
        let mut over = import_timed("series@example.com");
        over.summary = "Moved sync".into();
        over.recurrence_id = Some(EventTime::Timed {
            local: at(8, 10),
            tz: "Europe/Madrid".into(),
        });
        over.start = EventTime::Timed {
            local: at(8, 14),
            tz: "Europe/Madrid".into(),
        };
        over.end = EventTime::Timed {
            local: at(8, 15),
            tz: "Europe/Madrid".into(),
        };
        over.attendees = vec![crate::models::CalendarAttendee {
            email: "p@example.com".into(),
            response: "needsAction".into(),
        }];
        event.overrides = vec![over];

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/events"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": "series-1"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/me/events/series-1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(instances_page("inst-2", "2026-12-08T09:00:00Z")))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/me/events/inst-2"))
            .and(body_partial_json(serde_json::json!({
                "subject": "Moved sync",
                "start": {"dateTime": "2026-12-08T14:00:00", "timeZone": "Romance Standard Time"},
                "end": {"dateTime": "2026-12-08T15:00:00", "timeZone": "Romance Standard Time"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let client = OutlookCalendarClient::new("tok".into(), None, None).with_base_url(server.uri());
        client.import_event(&event).await.expect("import");
        let requests = server.received_requests().await.expect("requests");
        let patch = requests.iter().find(|r| r.method.as_str() == "PATCH").expect("patch");
        let body: serde_json::Value = serde_json::from_slice(&patch.body).expect("json");
        assert!(body.get("attendees").is_none());
    }

    #[tokio::test]
    async fn outlook_import_rolls_back_on_partial_failure() {
        use crate::sync::calendar_provider::CalendarProvider;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/events"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": "series-1"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/me/events/series-1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(instances_page("inst-2", "2026-12-08T09:00:00Z")))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/me/events/inst-2"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/me/events/series-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let client = OutlookCalendarClient::new("tok".into(), None, None).with_base_url(server.uri());
        assert!(client.import_event(&weekly_with_exdate()).await.is_err());
    }

    #[tokio::test]
    async fn outlook_import_fails_and_rolls_back_when_no_instance_matches() {
        use crate::sync::calendar_provider::CalendarProvider;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/events"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": "series-1"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/me/events/series-1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"value": []})))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/me/events/series-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let client = OutlookCalendarClient::new("tok".into(), None, None).with_base_url(server.uri());
        assert!(client.import_event(&weekly_with_exdate()).await.is_err());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the guard serializes the global logger across the await
    async fn a_failed_outlook_rollback_is_logged_without_event_content() {
        use crate::sync::calendar_provider::CalendarProvider;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let _guard = crate::services::events::seam_test_lock();
        let logger = crate::services::logger::install_for_testing();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/events"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": "series-1"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/me/events/series-1/instances"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"value": []})))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/me/events/series-1"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Planning sync leaked body"))
            .mount(&server)
            .await;

        let client = OutlookCalendarClient::new("tok".into(), None, None).with_base_url(server.uri());
        assert!(client.import_event(&weekly_with_exdate()).await.is_err());
        let entries = logger.events();
        let errors: Vec<_> = entries
            .iter()
            .filter(|e| {
                e.level == "error" && e.source == "sync" && e.message.contains("Outlook Calendar import rollback")
            })
            .collect();
        // Other tests log to the global logger too, so look only at ours.
        assert_eq!(errors.len(), 1, "got: {entries:?}");
        assert!(!errors[0].message.contains("Planning"), "no event or body content");
        crate::services::logger::install(std::sync::Arc::new(crate::services::logger::NoopLogger));
    }

    #[tokio::test]
    async fn outlook_find_checks_our_copies_first_then_the_uid() {
        use crate::sync::calendar_provider::CalendarProvider;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/me/events"))
            .and(query_param("$filter", outlook_imported_uid_filter("u@example.com")))
            .and(query_param("$select", "id"))
            .and(query_param("$top", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"value": [{"id": "ours"}]})))
            .mount(&server)
            .await;
        let client = OutlookCalendarClient::new("tok".into(), None, None).with_base_url(server.uri());
        let found = client.find_event_by_uid("u@example.com").await.expect("find");
        assert_eq!(
            found,
            Some(FoundEvent {
                provider_event_id: "ours".into(),
                imported_by_emailops: true
            })
        );
    }

    #[tokio::test]
    async fn outlook_find_falls_back_to_the_ical_uid_and_then_none() {
        use crate::sync::calendar_provider::CalendarProvider;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/me/events"))
            .and(query_param("$filter", outlook_uid_filter("u@example.com")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"value": [{"id": "invite"}]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/me/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"value": []})))
            .mount(&server)
            .await;
        let client = OutlookCalendarClient::new("tok".into(), None, None).with_base_url(server.uri());
        let found = client.find_event_by_uid("u@example.com").await.expect("find");
        assert_eq!(
            found,
            Some(FoundEvent {
                provider_event_id: "invite".into(),
                imported_by_emailops: false
            })
        );
        assert_eq!(client.find_event_by_uid("none@example.com").await.expect("find"), None);
    }

    #[tokio::test]
    async fn outlook_respond_acts_on_the_given_event() {
        use crate::sync::calendar_provider::{CalendarProvider, RsvpResponse};
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/me/events/evt1/tentativelyAccept"))
            .and(body_json(serde_json::json!({"sendResponse": true})))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;
        let client = OutlookCalendarClient::new("tok".into(), None, None).with_base_url(server.uri());
        client
            .respond_to_event("primary", "evt1", RsvpResponse::Tentative, "me@example.com")
            .await
            .expect("respond");
    }
}
