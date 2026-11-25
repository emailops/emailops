//! Pure planner for importing a parsed `.ics` file into the primary calendar
//! of a Google or Outlook account. It decides, per UID, what can be imported
//! and why something cannot; it never touches the network or the database.
//! `AlreadyInCalendar` is set later by the executor.

use std::collections::HashMap;

use chrono::{NaiveDate, NaiveDateTime, TimeZone};
use serde::Serialize;

use super::create::looks_like_email;
use super::ics::{
    param_value, resolve_tzid, skipped_local_time, unescape_text, with_iana_tzid, IcsProperty, IcsZone, ParsedCalendar,
    RawVEvent,
};
use crate::models::error::{AppError, Result};
use crate::models::CalendarAttendee;
use crate::services::logger;
use crate::sync::calendar_provider::{CalendarKind, CalendarProvider};
use crate::sync::outlook_calendar::graph_recurrence_from_rrule;

pub const MAX_ICS_IMPORT_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_IMPORT_SERIES: usize = 500;
/// Overrides plus EXDATEs one series may carry. Each costs provider calls, so
/// an untrusted file must not drive thousands of them.
const MAX_SERIES_EXCEPTIONS: usize = MAX_IMPORT_SERIES;

/// A point in time as the .ics wrote it: a calendar date or a wall time in a zone.
#[derive(Debug, Clone, PartialEq)]
pub enum EventTime {
    AllDay(NaiveDate),
    /// `tz` is an IANA name, or "UTC".
    Timed {
        local: NaiveDateTime,
        tz: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportableEvent {
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub start: EventTime,
    pub end: EventTime,
    pub rrule: Option<String>,
    pub exdates: Vec<EventTime>,
    /// One `RDATE[;params]:value` content line per property, TZID as IANA name.
    pub rdates: Vec<String>,
    pub recurrence_id: Option<EventTime>,
    pub overrides: Vec<ImportableEvent>,
    /// Lowercased organizer address.
    pub organizer: Option<String>,
    pub attendees: Vec<CalendarAttendee>,
    pub sequence: u32,
    /// IANA zone of the file's floating times and of all-day series:
    /// X-WR-TIMEZONE when valid, else the caller's default.
    pub fallback_tz: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnsupportedReason {
    MissingUidOrStart,
    Cancelled,
    /// A rule part Outlook cannot represent exactly, e.g. `FREQ=HOURLY`.
    OutlookRecurrence(String),
    OutlookRdate,
    OutlookOrphanOverride,
    /// More than one RRULE (`"RRULE"`) or any EXRULE (`"EXRULE"`).
    UnsupportedRecurrence(String),
    /// A property whose value cannot be carried faithfully (named).
    InvalidProperty(String),
    /// A TZID that no known zone or document definition resolves.
    UnknownTimezone(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportItemStatus {
    Importable,
    /// Set by the executor, never by the planner.
    AlreadyInCalendar,
    /// Set by the executor when the duplicate check failed (not an auth error).
    LookupFailed(String),
    Unsupported(UnsupportedReason),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportItem {
    pub event: ImportableEvent,
    pub status: ImportItemStatus,
}

impl EventTime {
    /// UTC epoch seconds. All-day dates are UTC midnight. A wall time a DST
    /// change skips is read with the offset before the change; an ambiguous
    /// one takes the earlier instant (same rules as `ics.rs`).
    pub fn to_epoch(&self) -> i64 {
        match self {
            EventTime::AllDay(date) => date.and_time(chrono::NaiveTime::MIN).and_utc().timestamp(),
            EventTime::Timed { local, tz } => match tz.parse::<chrono_tz::Tz>() {
                Ok(zone) => match zone.from_local_datetime(local).earliest() {
                    Some(t) => t.timestamp(),
                    None => skipped_local_time(zone, *local),
                },
                Err(_) => local.and_utc().timestamp(),
            },
        }
    }
}

/// The master of one UID and its overrides, as the file wrote them.
#[derive(Default)]
struct Group {
    master: Option<RawVEvent>,
    overrides: Vec<RawVEvent>,
    /// A second VEVENT with the same UID and no RECURRENCE-ID.
    duplicate_master: bool,
}

/// Why a value could not be read.
enum TimeError {
    Invalid,
    UnknownTzid(String),
}

/// A series the planner refuses, with the VEVENT to show in the preview.
type Rejection<'a> = (UnsupportedReason, &'a RawVEvent);

enum Entry {
    Group(String),
    NoUid(RawVEvent),
}

/// Plan the import of `cal` into `target`. Items follow the document order of
/// the first appearance of each UID.
pub fn plan_ics_import(cal: &ParsedCalendar, target: CalendarKind, default_tz: &str) -> Result<Vec<ImportItem>> {
    if target == CalendarKind::IcsFeeds {
        return Err(AppError::ReadOnlyCalendar);
    }
    if default_tz.parse::<chrono_tz::Tz>().is_err() {
        return Err(AppError::InvalidInput(format!("unknown time zone: {default_tz}")));
    }
    let ctx = Ctx {
        cal,
        fallback_tz: cal
            .timezone
            .as_deref()
            .filter(|tz| tz.parse::<chrono_tz::Tz>().is_ok())
            .unwrap_or(default_tz)
            .to_string(),
    };

    let mut order = Vec::new();
    let mut groups: HashMap<String, Group> = HashMap::new();
    for raw in &cal.events {
        let uid = raw.first_value("UID").map(str::trim).unwrap_or_default();
        if uid.is_empty() {
            order.push(Entry::NoUid(raw.clone()));
            continue;
        }
        let group = groups.entry(uid.to_string()).or_insert_with(|| {
            order.push(Entry::Group(uid.to_string()));
            Group::default()
        });
        if raw.first("RECURRENCE-ID").is_some() {
            group.overrides.push(raw.clone());
        } else if group.master.is_none() {
            group.master = Some(raw.clone());
        } else {
            group.duplicate_master = true;
        }
    }
    if groups.len() > MAX_IMPORT_SERIES {
        return Err(AppError::InvalidInput(format!(
            "too many events in one file: {} (limit {MAX_IMPORT_SERIES})",
            groups.len()
        )));
    }

    Ok(order
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::NoUid(raw) => Some(ctx.rejected(&raw, UnsupportedReason::MissingUidOrStart)),
            Entry::Group(uid) => groups.remove(&uid).map(|group| ctx.plan_group(group, target)),
        })
        .collect())
}

struct Ctx<'a> {
    cal: &'a ParsedCalendar,
    /// X-WR-TIMEZONE when valid, else the caller's default.
    fallback_tz: String,
}

impl Ctx<'_> {
    fn rejected(&self, raw: &RawVEvent, reason: UnsupportedReason) -> ImportItem {
        ImportItem {
            event: self.event(raw).unwrap_or_else(|_| self.placeholder(raw)),
            status: ImportItemStatus::Unsupported(reason),
        }
    }

    fn plan_group(&self, group: Group, target: CalendarKind) -> ImportItem {
        match self.try_group(&group, target) {
            Ok(item) => item,
            Err((reason, raw)) => self.rejected(raw, reason),
        }
    }

    fn try_group<'g>(&self, group: &'g Group, target: CalendarKind) -> std::result::Result<ImportItem, Rejection<'g>> {
        let Group {
            master,
            overrides: raw_overrides,
            ..
        } = group;
        if group.duplicate_master {
            if let Some(master) = master {
                return Err((UnsupportedReason::InvalidProperty("UID".into()), master));
            }
        }
        // An override we cannot carry would be lost silently: refuse the series.
        let preview = |raw: &'g RawVEvent| master.as_ref().unwrap_or(raw);
        let mut kept = Vec::new();
        let mut cancelled = Vec::new();
        for raw in raw_overrides {
            let event = self.event(raw).map_err(|reason| (reason, preview(raw)))?;
            if !is_cancelled(raw) {
                kept.push(event);
            } else if master.is_none() {
                return Err((UnsupportedReason::Cancelled, raw));
            } else if let Some(id) = event.recurrence_id {
                // A cancelled instance of a series is an excluded date.
                cancelled.push(id);
            }
        }
        let Some(raw_master) = master else {
            // Only overrides: the first one stands for the item.
            let mut kept = kept.into_iter();
            let Some(mut event) = kept.next() else {
                return Ok(unreachable_empty());
            };
            event.overrides = kept.collect();
            let status = if target == CalendarKind::Outlook {
                UnsupportedReason::OutlookOrphanOverride
            } else if !event.overrides.is_empty() {
                // Google cannot import several standalone events with one iCalUID.
                UnsupportedReason::UnsupportedRecurrence("RECURRENCE-ID".into())
            } else {
                return Ok(ImportItem {
                    event,
                    status: ImportItemStatus::Importable,
                });
            };
            return Ok(ImportItem {
                event,
                status: ImportItemStatus::Unsupported(status),
            });
        };
        let mut event = self.event(raw_master).map_err(|reason| (reason, raw_master))?;
        // Name the count that crosses the cap: the overrides alone, or the
        // EXDATEs on top of them.
        if raw_overrides.len() > MAX_SERIES_EXCEPTIONS {
            return Err((UnsupportedReason::InvalidProperty("RECURRENCE-ID".into()), raw_master));
        }
        event.overrides = kept;
        event.exdates.extend(cancelled);
        if event.overrides.len() + event.exdates.len() > MAX_SERIES_EXCEPTIONS {
            return Err((UnsupportedReason::InvalidProperty("EXDATE".into()), raw_master));
        }
        let status = self.master_status(raw_master, &event, target);
        Ok(ImportItem { event, status })
    }

    fn master_status(&self, raw: &RawVEvent, event: &ImportableEvent, target: CalendarKind) -> ImportItemStatus {
        use ImportItemStatus::{Importable, Unsupported};
        if is_cancelled(raw) {
            return Unsupported(UnsupportedReason::Cancelled);
        }
        if raw.all("RRULE").count() > 1 {
            return Unsupported(UnsupportedReason::UnsupportedRecurrence("RRULE".into()));
        }
        if raw.first("EXRULE").is_some() {
            return Unsupported(UnsupportedReason::UnsupportedRecurrence("EXRULE".into()));
        }
        let recurring = event.rrule.is_some() || !event.rdates.is_empty();
        if recurring && self.starts_in_offset_zone(raw) {
            // Converted to UTC, BYDAY/BYMONTHDAY would be evaluated on the wrong date.
            return Unsupported(UnsupportedReason::UnsupportedRecurrence("TZID".into()));
        }
        if target == CalendarKind::Outlook {
            if !event.rdates.is_empty() {
                return Unsupported(UnsupportedReason::OutlookRdate);
            }
            if let Some(rrule) = &event.rrule {
                let (first_start, tz) = match &event.start {
                    EventTime::AllDay(d) => (d.and_time(chrono::NaiveTime::MIN), self.fallback_tz.as_str()),
                    EventTime::Timed { local, tz } => (*local, tz.as_str()),
                };
                if let Err(part) = graph_recurrence_from_rrule(rrule, first_start, tz) {
                    return Unsupported(UnsupportedReason::OutlookRecurrence(part));
                }
            }
        }
        Importable
    }

    /// DTSTART names a fixed-offset zone that no IANA zone represents.
    fn starts_in_offset_zone(&self, raw: &RawVEvent) -> bool {
        raw.first("DTSTART")
            .filter(|p| !p.value.trim().ends_with('Z'))
            .and_then(|p| param_value(&p.params, "TZID"))
            .is_some_and(|id| matches!(resolve_tzid(id, &self.cal.fixed_zones), Some(IcsZone::Offset(_))))
    }

    /// Epoch-1970 times for an event without DTSTART, so it still shows in the preview.
    fn placeholder(&self, raw: &RawVEvent) -> ImportableEvent {
        let epoch = EventTime::Timed {
            local: chrono::DateTime::UNIX_EPOCH.naive_utc(),
            tz: "UTC".into(),
        };
        self.build(raw, epoch.clone(), epoch)
    }

    /// The event, or the reason it cannot be carried faithfully.
    fn event(&self, raw: &RawVEvent) -> std::result::Result<ImportableEvent, UnsupportedReason> {
        let prop = raw.first("DTSTART").ok_or(UnsupportedReason::MissingUidOrStart)?;
        let start = self.time(prop, &prop.value).map_err(|e| match e {
            TimeError::Invalid => UnsupportedReason::MissingUidOrStart,
            TimeError::UnknownTzid(tzid) => UnsupportedReason::UnknownTimezone(tzid),
        })?;
        let end = self.end_time(raw, &start)?;
        let mut event = self.build(raw, start, end);
        for prop in raw.all("EXDATE") {
            for value in prop.value.split(',') {
                event.exdates.push(self.time_of(prop, value, "EXDATE")?);
            }
        }
        if let Some(prop) = raw.first("RECURRENCE-ID") {
            // RANGE (THISANDFUTURE) changes every later occurrence too; both
            // providers would change only this one.
            if param_value(&prop.params, "RANGE").is_some() {
                return Err(UnsupportedReason::UnsupportedRecurrence("RANGE".into()));
            }
            event.recurrence_id = Some(self.time_of(prop, &prop.value, "RECURRENCE-ID")?);
        }
        Ok(event)
    }

    /// The fields that cannot fail; EXDATE and RECURRENCE-ID are left to `event`.
    fn build(&self, raw: &RawVEvent, start: EventTime, end: EventTime) -> ImportableEvent {
        let text = |name: &str| {
            raw.first_value(name)
                .map(|v| unescape_text(v.trim()))
                .unwrap_or_default()
        };
        ImportableEvent {
            uid: raw.first_value("UID").map(|u| u.trim().to_string()).unwrap_or_default(),
            summary: text("SUMMARY"),
            description: text("DESCRIPTION"),
            location: text("LOCATION"),
            start,
            end,
            rrule: raw.first_value("RRULE").map(|r| r.trim().to_string()),
            exdates: Vec::new(),
            rdates: raw.all("RDATE").map(|p| self.recurrence_line(p)).collect(),
            recurrence_id: None,
            overrides: Vec::new(),
            organizer: raw.first("ORGANIZER").and_then(calendar_user_address),
            attendees: raw
                .all("ATTENDEE")
                .filter_map(|p| {
                    Some(CalendarAttendee {
                        email: calendar_user_address(p)?,
                        response: attendee_response(&p.params),
                    })
                })
                .collect(),
            sequence: raw
                .first_value("SEQUENCE")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0),
            fallback_tz: self.fallback_tz.clone(),
        }
    }

    /// `NAME[;params]:value` with any TZID rewritten to its IANA name, the form
    /// Google's `recurrence` array takes.
    fn recurrence_line(&self, prop: &IcsProperty) -> String {
        let mut line = prop.name.clone();
        for param in &prop.params {
            line.push(';');
            line.push_str(&with_iana_tzid(param, &self.cal.fixed_zones));
        }
        format!("{line}:{}", prop.value.trim())
    }

    /// DTEND, else DURATION, else RFC 5545 §3.6.1: one day for all-day events,
    /// zero length for timed ones. A present but unreadable value is an error.
    fn end_time(&self, raw: &RawVEvent, start: &EventTime) -> std::result::Result<EventTime, UnsupportedReason> {
        if let Some(prop) = raw.first("DTEND") {
            return self.time_of(prop, &prop.value, "DTEND");
        }
        let secs = match raw.first_value("DURATION") {
            Some(v) => {
                Some(parse_iso_duration(v).ok_or_else(|| UnsupportedReason::InvalidProperty("DURATION".into()))?)
            }
            None => None,
        };
        Ok(match start {
            EventTime::AllDay(d) => EventTime::AllDay(*d + chrono::Duration::days(secs.map_or(1, |s| s / 86_400))),
            EventTime::Timed { local, tz } => EventTime::Timed {
                local: *local + chrono::Duration::seconds(secs.unwrap_or(0)),
                tz: tz.clone(),
            },
        })
    }

    /// `time`, with its failure named after the property.
    fn time_of(
        &self,
        prop: &IcsProperty,
        value: &str,
        name: &str,
    ) -> std::result::Result<EventTime, UnsupportedReason> {
        self.time(prop, value).map_err(|e| match e {
            TimeError::Invalid => UnsupportedReason::InvalidProperty(name.into()),
            TimeError::UnknownTzid(tzid) => UnsupportedReason::UnknownTimezone(tzid),
        })
    }

    /// Reads `value` with the params of `prop`. Zone order: `Z` suffix, the
    /// TZID, then (floating times only) the document's X-WR-TIMEZONE and the
    /// caller's default. A TZID nobody can resolve is an error.
    fn time(&self, prop: &IcsProperty, value: &str) -> std::result::Result<EventTime, TimeError> {
        let value = value.trim();
        if param_value(&prop.params, "VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE"))
            || (value.len() == 8 && !value.contains('T'))
        {
            return NaiveDate::parse_from_str(value, "%Y%m%d")
                .map(EventTime::AllDay)
                .map_err(|_| TimeError::Invalid);
        }
        if let Some(utc) = value.strip_suffix('Z') {
            let local = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").map_err(|_| TimeError::Invalid)?;
            return Ok(EventTime::Timed {
                local,
                tz: "UTC".into(),
            });
        }
        let local = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").map_err(|_| TimeError::Invalid)?;
        let Some(tzid) = param_value(&prop.params, "TZID") else {
            return Ok(EventTime::Timed {
                local,
                tz: self.fallback_tz.clone(),
            });
        };
        Ok(match resolve_tzid(tzid, &self.cal.fixed_zones) {
            Some(IcsZone::Named(tz)) => EventTime::Timed {
                local,
                tz: tz.name().to_string(),
            },
            Some(IcsZone::Offset(offset)) => EventTime::Timed {
                local: local - chrono::Duration::seconds(i64::from(offset.local_minus_utc())),
                tz: "UTC".into(),
            },
            None => return Err(TimeError::UnknownTzid(tzid.to_string())),
        })
    }
}

fn is_cancelled(raw: &RawVEvent) -> bool {
    raw.first_value("STATUS")
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("CANCELLED"))
}

/// A group is only created together with its first event, so it is never empty.
fn unreachable_empty() -> ImportItem {
    let day = EventTime::AllDay(NaiveDate::MIN);
    ImportItem {
        event: ImportableEvent {
            uid: String::new(),
            summary: String::new(),
            description: String::new(),
            location: String::new(),
            start: day.clone(),
            end: day,
            rrule: None,
            exdates: Vec::new(),
            rdates: Vec::new(),
            recurrence_id: None,
            overrides: Vec::new(),
            organizer: None,
            attendees: Vec::new(),
            sequence: 0,
            fallback_tz: "UTC".into(),
        },
        status: ImportItemStatus::Unsupported(UnsupportedReason::MissingUidOrStart),
    }
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
                total += n * match (unit, in_time) {
                    ('W', false) => 7 * 86_400,
                    ('D', false) => 86_400,
                    ('H', true) => 3_600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
            }
        }
    }
    if !number.is_empty() {
        return None;
    }
    Some(if negative { -total } else { total })
}

/// The lowercased email address of an ORGANIZER or ATTENDEE: its `mailto:`
/// value, else its EMAIL parameter (RFC 7986). `None` when that is not an
/// email address, e.g. an `invalid:nomail` room or a bare `urn:uuid:`.
fn calendar_user_address(prop: &IcsProperty) -> Option<String> {
    let value = prop.value.trim();
    let address = match value.get(..7) {
        Some(prefix) if prefix.eq_ignore_ascii_case("mailto:") => &value[7..],
        _ => param_value(&prop.params, "EMAIL")?,
    };
    let address = address.trim().to_ascii_lowercase();
    looks_like_email(&address).then_some(address)
}

fn attendee_response(params: &[String]) -> String {
    match param_value(params, "PARTSTAT")
        .map(|p| p.trim_matches('"').to_ascii_uppercase())
        .as_deref()
    {
        Some("ACCEPTED") => "accepted",
        Some("DECLINED") => "declined",
        Some("TENTATIVE") => "tentative",
        _ => "needsAction",
    }
    .to_string()
}

fn read_error(e: std::io::Error) -> AppError {
    AppError::IoError(format!("could not read the file: {}", e.kind()))
}

/// Read an `.ics` file the user picked. The extension and size are checked
/// before reading; a leading UTF-8 BOM is dropped.
pub fn read_ics_file(path: &std::path::Path) -> Result<String> {
    let is_ics = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ics"));
    if !is_ics {
        return Err(AppError::InvalidInput("the file is not an .ics file".into()));
    }
    let size = std::fs::metadata(path).map_err(read_error)?.len();
    if size > MAX_ICS_IMPORT_BYTES as u64 {
        return Err(AppError::InvalidInput(format!(
            "the file is too large (limit {} MB)",
            MAX_ICS_IMPORT_BYTES / (1024 * 1024)
        )));
    }
    let bytes = std::fs::read(path).map_err(read_error)?;
    let text = String::from_utf8(bytes).map_err(|_| AppError::InvalidInput("the file is not UTF-8 text".into()))?;
    Ok(match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_string(),
        None => text,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IcsImportPreviewItem {
    pub uid: String,
    pub title: String,
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: bool,
    pub recurrence: Option<String>,
    /// `importable` | `alreadyInCalendar` | `unsupported`.
    pub status: String,
    /// Stable code of the unsupported reason.
    pub reason: Option<String>,
    pub reason_detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IcsImportPreview {
    pub file_name: String,
    pub items: Vec<IcsImportPreviewItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IcsImportFailure {
    pub title: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IcsImportSummary {
    pub imported: u32,
    pub already_present: u32,
    pub unsupported: u32,
    pub failed: Vec<IcsImportFailure>,
}

/// The wire code of an unsupported reason and its payload, when it has one.
pub(crate) fn reason_code(reason: &UnsupportedReason) -> (&'static str, Option<String>) {
    match reason {
        UnsupportedReason::MissingUidOrStart => ("missingUidOrStart", None),
        UnsupportedReason::Cancelled => ("cancelled", None),
        UnsupportedReason::OutlookRecurrence(d) => ("outlookRecurrence", Some(d.clone())),
        UnsupportedReason::OutlookRdate => ("outlookRdate", None),
        UnsupportedReason::OutlookOrphanOverride => ("outlookOrphanOverride", None),
        UnsupportedReason::UnsupportedRecurrence(d) => ("unsupportedRecurrence", Some(d.clone())),
        UnsupportedReason::InvalidProperty(d) => ("invalidProperty", Some(d.clone())),
        UnsupportedReason::UnknownTimezone(d) => ("unknownTimezone", Some(d.clone())),
    }
}

/// Errors that mean "sign in again": they abort the whole run.
pub(crate) fn is_auth_error(e: &AppError) -> bool {
    matches!(
        e,
        AppError::NeedsReauth { .. } | AppError::CalendarPermissionDenied { .. }
    )
}

fn plan_text(ics: &str, kind: CalendarKind, default_tz: &str) -> Result<Vec<ImportItem>> {
    plan_ics_import(&super::ics::parse_vcalendar(ics), kind, default_tz)
}

/// Mark the importable items the calendar already holds. An auth error aborts;
/// any other lookup error is logged (code only) and marks the item
/// `LookupFailed`, so it is never imported blind.
async fn mark_present(provider: &dyn CalendarProvider, items: &mut [ImportItem]) -> Result<()> {
    for item in items.iter_mut().filter(|i| i.status == ImportItemStatus::Importable) {
        match provider.find_event_by_uid(&item.event.uid).await {
            Ok(Some(_)) => item.status = ImportItemStatus::AlreadyInCalendar,
            Ok(None) => {}
            Err(e) if is_auth_error(&e) => return Err(e),
            Err(e) => {
                logger::log(
                    "error",
                    "sync",
                    format!("Calendar duplicate check failed: {}", e.code()),
                );
                item.status = ImportItemStatus::LookupFailed(e.to_string());
            }
        }
    }
    Ok(())
}

/// Read the file at `path` and preview its import.
pub async fn preview_ics_file(
    provider: &dyn CalendarProvider,
    kind: CalendarKind,
    path: &std::path::Path,
    default_tz: &str,
) -> Result<IcsImportPreview> {
    let ics = read_ics_file(path)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    preview_ics_import(provider, kind, &ics, &file_name, default_tz).await
}

pub async fn preview_ics_import(
    provider: &dyn CalendarProvider,
    kind: CalendarKind,
    ics: &str,
    file_name: &str,
    default_tz: &str,
) -> Result<IcsImportPreview> {
    let mut items = plan_text(ics, kind, default_tz)?;
    mark_present(provider, &mut items).await?;
    let items = items
        .into_iter()
        .map(|item| {
            let (status, reason, reason_detail) = match &item.status {
                ImportItemStatus::Importable => ("importable", None, None),
                ImportItemStatus::AlreadyInCalendar => ("alreadyInCalendar", None, None),
                ImportItemStatus::LookupFailed(d) => ("unsupported", Some("lookupFailed".to_string()), Some(d.clone())),
                ImportItemStatus::Unsupported(r) => {
                    let (code, detail) = reason_code(r);
                    ("unsupported", Some(code.to_string()), detail)
                }
            };
            let e = item.event;
            IcsImportPreviewItem {
                uid: e.uid,
                title: e.summary,
                start_time: e.start.to_epoch(),
                end_time: e.end.to_epoch(),
                is_all_day: matches!(e.start, EventTime::AllDay(_)),
                recurrence: e.rrule,
                status: status.to_string(),
                reason,
                reason_detail,
            }
        })
        .collect();
    Ok(IcsImportPreview {
        file_name: file_name.to_string(),
        items,
    })
}

pub async fn import_ics(
    provider: &dyn CalendarProvider,
    kind: CalendarKind,
    ics: &str,
    default_tz: &str,
) -> Result<IcsImportSummary> {
    let result = run_import(provider, kind, ics, default_tz).await;
    match &result {
        Ok(s) => logger::log(
            "success",
            "sync",
            format!(
                "Calendar import finished: {} imported, {} already present, {} unsupported, {} failed",
                s.imported,
                s.already_present,
                s.unsupported,
                s.failed.len()
            ),
        ),
        Err(e) => logger::log("error", "sync", format!("Calendar import failed: {}", e.code())),
    }
    result
}

async fn run_import(
    provider: &dyn CalendarProvider,
    kind: CalendarKind,
    ics: &str,
    default_tz: &str,
) -> Result<IcsImportSummary> {
    let mut items = plan_text(ics, kind, default_tz)?;
    logger::log("info", "sync", format!("Importing {} calendar events", items.len()));
    mark_present(provider, &mut items).await?;
    let mut summary = IcsImportSummary {
        imported: 0,
        already_present: 0,
        unsupported: 0,
        failed: Vec::new(),
    };
    for item in &items {
        match &item.status {
            ImportItemStatus::AlreadyInCalendar => summary.already_present += 1,
            ImportItemStatus::Unsupported(_) => summary.unsupported += 1,
            ImportItemStatus::LookupFailed(reason) => summary.failed.push(IcsImportFailure {
                title: item.event.summary.clone(),
                reason: reason.clone(),
            }),
            ImportItemStatus::Importable => match provider.import_event(&item.event).await {
                Ok(_) => summary.imported += 1,
                Err(e) if is_auth_error(&e) => return Err(e),
                Err(e) => summary.failed.push(IcsImportFailure {
                    title: item.event.summary.clone(),
                    reason: e.to_string(),
                }),
            },
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::calendar::ics::parse_vcalendar;

    fn cal(vevents: &[&str]) -> ParsedCalendar {
        cal_with("", vevents)
    }

    fn cal_with(header: &str, vevents: &[&str]) -> ParsedCalendar {
        let mut ics = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{header}");
        for body in vevents {
            ics.push_str("BEGIN:VEVENT\r\n");
            for line in body.lines() {
                ics.push_str(line);
                ics.push_str("\r\n");
            }
            ics.push_str("END:VEVENT\r\n");
        }
        ics.push_str("END:VCALENDAR\r\n");
        parse_vcalendar(&ics)
    }

    fn plan(vevents: &[&str], target: CalendarKind) -> Vec<ImportItem> {
        plan_ics_import(&cal(vevents), target, "Europe/Madrid").unwrap()
    }

    fn one(body: &str, target: CalendarKind) -> ImportItem {
        let mut items = plan(&[body], target);
        assert_eq!(items.len(), 1, "expected one item");
        items.remove(0)
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    fn timed(local: NaiveDateTime, tz: &str) -> EventTime {
        EventTime::Timed { local, tz: tz.into() }
    }

    #[test]
    fn single_timed_event_is_importable() {
        let item = one(
            "UID:a@example.com\nDTSTART;TZID=Europe/Madrid:20261205T083000\nSUMMARY:Planning",
            CalendarKind::Google,
        );
        assert_eq!(item.event.start, timed(at(2026, 12, 5, 8, 30), "Europe/Madrid"));
        assert_eq!(item.event.summary, "Planning");
        assert_eq!(item.status, ImportItemStatus::Importable);
    }

    #[test]
    fn windows_tzid_is_resolved_to_iana() {
        let item = one(
            "UID:a@example.com\nDTSTART;TZID=Romance Standard Time:20261205T083000",
            CalendarKind::Google,
        );
        assert_eq!(item.event.start, timed(at(2026, 12, 5, 8, 30), "Europe/Paris"));
    }

    #[test]
    fn utc_time_keeps_utc() {
        let item = one("UID:a@example.com\nDTSTART:20261205T083000Z", CalendarKind::Google);
        assert_eq!(item.event.start, timed(at(2026, 12, 5, 8, 30), "UTC"));
    }

    #[test]
    fn floating_time_uses_default_tz() {
        let item = one("UID:a@example.com\nDTSTART:20261205T083000", CalendarKind::Google);
        assert_eq!(item.event.start, timed(at(2026, 12, 5, 8, 30), "Europe/Madrid"));
    }

    #[test]
    fn floating_time_prefers_document_timezone() {
        let c = cal_with(
            "X-WR-TIMEZONE:America/New_York\r\n",
            &["UID:a@example.com\nDTSTART:20261205T083000"],
        );
        let items = plan_ics_import(&c, CalendarKind::Google, "Europe/Madrid").unwrap();
        assert_eq!(items[0].event.start, timed(at(2026, 12, 5, 8, 30), "America/New_York"));
    }

    #[test]
    fn events_carry_the_fallback_zone() {
        let c = cal_with(
            "X-WR-TIMEZONE:America/New_York\r\n",
            &["UID:a@example.com\nDTSTART;VALUE=DATE:20261205\nRRULE:FREQ=DAILY"],
        );
        let items = plan_ics_import(&c, CalendarKind::Outlook, "Europe/Madrid").unwrap();
        assert_eq!(items[0].event.fallback_tz, "America/New_York");
        let item = one("UID:a@example.com\nDTSTART;VALUE=DATE:20261205", CalendarKind::Outlook);
        assert_eq!(item.event.fallback_tz, "Europe/Madrid");
    }

    #[test]
    fn non_whole_hour_offset_zone_becomes_utc() {
        let c = cal_with(
            "BEGIN:VTIMEZONE\r\nTZID:Example Half Hour\r\nBEGIN:STANDARD\r\nDTSTART:16010101T000000\r\nTZOFFSETFROM:+0530\r\nTZOFFSETTO:+0530\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n",
            &["UID:a@example.com\nDTSTART;TZID=Example Half Hour:20261205T083000"],
        );
        let items = plan_ics_import(&c, CalendarKind::Google, "Europe/Madrid").unwrap();
        assert_eq!(items[0].event.start, timed(at(2026, 12, 5, 3, 0), "UTC"));
    }

    #[test]
    fn all_day_without_dtend_lasts_one_day() {
        let item = one("UID:a@example.com\nDTSTART;VALUE=DATE:20261205", CalendarKind::Google);
        assert_eq!(
            item.event.end,
            EventTime::AllDay(NaiveDate::from_ymd_opt(2026, 12, 6).unwrap())
        );
    }

    #[test]
    fn duration_sets_the_end() {
        let timed_item = one(
            "UID:a@example.com\nDTSTART:20261205T083000Z\nDURATION:PT1H30M",
            CalendarKind::Google,
        );
        assert_eq!(timed_item.event.end, timed(at(2026, 12, 5, 10, 0), "UTC"));
        let all_day = one(
            "UID:b@example.com\nDTSTART;VALUE=DATE:20261205\nDURATION:P2D",
            CalendarKind::Google,
        );
        assert_eq!(
            all_day.event.end,
            EventTime::AllDay(NaiveDate::from_ymd_opt(2026, 12, 7).unwrap())
        );
    }

    #[test]
    fn timed_without_end_or_duration_has_zero_length() {
        let item = one("UID:a@example.com\nDTSTART:20261205T083000Z", CalendarKind::Google);
        assert_eq!(item.event.end, item.event.start);
    }

    #[test]
    fn overrides_attach_to_their_master() {
        let items = plan(
            &[
                "UID:a@example.com\nDTSTART:20261205T083000Z\nRRULE:FREQ=WEEKLY",
                "UID:a@example.com\nRECURRENCE-ID:20261212T083000Z\nDTSTART:20261212T090000Z",
            ],
            CalendarKind::Google,
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].event.overrides.len(), 1);
        assert_eq!(
            items[0].event.overrides[0].recurrence_id,
            Some(timed(at(2026, 12, 12, 8, 30), "UTC"))
        );
    }

    #[test]
    fn orphan_override_is_supported_on_google_only() {
        let body = "UID:a@example.com\nRECURRENCE-ID:20261212T083000Z\nDTSTART:20261212T090000Z";
        assert_eq!(one(body, CalendarKind::Google).status, ImportItemStatus::Importable);
        assert_eq!(
            one(body, CalendarKind::Outlook).status,
            ImportItemStatus::Unsupported(UnsupportedReason::OutlookOrphanOverride)
        );
    }

    #[test]
    fn missing_uid_or_start_is_unsupported() {
        for body in [
            "DTSTART:20261205T083000Z\nSUMMARY:No uid",
            "UID:a@example.com\nSUMMARY:No start",
        ] {
            let item = one(body, CalendarKind::Google);
            assert_eq!(
                item.status,
                ImportItemStatus::Unsupported(UnsupportedReason::MissingUidOrStart)
            );
            assert!(item.event.summary.starts_with("No "));
            assert_eq!(item.event.end, item.event.start);
        }
        let item = one("UID:a@example.com\nSUMMARY:No start", CalendarKind::Google);
        assert_eq!(item.event.start, timed(at(1970, 1, 1, 0, 0), "UTC"));
    }

    #[test]
    fn cancelled_master_is_not_imported() {
        let item = one(
            "UID:a@example.com\nDTSTART:20261205T083000Z\nSTATUS:CANCELLED",
            CalendarKind::Google,
        );
        assert_eq!(item.status, ImportItemStatus::Unsupported(UnsupportedReason::Cancelled));
    }

    #[test]
    fn outlook_rejects_untranslatable_rules() {
        let body = "UID:a@example.com\nDTSTART:20261205T083000Z\nRRULE:FREQ=HOURLY";
        assert_eq!(
            one(body, CalendarKind::Outlook).status,
            ImportItemStatus::Unsupported(UnsupportedReason::OutlookRecurrence("FREQ=HOURLY".into()))
        );
        let google = one(body, CalendarKind::Google);
        assert_eq!(google.status, ImportItemStatus::Importable);
        assert_eq!(google.event.rrule.as_deref(), Some("FREQ=HOURLY"));
    }

    #[test]
    fn outlook_accepts_translatable_rules() {
        let item = one(
            "UID:a@example.com\nDTSTART:20261205T083000Z\nRRULE:FREQ=WEEKLY;COUNT=4",
            CalendarKind::Outlook,
        );
        assert_eq!(item.status, ImportItemStatus::Importable);
    }

    #[test]
    fn outlook_rejects_rdate() {
        let body = "UID:a@example.com\nDTSTART:20261205T083000Z\nRDATE:20261210T083000Z";
        assert_eq!(
            one(body, CalendarKind::Outlook).status,
            ImportItemStatus::Unsupported(UnsupportedReason::OutlookRdate)
        );
        assert_eq!(one(body, CalendarKind::Google).event.rdates.len(), 1);
    }

    #[test]
    fn multiple_rrules_are_unsupported_on_both_targets() {
        let body = "UID:a@example.com\nDTSTART:20261205T083000Z\nRRULE:FREQ=WEEKLY\nRRULE:FREQ=MONTHLY";
        for target in [CalendarKind::Google, CalendarKind::Outlook] {
            assert_eq!(
                one(body, target).status,
                ImportItemStatus::Unsupported(UnsupportedReason::UnsupportedRecurrence("RRULE".into()))
            );
        }
    }

    #[test]
    fn exrule_is_unsupported_on_both_targets() {
        let body = "UID:a@example.com\nDTSTART:20261205T083000Z\nRRULE:FREQ=WEEKLY\nEXRULE:FREQ=MONTHLY";
        for target in [CalendarKind::Google, CalendarKind::Outlook] {
            assert_eq!(
                one(body, target).status,
                ImportItemStatus::Unsupported(UnsupportedReason::UnsupportedRecurrence("EXRULE".into()))
            );
        }
    }

    #[test]
    fn exdates_are_collected() {
        let item = one(
            "UID:a@example.com\nDTSTART:20261205T083000Z\nRRULE:FREQ=WEEKLY\nEXDATE:20261212T083000Z,20261219T083000Z\nEXDATE:20261226T083000Z",
            CalendarKind::Google,
        );
        assert_eq!(item.event.exdates.len(), 3);
    }

    #[test]
    fn organizer_and_attendees_are_parsed() {
        let item = one(
            "UID:a@example.com\nDTSTART:20261205T083000Z\nORGANIZER:mailto:Lead@Example.com\nATTENDEE;PARTSTAT=ACCEPTED:mailto:ana@example.org\nATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:bo@example.org\nATTENDEE:mailto:cy@example.org",
            CalendarKind::Google,
        );
        assert_eq!(item.event.organizer.as_deref(), Some("lead@example.com"));
        let got: Vec<(&str, &str)> = item
            .event
            .attendees
            .iter()
            .map(|a| (a.email.as_str(), a.response.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("ana@example.org", "accepted"),
                ("bo@example.org", "needsAction"),
                ("cy@example.org", "needsAction")
            ]
        );
    }

    #[test]
    fn attendees_without_an_email_address_are_dropped() {
        let item = one(
            "UID:a@example.com\nDTSTART:20261205T083000Z\n\
             ATTENDEE;CN=Room 4:invalid:nomail\n\
             ATTENDEE;EMAIL=Cy@Example.org;PARTSTAT=ACCEPTED:urn:uuid:00000000-0000-0000-0000-000000000001\n\
             ATTENDEE;CN=No mailto:urn:uuid:00000000-0000-0000-0000-000000000002\n\
             ATTENDEE:mailto:bo at example.org\n\
             ATTENDEE:mailto:nobody\n\
             ATTENDEE:mailto:ana@example.org",
            CalendarKind::Google,
        );
        let got: Vec<(&str, &str)> = item
            .event
            .attendees
            .iter()
            .map(|a| (a.email.as_str(), a.response.as_str()))
            .collect();
        assert_eq!(
            got,
            [("cy@example.org", "accepted"), ("ana@example.org", "needsAction")]
        );
    }

    #[test]
    fn an_organizer_without_an_email_address_is_none() {
        for (line, want) in [
            ("ORGANIZER;CN=Lead:invalid:nomail", None),
            (
                "ORGANIZER;EMAIL=Lead@Example.org:urn:uuid:00000000-0000-0000-0000-000000000003",
                Some("lead@example.org"),
            ),
            ("ORGANIZER:mailto:lead@intranet", None),
            ("ORGANIZER:MAILTO:Lead@Example.org", Some("lead@example.org")),
        ] {
            let item = one(
                &format!("UID:a@example.com\nDTSTART:20261205T083000Z\n{line}"),
                CalendarKind::Google,
            );
            assert_eq!(item.event.organizer.as_deref(), want, "{line}");
        }
    }

    #[test]
    fn a_series_with_more_than_500_exceptions_is_rejected() {
        const MASTER: &str = "UID:a@example.com\nDTSTART:20261205T083000Z\nRRULE:FREQ=DAILY";
        const OVERRIDE: &str = "UID:a@example.com\nRECURRENCE-ID:20261212T083000Z\nDTSTART:20261212T090000Z";
        let series = |overrides: usize, exdates: usize| {
            let master = format!("{MASTER}{}", "\nEXDATE:20261219T083000Z".repeat(exdates));
            let mut bodies = vec![master];
            bodies.extend(std::iter::repeat_n(OVERRIDE.to_string(), overrides));
            let refs: Vec<&str> = bodies.iter().map(String::as_str).collect();
            status_of(&refs, CalendarKind::Google)
        };
        assert_eq!(series(0, 500), ImportItemStatus::Importable);
        assert_eq!(series(250, 250), ImportItemStatus::Importable);
        assert_eq!(series(0, 501), invalid("EXDATE"));
        assert_eq!(series(501, 0), invalid("RECURRENCE-ID"));
        assert_eq!(series(300, 201), invalid("EXDATE"));
    }

    #[test]
    fn a_this_and_future_override_is_unsupported() {
        let bodies = [
            "UID:a@example.com\nDTSTART:20261205T083000Z\nRRULE:FREQ=WEEKLY",
            "UID:a@example.com\nRECURRENCE-ID;RANGE=THISANDFUTURE:20261212T083000Z\nDTSTART:20261212T090000Z",
        ];
        for target in [CalendarKind::Google, CalendarKind::Outlook] {
            assert_eq!(
                status_of(&bodies, target),
                ImportItemStatus::Unsupported(UnsupportedReason::UnsupportedRecurrence("RANGE".into()))
            );
        }
    }

    #[test]
    fn sequence_defaults_to_zero() {
        let item = one("UID:a@example.com\nDTSTART:20261205T083000Z", CalendarKind::Google);
        assert_eq!(item.event.sequence, 0);
        let item = one(
            "UID:a@example.com\nDTSTART:20261205T083000Z\nSEQUENCE:3",
            CalendarKind::Google,
        );
        assert_eq!(item.event.sequence, 3);
    }

    #[test]
    fn more_than_500_series_is_rejected() {
        let bodies: Vec<String> = (0..501)
            .map(|i| format!("UID:u{i}@example.com\nDTSTART:20261205T083000Z"))
            .collect();
        let refs: Vec<&str> = bodies.iter().map(String::as_str).collect();
        let err = plan_ics_import(&cal(&refs), CalendarKind::Google, "Europe/Madrid").unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[test]
    fn valarm_properties_do_not_leak() {
        let item = one(
            "UID:a@example.com\nDTSTART:20261205T083000Z\nSUMMARY:Real\nBEGIN:VALARM\nACTION:DISPLAY\nSUMMARY:Alarm text\nEND:VALARM",
            CalendarKind::Google,
        );
        assert_eq!(item.event.summary, "Real");
    }

    #[test]
    fn items_follow_first_appearance_of_each_uid() {
        let items = plan(
            &[
                "UID:b@example.com\nDTSTART:20261205T083000Z",
                "UID:a@example.com\nDTSTART:20261205T083000Z",
                "UID:b@example.com\nRECURRENCE-ID:20261212T083000Z\nDTSTART:20261212T083000Z",
            ],
            CalendarKind::Google,
        );
        let uids: Vec<&str> = items.iter().map(|i| i.event.uid.as_str()).collect();
        assert_eq!(uids, ["b@example.com", "a@example.com"]);
    }

    #[test]
    fn ics_feeds_target_is_read_only() {
        let err = plan_ics_import(&cal(&[]), CalendarKind::IcsFeeds, "Europe/Madrid").unwrap_err();
        assert!(matches!(err, AppError::ReadOnlyCalendar));
    }

    #[test]
    fn unknown_default_tz_is_invalid_input() {
        let err = plan_ics_import(&cal(&[]), CalendarKind::Google, "Mars/Olympus").unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)));
    }

    #[test]
    fn to_epoch_all_day_is_utc_midnight() {
        let t = EventTime::AllDay(NaiveDate::from_ymd_opt(2026, 12, 5).unwrap());
        assert_eq!(t.to_epoch(), at(2026, 12, 5, 0, 0).and_utc().timestamp());
    }

    #[test]
    fn to_epoch_timed_uses_the_zone() {
        let t = timed(at(2026, 12, 5, 8, 30), "Europe/Madrid");
        assert_eq!(t.to_epoch(), at(2026, 12, 5, 7, 30).and_utc().timestamp());
        assert_eq!(
            timed(at(2026, 12, 5, 8, 30), "UTC").to_epoch(),
            at(2026, 12, 5, 8, 30).and_utc().timestamp()
        );
    }

    #[test]
    fn to_epoch_skipped_local_time_uses_offset_before_the_change() {
        // 2026-03-29 02:30 does not exist in Madrid; the offset before is +01:00.
        let t = timed(at(2026, 3, 29, 2, 30), "Europe/Madrid");
        assert_eq!(t.to_epoch(), at(2026, 3, 29, 1, 30).and_utc().timestamp());
    }

    #[test]
    fn to_epoch_ambiguous_local_time_takes_the_earliest() {
        // 2026-10-25 02:30 happens twice in Madrid; the first is +02:00.
        let t = timed(at(2026, 10, 25, 2, 30), "Europe/Madrid");
        assert_eq!(t.to_epoch(), at(2026, 10, 25, 0, 30).and_utc().timestamp());
    }

    #[test]
    fn rdate_keeps_params_with_iana_tzid() {
        let item = one(
            "UID:a@example.com\nDTSTART:20261205T083000Z\nRDATE;TZID=Romance Standard Time:20261210T100000\nRDATE;VALUE=DATE:20261224",
            CalendarKind::Google,
        );
        assert_eq!(
            item.event.rdates,
            ["RDATE;TZID=Europe/Paris:20261210T100000", "RDATE;VALUE=DATE:20261224"]
        );
    }

    #[test]
    fn override_without_start_makes_the_series_unsupported() {
        let items = plan(
            &[
                "UID:a@example.com\nDTSTART:20261205T083000Z\nRRULE:FREQ=WEEKLY",
                "UID:a@example.com\nRECURRENCE-ID:20261212T083000Z\nSUMMARY:No start",
            ],
            CalendarKind::Google,
        );
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].status,
            ImportItemStatus::Unsupported(UnsupportedReason::MissingUidOrStart)
        );
    }

    #[test]
    fn several_orphan_overrides_are_unsupported() {
        let bodies = [
            "UID:a@example.com\nRECURRENCE-ID:20261212T083000Z\nDTSTART:20261212T090000Z",
            "UID:a@example.com\nRECURRENCE-ID:20261219T083000Z\nDTSTART:20261219T090000Z",
        ];
        let google = plan(&bodies, CalendarKind::Google);
        assert_eq!(google.len(), 1);
        assert_eq!(
            google[0].status,
            ImportItemStatus::Unsupported(UnsupportedReason::UnsupportedRecurrence("RECURRENCE-ID".into()))
        );
        let outlook = plan(&bodies, CalendarKind::Outlook);
        assert_eq!(
            outlook[0].status,
            ImportItemStatus::Unsupported(UnsupportedReason::OutlookOrphanOverride)
        );
    }

    fn status_of(bodies: &[&str], target: CalendarKind) -> ImportItemStatus {
        let items = plan(bodies, target);
        assert_eq!(items.len(), 1);
        items[0].status.clone()
    }

    fn invalid(name: &str) -> ImportItemStatus {
        ImportItemStatus::Unsupported(UnsupportedReason::InvalidProperty(name.into()))
    }

    #[test]
    fn duplicate_masters_are_rejected() {
        let status = status_of(
            &[
                "UID:a@example.com\nDTSTART:20261205T083000Z",
                "UID:a@example.com\nDTSTART:20261206T083000Z",
            ],
            CalendarKind::Google,
        );
        assert_eq!(status, invalid("UID"));
    }

    #[test]
    fn unparseable_exdate_is_rejected() {
        let body = "UID:a@example.com\nDTSTART:20261205T083000Z\nEXDATE:20261212T083000Z,garbage";
        assert_eq!(status_of(&[body], CalendarKind::Google), invalid("EXDATE"));
    }

    #[test]
    fn unparseable_recurrence_id_is_rejected() {
        let body = "UID:a@example.com\nRECURRENCE-ID:garbage\nDTSTART:20261212T090000Z";
        assert_eq!(status_of(&[body], CalendarKind::Google), invalid("RECURRENCE-ID"));
    }

    #[test]
    fn unparseable_dtend_is_rejected() {
        let body = "UID:a@example.com\nDTSTART:20261205T083000Z\nDTEND:garbage";
        assert_eq!(status_of(&[body], CalendarKind::Google), invalid("DTEND"));
    }

    #[test]
    fn unparseable_duration_is_rejected() {
        let body = "UID:a@example.com\nDTSTART:20261205T083000Z\nDURATION:garbage";
        assert_eq!(status_of(&[body], CalendarKind::Google), invalid("DURATION"));
    }

    #[test]
    fn cancelled_override_becomes_an_exdate_of_its_master() {
        let items = plan(
            &[
                "UID:a@example.com\nDTSTART:20261205T083000Z\nRRULE:FREQ=WEEKLY",
                "UID:a@example.com\nRECURRENCE-ID:20261212T083000Z\nDTSTART:20261212T083000Z\nSTATUS:CANCELLED",
            ],
            CalendarKind::Google,
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, ImportItemStatus::Importable);
        assert!(items[0].event.overrides.is_empty());
        assert_eq!(items[0].event.exdates, [timed(at(2026, 12, 12, 8, 30), "UTC")]);
    }

    #[test]
    fn cancelled_orphan_override_is_cancelled() {
        let body = "UID:a@example.com\nRECURRENCE-ID:20261212T083000Z\nDTSTART:20261212T083000Z\nSTATUS:CANCELLED";
        assert_eq!(
            status_of(&[body], CalendarKind::Google),
            ImportItemStatus::Unsupported(UnsupportedReason::Cancelled)
        );
    }

    #[test]
    fn unresolvable_tzid_is_rejected_but_floating_is_not() {
        let body = "UID:a@example.com\nDTSTART;TZID=Mars/Olympus:20261205T083000";
        assert_eq!(
            status_of(&[body], CalendarKind::Google),
            ImportItemStatus::Unsupported(UnsupportedReason::UnknownTimezone("Mars/Olympus".into()))
        );
        let floating = "UID:a@example.com\nDTSTART:20261205T083000";
        assert_eq!(
            status_of(&[floating], CalendarKind::Google),
            ImportItemStatus::Importable
        );
    }

    #[test]
    fn recurring_series_in_a_non_whole_hour_zone_is_rejected() {
        let header = "BEGIN:VTIMEZONE\r\nTZID:Example Half Hour\r\nBEGIN:STANDARD\r\nDTSTART:16010101T000000\r\nTZOFFSETFROM:+0530\r\nTZOFFSETTO:+0530\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n";
        let c = cal_with(
            header,
            &["UID:a@example.com\nDTSTART;TZID=Example Half Hour:20261205T083000\nRRULE:FREQ=WEEKLY"],
        );
        let items = plan_ics_import(&c, CalendarKind::Google, "Europe/Madrid").unwrap();
        assert_eq!(
            items[0].status,
            ImportItemStatus::Unsupported(UnsupportedReason::UnsupportedRecurrence("TZID".into()))
        );
    }

    // ---- executor ----

    use crate::sync::calendar_provider::{CalendarProvider, FakeCalendarProvider, FoundEvent};

    fn fake() -> FakeCalendarProvider {
        FakeCalendarProvider::with_events(vec![])
    }

    fn file(vevents: &[&str]) -> String {
        let mut ics = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n");
        for body in vevents {
            ics.push_str("BEGIN:VEVENT\r\n");
            for line in body.lines() {
                ics.push_str(line);
                ics.push_str("\r\n");
            }
            ics.push_str("END:VEVENT\r\n");
        }
        ics.push_str("END:VCALENDAR\r\n");
        ics
    }

    const EV_A: &str = "UID:a@example.com\nDTSTART:20261205T083000Z\nSUMMARY:Quarterly Zebra Review";
    const EV_B: &str = "UID:b@example.com\nDTSTART:20261206T083000Z\nSUMMARY:Okapi Sync";
    const EV_CANCELLED: &str = "UID:c@example.com\nDTSTART:20261207T083000Z\nSTATUS:CANCELLED\nSUMMARY:Gone";

    fn write_tmp(name: &str, bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn read_rejects_files_over_10_mb() {
        let (_d, path) = write_tmp("big.ics", &vec![b'a'; MAX_ICS_IMPORT_BYTES + 1]);
        assert!(matches!(read_ics_file(&path), Err(AppError::InvalidInput(_))));
    }

    #[test]
    fn read_rejects_non_utf8() {
        let (_d, path) = write_tmp("a.ics", b"BEGIN:VCALENDAR\xE9");
        match read_ics_file(&path) {
            Err(AppError::InvalidInput(m)) => assert!(m.contains("UTF-8")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn read_strips_a_utf8_bom() {
        let (_d, path) = write_tmp("a.ICS", b"\xEF\xBB\xBFBEGIN:VCALENDAR");
        assert_eq!(read_ics_file(&path).unwrap(), "BEGIN:VCALENDAR");
    }

    #[test]
    fn read_rejects_other_extensions() {
        let (_d, path) = write_tmp("a.txt", b"BEGIN:VCALENDAR");
        assert!(matches!(read_ics_file(&path), Err(AppError::InvalidInput(_))));
    }

    #[tokio::test]
    async fn preview_marks_events_already_in_the_calendar() {
        let mut p = fake();
        p.found_by_uid.insert(
            "a@example.com".into(),
            FoundEvent {
                provider_event_id: "x".into(),
                imported_by_emailops: false,
                sequence: 0,
            },
        );
        let pv = preview_ics_import(&p, CalendarKind::Google, &file(&[EV_A, EV_B]), "f.ics", "UTC")
            .await
            .unwrap();
        assert_eq!(pv.file_name, "f.ics");
        assert_eq!(pv.items[0].status, "alreadyInCalendar");
        assert_eq!(pv.items[1].status, "importable");
        assert_eq!(pv.items[1].title, "Okapi Sync");
        assert_eq!(pv.items[1].start_time, 1796545800);
    }

    #[tokio::test]
    async fn preview_reports_unsupported_reason_code() {
        let pv = preview_ics_import(&fake(), CalendarKind::Google, &file(&[EV_CANCELLED]), "f.ics", "UTC")
            .await
            .unwrap();
        assert_eq!(pv.items[0].status, "unsupported");
        assert_eq!(pv.items[0].reason.as_deref(), Some("cancelled"));
    }

    #[test]
    fn every_unsupported_reason_maps_to_its_code() {
        let cases = [
            (UnsupportedReason::MissingUidOrStart, "missingUidOrStart", None),
            (UnsupportedReason::Cancelled, "cancelled", None),
            (
                UnsupportedReason::OutlookRecurrence("FREQ=HOURLY".into()),
                "outlookRecurrence",
                Some("FREQ=HOURLY"),
            ),
            (UnsupportedReason::OutlookRdate, "outlookRdate", None),
            (UnsupportedReason::OutlookOrphanOverride, "outlookOrphanOverride", None),
            (
                UnsupportedReason::UnsupportedRecurrence("EXRULE".into()),
                "unsupportedRecurrence",
                Some("EXRULE"),
            ),
            (
                UnsupportedReason::InvalidProperty("DTEND".into()),
                "invalidProperty",
                Some("DTEND"),
            ),
            (
                UnsupportedReason::UnknownTimezone("Foo".into()),
                "unknownTimezone",
                Some("Foo"),
            ),
        ];
        for (reason, code, detail) in cases {
            assert_eq!(reason_code(&reason), (code, detail.map(str::to_string)));
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the guard serializes the global logger across the await
    async fn import_skips_present_and_unsupported_and_counts() {
        let _guard = crate::services::events::seam_test_lock();
        let mut p = fake();
        p.found_by_uid.insert(
            "a@example.com".into(),
            FoundEvent {
                provider_event_id: "x".into(),
                imported_by_emailops: false,
                sequence: 0,
            },
        );
        let s = import_ics(&p, CalendarKind::Google, &file(&[EV_A, EV_B, EV_CANCELLED]), "UTC")
            .await
            .unwrap();
        assert_eq!((s.imported, s.already_present, s.unsupported), (1, 1, 1));
        assert_eq!(p.imported.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the guard serializes the global logger across the await
    async fn reimporting_the_same_file_imports_nothing() {
        let _guard = crate::services::events::seam_test_lock();
        let p = fake();
        let ics = file(&[EV_A, EV_B]);
        let first = import_ics(&p, CalendarKind::Google, &ics, "UTC").await.unwrap();
        assert_eq!(first.imported, 2);
        let second = import_ics(&p, CalendarKind::Google, &ics, "UTC").await.unwrap();
        assert_eq!((second.imported, second.already_present), (0, 2));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the guard serializes the global logger across the await
    async fn a_failed_event_does_not_stop_the_rest() {
        let _guard = crate::services::events::seam_test_lock();
        let mut p = fake();
        p.import_errors.insert("a@example.com".into(), "boom".into());
        let s = import_ics(&p, CalendarKind::Google, &file(&[EV_A, EV_B]), "UTC")
            .await
            .unwrap();
        assert_eq!(s.imported, 1);
        assert_eq!(s.failed.len(), 1);
        assert_eq!(s.failed[0].title, "Quarterly Zebra Review");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the guard serializes the global logger across the await
    async fn an_auth_error_aborts_the_import() {
        let _guard = crate::services::events::seam_test_lock();
        // `error` on the fake fails find_event_by_uid; reauth needs a custom provider.
        let p = AuthFailing { on_find: true };
        let err = import_ics(&p, CalendarKind::Google, &file(&[EV_A]), "UTC")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NeedsReauth { .. }));
        let err = preview_ics_import(&p, CalendarKind::Google, &file(&[EV_A]), "f.ics", "UTC")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NeedsReauth { .. }));
        let p = AuthFailing { on_find: false };
        let err = import_ics(&p, CalendarKind::Google, &file(&[EV_A]), "UTC")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::CalendarPermissionDenied { .. }));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the guard serializes the global logger across the await
    async fn import_logs_counts_only() {
        let _guard = crate::services::events::seam_test_lock();
        let logger = crate::services::logger::install_for_testing();
        import_ics(&fake(), CalendarKind::Google, &file(&[EV_A, EV_B]), "UTC")
            .await
            .unwrap();
        let events = logger.events();
        assert!(events.iter().any(|e| e.level == "info" && e.source == "sync"));
        assert!(events.iter().any(|e| e.level == "success" && e.source == "sync"));
        for e in &events {
            assert!(
                !e.message.contains("Zebra") && !e.message.contains("Okapi"),
                "{}",
                e.message
            );
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the guard serializes the global logger across the await
    async fn preview_reports_a_failed_lookup() {
        let _guard = crate::services::events::seam_test_lock();
        let mut p = fake();
        p.find_errors.insert("a@example.com".into(), "lookup boom".into());
        let pv = preview_ics_import(&p, CalendarKind::Google, &file(&[EV_A, EV_B]), "f.ics", "UTC")
            .await
            .unwrap();
        assert_eq!(pv.items[0].status, "unsupported");
        assert_eq!(pv.items[0].reason.as_deref(), Some("lookupFailed"));
        assert!(pv.items[0].reason_detail.as_deref().unwrap().contains("lookup boom"));
        assert_eq!(pv.items[1].status, "importable");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the guard serializes the global logger across the await
    async fn import_skips_an_event_whose_lookup_failed() {
        let _guard = crate::services::events::seam_test_lock();
        let logger = crate::services::logger::install_for_testing();
        let mut p = fake();
        p.find_errors.insert("a@example.com".into(), "lookup boom".into());
        let s = import_ics(&p, CalendarKind::Google, &file(&[EV_A, EV_B]), "UTC")
            .await
            .unwrap();
        assert_eq!((s.imported, s.failed.len()), (1, 1));
        assert_eq!(s.failed[0].title, "Quarterly Zebra Review");
        let imported = p.imported.lock().unwrap();
        assert!(imported.iter().all(|e| e.uid != "a@example.com"));
        let errors: Vec<_> = logger.events().into_iter().filter(|e| e.level == "error").collect();
        assert!(errors.iter().any(|e| e.message.contains("duplicate check failed")));
        assert!(errors
            .iter()
            .all(|e| !e.message.contains("Zebra") && !e.message.contains("a@example.com")));
    }

    /// A provider whose lookups (or imports) fail with an auth-class error.
    struct AuthFailing {
        on_find: bool,
    }

    #[async_trait::async_trait]
    impl CalendarProvider for AuthFailing {
        async fn list_calendars(&self) -> Result<Vec<crate::sync::calendar_provider::ProviderCalendar>> {
            unreachable!()
        }
        async fn list_events(
            &self,
            _: &str,
            _: i64,
            _: i64,
        ) -> Result<Vec<crate::sync::calendar_provider::ProviderCalendarEvent>> {
            unreachable!()
        }
        async fn create_event(
            &self,
            _: &crate::sync::calendar_provider::NewCalendarEvent,
        ) -> Result<crate::sync::calendar_provider::ProviderCalendarEvent> {
            unreachable!()
        }
        async fn delete_event(&self, _: &str, _: &str, _: bool, _: &str) -> Result<()> {
            unreachable!()
        }
        async fn truncate_recurring_event(&self, _: &str, _: &str, _: i64, _: bool) -> Result<()> {
            unreachable!()
        }
        async fn rsvp_by_ical_uid(
            &self,
            _: &str,
            _: crate::sync::calendar_provider::RsvpResponse,
            _: &str,
        ) -> Result<()> {
            unreachable!()
        }
        async fn find_event_by_uid(&self, _: &str) -> Result<Option<FoundEvent>> {
            if self.on_find {
                Err(AppError::NeedsReauth { account_id: "a".into() })
            } else {
                Ok(None)
            }
        }
        async fn import_event(&self, _: &ImportableEvent) -> Result<String> {
            Err(AppError::CalendarPermissionDenied { account_id: "a".into() })
        }
        async fn update_imported_event(&self, _: &str, _: &ImportableEvent) -> Result<()> {
            unreachable!()
        }
        async fn respond_to_event(
            &self,
            _: &str,
            _: &str,
            _: crate::sync::calendar_provider::RsvpResponse,
            _: &str,
            _: bool,
        ) -> Result<()> {
            unreachable!()
        }
    }
}
