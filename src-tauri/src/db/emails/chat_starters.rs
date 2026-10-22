//! Mailbox reads behind the chat's starter requests
//! (`services::chat::starters`): recent correspondents and recent threads in
//! the chat's category scope. Inbound inbox mail only, inside a time window,
//! so both reads stay bounded on a large mailbox.
//!
//! Only threads the owner also wrote in count: a conversation, not a mailing
//! list. That keeps newsletters, metrics reports and notifications out without
//! a list of sender names to maintain.

use crate::db::Database;
use crate::models::error::Result;

/// How many emails the recent-thread read looks at before it stops.
const THREAD_SCAN_LIMIT: i64 = 200;
/// How many recent threads it returns.
const THREAD_LIMIT: usize = 20;
/// How many correspondents the sender read returns.
const SENDER_LIMIT: i64 = 20;

/// A correspondent seen recently in the chat's category scope, with the
/// number of distinct threads they wrote in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderCandidate {
    pub email: String,
    pub name: String,
    pub threads: u32,
}

/// A recent inbound thread in the chat's category scope, described by its
/// newest inbound email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadCandidate {
    pub thread_id: String,
    pub subject: String,
    pub sender_email: String,
    pub sender_name: String,
    /// The owner has not written in the thread since that email.
    pub awaiting_reply: bool,
}

impl Database {
    /// Correspondents of inbox mail received since `since` (unix seconds) in
    /// `categories`, ranked by the threads they share with the owner
    /// (`own_addresses`, lowercased).
    pub fn chat_starter_senders(
        &self,
        account_id: &str,
        categories: &[String],
        own_addresses: &[String],
        since: i64,
    ) -> Result<Vec<SenderCandidate>> {
        if own_addresses.is_empty() {
            return Ok(Vec::new());
        }
        let (scope, mut args) = scope_clause(account_id, categories, own_addresses, since);
        let sql = format!(
            "SELECT lower(trim(sender_email)) AS addr, MAX(sender), COUNT(DISTINCT thread_id) AS n
             FROM emails WHERE {scope}
             GROUP BY addr ORDER BY n DESC, addr LIMIT ?{}",
            args.len() + 1
        );
        args.push(Box::new(SENDER_LIMIT));
        let conn = self.reader();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())), |row| {
            Ok(SenderCandidate {
                email: row.get(0)?,
                name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                threads: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Inbox threads the owner wrote in, received since `since` in
    /// `categories`, newest first, one entry per thread (its newest email).
    pub fn chat_starter_threads(
        &self,
        account_id: &str,
        categories: &[String],
        own_addresses: &[String],
        since: i64,
    ) -> Result<Vec<ThreadCandidate>> {
        if own_addresses.is_empty() {
            return Ok(Vec::new());
        }
        let (scope, mut args) = scope_clause(account_id, categories, own_addresses, since);
        // The owner's later messages can sit in any mailbox (Sent, archive), so
        // this looks across the whole thread, not just the inbox scope.
        let answered = format!(
            "EXISTS (SELECT 1 FROM emails o WHERE o.account_id = ?1 AND o.thread_id = emails.thread_id
                     AND o.is_deleted = 0 AND o.timestamp > emails.timestamp AND {})",
            owner_match(
                "o.sender_email",
                owner_first_param(&args, own_addresses),
                own_addresses.len()
            )
        );
        let sql = format!(
            "SELECT thread_id, subject, sender_email, sender, NOT {answered} FROM emails WHERE {scope}
             ORDER BY timestamp DESC, id DESC LIMIT ?{}",
            args.len() + 1
        );
        args.push(Box::new(THREAD_SCAN_LIMIT));
        let conn = self.reader();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())), |row| {
            Ok(ThreadCandidate {
                thread_id: row.get(0)?,
                subject: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                sender_email: row.get(2)?,
                sender_name: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                awaiting_reply: row.get(4)?,
            })
        })?;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for row in rows {
            let candidate = row?;
            if seen.insert(candidate.thread_id.clone()) {
                out.push(candidate);
                if out.len() == THREAD_LIMIT {
                    break;
                }
            }
        }
        Ok(out)
    }
}

/// The shared WHERE clause: this account's live inbound inbox mail since
/// `since` in `categories`, in threads one of `own_addresses` wrote in, with
/// its positional arguments.
fn scope_clause(
    account_id: &str,
    categories: &[String],
    own_addresses: &[String],
    since: i64,
) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.to_string()), Box::new(since)];
    let mut clause = String::from("account_id = ?1 AND is_deleted = 0 AND mailbox = 'inbox' AND timestamp >= ?2");
    if !categories.is_empty() {
        let placeholders: Vec<String> = (0..categories.len())
            .map(|i| format!("?{}", args.len() + 1 + i))
            .collect();
        clause.push_str(&format!(" AND category IN ({})", placeholders.join(", ")));
        for c in categories {
            args.push(Box::new(c.clone()));
        }
    }
    let by_owner = owner_match("sender_email", args.len() + 1, own_addresses.len());
    clause.push_str(&format!(
        " AND NOT {by_owner} AND thread_id IN (SELECT thread_id FROM emails WHERE account_id = ?1 AND {by_owner})"
    ));
    for a in own_addresses {
        args.push(Box::new(a.clone()));
    }
    (clause, args)
}

/// `(<column> = ?first COLLATE NOCASE OR …)` over `count` owner addresses.
/// The `COLLATE NOCASE` form keeps idx_emails_sender_email_nocase usable.
fn owner_match(column: &str, first: usize, count: usize) -> String {
    let parts: Vec<String> = (0..count)
        .map(|i| format!("{column} = ?{} COLLATE NOCASE", first + i))
        .collect();
    format!("({})", parts.join(" OR "))
}

/// Where `scope_clause` put the owner addresses: they are its last arguments.
fn owner_first_param(args: &[Box<dyn rusqlite::ToSql>], own_addresses: &[String]) -> usize {
    args.len() - own_addresses.len() + 1
}

#[cfg(test)]
mod tests {
    use crate::db::Database;

    const ME: &str = "me@example.com";

    fn insert(
        db: &Database,
        id: &str,
        thread: &str,
        sender: &str,
        subject: &str,
        ts: i64,
        category: &str,
        mailbox: &str,
    ) {
        let conn = db.connection();
        super::super::test_helpers::ensure_account(&conn, "acct");
        conn.execute(
            "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
             VALUES (?1,'acct',?2,?3,?4,?5,'acme.test','[]','[]','snip',?6,0,?7,?8,0)",
            rusqlite::params![
                id,
                thread,
                subject,
                format!("Name {sender}"),
                sender,
                ts,
                category,
                mailbox
            ],
        )
        .unwrap();
    }

    /// The owner's reply in `thread`, stored the way IMAP and Gmail threading
    /// often keep it: in the inbox, not flagged as sent.
    fn reply(db: &Database, id: &str, thread: &str) {
        insert(db, id, thread, "Me@Example.com", "Re", 90, "primary", "inbox");
    }

    fn primary() -> Vec<String> {
        vec!["primary".to_string()]
    }

    fn own() -> Vec<String> {
        vec![ME.to_string()]
    }

    #[test]
    fn senders_are_ranked_by_threads_the_owner_also_wrote_in() {
        let db = Database::new_for_testing().unwrap();
        insert(&db, "1", "t1", "ana@acme.test", "a", 100, "primary", "inbox");
        insert(&db, "2", "t1", "ana@acme.test", "a", 110, "primary", "inbox");
        insert(&db, "3", "t2", "bob@acme.test", "b", 120, "primary", "inbox");
        insert(&db, "4", "t3", "bob@acme.test", "c", 130, "primary", "inbox");
        for (id, t) in [("r1", "t1"), ("r2", "t2"), ("r3", "t3")] {
            reply(&db, id, t);
        }
        let senders = db.chat_starter_senders("acct", &primary(), &own(), 50).unwrap();
        let got: Vec<(&str, u32)> = senders.iter().map(|s| (s.email.as_str(), s.threads)).collect();
        assert_eq!(got, vec![("bob@acme.test", 2), ("ana@acme.test", 1)]);
        assert_eq!(senders[0].name, "Name bob@acme.test");
    }

    #[test]
    fn a_sender_the_owner_never_wrote_back_to_is_not_a_correspondent() {
        let db = Database::new_for_testing().unwrap();
        insert(&db, "1", "t1", "digest@news.test", "Weekly", 100, "primary", "inbox");
        insert(&db, "2", "t2", "digest@news.test", "Weekly", 110, "primary", "inbox");
        insert(&db, "3", "t3", "ana@acme.test", "a", 120, "primary", "inbox");
        reply(&db, "r3", "t3");
        let senders = db.chat_starter_senders("acct", &primary(), &own(), 50).unwrap();
        let got: Vec<&str> = senders.iter().map(|s| s.email.as_str()).collect();
        assert_eq!(got, vec!["ana@acme.test"]);
    }

    #[test]
    fn senders_outside_the_window_scope_or_inbox_are_ignored() {
        let db = Database::new_for_testing().unwrap();
        insert(&db, "1", "t1", "old@acme.test", "a", 10, "primary", "inbox");
        insert(&db, "2", "t2", "promo@acme.test", "a", 100, "promotions", "inbox");
        insert(&db, "3", "t3", "spam@acme.test", "a", 100, "primary", "spam");
        insert(&db, "4", "t4", "ana@acme.test", "a", 100, "primary", "inbox");
        db.connection()
            .execute("UPDATE emails SET is_deleted = 1 WHERE id = '4'", [])
            .unwrap();
        insert(&db, "5", "t5", "cy@acme.test", "a", 100, "primary", "inbox");
        for (id, t) in [("r1", "t1"), ("r2", "t2"), ("r3", "t3"), ("r4", "t4"), ("r5", "t5")] {
            reply(&db, id, t);
        }
        let senders = db.chat_starter_senders("acct", &primary(), &own(), 50).unwrap();
        let got: Vec<&str> = senders.iter().map(|s| s.email.as_str()).collect();
        assert_eq!(got, vec!["cy@acme.test"]);
    }

    #[test]
    fn threads_come_newest_first_once_each() {
        let db = Database::new_for_testing().unwrap();
        insert(&db, "1", "t1", "ana@acme.test", "Old one", 100, "primary", "inbox");
        insert(&db, "2", "t2", "bob@acme.test", "Middle", 110, "primary", "inbox");
        insert(&db, "3", "t1", "ana@acme.test", "Re: Old one", 120, "primary", "inbox");
        insert(&db, "4", "t3", "cy@acme.test", "Promo", 130, "promotions", "inbox");
        for (id, t) in [("r1", "t1"), ("r2", "t2"), ("r3", "t3")] {
            reply(&db, id, t);
        }
        let threads = db.chat_starter_threads("acct", &primary(), &own(), 50).unwrap();
        let got: Vec<&str> = threads.iter().map(|t| t.subject.as_str()).collect();
        assert_eq!(got, vec!["Re: Old one", "Middle"]);
    }

    #[test]
    fn a_thread_the_owner_never_wrote_in_is_skipped() {
        let db = Database::new_for_testing().unwrap();
        insert(&db, "1", "t1", "ana@acme.test", "Talked", 100, "primary", "inbox");
        insert(&db, "2", "t2", "digest@news.test", "Weekly", 110, "primary", "inbox");
        reply(&db, "r1", "t1");
        let threads = db.chat_starter_threads("acct", &primary(), &own(), 50).unwrap();
        let got: Vec<&str> = threads.iter().map(|t| t.subject.as_str()).collect();
        assert_eq!(got, vec!["Talked"]);
    }

    #[test]
    fn a_thread_whose_newest_email_is_inbound_awaits_a_reply() {
        let db = Database::new_for_testing().unwrap();
        insert(&db, "1", "t1", "ana@acme.test", "Waiting", 100, "primary", "inbox");
        reply(&db, "r1", "t1");
        let threads = db.chat_starter_threads("acct", &primary(), &own(), 50).unwrap();
        assert!(threads[0].awaiting_reply);
        assert_eq!(threads[0].sender_name, "Name ana@acme.test");
        assert_eq!(threads[0].thread_id, "t1");
    }

    #[test]
    fn a_thread_the_owner_answered_last_does_not_await_a_reply() {
        let db = Database::new_for_testing().unwrap();
        insert(&db, "1", "t1", "ana@acme.test", "Answered", 100, "primary", "inbox");
        // The answer sits in Sent, outside the inbox scope the scan reads.
        insert(
            &db,
            "r1",
            "t1",
            "me@example.com",
            "Re: Answered",
            150,
            "primary",
            "sent",
        );
        let threads = db.chat_starter_threads("acct", &primary(), &own(), 50).unwrap();
        assert!(!threads[0].awaiting_reply);
    }

    #[test]
    fn no_owner_address_means_no_candidates() {
        let db = Database::new_for_testing().unwrap();
        insert(&db, "1", "t1", "ana@acme.test", "a", 100, "primary", "inbox");
        assert!(db.chat_starter_senders("acct", &primary(), &[], 50).unwrap().is_empty());
        assert!(db.chat_starter_threads("acct", &primary(), &[], 50).unwrap().is_empty());
    }
}
