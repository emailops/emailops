use super::*;

/// A separator-delimited run this long, with digits scattered through it, is a
/// generated token — no human picks one as a mailbox name.
const OPAQUE_TOKEN_MIN: usize = 20;
/// Below this many digits, the numerals are decoration ("cloudnative4devs"),
/// not entropy.
const OPAQUE_TOKEN_MIN_DIGITS: usize = 3;
/// A pure-hex run this long is a UUID or a digest (16 hex chars = 64 bits).
const HEX_BLOB_MIN: usize = 16;

/// Does this local-part segment read as a randomly generated token?
///
/// The discriminator against a real mailbox is WHERE the digits sit. Humans
/// append them — `robertaquinnbarlow90`, `roselynhartfordbaum70` — so stripping
/// the trailing run leaves a pure word. A generated token scatters them —
/// `7kqmz3wtbnvxrjhdyplsc48`, `3bxwqmzpvrt58k2ndhguf` — so digits survive the
/// strip. Length alone would eat both, which is why it is not the test.
fn is_opaque_token(seg: &str) -> bool {
    if seg.len() < OPAQUE_TOKEN_MIN || !seg.chars().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    if !seg.chars().any(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    if seg.chars().filter(|c| c.is_ascii_digit()).count() < OPAQUE_TOKEN_MIN_DIGITS {
        return false;
    }
    seg.trim_end_matches(|c: char| c.is_ascii_digit())
        .chars()
        .any(|c| c.is_ascii_digit())
}

/// Is this an address that mail infrastructure generated rather than a person?
///
/// Covers the shapes that reach the contact pool through ordinary, non-spam
/// mail and then surface in autocomplete because they happen to embed the
/// user's own address:
///
/// - **VERP return paths** — `bounce-<token>-user=example.com@esp.net`. The
///   `=` encodes a recipient address in the local part, so a prefix search for
///   the user's own name matches the envelope of every bulk message they got.
/// - **Per-notification plus tags and unsubscribe links** —
///   `noreply+<64 hex>@…`, `unsubscribe-<70 char token>@…`.
/// - **Digest/UUID mailboxes** — `<8-4-4-4-12 uuid>@reply.example.com`.
///
/// Deliberately NOT a vendor domain blocklist: ESP hostnames change constantly,
/// and the token shape is the durable signal. Deliberately not a length rule
/// either — long *word* addresses are real mailing lists
/// (`city-cloud-computing-meetup-list@…`) and long name-plus-digit addresses
/// are real people (`robertaquinnbarlow90@…`); both must survive.
pub(super) fn is_machine_generated_address(email: &str) -> bool {
    let Some((local, domain)) = email.rsplit_once('@') else {
        return false;
    };
    let local = local.trim();

    // A `bounce`/`bounces` label is the conventional name for a return-path
    // subdomain (`calendar-server.bounces.example.com`). Nobody's mailbox lives
    // under one.
    if domain.split('.').any(|label| label == "bounce" || label == "bounces") {
        return true;
    }

    // VERP: an address encoded inside the local part.
    if local.contains('=') {
        return true;
    }

    // A randomly generated run, anywhere in the local part.
    if local.split(['-', '+', '.', '_']).any(is_opaque_token) {
        return true;
    }

    // A hex blob, once the hyphens/underscores inside one `.`/`+` chunk are
    // removed — this is what catches UUID tags, whose individual groups are
    // each too short to look opaque on their own.
    local.split(['+', '.']).any(|chunk| {
        let compact_len = chunk.chars().filter(|c| *c != '-' && *c != '_').count();
        compact_len >= HEX_BLOB_MIN && chunk.chars().all(|c| c.is_ascii_hexdigit() || c == '-' || c == '_')
    })
}

/// Local-part markers for an unattended mailbox, in the locales the app ships
/// plus the `mailer-daemon` convention. Compared against the local part with
/// its separators removed, so `no-reply`, `no_reply`, `no.reply` and `noreply`
/// all collapse to the same marker.
const NO_REPLY_MARKERS: &[&str] = &[
    "noreply",
    "donotreply",
    "noresponder",
    "nepasrepondre",
    "nichtantworten",
    "mailerdaemon",
];

/// Is this an unattended mailbox that discards anything sent to it?
///
/// Matched two ways, because the marker is not always at the front:
/// `no-reply-<token>@…` collapses to a `noreply` PREFIX, while
/// `photos-noreply+<uuid>@…` carries `noreply` as its own SEGMENT.
///
/// Matching is deliberately anchored rather than a substring search — a
/// contains-check would eat `bouncehouseparties@…` and `noreen.smith@…`.
///
/// Applied to the recipient pool only. On the search side a no-reply sender is
/// a useful filter ("everything from that notifier"), so
/// [`Database::autocomplete_senders`] keeps them.
pub(super) fn is_no_reply_address(email: &str) -> bool {
    let Some((local, _domain)) = email.rsplit_once('@') else {
        return false;
    };
    let local = local.trim().to_ascii_lowercase();

    let compact: String = local.chars().filter(|c| !matches!(c, '-' | '_' | '.')).collect();
    if NO_REPLY_MARKERS.iter().any(|m| compact.starts_with(m)) {
        return true;
    }

    local
        .split(['-', '+', '.', '_'])
        .any(|seg| NO_REPLY_MARKERS.contains(&seg))
}

/// Autocomplete over-fetches so that dropping machine and no-reply addresses in
/// Rust still leaves a full page of suggestions.
///
/// Sized off the worst case rather than the average. Filtered addresses are
/// ~11% of a real contact pool, but they cluster hard by prefix: typing a large
/// vendor's name can match 90% no-reply, because that is most of what a vendor
/// ever sends. A shallow multiple returns a two-entry stub exactly when the
/// user is typing a common name. The extra rows are nearly free — the GROUP BY
/// and sort behind them run over the whole match set either way, so only the
/// row decode grows.
fn overfetch(limit: i32) -> i32 {
    limit.saturating_mul(12).clamp(limit, 500)
}

impl Database {
    /// Autocomplete sender email addresses matching a prefix.
    /// Returns distinct sender_email values ordered by recency (most recent email wins).
    ///
    /// Spam, trash, and soft-deleted mail is excluded — the same
    /// `mailbox NOT IN ('spam','trash') AND is_deleted = 0` scope every other
    /// read path uses. A spam sender is not a correspondent, and offering one
    /// as a completion puts a phishing address one keystroke from being mailed.
    /// Custom IMAP folders (`mailbox = 'folder:…'`) are ordinary filed mail and
    /// stay in scope. Machine-generated envelope addresses are dropped after
    /// the query — see [`is_machine_generated_address`].
    pub fn autocomplete_senders(&self, account_id: &str, prefix: &str, limit: i32) -> Result<Vec<(String, String)>> {
        let conn = self.reader();
        // LIKE pattern: case-insensitive prefix match on email or sender name
        let pattern = format!("%{}%", prefix.to_lowercase());
        let mut stmt = conn.prepare(
            "SELECT sender_email, sender, MAX(timestamp) as latest
             FROM emails
             WHERE account_id = ?1
               AND is_deleted = 0
               AND mailbox NOT IN ('spam', 'trash')
               AND (LOWER(sender_email) LIKE ?2 OR LOWER(sender) LIKE ?2)
             GROUP BY sender_email
             ORDER BY latest DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![account_id, pattern, overfetch(limit)], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows
            .filter_map(|r| r.ok())
            .filter(|(email, _)| !is_machine_generated_address(email))
            .take(limit.max(0) as usize)
            .collect())
    }

    /// Smart autocomplete for recipients — prioritizes same-domain matches and frequent contacts.
    /// Returns (email, name, is_domain_match) tuples.
    ///
    /// Both branches of the contact union are scoped to non-spam, non-trash,
    /// non-deleted mail (see [`Database::autocomplete_senders`]). The recipients
    /// branch needs the filter just as much as the senders branch: a single spam
    /// message addressed to a harvested bcc list would otherwise inject every
    /// address on it into the user's completions.
    ///
    /// Ranking is `domain_match → direct_contact → freq → recency`.
    /// `direct_contact` separates people the user actually corresponds with
    /// (anyone who sent them mail, plus anyone they addressed in sent mail) from
    /// addresses that only ever shared a To/Cc line on RECEIVED mail — strangers
    /// on misdirected or harvested mail. Those stay suggestable but sort last,
    /// so they no longer outrank real contacts on raw frequency.
    pub fn autocomplete_recipients(
        &self,
        account_id: &str,
        prefix: &str,
        context_domain: Option<&str>,
        limit: i32,
    ) -> Result<Vec<(String, String, bool)>> {
        let conn = self.reader();
        let pattern = format!("%{}%", prefix.to_lowercase());
        let domain_pattern = context_domain
            .map(|d| format!("%@{}", d.to_lowercase()))
            .unwrap_or_default();

        // Extract clean email addresses from recipients_json which may contain
        // "Name <email>" format or plain email strings.
        // Use CASE to extract the part between < and > if present, otherwise use as-is.
        let sql = "
            WITH all_contacts AS (
                SELECT LOWER(sender_email) AS email, sender AS name, timestamp, 1 AS direct_contact
                FROM emails
                WHERE account_id = ?1 AND is_deleted = 0 AND mailbox NOT IN ('spam', 'trash')
                UNION ALL
                SELECT LOWER(
                    CASE
                        WHEN INSTR(TRIM(je.value), '<') > 0
                        THEN SUBSTR(TRIM(je.value),
                                    INSTR(TRIM(je.value), '<') + 1,
                                    INSTR(TRIM(je.value), '>') - INSTR(TRIM(je.value), '<') - 1)
                        ELSE TRIM(je.value)
                    END
                ) AS email,
                CASE
                    WHEN INSTR(TRIM(je.value), '<') > 0
                    THEN TRIM(SUBSTR(TRIM(je.value), 1, INSTR(TRIM(je.value), '<') - 1))
                    ELSE ''
                END AS name,
                e.timestamp,
                -- Addressed BY the user (sent mail) = a real contact. Addressed
                -- ALONGSIDE the user on inbound mail = a stranger sharing a To/Cc
                -- line, which must not outrank anyone on raw frequency.
                CASE WHEN e.is_sent = 1 OR e.mailbox = 'sent' THEN 1 ELSE 0 END AS direct_contact
                FROM emails e, json_each(e.recipients_json) je
                WHERE e.account_id = ?1
                  AND e.is_deleted = 0
                  AND e.mailbox NOT IN ('spam', 'trash')
                  AND LENGTH(TRIM(je.value)) > 3
            )
            SELECT email, MAX(CASE WHEN name != '' THEN name ELSE '' END) AS name, COUNT(*) AS freq,
                   CASE WHEN ?4 != '' AND email LIKE ?4 THEN 1 ELSE 0 END AS domain_match
            FROM all_contacts
            WHERE email LIKE ?2 OR LOWER(name) LIKE ?2
            GROUP BY email
            ORDER BY domain_match DESC, MAX(direct_contact) DESC, freq DESC, MAX(timestamp) DESC
            LIMIT ?3
        ";

        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params![account_id, pattern, overfetch(limit), domain_pattern], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(3)? != 0,
            ))
        })?;
        Ok(rows
            .filter_map(|r| r.ok())
            .filter(|(email, _, _)| !is_machine_generated_address(email) && !is_no_reply_address(email))
            .take(limit.max(0) as usize)
            .collect())
    }

    /// Get the 0-based position of an email in the inbox, grouped by thread recency.
    ///
    /// Must use the inbox-scoped predicate for consistency with the inbox view
    /// in `get_emails` — otherwise positions reference threads that the inbox
    /// view excludes (e.g. threads where the latest message is a Sent reply).
    ///
    /// Under [`AccountScope::AllEnabled`] the position is computed over the
    /// unified (all enabled accounts) inbox. The target thread is always
    /// resolved within the email's OWN account — thread ids are not globally
    /// unique, so a same-id thread in another account must not hijack the
    /// representative lookup.
    pub fn get_email_inbox_position(&self, scope: crate::db::AccountScope<'_>, email_id: &str) -> Result<i32> {
        let conn = self.reader();
        // The email id binds as ?1 in both variants; the single-account scope
        // appends its account id as ?2.
        let (list_scope_cond, account_param): (&str, Option<&str>) = match scope {
            crate::db::AccountScope::Account(id) => ("e.account_id = ?2", Some(id)),
            crate::db::AccountScope::AllEnabled => {
                ("e.account_id IN (SELECT id FROM accounts WHERE enabled = 1)", None)
            }
        };
        let sql = format!(
            "WITH target AS (
                SELECT rep.id, rep.timestamp
                FROM emails rep
                WHERE rep.account_id = (SELECT account_id FROM emails WHERE id = ?1)
                  AND rep.thread_id = (SELECT thread_id FROM emails WHERE id = ?1)
                  AND {rep_latest}
                LIMIT 1
             )
             SELECT COUNT(*)
             FROM emails e, target
             WHERE {list_scope_cond}
               AND {e_latest}
               AND (
                   e.timestamp > target.timestamp
                   OR (e.timestamp = target.timestamp AND e.id > target.id)
               )",
            rep_latest = latest_inbox_email_predicate("rep"),
            e_latest = latest_inbox_email_predicate("e"),
        );
        let position: i32 = match account_param {
            Some(id) => conn.query_row(&sql, params![email_id, id], |row| row.get(0))?,
            None => conn.query_row(&sql, params![email_id], |row| row.get(0))?,
        };
        Ok(position)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use super::{is_machine_generated_address, is_no_reply_address};
    use crate::db::{AccountScope, Database};

    /// Every fixture below is synthetic. They reproduce the *shape* of real
    /// infrastructure mail — token length, charset, digit placement, separator
    /// layout — with invented names, tokens, and domains throughout.
    #[test]
    fn machine_generated_addresses_are_recognised() {
        for addr in [
            // VERP return path: the local part encodes a recipient address.
            "wq7f3mzp0kdvxr2ntbjy8slgh6cauq1w-alice=example.com@mail1.esphost.test",
            "bounce-alice=example.com@mailer.example.net",
            // A `bounces` label in the domain, on its own.
            "notify-alice@calendar-server.bounces.example.com",
            // Long opaque token: letters + digits scattered, no word structure.
            "unsubscribe-k3vxq9mzbtrw7fjdhs2ypnc85gaeul40xiqvzdbnf6mkhwrst@lists.example.com",
            "alice1921+v8qzmxkw3ptrn6ydhbscf27jgleu90aivztqmrnbxhs4@example.com",
            "no-reply-7kqmz3wtbnvxrjhdyplsc48@example.com",
            "alice+3bxwqmzpvrt58k2ndhguf@example.com",
            // Hex digest tag.
            "noreply+7f3a9c1e0b52d84a6f0c3e9b1d7a45c8e2b06f91@notify.example.com",
            // A hex-encoded address embedded in the local part
            // (616c696365406578616d706c652e636f6d = "alice@example.com").
            "social-follow-616c696365406578616d706c652e636f6d-b2888@postmaster.example.com",
            // UUID tag — each hyphen group is short, only the whole blob reads as hex.
            "photos-noreply+4b81e27c-93df-4a15-b6e0-27c9fd3a8b41@example.com",
            "no-reply-general.8c42fa19-bd07-4e6a-91f3d05b7ce2a864@example.nl",
            "2841937-604812-9d3f7a1c58e02b64d7f931ac05e8b2fa@reply.example.com",
        ] {
            assert!(is_machine_generated_address(addr), "should be machine: {addr}");
        }
    }

    /// The filter must not eat real mailboxes. Mailing lists and name-plus-digit
    /// addresses are long and alphanumeric, and both are entirely legitimate to
    /// address. All fixtures are invented.
    #[test]
    fn human_addresses_survive_the_machine_filter() {
        for addr in [
            "alice@example.com",
            "alice.smith@example.com",
            "alice.smith2024@example.com",
            "alice+shopping@example.com",
            "alice+newsletter+work@example.com",
            // Mailing lists — long, hyphenated, but made of words.
            "distributed-systems-reading-group-announce@example.com",
            "city-cloud-computing-meetup-list@example.com",
            "the-weekly-dispatch-for-builders@news.example.com",
            // Short opaque-ish tags stay: below the token threshold.
            "alice+tagxyzw6281974c@example.com",
            // Name-plus-digits is THE human pattern — long, alphanumeric, but
            // the digits are a suffix, not scattered through a random token.
            "robertaquinnbarlow90@example.com",
            "roselynhartfordbaum70@example.com",
            "losvecinosdelbarrio85@example.com",
            "contactline09988776655@example.com",
            // A digit used as a letter ("4devs") is not a token either.
            "cloudnative4devsforum-list@example.com",
            "espaciodetecnologias3-space@example.com",
            // Not an address at all.
            "not-an-address",
        ] {
            assert!(!is_machine_generated_address(addr), "should be human: {addr}");
        }
    }

    /// You cannot reply to a no-reply address, so it is never a recipient the
    /// user wants. Markers are matched across the locales the app ships.
    #[test]
    fn no_reply_addresses_are_recognised() {
        for addr in [
            "noreply@example.com",
            "no-reply@example.com",
            "no_reply@example.com",
            "no.reply@example.com",
            "donotreply@example.com",
            "do-not-reply@example.com",
            "donotreplymail@notify.example.com",
            "no-responder-general@example.es",
            "ne-pas-repondre@example.fr",
            "nicht-antworten@example.de",
            "mailer-daemon@example.com",
            // Marker in the middle, not at the start.
            "photos-noreply+4b81e27c-93df-4a15-b6e0-27c9fd3a8b41@example.com",
            // The shape the token rules deliberately miss: a random tail with
            // too few digits to read as entropy.
            "no-reply-qm8wbtzvxrkdhplsnyjcfg@example.com",
        ] {
            assert!(is_no_reply_address(addr), "should be no-reply: {addr}");
        }
    }

    /// The marker must not swallow ordinary mailboxes that merely start with
    /// the same letters, nor the `bounce` word in a real business name.
    #[test]
    fn no_reply_markers_do_not_overreach() {
        for addr in [
            "reply@example.com",
            "replies@example.com",
            "noreen.smith@example.com",
            "norbert@example.com",
            "bouncehouseparties@example.com",
            "alice@example.com",
            "not-an-address",
        ] {
            assert!(!is_no_reply_address(addr), "should be addressable: {addr}");
        }
    }

    #[test]
    fn autocomplete_recipients_drops_no_reply_addresses() {
        let db = Database::new_for_testing().unwrap();
        insert_contact_email(
            &db,
            "e1",
            "acc1",
            "t1",
            "Alice",
            "alice@example.com",
            "[]",
            "inbox",
            100,
        );
        insert_contact_email(
            &db,
            "e2",
            "acc1",
            "t2",
            "Alerts",
            "no-reply@alice.example.com",
            "[]",
            "inbox",
            200,
        );

        let hits = db.autocomplete_recipients("acc1", "alice", None, 10).unwrap();
        let emails: Vec<&str> = hits.iter().map(|(e, _, _)| e.as_str()).collect();
        assert_eq!(emails, vec!["alice@example.com"]);
    }

    /// Search filters by sender, where a no-reply address is a perfectly useful
    /// facet ("show me everything from that notifier"). Only the compose-side
    /// recipient pool drops them.
    #[test]
    fn autocomplete_senders_keeps_no_reply_addresses() {
        let db = Database::new_for_testing().unwrap();
        insert_contact_email(
            &db,
            "e1",
            "acc1",
            "t1",
            "Alerts",
            "no-reply@alice.example.com",
            "[]",
            "inbox",
            200,
        );

        let hits = db.autocomplete_senders("acc1", "alice", 10).unwrap();
        let emails: Vec<&str> = hits.iter().map(|(e, _)| e.as_str()).collect();
        assert_eq!(emails, vec!["no-reply@alice.example.com"]);
    }

    #[test]
    fn autocomplete_senders_drops_machine_generated_addresses() {
        let db = Database::new_for_testing().unwrap();
        insert_contact_email(
            &db,
            "e1",
            "acc1",
            "t1",
            "Alice",
            "alice@example.com",
            "[]",
            "inbox",
            100,
        );
        insert_contact_email(
            &db,
            "e2",
            "acc1",
            "t2",
            "Campaign",
            "wq7f3mzp0kdvxr2ntbjy8slgh6cauq1w-alice=example.com@mail1.esphost.test",
            "[]",
            "inbox",
            200,
        );

        let hits = db.autocomplete_senders("acc1", "alice", 10).unwrap();
        let emails: Vec<&str> = hits.iter().map(|(e, _)| e.as_str()).collect();
        assert_eq!(emails, vec!["alice@example.com"]);
    }

    #[test]
    fn autocomplete_recipients_drops_machine_generated_addresses() {
        let db = Database::new_for_testing().unwrap();
        insert_contact_email(
            &db,
            "e1",
            "acc1",
            "t1",
            "Alice",
            "alice@example.com",
            "[]",
            "inbox",
            100,
        );
        insert_contact_email(
            &db,
            "e2",
            "acc1",
            "t2",
            "Campaign",
            "wq7f3mzp0kdvxr2ntbjy8slgh6cauq1w-alice=example.com@mail1.esphost.test",
            r#"["noreply+7f3a9c1e0b52d84a6f0c3e9b1d7a45c8e2b06f91@alice.example.com"]"#,
            "inbox",
            200,
        );

        let hits = db.autocomplete_recipients("acc1", "alice", None, 10).unwrap();
        let emails: Vec<&str> = hits.iter().map(|(e, _, _)| e.as_str()).collect();
        assert_eq!(emails, vec!["alice@example.com"]);
    }

    /// Some prefixes are dominated by filtered addresses — on a real mailbox a
    /// vendor-name prefix can be ~90% no-reply. The over-fetch has to be deep
    /// enough that a full page still comes back, not a stub of two entries.
    #[test]
    fn autocomplete_recipients_fills_page_when_most_matches_are_filtered() {
        let db = Database::new_for_testing().unwrap();
        // 30 no-reply senders, all more recent than the humans, so a shallow
        // over-fetch would return almost nothing but them.
        for i in 0..30 {
            insert_contact_email(
                &db,
                &format!("n{i}"),
                "acc1",
                &format!("tn{i}"),
                "Alerts",
                &format!("no-reply-{i}@alice.example.com"),
                "[]",
                "inbox",
                1000 + i,
            );
        }
        for i in 0..8 {
            insert_contact_email(
                &db,
                &format!("h{i}"),
                "acc1",
                &format!("th{i}"),
                "Alice",
                &format!("alice{i}@example.com"),
                "[]",
                "inbox",
                100 + i,
            );
        }

        let hits = db.autocomplete_recipients("acc1", "alice", None, 8).unwrap();
        assert_eq!(hits.len(), 8, "filtered addresses must not truncate the page");
        assert!(hits.iter().all(|(e, _, _)| !e.starts_with("no-reply")));
    }

    /// Over-fetching must actually backfill: with more machine addresses than
    /// the limit, the caller still gets a full page of real contacts.
    #[test]
    fn autocomplete_recipients_backfills_past_machine_addresses() {
        let db = Database::new_for_testing().unwrap();
        // 6 machine addresses, all more recent/frequent than the humans.
        for i in 0..6 {
            insert_contact_email(
                &db,
                &format!("m{i}"),
                "acc1",
                &format!("tm{i}"),
                "Campaign",
                &format!("bounce{i}-alice=example.com@esphost.net"),
                "[]",
                "inbox",
                900 + i,
            );
        }
        for i in 0..3 {
            insert_contact_email(
                &db,
                &format!("h{i}"),
                "acc1",
                &format!("th{i}"),
                "Alice",
                &format!("alice{i}@example.com"),
                "[]",
                "inbox",
                100 + i,
            );
        }

        let hits = db.autocomplete_recipients("acc1", "alice", None, 3).unwrap();
        assert_eq!(hits.len(), 3, "machine addresses must not eat the page");
        assert!(hits.iter().all(|(e, _, _)| !e.contains('=')));
    }

    /// Spam/trash senders are not people the user corresponds with — suggesting
    /// them in a compose "To" field is how a phishing address ends up one
    /// keystroke away from being mailed.
    #[test]
    fn autocomplete_senders_excludes_spam_trash_and_deleted() {
        let db = Database::new_for_testing().unwrap();
        insert_contact_email(
            &db,
            "e1",
            "acc1",
            "t1",
            "Alice",
            "alice@example.com",
            "[]",
            "inbox",
            100,
        );
        insert_contact_email(&db, "e2", "acc1", "t2", "Bot", "alice@spam.test", "[]", "spam", 200);
        insert_contact_email(&db, "e3", "acc1", "t3", "Bot", "alice@trash.test", "[]", "trash", 300);
        insert_contact_email(&db, "e4", "acc1", "t4", "Bot", "alice@gone.test", "[]", "inbox", 400);
        set_email_deleted(&db, "e4");

        let hits = db.autocomplete_senders("acc1", "alice", 10).unwrap();
        let emails: Vec<&str> = hits.iter().map(|(e, _)| e.as_str()).collect();
        assert_eq!(emails, vec!["alice@example.com"]);
    }

    #[test]
    fn autocomplete_recipients_excludes_spam_trash_and_deleted_senders() {
        let db = Database::new_for_testing().unwrap();
        insert_contact_email(
            &db,
            "e1",
            "acc1",
            "t1",
            "Alice",
            "alice@example.com",
            "[]",
            "inbox",
            100,
        );
        insert_contact_email(&db, "e2", "acc1", "t2", "Bot", "alice@spam.test", "[]", "spam", 200);
        insert_contact_email(&db, "e3", "acc1", "t3", "Bot", "alice@trash.test", "[]", "trash", 300);
        insert_contact_email(&db, "e4", "acc1", "t4", "Bot", "alice@gone.test", "[]", "inbox", 400);
        set_email_deleted(&db, "e4");

        let hits = db.autocomplete_recipients("acc1", "alice", None, 10).unwrap();
        let emails: Vec<&str> = hits.iter().map(|(e, _, _)| e.as_str()).collect();
        assert_eq!(emails, vec!["alice@example.com"]);
    }

    /// The recipients branch of the union has to be filtered too — a spam mail
    /// addressed to a bcc list would otherwise leak every address on it.
    #[test]
    fn autocomplete_recipients_excludes_recipients_of_spam_trash_and_deleted() {
        let db = Database::new_for_testing().unwrap();
        insert_contact_email(
            &db,
            "e1",
            "acc1",
            "t1",
            "Me",
            "me@example.com",
            r#"["Alice Ok <alice@example.com>"]"#,
            "sent",
            100,
        );
        insert_contact_email(
            &db,
            "e2",
            "acc1",
            "t2",
            "Bot",
            "bot@spam.test",
            r#"["alice@spam.test"]"#,
            "spam",
            200,
        );
        insert_contact_email(
            &db,
            "e3",
            "acc1",
            "t3",
            "Bot",
            "bot@trash.test",
            r#"["alice@trash.test"]"#,
            "trash",
            300,
        );
        insert_contact_email(
            &db,
            "e4",
            "acc1",
            "t4",
            "Bot",
            "bot@gone.test",
            r#"["alice@gone.test"]"#,
            "inbox",
            400,
        );
        set_email_deleted(&db, "e4");

        let hits = db.autocomplete_recipients("acc1", "alice", None, 10).unwrap();
        let emails: Vec<&str> = hits.iter().map(|(e, _, _)| e.as_str()).collect();
        assert_eq!(emails, vec!["alice@example.com"]);
    }

    /// An address that only ever appeared on the To/Cc line of mail the user
    /// RECEIVED is not a correspondent — it is a stranger the user happened to
    /// be co-addressed with (misdirected mail, harvested lists). It stays in the
    /// pool but must rank below real contacts, however often it appears.
    #[test]
    fn autocomplete_recipients_ranks_direct_contacts_above_inbound_co_recipients() {
        let db = Database::new_for_testing().unwrap();
        // Stranger: only ever a co-recipient on received mail — but on 3 of them,
        // so raw frequency would otherwise float it to the top.
        for (i, id) in ["s1", "s2", "s3"].iter().enumerate() {
            insert_contact_email(
                &db,
                id,
                "acc1",
                &format!("t-{id}"),
                "Newsletter",
                "bot@news.test",
                r#"["alice@stranger.test"]"#,
                "inbox",
                100 + i as i64,
            );
        }
        // Addressed once, by the user, in mail the user sent.
        insert_contact_email(
            &db,
            "e1",
            "acc1",
            "t1",
            "Me",
            "me@example.com",
            r#"["Alice Sent <alice@sent.test>"]"#,
            "sent",
            200,
        );
        // Wrote to the user once — a real correspondent via the senders branch.
        insert_contact_email(
            &db,
            "e2",
            "acc1",
            "t2",
            "Alice In",
            "alice@inbound.test",
            "[]",
            "inbox",
            300,
        );

        let hits = db.autocomplete_recipients("acc1", "alice", None, 10).unwrap();
        let emails: Vec<&str> = hits.iter().map(|(e, _, _)| e.as_str()).collect();
        assert_eq!(
            emails,
            vec!["alice@inbound.test", "alice@sent.test", "alice@stranger.test"],
            "direct contacts must outrank an inbound-only co-recipient"
        );
    }

    /// Ranking the context domain first is pre-existing behavior — the new
    /// direct-contact tiebreak slots in behind it, not in front.
    #[test]
    fn autocomplete_recipients_keeps_domain_match_as_primary_sort() {
        let db = Database::new_for_testing().unwrap();
        insert_contact_email(
            &db,
            "e1",
            "acc1",
            "t1",
            "Newsletter",
            "bot@news.test",
            r#"["alice@acme.test"]"#,
            "inbox",
            100,
        );
        insert_contact_email(
            &db,
            "e2",
            "acc1",
            "t2",
            "Me",
            "me@example.com",
            r#"["alice@other.test"]"#,
            "sent",
            200,
        );

        let hits = db
            .autocomplete_recipients("acc1", "alice", Some("acme.test"), 10)
            .unwrap();
        let emails: Vec<&str> = hits.iter().map(|(e, _, _)| e.as_str()).collect();
        assert_eq!(emails, vec!["alice@acme.test", "alice@other.test"]);
        assert!(hits[0].2, "context-domain hit must report domain_match");
    }

    /// Custom IMAP folders are ordinary filed mail — their correspondents must
    /// still be suggested.
    #[test]
    fn autocomplete_recipients_keeps_custom_folder_contacts() {
        let db = Database::new_for_testing().unwrap();
        insert_contact_email(
            &db,
            "e1",
            "acc1",
            "t1",
            "Alice",
            "alice@example.com",
            "[]",
            "folder:Clients/Acme",
            100,
        );

        let hits = db.autocomplete_recipients("acc1", "alice", None, 10).unwrap();
        let emails: Vec<&str> = hits.iter().map(|(e, _, _)| e.as_str()).collect();
        assert_eq!(emails, vec!["alice@example.com"]);
    }

    #[test]
    fn inbox_position_single_account_counts_newer_threads() {
        let db = Database::new_for_testing().unwrap();
        insert_email(&db, "e1", "acc1", "t1", 100);
        insert_email(&db, "e2", "acc1", "t2", 200);
        insert_email(&db, "e3", "acc1", "t3", 300);

        assert_eq!(
            db.get_email_inbox_position(AccountScope::Account("acc1"), "e3")
                .unwrap(),
            0
        );
        assert_eq!(
            db.get_email_inbox_position(AccountScope::Account("acc1"), "e1")
                .unwrap(),
            2
        );
    }

    #[test]
    fn inbox_position_all_enabled_counts_across_accounts() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@ex.com");
        insert_account(&db, "acc2", "a2@ex.com");
        insert_account(&db, "acc3", "a3@ex.com");

        insert_email(&db, "e1", "acc1", "t1", 100);
        insert_email(&db, "e2", "acc2", "t2", 200);
        insert_email(&db, "e3", "acc1", "t3", 300);
        // Disabled account's threads must not shift positions.
        insert_email(&db, "e4", "acc3", "t4", 400);
        set_account_enabled(&db, "acc3", false);

        // Unified order: e3 (300), e2 (200), e1 (100).
        assert_eq!(db.get_email_inbox_position(AccountScope::AllEnabled, "e3").unwrap(), 0);
        assert_eq!(db.get_email_inbox_position(AccountScope::AllEnabled, "e2").unwrap(), 1);
        assert_eq!(db.get_email_inbox_position(AccountScope::AllEnabled, "e1").unwrap(), 2);
    }

    #[test]
    fn inbox_position_all_enabled_thread_collision_scopes_target_to_own_account() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@ex.com");
        insert_account(&db, "acc2", "a2@ex.com");

        // Same thread_id string in both accounts. acc2's copy is newer, so it
        // must NOT be picked as the representative for acc1's email.
        insert_email(&db, "e1a", "acc1", "shared", 100);
        insert_email(&db, "e2a", "acc2", "shared", 300);
        insert_email(&db, "e-mid", "acc1", "t-mid", 200);

        // Unified order: e2a (300), e-mid (200), e1a (100).
        assert_eq!(db.get_email_inbox_position(AccountScope::AllEnabled, "e1a").unwrap(), 2);
    }
}
