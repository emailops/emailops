//! Microsoft Graph calendar client. Fetches expanded event instances via
//! `/me/calendarView` (recurrences pre-expanded by Graph) over a time window.
//! Requests `Prefer: outlook.timezone="UTC"` so every dateTime arrives in UTC.
//! Parsing is a pure function ([`parse_graph_event`]) for HTTP-free tests.

use async_trait::async_trait;
use reqwest::{Client, Response, StatusCode};
use std::time::Duration;
use tokio::time::sleep;

use crate::models::error::{AppError, Result};
use crate::sync::calendar_provider::{CalendarProvider, ProviderCalendarEvent};
use crate::sync::http_retry::{classify_attempt, Attempt, RetryDecision};

const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0";
const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 1_000;

/// Fields fetched for every event — enough to build a `ProviderCalendarEvent`
/// without a second round-trip.
const EVENT_SELECT_FIELDS: &str = "id,subject,body,bodyPreview,location,start,end,isAllDay,isCancelled,\
    organizer,attendees,onlineMeeting,onlineMeetingUrl,webLink,originalStartTimeZone";

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
            client: Client::new(),
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
        let new_tokens = crate::sync::oauth::refresh_oauth_token(&config, refresh_token).await?;
        crate::services::accounts::store_tokens(account_id, &new_tokens)?;
        *self
            .access_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = new_tokens.access_token;
        Ok(())
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

    async fn send_post_json_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
        operation: &str,
    ) -> Result<Response> {
        self.send_request_with_retry(operation, |client, token| {
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
                Err(_) => Attempt::TransportError,
            };
            match &response {
                Ok(resp) => last_cause = format!("HTTP {}", resp.status()),
                Err(e) => last_cause = e.to_string(),
            }

            match classify_attempt(outcome, attempt, MAX_RETRIES, refreshed) {
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
    async fn list_events(&self, window_start: i64, window_end: i64) -> Result<Vec<ProviderCalendarEvent>> {
        let start = epoch_to_rfc3339(window_start)?;
        let end = epoch_to_rfc3339(window_end)?;
        let mut events = Vec::new();
        let mut url = format!(
            "{}/me/calendarView?startDateTime={}&endDateTime={}&$top=100&$select={}",
            self.base_url,
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
        let response = self.send_post_json_with_retry(&url, &body, "create event").await?;
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

    async fn delete_event(&self, provider_event_id: &str, notify: bool, message: &str) -> Result<()> {
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
            self.send_post_json_with_retry(&url, &body, "cancel event").await?
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

    async fn truncate_recurring_event(&self, master_id: &str, first_removed_start: i64, _notify: bool) -> Result<()> {
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
        // OData string literals escape single quotes by doubling them.
        let escaped_uid = ical_uid.replace('\'', "''");
        let lookup_url = format!(
            "{}/me/events?$filter=iCalUId%20eq%20'{}'&$select=id&$top=1",
            self.base_url,
            urlencoding::encode(&escaped_uid)
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
        let acted = self.send_post_json_with_retry(&action_url, &body, "rsvp").await?;
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
}
