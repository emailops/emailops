//! Reconciliation of optimistic local Sent rows against provider-ingested
//! Sent copies.
//!
//! When a send happens on a provider that reports no canonical message id
//! (Outlook, IMAP), `send.rs` inserts a synthetic `pending_sync = 1` row so
//! the message shows up immediately. A later sync ingests the provider's
//! real Sent copy under its own id — this module matches the two and deletes
//! the synthetic row so the thread never shows the message twice.
//!
//! Pure planner ([`plan_sent_reconciliation`]) + thin executor
//! ([`reconcile_pending_sent`]); the planner is table-tested, the executor
//! is exercised by the sync integration tests.

use std::sync::Arc;

use crate::db::Database;
use crate::models::Email;

use super::events::emit_account_log;

/// One reconciliation step: delete a matched pending row and, when needed,
/// move the incoming provider row into the local conversation's thread.
#[derive(Debug, Clone, PartialEq)]
pub struct ReconcileAction {
    /// Pending (synthetic) row to delete.
    pub delete_pending_id: String,
    /// `Some((incoming_id, pending_thread_id))` when the provider's Sent copy
    /// landed under a different thread_id than the local conversation (IMAP
    /// References-hash divergence, Outlook new-mail conversationId). The
    /// incoming row adopts the local thread so the reply stays visible in the
    /// open thread view.
    pub adopt_thread: Option<(String, String)>,
}

/// Maximum clock skew between the optimistic row's local timestamp and the
/// provider's Sent-copy timestamp for the heuristic match.
const HEURISTIC_WINDOW_SECS: i64 = 600;

/// Match pending optimistic rows against freshly ingested provider rows.
///
/// - `pending`: current `pending_sync = 1` rows for the account.
/// - `incoming`: rows just handed to `insert_emails_batch` in this chunk.
/// - `account_email`: the account's own address — only the user's own
///   outgoing mail is a candidate.
///
/// Two passes: exact RFC `Message-ID` equality (IMAP — our MIME's Message-ID
/// survives the round-trip), then a conservative heuristic for pending rows
/// with no Message-ID (Outlook): normalized subject + identical recipient
/// address set + timestamps within [`HEURISTIC_WINDOW_SECS`]. Candidate
/// pairs are assigned one-to-one, closest-in-time first, so two similar
/// replies sent in quick succession each consume exactly one incoming copy.
/// An unmatched pending row is never deleted.
pub fn plan_sent_reconciliation(pending: &[Email], incoming: &[Email], account_email: &str) -> Vec<ReconcileAction> {
    let account_email = account_email.to_lowercase();
    let candidates: Vec<&Email> = incoming
        .iter()
        .filter(|e| e.sender_email.to_lowercase() == account_email)
        .collect();
    if pending.is_empty() || candidates.is_empty() {
        return Vec::new();
    }

    let mut used_pending: Vec<bool> = vec![false; pending.len()];
    let mut used_incoming: Vec<bool> = vec![false; candidates.len()];
    let mut actions = Vec::new();

    // Pass 1: exact Message-ID match.
    for (pi, p) in pending.iter().enumerate() {
        let Some(pmid) = normalized_message_id(p) else { continue };
        for (ci, c) in candidates.iter().enumerate() {
            if used_incoming[ci] {
                continue;
            }
            if normalized_message_id(c).as_deref() == Some(pmid.as_str()) {
                used_pending[pi] = true;
                used_incoming[ci] = true;
                actions.push(action_for(p, c));
                break;
            }
        }
    }

    // Pass 2: heuristic for pending rows without a Message-ID. Collect every
    // plausible pair, then assign greedily by |Δt| (deterministic tiebreak on
    // ids) so each pending and each incoming is consumed at most once.
    let mut pairs: Vec<(i64, usize, usize)> = Vec::new();
    for (pi, p) in pending.iter().enumerate() {
        if used_pending[pi] || normalized_message_id(p).is_some() {
            continue;
        }
        for (ci, c) in candidates.iter().enumerate() {
            if used_incoming[ci] {
                continue;
            }
            let dt = (c.timestamp - p.timestamp).abs();
            if dt <= HEURISTIC_WINDOW_SECS
                && normalize_subject(&p.subject) == normalize_subject(&c.subject)
                && recipient_set(&p.recipients) == recipient_set(&c.recipients)
            {
                pairs.push((dt, pi, ci));
            }
        }
    }
    pairs.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| pending[a.1].id.cmp(&pending[b.1].id))
            .then_with(|| candidates[a.2].id.cmp(&candidates[b.2].id))
    });
    for (_dt, pi, ci) in pairs {
        if used_pending[pi] || used_incoming[ci] {
            continue;
        }
        used_pending[pi] = true;
        used_incoming[ci] = true;
        actions.push(action_for(&pending[pi], candidates[ci]));
    }

    actions
}

/// Thin executor: match the account's pending optimistic rows against the
/// rows just written by `insert_emails_batch` and apply the plan. Errors are
/// logged and never fail the sync — an unreconciled pending row is unflagged
/// by the stale sweep after 24h anyway.
pub fn reconcile_pending_sent(db: &Arc<Database>, account_id: &str, account_email: &str, just_inserted: &[Email]) {
    let pending = match db.get_pending_sent_emails(account_id) {
        Ok(p) => p,
        Err(e) => {
            emit_account_log(
                "error",
                "sync",
                account_email,
                &format!("Could not load pending sent copies for reconciliation: {e}"),
            );
            return;
        }
    };
    if pending.is_empty() {
        return;
    }

    let actions = plan_sent_reconciliation(&pending, just_inserted, account_email);
    if actions.is_empty() {
        return;
    }

    for action in &actions {
        if let Some((incoming_id, thread_id)) = &action.adopt_thread {
            if let Err(e) = db.update_email_thread_id(incoming_id, thread_id) {
                emit_account_log(
                    "error",
                    "sync",
                    account_email,
                    &format!("Could not move sent copy {incoming_id} into thread {thread_id}: {e}"),
                );
            }
        }
    }
    let ids: Vec<String> = actions.iter().map(|a| a.delete_pending_id.clone()).collect();
    match db.delete_pending_sent_emails(&ids) {
        Ok(()) => emit_account_log(
            "debug",
            "sync",
            account_email,
            &format!(
                "Reconciled {} locally stored sent cop{}",
                ids.len(),
                if ids.len() == 1 { "y" } else { "ies" }
            ),
        ),
        Err(e) => emit_account_log(
            "error",
            "sync",
            account_email,
            &format!("Could not delete reconciled sent copies: {e}"),
        ),
    }
}

fn action_for(pending: &Email, incoming: &Email) -> ReconcileAction {
    ReconcileAction {
        delete_pending_id: pending.id.clone(),
        adopt_thread: (incoming.thread_id != pending.thread_id)
            .then(|| (incoming.id.clone(), pending.thread_id.clone())),
    }
}

fn normalized_message_id(email: &Email) -> Option<String> {
    email
        .message_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Casefold and repeatedly strip `re:` / `fw:` / `fwd:` prefixes so
/// "Re: RE: Budget" and "Budget" compare equal.
fn normalize_subject(subject: &str) -> String {
    let mut s = subject.trim().to_lowercase();
    loop {
        let stripped = ["re:", "fwd:", "fw:"]
            .iter()
            .find_map(|p| s.strip_prefix(p))
            .map(|rest| rest.trim_start().to_string());
        match stripped {
            Some(rest) => s = rest,
            None => break,
        }
    }
    s
}

/// Bare lowercase addresses as a set. Provider-ingested recipient entries may
/// be display forms (`"Ada L <ada@x.com>"`) while the pending row holds bare
/// addresses — extract the `<...>` part when present.
fn recipient_set(recipients: &[String]) -> std::collections::BTreeSet<String> {
    recipients.iter().map(|r| bare_address(r)).collect()
}

fn bare_address(recipient: &str) -> String {
    let r = recipient.trim();
    match (r.rfind('<'), r.rfind('>')) {
        (Some(start), Some(end)) if start < end => r[start + 1..end].trim().to_lowercase(),
        _ => r.to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email(id: &str, thread_id: &str, message_id: Option<&str>, subject: &str, to: &[&str], ts: i64) -> Email {
        Email {
            id: id.to_string(),
            account_id: "acc".to_string(),
            thread_id: thread_id.to_string(),
            message_id: message_id.map(str::to_string),
            subject: subject.to_string(),
            sender: "Me".to_string(),
            sender_email: "me@example.com".to_string(),
            recipients: to.iter().map(|s| s.to_string()).collect(),
            cc: vec![],
            body: String::new(),
            snippet: String::new(),
            timestamp: ts,
            is_read: true,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: "sent".to_string(),
        }
    }

    #[test]
    fn exact_message_id_match_wins() {
        let pending = [email("local-1", "t-local", Some("<m1@local>"), "Hi", &["a@x.com"], 100)];
        let incoming = [email("imap-9", "t-local", Some("<m1@local>"), "Hi", &["a@x.com"], 105)];
        let actions = plan_sent_reconciliation(&pending, &incoming, "me@example.com");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].delete_pending_id, "local-1");
        assert!(actions[0].adopt_thread.is_none(), "same thread → no adoption");
    }

    #[test]
    fn message_id_match_ignores_subject_and_time() {
        // Exact identity beats every heuristic precondition.
        let pending = [email("local-1", "t-local", Some("<m1@local>"), "Hi", &["a@x.com"], 100)];
        let incoming = [email(
            "imap-9",
            "t-local",
            Some("<m1@local>"),
            "Completely different subject",
            &["other@y.com"],
            99_999,
        )];
        let actions = plan_sent_reconciliation(&pending, &incoming, "me@example.com");
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn incoming_from_other_senders_is_ignored() {
        let pending = [email("local-1", "t", Some("<m1@local>"), "Hi", &["a@x.com"], 100)];
        let mut other = email("in-1", "t", Some("<m1@local>"), "Hi", &["a@x.com"], 100);
        other.sender_email = "someone-else@example.com".to_string();
        let actions = plan_sent_reconciliation(&pending, &[other], "me@example.com");
        assert!(actions.is_empty(), "only the account's own mail reconciles");
    }

    #[test]
    fn heuristic_matches_subject_recipients_and_window() {
        let pending = [email("local-1", "t-conv", None, "Re: Budget", &["a@x.com"], 1000)];
        let incoming = [email(
            "gr-1",
            "t-conv",
            Some("<graph@server>"),
            "RE: Budget",
            &["Ada L <A@X.com>"],
            1300,
        )];
        let actions = plan_sent_reconciliation(&pending, &incoming, "me@example.com");
        assert_eq!(
            actions.len(),
            1,
            "subject casefold + display-name recipients must match"
        );
        assert_eq!(actions[0].delete_pending_id, "local-1");
    }

    #[test]
    fn heuristic_rejects_outside_time_window() {
        let pending = [email("local-1", "t", None, "Budget", &["a@x.com"], 1000)];
        let incoming = [email("gr-1", "t", None, "Budget", &["a@x.com"], 1000 + 601)];
        let actions = plan_sent_reconciliation(&pending, &incoming, "me@example.com");
        assert!(actions.is_empty());
    }

    #[test]
    fn heuristic_rejects_different_recipients() {
        let pending = [email("local-1", "t", None, "Budget", &["a@x.com"], 1000)];
        let incoming = [email("gr-1", "t", None, "Budget", &["a@x.com", "b@y.com"], 1005)];
        let actions = plan_sent_reconciliation(&pending, &incoming, "me@example.com");
        assert!(actions.is_empty(), "recipient sets must be identical");
    }

    #[test]
    fn closest_timestamp_wins_one_to_one() {
        // Two quick replies with identical subject/recipients: each incoming
        // copy consumes exactly one pending row, closest Δt first.
        let pending = [
            email("local-a", "t", None, "Re: Budget", &["a@x.com"], 1000),
            email("local-b", "t", None, "Re: Budget", &["a@x.com"], 1100),
        ];
        let incoming = [
            email("gr-a", "t", None, "Re: Budget", &["a@x.com"], 1010),
            email("gr-b", "t", None, "Re: Budget", &["a@x.com"], 1110),
        ];
        let actions = plan_sent_reconciliation(&pending, &incoming, "me@example.com");
        assert_eq!(actions.len(), 2, "both pending rows reconcile");
        let deleted: std::collections::BTreeSet<&str> = actions.iter().map(|a| a.delete_pending_id.as_str()).collect();
        assert!(deleted.contains("local-a") && deleted.contains("local-b"));
    }

    #[test]
    fn single_incoming_consumes_only_one_of_two_pending() {
        let pending = [
            email("local-a", "t", None, "Re: Budget", &["a@x.com"], 1000),
            email("local-b", "t", None, "Re: Budget", &["a@x.com"], 1100),
        ];
        let incoming = [email("gr-a", "t", None, "Re: Budget", &["a@x.com"], 1090)];
        let actions = plan_sent_reconciliation(&pending, &incoming, "me@example.com");
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].delete_pending_id, "local-b",
            "closest in time (Δ10 vs Δ90) wins"
        );
    }

    #[test]
    fn unmatched_pending_survives() {
        let pending = [email("local-1", "t", None, "Budget", &["a@x.com"], 1000)];
        let incoming = [email("gr-1", "t", None, "Other topic", &["a@x.com"], 1005)];
        let actions = plan_sent_reconciliation(&pending, &incoming, "me@example.com");
        assert!(actions.is_empty(), "never delete without a confident match");
    }

    #[test]
    fn divergent_thread_id_triggers_adoption() {
        // IMAP hashes References → the Sent copy of a mid-thread reply can
        // land in a different thread. The incoming row must adopt the local
        // conversation's thread id.
        let pending = [email(
            "local-1",
            "t-conversation",
            Some("<m1@local>"),
            "Re: Plan",
            &["a@x.com"],
            100,
        )];
        let incoming = [email(
            "imap-sent-7",
            "t-divergent-hash",
            Some("<m1@local>"),
            "Re: Plan",
            &["a@x.com"],
            103,
        )];
        let actions = plan_sent_reconciliation(&pending, &incoming, "me@example.com");
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].adopt_thread,
            Some(("imap-sent-7".to_string(), "t-conversation".to_string()))
        );
    }

    #[test]
    fn subject_normalization_strips_stacked_prefixes() {
        assert_eq!(normalize_subject("Re: RE: fwd: Budget  "), "budget");
        assert_eq!(normalize_subject("Budget"), "budget");
        assert_eq!(
            normalize_subject("REPORT"),
            "report",
            "words starting with 're' survive"
        );
    }

    #[test]
    fn bare_address_extracts_display_forms() {
        assert_eq!(bare_address("Ada L <Ada@X.com>"), "ada@x.com");
        assert_eq!(bare_address("  plain@x.com "), "plain@x.com");
    }
}
