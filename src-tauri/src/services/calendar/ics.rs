//! Minimal, pure iCalendar (RFC 5545) structural parser shared by the invite
//! card (`invite.rs`) and the ICS-feed calendar source
//! (`sync::ics_feed_calendar`). It splits a document into VEVENT property
//! lists and converts date/time values; it does not interpret recurrence
//! (that is the feed expander's job). ICS input is UNTRUSTED: parsing is
//! purely structural and never evaluates anything.

use super::windows_zones;

/// One content line: `NAME;PARAM=..:VALUE`, name upper-cased, params raw.
#[derive(Debug, Clone, PartialEq)]
pub struct IcsProperty {
    pub name: String,
    pub params: Vec<String>,
    pub value: String,
}

/// The properties of one VEVENT, in document order. Nested components
/// (VALARM) are skipped so an alarm's properties never shadow the event's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawVEvent {
    pub props: Vec<IcsProperty>,
}

impl RawVEvent {
    pub fn first(&self, name: &str) -> Option<&IcsProperty> {
        self.props.iter().find(|p| p.name == name)
    }

    pub fn first_value(&self, name: &str) -> Option<&str> {
        self.first(name).map(|p| p.value.as_str())
    }

    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a IcsProperty> + 'a {
        self.props.iter().filter(move |p| p.name == name)
    }
}

/// A VCALENDAR document reduced to what the callers need.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedCalendar {
    /// `X-WR-CALNAME`, unescaped.
    pub name: Option<String>,
    /// `X-WR-TIMEZONE` (IANA name) when the publisher set one.
    pub timezone: Option<String>,
    pub events: Vec<RawVEvent>,
}

/// Split an iCalendar document into its VEVENTs. Malformed input yields fewer
/// events, never an error: a feed is untrusted and partially usable.
pub fn parse_vcalendar(ics: &str) -> ParsedCalendar {
    let mut cal = ParsedCalendar::default();
    let mut current: Option<RawVEvent> = None;
    // Depth of nested components inside the current VEVENT (VALARM …).
    let mut nested = 0usize;

    for line in unfold_ics_lines(ics) {
        let Some((name, params, value)) = split_content_line(&line) else {
            continue;
        };
        match name.as_str() {
            "BEGIN" => {
                if current.is_some() {
                    nested += 1;
                } else if value.trim().eq_ignore_ascii_case("VEVENT") {
                    current = Some(RawVEvent::default());
                    nested = 0;
                }
            }
            "END" => {
                if nested > 0 {
                    nested -= 1;
                } else if value.trim().eq_ignore_ascii_case("VEVENT") {
                    if let Some(event) = current.take() {
                        cal.events.push(event);
                    }
                }
            }
            _ => match current.as_mut() {
                Some(event) if nested == 0 => event.props.push(IcsProperty { name, params, value }),
                Some(_) => {}
                None => match name.as_str() {
                    "X-WR-CALNAME" => cal.name = Some(unescape_text(value.trim())),
                    "X-WR-TIMEZONE" => cal.timezone = Some(value.trim().to_string()),
                    _ => {}
                },
            },
        }
    }
    cal
}

/// Unfold RFC 5545 folded lines (CRLF followed by space/tab continues the line).
pub fn unfold_ics_lines(ics: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in ics.replace("\r\n", "\n").split('\n') {
        if let Some(continuation) = raw.strip_prefix(' ').or_else(|| raw.strip_prefix('\t')) {
            if let Some(last) = lines.last_mut() {
                last.push_str(continuation);
                continue;
            }
        }
        lines.push(raw.to_string());
    }
    lines
}

/// Split a content line into (name, params, value) at the first `:` outside
/// double quotes. `DTSTART;TZID=Europe/Madrid:20260728T073000` →
/// ("DTSTART", ["TZID=Europe/Madrid"], "20260728T073000").
pub fn split_content_line(line: &str) -> Option<(String, Vec<String>, String)> {
    let mut in_quotes = false;
    let mut colon_at = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon_at = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon_at = colon_at?;
    let (head, value) = line.split_at(colon_at);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts.map(|p| p.to_string()).collect();
    Some((name, params, value[1..].to_string()))
}

/// Unescape RFC 5545 TEXT values (`\\n` → newline, `\\,` `\;` `\\\\` literal).
pub fn unescape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some(escaped) => out.push(escaped),
            None => out.push('\\'),
        }
    }
    out
}

/// Parse an ICS date/time value with its params into UTC epoch seconds.
/// Handles `...Z` (UTC), `TZID=<zone>` local wall time, floating local time
/// (treated as UTC — best effort), and `VALUE=DATE` all-day dates.
pub fn parse_ics_time(value: &str, params: &[String]) -> Option<(i64, bool)> {
    let value = value.trim();
    if params.iter().any(|p| p.eq_ignore_ascii_case("VALUE=DATE")) || (value.len() == 8 && !value.contains('T')) {
        let date = chrono::NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some((date.and_hms_opt(0, 0, 0)?.and_utc().timestamp(), true));
    }
    if let Some(stripped) = value.strip_suffix('Z') {
        let naive = chrono::NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
        return Some((naive.and_utc().timestamp(), false));
    }
    let naive = chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    let tzid = params
        .iter()
        .find_map(|p| p.strip_prefix("TZID=").or_else(|| p.strip_prefix("tzid=")));
    if let Some(tz) = tzid.and_then(resolve_tzid) {
        use chrono::TimeZone;
        let epoch = match tz.from_local_datetime(&naive).earliest() {
            Some(local) => local.timestamp(),
            None => skipped_local_time(tz, naive),
        };
        return Some((epoch, false));
    }
    // Unknown zone / floating time: UTC is the least-wrong fallback.
    Some((naive.and_utc().timestamp(), false))
}

/// The zone a `TZID` names: an IANA name, or the Windows name Outlook writes.
fn resolve_tzid(tzid: &str) -> Option<chrono_tz::Tz> {
    let name = tzid.trim().trim_matches('"');
    name.parse()
        .ok()
        .or_else(|| windows_zones::iana_name(name)?.parse().ok())
}

/// `param` with a `TZID` rewritten to its IANA name, for readers that know no
/// Windows names, such as the `rrule` parser.
pub fn with_iana_tzid(param: &str) -> String {
    let zone = param
        .strip_prefix("TZID=")
        .or_else(|| param.strip_prefix("tzid="))
        .and_then(resolve_tzid);
    match zone {
        Some(tz) => format!("TZID={}", tz.name()),
        None => param.to_string(),
    }
}

/// RFC 5545 §3.3.5: a local time the change to daylight time skips is read
/// with the offset in force before the change. Zones change offset at most
/// once a day, so the day before is on the right side of it.
fn skipped_local_time(tz: chrono_tz::Tz, naive: chrono::NaiveDateTime) -> i64 {
    use chrono::{Offset, TimeZone};
    let before = tz.offset_from_utc_datetime(&(naive - chrono::Days::new(1))).fix();
    naive.and_utc().timestamp() - i64::from(before.local_minus_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_EVENTS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Work\\, shared\r\nX-WR-TIMEZONE:Europe/Madrid\r\nBEGIN:VEVENT\r\nUID:one@example.com\r\nDTSTART;TZID=Europe/Madrid:20260728T073000\r\nSUMMARY:First\r\nBEGIN:VALARM\r\nTRIGGER:-PT10M\r\nACTION:DISPLAY\r\nEND:VALARM\r\nDESCRIPTION:after the alarm\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:two@example.com\r\nDTSTART:20260729T070000Z\r\nSUMMARY:Second\r\nEXDATE:20260805T070000Z\r\nEXDATE:20260812T070000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn parse_vcalendar_returns_every_vevent_and_the_calendar_name() {
        let cal = parse_vcalendar(TWO_EVENTS);
        assert_eq!(cal.name.as_deref(), Some("Work, shared"));
        assert_eq!(cal.timezone.as_deref(), Some("Europe/Madrid"));
        assert_eq!(cal.events.len(), 2);
        assert_eq!(cal.events[0].first_value("UID"), Some("one@example.com"));
        assert_eq!(cal.events[1].first_value("SUMMARY"), Some("Second"));
    }

    #[test]
    fn nested_valarm_does_not_end_the_vevent() {
        let cal = parse_vcalendar(TWO_EVENTS);
        let first = &cal.events[0];
        assert_eq!(first.first_value("DESCRIPTION"), Some("after the alarm"));
        // Alarm properties stay out of the event's own property list.
        assert_eq!(first.first("TRIGGER"), None);
    }

    #[test]
    fn all_returns_repeated_properties_in_order() {
        let cal = parse_vcalendar(TWO_EVENTS);
        let exdates: Vec<&str> = cal.events[1].all("EXDATE").map(|p| p.value.as_str()).collect();
        assert_eq!(exdates, vec!["20260805T070000Z", "20260812T070000Z"]);
    }

    #[test]
    fn property_keeps_params_and_raw_value() {
        let cal = parse_vcalendar(TWO_EVENTS);
        let dtstart = cal.events[0].first("DTSTART").expect("DTSTART present");
        assert_eq!(dtstart.params, vec!["TZID=Europe/Madrid".to_string()]);
        assert_eq!(dtstart.value, "20260728T073000");
    }

    #[test]
    fn document_without_events_is_empty_but_named() {
        let cal = parse_vcalendar("BEGIN:VCALENDAR\r\nX-WR-CALNAME:Empty\r\nEND:VCALENDAR\r\n");
        assert_eq!(cal.name.as_deref(), Some("Empty"));
        assert!(cal.events.is_empty());
    }

    #[test]
    fn unfold_joins_continuation_lines() {
        let lines = unfold_ics_lines("SUMMARY:long\r\n  title\r\nUID:x\r\n");
        assert_eq!(lines[0], "SUMMARY:long title");
        assert_eq!(lines[1], "UID:x");
    }

    fn tzid(zone: &str) -> Vec<String> {
        vec![format!("TZID={zone}")]
    }

    fn utc(date: &str) -> i64 {
        parse_ics_time(date, &[]).expect("valid UTC value").0
    }

    #[test]
    fn a_windows_zone_name_is_read_as_its_zone_not_as_utc() {
        assert_eq!(
            parse_ics_time("20260728T093000", &tzid("Romance Standard Time")),
            Some((utc("20260728T073000Z"), false)),
        );
        assert_eq!(
            parse_ics_time("20260115T093000", &tzid("\"Pacific Standard Time\"")),
            Some((utc("20260115T173000Z"), false)),
        );
    }

    /// RFC 5545 §3.3.5: a local time skipped by the change to daylight time is
    /// read with the offset in force before the change.
    #[test]
    fn a_time_inside_the_daylight_saving_gap_uses_the_offset_before_it() {
        assert_eq!(
            parse_ics_time("20070311T023000", &tzid("America/New_York")),
            Some((utc("20070311T073000Z"), false)),
        );
        assert_eq!(
            parse_ics_time("20260329T023000", &tzid("Europe/Madrid")),
            Some((utc("20260329T013000Z"), false)),
        );
        // A gap of half an hour, not a whole one.
        assert_eq!(
            parse_ics_time("20261004T021500", &tzid("Australia/Lord_Howe")),
            Some((utc("20261003T154500Z"), false)),
        );
    }

    #[test]
    fn a_repeated_local_time_keeps_its_first_occurrence() {
        assert_eq!(
            parse_ics_time("20261025T023000", &tzid("Europe/Madrid")),
            Some((utc("20261025T003000Z"), false)),
        );
    }

    #[test]
    fn an_unknown_zone_still_falls_back_to_utc() {
        assert_eq!(
            parse_ics_time("20260728T093000", &tzid("Customized Time Zone")),
            Some((utc("20260728T093000Z"), false)),
        );
    }

    #[test]
    fn parse_ics_time_handles_utc_tzid_and_date() {
        assert_eq!(parse_ics_time("20260729T070000Z", &[]), Some((1_785_308_400, false)));
        assert_eq!(
            parse_ics_time("20260728T073000", &["TZID=Europe/Madrid".to_string()]),
            Some((1_785_216_600, false))
        );
        assert_eq!(
            parse_ics_time("20260728", &["VALUE=DATE".to_string()]),
            Some((1_785_196_800, true))
        );
    }
}
