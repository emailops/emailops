// Starter requests for an empty chat conversation.
//
// The chat suggests a handful of requests the user can click: fixed ones
// (today's mail, this week, a draft) plus ones built from the account's own
// mail (a frequent recent correspondent, a recent thread). No model call — the
// mailbox-derived ones come straight from SQLite, so the list shows instantly.
//
// The backend picks *what* to suggest; the frontend words it in the user's
// language from `kind` and the parameters, so the text stays in the locales.

use serde::Serialize;

use crate::db::Database;
use crate::models::error::Result;
use crate::util::email_addr::classify_kind;

pub use crate::db::emails::chat_starters::{SenderCandidate, ThreadCandidate};

/// How far back the mailbox-derived starters look.
const WINDOW_SECS: i64 = 30 * 24 * 60 * 60;

/// A thread subject longer than this is cut, so the suggestion fits one row.
pub const MAX_SUBJECT_CHARS: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StarterKind {
    Today,
    Sender,
    Thread,
    ThisWeek,
    Draft,
}

/// `Send` sends the request as soon as it is clicked; `Prefill` puts a
/// sentence stem in the input for the user to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StarterAction {
    Send,
    Prefill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStarter {
    pub kind: StarterKind,
    pub action: StarterAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

/// The starters for an empty conversation on `account_id`, read from mail in
/// the chat's category scope over the last 30 days.
pub fn chat_starters(db: &Database, account_id: &str) -> Result<Vec<ChatStarter>> {
    chat_starters_at(db, account_id, crate::services::clock::now_secs())
}

pub fn chat_starters_at(db: &Database, account_id: &str, now: i64) -> Result<Vec<ChatStarter>> {
    let categories = super::default_categories(db);
    let since = now - WINDOW_SECS;
    let own = db.user_addresses(account_id)?;
    let senders = db.chat_starter_senders(account_id, &categories, &own, since)?;
    let threads = db.chat_starter_threads(account_id, &categories, &own, since)?;
    Ok(plan_chat_starters(&senders, &threads, &own))
}

/// Pick the starters to show. `senders` and `threads` come from the mailbox
/// (any order for senders; threads newest first); `own_addresses` are the
/// account owner's addresses, lowercased.
pub fn plan_chat_starters(
    senders: &[SenderCandidate],
    threads: &[ThreadCandidate],
    own_addresses: &[String],
) -> Vec<ChatStarter> {
    let is_person = |email: &str| {
        let email = email.trim().to_lowercase();
        !email.is_empty() && !own_addresses.contains(&email) && classify_kind(&email) == "person"
    };

    let mut ranked: Vec<&SenderCandidate> = senders.iter().filter(|s| is_person(&s.email)).collect();
    ranked.sort_by(|a, b| b.threads.cmp(&a.threads).then_with(|| a.email.cmp(&b.email)));
    let top = ranked.first();
    let sender = top.map(|s| {
        let name = s.name.trim();
        if name.is_empty() { s.email.trim() } else { name }.to_string()
    });

    // Prefer a thread from someone else, so the two mailbox starters do not
    // both point at the same person; fall back to theirs when it is the only one.
    let eligible: Vec<(&ThreadCandidate, String)> = threads
        .iter()
        .filter(|t| is_person(&t.sender_email))
        .map(|t| (t, clean_subject(&t.subject)))
        .filter(|(_, s)| !s.is_empty())
        .collect();
    let same_person =
        |t: &ThreadCandidate| top.is_some_and(|s| s.email.trim().eq_ignore_ascii_case(t.sender_email.trim()));
    let subject = eligible
        .iter()
        .find(|(t, _)| !same_person(t))
        .or_else(|| eligible.first())
        .map(|(_, s)| s.clone());

    let mut starters = vec![fixed(StarterKind::Today, StarterAction::Send)];
    if let Some(name) = sender {
        starters.push(ChatStarter {
            sender_name: Some(name),
            ..fixed(StarterKind::Sender, StarterAction::Send)
        });
    }
    if let Some(subject) = subject {
        starters.push(ChatStarter {
            subject: Some(subject),
            ..fixed(StarterKind::Thread, StarterAction::Send)
        });
    }
    starters.push(fixed(StarterKind::ThisWeek, StarterAction::Send));
    starters.push(fixed(StarterKind::Draft, StarterAction::Prefill));
    starters
}

fn fixed(kind: StarterKind, action: StarterAction) -> ChatStarter {
    ChatStarter {
        kind,
        action,
        sender_name: None,
        subject: None,
    }
}

/// Drop leading reply/forward markers (English, Spanish, German) and cut a
/// long subject to `MAX_SUBJECT_CHARS` characters plus an ellipsis.
fn clean_subject(raw: &str) -> String {
    const PREFIXES: &[&str] = &["re:", "fw:", "fwd:", "rv:", "aw:", "wg:", "tr:"];
    let mut s = raw.trim();
    while let Some(prefix) = PREFIXES
        .iter()
        .find(|p| s.get(..p.len()).is_some_and(|head| head.eq_ignore_ascii_case(p)))
    {
        s = s[prefix.len()..].trim_start();
    }
    let s = s.trim();
    if s.chars().count() > MAX_SUBJECT_CHARS {
        let cut: String = s.chars().take(MAX_SUBJECT_CHARS).collect();
        format!("{}…", cut.trim_end())
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sender(email: &str, name: &str, threads: u32) -> SenderCandidate {
        SenderCandidate {
            email: email.to_string(),
            name: name.to_string(),
            threads,
        }
    }

    fn thread(subject: &str, sender_email: &str) -> ThreadCandidate {
        ThreadCandidate {
            subject: subject.to_string(),
            sender_email: sender_email.to_string(),
        }
    }

    fn kinds(starters: &[ChatStarter]) -> Vec<StarterKind> {
        starters.iter().map(|s| s.kind).collect()
    }

    fn own() -> Vec<String> {
        vec!["me@example.com".to_string()]
    }

    #[test]
    fn the_executor_suggests_a_recent_correspondent_and_skips_the_owner() {
        let db = Database::new_for_testing().unwrap();
        let now = 1_000_000_000;
        let conn = db.connection();
        conn.execute(
            "INSERT INTO accounts (id, provider, email, name, created_at)
             VALUES ('acct', 'gmail', 'me@example.com', 'Me', 0)",
            [],
        )
        .unwrap();
        for (id, thread, sender, subject, ts) in [
            ("1", "t1", "ana@acme.test", "Kickoff agenda", now - 3600),
            ("2", "t1", "me@example.com", "Re: Kickoff agenda", now - 1800),
            ("4", "t2", "me@example.com", "Note to self", now - 60),
            ("3", "t3", "old@acme.test", "Ancient", now - WINDOW_SECS - 1),
        ] {
            conn.execute(
                "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                 VALUES (?1,'acct',?2,?3,'Ana Ruiz',?4,'acme.test','[]','[]','snip',?5,0,'primary','inbox',0)",
                rusqlite::params![id, thread, subject, sender, ts],
            )
            .unwrap();
        }
        drop(conn);

        let starters = chat_starters_at(&db, "acct", now).unwrap();
        assert_eq!(starters[1].sender_name.as_deref(), Some("Ana Ruiz"));
        assert_eq!(starters[2].subject.as_deref(), Some("Kickoff agenda"));
        assert_eq!(starters.len(), 5);
    }

    #[test]
    fn an_empty_mailbox_gets_only_the_fixed_starters() {
        let starters = plan_chat_starters(&[], &[], &own());
        assert_eq!(
            kinds(&starters),
            vec![StarterKind::Today, StarterKind::ThisWeek, StarterKind::Draft]
        );
    }

    #[test]
    fn mailbox_starters_sit_between_today_and_this_week() {
        let starters = plan_chat_starters(
            &[sender("ana@acme.test", "Ana Ruiz", 3)],
            &[thread("Q3 budget", "bob@acme.test")],
            &own(),
        );
        assert_eq!(
            kinds(&starters),
            vec![
                StarterKind::Today,
                StarterKind::Sender,
                StarterKind::Thread,
                StarterKind::ThisWeek,
                StarterKind::Draft
            ]
        );
    }

    #[test]
    fn only_the_draft_starter_prefills() {
        let starters = plan_chat_starters(
            &[sender("ana@acme.test", "Ana Ruiz", 3)],
            &[thread("Q3 budget", "bob@acme.test")],
            &own(),
        );
        for s in &starters {
            let expected = if s.kind == StarterKind::Draft {
                StarterAction::Prefill
            } else {
                StarterAction::Send
            };
            assert_eq!(s.action, expected, "{:?}", s.kind);
        }
    }

    #[test]
    fn the_sender_with_most_threads_is_suggested() {
        let starters = plan_chat_starters(
            &[
                sender("ana@acme.test", "Ana Ruiz", 2),
                sender("bob@acme.test", "Bob Lee", 5),
                sender("cy@acme.test", "Cy Moss", 1),
            ],
            &[],
            &own(),
        );
        assert_eq!(starters[1].sender_name.as_deref(), Some("Bob Lee"));
    }

    #[test]
    fn a_thread_count_tie_is_broken_by_address_so_the_pick_is_stable() {
        let starters = plan_chat_starters(
            &[sender("zoe@acme.test", "Zoe", 2), sender("ana@acme.test", "Ana", 2)],
            &[],
            &own(),
        );
        assert_eq!(starters[1].sender_name.as_deref(), Some("Ana"));
    }

    #[test]
    fn the_owner_is_never_suggested_as_a_sender() {
        let starters = plan_chat_starters(
            &[sender("Me@Example.com", "Me", 9), sender("ana@acme.test", "Ana", 1)],
            &[],
            &own(),
        );
        assert_eq!(starters[1].sender_name.as_deref(), Some("Ana"));
    }

    #[test]
    fn automated_senders_are_never_suggested() {
        let starters = plan_chat_starters(
            &[
                sender("noreply@shop.test", "Shop", 9),
                sender("newsletter@news.test", "News", 8),
                sender("ana@acme.test", "Ana", 1),
            ],
            &[],
            &own(),
        );
        assert_eq!(starters[1].sender_name.as_deref(), Some("Ana"));
    }

    #[test]
    fn no_sender_starter_when_every_candidate_is_excluded() {
        let starters = plan_chat_starters(&[sender("noreply@shop.test", "Shop", 9)], &[], &own());
        assert!(!kinds(&starters).contains(&StarterKind::Sender));
    }

    #[test]
    fn a_sender_without_a_display_name_is_named_by_address() {
        let starters = plan_chat_starters(&[sender("ana@acme.test", "  ", 1)], &[], &own());
        assert_eq!(starters[1].sender_name.as_deref(), Some("ana@acme.test"));
    }

    #[test]
    fn the_newest_thread_is_suggested() {
        let starters = plan_chat_starters(
            &[],
            &[
                thread("Kickoff agenda", "ana@acme.test"),
                thread("Older", "bob@acme.test"),
            ],
            &own(),
        );
        assert_eq!(starters[1].subject.as_deref(), Some("Kickoff agenda"));
    }

    #[test]
    fn threads_from_the_owner_or_automated_senders_are_skipped() {
        let starters = plan_chat_starters(
            &[],
            &[
                thread("My own note", "me@example.com"),
                thread("Your order shipped", "no-reply@shop.test"),
                thread("Kickoff agenda", "ana@acme.test"),
            ],
            &own(),
        );
        assert_eq!(starters[1].subject.as_deref(), Some("Kickoff agenda"));
    }

    #[test]
    fn reply_and_forward_prefixes_are_stripped_from_the_subject() {
        for (raw, clean) in [
            ("Re: Kickoff agenda", "Kickoff agenda"),
            ("RE: Fwd: Kickoff agenda", "Kickoff agenda"),
            ("RV: Presupuesto", "Presupuesto"),
            ("AW: Angebot", "Angebot"),
            ("FW:  Kickoff", "Kickoff"),
        ] {
            let starters = plan_chat_starters(&[], &[thread(raw, "ana@acme.test")], &own());
            assert_eq!(starters[1].subject.as_deref(), Some(clean), "{raw}");
        }
    }

    #[test]
    fn the_thread_comes_from_someone_other_than_the_suggested_sender_when_it_can() {
        let starters = plan_chat_starters(
            &[sender("ana@acme.test", "Ana", 3)],
            &[
                thread("Ana's newest", "ana@acme.test"),
                thread("Bob's thread", "bob@acme.test"),
            ],
            &own(),
        );
        assert_eq!(starters[2].subject.as_deref(), Some("Bob's thread"));
    }

    #[test]
    fn the_suggested_sender_s_thread_is_used_when_there_is_no_other() {
        let starters = plan_chat_starters(
            &[sender("ana@acme.test", "Ana", 3)],
            &[thread("Ana's newest", "ana@acme.test")],
            &own(),
        );
        assert_eq!(starters[2].subject.as_deref(), Some("Ana's newest"));
    }

    #[test]
    fn a_thread_with_an_empty_subject_is_skipped() {
        let starters = plan_chat_starters(
            &[],
            &[
                thread("Re:  ", "ana@acme.test"),
                thread("Kickoff agenda", "bob@acme.test"),
            ],
            &own(),
        );
        assert_eq!(starters[1].subject.as_deref(), Some("Kickoff agenda"));
    }

    #[test]
    fn a_long_subject_is_cut_on_a_char_boundary_with_an_ellipsis() {
        let long = "ñ".repeat(80);
        let starters = plan_chat_starters(&[], &[thread(&long, "ana@acme.test")], &own());
        let subject = starters[1].subject.clone().unwrap_or_default();
        assert_eq!(subject.chars().count(), MAX_SUBJECT_CHARS + 1);
        assert!(subject.ends_with('…'));
    }
}
