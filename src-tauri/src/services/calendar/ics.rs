//! Minimal, pure iCalendar (RFC 5545) structural parser shared by the invite
//! card (`invite.rs`) and the ICS-feed calendar source
//! (`sync::ics_feed_calendar`). It splits a document into VEVENT property
//! lists and converts date/time values; it does not interpret recurrence
//! (that is the feed expander's job). ICS input is UNTRUSTED: parsing is
//! purely structural and never evaluates anything.

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
    if let Some(tzid) = tzid {
        if let Ok(tz) = tzid.trim_matches('"').parse::<chrono_tz::Tz>() {
            use chrono::TimeZone;
            if let Some(local) = tz.from_local_datetime(&naive).earliest() {
                return Some((local.timestamp(), false));
            }
        }
    }
    // Unknown zone / floating time: UTC is the least-wrong fallback.
    Some((naive.and_utc().timestamp(), false))
}

/// Proven fixed VTIMEZONE offsets for non-IANA identifiers. This deliberately
/// does not interpret seasonal rules, infer a geographic zone, or change the
/// existing parser's behavior for callers without document context.
pub(super) fn fixed_ics_zones(
    ics: &str,
) -> Option<std::collections::HashMap<String, chrono::FixedOffset>> {
    use std::collections::{HashMap, HashSet};
    let mut zones = HashMap::new();
    let mut seen = HashSet::new();
    let (mut inside, mut standard, mut unsupported) = (false, false, false);
    let (mut id, mut from, mut to): (
        Option<String>,
        Option<chrono::FixedOffset>,
        Option<chrono::FixedOffset>,
    ) = (None, None, None);
    let (mut observations, mut id_count, mut from_count, mut to_count) = (0, 0, 0, 0);
    for line in unfold_ics_lines(ics) {
        let Some((name, _, value)) = split_content_line(&line) else {
            continue;
        };
        if name == "BEGIN" && value.eq_ignore_ascii_case("VTIMEZONE") {
            if inside {
                return None;
            }
            inside = true;
            standard = false;
            unsupported = false;
            id = None;
            from = None;
            to = None;
            observations = 0;
            id_count = 0;
            from_count = 0;
            to_count = 0;
            continue;
        }
        if !inside {
            continue;
        }
        if name == "END" && value.eq_ignore_ascii_case("VTIMEZONE") {
            if standard {
                return None;
            }
            let key = id.take()?;
            if !seen.insert(key.clone()) {
                return None;
            }
            if !unsupported && observations == 1 && from_count == 1 && to_count == 1 {
                if let (Some(a), Some(b)) = (from, to) {
                    if a == b {
                        zones.insert(key, a);
                    }
                }
            }
            inside = false;
        } else if name == "BEGIN" {
            if value.eq_ignore_ascii_case("STANDARD") && !standard {
                standard = true;
                observations += 1;
            } else {
                unsupported = true;
            }
        } else if name == "END" && value.eq_ignore_ascii_case("STANDARD") {
            if !standard {
                return None;
            }
            standard = false;
        } else if name == "TZID" {
            id_count += 1;
            if standard || id_count != 1 || value.trim().is_empty() {
                return None;
            }
            id = Some(value.trim().trim_matches('"').to_string());
        } else if name == "RRULE" || name == "RDATE" {
            unsupported = true;
        } else if name == "TZOFFSETFROM" || name == "TZOFFSETTO" {
            if !standard {
                unsupported = true;
            }
            let value = value.trim();
            let offset = if (value.len() == 5 || value.len() == 7)
                && matches!(value.as_bytes()[0], b'+' | b'-')
                && value[1..].bytes().all(|b| b.is_ascii_digit())
            {
                let h: i32 = value[1..3].parse().ok()?;
                let m: i32 = value[3..5].parse().ok()?;
                let s: i32 = value.get(5..7).unwrap_or("00").parse().ok()?;
                if h < 24 && m < 60 && s < 60 {
                    chrono::FixedOffset::east_opt(
                        (h * 3600 + m * 60 + s) * if value.starts_with('-') { -1 } else { 1 },
                    )
                } else {
                    None
                }
            } else {
                None
            };
            if name == "TZOFFSETFROM" {
                from_count += 1;
                from = offset;
            } else {
                to_count += 1;
                to = offset;
            }
        }
    }
    if inside {
        None
    } else {
        Some(zones)
    }
}

/// Parse with an embedded, unambiguous fixed-zone definition when the TZID
/// is not an IANA zone. Other cases preserve the shared parser's semantics.
pub(super) fn parse_ics_time_with_fixed_zones(
    value: &str,
    params: &[String],
    zones: &std::collections::HashMap<String, chrono::FixedOffset>,
) -> Option<(i64, bool)> {
    let raw = value.trim();
    let all_day = params.iter().any(|p| p.eq_ignore_ascii_case("VALUE=DATE"))
        || (raw.len() == 8 && !raw.contains('T'));
    let tzid = params
        .iter()
        .find_map(|p| p.strip_prefix("TZID=").or_else(|| p.strip_prefix("tzid=")));
    if !all_day && !raw.ends_with('Z') {
        if let Some(id) = tzid.map(|id| id.trim_matches('"')) {
            if id.parse::<chrono_tz::Tz>().is_err() {
                if let Some(offset) = zones.get(id) {
                    use chrono::TimeZone;
                    let naive = chrono::NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%S").ok()?;
                    return offset
                        .from_local_datetime(&naive)
                        .single()
                        .map(|t| (t.timestamp(), false));
                }
            }
        }
    }
    parse_ics_time(value, params)
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

#[cfg(test)]
mod fixed_zone_tests {
    use super::*;
    fn zone(from: &str, to: &str, extra: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Example Fixed Time\r\nBEGIN:STANDARD\r\nTZOFFSETFROM:{from}\r\nTZOFFSETTO:{to}\r\nDTSTART:16010101T000000\r\n{extra}END:STANDARD\r\nEND:VTIMEZONE\r\nEND:VCALENDAR\r\n")
    }
    #[test]
    fn embedded_fixed_offset_resolves_non_iana_wall_clock() {
        let zones = fixed_ics_zones(&zone("+0200", "+0200", "")).unwrap();
        let (t, all_day) = parse_ics_time_with_fixed_zones(
            "20261023T103000",
            &["TZID=\"Example Fixed Time\"".into()],
            &zones,
        )
        .unwrap();
        assert_eq!(
            chrono::DateTime::from_timestamp(t, 0).unwrap().to_rfc3339(),
            "2026-10-23T08:30:00+00:00"
        );
        assert!(!all_day);
    }
    #[test]
    fn negative_offset_including_seconds_preserved() {
        let zones = fixed_ics_zones(&zone("-033030", "-033030", "")).unwrap();
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
            assert!(fixed_ics_zones(&input).unwrap().is_empty());
        }
    }
    #[test]
    fn invalid_offsets_are_not_used() {
        for offset in ["+2500", "+0260", "-006099", "0200", "+éé"] {
            assert!(fixed_ics_zones(&zone(offset, offset, ""))
                .unwrap()
                .is_empty());
        }
    }
    #[test]
    fn duplicate_identity_is_ambiguous_even_if_offsets_match() {
        let input = zone("+0200", "+0200", "");
        assert!(fixed_ics_zones(&(input.clone() + &input)).is_none());
        assert!(fixed_ics_zones(&input.replace(
            "TZID:Example Fixed Time",
            "TZID:Example Fixed Time\r\nTZID:Another Name"
        ))
        .is_none());
    }
    #[test]
    fn unclosed_or_nested_zone_is_not_accepted() {
        let input = zone("+0200", "+0200", "");
        assert!(fixed_ics_zones(&input.replace("END:VTIMEZONE\r\n", "")).is_none());
        assert!(fixed_ics_zones(
            &input.replace("BEGIN:STANDARD", "BEGIN:VTIMEZONE\r\nBEGIN:STANDARD")
        )
        .is_none());
    }
    #[test]
    fn repeated_offsets_or_observances_are_not_guessed() {
        let input = zone("+0200", "+0200", "TZOFFSETFROM:+0200\r\n");
        assert!(fixed_ics_zones(&input).unwrap().is_empty());
        let input = zone("+0200", "+0200", "").replace("END:VTIMEZONE", "BEGIN:STANDARD\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0200\r\nEND:STANDARD\r\nEND:VTIMEZONE");
        assert!(fixed_ics_zones(&input).unwrap().is_empty());
    }
    #[test]
    fn iana_dst_rules_take_precedence_over_fixed_name_redefinition() {
        let mut zones = std::collections::HashMap::new();
        zones.insert(
            "Europe/Zurich".into(),
            chrono::FixedOffset::east_opt(0).unwrap(),
        );
        let (t, _) = parse_ics_time_with_fixed_zones(
            "20261026T103000",
            &["TZID=Europe/Zurich".into()],
            &zones,
        )
        .unwrap();
        assert_eq!(
            chrono::DateTime::from_timestamp(t, 0).unwrap().to_rfc3339(),
            "2026-10-26T09:30:00+00:00"
        );
    }
    #[test]
    fn utc_and_date_only_keep_existing_semantics() {
        let zones = fixed_ics_zones(&zone("+0200", "+0200", "")).unwrap();
        for (value, params) in [
            ("20261023T103000Z", vec!["TZID=Example Fixed Time".into()]),
            (
                "20261023",
                vec!["VALUE=DATE".into(), "TZID=Example Fixed Time".into()],
            ),
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
            parse_ics_time_with_fixed_zones(
                "20261023T103000",
                &params,
                &std::collections::HashMap::new()
            ),
            parse_ics_time("20261023T103000", &params)
        );
    }
}
