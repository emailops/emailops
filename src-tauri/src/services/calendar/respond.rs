//! Answering a calendar invitation from its email: look the event up in the
//! account's calendar, import it when it is missing (after the provider's own
//! delivery had time to run), and send the iTIP reply to the organizer.
//!
//! The reply goes only to the ORGANIZER, never to anyone else, and never when
//! importing a "yes"/"maybe" invitation failed.

use serde::Serialize;

use super::ics::{parse_vcalendar, RawVEvent};
use super::import::{plan_ics_import, reason_code, ImportItemStatus, ImportableEvent};
use super::invite::{is_publish_invite, reply_target};
use super::itip::{build_itip_reply, itip_reply_subject, send_itip_reply};
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{Account, CalendarAttendee};
use crate::services::i18n::Language;
use crate::services::logger;
use crate::sync::calendar_provider::{CalendarKind, CalendarProvider, RsvpResponse};
use crate::sync::provider::EmailProvider;

/// How long the provider gets to deliver an invitation into the calendar by
/// itself before EmailOps imports it.
pub const INVITE_ARRIVAL_GRACE_SECS: i64 = 600;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InviteResponseOutcome {
    #[serde(rename_all = "camelCase")]
    Responded {
        reply_sent_to: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Imported {
        reply_sent_to: Option<String>,
        reply_error: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Removed {
        reply_sent_to: Option<String>,
        reply_error: Option<String>,
    },
    AlreadyInCalendar,
    NotYetInCalendar,
    #[serde(rename_all = "camelCase")]
    CannotImport {
        reason: String,
        reason_detail: Option<String>,
    },
}

/// Everything `respond_to_invite` needs besides the two providers.
pub struct InviteRequest<'a> {
    pub account: &'a Account,
    pub kind: CalendarKind,
    /// The whole attached document; the first VEVENT is the invitation.
    pub ics: &'a str,
    /// Epoch seconds the email arrived.
    pub email_received_at: i64,
    pub response: RsvpResponse,
    pub default_tz: &'a str,
    pub primary_calendar_id: &'a str,
    pub language: Language,
    /// Epoch seconds.
    pub now: i64,
}

/// The invitation's first VEVENT and its UID.
fn first_event(calendar: &super::ics::ParsedCalendar) -> Result<(&RawVEvent, &str)> {
    let event = calendar
        .events
        .first()
        .ok_or_else(|| AppError::InvalidInput("the invitation has no event".into()))?;
    let uid = event
        .first_value("UID")
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .ok_or_else(|| AppError::InvalidInput("the invitation has no UID".into()))?;
    Ok((event, uid))
}

/// The planned series for `uid`, or the outcome explaining why it cannot be
/// imported.
fn plan_target(
    calendar: &super::ics::ParsedCalendar,
    uid: &str,
    kind: CalendarKind,
    default_tz: &str,
) -> Result<std::result::Result<ImportableEvent, InviteResponseOutcome>> {
    let item = plan_ics_import(calendar, kind, default_tz)?
        .into_iter()
        .find(|item| item.event.uid == uid);
    Ok(match item {
        Some(item) => match item.status {
            ImportItemStatus::Unsupported(reason) => {
                let (code, detail) = reason_code(&reason);
                Err(InviteResponseOutcome::CannotImport {
                    reason: code.to_string(),
                    reason_detail: detail,
                })
            }
            _ => Ok(item.event),
        },
        None => Err(InviteResponseOutcome::CannotImport {
            reason: "missingUidOrStart".to_string(),
            reason_detail: None,
        }),
    })
}

/// Set the account's own entry to `response`, adding it when the invitation
/// did not list the account.
fn set_my_response(attendees: &mut Vec<CalendarAttendee>, me: &str, response: RsvpResponse) {
    let status = response.as_google_status().to_string();
    match attendees.iter_mut().find(|a| a.email.eq_ignore_ascii_case(me)) {
        Some(mine) => mine.response = status,
        None => attendees.push(CalendarAttendee {
            email: me.to_string(),
            response: status,
        }),
    }
}

/// Sends the iTIP reply for one invitation.
struct Replier<'a> {
    mail: &'a dyn EmailProvider,
    req: &'a InviteRequest<'a>,
    event: &'a RawVEvent,
    /// `None` when there is nobody to answer (see `reply_target`).
    organizer: Option<String>,
}

impl Replier<'_> {
    /// The address the reply went to; `None` when none was sent.
    async fn send(&self, response: RsvpResponse) -> Result<Option<String>> {
        let Some(organizer) = &self.organizer else {
            return Ok(None);
        };
        let account = self.req.account;
        let Some(ics) = build_itip_reply(self.event, &account.email, response, self.req.now) else {
            return Ok(None);
        };
        let subject = itip_reply_subject(
            response,
            self.event.first_value("SUMMARY").unwrap_or_default(),
            self.req.language,
        );
        send_itip_reply(self.mail, account, organizer, &subject, &ics).await?;
        Ok(Some(organizer.clone()))
    }

    /// Send once the calendar already changed: a failure (auth included) is
    /// reported next to the outcome, never as an error that hides the change.
    async fn send_after_change(&self, response: RsvpResponse) -> (Option<String>, Option<String>) {
        match self.send(response).await {
            Ok(sent_to) => (sent_to, None),
            Err(e) => {
                logger::log("error", "sync", format!("Calendar invite reply failed: {}", e.code()));
                (None, Some(e.to_string()))
            }
        }
    }
}

/// Answer the invitation: see the module docs. Auth errors propagate unchanged.
pub async fn respond_to_invite(
    calendar: &dyn CalendarProvider,
    mail: &dyn EmailProvider,
    req: &InviteRequest<'_>,
) -> Result<InviteResponseOutcome> {
    log_outcome(decide(calendar, mail, req).await)
}

fn log_outcome(result: Result<InviteResponseOutcome>) -> Result<InviteResponseOutcome> {
    match &result {
        Ok(outcome) => {
            let (level, text) = match outcome {
                InviteResponseOutcome::Responded { .. } => ("success", "Calendar invite answered"),
                InviteResponseOutcome::Imported { reply_error: None, .. } => {
                    ("success", "Calendar invite imported and answered")
                }
                InviteResponseOutcome::Imported { .. } => ("info", "Calendar invite imported, reply not sent"),
                InviteResponseOutcome::Removed { reply_error: None, .. } => {
                    ("success", "Calendar invite declined and removed")
                }
                InviteResponseOutcome::Removed { .. } => ("info", "Calendar invite removed, reply not sent"),
                InviteResponseOutcome::AlreadyInCalendar => ("info", "Calendar invite already in the calendar"),
                InviteResponseOutcome::NotYetInCalendar => ("info", "Calendar invite not in the calendar yet"),
                InviteResponseOutcome::CannotImport { .. } => ("info", "Calendar invite cannot be imported"),
            };
            logger::log(level, "sync", text);
        }
        Err(e) => logger::log(
            "error",
            "sync",
            format!("Calendar invite response failed: {}", e.code()),
        ),
    }
    result
}

async fn decide(
    calendar: &dyn CalendarProvider,
    mail: &dyn EmailProvider,
    req: &InviteRequest<'_>,
) -> Result<InviteResponseOutcome> {
    let parsed = parse_vcalendar(req.ics);
    let (event, uid) = first_event(&parsed)?;
    if is_publish_invite(&parsed) {
        return Err(AppError::InvalidInput(
            "this event has no one to answer; add it to the calendar instead".into(),
        ));
    }
    let me = req.account.email.as_str();
    let replier = Replier {
        mail,
        req,
        event,
        organizer: reply_target(event, me),
    };
    let declined = req.response == RsvpResponse::Declined;
    // An answer to one occurrence: our copy is found by the series' UID, so it
    // is the whole series and must not change.
    let one_occurrence = event.first("RECURRENCE-ID").is_some();

    match calendar.find_event_by_uid(uid).await? {
        Some(found) if found.imported_by_emailops => {
            if declined && !one_occurrence {
                calendar
                    .delete_event(req.primary_calendar_id, &found.provider_event_id, false, "")
                    .await?;
                let (reply_sent_to, reply_error) = replier.send_after_change(req.response).await;
                Ok(InviteResponseOutcome::Removed {
                    reply_sent_to,
                    reply_error,
                })
            } else {
                // Our Google copy carries my answer; keep it current without
                // emailing its attendees. Outlook copies have no attendees.
                if !declined && !one_occurrence && req.kind == CalendarKind::Google {
                    calendar
                        .respond_to_event(
                            req.primary_calendar_id,
                            &found.provider_event_id,
                            req.response,
                            me,
                            false,
                        )
                        .await?;
                }
                Ok(InviteResponseOutcome::Responded {
                    reply_sent_to: replier.send(req.response).await?,
                })
            }
        }
        Some(_) => {
            calendar.rsvp_by_ical_uid(uid, req.response, me).await?;
            Ok(InviteResponseOutcome::Responded { reply_sent_to: None })
        }
        None if req.now - req.email_received_at < INVITE_ARRIVAL_GRACE_SECS => {
            Ok(InviteResponseOutcome::NotYetInCalendar)
        }
        None if declined => Ok(InviteResponseOutcome::Responded {
            reply_sent_to: replier.send(req.response).await?,
        }),
        None => {
            let mut target = match plan_target(&parsed, uid, req.kind, req.default_tz)? {
                Ok(target) => target,
                Err(outcome) => return Ok(outcome),
            };
            set_my_response(&mut target.attendees, me, req.response);
            calendar.import_event(&target).await?;
            let (reply_sent_to, reply_error) = replier.send_after_change(req.response).await;
            Ok(InviteResponseOutcome::Imported {
                reply_sent_to,
                reply_error,
            })
        }
    }
}

/// Import a published (or any) invitation into the calendar without answering.
pub async fn add_invite_to_calendar(
    calendar: &dyn CalendarProvider,
    kind: CalendarKind,
    ics: &str,
    default_tz: &str,
) -> Result<InviteResponseOutcome> {
    log_outcome(add(calendar, kind, ics, default_tz).await)
}

async fn add(
    calendar: &dyn CalendarProvider,
    kind: CalendarKind,
    ics: &str,
    default_tz: &str,
) -> Result<InviteResponseOutcome> {
    let parsed = parse_vcalendar(ics);
    let (_, uid) = first_event(&parsed)?;
    if !is_publish_invite(&parsed) {
        return Err(AppError::InvalidInput("this invitation expects an answer".into()));
    }
    Ok(match plan_target(&parsed, uid, kind, default_tz)? {
        Err(outcome) => outcome,
        Ok(target) => {
            if calendar.find_event_by_uid(uid).await?.is_some() {
                InviteResponseOutcome::AlreadyInCalendar
            } else {
                calendar.import_event(&target).await?;
                InviteResponseOutcome::Imported {
                    reply_sent_to: None,
                    reply_error: None,
                }
            }
        }
    })
}

/// How much of a recurring series a response from the event detail affects.
/// Non-recurring events only ever use `Instance`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseScope {
    /// Just the clicked occurrence (or the whole event when non-recurring).
    Instance,
    /// The whole series.
    Series,
}

impl ResponseScope {
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "instance" => Some(Self::Instance),
            "series" => Some(Self::Series),
            _ => None,
        }
    }
}

/// Change the account's own answer on an event already in its calendar.
/// Provider-first, local-second: when the provider call fails the local rows
/// stay as they were.
pub async fn respond_to_calendar_event(
    db: &Database,
    account: &Account,
    provider: &dyn CalendarProvider,
    calendar_id: &str,
    provider_event_id: &str,
    response: RsvpResponse,
    scope: ResponseScope,
) -> Result<()> {
    let event = db
        .get_calendar_event_by_provider_id(&account.id, calendar_id, provider_event_id)?
        .ok_or_else(|| AppError::NotFound(format!("calendar event '{provider_event_id}' not found")))?;

    let read_only = db.list_calendars(&account.id)?.iter().any(|c| {
        c.provider_calendar_id == calendar_id && matches!(c.access_role.as_str(), "reader" | "freeBusyReader")
    });
    if read_only {
        return Err(AppError::ReadOnlyCalendar);
    }

    let (target_id, mut rows) = match scope {
        ResponseScope::Series => {
            // A master row has no `recurring_event_id`; it is its own series id.
            let master_id = event.recurring_event_id.as_deref().unwrap_or(provider_event_id);
            (
                master_id.to_string(),
                db.list_calendar_events_for_master(&account.id, calendar_id, master_id)?,
            )
        }
        ResponseScope::Instance => (provider_event_id.to_string(), vec![event]),
    };

    provider
        .respond_to_event(calendar_id, &target_id, response, &account.email, true)
        .await?;

    let status = response.as_google_status();
    for row in &mut rows {
        match row
            .attendees
            .iter_mut()
            .find(|a| a.email.eq_ignore_ascii_case(&account.email))
        {
            Some(me) => me.response = status.to_string(),
            None => row.attendees.push(CalendarAttendee {
                email: account.email.clone(),
                response: status.to_string(),
            }),
        }
    }
    db.upsert_calendar_events(&rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::calendar_provider::{FakeCalendarProvider, FoundEvent};
    use crate::sync::provider::FakeEmailProvider;

    const UID: &str = "inv-1@example.org";
    const NOW: i64 = 1_796_119_200;
    const OLD: i64 = NOW - 11 * 60;
    const YOUNG: i64 = NOW - 5 * 60;

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

    fn ics(organizer: Option<&str>, extra: &str) -> String {
        let organizer = organizer
            .map(|o| format!("ORGANIZER;CN=Lead:mailto:{o}\r\n"))
            .unwrap_or_default();
        format!(
            "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:{UID}\r\n\
             DTSTART:20261201T100000Z\r\nDTEND:20261201T110000Z\r\nSUMMARY:Planning\r\n\
             {organizer}{extra}END:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    }

    fn lead_ics() -> String {
        ics(Some("Lead@Example.com"), "")
    }

    fn request<'a>(account: &'a Account, ics: &'a str, received_at: i64, response: RsvpResponse) -> InviteRequest<'a> {
        InviteRequest {
            account,
            kind: CalendarKind::Google,
            ics,
            email_received_at: received_at,
            response,
            default_tz: "Europe/Madrid",
            primary_calendar_id: "primary",
            language: Language::En,
            now: NOW,
        }
    }

    fn mail() -> FakeEmailProvider {
        FakeEmailProvider::new("me@example.com", "Me")
    }

    /// The sent MIME with quoted-printable escapes undone.
    fn sent_text(mail: &FakeEmailProvider) -> Vec<String> {
        mail.raw_sent()
            .iter()
            .map(|raw| String::from_utf8_lossy(raw).replace("=\r\n", "").replace("=3D", "="))
            .collect()
    }

    async fn run(
        calendar: &FakeCalendarProvider,
        mail: &FakeEmailProvider,
        ics: &str,
        received_at: i64,
        response: RsvpResponse,
    ) -> Result<InviteResponseOutcome> {
        let account = account();
        respond_to_invite(calendar, mail, &request(&account, ics, received_at, response)).await
    }

    /// A calendar whose delete always fails; everything else is the fake.
    struct FailingDelete(FakeCalendarProvider);

    #[async_trait::async_trait]
    impl CalendarProvider for FailingDelete {
        async fn list_calendars(&self) -> Result<Vec<crate::sync::calendar_provider::ProviderCalendar>> {
            self.0.list_calendars().await
        }
        async fn list_events(
            &self,
            calendar_id: &str,
            window_start: i64,
            window_end: i64,
        ) -> Result<Vec<crate::sync::calendar_provider::ProviderCalendarEvent>> {
            self.0.list_events(calendar_id, window_start, window_end).await
        }
        async fn create_event(
            &self,
            event: &crate::sync::calendar_provider::NewCalendarEvent,
        ) -> Result<crate::sync::calendar_provider::ProviderCalendarEvent> {
            self.0.create_event(event).await
        }
        async fn delete_event(&self, _: &str, _: &str, _: bool, _: &str) -> Result<()> {
            Err(AppError::SyncError("delete failed".into()))
        }
        async fn truncate_recurring_event(&self, c: &str, m: &str, s: i64, n: bool) -> Result<()> {
            self.0.truncate_recurring_event(c, m, s, n).await
        }
        async fn rsvp_by_ical_uid(&self, u: &str, r: RsvpResponse, e: &str) -> Result<()> {
            self.0.rsvp_by_ical_uid(u, r, e).await
        }
        async fn find_event_by_uid(&self, u: &str) -> Result<Option<FoundEvent>> {
            self.0.find_event_by_uid(u).await
        }
        async fn import_event(&self, e: &ImportableEvent) -> Result<String> {
            self.0.import_event(e).await
        }
        async fn respond_to_event(&self, c: &str, p: &str, r: RsvpResponse, e: &str, n: bool) -> Result<()> {
            self.0.respond_to_event(c, p, r, e, n).await
        }
    }

    fn found(ours: bool) -> FoundEvent {
        FoundEvent {
            provider_event_id: "ev-1".into(),
            imported_by_emailops: ours,
        }
    }

    #[tokio::test]
    async fn invite_already_delivered_uses_the_provider_rsvp() {
        let mut calendar = FakeCalendarProvider::with_events(vec![]);
        calendar.found_by_uid.insert(UID.into(), found(false));
        let mail = mail();
        let out = run(&calendar, &mail, &lead_ics(), OLD, RsvpResponse::Accepted)
            .await
            .expect("respond");
        assert_eq!(out, InviteResponseOutcome::Responded { reply_sent_to: None });
        assert_eq!(calendar.rsvps.lock().unwrap().len(), 1);
        assert!(mail.raw_sent().is_empty());
    }

    #[tokio::test]
    async fn young_missing_invite_waits() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let mail = mail();
        let out = run(&calendar, &mail, &lead_ics(), YOUNG, RsvpResponse::Accepted)
            .await
            .expect("respond");
        assert_eq!(out, InviteResponseOutcome::NotYetInCalendar);
        assert!(calendar.imported.lock().unwrap().is_empty());
        assert!(mail.raw_sent().is_empty());
    }

    #[tokio::test]
    async fn accepting_an_old_missing_invite_imports_then_replies() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let mail = mail();
        let out = run(&calendar, &mail, &lead_ics(), OLD, RsvpResponse::Accepted)
            .await
            .expect("respond");
        assert_eq!(
            out,
            InviteResponseOutcome::Imported {
                reply_sent_to: Some("lead@example.com".into()),
                reply_error: None
            }
        );
        assert_eq!(calendar.imported.lock().unwrap().len(), 1);
        let sent = sent_text(&mail);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("PARTSTAT=ACCEPTED"), "{}", sent[0]);
    }

    #[tokio::test]
    async fn the_reply_goes_only_to_the_organizer() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let mail = mail();
        let doc = ics(
            Some("lead@example.com"),
            "ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:other@example.org\r\n\
             ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@example.com\r\n",
        );
        run(&calendar, &mail, &doc, OLD, RsvpResponse::Accepted)
            .await
            .expect("respond");
        let sent = sent_text(&mail);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("To: lead@example.com"), "{}", sent[0]);
        assert!(!sent[0].contains("other@example.org"), "{}", sent[0]);
        assert!(!sent[0].contains("Cc:"), "{}", sent[0]);
    }

    #[tokio::test]
    async fn my_listed_partstat_is_updated_on_import() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let doc = ics(
            Some("lead@example.com"),
            "ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:Me@Example.com\r\n\
             ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:other@example.org\r\n",
        );
        run(&calendar, &mail(), &doc, OLD, RsvpResponse::Tentative)
            .await
            .expect("respond");
        let imported = calendar.imported.lock().unwrap();
        let attendees = &imported[0].attendees;
        assert_eq!(attendees.len(), 2);
        let mine = attendees
            .iter()
            .find(|a| a.email.eq_ignore_ascii_case("me@example.com"))
            .expect("me");
        assert_eq!(mine.response, "tentative");
        let other = attendees
            .iter()
            .find(|a| a.email == "other@example.org")
            .expect("other");
        assert_ne!(other.response, "tentative");
    }

    #[tokio::test]
    async fn my_partstat_is_appended_when_i_am_not_listed() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        run(&calendar, &mail(), &lead_ics(), OLD, RsvpResponse::Accepted)
            .await
            .expect("respond");
        let imported = calendar.imported.lock().unwrap();
        assert_eq!(
            imported[0].attendees,
            vec![CalendarAttendee {
                email: "me@example.com".into(),
                response: "accepted".into()
            }]
        );
    }

    #[tokio::test]
    async fn failed_import_sends_nothing() {
        let mut calendar = FakeCalendarProvider::with_events(vec![]);
        calendar.import_errors.insert(UID.into(), "boom".into());
        let mail = mail();
        let result = run(&calendar, &mail, &lead_ics(), OLD, RsvpResponse::Accepted).await;
        assert!(result.is_err());
        assert!(mail.raw_sent().is_empty());
    }

    #[tokio::test]
    async fn unsupported_import_sends_nothing() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let mail = mail();
        let doc = ics(Some("lead@example.com"), "RRULE:FREQ=HOURLY\r\n");
        let account = account();
        let mut req = request(&account, &doc, OLD, RsvpResponse::Accepted);
        req.kind = CalendarKind::Outlook;
        let out = respond_to_invite(&calendar, &mail, &req).await.expect("respond");
        assert_eq!(
            out,
            InviteResponseOutcome::CannotImport {
                reason: "outlookRecurrence".into(),
                reason_detail: Some("FREQ=HOURLY".into())
            }
        );
        assert!(mail.raw_sent().is_empty());
        assert!(calendar.imported.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn declining_a_missing_invite_replies_without_importing() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let mail = mail();
        let out = run(&calendar, &mail, &lead_ics(), OLD, RsvpResponse::Declined)
            .await
            .expect("respond");
        assert_eq!(
            out,
            InviteResponseOutcome::Responded {
                reply_sent_to: Some("lead@example.com".into())
            }
        );
        assert!(calendar.imported.lock().unwrap().is_empty());
        let sent = sent_text(&mail);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("PARTSTAT=DECLINED"), "{}", sent[0]);
    }

    #[tokio::test]
    async fn declining_after_accepting_removes_our_copy() {
        let mut calendar = FakeCalendarProvider::with_events(vec![]);
        calendar.found_by_uid.insert(UID.into(), found(true));
        let mail = mail();
        let out = run(&calendar, &mail, &lead_ics(), OLD, RsvpResponse::Declined)
            .await
            .expect("respond");
        assert_eq!(
            out,
            InviteResponseOutcome::Removed {
                reply_sent_to: Some("lead@example.com".into()),
                reply_error: None
            }
        );
        let deleted = calendar.deleted.lock().unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0], ("primary".into(), "ev-1".into(), false, String::new()));
        assert!(
            calendar.responses.lock().unwrap().is_empty(),
            "no status on a deleted copy"
        );
        let sent = sent_text(&mail);
        assert!(sent[0].contains("PARTSTAT=DECLINED"), "{}", sent[0]);
    }

    #[tokio::test]
    async fn answering_again_updates_our_google_copy_quietly_then_replies() {
        let mut calendar = FakeCalendarProvider::with_events(vec![]);
        calendar.found_by_uid.insert(UID.into(), found(true));
        let mail = mail();
        let out = run(&calendar, &mail, &lead_ics(), OLD, RsvpResponse::Tentative)
            .await
            .expect("respond");
        assert_eq!(
            out,
            InviteResponseOutcome::Responded {
                reply_sent_to: Some("lead@example.com".into())
            }
        );
        assert_eq!(
            *calendar.responses.lock().unwrap(),
            vec![("primary".into(), "ev-1".into(), "tentative".into(), false)]
        );
        let sent = sent_text(&mail);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("PARTSTAT=TENTATIVE"), "{}", sent[0]);
    }

    #[tokio::test]
    async fn a_failed_status_update_on_our_copy_sends_nothing() {
        let mut calendar = FakeCalendarProvider::with_events(vec![]);
        calendar.found_by_uid.insert(UID.into(), found(true));
        calendar.respond_errors.insert("ev-1".into(), "patch failed".into());
        let mail = mail();
        run(&calendar, &mail, &lead_ics(), OLD, RsvpResponse::Accepted)
            .await
            .expect_err("the status update failed");
        assert!(mail.raw_sent().is_empty(), "no reply unless the calendar changed");
    }

    #[tokio::test]
    async fn an_outlook_copy_gets_no_status_update() {
        let mut calendar = FakeCalendarProvider::with_events(vec![]);
        calendar.found_by_uid.insert(UID.into(), found(true));
        let mail = mail();
        let account = account();
        let lead = lead_ics();
        let mut req = request(&account, &lead, OLD, RsvpResponse::Accepted);
        req.kind = CalendarKind::Outlook;
        let out = respond_to_invite(&calendar, &mail, &req).await.expect("respond");
        assert_eq!(
            out,
            InviteResponseOutcome::Responded {
                reply_sent_to: Some("lead@example.com".into())
            }
        );
        assert!(calendar.responses.lock().unwrap().is_empty());
        assert_eq!(mail.raw_sent().len(), 1);
    }

    /// An invitation for one occurrence of a series.
    fn occurrence_ics() -> String {
        ics(Some("lead@example.com"), "RECURRENCE-ID:20261208T100000Z\r\n")
    }

    #[tokio::test]
    async fn declining_one_occurrence_keeps_our_series() {
        let mut calendar = FakeCalendarProvider::with_events(vec![]);
        calendar.found_by_uid.insert(UID.into(), found(true));
        let mail = mail();
        let out = run(&calendar, &mail, &occurrence_ics(), OLD, RsvpResponse::Declined)
            .await
            .expect("respond");
        assert_eq!(
            out,
            InviteResponseOutcome::Responded {
                reply_sent_to: Some("lead@example.com".into())
            }
        );
        assert!(calendar.deleted.lock().unwrap().is_empty(), "the series must survive");
        let sent = sent_text(&mail);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("RECURRENCE-ID:20261208T100000Z"), "{}", sent[0]);
        assert!(sent[0].contains("PARTSTAT=DECLINED"), "{}", sent[0]);
    }

    #[tokio::test]
    async fn answering_one_occurrence_leaves_our_series_status() {
        let mut calendar = FakeCalendarProvider::with_events(vec![]);
        calendar.found_by_uid.insert(UID.into(), found(true));
        let mail = mail();
        run(&calendar, &mail, &occurrence_ics(), OLD, RsvpResponse::Tentative)
            .await
            .expect("respond");
        assert!(
            calendar.responses.lock().unwrap().is_empty(),
            "a status on the series would cover every occurrence"
        );
        let sent = sent_text(&mail);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("RECURRENCE-ID:20261208T100000Z"), "{}", sent[0]);
    }

    #[tokio::test]
    async fn accepting_again_does_not_reimport() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let mail = mail();
        run(&calendar, &mail, &lead_ics(), OLD, RsvpResponse::Accepted)
            .await
            .expect("first");
        let out = run(&calendar, &mail, &lead_ics(), OLD, RsvpResponse::Accepted)
            .await
            .expect("second");
        assert_eq!(
            out,
            InviteResponseOutcome::Responded {
                reply_sent_to: Some("lead@example.com".into())
            }
        );
        assert_eq!(calendar.imported.lock().unwrap().len(), 1);
        assert_eq!(mail.raw_sent().len(), 2);
    }

    #[tokio::test]
    async fn no_reply_to_myself() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let mail = mail();
        let doc = ics(Some("ME@example.com"), "");
        let out = run(&calendar, &mail, &doc, OLD, RsvpResponse::Accepted)
            .await
            .expect("respond");
        assert_eq!(
            out,
            InviteResponseOutcome::Imported {
                reply_sent_to: None,
                reply_error: None
            }
        );
        assert!(mail.raw_sent().is_empty());
    }

    #[tokio::test]
    async fn responding_to_a_publish_invite_is_refused() {
        let with_method = ics(Some("lead@example.com"), "").replace("METHOD:REQUEST", "METHOD:PUBLISH");
        for doc in [ics(None, ""), with_method] {
            let calendar = FakeCalendarProvider::with_events(vec![]);
            let mail = mail();
            let err = run(&calendar, &mail, &doc, OLD, RsvpResponse::Accepted)
                .await
                .expect_err("refused");
            assert!(matches!(err, AppError::InvalidInput(_)), "{err}");
            assert!(calendar.imported.lock().unwrap().is_empty());
            assert!(mail.raw_sent().is_empty());
        }
    }

    #[tokio::test]
    async fn adding_an_invite_that_expects_an_answer_is_refused() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let err = add_invite_to_calendar(&calendar, CalendarKind::Google, &lead_ics(), "Europe/Madrid")
            .await
            .expect_err("refused");
        assert!(matches!(err, AppError::InvalidInput(_)), "{err}");
        assert!(calendar.imported.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_invalid_organizer_still_imports_but_sends_nothing() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let mail = mail();
        let doc = ics(Some("lead@intranet"), "");
        let out = run(&calendar, &mail, &doc, OLD, RsvpResponse::Accepted)
            .await
            .expect("respond");
        assert_eq!(
            out,
            InviteResponseOutcome::Imported {
                reply_sent_to: None,
                reply_error: None
            }
        );
        assert_eq!(calendar.imported.lock().unwrap().len(), 1);
        assert!(mail.raw_sent().is_empty());
    }

    #[tokio::test]
    async fn a_failed_reply_after_import_keeps_the_imported_outcome() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let mail = mail();
        mail.fail_sends(Some("smtp down"));
        let out = run(&calendar, &mail, &lead_ics(), OLD, RsvpResponse::Accepted)
            .await
            .expect("not an error");
        let InviteResponseOutcome::Imported {
            reply_sent_to,
            reply_error,
        } = out
        else {
            panic!("expected Imported, got {out:?}");
        };
        assert_eq!(reply_sent_to, None);
        assert!(reply_error.is_some_and(|m| m.contains("smtp down")));
        assert_eq!(calendar.imported.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_reply_without_a_calendar_change_is_an_error() {
        let mut calendar = FakeCalendarProvider::with_events(vec![]);
        calendar.found_by_uid.insert(UID.into(), found(true));
        let mail = mail();
        mail.fail_sends(Some("smtp down"));
        run(&calendar, &mail, &lead_ics(), OLD, RsvpResponse::Accepted)
            .await
            .expect_err("responded needs the reply");
    }

    #[tokio::test]
    async fn declining_deletes_our_copy_before_replying() {
        let mut inner = FakeCalendarProvider::with_events(vec![]);
        inner.found_by_uid.insert(UID.into(), found(true));
        let calendar = FailingDelete(inner);
        let mail = mail();
        let account = account();
        let lead = lead_ics();
        let req = request(&account, &lead, OLD, RsvpResponse::Declined);
        respond_to_invite(&calendar, &mail, &req)
            .await
            .expect_err("delete fails");
        assert!(mail.raw_sent().is_empty(), "nothing may be sent when the delete failed");
    }

    #[tokio::test]
    async fn a_failed_decline_reply_after_the_delete_keeps_the_removed_outcome() {
        let mut calendar = FakeCalendarProvider::with_events(vec![]);
        calendar.found_by_uid.insert(UID.into(), found(true));
        let mail = mail();
        mail.fail_sends(Some("smtp down"));
        let out = run(&calendar, &mail, &lead_ics(), OLD, RsvpResponse::Declined)
            .await
            .expect("not an error");
        let InviteResponseOutcome::Removed {
            reply_sent_to,
            reply_error,
        } = out
        else {
            panic!("expected Removed, got {out:?}");
        };
        assert_eq!(reply_sent_to, None);
        assert!(reply_error.is_some_and(|m| m.contains("smtp down")));
        assert_eq!(calendar.deleted.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_target_is_the_first_event_of_the_document() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let second = "BEGIN:VEVENT\r\nUID:second@example.org\r\nDTSTART:20261202T100000Z\r\nDTEND:20261202T110000Z\r\nEND:VEVENT\r\n";
        let doc = lead_ics().replace("END:VCALENDAR", &format!("{second}END:VCALENDAR"));
        run(&calendar, &mail(), &doc, OLD, RsvpResponse::Accepted)
            .await
            .expect("respond");
        let imported = calendar.imported.lock().unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].uid, UID);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the guard serializes the global logger across the await
    async fn outcomes_are_logged_without_event_details() {
        let _seam = crate::services::events::seam_test_lock();
        let logs = crate::services::logger::install_for_testing();
        let doc = lead_ics()
            .replace(UID, "zebra-uid")
            .replace("Planning", "Zebra Quarterly")
            .replace("Lead@Example.com", "lead@zebra-org.test");
        let calendar = FakeCalendarProvider::with_events(vec![]);
        run(&calendar, &mail(), &doc, OLD, RsvpResponse::Accepted)
            .await
            .expect("respond");
        let mut failing = FakeCalendarProvider::with_events(vec![]);
        failing.find_errors.insert("zebra-uid".into(), "boom".into());
        run(&failing, &mail(), &doc, OLD, RsvpResponse::Accepted)
            .await
            .expect_err("lookup fails");
        // Other tests in this module log through the same global seam.
        let events = logs.events();
        let has = |level: &str| events.iter().any(|e| e.level == level && e.source == "sync");
        assert!(has("success") && has("error"), "{events:?}");
        for e in &events {
            for secret in ["Zebra Quarterly", "zebra-org", "zebra-uid"] {
                assert!(!e.message.contains(secret), "{}", e.message);
            }
        }
    }

    #[tokio::test]
    async fn add_to_calendar_imports_publish_events() {
        let calendar = FakeCalendarProvider::with_events(vec![]);
        let doc = ics(None, "").replace("METHOD:REQUEST", "METHOD:PUBLISH");
        let first = add_invite_to_calendar(&calendar, CalendarKind::Google, &doc, "Europe/Madrid")
            .await
            .expect("add");
        assert_eq!(
            first,
            InviteResponseOutcome::Imported {
                reply_sent_to: None,
                reply_error: None
            }
        );
        assert_eq!(calendar.imported.lock().unwrap().len(), 1);
        let second = add_invite_to_calendar(&calendar, CalendarKind::Google, &doc, "Europe/Madrid")
            .await
            .expect("add again");
        assert_eq!(second, InviteResponseOutcome::AlreadyInCalendar);
        assert_eq!(calendar.imported.lock().unwrap().len(), 1);
    }

    #[test]
    fn outcomes_serialize_with_a_kind_tag() {
        let json = serde_json::to_value(InviteResponseOutcome::Imported {
            reply_sent_to: Some("lead@example.com".into()),
            reply_error: Some("smtp down".into()),
        })
        .expect("json");
        assert_eq!(
            json,
            serde_json::json!({"kind": "imported", "replySentTo": "lead@example.com", "replyError": "smtp down"})
        );
        let json = serde_json::to_value(InviteResponseOutcome::AlreadyInCalendar).expect("json");
        assert_eq!(json, serde_json::json!({"kind": "alreadyInCalendar"}));
    }

    // --- respond_to_calendar_event (change my answer from the event detail) ---

    use crate::models::{Calendar, CalendarEvent};

    fn stored_event(id: &str, start: i64, master: Option<&str>) -> CalendarEvent {
        CalendarEvent {
            id: format!("a1:primary:{id}"),
            account_id: "a1".into(),
            provider_event_id: id.into(),
            calendar_id: "primary".into(),
            title: "Planning".into(),
            description: String::new(),
            location: String::new(),
            start_time: start,
            end_time: start + 1_800,
            is_all_day: false,
            timezone: String::new(),
            organizer: "lead@example.com".into(),
            attendees: vec![
                CalendarAttendee {
                    email: "lead@example.com".into(),
                    response: "accepted".into(),
                },
                CalendarAttendee {
                    email: "Me@Example.com".into(),
                    response: "needsAction".into(),
                },
            ],
            meeting_link: None,
            meeting_platform: None,
            status: "confirmed".into(),
            html_link: None,
            notified_at: None,
            recurring_event_id: master.map(str::to_string),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn db_with(events: &[CalendarEvent]) -> Database {
        let db = Database::new_for_testing().expect("test db");
        db.seed_test_account("a1");
        db.upsert_calendar_events(events).expect("seed events");
        db
    }

    fn my_response(db: &Database, id: &str) -> Option<String> {
        db.get_calendar_event_by_provider_id("a1", "primary", id)
            .expect("read")
            .expect("row")
            .attendees
            .into_iter()
            .find(|a| a.email.eq_ignore_ascii_case("me@example.com"))
            .map(|a| a.response)
    }

    async fn respond(
        db: &Database,
        provider: &dyn CalendarProvider,
        id: &str,
        response: RsvpResponse,
        scope: ResponseScope,
    ) -> Result<()> {
        respond_to_calendar_event(db, &account(), provider, "primary", id, response, scope).await
    }

    #[test]
    fn scope_parses_only_known_wire_values() {
        assert_eq!(ResponseScope::from_wire("instance"), Some(ResponseScope::Instance));
        assert_eq!(ResponseScope::from_wire("series"), Some(ResponseScope::Series));
        assert_eq!(ResponseScope::from_wire("all"), None);
    }

    #[tokio::test]
    async fn instance_response_updates_provider_and_local_row() {
        let db = db_with(&[stored_event("ev1", 1_000, None)]);
        let provider = FakeCalendarProvider::with_events(vec![]);

        respond(&db, &provider, "ev1", RsvpResponse::Tentative, ResponseScope::Instance)
            .await
            .expect("respond");

        let responses = provider.responses.lock().expect("lock").clone();
        assert_eq!(
            responses,
            vec![("primary".to_string(), "ev1".to_string(), "tentative".to_string(), true)]
        );
        let row = db
            .get_calendar_event_by_provider_id("a1", "primary", "ev1")
            .expect("read")
            .expect("row");
        assert_eq!(
            row.attendees,
            vec![
                CalendarAttendee {
                    email: "lead@example.com".into(),
                    response: "accepted".into()
                },
                CalendarAttendee {
                    email: "Me@Example.com".into(),
                    response: "tentative".into()
                },
            ]
        );
    }

    #[tokio::test]
    async fn my_attendee_is_appended_when_absent() {
        let mut event = stored_event("ev1", 1_000, None);
        event.attendees.truncate(1);
        let db = db_with(&[event]);
        let provider = FakeCalendarProvider::with_events(vec![]);

        respond(&db, &provider, "ev1", RsvpResponse::Accepted, ResponseScope::Instance)
            .await
            .expect("respond");

        assert_eq!(my_response(&db, "ev1").as_deref(), Some("accepted"));
    }

    #[tokio::test]
    async fn series_response_targets_the_master_and_all_local_instances() {
        let db = db_with(&[
            stored_event("m_1", 1_000, Some("m")),
            stored_event("m_2", 2_000, Some("m")),
            stored_event("n_1", 1_500, Some("n")),
        ]);
        let provider = FakeCalendarProvider::with_events(vec![]);

        respond(&db, &provider, "m_2", RsvpResponse::Declined, ResponseScope::Series)
            .await
            .expect("respond");

        let responses = provider.responses.lock().expect("lock").clone();
        assert_eq!(
            responses,
            vec![("primary".to_string(), "m".to_string(), "declined".to_string(), true)]
        );
        assert_eq!(my_response(&db, "m_1").as_deref(), Some("declined"));
        assert_eq!(my_response(&db, "m_2").as_deref(), Some("declined"));
        assert_eq!(my_response(&db, "n_1").as_deref(), Some("needsAction"));
    }

    #[tokio::test]
    async fn series_scope_on_a_single_event_acts_on_the_event() {
        let db = db_with(&[stored_event("ev1", 1_000, None)]);
        let provider = FakeCalendarProvider::with_events(vec![]);

        respond(&db, &provider, "ev1", RsvpResponse::Accepted, ResponseScope::Series)
            .await
            .expect("respond");

        assert_eq!(provider.responses.lock().expect("lock")[0].1, "ev1");
        assert_eq!(my_response(&db, "ev1").as_deref(), Some("accepted"));
    }

    #[tokio::test]
    async fn provider_failure_leaves_local_rows_untouched() {
        let db = db_with(&[stored_event("ev1", 1_000, None)]);
        let provider = FakeCalendarProvider::failing("boom");

        let result = respond(&db, &provider, "ev1", RsvpResponse::Accepted, ResponseScope::Instance).await;

        assert!(matches!(result, Err(AppError::SyncError(_))));
        assert_eq!(my_response(&db, "ev1").as_deref(), Some("needsAction"));
    }

    #[tokio::test]
    async fn read_only_calendar_is_refused_before_calling_the_provider() {
        let db = db_with(&[stored_event("ev1", 1_000, None)]);
        for role in ["reader", "freeBusyReader"] {
            db.upsert_calendars(&[Calendar {
                id: crate::db::calendars::calendar_row_id("a1", "primary"),
                account_id: "a1".into(),
                provider_calendar_id: "primary".into(),
                name: "Shared".into(),
                color: String::new(),
                is_primary: true,
                access_role: role.into(),
                is_visible: true,
                sort_order: 0,
                owner_email: None,
                feed_host: None,
                created_at: 0,
                updated_at: 0,
            }])
            .expect("seed calendar");
            let provider = FakeCalendarProvider::with_events(vec![]);

            let result = respond(&db, &provider, "ev1", RsvpResponse::Accepted, ResponseScope::Instance).await;

            assert!(matches!(result, Err(AppError::ReadOnlyCalendar)), "{role}");
            assert!(provider.responses.lock().expect("lock").is_empty(), "{role}");
        }
    }

    #[tokio::test]
    async fn provider_read_only_error_is_propagated() {
        struct ReadOnly(FakeCalendarProvider);
        #[async_trait::async_trait]
        impl CalendarProvider for ReadOnly {
            async fn list_calendars(&self) -> Result<Vec<crate::sync::calendar_provider::ProviderCalendar>> {
                self.0.list_calendars().await
            }
            async fn list_events(
                &self,
                c: &str,
                a: i64,
                b: i64,
            ) -> Result<Vec<crate::sync::calendar_provider::ProviderCalendarEvent>> {
                self.0.list_events(c, a, b).await
            }
            async fn create_event(
                &self,
                e: &crate::sync::calendar_provider::NewCalendarEvent,
            ) -> Result<crate::sync::calendar_provider::ProviderCalendarEvent> {
                self.0.create_event(e).await
            }
            async fn delete_event(&self, c: &str, p: &str, n: bool, m: &str) -> Result<()> {
                self.0.delete_event(c, p, n, m).await
            }
            async fn truncate_recurring_event(&self, c: &str, m: &str, s: i64, n: bool) -> Result<()> {
                self.0.truncate_recurring_event(c, m, s, n).await
            }
            async fn rsvp_by_ical_uid(&self, u: &str, r: RsvpResponse, e: &str) -> Result<()> {
                self.0.rsvp_by_ical_uid(u, r, e).await
            }
            async fn find_event_by_uid(&self, u: &str) -> Result<Option<FoundEvent>> {
                self.0.find_event_by_uid(u).await
            }
            async fn import_event(&self, e: &ImportableEvent) -> Result<String> {
                self.0.import_event(e).await
            }
            async fn respond_to_event(&self, _: &str, _: &str, _: RsvpResponse, _: &str, _: bool) -> Result<()> {
                Err(AppError::ReadOnlyCalendar)
            }
        }
        let db = db_with(&[stored_event("ev1", 1_000, None)]);
        let provider = ReadOnly(FakeCalendarProvider::with_events(vec![]));

        let result = respond(&db, &provider, "ev1", RsvpResponse::Accepted, ResponseScope::Instance).await;

        assert!(matches!(result, Err(AppError::ReadOnlyCalendar)));
        assert_eq!(my_response(&db, "ev1").as_deref(), Some("needsAction"));
    }

    #[tokio::test]
    async fn unknown_event_is_not_found() {
        let db = db_with(&[]);
        let provider = FakeCalendarProvider::with_events(vec![]);

        let result = respond(&db, &provider, "nope", RsvpResponse::Accepted, ResponseScope::Instance).await;

        assert!(matches!(result, Err(AppError::NotFound(_))));
        assert!(provider.responses.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn series_response_clicked_on_the_master_updates_every_local_row() {
        let db = db_with(&[
            stored_event("m", 500, None),
            stored_event("m_1", 1_000, Some("m")),
            stored_event("m_2", 2_000, Some("m")),
        ]);
        let provider = FakeCalendarProvider::with_events(vec![]);

        respond(&db, &provider, "m", RsvpResponse::Accepted, ResponseScope::Series)
            .await
            .expect("respond");

        assert_eq!(provider.responses.lock().expect("lock")[0].1, "m");
        for id in ["m", "m_1", "m_2"] {
            assert_eq!(my_response(&db, id).as_deref(), Some("accepted"), "{id}");
        }
    }

    #[tokio::test]
    async fn a_missing_calendar_row_does_not_block_the_provider_call() {
        // Fail-open by design: no local `calendars` row means no known role.
        let db = db_with(&[stored_event("ev1", 1_000, None)]);
        let provider = FakeCalendarProvider::with_events(vec![]);

        respond(&db, &provider, "ev1", RsvpResponse::Accepted, ResponseScope::Instance)
            .await
            .expect("respond");

        assert_eq!(provider.responses.lock().expect("lock").len(), 1);
    }
}
