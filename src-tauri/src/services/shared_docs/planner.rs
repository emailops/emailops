//! Pure decisions of the shared-document sync: what to do with an arriving
//! envelope, which documents are due to be mailed, and to whom.

use super::envelope::Envelope;
pub use crate::models::shared_docs::DocStatus;
use crate::services::junk::auth::{AuthAssessment, AuthResult};

/// Seconds without a local edit before a document's changes are mailed, so a
/// burst of typing goes out as one message.
pub const FLUSH_DEBOUNCE_SECS: i64 = 120;

/// What this install already knows of the document an envelope names.
pub struct KnownDoc<'a> {
    pub status: DocStatus,
    pub participants: &'a [String],
}

/// What to do with an arriving envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arrival {
    /// Merge it into the known document.
    Apply,
    /// A document this install has never seen: store it as an invitation.
    Invitation,
    /// Leave the document alone, for the reason given (logged).
    Ignore(&'static str),
}

/// Pure: decide what an envelope sent by `sender` to the account `me` does.
/// Addresses are compared normalized (lowercase).
pub fn plan_arrival(envelope: &Envelope, sender: &str, me: &str, known: Option<&KnownDoc<'_>>) -> Arrival {
    let sender = sender.trim().to_lowercase();
    let me = me.trim().to_lowercase();
    if !envelope.participants.contains(&sender) {
        return Arrival::Ignore("the sender is not one of its participants");
    }
    if !envelope.participants.contains(&me) {
        return Arrival::Ignore("this account is not one of its participants");
    }
    match known {
        None => Arrival::Invitation,
        Some(doc) if doc.status == DocStatus::Left => Arrival::Ignore("you left this document"),
        Some(doc) if !doc.participants.contains(&sender) => {
            Arrival::Ignore("the sender is not one of its participants")
        }
        Some(_) => Arrival::Apply,
    }
}

/// Pure: the participants after a known participant's envelope — the stored
/// ones plus any the sender added, in first-seen order.
pub fn merged_participants(stored: &[String], incoming: &[String]) -> Vec<String> {
    let mut out = stored.to_vec();
    for address in incoming {
        if !out.contains(address) {
            out.push(address.clone());
        }
    }
    out
}

/// Pure: who a document's messages go to — every participant but `me`.
pub fn recipients(participants: &[String], me: &str) -> Vec<String> {
    let me = me.trim().to_lowercase();
    participants.iter().filter(|p| **p != me).cloned().collect()
}

/// A document as the flush planner sees it.
#[derive(Debug, Clone)]
pub struct FlushCandidate {
    pub id: String,
    pub status: DocStatus,
    /// The user agreed to mail this document's changes (shared or accepted).
    pub consented: bool,
    /// When the first change not mailed yet was made; `None` when clean.
    pub dirty_since: Option<i64>,
    /// The last local change: the debounce runs from here.
    pub updated_at: i64,
}

/// Pure: the documents due to be mailed at `now` — active, consented, with
/// unsent changes and no edit for [`FLUSH_DEBOUNCE_SECS`], oldest first.
pub fn due_flushes(candidates: &[FlushCandidate], now: i64) -> Vec<String> {
    let mut due: Vec<&FlushCandidate> = candidates
        .iter()
        .filter(|c| c.status == DocStatus::Active && c.consented)
        .filter(|c| c.dirty_since.is_some() && c.updated_at + FLUSH_DEBOUNCE_SECS <= now)
        .collect();
    due.sort_by_key(|c| c.dirty_since);
    due.into_iter().map(|c| c.id.clone()).collect()
}

/// Pure: why a message must not be taken as coming from the participant its
/// `From` names, if it must not — judged on what the receiving server recorded
/// in `Authentication-Results` ([`AuthAssessment`]). Refused: the sender's
/// domain failed DMARC, or it publishes no DMARC policy and the message failed
/// SPF without a valid DKIM signature. A verdict we cannot attribute to the
/// account's own mail server proves nothing either way, so it is accepted.
pub fn sender_rejection(auth: &AuthAssessment) -> Option<&'static str> {
    if !auth.trusted {
        return None;
    }
    if auth.dmarc_hard_fail() {
        return Some("the sender's domain failed DMARC");
    }
    let no_dmarc = matches!(auth.dmarc, None | Some(AuthResult::None));
    if no_dmarc && auth.spf_hard_fail() && auth.dkim != Some(AuthResult::Pass) {
        return Some("the sender failed SPF and has no valid DKIM signature");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::shared_docs::DocKind;
    use crate::services::shared_docs::crdt;

    fn auth(
        trusted: bool,
        spf: Option<AuthResult>,
        dkim: Option<AuthResult>,
        dmarc: Option<AuthResult>,
    ) -> AuthAssessment {
        AuthAssessment {
            trusted,
            spf,
            dkim,
            dmarc,
        }
    }

    #[test]
    fn a_sender_whose_domain_fails_dmarc_is_refused() {
        use AuthResult::*;
        let cases = [
            (
                auth(true, Some(Pass), Some(Pass), Some(Pass)),
                false,
                "everything passes",
            ),
            (auth(true, Some(Pass), Some(Fail), Some(Fail)), true, "DMARC fails"),
            (
                auth(true, Some(Fail), Option::None, Some(None)),
                true,
                "no DMARC policy, SPF fails, no DKIM",
            ),
            (
                auth(true, Some(Fail), Some(Pass), Option::None),
                false,
                "SPF fails but DKIM passes",
            ),
            (
                auth(true, Some(SoftFail), Option::None, Option::None),
                false,
                "a soft fail is not proof",
            ),
            (
                auth(false, Some(Fail), Some(Fail), Some(Fail)),
                false,
                "an unattributable verdict is not proof",
            ),
            (AuthAssessment::default(), false, "no header at all"),
        ];
        for (assessment, refused, case) in cases {
            assert_eq!(sender_rejection(&assessment).is_some(), refused, "{case}");
        }
    }

    fn envelope(participants: &[&str]) -> Envelope {
        Envelope {
            purpose: crate::services::shared_docs::envelope::Purpose::Update,
            doc_id: "6f1c2a7e-3b4d-4e5f-8a9b-0c1d2e3f4a5b".into(),
            kind: DocKind::Doc,
            title: "Notes".into(),
            participants: participants.iter().map(|p| p.to_string()).collect(),
            state_vector: crdt::empty_state_vector(),
            update: crdt::empty_state(),
        }
    }

    fn owned(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn an_unknown_document_from_a_participant_is_an_invitation() {
        let env = envelope(&["ana@example.com", "ben@example.org"]);
        assert_eq!(
            plan_arrival(&env, "Ana@Example.com", "ben@example.org", None),
            Arrival::Invitation
        );
    }

    #[test]
    fn a_sender_or_receiver_outside_the_participants_is_ignored() {
        let env = envelope(&["ana@example.com", "ben@example.org"]);
        assert!(matches!(
            plan_arrival(&env, "eve@example.net", "ben@example.org", None),
            Arrival::Ignore(_)
        ));
        assert!(matches!(
            plan_arrival(&env, "ana@example.com", "carl@example.org", None),
            Arrival::Ignore(_)
        ));
    }

    #[test]
    fn a_known_document_applies_only_from_a_stored_participant() {
        let stored = owned(&["ana@example.com", "ben@example.org"]);
        let known = KnownDoc {
            status: DocStatus::Active,
            participants: &stored,
        };
        let env = envelope(&["ana@example.com", "ben@example.org", "eve@example.net"]);
        assert_eq!(
            plan_arrival(&env, "ana@example.com", "ben@example.org", Some(&known)),
            Arrival::Apply
        );
        // Eve listing herself does not make her a participant here.
        assert!(matches!(
            plan_arrival(&env, "eve@example.net", "ben@example.org", Some(&known)),
            Arrival::Ignore(_)
        ));
    }

    #[test]
    fn invited_documents_keep_applying_and_left_ones_do_not() {
        let stored = owned(&["ana@example.com", "ben@example.org"]);
        let env = envelope(&["ana@example.com", "ben@example.org"]);
        let invited = KnownDoc {
            status: DocStatus::Invited,
            participants: &stored,
        };
        let left = KnownDoc {
            status: DocStatus::Left,
            participants: &stored,
        };
        assert_eq!(
            plan_arrival(&env, "ana@example.com", "ben@example.org", Some(&invited)),
            Arrival::Apply
        );
        assert!(matches!(
            plan_arrival(&env, "ana@example.com", "ben@example.org", Some(&left)),
            Arrival::Ignore(_)
        ));
    }

    #[test]
    fn participants_grow_in_first_seen_order() {
        assert_eq!(
            merged_participants(&owned(&["a@x.com", "b@x.com"]), &owned(&["b@x.com", "c@x.com"])),
            owned(&["a@x.com", "b@x.com", "c@x.com"])
        );
    }

    #[test]
    fn recipients_are_everyone_but_me() {
        assert_eq!(
            recipients(&owned(&["a@x.com", "b@x.com", "c@x.com"]), "B@x.com"),
            owned(&["a@x.com", "c@x.com"])
        );
    }

    fn candidate(id: &str, dirty_since: Option<i64>, updated_at: i64) -> FlushCandidate {
        FlushCandidate {
            id: id.into(),
            status: DocStatus::Active,
            consented: true,
            dirty_since,
            updated_at,
        }
    }

    #[test]
    fn a_document_is_due_once_edits_have_paused() {
        let now = 1_000;
        let typing = candidate("typing", Some(800), now - 10);
        let paused = candidate("paused", Some(700), now - FLUSH_DEBOUNCE_SECS);
        let clean = candidate("clean", None, 0);
        let older = candidate("older", Some(100), 200);
        assert_eq!(
            due_flushes(&[typing, paused, clean, older], now),
            vec!["older", "paused"]
        );
    }

    #[test]
    fn nothing_is_mailed_without_consent_or_while_invited_or_left() {
        let mut no_consent = candidate("a", Some(1), 1);
        no_consent.consented = false;
        let mut invited = candidate("b", Some(1), 1);
        invited.status = DocStatus::Invited;
        let mut left = candidate("c", Some(1), 1);
        left.status = DocStatus::Left;
        assert!(due_flushes(&[no_consent, invited, left], 10_000).is_empty());
    }
}
