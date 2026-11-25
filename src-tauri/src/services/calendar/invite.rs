//! Calendar-invite handling: detect an iCalendar (.ics / text/calendar)
//! attachment on an email, parse the VEVENT into a displayable card, and RSVP
//! through the account's calendar provider.
//!
//! The ICS parser is pure and deliberately minimal — enough for the invite
//! card (UID, times, summary, organizer, location, recurrence text), not a
//! general iCalendar implementation. Invites are UNTRUSTED input: parsing is
//! purely structural and the output is rendered as plain text.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::ics::{parse_ics_time_with_fixed_zones, parse_vcalendar, unescape_text, RawVEvent};
use crate::db::Database;
use crate::models::error::{AppError, Result};

/// A parsed calendar invite, ready for the email-view card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarInvite {
    /// iCalendar UID — the cross-system event identity used to RSVP.
    pub uid: String,
    pub summary: String,
    pub location: String,
    /// Organizer email (from `ORGANIZER:mailto:`), lowercased.
    pub organizer: String,
    /// UTC epoch seconds.
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: bool,
    /// Raw RRULE value for display (e.g. "FREQ=WEEKLY;BYDAY=TU"); `None` for
    /// one-off events.
    pub recurrence: Option<String>,
    /// iTIP method: "REQUEST" (invitation), "CANCEL", … Defaults to "REQUEST".
    pub method: String,
    /// A broadcast (`METHOD:PUBLISH`) or an event without ORGANIZER: nobody to answer.
    pub is_publish: bool,
    /// Where an answer goes: the organizer, unless this is a publish or I am the organizer.
    pub reply_to: Option<String>,
    /// The organizer sent this very email.
    pub organizer_is_sender: bool,
}

/// Parse the first VEVENT of an iCalendar document. Returns `None` when there
/// is no usable event (missing UID or DTSTART).
pub fn parse_ics_invite(ics: &str) -> Option<CalendarInvite> {
    let calendar = parse_vcalendar(ics);
    let event = calendar.events.first()?;
    let method = calendar.method.clone().unwrap_or_else(|| "REQUEST".to_string());
    let mut uid = None;
    let mut summary = String::new();
    let mut location = String::new();
    let organizer = organizer_address(event).unwrap_or_default();
    let mut start: Option<(i64, bool)> = None;
    let mut end: Option<(i64, bool)> = None;
    let mut recurrence = None;

    for prop in &event.props {
        let value = &prop.value;
        match prop.name.as_str() {
            "UID" => uid = Some(value.trim().to_string()),
            "SUMMARY" => summary = unescape_text(value.trim()),
            "LOCATION" => location = unescape_text(value.trim()),
            "DTSTART" => start = parse_ics_time_with_fixed_zones(value, &prop.params, &calendar.fixed_zones),
            "DTEND" => end = parse_ics_time_with_fixed_zones(value, &prop.params, &calendar.fixed_zones),
            "RRULE" => recurrence = Some(value.trim().to_string()),
            _ => {}
        }
    }

    let uid = uid?;
    let (start_time, is_all_day) = start?;
    let end_time = end.map(|(t, _)| t).unwrap_or(start_time + 3_600);
    let is_publish = is_publish_invite(&calendar);
    Some(CalendarInvite {
        uid,
        summary,
        location,
        start_time,
        end_time,
        is_all_day,
        recurrence,
        is_publish,
        reply_to: if is_publish { None } else { reply_target(event, "") },
        organizer,
        method,
        organizer_is_sender: false,
    })
}

/// The ORGANIZER address of `event`: `mailto:` stripped case-insensitively,
/// lowercased; `None` when absent or empty.
pub(crate) fn organizer_address(event: &RawVEvent) -> Option<String> {
    let value = event.first_value("ORGANIZER")?.trim();
    let address = match value.get(..7) {
        Some(prefix) if prefix.eq_ignore_ascii_case("mailto:") => &value[7..],
        _ => value,
    };
    let address = address.trim().to_ascii_lowercase();
    (!address.is_empty()).then_some(address)
}

/// A broadcast: `METHOD:PUBLISH`, or a first event with no ORGANIZER. It
/// expects no answer (RFC 5546 defines no REPLY to PUBLISH).
pub(crate) fn is_publish_invite(cal: &super::ics::ParsedCalendar) -> bool {
    cal.method
        .as_deref()
        .is_some_and(|m| m.trim().eq_ignore_ascii_case("PUBLISH"))
        || cal.events.first().and_then(organizer_address).is_none()
}

/// Where an answer to `event` goes: its organizer, unless there is none, it is
/// the account itself, or it is not a usable email address.
pub(crate) fn reply_target(event: &RawVEvent, account_email: &str) -> Option<String> {
    organizer_address(event).filter(|o| !o.eq_ignore_ascii_case(account_email) && super::create::looks_like_email(o))
}

/// Fill the fields that depend on who sent the email and who reads it.
pub(crate) fn address_invite(invite: &mut CalendarInvite, event: &RawVEvent, sender_email: &str, my_email: &str) {
    invite.reply_to = if invite.is_publish {
        None
    } else {
        reply_target(event, my_email)
    };
    invite.organizer_is_sender = invite
        .reply_to
        .as_deref()
        .is_some_and(|reply_to| reply_to.eq_ignore_ascii_case(sender_email));
}

/// Whether an attachment looks like a calendar invite part.
pub(crate) fn is_invite_attachment(meta: &crate::models::EmailAttachmentMeta) -> bool {
    meta.mime_type.to_ascii_lowercase().contains("calendar") || meta.filename.to_ascii_lowercase().ends_with(".ics")
}

/// Find and parse the calendar invite attached to an email, as seen by the
/// account that received it.
pub async fn get_calendar_invite(
    db: &Arc<Database>,
    app_data_dir: &Path,
    email_id: &str,
) -> Result<Option<CalendarInvite>> {
    let email = load_email(db, email_id)?;
    let Some(ics) = read_invite_ics(db, app_data_dir, &email).await? else {
        return Ok(None);
    };
    let calendar = parse_vcalendar(&ics);
    let (Some(event), Some(mut invite)) = (calendar.events.first(), parse_ics_invite(&ics)) else {
        return Ok(None);
    };
    let account = db
        .get_account(&email.account_id)?
        .ok_or_else(|| AppError::NotFound(format!("account '{}' not found", email.account_id)))?;
    address_invite(&mut invite, event, &email.sender_email, &account.email);
    Ok(Some(invite))
}

/// The whole `.ics` document attached to an email, fetched from disk, inline
/// data, or the mail provider (in that order). `None` without an invite part.
pub async fn fetch_invite_ics(db: &Arc<Database>, app_data_dir: &Path, email_id: &str) -> Result<Option<String>> {
    let email = load_email(db, email_id)?;
    read_invite_ics(db, app_data_dir, &email).await
}

fn load_email(db: &Database, email_id: &str) -> Result<crate::models::Email> {
    db.get_email_by_id(email_id)?
        .ok_or_else(|| AppError::NotFound(format!("email '{email_id}' not found")))
}

async fn read_invite_ics(
    db: &Arc<Database>,
    app_data_dir: &Path,
    email: &crate::models::Email,
) -> Result<Option<String>> {
    let email_id = email.id.as_str();
    let metas = db.get_email_attachment_metas(email_id)?;
    let Some(meta) = metas.into_iter().find(is_invite_attachment) else {
        return Ok(None);
    };

    let ics = meta
        .file_path
        .as_deref()
        .and_then(|path| read_stored_invite(app_data_dir, path));
    let ics = match ics {
        Some(content) => content,
        None => {
            if let Some(inline_b64) = db.get_attachment_inline_data(email_id, &meta.filename)? {
                decode_ics_base64(&inline_b64)?
            } else {
                let account = db
                    .get_account(&email.account_id)?
                    .ok_or_else(|| AppError::NotFound(format!("account '{}' not found", email.account_id)))?;
                let provider = crate::services::emails::build_provider(&account, None).await?;
                let bytes = provider
                    .fetch_attachment_bytes(email_id, &meta.provider_attachment_id)
                    .await?;
                String::from_utf8_lossy(&bytes).into_owned()
            }
        }
    };
    Ok(Some(ics))
}

/// Read a locally stored invite part. `relative_path` is relative to the data
/// dir (as every stored attachment path is) and must resolve inside it.
/// Failures are logged and return `None` so the caller falls back to inline
/// data or the provider.
fn read_stored_invite(app_data_dir: &Path, relative_path: &str) -> Option<String> {
    let read = crate::services::attachments::safe_attachment_path(app_data_dir, relative_path).and_then(|path| {
        std::fs::read_to_string(&path)
            .map_err(|e| AppError::IoError(format!("Failed to read stored invite '{relative_path}': {e}")))
    });
    match read {
        Ok(content) => Some(content),
        Err(e) => {
            // A missing file just means it was never downloaded (or was
            // cleaned up); the provider fallback covers it.
            let level = if matches!(e, AppError::NotFound(_)) {
                "debug"
            } else {
                "error"
            };
            crate::services::logger::log(
                level,
                "system",
                format!("Stored calendar invite unavailable, fetching it again: {e}"),
            );
            None
        }
    }
}

fn decode_ics_base64(b64: &str) -> Result<String> {
    use base64::Engine;
    let cleaned: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&cleaned)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&cleaned))
        .map_err(|e| AppError::IoError(format!("invalid inline attachment data: {e}")))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::calendar::ics::parse_vcalendar;

    #[test]
    fn embedded_fixed_timezone_is_used_for_invite_display() {
        let ics="BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Example Fixed Time\r\nBEGIN:STANDARD\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0200\r\nDTSTART:16010101T000000\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:synthetic@example.test\r\nDTSTART;TZID=\"Example Fixed Time\":20261023T103000\r\nDTEND;TZID=\"Example Fixed Time\":20261023T113000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let invite = parse_ics_invite(ics).expect("fixed-offset invite");
        assert_eq!(
            chrono::DateTime::from_timestamp(invite.start_time, 0)
                .unwrap()
                .to_rfc3339(),
            "2026-10-23T08:30:00+00:00"
        );
        assert_eq!(invite.end_time - invite.start_time, 3600);
    }

    #[test]
    fn duplicate_or_malformed_timezones_do_not_hide_an_invite() {
        let zone = "BEGIN:VTIMEZONE\nTZID:Custom\nBEGIN:STANDARD\nTZOFFSETFROM:+0200\nTZOFFSETTO:+0200\nDTSTART:16010101T000000\nEND:STANDARD\nEND:VTIMEZONE\n";
        for definitions in [
            format!("{zone}{zone}"),
            zone.replace("TZOFFSETTO:+0200", "TZOFFSETTO:invalid"),
            zone.replace("END:VTIMEZONE\n", ""),
        ] {
            let ics = format!("BEGIN:VCALENDAR\n{definitions}BEGIN:VEVENT\nUID:retained@example.test\nDTSTART;TZID=Custom:20261023T103000\nEND:VEVENT\nEND:VCALENDAR\n");
            let invite = parse_ics_invite(&ics).expect("event survives unusable timezone metadata");
            assert_eq!(invite.uid, "retained@example.test");
            assert_eq!(
                chrono::DateTime::from_timestamp(invite.start_time, 0)
                    .unwrap()
                    .to_rfc3339(),
                "2026-10-23T10:30:00+00:00"
            );
        }
    }

    #[test]
    fn malformed_timezone_does_not_disable_iana_or_utc_invites() {
        for start in ["DTSTART;TZID=Europe/Madrid:20261023T103000", "DTSTART:20261023T083000Z"] {
            let ics = format!("BEGIN:VCALENDAR\nBEGIN:VTIMEZONE\nTZID:Bad\nEND:VTIMEZONE\nBEGIN:VEVENT\nUID:retained@example.test\n{start}\nEND:VEVENT\nEND:VCALENDAR\n");
            let invite = parse_ics_invite(&ics).expect("unrelated broken timezone does not reject event");
            assert_eq!(
                chrono::DateTime::from_timestamp(invite.start_time, 0)
                    .unwrap()
                    .to_rfc3339(),
                "2026-10-23T08:30:00+00:00"
            );
        }
    }

    #[test]
    fn shared_event_parser_preserves_method_and_skips_nested_alarm_fields() {
        let ics = "BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nUID:event@example.test\r\nDTSTART:20261023T083000Z\r\nSUMMARY:Folded \r\n summary\r\nBEGIN:VALARM\r\nUID:alarm@example.test\r\nSUMMARY:Alarm text\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let invite = parse_ics_invite(ics).expect("first event");
        assert_eq!(invite.uid, "event@example.test");
        assert_eq!(invite.summary, "Folded summary");
        assert_eq!(invite.method, "CANCEL");
    }

    /// The invite shares the feed's parser, which keeps only events that end:
    /// a truncated attachment shows no card rather than a partial one.
    #[test]
    fn an_invite_whose_event_never_ends_shows_no_card() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:truncated@example.test\r\nDTSTART:20261023T083000Z\r\nSUMMARY:Partial\r\nEND:VCALENDAR\r\n";
        assert_eq!(parse_ics_invite(ics), None);
    }

    const SAMPLE_INVITE: &str = "BEGIN:VCALENDAR\r\nPRODID:-//Test//EN\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nDTSTART;TZID=Europe/Madrid:20260728T073000\r\nDTEND;TZID=Europe/Madrid:20260728T083000\r\nRRULE:FREQ=WEEKLY;BYDAY=TU\r\nDTSTAMP:20260723T103000Z\r\nORGANIZER;CN=Organizer:mailto:Organizer@Example.com\r\nUID:abc123DEF@example.com\r\nSUMMARY:Team sync\\, weekly\r\nLOCATION:Room 4\\; floor 2\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    /// Seed an IMAP account + email whose `.ics` part is recorded as stored at
    /// the data-dir-relative `relative` path. The IMAP account has no
    /// credentials, so any fall-through to the provider fails.
    fn seed_stored_invite(db: &Database, relative: &str) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
                 VALUES ('acc-inv', 'imap', 'me@example.com', 'Test', 0, 0, 1)",
                [],
            )
            .expect("insert account");
        db.insert_email(&crate::models::Email {
            id: "msg-inv".into(),
            account_id: "acc-inv".into(),
            thread_id: "msg-inv".into(),
            message_id: None,
            references: None,
            subject: "Invitation".into(),
            sender: "Organizer".into(),
            sender_email: "organizer@example.com".into(),
            recipients: vec!["me@example.com".into()],
            cc: vec![],
            body: "body".into(),
            snippet: "snippet".into(),
            timestamp: 1_700_000_000,
            is_read: false,
            triage_status: None,
            category: "primary".into(),
            mailbox: "inbox".into(),
            is_sent: false,
            is_starred: false,
            size_bytes: None,
            has_attachments: None,
            headers: None,
        })
        .expect("insert email");
        db.insert_email_attachment_meta("msg-inv", "acc-inv", "att-1", "invite.ics", "text/calendar", 10)
            .expect("insert meta");
        db.set_email_attachment_file_path("msg-inv", "invite.ics", relative)
            .expect("set file path");
    }

    const TWO_EVENTS: &str = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:first@example.com\r\nDTSTART:20261201T100000Z\r\nDTEND:20261201T110000Z\r\nORGANIZER:mailto:lead@example.com\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:second@example.com\r\nDTSTART:20261202T100000Z\r\nDTEND:20261202T110000Z\r\nORGANIZER:mailto:lead@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn seeded_two_event_db() -> (Arc<Database>, tempfile::TempDir) {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let data_dir = tempfile::tempdir().expect("tmp dir");
        let relative = "attachments/acc-inv/auto/invite.ics";
        let absolute = data_dir.path().join(relative);
        std::fs::create_dir_all(absolute.parent().expect("parent")).expect("mkdir");
        std::fs::write(&absolute, TWO_EVENTS).expect("write ics");
        seed_stored_invite(&db, relative);
        (db, data_dir)
    }

    #[tokio::test]
    async fn fetch_returns_the_whole_document() {
        let (db, data_dir) = seeded_two_event_db();
        let ics = fetch_invite_ics(&db, data_dir.path(), "msg-inv")
            .await
            .expect("fetch")
            .expect("invite part");
        assert_eq!(ics.matches("BEGIN:VEVENT").count(), 2);
    }

    #[tokio::test]
    async fn fetch_returns_none_without_an_invite_part() {
        let (db, data_dir) = seeded_two_event_db();
        db.connection()
            .execute("DELETE FROM email_attachment_meta WHERE email_id = 'msg-inv'", [])
            .expect("drop part");
        assert_eq!(
            fetch_invite_ics(&db, data_dir.path(), "msg-inv").await.expect("fetch"),
            None
        );
    }

    fn first_event(ics: &str) -> RawVEvent {
        parse_vcalendar(ics).events.remove(0)
    }

    fn with_organizer(value: &str) -> String {
        TWO_EVENTS.replacen("ORGANIZER:mailto:lead@example.com", value, 1)
    }

    #[test]
    fn reply_target_is_the_normalized_organizer() {
        let event = first_event(&with_organizer("ORGANIZER;CN=Lead:MAILTO:Lead@Example.COM"));
        assert_eq!(
            reply_target(&event, "me@example.com").as_deref(),
            Some("lead@example.com")
        );
    }

    #[test]
    fn reply_target_is_none_for_nobody_to_answer() {
        let missing = with_organizer("X-NOTE:none");
        let invalid = with_organizer("ORGANIZER:mailto:lead@intranet");
        let me = with_organizer("ORGANIZER:mailto:ME@example.com");
        for ics in [missing, invalid, me] {
            assert_eq!(reply_target(&first_event(&ics), "me@example.com"), None, "{ics}");
        }
    }

    #[test]
    fn publish_predicate_covers_method_and_missing_organizer() {
        assert!(!is_publish_invite(&parse_vcalendar(TWO_EVENTS)));
        assert!(is_publish_invite(&parse_vcalendar(
            &TWO_EVENTS.replace("METHOD:REQUEST", "METHOD:publish")
        )));
        assert!(is_publish_invite(&parse_vcalendar(&with_organizer("X-NOTE:none"))));
    }

    #[test]
    fn the_card_agrees_with_the_service_on_an_invalid_organizer() {
        let ics = with_organizer("ORGANIZER:mailto:lead@intranet");
        let mut invite = parse_ics_invite(&ics).expect("parse");
        address_invite(&mut invite, &first_event(&ics), "lead@intranet", "me@example.com");
        assert_eq!(invite.reply_to, None);
        assert!(!invite.organizer_is_sender);
    }

    #[test]
    fn publish_invites_are_flagged() {
        let no_organizer = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:u1\nDTSTART:20261201T100000Z\nDTEND:20261201T110000Z\nEND:VEVENT\nEND:VCALENDAR\n";
        let publish = "BEGIN:VCALENDAR\nMETHOD:PUBLISH\nBEGIN:VEVENT\nUID:u1\nDTSTART:20261201T100000Z\nDTEND:20261201T110000Z\nORGANIZER:mailto:lead@example.com\nEND:VEVENT\nEND:VCALENDAR\n";
        for ics in [no_organizer, publish] {
            let invite = parse_ics_invite(ics).expect("parse");
            assert!(invite.is_publish, "{ics}");
            assert_eq!(invite.reply_to, None, "{ics}");
        }
        assert!(!parse_ics_invite(SAMPLE_INVITE).expect("parse").is_publish);
    }

    #[test]
    fn organizer_differs_from_sender_is_flagged() {
        let mut invite = parse_ics_invite(TWO_EVENTS).expect("parse");
        address_invite(
            &mut invite,
            &first_event(TWO_EVENTS),
            "news@example.org",
            "me@example.com",
        );
        assert_eq!(invite.reply_to.as_deref(), Some("lead@example.com"));
        assert!(!invite.organizer_is_sender);
    }

    #[test]
    fn organizer_who_sent_the_email_is_flagged_case_insensitively() {
        let mut invite = parse_ics_invite(TWO_EVENTS).expect("parse");
        address_invite(
            &mut invite,
            &first_event(TWO_EVENTS),
            "Lead@Example.com",
            "me@example.com",
        );
        assert!(invite.organizer_is_sender);
    }

    #[test]
    fn an_invite_organized_by_me_has_no_reply_address() {
        let mut invite = parse_ics_invite(TWO_EVENTS).expect("parse");
        address_invite(
            &mut invite,
            &first_event(TWO_EVENTS),
            "lead@example.com",
            "LEAD@example.com",
        );
        assert_eq!(invite.reply_to, None);
        assert!(!invite.organizer_is_sender);
    }

    // Regression: the stored path is relative to the data dir but was read
    // against the process CWD, so a locally stored invite was never found.
    #[tokio::test]
    async fn reads_a_locally_stored_invite_from_the_data_dir() {
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let data_dir = tempfile::tempdir().expect("tmp dir");
        let relative = "attachments/acc-inv/auto/invite.ics";
        let absolute = data_dir.path().join(relative);
        std::fs::create_dir_all(absolute.parent().expect("parent")).expect("mkdir");
        std::fs::write(&absolute, SAMPLE_INVITE).expect("write ics");
        seed_stored_invite(&db, relative);

        let invite = get_calendar_invite(&db, data_dir.path(), "msg-inv")
            .await
            .expect("stored invite must be read without the provider")
            .expect("invite");

        assert_eq!(invite.uid, "abc123DEF@example.com");
    }

    // The read error used to be swallowed by `.ok()`.
    #[test]
    fn a_stored_invite_that_cannot_be_read_is_logged() {
        let _seam = crate::services::events::seam_test_lock();
        let logs = crate::services::logger::install_for_testing();
        let data_dir = tempfile::tempdir().expect("tmp dir");

        let content = read_stored_invite(data_dir.path(), "attachments/acc-inv/auto/missing.ics");

        assert_eq!(content, None);
        assert!(
            logs.events().iter().any(|e| e.message.contains("missing.ics")),
            "the failed read must be logged, got {:?}",
            logs.events()
        );
    }

    #[test]
    fn a_stored_invite_path_outside_the_data_dir_is_not_read() {
        let _seam = crate::services::events::seam_test_lock();
        let _logs = crate::services::logger::install_for_testing();
        let root = tempfile::tempdir().expect("tmp dir");
        let data_dir = root.path().join("data");
        std::fs::create_dir_all(&data_dir).expect("mkdir");
        std::fs::write(root.path().join("outside.ics"), SAMPLE_INVITE).expect("write ics");

        assert_eq!(read_stored_invite(&data_dir, "../outside.ics"), None);
    }

    #[test]
    fn parses_a_google_style_request_invite() {
        let invite = parse_ics_invite(SAMPLE_INVITE).expect("parse");
        assert_eq!(invite.uid, "abc123DEF@example.com");
        assert_eq!(invite.summary, "Team sync, weekly");
        assert_eq!(invite.location, "Room 4; floor 2");
        assert_eq!(invite.organizer, "organizer@example.com");
        // 07:30 Madrid (CEST, +02:00) == 05:30Z on 2026-07-28.
        assert_eq!(invite.start_time, 1785216600);
        assert_eq!(invite.end_time - invite.start_time, 3_600);
        assert!(!invite.is_all_day);
        assert_eq!(invite.recurrence.as_deref(), Some("FREQ=WEEKLY;BYDAY=TU"));
        assert_eq!(invite.method, "REQUEST");
    }

    #[test]
    fn parses_utc_times_and_defaults_method_to_request() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:u1\nDTSTART:20260728T053000Z\nDTEND:20260728T063000Z\nSUMMARY:Call\nEND:VEVENT\nEND:VCALENDAR\n";
        let invite = parse_ics_invite(ics).expect("parse");
        assert_eq!(invite.start_time, 1785216600);
        assert_eq!(invite.method, "REQUEST");
        assert_eq!(invite.recurrence, None);
    }

    #[test]
    fn parses_all_day_value_date() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:u1\nDTSTART;VALUE=DATE:20260728\nDTEND;VALUE=DATE:20260729\nSUMMARY:Offsite\nEND:VEVENT\nEND:VCALENDAR\n";
        let invite = parse_ics_invite(ics).expect("parse");
        assert!(invite.is_all_day);
        assert_eq!(invite.end_time - invite.start_time, 86_400);
    }

    #[test]
    fn unfolds_continuation_lines() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u1\r\nDTSTART:20260728T053000Z\r\nSUMMARY:A very long su\r\n mmary that was folded\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let invite = parse_ics_invite(ics).expect("parse");
        assert_eq!(invite.summary, "A very long summary that was folded");
    }

    #[test]
    fn cancel_method_is_surfaced() {
        let ics = "BEGIN:VCALENDAR\nMETHOD:CANCEL\nBEGIN:VEVENT\nUID:u1\nDTSTART:20260728T053000Z\nEND:VEVENT\nEND:VCALENDAR\n";
        assert_eq!(parse_ics_invite(ics).expect("parse").method, "CANCEL");
    }

    #[test]
    fn missing_uid_or_start_is_rejected() {
        let no_uid = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART:20260728T053000Z\nEND:VEVENT\nEND:VCALENDAR\n";
        assert!(parse_ics_invite(no_uid).is_none());
        let no_start = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:u1\nSUMMARY:x\nEND:VEVENT\nEND:VCALENDAR\n";
        assert!(parse_ics_invite(no_start).is_none());
    }

    #[test]
    fn missing_dtend_defaults_to_one_hour() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:u1\nDTSTART:20260728T053000Z\nEND:VEVENT\nEND:VCALENDAR\n";
        let invite = parse_ics_invite(ics).expect("parse");
        assert_eq!(invite.end_time - invite.start_time, 3_600);
    }

    #[test]
    fn param_with_quoted_colon_does_not_break_the_split() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:u1\nDTSTART:20260728T053000Z\nORGANIZER;CN=\"Boss: the one\":mailto:boss@example.com\nEND:VEVENT\nEND:VCALENDAR\n";
        let invite = parse_ics_invite(ics).expect("parse");
        assert_eq!(invite.organizer, "boss@example.com");
    }

    #[test]
    fn unknown_tzid_falls_back_to_utc_instead_of_failing() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:u1\nDTSTART;TZID=Not/AZone:20260728T053000\nEND:VEVENT\nEND:VCALENDAR\n";
        let invite = parse_ics_invite(ics).expect("parse");
        assert_eq!(invite.start_time, 1785216600);
    }
}
