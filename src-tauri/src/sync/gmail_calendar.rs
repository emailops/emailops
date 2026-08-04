//! Google Calendar v3 client for the calendar sync. Fetches expanded event
//! instances (`singleEvents=true`) over a time window from the account's
//! primary calendar. Parsing is a pure function ([`parse_google_event`]) so it
//! is unit-testable without HTTP.

use async_trait::async_trait;
use reqwest::{Client, Response, StatusCode};
use std::time::Duration;
use tokio::time::sleep;

use crate::models::error::{AppError, Result};
use crate::sync::calendar_provider::{CalendarProvider, ProviderCalendar, ProviderCalendarEvent};
use crate::sync::http_retry::{classify_attempt, Attempt, RetryDecision};

const CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3";
const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 1_000;

pub struct GoogleCalendarClient {
    client: Client,
    access_token: std::sync::Mutex<String>,
    refresh_token: Option<String>,
    account_id: Option<String>,
    base_url: String,
}

impl GoogleCalendarClient {
    pub fn new(access_token: String, refresh_token: Option<String>, account_id: Option<String>) -> Self {
        Self {
            client: Client::new(),
            access_token: std::sync::Mutex::new(access_token),
            refresh_token,
            account_id,
            base_url: CALENDAR_API_BASE.to_string(),
        }
    }

    /// Test-only base URL override (wiremock).
    #[allow(dead_code)]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Refresh the access token in place (mirrors `GmailClient::refresh_access_token`).
    async fn refresh_access_token(&self) -> Result<()> {
        let Some(refresh_token) = &self.refresh_token else {
            return Err(AppError::AuthError(
                "Google Calendar session expired and no refresh token is stored. Please re-authenticate.".to_string(),
            ));
        };
        let Some(account_id) = &self.account_id else {
            return Err(AppError::AuthError(
                "Google Calendar token refresh failed: account ID unknown.".to_string(),
            ));
        };
        let config = crate::sync::oauth::OAuthConfig::for_provider("gmail");
        let new_tokens = crate::sync::oauth::refresh_oauth_token(&config, refresh_token).await?;
        crate::services::accounts::store_tokens(account_id, &new_tokens)?;
        *self
            .access_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = new_tokens.access_token;
        Ok(())
    }

    async fn send_get_with_retry(&self, url: &str, operation: &str) -> Result<Response> {
        self.send_request_with_retry(operation, |client, token| client.get(url).bearer_auth(token))
            .await
    }

    async fn send_post_json_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
        operation: &str,
    ) -> Result<Response> {
        self.send_request_with_retry(operation, |client, token| {
            client.post(url).bearer_auth(token).json(body)
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
                        .map_err(|e| AppError::SyncError(format!("Google Calendar {operation} failed: {e}")))
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
            "Google Calendar {operation} failed after {} attempts: {last_cause}",
            MAX_RETRIES + 1
        )))
    }
}

#[async_trait]
impl CalendarProvider for GoogleCalendarClient {
    async fn list_calendars(&self) -> Result<Vec<ProviderCalendar>> {
        let mut calendars = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = format!("{}/users/me/calendarList?maxResults=250", self.base_url);
            if let Some(token) = &page_token {
                url.push_str(&format!("&pageToken={}", urlencoding::encode(token)));
            }
            let response = self.send_get_with_retry(&url, "list calendars").await?;
            if !response.status().is_success() {
                let status = response.status().as_u16();
                let error_text = response.text().await.unwrap_or_default();
                return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                    "Google",
                    status,
                    &error_text,
                    self.account_id.as_deref(),
                ));
            }
            let page: serde_json::Value = response.json().await?;
            if let Some(items) = page.get("items").and_then(|v| v.as_array()) {
                calendars.extend(items.iter().filter_map(parse_google_calendar));
            }
            page_token = page
                .get("nextPageToken")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if page_token.is_none() {
                break;
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
        let time_min = epoch_to_rfc3339(window_start)?;
        let time_max = epoch_to_rfc3339(window_end)?;
        let mut events = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = format!(
                "{}/calendars/{}/events?singleEvents=true&maxResults=250&timeMin={}&timeMax={}",
                self.base_url,
                urlencoding::encode(calendar_id),
                urlencoding::encode(&time_min),
                urlencoding::encode(&time_max),
            );
            if let Some(token) = &page_token {
                url.push_str(&format!("&pageToken={}", urlencoding::encode(token)));
            }
            let response = self.send_get_with_retry(&url, "list events").await?;
            if !response.status().is_success() {
                let status = response.status().as_u16();
                let error_text = response.text().await.unwrap_or_default();
                return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                    "Google",
                    status,
                    &error_text,
                    self.account_id.as_deref(),
                ));
            }
            let page: serde_json::Value = response.json().await?;
            if let Some(items) = page.get("items").and_then(|v| v.as_array()) {
                events.extend(items.iter().filter_map(parse_google_event));
            }
            page_token = page
                .get("nextPageToken")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if page_token.is_none() {
                break;
            }
        }
        Ok(events)
    }

    async fn create_event(
        &self,
        event: &crate::sync::calendar_provider::NewCalendarEvent,
    ) -> Result<ProviderCalendarEvent> {
        let (query, body) = build_google_create_request(event)?;
        let url = format!("{}/calendars/primary/events{}", self.base_url, query);
        let response = self.send_post_json_with_retry(&url, &body, "create event").await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                "Google",
                status,
                &error_text,
                self.account_id.as_deref(),
            ));
        }
        let raw: serde_json::Value = response.json().await?;
        parse_google_event(&raw)
            .ok_or_else(|| AppError::SyncError("Google Calendar returned an unparseable created event".to_string()))
    }

    async fn delete_event(
        &self,
        calendar_id: &str,
        provider_event_id: &str,
        notify: bool,
        _message: &str,
    ) -> Result<()> {
        // Google's API sends its standard cancellation email; a custom message
        // is not supported (`_message` intentionally unused).
        let url = format!(
            "{}/calendars/{}/events/{}?sendUpdates={}",
            self.base_url,
            urlencoding::encode(calendar_id),
            urlencoding::encode(provider_event_id),
            if notify { "all" } else { "none" },
        );
        let response = self
            .send_request_with_retry("delete event", |client, token| client.delete(&url).bearer_auth(token))
            .await?;
        let status = response.status();
        // 404/410: already gone upstream — the local mirror should still drop it.
        if status.is_success() || status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
            return Ok(());
        }
        let error_text = response.text().await.unwrap_or_default();
        Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
            "Google",
            status.as_u16(),
            &error_text,
            self.account_id.as_deref(),
        ))
    }

    async fn truncate_recurring_event(
        &self,
        calendar_id: &str,
        master_id: &str,
        first_removed_start: i64,
        notify: bool,
    ) -> Result<()> {
        // Fetch the master's recurrence, rewrite the RRULEs with an UNTIL just
        // before the first removed occurrence, and PATCH it back.
        let master_url = format!(
            "{}/calendars/{}/events/{}",
            self.base_url,
            urlencoding::encode(calendar_id),
            urlencoding::encode(master_id)
        );
        let response = self.send_get_with_retry(&master_url, "get recurring master").await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                "Google",
                status,
                &error_text,
                self.account_id.as_deref(),
            ));
        }
        let master: serde_json::Value = response.json().await?;
        let recurrence: Vec<String> = master
            .get("recurrence")
            .and_then(|v| v.as_array())
            .map(|list| list.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        if recurrence.is_empty() {
            return Err(AppError::SyncError(
                "event series has no recurrence rule to truncate".to_string(),
            ));
        }
        let body = serde_json::json!({ "recurrence": rrule_with_until(&recurrence, first_removed_start) });
        let patch_url = format!("{}?sendUpdates={}", master_url, if notify { "all" } else { "none" });
        let response = self
            .send_request_with_retry("truncate series", |client, token| {
                client.patch(&patch_url).bearer_auth(token).json(&body)
            })
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                "Google",
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
        self_email: &str,
    ) -> Result<()> {
        // Resolve the invite's UID to the event in this account's calendar
        // (providers auto-add invitations as needsAction events).
        let lookup_url = format!(
            "{}/calendars/primary/events?iCalUID={}&maxResults=1",
            self.base_url,
            urlencoding::encode(ical_uid)
        );
        let lookup = self.send_get_with_retry(&lookup_url, "find invite event").await?;
        if !lookup.status().is_success() {
            let status = lookup.status().as_u16();
            let error_text = lookup.text().await.unwrap_or_default();
            return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                "Google",
                status,
                &error_text,
                self.account_id.as_deref(),
            ));
        }
        let page: serde_json::Value = lookup.json().await?;
        let event = page
            .get("items")
            .and_then(|v| v.as_array())
            .and_then(|items| items.first())
            .ok_or_else(|| {
                AppError::NotFound(
                    "This invitation hasn't reached your calendar yet — try again in a moment.".to_string(),
                )
            })?;
        let event_id = event
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::SyncError("invite event has no id".to_string()))?;

        // Google's PATCH replaces the whole attendees array — send it back
        // with only our own responseStatus changed.
        let attendees = attendees_with_self_response(event.get("attendees"), self_email, response.as_google_status());
        let patch_url = format!(
            "{}/calendars/primary/events/{}?sendUpdates=all",
            self.base_url,
            urlencoding::encode(event_id)
        );
        let body = serde_json::json!({ "attendees": attendees });
        let patched = self
            .send_request_with_retry("rsvp", |client, token| {
                client.patch(&patch_url).bearer_auth(token).json(&body)
            })
            .await?;
        if !patched.status().is_success() {
            let status = patched.status().as_u16();
            let error_text = patched.text().await.unwrap_or_default();
            return Err(crate::sync::calendar_provider::classify_calendar_fetch_error(
                "Google",
                status,
                &error_text,
                self.account_id.as_deref(),
            ));
        }
        Ok(())
    }
}

/// Google `accessRole` values we store. Anything unrecognised degrades to the
/// least-privileged role rather than failing the sync (the DB column has a
/// CHECK constraint, so an unknown value would abort the whole batch).
fn normalize_access_role(raw: &str) -> String {
    match raw {
        "owner" | "writer" | "reader" | "freeBusyReader" => raw.to_string(),
        _ => "reader".to_string(),
    }
}

/// Parse one `calendarList` entry. Returns `None` for entries that carry no
/// usable id, and for calendars the user has removed (`deleted: true`).
///
/// Pure so it is unit-testable without HTTP.
fn parse_google_calendar(raw: &serde_json::Value) -> Option<ProviderCalendar> {
    let provider_calendar_id = raw.get("id").and_then(|v| v.as_str())?.to_string();
    if raw.get("deleted").and_then(|v| v.as_bool()).unwrap_or(false) {
        return None;
    }
    // `summaryOverride` is the user's own rename and is what Google's UI shows.
    let name = raw
        .get("summaryOverride")
        .and_then(|v| v.as_str())
        .or_else(|| raw.get("summary").and_then(|v| v.as_str()))
        .unwrap_or_default()
        .to_string();
    Some(ProviderCalendar {
        provider_calendar_id,
        name,
        color: raw
            .get("backgroundColor")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        is_primary: raw.get("primary").and_then(|v| v.as_bool()).unwrap_or(false),
        access_role: normalize_access_role(raw.get("accessRole").and_then(|v| v.as_str()).unwrap_or_default()),
        // Only an explicit `false` hides a calendar: a missing flag must never
        // make a calendar silently vanish from the app.
        selected: raw.get("selected").and_then(|v| v.as_bool()).unwrap_or(true),
    })
}

fn epoch_to_rfc3339(epoch_seconds: i64) -> Result<String> {
    chrono::DateTime::from_timestamp(epoch_seconds, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .ok_or_else(|| AppError::InvalidInput(format!("timestamp {epoch_seconds} out of range")))
}

/// Recurrence preset → Google RRULE. Weekly deliberately omits `BYDAY`: the
/// weekday derives from the event start in its `timeZone`, which keeps
/// occurrences on the user's local weekday across DST.
pub(crate) fn google_rrule(recurrence: crate::sync::calendar_provider::EventRecurrence) -> Option<&'static str> {
    use crate::sync::calendar_provider::EventRecurrence as R;
    match recurrence {
        R::None => None,
        R::Daily => Some("RRULE:FREQ=DAILY"),
        R::Weekly => Some("RRULE:FREQ=WEEKLY"),
        R::Weekdays => Some("RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),
        R::Monthly => Some("RRULE:FREQ=MONTHLY"),
        R::Yearly => Some("RRULE:FREQ=YEARLY"),
    }
}

/// Build the `events.insert` request: `(query-string, JSON body)`. Pure except
/// for the conference `requestId` (random by API contract), so tests assert on
/// everything else.
pub(crate) fn build_google_create_request(
    event: &crate::sync::calendar_provider::NewCalendarEvent,
) -> Result<(String, serde_json::Value)> {
    let mut query: Vec<&str> = Vec::new();
    let mut body = serde_json::json!({
        "summary": event.title,
        "start": {"dateTime": epoch_to_rfc3339(event.start_time)?},
        "end": {"dateTime": epoch_to_rfc3339(event.end_time)?},
    });
    if !event.description.is_empty() {
        body["description"] = serde_json::json!(event.description);
    }
    if !event.attendees.is_empty() {
        body["attendees"] = serde_json::json!(event
            .attendees
            .iter()
            .map(|a| serde_json::json!({"email": a}))
            .collect::<Vec<_>>());
        // Without this Google records attendees but never emails them.
        query.push("sendUpdates=all");
    }
    if let Some(rrule) = google_rrule(event.recurrence) {
        body["recurrence"] = serde_json::json!([rrule]);
        // Google requires an explicit timeZone on recurring events; it also
        // anchors occurrences to local wall time across DST.
        body["start"]["timeZone"] = serde_json::json!(event.time_zone);
        body["end"]["timeZone"] = serde_json::json!(event.time_zone);
    }
    if event.request_meet_link {
        // conferenceDataVersion=1 is required or the createRequest is
        // silently dropped and no Meet link is generated.
        query.push("conferenceDataVersion=1");
        body["conferenceData"] = serde_json::json!({
            "createRequest": {
                "requestId": uuid::Uuid::new_v4().to_string(),
                "conferenceSolutionKey": {"type": "hangoutsMeet"}
            }
        });
    }
    let query = if query.is_empty() {
        String::new()
    } else {
        format!("?{}", query.join("&"))
    };
    Ok((query, body))
}

/// Parse one Google Calendar `Event` resource into the provider-neutral shape.
/// Returns `None` for items we can't render (missing id or times) — e.g.
/// cancelled *thin* instances, which carry only `id` + `status`.
pub(crate) fn parse_google_event(raw: &serde_json::Value) -> Option<ProviderCalendarEvent> {
    let id = raw.get("id")?.as_str()?.to_string();
    let status = raw
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("confirmed")
        .to_string();
    let start = raw.get("start")?;
    let end = raw.get("end")?;
    let is_all_day = start.get("date").is_some();
    let start_time = parse_google_time(start)?;
    let end_time = parse_google_time(end)?;
    let timezone = start
        .get("timeZone")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let str_field = |key: &str| raw.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string();

    let mut structured_meeting_urls = Vec::new();
    if let Some(entry_points) = raw
        .get("conferenceData")
        .and_then(|c| c.get("entryPoints"))
        .and_then(|v| v.as_array())
    {
        // Video entry points first — a conference also lists phone/SIP entries.
        for ep in entry_points {
            let kind = ep.get("entryPointType").and_then(|v| v.as_str()).unwrap_or_default();
            if kind == "video" {
                if let Some(uri) = ep.get("uri").and_then(|v| v.as_str()) {
                    structured_meeting_urls.push(uri.to_string());
                }
            }
        }
    }
    if let Some(hangout) = raw.get("hangoutLink").and_then(|v| v.as_str()) {
        structured_meeting_urls.push(hangout.to_string());
    }

    let attendees = raw
        .get("attendees")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    let email = a.get("email").and_then(|v| v.as_str())?.to_string();
                    // Google's responseStatus values match our normalized set;
                    // the organizer carries a separate boolean flag.
                    let response = if a.get("organizer").and_then(|v| v.as_bool()).unwrap_or(false) {
                        "organizer".to_string()
                    } else {
                        match a.get("responseStatus").and_then(|v| v.as_str()) {
                            Some(s @ ("accepted" | "declined" | "tentative")) => s.to_string(),
                            _ => "needsAction".to_string(),
                        }
                    };
                    Some(crate::models::CalendarAttendee { email, response })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(ProviderCalendarEvent {
        provider_event_id: id,
        title: str_field("summary"),
        description: str_field("description"),
        location: str_field("location"),
        start_time,
        end_time,
        is_all_day,
        timezone,
        organizer: raw
            .get("organizer")
            .and_then(|o| o.get("email"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        attendees,
        structured_meeting_urls,
        status,
        html_link: raw.get("htmlLink").and_then(|v| v.as_str()).map(|s| s.to_string()),
        recurring_event_id: raw
            .get("recurringEventId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

/// Return the event's attendees array with `self_email`'s `responseStatus`
/// set to `status` (appending an entry when the user isn't listed yet —
/// forwarded invites). Pure for tests; email match is case-insensitive.
pub(crate) fn attendees_with_self_response(
    attendees: Option<&serde_json::Value>,
    self_email: &str,
    status: &str,
) -> serde_json::Value {
    let mut list: Vec<serde_json::Value> = attendees
        .and_then(|v| v.as_array())
        .map(|a| a.to_vec())
        .unwrap_or_default();
    let mut found = false;
    for attendee in &mut list {
        let matches = attendee
            .get("email")
            .and_then(|v| v.as_str())
            .is_some_and(|email| email.eq_ignore_ascii_case(self_email));
        if matches {
            attendee["responseStatus"] = serde_json::json!(status);
            found = true;
        }
    }
    if !found {
        list.push(serde_json::json!({"email": self_email, "responseStatus": status}));
    }
    serde_json::Value::Array(list)
}

/// Rewrite a Google `recurrence` array so the series ends strictly before
/// `first_removed_start` (epoch seconds): every RRULE line loses any COUNT and
/// gains `UNTIL=<first_removed_start - 1s>` in the required UTC basic format.
/// Non-RRULE lines (EXDATE/RDATE) pass through untouched.
pub(crate) fn rrule_with_until(recurrence: &[String], first_removed_start: i64) -> Vec<String> {
    let until = chrono::DateTime::from_timestamp(first_removed_start.saturating_sub(1), 0)
        .map(|dt| dt.format("%Y%m%dT%H%M%SZ").to_string())
        .unwrap_or_default();
    recurrence
        .iter()
        .map(|line| {
            let Some(rule) = line.strip_prefix("RRULE:") else {
                return line.clone();
            };
            let kept: Vec<&str> = rule
                .split(';')
                .filter(|part| !part.starts_with("UNTIL=") && !part.starts_with("COUNT="))
                .collect();
            format!("RRULE:{};UNTIL={}", kept.join(";"), until)
        })
        .collect()
}

/// Google start/end object: `{"dateTime": RFC3339}` for timed events,
/// `{"date": "YYYY-MM-DD"}` for all-day (parsed as UTC midnight).
fn parse_google_time(time: &serde_json::Value) -> Option<i64> {
    if let Some(date_time) = time.get("dateTime").and_then(|v| v.as_str()) {
        return chrono::DateTime::parse_from_rfc3339(date_time)
            .ok()
            .map(|dt| dt.timestamp());
    }
    let date = time.get("date").and_then(|v| v.as_str())?;
    let naive = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some(naive.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_google_calendar ──────────────────────────────────────────────

    #[test]
    fn parses_a_shared_calendar_with_its_colour_and_access_role() {
        let raw = serde_json::json!({
            "id": "team123@group.calendar.google.com",
            "summary": "Team calendar",
            "backgroundColor": "#33b679",
            "foregroundColor": "#000000",
            "colorId": "8",
            "selected": true,
            "accessRole": "reader",
        });

        let calendar = parse_google_calendar(&raw).expect("parse");

        assert_eq!(calendar.provider_calendar_id, "team123@group.calendar.google.com");
        assert_eq!(calendar.name, "Team calendar");
        assert_eq!(calendar.color, "#33b679");
        assert_eq!(calendar.access_role, "reader");
        assert!(!calendar.is_primary);
        assert!(calendar.selected);
    }

    #[test]
    fn primary_calendar_is_flagged() {
        let raw = serde_json::json!({
            "id": "someone@example.com",
            "summary": "someone@example.com",
            "primary": true,
            "accessRole": "owner",
        });

        let calendar = parse_google_calendar(&raw).expect("parse");

        assert!(calendar.is_primary);
        assert_eq!(calendar.access_role, "owner");
    }

    #[test]
    fn a_renamed_calendar_uses_the_users_own_name() {
        // summaryOverride is what Google's own UI shows for a calendar the
        // user renamed locally.
        let raw = serde_json::json!({
            "id": "shared@group.calendar.google.com",
            "summary": "Original owner's name",
            "summaryOverride": "What I call it",
            "accessRole": "reader",
        });

        assert_eq!(parse_google_calendar(&raw).expect("parse").name, "What I call it");
    }

    #[test]
    fn deleted_calendars_are_skipped() {
        let raw = serde_json::json!({
            "id": "gone@group.calendar.google.com",
            "summary": "Removed",
            "deleted": true,
            "accessRole": "reader",
        });

        assert!(parse_google_calendar(&raw).is_none());
    }

    #[test]
    fn a_missing_selected_flag_keeps_the_calendar_visible() {
        // Only an explicit false hides a calendar — a calendar must never
        // vanish from the app just because the field was omitted.
        let raw = serde_json::json!({
            "id": "cal@group.calendar.google.com",
            "summary": "No selected field",
            "accessRole": "owner",
        });

        assert!(parse_google_calendar(&raw).expect("parse").selected);
    }

    #[test]
    fn a_calendar_hidden_in_google_starts_hidden() {
        let raw = serde_json::json!({
            "id": "holidays@group.v.calendar.google.com",
            "summary": "Holidays",
            "selected": false,
            "accessRole": "reader",
        });

        assert!(!parse_google_calendar(&raw).expect("parse").selected);
    }

    #[test]
    fn an_unknown_access_role_degrades_to_reader() {
        // The DB column has a CHECK constraint; an unrecognised role must not
        // abort the whole calendar batch.
        let raw = serde_json::json!({
            "id": "cal@group.calendar.google.com",
            "summary": "Odd",
            "accessRole": "somethingNew",
        });

        assert_eq!(parse_google_calendar(&raw).expect("parse").access_role, "reader");
    }

    #[test]
    fn a_calendar_without_a_colour_parses_with_an_empty_colour() {
        let raw = serde_json::json!({ "id": "cal@group.calendar.google.com", "summary": "Plain" });

        assert_eq!(parse_google_calendar(&raw).expect("parse").color, "");
    }

    #[test]
    fn an_entry_without_an_id_is_skipped() {
        assert!(parse_google_calendar(&serde_json::json!({ "summary": "Nameless" })).is_none());
    }

    #[test]
    fn parses_timed_event_with_meet_conference() {
        let raw = serde_json::json!({
            "id": "ev-google-1",
            "status": "confirmed",
            "summary": "Weekly sync",
            "description": "Agenda in the doc",
            "location": "HQ / video",
            "start": {"dateTime": "2026-07-22T10:00:00+02:00", "timeZone": "Europe/Madrid"},
            "end": {"dateTime": "2026-07-22T10:30:00+02:00", "timeZone": "Europe/Madrid"},
            "organizer": {"email": "organizer@example.com"},
            "attendees": [
                {"email": "a@example.com", "responseStatus": "accepted"},
                {"email": "b@example.com", "responseStatus": "needsAction"}
            ],
            "conferenceData": {"entryPoints": [
                {"entryPointType": "video", "uri": "https://meet.google.com/abc-defg-hij"},
                {"entryPointType": "phone", "uri": "tel:+34-000-000-000"}
            ]},
            "hangoutLink": "https://meet.google.com/abc-defg-hij",
            "htmlLink": "https://www.google.com/calendar/event?eid=xyz"
        });

        let event = parse_google_event(&raw).expect("parse");
        assert_eq!(event.provider_event_id, "ev-google-1");
        assert_eq!(event.title, "Weekly sync");
        // 10:00+02:00 == 08:00Z
        assert_eq!(event.start_time, 1784707200);
        assert_eq!(event.end_time - event.start_time, 1800);
        assert!(!event.is_all_day);
        assert_eq!(event.timezone, "Europe/Madrid");
        assert_eq!(event.organizer, "organizer@example.com");
        let emails: Vec<&str> = event.attendees.iter().map(|a| a.email.as_str()).collect();
        assert_eq!(emails, vec!["a@example.com", "b@example.com"]);
        assert_eq!(event.attendees[0].response, "accepted");
        assert_eq!(event.attendees[1].response, "needsAction");
        // Video entry point first, hangoutLink second; phone entry excluded.
        assert_eq!(
            event.structured_meeting_urls,
            vec![
                "https://meet.google.com/abc-defg-hij",
                "https://meet.google.com/abc-defg-hij"
            ]
        );
        assert_eq!(event.status, "confirmed");
        assert_eq!(
            event.html_link.as_deref(),
            Some("https://www.google.com/calendar/event?eid=xyz")
        );
    }

    #[test]
    fn parses_all_day_event_from_date_fields() {
        let raw = serde_json::json!({
            "id": "ev-allday",
            "summary": "Company offsite",
            "start": {"date": "2026-07-23"},
            "end": {"date": "2026-07-24"}
        });

        let event = parse_google_event(&raw).expect("parse");
        assert!(event.is_all_day);
        assert_eq!(event.start_time, 1784764800); // 2026-07-23T00:00:00Z
        assert_eq!(event.end_time - event.start_time, 86_400);
        assert!(event.structured_meeting_urls.is_empty());
    }

    #[test]
    fn skips_thin_cancelled_instance_without_times() {
        // Incremental-style cancelled instances carry only id + status.
        let raw = serde_json::json!({"id": "ev-cancelled", "status": "cancelled"});
        assert!(parse_google_event(&raw).is_none());
    }

    #[test]
    fn keeps_cancelled_status_when_times_present() {
        let raw = serde_json::json!({
            "id": "ev-c",
            "status": "cancelled",
            "start": {"dateTime": "2026-07-22T10:00:00Z"},
            "end": {"dateTime": "2026-07-22T11:00:00Z"}
        });
        let event = parse_google_event(&raw).expect("parse");
        assert_eq!(event.status, "cancelled");
    }

    // ── build_google_create_request ────────────────────────────────────────

    fn new_event() -> crate::sync::calendar_provider::NewCalendarEvent {
        crate::sync::calendar_provider::NewCalendarEvent {
            title: "Planning".to_string(),
            description: String::new(),
            attendees: Vec::new(),
            start_time: 1784707200, // 2026-07-22T08:00:00Z (a Wednesday)
            end_time: 1784710800,
            time_zone: "Europe/Madrid".to_string(),
            recurrence: crate::sync::calendar_provider::EventRecurrence::None,
            request_meet_link: false,
        }
    }

    #[test]
    fn minimal_event_body_has_no_optional_fields_or_query() {
        let (query, body) = build_google_create_request(&new_event()).expect("build");
        assert_eq!(query, "");
        assert_eq!(body["summary"], "Planning");
        assert_eq!(body["start"]["dateTime"], "2026-07-22T08:00:00Z");
        assert!(body.get("description").is_none());
        assert!(body.get("attendees").is_none());
        assert!(body.get("recurrence").is_none());
        assert!(body.get("conferenceData").is_none());
    }

    #[test]
    fn attendees_add_invite_emails_and_send_updates() {
        let mut event = new_event();
        event.attendees = vec!["ana@example.com".to_string(), "bo@example.org".to_string()];
        let (query, body) = build_google_create_request(&event).expect("build");
        assert_eq!(
            query, "?sendUpdates=all",
            "invitees must actually receive the invitation email"
        );
        assert_eq!(body["attendees"][0]["email"], "ana@example.com");
        assert_eq!(body["attendees"][1]["email"], "bo@example.org");
    }

    #[test]
    fn recurrence_adds_rrule_and_required_timezone() {
        let mut event = new_event();
        event.recurrence = crate::sync::calendar_provider::EventRecurrence::Weekly;
        let (_, body) = build_google_create_request(&event).expect("build");
        assert_eq!(body["recurrence"][0], "RRULE:FREQ=WEEKLY");
        // Google mandates an explicit timeZone on recurring events — it keeps
        // occurrences on local wall time across DST.
        assert_eq!(body["start"]["timeZone"], "Europe/Madrid");
        assert_eq!(body["end"]["timeZone"], "Europe/Madrid");
    }

    #[test]
    fn meet_link_and_attendees_combine_both_query_params() {
        let mut event = new_event();
        event.attendees = vec!["ana@example.com".to_string()];
        event.request_meet_link = true;
        let (query, body) = build_google_create_request(&event).expect("build");
        assert_eq!(query, "?sendUpdates=all&conferenceDataVersion=1");
        assert_eq!(
            body["conferenceData"]["createRequest"]["conferenceSolutionKey"]["type"],
            "hangoutsMeet"
        );
    }

    #[test]
    fn description_is_included_when_present() {
        let mut event = new_event();
        event.description = "Agenda: budget".to_string();
        let (_, body) = build_google_create_request(&event).expect("build");
        assert_eq!(body["description"], "Agenda: budget");
    }

    #[test]
    fn parses_recurring_instance_master_link() {
        let raw = serde_json::json!({
            "id": "master_20260728T053000Z",
            "recurringEventId": "master",
            "start": {"dateTime": "2026-07-28T07:30:00+02:00"},
            "end": {"dateTime": "2026-07-28T08:30:00+02:00"}
        });
        let event = parse_google_event(&raw).expect("parse");
        assert_eq!(event.recurring_event_id.as_deref(), Some("master"));
    }

    // ── attendees_with_self_response ───────────────────────────────────────

    #[test]
    fn rsvp_updates_own_entry_case_insensitively_and_keeps_others() {
        let attendees = serde_json::json!([
            {"email": "Me@Example.com", "responseStatus": "needsAction"},
            {"email": "other@example.com", "responseStatus": "accepted", "optional": true}
        ]);
        let updated = attendees_with_self_response(Some(&attendees), "me@example.com", "declined");
        assert_eq!(updated[0]["responseStatus"], "declined");
        assert_eq!(updated[1]["responseStatus"], "accepted");
        assert_eq!(updated[1]["optional"], true, "unrelated attendee fields survive");
    }

    #[test]
    fn rsvp_appends_self_when_not_listed() {
        let updated = attendees_with_self_response(None, "me@example.com", "tentative");
        assert_eq!(updated[0]["email"], "me@example.com");
        assert_eq!(updated[0]["responseStatus"], "tentative");
    }

    // ── rrule_with_until ───────────────────────────────────────────────────

    #[test]
    fn until_replaces_count_and_existing_until() {
        // 1784707200 = 2026-07-22T08:00:00Z → UNTIL is one second earlier.
        let rules = vec![
            "RRULE:FREQ=WEEKLY;BYDAY=TU;COUNT=30".to_string(),
            "RRULE:FREQ=DAILY;UNTIL=20301231T000000Z".to_string(),
            "EXDATE;TZID=Europe/Madrid:20260721T073000".to_string(),
        ];
        let rewritten = rrule_with_until(&rules, 1784707200);
        assert_eq!(
            rewritten,
            vec![
                "RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20260722T075959Z",
                "RRULE:FREQ=DAILY;UNTIL=20260722T075959Z",
                "EXDATE;TZID=Europe/Madrid:20260721T073000",
            ]
        );
    }

    #[test]
    fn rrule_table_covers_all_presets() {
        use crate::sync::calendar_provider::EventRecurrence as R;
        assert_eq!(google_rrule(R::None), None);
        assert_eq!(google_rrule(R::Daily), Some("RRULE:FREQ=DAILY"));
        assert_eq!(google_rrule(R::Weekly), Some("RRULE:FREQ=WEEKLY"));
        assert_eq!(
            google_rrule(R::Weekdays),
            Some("RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR")
        );
        assert_eq!(google_rrule(R::Monthly), Some("RRULE:FREQ=MONTHLY"));
        assert_eq!(google_rrule(R::Yearly), Some("RRULE:FREQ=YEARLY"));
    }

    #[test]
    fn missing_optional_fields_default_to_empty() {
        let raw = serde_json::json!({
            "id": "ev-min",
            "start": {"dateTime": "2026-07-22T10:00:00Z"},
            "end": {"dateTime": "2026-07-22T11:00:00Z"}
        });
        let event = parse_google_event(&raw).expect("parse");
        assert_eq!(event.title, "");
        assert_eq!(event.organizer, "");
        assert!(event.attendees.is_empty());
        assert_eq!(event.status, "confirmed");
        assert_eq!(event.html_link, None);
    }
}
