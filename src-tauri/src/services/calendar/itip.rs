//! iTIP REPLY (RFC 5546) for a calendar invitation: the `text/calendar;
//! method=REPLY` document the organizer's calendar reads to update our answer,
//! its localized subject, and the send through the account's mail provider.
//! The source event is UNTRUSTED ICS, so values are copied verbatim, never
//! interpreted.

use crate::models::error::{AppError, Result};
use crate::models::Account;
use crate::services::calendar::ics::{unescape_text, IcsProperty, RawVEvent};
use crate::services::i18n::Language;
use crate::sync::calendar_provider::RsvpResponse;
use crate::sync::mime_builder::build_itip_reply_message;
use crate::sync::provider::EmailProvider;

/// Longest event title kept in a reply subject.
const MAX_SUBJECT_TITLE_CHARS: usize = 120;
/// RFC 5545 section 3.1: content lines are folded at 75 octets.
const MAX_LINE_OCTETS: usize = 75;

fn partstat(response: RsvpResponse) -> &'static str {
    match response {
        RsvpResponse::Accepted => "ACCEPTED",
        RsvpResponse::Declined => "DECLINED",
        RsvpResponse::Tentative => "TENTATIVE",
    }
}

fn render_property(prop: &IcsProperty) -> String {
    let mut line = prop.name.clone();
    for param in &prop.params {
        line.push(';');
        line.push_str(param);
    }
    line.push(':');
    line.push_str(&prop.value);
    line
}

/// Fold one content line at 75 octets on UTF-8 char boundaries, joining the
/// pieces with CRLF + a single space (the space counts toward the 75).
fn fold_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / MAX_LINE_OCTETS * 3);
    let mut octets = 0;
    for c in line.chars() {
        if octets + c.len_utf8() > MAX_LINE_OCTETS {
            out.push_str("\r\n ");
            octets = 1;
        }
        out.push(c);
        octets += c.len_utf8();
    }
    out
}

/// The REPLY document for `event`, answering as `attendee_email` at `now`
/// (epoch seconds, UTC). `None` when the event has no ORGANIZER (there is
/// nobody to answer) or `attendee_email` is not an email address.
pub fn build_itip_reply(event: &RawVEvent, attendee_email: &str, response: RsvpResponse, now: i64) -> Option<String> {
    if !looks_like_email(attendee_email) {
        return None;
    }
    let organizer = event.first("ORGANIZER")?;
    let stamp = chrono::DateTime::from_timestamp(now, 0)?.format("%Y%m%dT%H%M%SZ");
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "PRODID:-//EmailOps//Calendar Reply//EN".to_string(),
        "VERSION:2.0".to_string(),
        "METHOD:REPLY".to_string(),
        "BEGIN:VEVENT".to_string(),
    ];
    if let Some(uid) = event.first("UID") {
        lines.push(render_property(uid));
    }
    lines.push(format!("SEQUENCE:{}", event.first_value("SEQUENCE").unwrap_or("0")));
    lines.push(format!("DTSTAMP:{stamp}"));
    for name in ["DTSTART", "RECURRENCE-ID"] {
        if let Some(prop) = event.first(name) {
            lines.push(render_property(prop));
        }
    }
    lines.push(render_property(organizer));
    if let Some(summary) = event.first("SUMMARY") {
        lines.push(render_property(summary));
    }
    lines.push(format!(
        "ATTENDEE;PARTSTAT={}:mailto:{attendee_email}",
        partstat(response)
    ));
    lines.push("END:VEVENT".to_string());
    lines.push("END:VCALENDAR".to_string());

    let mut out = String::new();
    for line in &lines {
        out.push_str(&fold_line(line));
        out.push_str("\r\n");
    }
    Some(out)
}

/// Subject of the reply mail: a localized verb plus the event title, one line
/// and at most 120 characters of title.
pub fn itip_reply_subject(response: RsvpResponse, summary: &str, language: Language) -> String {
    let prefix = match (language, response) {
        (Language::En, RsvpResponse::Accepted) => "Accepted: ",
        (Language::En, RsvpResponse::Declined) => "Declined: ",
        (Language::En, RsvpResponse::Tentative) => "Tentative: ",
        (Language::Es, RsvpResponse::Accepted) => "Aceptado: ",
        (Language::Es, RsvpResponse::Declined) => "Rechazado: ",
        (Language::Es, RsvpResponse::Tentative) => "Provisional: ",
        (Language::Fr, RsvpResponse::Accepted) => "Accepté : ",
        (Language::Fr, RsvpResponse::Declined) => "Refusé : ",
        (Language::Fr, RsvpResponse::Tentative) => "Provisoire : ",
        (Language::De, RsvpResponse::Accepted) => "Zugesagt: ",
        (Language::De, RsvpResponse::Declined) => "Abgelehnt: ",
        (Language::De, RsvpResponse::Tentative) => "Mit Vorbehalt: ",
    };
    let title: String = unescape_text(summary)
        .chars()
        .filter(|c| !matches!(c, '\r' | '\n'))
        .collect();
    let title: String = title.trim().chars().take(MAX_SUBJECT_TITLE_CHARS).collect();
    format!("{prefix}{title}")
}

fn looks_like_email(address: &str) -> bool {
    let mut parts = address.split('@');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(local), Some(domain), None) => {
            !local.is_empty() && domain.contains('.') && !address.chars().any(char::is_whitespace)
        }
        _ => false,
    }
}

/// Email `ics` (a REPLY) to the organizer, and only to them, as raw MIME
/// through the account's provider. The text part repeats the subject.
pub async fn send_itip_reply(
    provider: &dyn EmailProvider,
    account: &Account,
    organizer: &str,
    subject: &str,
    ics: &str,
) -> Result<()> {
    if !looks_like_email(organizer) {
        return Err(AppError::InvalidInput(
            "the organizer is not a valid email address".into(),
        ));
    }
    let from_name = (!account.name.is_empty()).then_some(account.name.as_str());
    let message = build_itip_reply_message(&account.email, from_name, organizer, subject, subject, ics)?;
    provider.send_raw_mime(&message.formatted()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Account;
    use crate::services::calendar::ics::parse_vcalendar;
    use crate::services::i18n::Language;
    use crate::sync::calendar_provider::RsvpResponse;
    use crate::sync::provider::FakeEmailProvider;

    fn event(extra: &str) -> crate::services::calendar::ics::RawVEvent {
        let ics = format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:abc-1@example.org\r\nSEQUENCE:3\r\n\
             DTSTART;TZID=Europe/Madrid:20261201T100000\r\nSUMMARY:Planning\r\n{extra}END:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        parse_vcalendar(&ics).events.remove(0)
    }

    const ORG: &str = "ORGANIZER;CN=Org Example:mailto:org@example.org\r\n";

    #[test]
    fn reply_has_method_reply_and_my_partstat() {
        let ev = event(ORG);
        let out = build_itip_reply(&ev, "me@example.com", RsvpResponse::Accepted, 1_796_119_200).expect("reply");
        assert!(out.contains("METHOD:REPLY\r\n"), "{out}");
        assert!(out.contains("PRODID:-//EmailOps//Calendar Reply//EN\r\n"), "{out}");
        assert!(out.contains("UID:abc-1@example.org\r\n"), "{out}");
        assert!(out.contains("SEQUENCE:3\r\n"), "{out}");
        assert!(
            out.contains("ORGANIZER;CN=Org Example:mailto:org@example.org\r\n"),
            "{out}"
        );
        assert!(out.contains("DTSTART;TZID=Europe/Madrid:20261201T100000\r\n"), "{out}");
        assert!(
            out.contains("ATTENDEE;PARTSTAT=ACCEPTED:mailto:me@example.com\r\n"),
            "{out}"
        );
        assert!(out.contains("DTSTAMP:20261201T100000Z\r\n"), "{out}");
        assert!(out.ends_with("END:VEVENT\r\nEND:VCALENDAR\r\n"), "{out}");
        assert!(!out.replace("\r\n", "").contains('\n'), "bare LF in {out:?}");
    }

    #[test]
    fn reply_uses_zero_sequence_when_absent() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u\r\nORGANIZER:mailto:o@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let ev = parse_vcalendar(ics).events.remove(0);
        let out = build_itip_reply(&ev, "me@example.com", RsvpResponse::Declined, 0).expect("reply");
        assert!(out.contains("SEQUENCE:0\r\n"), "{out}");
        assert!(out.contains("PARTSTAT=DECLINED"), "{out}");
    }

    #[test]
    fn reply_keeps_recurrence_id() {
        let ev = event(&format!("{ORG}RECURRENCE-ID;TZID=Europe/Madrid:20261208T100000\r\n"));
        let out = build_itip_reply(&ev, "me@example.com", RsvpResponse::Tentative, 0).expect("reply");
        assert!(
            out.contains("RECURRENCE-ID;TZID=Europe/Madrid:20261208T100000\r\n"),
            "{out}"
        );
        assert!(out.contains("PARTSTAT=TENTATIVE"), "{out}");
    }

    #[test]
    fn no_organizer_no_reply() {
        assert!(build_itip_reply(&event(""), "me@example.com", RsvpResponse::Accepted, 0).is_none());
    }

    #[test]
    fn long_multibyte_lines_are_folded_on_char_boundaries() {
        let summary = "Reunión ñandú ".repeat(30);
        let ics = format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u\r\n{ORG}SUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        let ev = parse_vcalendar(&ics).events.remove(0);
        let out = build_itip_reply(&ev, "me@example.com", RsvpResponse::Accepted, 0).expect("reply");
        for line in out.split("\r\n") {
            assert!(line.len() <= 75, "line of {} octets: {line}", line.len());
        }
        assert!(out.contains("\r\n "), "expected a fold in {out}");
        let unfolded = out.replace("\r\n ", "");
        assert!(unfolded.contains(&format!("SUMMARY:{summary}")), "{unfolded}");
    }

    #[test]
    fn subject_is_localized_and_sanitized() {
        let title = format!("Line one\r\nLine two {}", "x".repeat(200));
        let s = itip_reply_subject(RsvpResponse::Accepted, &title, Language::En);
        assert!(!s.contains('\r') && !s.contains('\n'));
        assert!(s.starts_with("Accepted: "));
        assert_eq!(s.chars().count(), "Accepted: ".len() + 120);

        let cases = [
            (Language::Es, RsvpResponse::Accepted, "Aceptado: T"),
            (Language::Es, RsvpResponse::Declined, "Rechazado: T"),
            (Language::Es, RsvpResponse::Tentative, "Provisional: T"),
            (Language::Fr, RsvpResponse::Accepted, "Accepté : T"),
            (Language::Fr, RsvpResponse::Declined, "Refusé : T"),
            (Language::Fr, RsvpResponse::Tentative, "Provisoire : T"),
            (Language::De, RsvpResponse::Accepted, "Zugesagt: T"),
            (Language::De, RsvpResponse::Declined, "Abgelehnt: T"),
            (Language::De, RsvpResponse::Tentative, "Mit Vorbehalt: T"),
            (Language::En, RsvpResponse::Declined, "Declined: T"),
            (Language::En, RsvpResponse::Tentative, "Tentative: T"),
        ];
        for (lang, resp, want) in cases {
            assert_eq!(itip_reply_subject(resp, "T", lang), want);
        }
    }

    #[test]
    fn subject_unescapes_and_truncates_by_chars() {
        let s = itip_reply_subject(
            RsvpResponse::Accepted,
            &format!("a\\, b {}", "ñ".repeat(200)),
            Language::En,
        );
        assert!(s.starts_with("Accepted: a, b ñ"), "{s}");
        assert_eq!(s.chars().count(), "Accepted: ".len() + 120);
    }

    fn account() -> Account {
        Account {
            id: "a1".into(),
            provider: "gmail".into(),
            email: "me@example.com".into(),
            name: "Me Example".into(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
            color: crate::models::AccountColor::Blue,
        }
    }

    #[tokio::test]
    async fn send_itip_reply_sends_raw_mime_to_the_organizer_only() {
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        send_itip_reply(
            &provider,
            &account(),
            "org@example.org",
            "Accepted: Planning",
            "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
        )
        .await
        .expect("sent");
        let raw = provider.raw_sent();
        assert_eq!(raw.len(), 1);
        let mime = String::from_utf8_lossy(&raw[0]).into_owned();
        assert!(mime.contains("To: org@example.org"), "{mime}");
        assert!(mime.contains("text/calendar; method=REPLY"), "{mime}");
        assert!(mime.contains("Me Example"), "{mime}");
    }

    #[tokio::test]
    async fn send_itip_reply_rejects_a_bad_organizer() {
        let provider = FakeEmailProvider::new("me@example.com", "Me");
        for bad in ["", "nobody", "a@b", "a b@example.org", "a@@example.org", "@example.org"] {
            let err = send_itip_reply(&provider, &account(), bad, "s", "ics")
                .await
                .expect_err(bad);
            assert!(
                matches!(err, crate::models::error::AppError::InvalidInput(_)),
                "{bad}: {err}"
            );
        }
        assert!(provider.raw_sent().is_empty());
    }

    #[test]
    fn folding_respects_the_75_octet_limit() {
        let exact = "a".repeat(75);
        assert_eq!(fold_line(&exact), exact, "75 octets are not folded");
        let over = "a".repeat(76);
        assert_eq!(fold_line(&over), format!("{}\r\n a", "a".repeat(75)));
        // A 2-octet char starting at octet 74 would end at 76: it moves whole.
        let straddle = format!("{}ñ", "a".repeat(74));
        assert_eq!(fold_line(&straddle), format!("{}\r\n ñ", "a".repeat(74)));
    }

    #[test]
    fn a_non_address_attendee_gets_no_reply() {
        let ev = event(ORG);
        for bad in ["", "me", "me@localhost", "m e@example.com"] {
            assert!(build_itip_reply(&ev, bad, RsvpResponse::Accepted, 0).is_none(), "{bad}");
        }
    }
}
