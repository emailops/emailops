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
}

/// Unfold RFC 5545 folded lines (CRLF followed by space/tab continues the line).
fn unfold_ics_lines(ics: &str) -> Vec<String> {
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
fn split_content_line(line: &str) -> Option<(String, Vec<String>, String)> {
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

/// Unescape RFC 5545 TEXT values (`\\n` → newline, `\\,` `\\;` `\\\\` literal).
fn unescape_text(value: &str) -> String {
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
fn parse_ics_time(value: &str, params: &[String]) -> Option<(i64, bool)> {
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

/// Parse the first VEVENT of an iCalendar document. Returns `None` when there
/// is no usable event (missing UID or DTSTART).
pub fn parse_ics_invite(ics: &str) -> Option<CalendarInvite> {
    let mut method = "REQUEST".to_string();
    let mut in_event = false;
    let mut uid = None;
    let mut summary = String::new();
    let mut location = String::new();
    let mut organizer = String::new();
    let mut start: Option<(i64, bool)> = None;
    let mut end: Option<(i64, bool)> = None;
    let mut recurrence = None;

    for line in unfold_ics_lines(ics) {
        let Some((name, params, value)) = split_content_line(&line) else {
            continue;
        };
        match name.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VEVENT") => in_event = true,
            "END" if value.eq_ignore_ascii_case("VEVENT") => break, // first VEVENT only
            "METHOD" if !in_event => method = value.trim().to_ascii_uppercase(),
            _ if !in_event => {}
            "UID" => uid = Some(value.trim().to_string()),
            "SUMMARY" => summary = unescape_text(value.trim()),
            "LOCATION" => location = unescape_text(value.trim()),
            "ORGANIZER" => {
                organizer = value
                    .trim()
                    .strip_prefix("mailto:")
                    .or_else(|| value.trim().strip_prefix("MAILTO:"))
                    .unwrap_or(value.trim())
                    .to_ascii_lowercase();
            }
            "DTSTART" => start = parse_ics_time(&value, &params),
            "DTEND" => end = parse_ics_time(&value, &params),
            "RRULE" => recurrence = Some(value.trim().to_string()),
            _ => {}
        }
    }

    let uid = uid?;
    let (start_time, is_all_day) = start?;
    let end_time = end.map(|(t, _)| t).unwrap_or(start_time + 3_600);
    Some(CalendarInvite {
        uid,
        summary,
        location,
        organizer,
        start_time,
        end_time,
        is_all_day,
        recurrence,
        method,
    })
}

/// Whether an attachment looks like a calendar invite part.
pub(crate) fn is_invite_attachment(meta: &crate::models::EmailAttachmentMeta) -> bool {
    meta.mime_type.to_ascii_lowercase().contains("calendar") || meta.filename.to_ascii_lowercase().ends_with(".ics")
}

/// Find and parse the calendar invite attached to an email, fetching the ICS
/// bytes from disk, inline data, or the mail provider (in that order).
pub async fn get_calendar_invite(
    db: &Arc<Database>,
    app_data_dir: &Path,
    email_id: &str,
) -> Result<Option<CalendarInvite>> {
    let Some(email) = db.get_email_by_id(email_id)? else {
        return Err(AppError::NotFound(format!("email '{email_id}' not found")));
    };
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
    Ok(parse_ics_invite(&ics))
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
            headers: None,
        })
        .expect("insert email");
        db.insert_email_attachment_meta("msg-inv", "acc-inv", "att-1", "invite.ics", "text/calendar", 10)
            .expect("insert meta");
        db.set_email_attachment_file_path("msg-inv", "invite.ics", relative)
            .expect("set file path");
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
