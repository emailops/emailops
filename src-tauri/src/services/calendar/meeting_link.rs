//! Pure extraction of a joinable meeting URL from a calendar event.
//!
//! Priority order:
//! 1. Structured conference URLs from the provider (Google `conferenceData`
//!    entry points, Graph `onlineMeeting.joinUrl`) — authoritative, accepted
//!    even for hosts we don't recognize (platform becomes `"other"`).
//! 2. Fallback text scan over location, then description: only URLs whose
//!    host belongs to a known meeting platform are accepted, so an arbitrary
//!    link in an event body is never mistaken for a join link.
//!
//! https-only by contract (WebView safety guardrail): `http:` and any other
//! scheme are rejected everywhere.

/// A joinable meeting URL plus the platform it was classified as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingLink {
    pub url: String,
    /// "meet" | "teams" | "webex" | "zoom" | "gotomeeting" | "jitsi" | "other"
    pub platform: &'static str,
}

/// Extract the best meeting link for an event. `structured_urls` come from the
/// provider's structured conference fields in provider priority order.
pub fn extract_meeting_link(structured_urls: &[String], location: &str, description: &str) -> Option<MeetingLink> {
    for raw in structured_urls {
        if let Some(url) = clean_https_url(raw.trim()) {
            let platform = classify_platform(&url).unwrap_or("other");
            return Some(MeetingLink { url, platform });
        }
    }
    [location, description].iter().find_map(|text| scan_text(text))
}

/// Find the first https URL in free text whose host is a known meeting
/// platform. Unknown hosts are skipped (not rejected outright) so a document
/// link before the join link doesn't mask it.
fn scan_text(text: &str) -> Option<MeetingLink> {
    let mut search_from = 0;
    while let Some(pos) = text[search_from..].find("https://") {
        let start = search_from + pos;
        let rest = &text[start..];
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\''))
            .unwrap_or(rest.len());
        if let Some(url) = clean_https_url(&rest[..end]) {
            if let Some(platform) = classify_platform(&url) {
                return Some(MeetingLink { url, platform });
            }
        }
        search_from = start + "https://".len();
    }
    None
}

/// Validate a candidate as an https URL, trimming wrapping brackets and
/// trailing prose punctuation. Returns the cleaned URL string.
fn clean_https_url(candidate: &str) -> Option<String> {
    let candidate = candidate.trim().trim_start_matches('<').trim_end_matches('>');
    let candidate = candidate.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}', '\'', '"']);
    let parsed = url::Url::parse(candidate).ok()?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return None;
    }
    Some(candidate.to_string())
}

/// Classify a URL's host as a known meeting platform.
fn classify_platform(url: &str) -> Option<&'static str> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let host_is = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    if host == "meet.google.com" {
        Some("meet")
    } else if host == "teams.microsoft.com" || host == "teams.live.com" {
        Some("teams")
    } else if host_is("webex.com") {
        Some("webex")
    } else if host_is("zoom.us") || host_is("zoom.com") {
        Some("zoom")
    } else if host_is("gotomeeting.com") || host_is("gotomeet.me") {
        Some("gotomeeting")
    } else if host == "meet.jit.si" {
        Some("jitsi")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Vec<String> {
        Vec::new()
    }

    // ── structured URLs ────────────────────────────────────────────────────

    #[test]
    fn structured_url_wins_over_text_fallback() {
        let link = extract_meeting_link(
            &["https://meet.google.com/abc-defg-hij".to_string()],
            "https://example.zoom.us/j/123456789",
            "",
        )
        .expect("structured link");
        assert_eq!(link.url, "https://meet.google.com/abc-defg-hij");
        assert_eq!(link.platform, "meet");
    }

    #[test]
    fn structured_url_with_unknown_host_is_accepted_as_other() {
        // Google conferenceData can point at third-party conferencing add-ons;
        // the provider says it's the join link, so trust it.
        let link = extract_meeting_link(&["https://conf.example-videocalls.io/room/42".to_string()], "", "")
            .expect("structured link");
        assert_eq!(link.platform, "other");
    }

    #[test]
    fn structured_non_https_url_is_rejected() {
        assert_eq!(
            extract_meeting_link(&["http://meet.google.com/abc-defg-hij".to_string()], "", ""),
            None
        );
    }

    #[test]
    fn first_valid_structured_url_wins() {
        let link = extract_meeting_link(
            &[
                "not a url at all".to_string(),
                "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0".to_string(),
            ],
            "",
            "",
        )
        .expect("second entry is valid");
        assert_eq!(link.platform, "teams");
    }

    // ── platform classification via text fallback ──────────────────────────

    #[test]
    fn extracts_google_meet_from_location() {
        let link = extract_meeting_link(&none(), "meet.google.com: https://meet.google.com/abc-defg-hij", "")
            .expect("meet link");
        assert_eq!(link.url, "https://meet.google.com/abc-defg-hij");
        assert_eq!(link.platform, "meet");
    }

    #[test]
    fn extracts_teams_link_from_description() {
        let description = "Join the call here:\nhttps://teams.microsoft.com/l/meetup-join/19%3ameeting_NzY4%40thread.v2/0?context=%7b%22Tid%22%3a%22x%22%7d\nAgenda: quarterly review";
        let link = extract_meeting_link(&none(), "", description).expect("teams link");
        assert!(link.url.starts_with("https://teams.microsoft.com/l/meetup-join/"));
        assert_eq!(link.platform, "teams");
    }

    #[test]
    fn extracts_teams_live_link() {
        let link = extract_meeting_link(&none(), "", "https://teams.live.com/meet/9312345678901?p=abc")
            .expect("teams.live link");
        assert_eq!(link.platform, "teams");
    }

    #[test]
    fn extracts_webex_link_from_subdomain_host() {
        let link = extract_meeting_link(
            &none(),
            "",
            "Meeting link: https://acme.webex.com/acme/j.php?MTID=m1234567890abcdef",
        )
        .expect("webex link");
        assert_eq!(link.platform, "webex");
    }

    #[test]
    fn extracts_zoom_link() {
        let link =
            extract_meeting_link(&none(), "https://us02web.zoom.us/j/1234567890?pwd=abc123", "").expect("zoom link");
        assert_eq!(link.platform, "zoom");
    }

    #[test]
    fn extracts_gotomeeting_link() {
        let link = extract_meeting_link(&none(), "", "https://global.gotomeeting.com/join/123456789")
            .expect("gotomeeting link");
        assert_eq!(link.platform, "gotomeeting");
    }

    #[test]
    fn extracts_jitsi_link() {
        let link = extract_meeting_link(&none(), "", "https://meet.jit.si/TeamWeekly42").expect("jitsi link");
        assert_eq!(link.platform, "jitsi");
    }

    // ── fallback precedence and false-positive resistance ──────────────────

    #[test]
    fn location_wins_over_description() {
        let link = extract_meeting_link(
            &none(),
            "https://meet.google.com/aaa-aaaa-aaa",
            "old link: https://example.zoom.us/j/999",
        )
        .expect("location link");
        assert_eq!(link.platform, "meet");
    }

    #[test]
    fn unknown_hosts_in_text_are_ignored() {
        // A newsletter link or company site in the description is NOT a meeting.
        assert_eq!(
            extract_meeting_link(
                &none(),
                "",
                "Read the brief at https://example.com/brief before the call"
            ),
            None
        );
    }

    #[test]
    fn http_link_in_text_is_ignored() {
        assert_eq!(
            extract_meeting_link(&none(), "", "http://meet.google.com/abc-defg-hij"),
            None
        );
    }

    #[test]
    fn returns_none_when_nothing_matches() {
        assert_eq!(
            extract_meeting_link(&none(), "Conference room 4B", "Bring the Q3 numbers"),
            None
        );
    }

    #[test]
    fn skips_unknown_url_and_takes_later_known_one() {
        let description = "Docs: https://docs.example.com/agenda — join: https://acme.webex.com/meet/jdoe";
        let link = extract_meeting_link(&none(), "", description).expect("webex link");
        assert_eq!(link.platform, "webex");
    }

    // ── URL boundary handling ──────────────────────────────────────────────

    #[test]
    fn trims_angle_brackets_around_url() {
        let link = extract_meeting_link(&none(), "", "Join: <https://meet.google.com/abc-defg-hij>").expect("link");
        assert_eq!(link.url, "https://meet.google.com/abc-defg-hij");
    }

    #[test]
    fn trims_trailing_punctuation() {
        let link = extract_meeting_link(&none(), "", "Join https://meet.jit.si/Weekly42.").expect("link");
        assert_eq!(link.url, "https://meet.jit.si/Weekly42");
    }

    #[test]
    fn keeps_query_parameters() {
        let link = extract_meeting_link(&none(), "", "https://us02web.zoom.us/j/123?pwd=x5T9(secure)").expect("link");
        assert!(
            link.url.contains("pwd=x5T9"),
            "query string must survive extraction, got {}",
            link.url
        );
    }

    #[test]
    fn html_description_with_href_still_extracts() {
        // Graph bodies are HTML; the URL appears inside attribute quotes.
        let description = r#"<a href="https://teams.microsoft.com/l/meetup-join/19%3am%40thread.v2/0">Join</a>"#;
        let link = extract_meeting_link(&none(), "", description).expect("teams link");
        assert_eq!(link.platform, "teams");
        assert!(
            !link.url.contains('"'),
            "quote must not leak into the URL: {}",
            link.url
        );
    }
}
