//! Read-only calendar source backed by published iCalendar feeds (Proton
//! Calendar "share via link", or any `https://…/calendar.ics`). One
//! [`IcsFeedCalendarProvider`] serves every feed of an account: each feed is
//! one `reader` calendar whose `provider_calendar_id` is the feed id, and its
//! events are expanded locally within the sync window (the feed carries RRULE
//! masters, not instances — unlike Google/Graph, which expand server-side).
//!
//! The feed URL is a credential (Proton embeds the calendar's decryption key
//! in it): it is read from the secrets vault, never persisted elsewhere and
//! never logged — errors mention the host only.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::TimeZone;

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::CalendarAttendee;
use crate::services::calendar::ics::{
    parse_ics_time_with_fixed_zones, parse_vcalendar, unescape_text, ParsedCalendar, RawVEvent,
};
use crate::sync::calendar_provider::{
    CalendarProvider, NewCalendarEvent, ProviderCalendar, ProviderCalendarEvent, RsvpResponse,
};

/// Secrets-vault service under which each feed URL is stored (account = feed id).
pub const CALENDAR_FEED_KEYRING_SERVICE: &str = "emailops-calendar-feed";

/// Upper bound on expanded instances per recurring event and window. A
/// 30+90-day window holds ~120 daily occurrences; anything near this cap is a
/// hostile or broken rule (`FREQ=MINUTELY` without UNTIL).
pub const MAX_OCCURRENCES_PER_EVENT: u16 = 1_000;

/// Largest feed body accepted (decoded text). Real calendars are a few
/// hundred KB; the cap keeps a misconfigured URL from pulling an arbitrary
/// download into memory.
const MAX_FEED_BYTES: usize = 10 * 1024 * 1024;

/// Fetch seam so the provider is testable without a network.
#[async_trait]
pub trait IcsFetcher: Send + Sync {
    /// The feed body as text. Errors must not include the URL.
    async fn fetch(&self, url: &str) -> Result<String>;
}

/// Production fetcher: https only, shared connect/request timeouts, bounded
/// body. Failures name the host, never the URL (it carries the feed key).
pub struct ReqwestIcsFetcher {
    client: reqwest::Client,
}

impl Default for ReqwestIcsFetcher {
    fn default() -> Self {
        Self {
            client: crate::sync::http_client::provider_http_client(crate::sync::http_client::API_REQUEST_TIMEOUT),
        }
    }
}

/// Host part of a feed URL for messages; "feed" when it cannot be parsed.
pub fn feed_host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_else(|| "feed".to_string())
}

#[async_trait]
impl IcsFetcher for ReqwestIcsFetcher {
    async fn fetch(&self, url: &str) -> Result<String> {
        let parsed =
            reqwest::Url::parse(url).map_err(|_| AppError::InvalidInput("invalid calendar feed URL".into()))?;
        if parsed.scheme() != "https" {
            return Err(AppError::InvalidInput("calendar feed URL must use https".into()));
        }
        let host = feed_host(url);
        let response = self
            .client
            .get(parsed)
            .header(reqwest::header::ACCEPT, "text/calendar, */*;q=0.1")
            .send()
            .await
            .map_err(|e| AppError::SyncError(format!("calendar feed {host}: {}", e.without_url())))?;
        let status = response.status();
        if !status.is_success() {
            return Err(AppError::SyncError(format!(
                "calendar feed {host}: HTTP {}",
                status.as_u16()
            )));
        }
        if response.content_length().is_some_and(|len| len > MAX_FEED_BYTES as u64) {
            return Err(AppError::SyncError(format!(
                "calendar feed {host}: body larger than {MAX_FEED_BYTES} bytes"
            )));
        }
        let mut body: Vec<u8> = Vec::new();
        let mut response = response;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| AppError::SyncError(format!("calendar feed {host}: {}", e.without_url())))?
        {
            if body.len() + chunk.len() > MAX_FEED_BYTES {
                return Err(AppError::SyncError(format!(
                    "calendar feed {host}: body larger than {MAX_FEED_BYTES} bytes"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }
}

/// In-memory fetcher for tests: canned bodies per URL, optional failures.
#[cfg(test)]
#[derive(Default)]
pub struct FakeIcsFetcher {
    bodies: HashMap<String, String>,
    failing: HashSet<String>,
    calls: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl FakeIcsFetcher {
    pub fn with_body(mut self, url: &str, body: &str) -> Self {
        self.bodies.insert(url.to_string(), body.to_string());
        self
    }

    pub fn failing(mut self, url: &str) -> Self {
        self.failing.insert(url.to_string());
        self
    }

    pub fn fetch_count(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
#[async_trait]
impl IcsFetcher for FakeIcsFetcher {
    async fn fetch(&self, url: &str) -> Result<String> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.failing.contains(url) {
            return Err(AppError::SyncError(format!(
                "calendar feed {}: simulated failure",
                feed_host(url)
            )));
        }
        self.bodies
            .get(url)
            .cloned()
            .ok_or_else(|| AppError::SyncError(format!("calendar feed {}: no canned body", feed_host(url))))
    }
}

/// One subscribed feed with its (secret) URL resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedSource {
    pub id: String,
    pub label: String,
    pub url: String,
}

/// Every ICS feed of one account as a [`CalendarProvider`].
pub struct IcsFeedCalendarProvider {
    feeds: Vec<FeedSource>,
    fetcher: Arc<dyn IcsFetcher>,
}

impl IcsFeedCalendarProvider {
    pub fn new(feeds: Vec<FeedSource>, fetcher: Arc<dyn IcsFetcher>) -> Self {
        Self { feeds, fetcher }
    }

    /// Load the account's feeds and their URLs from the vault. A feed whose
    /// secret is missing is logged and left out (its calendar then drops out
    /// of the registry on the next sync) rather than failing every other feed.
    pub fn from_db(db: &Database, account_id: &str, fetcher: Arc<dyn IcsFetcher>) -> Result<Self> {
        let mut feeds = Vec::new();
        for feed in db.list_calendar_feeds(account_id)? {
            match crate::services::secrets_vault::get(CALENDAR_FEED_KEYRING_SERVICE, &feed.id)? {
                Some(url) => feeds.push(FeedSource {
                    id: feed.id,
                    label: feed.label,
                    url,
                }),
                None => crate::services::logger::log(
                    "error",
                    "sync",
                    format!(
                        "calendar feed '{}' ({}) has no URL in the vault — remove and re-add it",
                        feed.label, feed.host
                    ),
                ),
            }
        }
        Ok(Self::new(feeds, fetcher))
    }

    fn feed(&self, calendar_id: &str) -> Result<&FeedSource> {
        self.feeds
            .iter()
            .find(|f| f.id == calendar_id)
            .ok_or_else(|| AppError::NotFound(format!("calendar feed '{calendar_id}' not found")))
    }
}

#[async_trait]
impl CalendarProvider for IcsFeedCalendarProvider {
    async fn list_calendars(&self) -> Result<Vec<ProviderCalendar>> {
        Ok(self
            .feeds
            .iter()
            .enumerate()
            .map(|(index, feed)| ProviderCalendar {
                provider_calendar_id: feed.id.clone(),
                name: feed.label.clone(),
                color: String::new(),
                is_primary: index == 0,
                access_role: "reader".to_string(),
                selected: true,
            })
            .collect())
    }

    async fn list_events(
        &self,
        calendar_id: &str,
        window_start: i64,
        window_end: i64,
    ) -> Result<Vec<ProviderCalendarEvent>> {
        let feed = self.feed(calendar_id)?;
        let body = self.fetcher.fetch(&feed.url).await?;
        Ok(expand_events(&parse_vcalendar(&body), window_start, window_end))
    }

    async fn create_event(&self, _event: &NewCalendarEvent) -> Result<ProviderCalendarEvent> {
        Err(AppError::ReadOnlyCalendar)
    }

    async fn delete_event(
        &self,
        _calendar_id: &str,
        _provider_event_id: &str,
        _notify: bool,
        _message: &str,
    ) -> Result<()> {
        Err(AppError::ReadOnlyCalendar)
    }

    async fn truncate_recurring_event(
        &self,
        _calendar_id: &str,
        _master_id: &str,
        _first_removed_start: i64,
        _notify: bool,
    ) -> Result<()> {
        Err(AppError::ReadOnlyCalendar)
    }

    async fn rsvp_by_ical_uid(&self, _ical_uid: &str, _response: RsvpResponse, _self_email: &str) -> Result<()> {
        Err(AppError::ReadOnlyCalendar)
    }
}

// ── expansion planner (pure) ─────────────────────────────────────────────────

/// Fields of one VEVENT (master or override) that every instance inherits.
struct EventBody {
    uid: String,
    title: String,
    description: String,
    location: String,
    start: i64,
    end: i64,
    is_all_day: bool,
    timezone: String,
    organizer: String,
    attendees: Vec<CalendarAttendee>,
    urls: Vec<String>,
    status: String,
}

impl EventBody {
    fn instance(
        &self,
        provider_event_id: String,
        start: i64,
        end: i64,
        recurring_event_id: Option<String>,
    ) -> ProviderCalendarEvent {
        ProviderCalendarEvent {
            provider_event_id,
            title: self.title.clone(),
            description: self.description.clone(),
            location: self.location.clone(),
            start_time: start,
            end_time: end,
            is_all_day: self.is_all_day,
            timezone: self.timezone.clone(),
            organizer: self.organizer.clone(),
            attendees: self.attendees.clone(),
            structured_meeting_urls: self.urls.clone(),
            status: self.status.clone(),
            html_link: None,
            recurring_event_id,
        }
    }
}

fn timezone_param(params: &[String]) -> Option<&str> {
    crate::services::calendar::ics::tzid_from_params(params)
}

/// Add `TZID=<default>` to floating times using the feed's `X-WR-TIMEZONE`.
fn params_with_default_tz(params: &[String], value: &str, default_tz: Option<&str>) -> Vec<String> {
    let has_tz = timezone_param(params).is_some();
    let is_date = params.iter().any(|p| p.eq_ignore_ascii_case("VALUE=DATE")) || !value.contains('T');
    match default_tz {
        Some(tz) if !has_tz && !is_date && !value.trim().ends_with('Z') => {
            let mut out = params.to_vec();
            out.push(format!("TZID={tz}"));
            out
        }
        _ => params.to_vec(),
    }
}

fn time_of(
    event: &RawVEvent,
    name: &str,
    default_tz: Option<&str>,
    fixed_zones: &HashMap<String, chrono::FixedOffset>,
) -> Option<(i64, bool)> {
    let prop = event.first(name)?;
    parse_ics_time_with_fixed_zones(
        &prop.value,
        &params_with_default_tz(&prop.params, &prop.value, default_tz),
        fixed_zones,
    )
}

/// ISO 8601 duration (`PT45M`, `P1D`, `P1DT2H30M`, `-PT1H`) in seconds.
fn parse_iso_duration(value: &str) -> Option<i64> {
    let value = value.trim();
    let (negative, rest) = match value.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let rest = rest.strip_prefix('P')?;
    let mut total: i64 = 0;
    let mut number = String::new();
    let mut in_time = false;
    for c in rest.chars() {
        match c {
            'T' => in_time = true,
            d if d.is_ascii_digit() => number.push(d),
            unit => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                let secs = match (unit, in_time) {
                    ('W', false) => n * 7 * 86_400,
                    ('D', false) => n * 86_400,
                    ('H', true) => n * 3_600,
                    ('M', true) => n * 60,
                    ('S', true) => n,
                    _ => return None,
                };
                total += secs;
            }
        }
    }
    if !number.is_empty() {
        return None;
    }
    Some(if negative { -total } else { total })
}

fn mailto_address(value: &str) -> String {
    let v = value.trim();
    v.strip_prefix("mailto:")
        .or_else(|| v.strip_prefix("MAILTO:"))
        .unwrap_or(v)
        .to_ascii_lowercase()
}

fn attendee_response(params: &[String]) -> String {
    let partstat = params
        .iter()
        .find_map(|p| p.strip_prefix("PARTSTAT=").or_else(|| p.strip_prefix("partstat=")))
        .map(|p| p.trim_matches('"').to_ascii_uppercase());
    match partstat.as_deref() {
        Some("ACCEPTED") => "accepted",
        Some("DECLINED") => "declined",
        Some("TENTATIVE") => "tentative",
        _ => "needsAction",
    }
    .to_string()
}

fn event_body(
    event: &RawVEvent,
    default_tz: Option<&str>,
    fixed_zones: &HashMap<String, chrono::FixedOffset>,
) -> Option<EventBody> {
    let uid = event.first_value("UID")?.trim().to_string();
    let (start, is_all_day) = time_of(event, "DTSTART", default_tz, fixed_zones)?;
    let end = match time_of(event, "DTEND", default_tz, fixed_zones) {
        Some((end, _)) => end,
        None => match event.first_value("DURATION").and_then(parse_iso_duration) {
            Some(secs) => start + secs,
            None if is_all_day => start + 86_400,
            None => start + 3_600,
        },
    };
    let dtstart = event.first("DTSTART")?;
    let params = params_with_default_tz(&dtstart.params, &dtstart.value, default_tz);
    let timezone = timezone_param(&params).unwrap_or_default().to_string();
    let text = |name: &str| {
        event
            .first_value(name)
            .map(|v| unescape_text(v.trim()))
            .unwrap_or_default()
    };
    let status = match event
        .first_value("STATUS")
        .map(|s| s.trim().to_ascii_uppercase())
        .as_deref()
    {
        Some("CANCELLED") => "cancelled",
        Some("TENTATIVE") => "tentative",
        _ => "confirmed",
    }
    .to_string();
    Some(EventBody {
        uid,
        title: text("SUMMARY"),
        description: text("DESCRIPTION"),
        location: text("LOCATION"),
        start,
        end: end.max(start),
        is_all_day,
        timezone,
        organizer: event.first_value("ORGANIZER").map(mailto_address).unwrap_or_default(),
        attendees: event
            .all("ATTENDEE")
            .map(|p| CalendarAttendee {
                email: mailto_address(&p.value),
                response: attendee_response(&p.params),
            })
            .collect(),
        // Proton Calendar publishes its video-conference link (Proton Meet,
        // Zoom) in its own property; it is the authoritative join URL, ahead
        // of a plain URL property.
        urls: event
            .all("X-PM-CONFERENCE-URL")
            .chain(event.all("URL"))
            .map(|p| p.value.trim().to_string())
            .filter(|u| u.starts_with("https://"))
            .collect(),
        status,
    })
}

fn overlaps(start: i64, end: i64, window_start: i64, window_end: i64) -> bool {
    start < window_end && end > window_start
}

/// Stable per-occurrence suffix: the occurrence's own start in UTC (its
/// RECURRENCE-ID), so a moved instance keeps its row across syncs.
fn occurrence_id(uid: &str, recurrence_epoch: i64, is_all_day: bool) -> String {
    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(recurrence_epoch, 0).unwrap_or_default();
    if is_all_day {
        format!("{uid}_{}", dt.format("%Y%m%d"))
    } else {
        format!("{uid}_{}", dt.format("%Y%m%dT%H%M%SZ"))
    }
}

/// The `DTSTART`/`RRULE`/`RDATE`/`EXDATE` lines of a master, normalised for the
/// `rrule` parser: all-day values become UTC midnights (it does not read
/// `VALUE=DATE`), floating values get the feed's default zone.
fn rrule_source(
    event: &RawVEvent,
    is_all_day: bool,
    default_tz: Option<&str>,
    fixed_zones: &HashMap<String, chrono::FixedOffset>,
    recurrence_offset: Option<i32>,
) -> Option<String> {
    let mut dtstart_lines = Vec::new();
    for prop in event.all("DTSTART") {
        let params = params_with_default_tz(&prop.params, &prop.value, default_tz);
        let fixed_property = timezone_param(&params)
            .is_some_and(|tzid| tzid.parse::<chrono_tz::Tz>().is_err() && fixed_zones.contains_key(tzid));
        if is_all_day || recurrence_offset.is_some() || fixed_property {
            for date in prop.value.split(',') {
                let date_params = if is_all_day {
                    vec!["VALUE=DATE".to_string()]
                } else {
                    params.clone()
                };
                let (epoch, _) = parse_ics_time_with_fixed_zones(date, &date_params, fixed_zones)?;
                let epoch = epoch.checked_add(i64::from(recurrence_offset.unwrap_or(0)))?;
                let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(epoch, 0)?;
                dtstart_lines.push(format!("DTSTART:{}", dt.format("%Y%m%dT%H%M%SZ")));
            }
        } else {
            let head = std::iter::once("DTSTART".to_string())
                .chain(params)
                .collect::<Vec<_>>()
                .join(";");
            dtstart_lines.push(format!("{head}:{}", prop.value.trim()));
        }
    }
    if dtstart_lines.is_empty() {
        return None;
    }
    let dtstart = dtstart_lines[0].clone();

    let mut rule = Vec::new();
    for part in event.first_value("RRULE")?.trim().split(';') {
        if let Some(offset) = recurrence_offset {
            if let Some((name, value)) = part.split_once('=') {
                if name.eq_ignore_ascii_case("UNTIL") {
                    if value.ends_with('Z') {
                        let (epoch, _) = parse_ics_time_with_fixed_zones(value, &[], fixed_zones)?;
                        let epoch = epoch.checked_add(i64::from(offset))?;
                        let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(epoch, 0)?;
                        rule.push(format!("UNTIL={}", dt.format("%Y%m%dT%H%M%SZ")));
                        continue;
                    } else if !value.contains('T') {
                        let (epoch, _) =
                            parse_ics_time_with_fixed_zones(&format!("{}T000000", value.trim()), &[], fixed_zones)?;
                        let epoch = epoch.checked_add(24 * 60 * 60 - 1)?;
                        let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(epoch, 0)?;
                        rule.push(format!("UNTIL={}", dt.format("%Y%m%dT%H%M%SZ")));
                        continue;
                    }
                }
            }
        }
        rule.push(part.to_string());
    }
    let rrule_line = format!("RRULE:{}", rule.join(";"));

    let mut optional_lines = Vec::new();
    for name in ["RDATE", "EXDATE"] {
        for prop in event.all(name) {
            let params = params_with_default_tz(&prop.params, &prop.value, default_tz);
            let fixed_property = timezone_param(&params)
                .is_some_and(|tzid| tzid.parse::<chrono_tz::Tz>().is_err() && fixed_zones.contains_key(tzid));
            let is_period = params.iter().any(|p| {
                p.split_once('=').map_or(false, |(name, value)| {
                    name.trim().eq_ignore_ascii_case("VALUE")
                        && value.trim().trim_matches('"').eq_ignore_ascii_case("PERIOD")
                })
            });
            if is_period {
                crate::services::logger::log(
                    "warn",
                    "sync",
                    "calendar feed: skipped invalid recurrence date".to_string(),
                );
                continue;
            }

            let normalized = if is_all_day || recurrence_offset.is_some() || fixed_property {
                let mut out = Vec::new();
                for date in prop.value.split(',') {
                    let date_params = if is_all_day {
                        vec!["VALUE=DATE".to_string()]
                    } else {
                        params.clone()
                    };
                    match parse_ics_time_with_fixed_zones(date, &date_params, fixed_zones) {
                        Some((epoch, _)) => {
                            if let Some(epoch) = epoch.checked_add(i64::from(recurrence_offset.unwrap_or(0))) {
                                if let Some(dt) = chrono::DateTime::<chrono::Utc>::from_timestamp(epoch, 0) {
                                    out.push(format!("{name}:{}", dt.format("%Y%m%dT%H%M%SZ")));
                                }
                            }
                        }
                        None => {
                            crate::services::logger::log(
                                "warn",
                                "sync",
                                "calendar feed: skipped invalid recurrence date".to_string(),
                            );
                        }
                    }
                }
                out
            } else {
                prop.value
                    .split(',')
                    .map(|date| {
                        let head = std::iter::once(name.to_string())
                            .chain(params.clone())
                            .collect::<Vec<_>>()
                            .join(";");
                        format!("{head}:{}", date.trim())
                    })
                    .collect()
            };

            for line in normalized {
                let candidate = format!("{}\n{}\n{}", dtstart, line, rrule_line);
                if rrule::RRuleSet::from_str(&candidate).is_ok() {
                    optional_lines.push(line);
                } else {
                    crate::services::logger::log(
                        "warn",
                        "sync",
                        "calendar feed: skipped unsupported recurrence date".to_string(),
                    );
                }
            }
        }
    }

    let mut lines = dtstart_lines;
    lines.extend(optional_lines);
    lines.push(rrule_line);
    Some(lines.join("\n"))
}

fn utc_dt(epoch: i64) -> Option<chrono::DateTime<rrule::Tz>> {
    rrule::Tz::UTC.timestamp_opt(epoch, 0).single()
}

/// Occurrence starts (UTC epoch) of a recurring master inside the window.
/// `None` when the rule cannot be parsed or evaluated.
fn occurrences(
    event: &RawVEvent,
    body: &EventBody,
    window_start: i64,
    window_end: i64,
    default_tz: Option<&str>,
    fixed_zones: &HashMap<String, chrono::FixedOffset>,
) -> Option<Vec<i64>> {
    let duration = (body.end - body.start).max(0);
    // rrule supports IANA zones, but not FixedOffset. Represent a fixed
    // zone's wall clock in UTC while expanding, then translate the results.
    // This preserves local BYDAY/BYHOUR even when its UTC date differs.
    let recurrence_offset = event
        .first("DTSTART")
        .filter(|p| !body.is_all_day && !p.value.trim().ends_with('Z'))
        .filter(|_| body.timezone.parse::<chrono_tz::Tz>().is_err())
        .and_then(|_| fixed_zones.get(&body.timezone))
        .map(chrono::FixedOffset::local_minus_utc);
    let shift = i64::from(recurrence_offset.unwrap_or(0));
    let source = rrule_source(event, body.is_all_day, default_tz, fixed_zones, recurrence_offset)?;
    let set = rrule::RRuleSet::from_str(&source).ok()?;
    let result = set
        .after(utc_dt(window_start.checked_sub(duration)?.checked_add(shift)?)?)
        .before(utc_dt(window_end.checked_add(shift)?)?)
        .all(MAX_OCCURRENCES_PER_EVENT);
    if result.limited {
        crate::services::logger::log(
            "error",
            "sync",
            format!(
                "calendar feed: recurring event '{}' hit the {MAX_OCCURRENCES_PER_EVENT}-instance cap",
                body.uid
            ),
        );
    }
    result.dates.iter().map(|d| d.timestamp().checked_sub(shift)).collect()
}

/// Expand a parsed feed into event instances overlapping
/// `[window_start, window_end)`: single events as-is, recurring masters via
/// their RRULE/EXDATE with RECURRENCE-ID overrides applied, cancelled ones
/// dropped. Never fails: a rule the parser rejects falls back to its master
/// occurrence (logged), so one bad event cannot hide the rest of the feed.
pub fn expand_events(cal: &ParsedCalendar, window_start: i64, window_end: i64) -> Vec<ProviderCalendarEvent> {
    let default_tz = cal.timezone.as_deref();
    let mut masters: Vec<&RawVEvent> = Vec::new();
    // uid → (recurrence-id epoch → override)
    let mut overrides: HashMap<String, Vec<(i64, &RawVEvent)>> = HashMap::new();
    for event in &cal.events {
        let Some(uid) = event.first_value("UID") else { continue };
        match time_of(event, "RECURRENCE-ID", default_tz, &cal.fixed_zones) {
            Some((rid, _)) => overrides.entry(uid.trim().to_string()).or_default().push((rid, event)),
            None => masters.push(event),
        }
    }

    let mut out = Vec::new();
    let mut seen_uids: HashSet<String> = HashSet::new();
    for master in masters {
        let Some(body) = event_body(master, default_tz, &cal.fixed_zones) else {
            continue;
        };
        if !seen_uids.insert(body.uid.clone()) {
            continue; // duplicate master for the same UID: first wins
        }
        let uid_overrides = overrides.remove(&body.uid).unwrap_or_default();
        if body.status == "cancelled" {
            continue;
        }
        if master.first("RRULE").is_none() {
            if overlaps(body.start, body.end, window_start, window_end) {
                out.push(body.instance(body.uid.clone(), body.start, body.end, None));
            }
            continue;
        }

        let Some(starts) = occurrences(master, &body, window_start, window_end, default_tz, &cal.fixed_zones) else {
            crate::services::logger::log(
                "error",
                "sync",
                format!(
                    "calendar feed: could not expand the recurrence of event '{}'; showing its first occurrence only",
                    body.uid
                ),
            );
            if overlaps(body.start, body.end, window_start, window_end) {
                out.push(body.instance(body.uid.clone(), body.start, body.end, None));
            }
            continue;
        };
        let duration = (body.end - body.start).max(0);
        let mut consumed: HashSet<i64> = HashSet::new();
        for start in starts {
            let id = occurrence_id(&body.uid, start, body.is_all_day);
            match uid_overrides.iter().find(|(rid, _)| *rid == start) {
                Some((rid, event)) => {
                    consumed.insert(*rid);
                    if let Some(ov) = event_body(event, default_tz, &cal.fixed_zones) {
                        if ov.status != "cancelled" && overlaps(ov.start, ov.end, window_start, window_end) {
                            out.push(ov.instance(id, ov.start, ov.end, Some(body.uid.clone())));
                        }
                    }
                }
                None => {
                    if overlaps(start, start + duration, window_start, window_end) {
                        out.push(body.instance(id, start, start + duration, Some(body.uid.clone())));
                    }
                }
            }
        }
        // Overrides whose original slot lies outside the window but were moved
        // into it.
        for (rid, event) in uid_overrides.iter().filter(|(rid, _)| !consumed.contains(rid)) {
            if let Some(ov) = event_body(event, default_tz, &cal.fixed_zones) {
                if ov.status != "cancelled" && overlaps(ov.start, ov.end, window_start, window_end) {
                    out.push(ov.instance(
                        occurrence_id(&body.uid, *rid, body.is_all_day),
                        ov.start,
                        ov.end,
                        Some(body.uid.clone()),
                    ));
                }
            }
        }
    }
    // Overrides whose master never appeared in the feed.
    for (uid, list) in overrides {
        for (rid, event) in list {
            if let Some(ov) = event_body(event, default_tz, &cal.fixed_zones) {
                if ov.status != "cancelled" && overlaps(ov.start, ov.end, window_start, window_end) {
                    out.push(ov.instance(
                        occurrence_id(&uid, rid, ov.is_all_day),
                        ov.start,
                        ov.end,
                        Some(uid.clone()),
                    ));
                }
            }
        }
    }
    out.sort_by(|a, b| {
        a.start_time
            .cmp(&b.start_time)
            .then_with(|| a.provider_event_id.cmp(&b.provider_event_id))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::calendar::ics::parse_vcalendar;
    use crate::sync::calendar_provider::CalendarProvider;
    use chrono::TimeZone;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        chrono::Utc
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .expect("valid")
            .timestamp()
    }

    fn vcal(events: &[&str]) -> String {
        let mut s = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Feed\r\n");
        for e in events {
            s.push_str("BEGIN:VEVENT\r\n");
            s.push_str(&e.replace('\n', "\r\n"));
            s.push_str("\r\nEND:VEVENT\r\n");
        }
        s.push_str("END:VCALENDAR\r\n");
        s
    }

    fn expand(events: &[&str], ws: i64, we: i64) -> Vec<ProviderCalendarEvent> {
        expand_events(&parse_vcalendar(&vcal(events)), ws, we)
    }

    #[test]
    fn unsupported_period_rdate_keeps_iana_and_floating_series_beyond_master() {
        for (zones, dtstart) in [
            (
                "",
                "DTSTART;TZID=Europe/Madrid:20260324T100000\r\nDTEND;TZID=Europe/Madrid:20260324T110000",
            ),
            (
                "X-WR-TIMEZONE:Europe/Madrid\r\n",
                "DTSTART:20260324T100000\r\nDTEND:20260324T110000",
            ),
        ] {
            let event = format!("UID:period-regression@example.test\r\n{dtstart}\r\nRRULE:FREQ=WEEKLY;COUNT=4\r\nRDATE;VALUE=PERIOD:20260401T100000/20260401T110000,20260402T100000/PT1H");
            let calendar = with_zones(&[&event], zones);
            let out = expand_events(&calendar, utc(2026, 3, 30, 0, 0), utc(2026, 4, 30, 0, 0));
            let actual: Vec<i64> = out.iter().map(|e| e.start_time).collect();
            let expected = vec![utc(2026, 3, 31, 8, 0), utc(2026, 4, 7, 8, 0), utc(2026, 4, 14, 8, 0)];
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn fixed_mixed_optional_dates_skip_invalid_elements_without_losing_series() {
        let event = "UID:dates-regression@example.test\r\nDTSTART;TZID=\"Example Fixed Time\":20260901T003000\r\nDTEND;TZID=\"Example Fixed Time\":20260901T013000\r\nRRULE:FREQ=DAILY;COUNT=4\r\nRDATE;TZID=\"Example Fixed Time\":bad-date,20260905T003000\r\nEXDATE;TZID=\"Example Fixed Time\":not-a-date,20260903T003000";
        let calendar = with_zones(&[event], FIXED_ZONE);
        let out = expand_events(&calendar, utc(2026, 9, 1, 0, 0), utc(2026, 9, 7, 0, 0));
        let actual: Vec<i64> = out.iter().map(|e| e.start_time).collect();
        let expected = vec![
            utc(2026, 9, 1, 22, 30),
            utc(2026, 9, 3, 22, 30),
            utc(2026, 9, 4, 22, 30),
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn date_only_fixed_until_includes_last_local_day_for_both_offset_signs() {
        for (zones, ws, we, expected) in [
            (
                FIXED_ZONE.to_string(),
                utc(2026, 10, 24, 0, 0),
                utc(2026, 10, 28, 0, 0),
                vec![utc(2026, 10, 24, 21, 30), utc(2026, 10, 25, 21, 30)],
            ),
            (
                FIXED_ZONE.replace("+0200", "-0330"),
                utc(2026, 10, 25, 0, 0),
                utc(2026, 10, 29, 0, 0),
                vec![utc(2026, 10, 25, 3, 0), utc(2026, 10, 26, 3, 0)],
            ),
        ] {
            let event = "UID:until-regression@example.test\r\nDTSTART;TZID=\"Example Fixed Time\":20261023T233000\r\nDTEND;TZID=\"Example Fixed Time\":20261023T234500\r\nRRULE:FREQ=DAILY;UNTIL=20261025";
            let calendar = with_zones(&[event], &zones);
            let out = expand_events(&calendar, ws, we);
            let actual: Vec<i64> = out.iter().map(|e| e.start_time).collect();
            assert_eq!(actual, expected);
        }
    }

    const FIXED_ZONE: &str = "BEGIN:VTIMEZONE\r\nTZID:Example Fixed Time\r\nBEGIN:STANDARD\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0200\r\nDTSTART:16010101T000000\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n";

    fn with_zones(events: &[&str], zones: &str) -> ParsedCalendar {
        let document = vcal(events).replacen("VERSION:2.0\r\n", &format!("VERSION:2.0\r\n{zones}"), 1);
        parse_vcalendar(&document)
    }

    #[test]
    fn embedded_fixed_zone_is_shared_by_explicit_and_default_feed_times() {
        let calendar = with_zones(&[
            "UID:fixed@x\nDTSTART;TzId=\" Example Fixed Time \":20261023T103000\nDTEND;TZID=\"Example Fixed Time\":20261023T113000",
            "UID:floating@x\nDTSTART:20261023T103000\nDTEND:20261023T113000",
            "UID:utc@x\nDTSTART;TZID=\"Example Fixed Time\":20261023T103000Z",
            "UID:day@x\nDTSTART;VALUE=DATE:20261023",
        ], &format!("{FIXED_ZONE}X-WR-TIMEZONE:Example Fixed Time\r\n"));
        let out = expand_events(&calendar, utc(2026, 10, 23, 0, 0), utc(2026, 10, 24, 0, 0));
        assert_eq!(out.len(), 4);
        for id in ["fixed@x", "floating@x"] {
            let event = out.iter().find(|event| event.provider_event_id == id).unwrap();
            assert_eq!(
                (event.start_time, event.end_time),
                (utc(2026, 10, 23, 8, 30), utc(2026, 10, 23, 9, 30))
            );
        }
        assert_eq!(
            out.iter()
                .find(|event| event.provider_event_id == "utc@x")
                .unwrap()
                .start_time,
            utc(2026, 10, 23, 10, 30)
        );
        let day = out.iter().find(|event| event.provider_event_id == "day@x").unwrap();
        assert!(day.is_all_day);
        assert_eq!(day.start_time, utc(2026, 10, 23, 0, 0));
    }

    #[test]
    fn fixed_zone_recurrence_keeps_wall_day_exdates_rdates_until_and_override_ids() {
        let calendar = with_zones(&[
            "UID:fixed-series@x\nDTSTART;TZID=\"Example Fixed Time\":20260728T003000\nDTEND;TZID=\"Example Fixed Time\":20260728T013000\nRRULE:FREQ=WEEKLY;BYDAY=TU;BYHOUR=0;BYMINUTE=30;UNTIL=20260810T223000Z\nEXDATE;TZID=\"Example Fixed Time\":20260804T003000\nRDATE;TZID=\"Example Fixed Time\":20260806T003000",
            "UID:fixed-series@x\nRECURRENCE-ID;TZID=\"Example Fixed Time\":20260811T003000\nDTSTART;TZID=\"Example Fixed Time\":20260811T020000\nDTEND;TZID=\"Example Fixed Time\":20260811T030000\nSUMMARY:Moved",
        ], FIXED_ZONE);
        let out = expand_events(&calendar, utc(2026, 7, 27, 0, 0), utc(2026, 8, 18, 0, 0));
        assert_eq!(
            out.iter().map(|event| event.start_time).collect::<Vec<_>>(),
            vec![
                utc(2026, 7, 27, 22, 30),
                utc(2026, 8, 5, 22, 30),
                utc(2026, 8, 11, 0, 0)
            ]
        );
        let moved = out.iter().find(|event| event.title == "Moved").unwrap();
        assert_eq!(moved.provider_event_id, "fixed-series@x_20260810T223000Z");
        assert_eq!(moved.end_time, utc(2026, 8, 11, 1, 0));
    }

    #[test]
    fn embedded_iana_definition_does_not_override_dst_recurrence() {
        let zones = FIXED_ZONE
            .replace("Example Fixed Time", "Europe/Madrid")
            .replace("+0200", "+0900");
        let calendar = with_zones(
            &["UID:iana@x\nDTSTART;TZID=Europe/Madrid:20260324T100000\nRRULE:FREQ=WEEKLY;COUNT=2"],
            &zones,
        );
        let out = expand_events(&calendar, utc(2026, 3, 23, 0, 0), utc(2026, 4, 6, 0, 0));
        assert_eq!(
            out.iter().map(|event| event.start_time).collect::<Vec<_>>(),
            vec![utc(2026, 3, 24, 9, 0), utc(2026, 3, 31, 8, 0)]
        );
    }

    #[test]
    fn unusable_timezone_metadata_keeps_feed_events_with_legacy_fallback() {
        for zones in [
            format!("{FIXED_ZONE}{FIXED_ZONE}"),
            FIXED_ZONE.replace("TZOFFSETTO:+0200", "TZOFFSETTO:invalid"),
        ] {
            let calendar = with_zones(
                &["UID:fallback@x\nDTSTART;TZID=\"Example Fixed Time\":20261023T103000"],
                &zones,
            );
            let out = expand_events(&calendar, utc(2026, 10, 23, 0, 0), utc(2026, 10, 24, 0, 0));
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].start_time, utc(2026, 10, 23, 10, 30));
        }
    }

    const WEEKLY: &str = "UID:w@x\nDTSTART;TZID=Europe/Madrid:20260728T073000\nDTEND;TZID=Europe/Madrid:20260728T083000\nRRULE:FREQ=WEEKLY;BYDAY=TU\nSUMMARY:Weekly";

    // ── expansion planner ────────────────────────────────────────────────

    #[test]
    fn single_event_inside_window_is_emitted_with_its_fields() {
        let out = expand(
            &["UID:one@x\nDTSTART:20260729T070000Z\nDTEND:20260729T080000Z\nSUMMARY:Standup\\, daily\nLOCATION:Room 1\nDESCRIPTION:Notes\nORGANIZER;CN=Boss:mailto:Boss@Example.com\nATTENDEE;CN=A:mailto:a@example.com\nURL:https://meet.example.com/abc\nSTATUS:TENTATIVE"],
            utc(2026, 7, 1, 0, 0),
            utc(2026, 8, 1, 0, 0),
        );
        assert_eq!(out.len(), 1);
        let e = &out[0];
        assert_eq!(e.provider_event_id, "one@x");
        assert_eq!(e.title, "Standup, daily");
        assert_eq!(e.location, "Room 1");
        assert_eq!(e.description, "Notes");
        assert_eq!(
            (e.start_time, e.end_time),
            (utc(2026, 7, 29, 7, 0), utc(2026, 7, 29, 8, 0))
        );
        assert!(!e.is_all_day);
        assert_eq!(e.organizer, "boss@example.com");
        assert_eq!(e.attendees.len(), 1);
        assert_eq!(e.attendees[0].email, "a@example.com");
        assert_eq!(
            e.structured_meeting_urls,
            vec!["https://meet.example.com/abc".to_string()]
        );
        assert_eq!(e.status, "tentative");
        assert_eq!(e.recurring_event_id, None);
        assert_eq!(e.html_link, None);
    }

    #[test]
    fn proton_conference_property_is_a_structured_meeting_url_ahead_of_url() {
        // Proton Calendar publishes the Meet link in its own property; it must
        // reach the meeting-link extractor as a structured (authoritative) URL.
        let out = expand(
            &["UID:p@x\nDTSTART:20260729T070000Z\nDTEND:20260729T080000Z\nSUMMARY:Sync\nX-PM-CONFERENCE-URL:https://meet.proton.me/id-AAA1A8A1A0#pwd-DDDDDDDDDDDD\nURL:https://example.com/agenda"],
            utc(2026, 7, 1, 0, 0),
            utc(2026, 8, 1, 0, 0),
        );
        assert_eq!(
            out[0].structured_meeting_urls,
            vec![
                "https://meet.proton.me/id-AAA1A8A1A0#pwd-DDDDDDDDDDDD".to_string(),
                "https://example.com/agenda".to_string()
            ]
        );
    }

    #[test]
    fn event_outside_window_is_not_emitted() {
        let out = expand(
            &["UID:one@x\nDTSTART:20260629T070000Z\nDTEND:20260629T080000Z\nSUMMARY:Old"],
            utc(2026, 7, 1, 0, 0),
            utc(2026, 8, 1, 0, 0),
        );
        assert!(out.is_empty());
    }

    #[test]
    fn missing_dtend_falls_back_to_duration_then_an_hour_or_a_day() {
        let ws = utc(2026, 7, 1, 0, 0);
        let we = utc(2026, 8, 1, 0, 0);
        let dur = expand(
            &["UID:d@x\nDTSTART:20260729T070000Z\nDURATION:PT45M\nSUMMARY:D"],
            ws,
            we,
        );
        assert_eq!(dur[0].end_time, utc(2026, 7, 29, 7, 45));
        let hour = expand(&["UID:h@x\nDTSTART:20260729T070000Z\nSUMMARY:H"], ws, we);
        assert_eq!(hour[0].end_time, utc(2026, 7, 29, 8, 0));
        let day = expand(&["UID:a@x\nDTSTART;VALUE=DATE:20260729\nSUMMARY:A"], ws, we);
        assert!(day[0].is_all_day);
        assert_eq!(
            (day[0].start_time, day[0].end_time),
            (utc(2026, 7, 29, 0, 0), utc(2026, 7, 30, 0, 0))
        );
    }

    #[test]
    fn weekly_rrule_expands_only_inside_the_window_with_stable_instance_ids() {
        let out = expand(&[WEEKLY], utc(2026, 7, 27, 0, 0), utc(2026, 8, 24, 0, 0));
        let starts: Vec<i64> = out.iter().map(|e| e.start_time).collect();
        assert_eq!(
            starts,
            vec![
                utc(2026, 7, 28, 5, 30),
                utc(2026, 8, 4, 5, 30),
                utc(2026, 8, 11, 5, 30),
                utc(2026, 8, 18, 5, 30)
            ]
        );
        assert_eq!(out[1].provider_event_id, "w@x_20260804T053000Z");
        assert_eq!(out[1].end_time, utc(2026, 8, 4, 6, 30));
        assert!(out.iter().all(|e| e.recurring_event_id.as_deref() == Some("w@x")));
        assert!(out.iter().all(|e| e.timezone == "Europe/Madrid"));
    }

    #[test]
    fn occurrences_before_the_window_are_skipped_not_counted() {
        // Window starts weeks after DTSTART: expansion must still find the
        // in-window ones (not stop at the first N from DTSTART).
        let out = expand(&[WEEKLY], utc(2026, 9, 1, 0, 0), utc(2026, 9, 15, 0, 0));
        let starts: Vec<i64> = out.iter().map(|e| e.start_time).collect();
        assert_eq!(starts, vec![utc(2026, 9, 1, 5, 30), utc(2026, 9, 8, 5, 30)]);
    }

    #[test]
    fn exdate_removes_that_occurrence() {
        let with_ex = format!("{WEEKLY}\nEXDATE;TZID=Europe/Madrid:20260804T073000");
        let out = expand(&[&with_ex], utc(2026, 7, 27, 0, 0), utc(2026, 8, 24, 0, 0));
        let starts: Vec<i64> = out.iter().map(|e| e.start_time).collect();
        assert_eq!(
            starts,
            vec![
                utc(2026, 7, 28, 5, 30),
                utc(2026, 8, 11, 5, 30),
                utc(2026, 8, 18, 5, 30)
            ]
        );
    }

    #[test]
    fn recurrence_id_override_replaces_the_instance_time_and_title_keeping_its_id() {
        let moved = "UID:w@x\nRECURRENCE-ID;TZID=Europe/Madrid:20260811T073000\nDTSTART;TZID=Europe/Madrid:20260811T090000\nDTEND;TZID=Europe/Madrid:20260811T100000\nSUMMARY:Moved";
        let out = expand(&[WEEKLY, moved], utc(2026, 7, 27, 0, 0), utc(2026, 8, 24, 0, 0));
        assert_eq!(out.len(), 4);
        let inst = out
            .iter()
            .find(|e| e.provider_event_id == "w@x_20260811T053000Z")
            .expect("override kept id");
        assert_eq!(inst.title, "Moved");
        assert_eq!(
            (inst.start_time, inst.end_time),
            (utc(2026, 8, 11, 7, 0), utc(2026, 8, 11, 8, 0))
        );
        assert_eq!(inst.recurring_event_id.as_deref(), Some("w@x"));
        assert!(
            !out.iter().any(|e| e.start_time == utc(2026, 8, 11, 5, 30)),
            "original slot replaced"
        );
    }

    #[test]
    fn override_moved_into_the_window_from_outside_is_emitted() {
        let moved = "UID:w@x\nRECURRENCE-ID;TZID=Europe/Madrid:20260922T073000\nDTSTART;TZID=Europe/Madrid:20260820T073000\nDTEND;TZID=Europe/Madrid:20260820T083000\nSUMMARY:Pulled in";
        let out = expand(&[WEEKLY, moved], utc(2026, 8, 17, 0, 0), utc(2026, 8, 24, 0, 0));
        let ids: Vec<&str> = out.iter().map(|e| e.provider_event_id.as_str()).collect();
        assert_eq!(ids, vec!["w@x_20260818T053000Z", "w@x_20260922T053000Z"]);
    }

    #[test]
    fn cancelled_override_and_cancelled_master_are_dropped() {
        let cancelled = "UID:w@x\nRECURRENCE-ID;TZID=Europe/Madrid:20260804T073000\nDTSTART;TZID=Europe/Madrid:20260804T073000\nSTATUS:CANCELLED\nSUMMARY:Weekly";
        let out = expand(&[WEEKLY, cancelled], utc(2026, 7, 27, 0, 0), utc(2026, 8, 24, 0, 0));
        assert_eq!(out.len(), 3);
        assert!(!out.iter().any(|e| e.start_time == utc(2026, 8, 4, 5, 30)));

        let master = "UID:c@x\nDTSTART:20260729T070000Z\nSTATUS:CANCELLED\nSUMMARY:Gone";
        assert!(expand(&[master], utc(2026, 7, 1, 0, 0), utc(2026, 8, 1, 0, 0)).is_empty());
    }

    #[test]
    fn all_day_recurrence_expands_to_utc_midnight_days() {
        let out = expand(
            &["UID:a@x\nDTSTART;VALUE=DATE:20260801\nRRULE:FREQ=DAILY;COUNT=3\nSUMMARY:Holiday"],
            utc(2026, 7, 27, 0, 0),
            utc(2026, 8, 24, 0, 0),
        );
        let starts: Vec<i64> = out.iter().map(|e| e.start_time).collect();
        assert_eq!(
            starts,
            vec![utc(2026, 8, 1, 0, 0), utc(2026, 8, 2, 0, 0), utc(2026, 8, 3, 0, 0)]
        );
        assert!(out.iter().all(|e| e.is_all_day && e.end_time == e.start_time + 86_400));
        assert_eq!(out[0].provider_event_id, "a@x_20260801");
    }

    #[test]
    fn tzid_rrule_tracks_local_wall_time_across_dst() {
        // Europe/Madrid switches to CEST on 29/03/2026: 10:00 local is 09:00Z
        // before and 08:00Z after.
        let out = expand(
            &["UID:t@x\nDTSTART;TZID=Europe/Madrid:20260324T100000\nDTEND;TZID=Europe/Madrid:20260324T110000\nRRULE:FREQ=WEEKLY\nSUMMARY:DST"],
            utc(2026, 3, 23, 0, 0),
            utc(2026, 4, 6, 0, 0),
        );
        let starts: Vec<i64> = out.iter().map(|e| e.start_time).collect();
        assert_eq!(starts, vec![utc(2026, 3, 24, 9, 0), utc(2026, 3, 31, 8, 0)]);
    }

    #[test]
    fn malformed_rrule_falls_back_to_the_master_only() {
        let out = expand(
            &["UID:b@x\nDTSTART:20260729T070000Z\nDTEND:20260729T080000Z\nRRULE:FREQ=BOGUS\nSUMMARY:Broken"],
            utc(2026, 7, 1, 0, 0),
            utc(2026, 8, 1, 0, 0),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].provider_event_id, "b@x");
        assert_eq!(out[0].start_time, utc(2026, 7, 29, 7, 0));
    }

    #[test]
    fn occurrence_cap_bounds_a_hostile_feed() {
        let out = expand(
            &["UID:m@x\nDTSTART:20260701T000000Z\nRRULE:FREQ=MINUTELY\nSUMMARY:Spam"],
            utc(2026, 7, 1, 0, 0),
            utc(2026, 8, 1, 0, 0),
        );
        assert!(!out.is_empty());
        assert!(out.len() <= usize::from(MAX_OCCURRENCES_PER_EVENT));
    }

    // ── provider over a fake fetcher ─────────────────────────────────────

    fn feeds() -> Vec<FeedSource> {
        vec![
            FeedSource {
                id: "feed-a".into(),
                label: "Work".into(),
                url: "https://cal.example.com/a.ics".into(),
            },
            FeedSource {
                id: "feed-b".into(),
                label: "Home".into(),
                url: "https://cal.example.com/b.ics".into(),
            },
        ]
    }

    #[tokio::test]
    async fn list_calendars_is_one_reader_calendar_per_feed_without_network() {
        let fetcher = Arc::new(FakeIcsFetcher::default()); // no bodies: any fetch would fail
        let provider = IcsFeedCalendarProvider::new(feeds(), fetcher.clone());
        let cals = provider.list_calendars().await.expect("calendars");
        assert_eq!(cals.len(), 2);
        assert_eq!(
            (cals[0].provider_calendar_id.as_str(), cals[0].name.as_str()),
            ("feed-a", "Work")
        );
        assert!(cals[0].is_primary && !cals[1].is_primary);
        assert!(cals
            .iter()
            .all(|c| c.access_role == "reader" && c.selected && c.color.is_empty()));
        assert_eq!(fetcher.fetch_count(), 0);
    }

    #[tokio::test]
    async fn list_events_fetches_the_feed_and_expands_it() {
        let fetcher = Arc::new(FakeIcsFetcher::default().with_body("https://cal.example.com/a.ics", &vcal(&[WEEKLY])));
        let provider = IcsFeedCalendarProvider::new(feeds(), fetcher);
        let events = provider
            .list_events("feed-a", utc(2026, 7, 27, 0, 0), utc(2026, 8, 24, 0, 0))
            .await
            .expect("events");
        assert_eq!(events.len(), 4);
    }

    #[tokio::test]
    async fn a_failing_feed_is_an_error_not_an_empty_calendar() {
        // `Err` matters: the sync's stale sweep only runs for calendars whose
        // fetch succeeded, so an outage must not wipe the feed's events.
        let fetcher = Arc::new(FakeIcsFetcher::default().failing("https://cal.example.com/a.ics"));
        let provider = IcsFeedCalendarProvider::new(feeds(), fetcher);
        let err = provider.list_events("feed-a", 0, 1).await.expect_err("fetch failed");
        assert!(
            !err.to_string().contains("a.ics"),
            "URL must not leak into errors: {err}"
        );
        let unknown = provider.list_events("nope", 0, 1).await.expect_err("unknown feed");
        assert!(matches!(unknown, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn writes_are_rejected_as_read_only() {
        let provider = IcsFeedCalendarProvider::new(feeds(), Arc::new(FakeIcsFetcher::default()));
        let new_event = crate::sync::calendar_provider::NewCalendarEvent {
            title: "x".into(),
            description: String::new(),
            attendees: vec![],
            start_time: 0,
            end_time: 3_600,
            time_zone: "UTC".into(),
            recurrence: crate::sync::calendar_provider::EventRecurrence::None,
            request_meet_link: false,
        };
        assert!(matches!(
            provider.create_event(&new_event).await,
            Err(AppError::ReadOnlyCalendar)
        ));
        assert!(matches!(
            provider.delete_event("feed-a", "e", false, "").await,
            Err(AppError::ReadOnlyCalendar)
        ));
    }

    #[tokio::test]
    async fn from_db_reads_feed_urls_from_the_vault_and_skips_feeds_without_one() {
        // Vault cache + keychain backend are process globals: hold the shared
        // seam lock while touching them (synchronously, so the guard is never
        // held across an await) and install an in-memory keychain.
        let fetcher = Arc::new(FakeIcsFetcher::default().with_body("https://cal.example.com/a.ics", &vcal(&[WEEKLY])));
        let provider = {
            let _guard = crate::services::events::seam_test_lock();
            crate::services::secrets_vault::reset_for_testing();
            let _keychain = crate::services::keychain::install_for_testing();
            let db = crate::db::Database::new_for_testing().expect("db");
            db.seed_test_account_with_provider("acc1", "imap");
            for (id, label) in [("feed-a", "Work"), ("feed-b", "Orphan")] {
                db.insert_calendar_feed(&crate::models::CalendarFeed {
                    id: id.into(),
                    account_id: "acc1".into(),
                    label: label.into(),
                    host: "cal.example.com".into(),
                    created_at: 1,
                })
                .expect("insert");
            }
            crate::services::secrets_vault::set(
                CALENDAR_FEED_KEYRING_SERVICE,
                "feed-a",
                "https://cal.example.com/a.ics",
            )
            .expect("vault");
            IcsFeedCalendarProvider::from_db(&db, "acc1", fetcher).expect("provider")
        };
        let cals = provider.list_calendars().await.expect("calendars");
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0].provider_calendar_id, "feed-a");
        assert_eq!(
            provider
                .list_events("feed-a", utc(2026, 7, 27, 0, 0), utc(2026, 8, 24, 0, 0))
                .await
                .expect("events")
                .len(),
            4
        );
    }
}
