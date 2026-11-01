//! Minimal, pure iCalendar (RFC 5545) structural parser shared by the invite
//! card (`invite.rs`) and the ICS-feed calendar source
//! (`sync::ics_feed_calendar`). It splits a document into VEVENT property
//! lists and converts date/time values; it does not interpret recurrence
//! (that is the feed expander's job). ICS input is UNTRUSTED: parsing is
//! purely structural and never evaluates anything.

use std::collections::{HashMap, HashSet};

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
    /// Calendar-wide scheduling method (REQUEST, CANCEL, ...).
    pub method: Option<String>,
    /// Unambiguous fixed offsets declared by this document. Unsupported or
    /// malformed zones never prevent callers from using the calendar events.
    pub fixed_zones: HashMap<String, chrono::FixedOffset>,
    pub events: Vec<RawVEvent>,
}

/// Split an iCalendar document into its VEVENTs. Malformed input yields fewer
/// events, never an error: a feed is untrusted and partially usable.
pub fn parse_vcalendar(ics: &str) -> ParsedCalendar {
    let mut cal = ParsedCalendar::default();
    let mut current: Option<RawVEvent> = None;
    // Depth of nested components inside the current VEVENT (VALARM …).
    let mut nested = 0usize;
    let mut zone_stack: Vec<ZoneComponent> = Vec::new();
    let mut zone_components = Vec::new();
    let mut malformed_zones = false;

    for line in unfold_ics_lines(ics) {
        let Some((name, params, value)) = split_content_line(&line) else {
            continue;
        };
        let component = value.trim();
        if current.is_none() && name == "BEGIN" && component.eq_ignore_ascii_case("VTIMEZONE") {
            if !zone_stack.is_empty() {
                malformed_zones = true;
                zone_stack.clear();
            }
            zone_stack.push(ZoneComponent::new("VTIMEZONE"));
            continue;
        }
        if !zone_stack.is_empty() {
            match name.as_str() {
                "BEGIN" if component.eq_ignore_ascii_case("VEVENT") => {
                    // Recover the event even when a preceding zone was not closed.
                    malformed_zones = true;
                    zone_stack.clear();
                }
                "BEGIN" => {
                    // Bound the component tree built from untrusted input.
                    if zone_stack.len() >= 32 {
                        malformed_zones = true;
                        zone_stack.clear();
                        continue;
                    }
                    zone_stack.push(ZoneComponent::new(component));
                    continue;
                }
                "END" => {
                    if zone_stack
                        .last()
                        .is_some_and(|c| c.name.eq_ignore_ascii_case(component))
                    {
                        if let Some(finished) = zone_stack.pop() {
                            if let Some(parent) = zone_stack.last_mut() {
                                parent.children.push(finished);
                            } else {
                                zone_components.push(finished);
                            }
                        }
                    } else {
                        malformed_zones = true;
                        zone_stack.clear();
                    }
                    continue;
                }
                _ => {
                    if let Some(zone) = zone_stack.last_mut() {
                        zone.props.push(IcsProperty { name, params, value });
                    }
                    continue;
                }
            }
        }
        match name.as_str() {
            "BEGIN" => {
                if current.is_some() {
                    nested += 1;
                } else if component.eq_ignore_ascii_case("VEVENT") {
                    current = Some(RawVEvent::default());
                    nested = 0;
                }
            }
            "END" => {
                if nested > 0 {
                    nested -= 1;
                } else if component.eq_ignore_ascii_case("VEVENT") {
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
                    "X-WR-TIMEZONE" => cal.timezone = Some(normalize_tzid(&value).to_string()),
                    "METHOD" => cal.method = Some(value.trim().to_ascii_uppercase()),
                    _ => {}
                },
            },
        }
    }
    if !malformed_zones && zone_stack.is_empty() {
        cal.fixed_zones = collect_fixed_zones(zone_components);
    }
    cal
}

/// A timezone component retains its own properties instead of mutable counters
/// shared between observations. No rules or arbitrary timezone code are evaluated.
#[derive(Default)]
struct ZoneComponent {
    name: String,
    props: Vec<IcsProperty>,
    children: Vec<ZoneComponent>,
}
impl ZoneComponent {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_ascii_uppercase(),
            ..Self::default()
        }
    }

    fn one_value(&self, name: &str) -> Option<&str> {
        let mut values = self.props.iter().filter(|p| p.name == name);
        let value = values.next()?.value.as_str();
        values.next().is_none().then_some(value)
    }

    fn contains_property(&self, name: &str) -> bool {
        self.props.iter().any(|p| p.name == name) || self.children.iter().any(|c| c.contains_property(name))
    }
}

fn normalize_tzid(value: &str) -> &str {
    value.trim().trim_matches('"').trim()
}

pub(crate) fn tzid_from_params(params: &[String]) -> Option<&str> {
    params.iter().find_map(|param| {
        let (name, value) = param.split_once('=')?;
        name.trim().eq_ignore_ascii_case("TZID").then(|| normalize_tzid(value))
    })
}

fn collect_fixed_zones(components: Vec<ZoneComponent>) -> HashMap<String, chrono::FixedOffset> {
    let mut zones = HashMap::new();
    let mut seen = HashSet::new();
    for zone in components {
        let Some(id) = zone.one_value("TZID").map(normalize_tzid).filter(|id| !id.is_empty()) else {
            return HashMap::new();
        };
        if !seen.insert(id.to_string()) || zone.children.iter().any(|c| c.contains_property("TZID")) {
            return HashMap::new();
        }
        if zone.children.is_empty() || zone.children.len() > 2 {
            continue;
        }
        let mut standard_index = None;
        let mut daylight_index = None;
        let mut bad_name = false;
        for (index, child) in zone.children.iter().enumerate() {
            match child.name.as_str() {
                "STANDARD" if standard_index.is_none() => standard_index = Some(index),
                "DAYLIGHT" if daylight_index.is_none() => daylight_index = Some(index),
                _ => bad_name = true,
            }
        }
        if bad_name || standard_index.is_none() {
            continue;
        }
        if zone.children.iter().any(|child| !child.children.is_empty())
            || zone.contains_property("RRULE")
            || zone.contains_property("RDATE")
            || zone
                .props
                .iter()
                .any(|p| matches!(p.name.as_str(), "TZOFFSETFROM" | "TZOFFSETTO"))
        {
            continue;
        }
        let mut all_offsets = Vec::new();
        let mut unsupported = false;
        for child in &zone.children {
            let Some(from) = child.one_value("TZOFFSETFROM").and_then(parse_utc_offset) else {
                unsupported = true;
                break;
            };
            let Some(to) = child.one_value("TZOFFSETTO").and_then(parse_utc_offset) else {
                unsupported = true;
                break;
            };
            if child.contains_property("RRULE") || child.contains_property("RDATE") {
                unsupported = true;
                break;
            }
            all_offsets.push(from);
            all_offsets.push(to);
        }
        if unsupported || all_offsets.is_empty() {
            continue;
        }
        if !all_offsets.iter().all(|offset| *offset == all_offsets[0]) {
            continue;
        }
        zones.insert(id.to_string(), all_offsets[0]);
    }
    zones
}

fn parse_utc_offset(value: &str) -> Option<chrono::FixedOffset> {
    let value = value.trim();
    if !matches!(value.len(), 5 | 7)
        || !matches!(value.as_bytes()[0], b'+' | b'-')
        || !value[1..].bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let hours: i32 = value[1..3].parse().ok()?;
    let minutes: i32 = value[3..5].parse().ok()?;
    let seconds: i32 = value.get(5..7).unwrap_or("00").parse().ok()?;
    if hours >= 24 || minutes >= 60 || seconds >= 60 {
        return None;
    }
    chrono::FixedOffset::east_opt((hours * 3600 + minutes * 60 + seconds) * if value.starts_with('-') { -1 } else { 1 })
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
    parse_ics_time_with_zones(value, params, None)
}

/// Parse with document-local fixed offsets for non-IANA identifiers. IANA zones
/// keep their native DST rules; absent/unsupported zones retain the UTC fallback.
pub fn parse_ics_time_with_fixed_zones(
    value: &str,
    params: &[String],
    zones: &HashMap<String, chrono::FixedOffset>,
) -> Option<(i64, bool)> {
    parse_ics_time_with_zones(value, params, Some(zones))
}

fn parse_ics_time_with_zones(
    value: &str,
    params: &[String],
    zones: Option<&HashMap<String, chrono::FixedOffset>>,
) -> Option<(i64, bool)> {
    use chrono::TimeZone;
    let value = value.trim();
    let parameter = |name: &str| {
        params.iter().find_map(|p| {
            let (key, value) = p.split_once('=')?;
            key.trim().eq_ignore_ascii_case(name).then_some(value.trim())
        })
    };
    if parameter("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE")) || (value.len() == 8 && !value.contains('T'))
    {
        let date = chrono::NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some((date.and_hms_opt(0, 0, 0)?.and_utc().timestamp(), true));
    }
    if let Some(stripped) = value.strip_suffix('Z') {
        let naive = chrono::NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
        return Some((naive.and_utc().timestamp(), false));
    }
    let naive = chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    if let Some(id) = tzid_from_params(params) {
        if let Ok(tz) = id.parse::<chrono_tz::Tz>() {
            if let Some(local) = tz.from_local_datetime(&naive).earliest() {
                return Some((local.timestamp(), false));
            }
        } else if let Some(offset) = zones.and_then(|zones| zones.get(id)) {
            return offset
                .from_local_datetime(&naive)
                .single()
                .map(|local| (local.timestamp(), false));
        }
    }
    Some((naive.and_utc().timestamp(), false))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_EVENTS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Work\\, shared\r\nX-WR-TIMEZONE:Europe/Madrid\r\nBEGIN:VEVENT\r\nUID:one@example.com\r\nDTSTART;TZID=Europe/Madrid:20260728T073000\r\nSUMMARY:First\r\nBEGIN:VALARM\r\nTRIGGER:-PT10M\r\nACTION:DISPLAY\r\nEND:VALARM\r\nDESCRIPTION:after the alarm\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:two@example.com\r\nDTSTART:20260729T070000Z\r\nSUMMARY:Second\r\nEXDATE:20260805T070000Z\r\nEXDATE:20260812T070000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn valid_fixed_zone_standard_daylight() {
        let ics="BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTIMEZONE\r\nTZID:Example Fixed Time\r\nBEGIN:STANDARD\r\nDTSTART:16010101T000000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0200\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16010101T000000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0200\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:e\r\nDTSTART;TZID=Example Fixed Time:20261023T103000\r\nDTEND;TZID=Example Fixed Time:20261023T113000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let cal = parse_vcalendar(&ics);
        assert_eq!(cal.fixed_zones.len(), 1);
        assert_eq!(
            cal.fixed_zones.get("Example Fixed Time").unwrap().local_minus_utc(),
            7200
        );
        assert_eq!(cal.events.len(), 1);
        let prop = cal.events[0].first("DTSTART").unwrap();
        let (ts, all_day) = parse_ics_time_with_fixed_zones(&prop.value, &prop.params, &cal.fixed_zones).unwrap();
        assert_eq!(
            chrono::DateTime::from_timestamp(ts, 0).unwrap().to_rfc3339(),
            "2026-10-23T08:30:00+00:00"
        );
        assert!(!all_day);
    }

    #[test]
    fn invalid_fixed_zone_observation_shapes() {
        let wrap = |body: &str| -> String {
            format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTIMEZONE\r\nTZID:Example Fixed Time\r\n{}\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:e\r\nDTSTART;TZID=Example Fixed Time:20261023T103000\r\nDTEND;TZID=Example Fixed Time:20261023T113000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n", body)
        };
        let obs = |c: &str, f: &str, t: &str| -> String {
            format!(
                "BEGIN:{}\r\nDTSTART:16010101T000000\r\nTZOFFSETFROM:{}\r\nTZOFFSETTO:{}\r\nEND:{}\r\n",
                c, f, t, c
            )
        };
        let cases = [
            format!(
                "{}{}{}",
                obs("STANDARD", "+0200", "+0200"),
                obs("DAYLIGHT", "+0200", "+0200"),
                obs("DAYLIGHT", "+0200", "+0200")
            ),
            format!(
                "{}{}{}",
                obs("STANDARD", "+0200", "+0200"),
                obs("STANDARD", "+0200", "+0200"),
                obs("DAYLIGHT", "+0200", "+0200")
            ),
            format!(
                "{}{}",
                obs("STANDARD", "+0200", "+0200"),
                obs("DAYLIGHT", "+0300", "+0300")
            ),
            format!(
                "{}BEGIN:DAYLIGHT\r\nDTSTART:16010101T000000\r\nTZOFFSETFROM:+0300\r\nEND:DAYLIGHT\r\n",
                obs("STANDARD", "+0200", "+0200")
            ),
            format!(
                "{}{}",
                obs("STANDARD", "+0200", "+0200"),
                obs("DAYLIGHT", "+0200", "+0300")
            ),
        ];
        for body in cases {
            let cal = parse_vcalendar(&wrap(&body));
            assert!(cal.fixed_zones.is_empty());
            assert_eq!(cal.events.len(), 1);
            let prop = cal.events[0].first("DTSTART").unwrap();
            let (ts, all_day) = parse_ics_time_with_fixed_zones(&prop.value, &prop.params, &cal.fixed_zones).unwrap();
            assert_eq!(
                chrono::DateTime::from_timestamp(ts, 0).unwrap().to_rfc3339(),
                "2026-10-23T10:30:00+00:00"
            );
            assert!(!all_day);
        }
    }

    #[test]
    fn truncated_vevent_missing_end_is_rejected() {
        let cal = parse_vcalendar(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:truncated@example.test\r\nSUMMARY:Partial\r\nEND:VCALENDAR\r\n",
        );
        assert!(cal.events.is_empty());
    }

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
    fn zone(from: &str, to: &str, extra: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Example Fixed Time\r\nBEGIN:STANDARD\r\nTZOFFSETFROM:{from}\r\nTZOFFSETTO:{to}\r\nDTSTART:16010101T000000\r\n{extra}END:STANDARD\r\nEND:VTIMEZONE\r\nEND:VCALENDAR\r\n")
    }
    #[test]
    fn embedded_fixed_offset_resolves_non_iana_wall_clock() {
        let zones = parse_vcalendar(&zone("+0200", "+0200", "")).fixed_zones;
        let (t, all_day) =
            parse_ics_time_with_fixed_zones("20261023T103000", &["TZID=\"Example Fixed Time\"".into()], &zones)
                .unwrap();
        assert_eq!(
            chrono::DateTime::from_timestamp(t, 0).unwrap().to_rfc3339(),
            "2026-10-23T08:30:00+00:00"
        );
        assert!(!all_day);
    }
    #[test]
    fn negative_offset_including_seconds_preserved() {
        let zones = parse_vcalendar(&zone("-033030", "-033030", "")).fixed_zones;
        assert_eq!(zones["Example Fixed Time"].local_minus_utc(), -12630);
    }
    #[test]
    fn seasonal_and_conflicting_offsets_are_not_assumed_fixed() {
        for input in [
            zone("+0100", "+0200", ""),
            zone("+0200", "+0200", "RRULE:FREQ=YEARLY\r\n"),
            zone("+0200", "+0200", "RDATE:20270328T020000\r\n"),
            zone("+0200", "+0200", "BEGIN:DAYLIGHT\r\nEND:DAYLIGHT\r\n"),
        ] {
            assert!(parse_vcalendar(&input).fixed_zones.is_empty());
        }
    }
    #[test]
    fn invalid_offsets_are_not_used() {
        for offset in ["+2500", "+0260", "-006099", "0200", "+éé"] {
            assert!(parse_vcalendar(&zone(offset, offset, "")).fixed_zones.is_empty());
        }
    }
    #[test]
    fn duplicate_identity_is_ambiguous_even_if_offsets_match() {
        let input = zone("+0200", "+0200", "");
        assert!(parse_vcalendar(&(input.clone() + &input)).fixed_zones.is_empty());
        assert!(parse_vcalendar(&input.replace(
            "TZID:Example Fixed Time",
            "TZID:Example Fixed Time\r\nTZID:Another Name"
        ))
        .fixed_zones
        .is_empty());
    }
    #[test]
    fn unclosed_or_nested_zone_is_not_accepted() {
        let input = zone("+0200", "+0200", "");
        assert!(parse_vcalendar(&input.replace("END:VTIMEZONE\r\n", ""))
            .fixed_zones
            .is_empty());
        assert!(
            parse_vcalendar(&input.replace("BEGIN:STANDARD", "BEGIN:VTIMEZONE\r\nBEGIN:STANDARD"))
                .fixed_zones
                .is_empty()
        );
    }
    #[test]
    fn repeated_offsets_or_observances_are_not_guessed() {
        let input = zone("+0200", "+0200", "TZOFFSETFROM:+0200\r\n");
        assert!(parse_vcalendar(&input).fixed_zones.is_empty());
        let input = zone("+0200", "+0200", "").replace(
            "END:VTIMEZONE",
            "BEGIN:STANDARD\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0200\r\nEND:STANDARD\r\nEND:VTIMEZONE",
        );
        assert!(parse_vcalendar(&input).fixed_zones.is_empty());
    }
    #[test]
    fn iana_dst_rules_take_precedence_over_fixed_name_redefinition() {
        let mut zones = std::collections::HashMap::new();
        zones.insert("Europe/Zurich".into(), chrono::FixedOffset::east_opt(0).unwrap());
        let (t, _) =
            parse_ics_time_with_fixed_zones("20261026T103000", &["TZID=Europe/Zurich".into()], &zones).unwrap();
        assert_eq!(
            chrono::DateTime::from_timestamp(t, 0).unwrap().to_rfc3339(),
            "2026-10-26T09:30:00+00:00"
        );
    }
    #[test]
    fn utc_and_date_only_keep_existing_semantics() {
        let zones = parse_vcalendar(&zone("+0200", "+0200", "")).fixed_zones;
        for (value, params) in [
            ("20261023T103000Z", vec!["TZID=Example Fixed Time".into()]),
            ("20261023", vec!["VALUE=DATE".into(), "TZID=Example Fixed Time".into()]),
        ] {
            assert_eq!(
                parse_ics_time_with_fixed_zones(value, &params, &zones),
                parse_ics_time(value, &params)
            );
        }
    }
    #[test]
    fn callers_without_document_context_keep_legacy_unknown_zone_behavior() {
        let params = vec!["TZID=Unknown/Example".into()];
        assert_eq!(
            parse_ics_time_with_fixed_zones("20261023T103000", &params, &std::collections::HashMap::new()),
            parse_ics_time("20261023T103000", &params)
        );
    }
    fn with_event(document: &str) -> String {
        document.replace("END:VCALENDAR", "BEGIN:VEVENT\r\nUID:fixed-zone-test@example.test\r\nDTSTART;TZID=\" Example Fixed Time \":20261023T103000\r\nSUMMARY:Fixed-zone event\r\nEND:VEVENT\r\nEND:VCALENDAR")
    }

    #[test]
    fn malformed_and_duplicate_zone_definitions_keep_events_with_legacy_fallback() {
        let input = zone("+0200", "+0200", "");
        let block = input
            .strip_prefix("BEGIN:VCALENDAR\r\n")
            .unwrap()
            .strip_suffix("END:VCALENDAR\r\n")
            .unwrap();
        let duplicate = format!("BEGIN:VCALENDAR\r\n{block}{block}END:VCALENDAR\r\n");
        for malformed in [
            duplicate,
            input.replace("TZID:Example Fixed Time\r\n", ""),
            input.replace("BEGIN:STANDARD", "BEGIN:STANDARD\r\nTZID:Example Fixed Time"),
            input.replace("END:STANDARD\r\n", ""),
            input.replace("END:VTIMEZONE\r\n", ""),
        ] {
            let cal = parse_vcalendar(&with_event(&malformed));
            assert_eq!(cal.events.len(), 1, "{malformed}");
            assert!(cal.fixed_zones.is_empty(), "{malformed}");
            let start = cal.events[0].first("DTSTART").unwrap();
            assert_eq!(
                parse_ics_time_with_fixed_zones(&start.value, &start.params, &cal.fixed_zones),
                parse_ics_time(&start.value, &start.params)
            );
        }
    }

    #[test]
    fn quoted_tzid_whitespace_and_component_values_share_normalization() {
        let input = with_event(&zone("+0200", "+0200", ""))
            .replace("TZID:Example Fixed Time", "TZID: \" Example Fixed Time  \"  ")
            .replace("BEGIN:VTIMEZONE", "BEGIN: VTIMEZONE  ")
            .replace("END:VTIMEZONE", "END: VTIMEZONE  ")
            .replace("BEGIN:STANDARD", "BEGIN: STANDARD  ")
            .replace("END:STANDARD", "END: STANDARD  ");
        let cal = parse_vcalendar(&input);
        assert_eq!(cal.fixed_zones.len(), 1);
        let start = cal.events[0].first("DTSTART").unwrap();
        assert_eq!(
            parse_ics_time_with_fixed_zones(&start.value, &start.params, &cal.fixed_zones),
            parse_ics_time("20261023T083000Z", &[])
        );
        assert_eq!(
            parse_ics_time("20260728T073000", &[" TzId = \" Europe/Madrid \" ".into()]),
            parse_ics_time("20260728T073000", &["TZID=Europe/Madrid".into()])
        );
    }

    #[test]
    fn folded_zone_identity_and_calendar_method_survive_shared_parse() {
        let input = with_event(&zone("+0200", "+0200", ""))
            .replace("BEGIN:VCALENDAR", "BEGIN:VCALENDAR\r\nMETHOD: cancel ")
            .replace("TZID:Example Fixed Time", "TZID:Example Fixed\r\n  Time");
        let cal = parse_vcalendar(&input);
        assert_eq!(cal.method.as_deref(), Some("CANCEL"));
        assert_eq!(cal.events.len(), 1);
        assert!(cal.fixed_zones.contains_key("Example Fixed Time"));
    }

    #[test]
    fn document_context_is_complete_even_when_zone_follows_event() {
        let input = with_event("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n");
        let definition = zone("+0200", "+0200", "");
        let block = definition
            .strip_prefix("BEGIN:VCALENDAR\r\n")
            .unwrap()
            .strip_suffix("END:VCALENDAR\r\n")
            .unwrap();
        let input = input.replace("END:VCALENDAR", &format!("{block}END:VCALENDAR"));
        let cal = parse_vcalendar(&input);
        let start = cal.events[0].first("DTSTART").unwrap();
        assert_eq!(
            parse_ics_time_with_fixed_zones(&start.value, &start.params, &cal.fixed_zones),
            parse_ics_time("20261023T083000Z", &[])
        );
    }

    #[test]
    fn deeply_nested_zone_is_discarded_without_hiding_the_event() {
        let nested = format!("{}{}", "BEGIN:X-NESTED\r\n".repeat(64), "END:X-NESTED\r\n".repeat(64));
        let cal = parse_vcalendar(&with_event(&zone("+0200", "+0200", &nested)));
        assert!(cal.fixed_zones.is_empty());
        assert_eq!(cal.events.len(), 1);
    }
}
