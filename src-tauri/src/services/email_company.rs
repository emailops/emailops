//! Derive and persist a "company" tag for every email using the
//! `email_tags(tag_type='company')` row. No schema change required.
//!
//! # Rule (asymmetric — mirrors how a human labels an email)
//!
//! * **Outbound** (owner is the sender): take the most-frequent recipient
//!   domain (To + Cc), skipping the owner's own domain. That's the "other
//!   party" the user is writing *to*.
//! * **Inbound** (owner is not the sender): use the sender's domain. That's
//!   the party writing *to the user*.
//!
//! Both branches then pass through [`crate::util::email_addr::company_label_for`]:
//! corporate domains lose the TLD (`acme.com` → `acme`), personal providers
//! collapse to the full address (`alice@gmail.com` → `alice@gmail.com`) so
//! every gmail/outlook/yahoo correspondent is tagged as a distinct entity
//! rather than getting bucketed into one giant "gmail" company.
//!
//! Both branches use the same labelling function as the memory and tasks
//! extractors so a contact's `memory_facts.company` and their emails'
//! `tag_value` share the same vocabulary; the sidebar filter stays consistent
//! with the Memory/Tasks chip bar.
//!
//! Special addresses that add no business signal (`noreply@`, bounces, etc.)
//! are allowed through — the user can filter them out via the Smart Filters
//! "remove" action if they become noisy. We deliberately do not apply the
//! excluded-senders list here because this tag drives *all* email filtering,
//! not just memory extraction.

use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use crate::db::Database;
use crate::models::error::Result;
use crate::models::AppLogEvent;

/// Batch size for each write transaction during backfill. Kept small so the
/// write lock does not starve the sync/classification path when the backfill
/// runs concurrently with a live sync.
const BACKFILL_BATCH: usize = 200;

/// Pick the most-frequent non-owner domain from a list of addresses, together
/// with the lexicographically-smallest address that contributed to it. The
/// anchor address is what [`crate::util::email_addr::company_label_for`] uses
/// when the picked domain is a personal-mail provider — without it, every
/// gmail recipient would collapse to a single `gmail` bucket.
///
/// Ties on count are broken lexicographically on the domain (smaller wins) so
/// the tag stays stable across runs.
fn most_frequent_other_domain(addrs: &[String], owner_domain: Option<&str>) -> Option<(String, String)> {
    // domain -> (count, lex-smallest contributing address)
    let mut buckets: std::collections::HashMap<String, (u32, String)> = std::collections::HashMap::new();
    for a in addrs {
        let lc = a.trim().to_ascii_lowercase();
        let Some(d) = crate::util::email_addr::extract_domain(&lc) else {
            continue;
        };
        if let Some(od) = owner_domain {
            if d == od {
                continue;
            }
        }
        // Take the local-part-bearing form of the address (strip display
        // name if any) so the anchor is a real `local@domain` we can tag with.
        let (_, addr_only) = crate::util::email_addr::split_name_addr(a);
        let addr_only = if addr_only.is_empty() { lc } else { addr_only };
        buckets
            .entry(d)
            .and_modify(|(c, anchor)| {
                *c += 1;
                // Stable tie-break: keep the lexicographically smaller address.
                if addr_only < *anchor {
                    *anchor = addr_only.clone();
                }
            })
            .or_insert((1, addr_only));
    }
    // Tie-breaker on domain: lexicographically smaller wins. Matches
    // `services/memory/extractor::derive_company_tag`.
    buckets
        .into_iter()
        .max_by(|a, b| a.1 .0.cmp(&b.1 .0).then_with(|| b.0.cmp(&a.0)))
        .map(|(domain, (_, anchor))| (domain, anchor))
}

/// Compute the company tag for one email given its envelope and the owner's
/// address. Returns `None` when nothing usable can be derived (self-to-self,
/// no recipients on an outbound mail, malformed sender, …).
///
/// The returned label runs through [`crate::util::email_addr::company_label_for`]:
/// corporate domains collapse to their second-level label (`acme.com` →
/// `acme`); known free / personal-mail providers preserve the full address
/// (`alice@gmail.com`) so each individual stays distinct.
pub fn derive_company_tag(
    sender_email: &str,
    owner_email: &str,
    recipients: &[String],
    cc: &[String],
) -> Option<String> {
    let owner = owner_email.trim().to_ascii_lowercase();
    let owner_domain = crate::util::email_addr::extract_domain(&owner);
    let sender_lc = sender_email.trim().to_ascii_lowercase();

    let is_outbound = !owner.is_empty() && sender_lc == owner;
    let (domain, anchor_addr) = if is_outbound {
        // Outbound: "who are we writing to?" — recipient side.
        let mut all: Vec<String> = Vec::with_capacity(recipients.len() + cc.len());
        all.extend(recipients.iter().cloned());
        all.extend(cc.iter().cloned());
        most_frequent_other_domain(&all, owner_domain.as_deref())?
    } else {
        // Inbound: "who wrote to us?" — sender side.
        let d = crate::util::email_addr::extract_domain(&sender_lc)?;
        if Some(d.as_str()) == owner_domain.as_deref() {
            // The "sender" equals our own domain but doesn't match our exact
            // address — typically a shared-domain inbound. Fall back to
            // recipients minus owner to stay informative.
            let mut all: Vec<String> = Vec::with_capacity(recipients.len() + cc.len());
            all.extend(recipients.iter().cloned());
            all.extend(cc.iter().cloned());
            most_frequent_other_domain(&all, owner_domain.as_deref())?
        } else {
            (d, sender_lc.clone())
        }
    };

    Some(crate::util::email_addr::company_label_for(&domain, Some(&anchor_addr)))
}

/// Backfill company tags for every email in the account that does not yet
/// have one. Progress is reported via `app-log` events (source=`company`).
/// Safe to re-run — already-tagged emails are skipped in SQL.
pub fn backfill_account(db: &Arc<Database>, app: Option<&AppHandle>, account_id: &str) -> Result<u32> {
    let owner_email = match db.get_account(account_id)? {
        Some(acc) => acc.email,
        None => {
            log(app, "error", &format!("account {account_id} not found"));
            return Ok(0);
        }
    };

    log(app, "info", &format!("Starting company-tag backfill for {owner_email}"));

    let mut total_written: u32 = 0;
    let mut total_skipped: u32 = 0;
    // Cursor-based pagination so rows for which derive_company_tag() returns
    // None (no recipients, self-to-self, malformed sender, …) don't reappear
    // in the next batch and stall the loop forever.
    let mut cursor: String = String::new();

    loop {
        let rows = fetch_untagged_batch_after(db, account_id, &cursor, BACKFILL_BATCH as i32)?;
        if rows.is_empty() {
            break;
        }

        // Advance the cursor before processing so a single unwritable batch
        // still moves us forward on the next iteration.
        if let Some((last_id, _, _, _)) = rows.last() {
            cursor = last_id.clone();
        }

        let mut to_write: Vec<(String, String)> = Vec::with_capacity(rows.len());
        for (email_id, sender_email, recipients_json, cc_json) in &rows {
            let recipients: Vec<String> = serde_json::from_str(recipients_json).unwrap_or_default();
            let cc: Vec<String> = serde_json::from_str(cc_json).unwrap_or_default();
            match derive_company_tag(sender_email, &owner_email, &recipients, &cc) {
                Some(tag) => to_write.push((email_id.clone(), tag)),
                None => total_skipped += 1,
            }
        }

        if !to_write.is_empty() {
            db.upsert_email_tags_batch("company", &to_write)?;
            total_written += to_write.len() as u32;
        }

        log(
            app,
            "debug",
            &format!(
                "company backfill progress: +{} tagged (skipped {}) — total {}/{}",
                to_write.len(),
                rows.len() - to_write.len(),
                total_written,
                total_written + total_skipped
            ),
        );

        // If the batch returned fewer rows than the limit, we've caught up.
        if rows.len() < BACKFILL_BATCH {
            break;
        }
    }

    log(
        app,
        "success",
        &format!("Company-tag backfill complete for {owner_email}: {total_written} tagged, {total_skipped} skipped"),
    );
    Ok(total_written)
}

/// One-shot retag for emails currently tagged with a *bare-domain shortname*
/// of a known personal-email provider (e.g. `gmail`, `outlook`, `yahoo`).
///
/// The original algorithm collapsed every gmail correspondent into a single
/// `gmail` bucket; the new algorithm preserves the full address. Existing
/// rows in `email_tags` still carry the old labels, so run this once after
/// upgrade to migrate them. Effect:
///
/// 1. Delete every `email_tags` row for this account where `tag_type='company'`
///    and `tag_value` is a known personal-domain shortname.
/// 2. Delete the corresponding `tag_priority` rows (stale aggregates).
/// 3. Caller should then re-run [`backfill_account`] and
///    [`crate::services::tag_priority::backfill_account`] to repopulate.
///
/// Returns the number of `email_tags` rows deleted.
pub fn retag_personal_domains(db: &Arc<Database>, account_id: &str) -> Result<u32> {
    // Build the shortname set from the same source-of-truth list so it never
    // drifts. e.g. "gmail.com" → "gmail", "yahoo.co.uk" → "yahoo.co".
    let shortnames: std::collections::HashSet<String> = crate::util::email_addr::PERSONAL_EMAIL_DOMAINS
        .iter()
        .map(|d| {
            // Mirror the old strip_tld behaviour exactly: strip one label.
            let lc = d.to_ascii_lowercase();
            match lc.rsplit_once('.') {
                Some((stem, _)) if !stem.is_empty() => stem.to_string(),
                _ => lc,
            }
        })
        .collect();

    if shortnames.is_empty() {
        return Ok(0);
    }

    let mut conn = db.connection();
    let tx = conn.transaction()?;

    // Bind shortnames as IN (?, ?, …). The list is small (<100), well under
    // SQLite's 32k parameter ceiling.
    let placeholders = std::iter::repeat_n("?", shortnames.len()).collect::<Vec<_>>().join(",");
    let shortnames_vec: Vec<&str> = shortnames.iter().map(|s| s.as_str()).collect();

    let deleted_tags = {
        let sql = format!(
            "DELETE FROM email_tags
             WHERE email_id IN (
                 SELECT id FROM emails WHERE account_id = ?1
             )
             AND tag_type = 'company'
             AND tag_value IN ({placeholders})"
        );
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(1 + shortnames_vec.len());
        params_vec.push(&account_id);
        for s in &shortnames_vec {
            params_vec.push(s);
        }
        tx.execute(&sql, rusqlite::params_from_iter(params_vec.iter()))? as u32
    };

    // Drop the stale tag_priority aggregates so backfill can recompute them
    // from the new per-address tags.
    {
        let sql = format!(
            "DELETE FROM tag_priority
             WHERE account_id = ?1 AND tag_type = 'company'
               AND tag_value IN ({placeholders})"
        );
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(1 + shortnames_vec.len());
        params_vec.push(&account_id);
        for s in &shortnames_vec {
            params_vec.push(s);
        }
        tx.execute(&sql, rusqlite::params_from_iter(params_vec.iter()))?;
    }

    tx.commit()?;
    Ok(deleted_tags)
}

/// Fetch the envelope fields for the next batch of emails that do not yet
/// carry a `company` tag, paginated by email id so the loop always advances
/// even when a whole batch of rows yields no derivable tag. `NOT EXISTS` +
/// the `idx_email_tags_email` index keeps this efficient on 50k+ mailboxes.
fn fetch_untagged_batch_after(
    db: &Arc<Database>,
    account_id: &str,
    after_id: &str,
    limit: i32,
) -> Result<Vec<(String, String, String, String)>> {
    let conn = db.reader();
    let mut stmt = conn.prepare(
        "SELECT e.id, e.sender_email, e.recipients_json, e.cc_json
         FROM emails e
         WHERE e.account_id = ?1
           AND e.is_deleted = 0
           AND e.id > ?2
           AND NOT EXISTS (
               SELECT 1 FROM email_tags t
               WHERE t.email_id = e.id AND t.tag_type = 'company'
           )
         ORDER BY e.id
         LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![account_id, after_id, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

fn log(app: Option<&AppHandle>, level: &str, message: &str) {
    if let Some(app) = app {
        let _ = app.emit(
            "app-log",
            AppLogEvent {
                level: level.to_string(),
                source: "company".to_string(),
                message: message.to_string(),
            },
        );
    }
    // Also echo to stdout so the CLI binary surfaces progress without a
    // Tauri runtime.
    if app.is_none() {
        println!("[{level}] {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use rusqlite::params;

    fn seed_account(db: &Database, id: &str, email: &str) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?2, 'Test', 0)",
                params![id, email],
            )
            .unwrap();
    }

    fn seed_email(db: &Database, id: &str, account_id: &str, sender: &str) {
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?1, 'Test', 0)",
            params![account_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails
                    (id, account_id, thread_id, subject, sender, sender_email,
                     sender_domain, recipients_json, cc_json, snippet, timestamp,
                     is_read, is_deleted, category, created_at)
                 VALUES (?1, ?2, ?3, 'subj', ?4, ?4, '', '[]', '[]', '', 0,
                         0, 0, 'primary', 0)",
            params![id, account_id, format!("t-{id}"), sender],
        )
        .unwrap();
        conn.execute("INSERT INTO email_bodies (email_id, body) VALUES (?1, '')", params![id])
            .unwrap();
    }

    fn seed_tag(db: &Database, email_id: &str, tag_value: &str) {
        db.connection()
            .execute(
                "INSERT INTO email_tags (email_id, tag_type, tag_value, confidence, created_at)
                 VALUES (?1, 'company', ?2, NULL, 0)",
                params![email_id, tag_value],
            )
            .unwrap();
    }

    #[test]
    fn outbound_picks_most_frequent_recipient() {
        let tag = derive_company_tag(
            "me@mine.com",
            "me@mine.com",
            &["alice@acme.com".into(), "bob@acme.com".into(), "ex@other.com".into()],
            &[],
        );
        assert_eq!(tag.as_deref(), Some("acme"));
    }

    #[test]
    fn inbound_uses_sender_domain() {
        let tag = derive_company_tag("alice@acme.com", "me@mine.com", &["me@mine.com".into()], &[]);
        assert_eq!(tag.as_deref(), Some("acme"));
    }

    #[test]
    fn outbound_with_only_owner_recipients_returns_none() {
        let tag = derive_company_tag("me@mine.com", "me@mine.com", &["me@mine.com".into()], &[]);
        assert!(tag.is_none());
    }

    #[test]
    fn inbound_from_own_domain_falls_back_to_recipients() {
        // Shared-domain inbound: sender is `teammate@mine.com`, not me. Sender
        // domain matches owner domain — tag the "external" side instead.
        let tag = derive_company_tag(
            "teammate@mine.com",
            "me@mine.com",
            &["me@mine.com".into(), "client@acme.com".into()],
            &[],
        );
        assert_eq!(tag.as_deref(), Some("acme"));
    }

    #[test]
    fn corporate_label_strips_tld_for_subdomain() {
        // Smoke-test that the shared labelling helper still strips the TLD
        // for corporate domains (the previous behaviour). Detailed cases
        // live in `util::email_addr::tests`.
        let tag = derive_company_tag("alice@foo.acme.com", "me@mine.com", &["me@mine.com".into()], &[]);
        assert_eq!(tag.as_deref(), Some("foo.acme"));
    }

    #[test]
    fn inbound_from_personal_domain_tags_with_address() {
        // The whole point of the personal-domain special case: an inbound
        // mail from `alice@gmail.com` must NOT collapse to `gmail` — it
        // must keep the individual's address so distinct gmail users stay
        // distinct in filters.
        let tag = derive_company_tag("Alice@Gmail.com", "me@mine.com", &["me@mine.com".into()], &[]);
        assert_eq!(tag.as_deref(), Some("alice@gmail.com"));
    }

    #[test]
    fn outbound_to_personal_domain_tags_with_recipient_address() {
        // Outbound: I write to one gmail user. Tag must be that user's
        // address, not the bare `gmail` bucket.
        let tag = derive_company_tag("me@mine.com", "me@mine.com", &["Alice@Gmail.com".into()], &[]);
        assert_eq!(tag.as_deref(), Some("alice@gmail.com"));
    }

    #[test]
    fn retag_personal_domains_clears_old_shortnames() {
        // Three emails: one tagged with the old `gmail` shortname, one with
        // the new `alice@gmail.com` form, one with `acme` (corporate, must
        // be left alone). Retag must clear only the first.
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        seed_email(&db, "e1", "acc1", "alice@gmail.com");
        seed_tag(&db, "e1", "gmail"); // old style
        seed_email(&db, "e2", "acc1", "bob@gmail.com");
        seed_tag(&db, "e2", "bob@gmail.com"); // new style
        seed_email(&db, "e3", "acc1", "carol@acme.com");
        seed_tag(&db, "e3", "acme"); // corporate — must survive

        // Seed a corresponding stale tag_priority row for the old shortname.
        db.connection()
            .execute(
                "INSERT INTO tag_priority VALUES
                 ('acc1','company','gmail', 0, 5, NULL, 0),
                 ('acc1','company','acme',  3, 2, NULL, 0)",
                [],
            )
            .unwrap();

        let deleted = retag_personal_domains(&db, "acc1").unwrap();
        assert_eq!(deleted, 1, "only the bare-domain personal tag should be cleared");

        let conn = db.reader();
        // Old shortname rows gone.
        let cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM email_tags WHERE tag_value = 'gmail'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(cnt, 0);
        // New per-address row preserved.
        let cnt: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM email_tags WHERE tag_value = 'bob@gmail.com'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cnt, 1);
        // Corporate row preserved.
        let cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM email_tags WHERE tag_value = 'acme'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(cnt, 1);
        // Stale tag_priority for `gmail` cleared, `acme` preserved.
        let cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM tag_priority WHERE tag_value = 'gmail'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(cnt, 0);
        let cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM tag_priority WHERE tag_value = 'acme'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(cnt, 1);
    }

    #[test]
    fn retag_personal_domains_does_not_cross_accounts() {
        // The retag must only touch the requested account's rows.
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        seed_account(&db, "acc2", "other@other.com");
        seed_email(&db, "e1", "acc1", "alice@gmail.com");
        seed_tag(&db, "e1", "gmail");
        seed_email(&db, "e2", "acc2", "alice@gmail.com");
        seed_tag(&db, "e2", "gmail");

        retag_personal_domains(&db, "acc1").unwrap();

        let conn = db.reader();
        // acc2's gmail tag must survive.
        let cnt: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM email_tags t JOIN emails e ON e.id = t.email_id
                 WHERE e.account_id = 'acc2' AND t.tag_value = 'gmail'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cnt, 1);
    }

    #[test]
    fn outbound_to_multiple_personal_domain_recipients_picks_lex_smaller() {
        // Two gmail recipients on the same mail — pick the lex-smaller
        // address so the tag is deterministic across runs.
        let tag = derive_company_tag(
            "me@mine.com",
            "me@mine.com",
            &["zack@gmail.com".into(), "alice@gmail.com".into()],
            &[],
        );
        assert_eq!(tag.as_deref(), Some("alice@gmail.com"));
    }

    #[test]
    fn outbound_case_insensitive_owner_match() {
        // Owner stored uppercase, sender returned lowercase by provider.
        let tag = derive_company_tag("me@mine.com", "Me@MINE.COM", &["alice@acme.com".into()], &[]);
        assert_eq!(tag.as_deref(), Some("acme"));
    }

    #[test]
    fn outbound_trims_whitespace_in_sender() {
        // Sender with stray whitespace (seen on some IMAP providers) must
        // still be recognised as outbound so the recipient side is used.
        let tag = derive_company_tag("  me@mine.com  ", "me@mine.com", &["bob@acme.com".into()], &[]);
        assert_eq!(tag.as_deref(), Some("acme"));
    }

    #[test]
    fn outbound_uses_cc_when_recipients_only_owner() {
        // If To: is just me (self-addressed BCC trick) but CC has the
        // external party, we should still pick it up.
        let tag = derive_company_tag(
            "me@mine.com",
            "me@mine.com",
            &["me@mine.com".into()],
            &["client@acme.com".into()],
        );
        assert_eq!(tag.as_deref(), Some("acme"));
    }

    #[test]
    fn outbound_tie_picks_lexicographically_smaller() {
        // Two recipient domains, each appearing once — pick the smaller
        // string so the tag is deterministic across runs.
        let tag = derive_company_tag(
            "me@mine.com",
            "me@mine.com",
            &["a@zulu.com".into(), "b@acme.com".into()],
            &[],
        );
        assert_eq!(tag.as_deref(), Some("acme"));
    }

    #[test]
    fn malformed_sender_inbound_returns_none() {
        // Sender has no `@` — can't derive a domain for inbound.
        let tag = derive_company_tag("not-an-email", "me@mine.com", &["me@mine.com".into()], &[]);
        assert!(tag.is_none());
    }

    #[test]
    fn outbound_ignores_owner_domain_among_recipients() {
        // Internal CC shouldn't count: the owner is writing to Acme, with
        // a teammate CC'd on mine.com. Tag must be "acme", not "mine".
        let tag = derive_company_tag(
            "me@mine.com",
            "me@mine.com",
            &[
                "alice@acme.com".into(),
                "teammate@mine.com".into(),
                "bob@acme.com".into(),
            ],
            &[],
        );
        assert_eq!(tag.as_deref(), Some("acme"));
    }
}
