use super::*;

impl Database {
    pub fn get_contacts(&self, account_id: &str) -> Result<Vec<Contact>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT sender_email, MAX(sender) AS name, COUNT(*) AS cnt
             FROM emails
             WHERE account_id = ?1 AND is_deleted = 0
             GROUP BY sender_email
             ORDER BY cnt DESC
             LIMIT 500",
        )?;
        let contacts = stmt
            .query_map(params![account_id], |row| {
                Ok(Contact {
                    email: row.get(0)?,
                    name: row.get(1)?,
                    email_count: row.get(2)?,
                    last_timestamp: None,
                    received_count: 0,
                    sent_count: 0,
                    first_timestamp: None,
                    company: None,
                    kind: String::new(),
                    domain: String::new(),
                    relationship_score: 0.0,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(contacts)
    }

    /// Search known contacts (derived from the emails table) by a natural-language
    /// query such as "alice de emailops". Each whitespace-separated token must
    /// match somewhere in the contact's display name or email address — tokens
    /// are ANDed so specificity increases as the user types more.
    ///
    /// Implementation: we materialise the small set of distinct senders via a
    /// `GROUP BY sender_email` (bounded by `MAX_CANDIDATE_SENDERS`) and filter
    /// in Rust. That is faster than `LIKE '%token%'` scans on the main emails
    /// table for large mailboxes because the GROUP BY uses an index-covered
    /// path and produces at most a few hundred rows.
    pub fn search_contacts(&self, account_id: &str, query: &str, limit: i32) -> Result<Vec<Contact>> {
        // Tokenise: split on whitespace, lowercase, keep tokens of >=2 chars,
        // strip tokens that are pure stop-words ('de', 'of', 'from', ...).
        const STOP_WORDS: &[&str] = &["de", "del", "la", "el", "of", "from", "the", "a", "an"];
        let tokens: Vec<String> = query
            .split_whitespace()
            .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
            .filter(|t| t.chars().count() >= 2 && !STOP_WORDS.contains(&t.as_str()))
            .collect();

        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        // Pull all distinct senders for the account in a single pass. The
        // result set is bounded by the actual number of unique senders — a
        // real mailbox rarely exceeds a few thousand and the GROUP BY runs
        // entirely against the `idx_emails_account_active` index.
        const MAX_CANDIDATE_SENDERS: i32 = 5000;
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT sender_email, MAX(sender) AS name, COUNT(*) AS cnt, MAX(timestamp) AS latest
             FROM emails
             WHERE account_id = ?1 AND is_deleted = 0
             GROUP BY sender_email
             LIMIT ?2",
        )?;
        let candidates: Vec<Contact> = stmt
            .query_map(params![account_id, MAX_CANDIDATE_SENDERS], |row| {
                Ok(Contact {
                    email: row.get::<_, String>(0)?,
                    name: row.get::<_, String>(1)?,
                    email_count: row.get::<_, i32>(2)?,
                    last_timestamp: row.get::<_, Option<i64>>(3)?,
                    received_count: 0,
                    sent_count: 0,
                    first_timestamp: None,
                    company: None,
                    kind: String::new(),
                    domain: String::new(),
                    relationship_score: 0.0,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Rank: all tokens must appear somewhere in (name + email). Sort by
        // (number of email-count) desc so frequent correspondents surface
        // first; break ties by recency.
        let mut hits: Vec<Contact> = candidates
            .into_iter()
            .filter(|c| {
                let haystack = format!("{} {}", c.name.to_lowercase(), c.email.to_lowercase());
                tokens.iter().all(|t| haystack.contains(t.as_str()))
            })
            .collect();

        hits.sort_by(|a, b| {
            b.email_count
                .cmp(&a.email_count)
                .then_with(|| b.last_timestamp.cmp(&a.last_timestamp))
        });
        hits.truncate(limit.max(1) as usize);
        Ok(hits)
    }

    /// Build the full contact list for an account, with bidirectional counts,
    /// company enrichment, kind classification, and a relationship score.
    ///
    /// Aggregation runs in three passes:
    ///   1. Inbound: group `emails.sender_email` by lowercased address.
    ///   2. Outbound: explode `recipients_json` + `cc_json` from `mailbox='sent'`
    ///      rows in Rust, normalising each entry through `extract_addr_lc`.
    ///   3. Companies: join `email_tags(tag_type='company')` with `emails`,
    ///      group by sender and pick the most-frequent tag per address.
    ///
    /// All three passes use `reader()` connections — no writes. The total
    /// memory footprint is bounded by the number of distinct addresses seen
    /// (typically a few thousand on a real mailbox), so the in-memory
    /// aggregation is intentional: it lets us compute counts, recency, and
    /// the relationship score without materialising N rows in SQL.
    pub fn list_contacts(&self, account_id: &str, query: &ContactsQuery) -> Result<ContactsPage> {
        let conn = self.reader();

        // Determine the user's address to exclude self-edges. If the account
        // can't be found we fall back to no exclusion rather than failing the
        // whole call.
        let self_email: String = conn
            .query_row(
                "SELECT lower(email) FROM accounts WHERE id = ?1",
                params![account_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_default();

        let mut accum: HashMap<String, ContactAccum> = HashMap::new();

        // ── Pass 1: inbound ──────────────────────────────────────────────
        let mut stmt = conn.prepare(
            "SELECT lower(sender_email) AS addr, sender, timestamp
             FROM emails
             WHERE account_id = ?1 AND is_deleted = 0",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        for r in rows {
            let (addr, name, ts) = r?;
            if addr.is_empty() || addr == self_email {
                continue;
            }
            let entry = accum.entry(addr).or_default();
            entry.received += 1;
            entry.bump_ts(ts);
            if entry.name.is_empty() && !name.trim().is_empty() {
                entry.name = name;
            }
        }
        drop(stmt);

        // ── Pass 2: outbound (sent mailbox OR sender_email = self) ───────
        // Gmail accounts often store user-sent emails with mailbox='inbox',
        // so we also detect them by matching sender_email to the account's
        // own address (mirrors the pattern in services/dashboard.rs).
        let mut stmt = conn.prepare(
            "SELECT recipients_json, cc_json, timestamp
             FROM emails
             WHERE account_id = ?1 AND is_deleted = 0
               AND (mailbox = 'sent' OR lower(sender_email) = ?2)",
        )?;
        let rows = stmt.query_map(params![account_id, self_email], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        for r in rows {
            let (to_json, cc_json, ts) = r?;
            for raw in parse_addr_list(&to_json).into_iter().chain(parse_addr_list(&cc_json)) {
                let (display, addr) = split_name_addr(&raw);
                if addr.is_empty() || addr == self_email {
                    continue;
                }
                let entry = accum.entry(addr).or_default();
                entry.sent += 1;
                entry.bump_ts(ts);
                if entry.name.is_empty() && !display.is_empty() {
                    entry.name = display;
                }
            }
        }
        drop(stmt);

        // ── Pass 3: company enrichment ───────────────────────────────────
        // For each (addr, company) pair count co-occurrences and pick the
        // mode. Restricted to addresses we already accumulated to keep the
        // result set small.
        let mut companies: HashMap<String, HashMap<String, i32>> = HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT lower(e.sender_email) AS addr, t.tag_value
             FROM email_tags t
             JOIN emails e ON e.id = t.email_id
             WHERE e.account_id = ?1 AND e.is_deleted = 0 AND t.tag_type = 'company'",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows {
            let (addr, tag) = r?;
            if !accum.contains_key(&addr) {
                continue;
            }
            *companies.entry(addr).or_default().entry(tag).or_insert(0) += 1;
        }
        drop(stmt);

        // ── Materialise + score ──────────────────────────────────────────
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let mut all: Vec<Contact> = accum
            .into_iter()
            .map(|(addr, a)| {
                let total = a.received + a.sent;
                let domain = crate::util::email_addr::extract_domain(&addr).unwrap_or_default();
                let kind = classify_kind(&addr).to_string();
                let company = companies
                    .get(&addr)
                    .and_then(|m| m.iter().max_by_key(|(_, c)| *c))
                    .map(|(tag, _)| tag.clone());
                let score = relationship_score(&a, now_secs);
                Contact {
                    name: if a.name.is_empty() { addr.clone() } else { a.name },
                    email: addr,
                    email_count: total,
                    received_count: a.received,
                    sent_count: a.sent,
                    last_timestamp: a.last_ts,
                    first_timestamp: a.first_ts,
                    company,
                    kind,
                    domain,
                    relationship_score: score,
                }
            })
            .collect();

        // ── Filter ───────────────────────────────────────────────────────
        if let Some(k) = query.kind.as_deref() {
            if k != "all" && !k.is_empty() {
                all.retain(|c| c.kind == k);
            }
        }
        if let Some(co) = query.company.as_deref() {
            if !co.is_empty() {
                all.retain(|c| c.company.as_deref() == Some(co));
            }
        }
        if let Some(d) = query.domain.as_deref() {
            if !d.is_empty() {
                let dl = d.to_lowercase();
                all.retain(|c| c.domain == dl);
            }
        }
        if let Some(q) = query.search.as_deref() {
            let q = q.trim();
            if !q.is_empty() {
                let tokens: Vec<String> = q
                    .split_whitespace()
                    .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
                    .filter(|t| t.chars().count() >= 2)
                    .collect();
                if !tokens.is_empty() {
                    all.retain(|c| {
                        let hay = format!(
                            "{} {} {}",
                            c.name.to_lowercase(),
                            c.email.to_lowercase(),
                            c.company.as_deref().unwrap_or("").to_lowercase()
                        );
                        tokens.iter().all(|t| hay.contains(t.as_str()))
                    });
                }
            }
        }

        let total_after_filter = all.len() as i32;

        // ── Sort ─────────────────────────────────────────────────────────
        let sort = query.sort.as_deref().unwrap_or("last");
        match sort {
            "total" => all.sort_by(|a, b| {
                b.email_count
                    .cmp(&a.email_count)
                    .then_with(|| b.last_timestamp.cmp(&a.last_timestamp))
            }),
            "received" => all.sort_by(|a, b| {
                b.received_count
                    .cmp(&a.received_count)
                    .then_with(|| b.last_timestamp.cmp(&a.last_timestamp))
            }),
            "sent" => all.sort_by(|a, b| {
                b.sent_count
                    .cmp(&a.sent_count)
                    .then_with(|| b.last_timestamp.cmp(&a.last_timestamp))
            }),
            "name" => all.sort_by_key(|a| a.name.to_lowercase()),
            "score" => all.sort_by(|a, b| {
                b.relationship_score
                    .partial_cmp(&a.relationship_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.last_timestamp.cmp(&a.last_timestamp))
            }),
            _ => all.sort_by(|a, b| {
                b.last_timestamp
                    .cmp(&a.last_timestamp)
                    .then_with(|| b.email_count.cmp(&a.email_count))
            }),
        }

        // ── Paginate ─────────────────────────────────────────────────────
        let offset = query.offset.unwrap_or(0).max(0) as usize;
        // Generous upper bound: aggregation already materialises every
        // contact, so clamping is just to prevent absurd payloads.
        let limit = query.limit.unwrap_or(100).clamp(1, 5000) as usize;
        let end = (offset + limit).min(all.len());
        let items = if offset < all.len() {
            all[offset..end].to_vec()
        } else {
            Vec::new()
        };
        let has_more = end < total_after_filter as usize;

        Ok(ContactsPage {
            items,
            total: total_after_filter,
            has_more,
        })
    }

    /// Detail payload for the contact drawer header (Phase 3.1). Reuses
    /// `list_contacts` to compute the contact, then derives aliases by
    /// finding other addresses that share the same lower-cased display name.
    pub fn get_contact_detail(&self, account_id: &str, address: &str) -> Result<Option<ContactDetail>> {
        let addr_lc = address.trim().to_lowercase();
        if addr_lc.is_empty() {
            return Ok(None);
        }

        // Pull a generous page (5000 covers every realistic mailbox) so we
        // find the address in a single pass without paging.
        let page = self.list_contacts(
            account_id,
            &ContactsQuery {
                limit: Some(5000),
                ..Default::default()
            },
        )?;
        let contact = match page.items.into_iter().find(|c| c.email == addr_lc) {
            Some(c) => c,
            None => return Ok(None),
        };

        let aliases = if contact.name.is_empty() || contact.name == contact.email {
            Vec::new()
        } else {
            let conn = self.reader();
            let mut stmt = conn.prepare(
                "SELECT DISTINCT lower(sender_email) FROM emails
                 WHERE account_id = ?1 AND is_deleted = 0
                   AND lower(sender) = lower(?2)
                   AND lower(sender_email) != ?3
                 LIMIT 10",
            )?;
            let rows = stmt.query_map(params![account_id, contact.name, contact.email], |row| {
                row.get::<_, String>(0)
            })?;
            rows.filter_map(|r| r.ok()).collect()
        };

        Ok(Some(ContactDetail { contact, aliases }))
    }

    /// Group contacts by company tag (Phase 4.5). Contacts with no company
    /// are returned in a single trailing group with `company = None`.
    pub fn list_contacts_by_company(&self, account_id: &str) -> Result<Vec<CompanyContactsGroup>> {
        // Pull all contacts (cap at 5000 to bound memory); if a real account
        // exceeds that we're already in pathological territory.
        let page = self.list_contacts(
            account_id,
            &ContactsQuery {
                limit: Some(5000),
                ..Default::default()
            },
        )?;

        let mut groups: HashMap<Option<String>, Vec<Contact>> = HashMap::new();
        for c in page.items {
            groups.entry(c.company.clone()).or_default().push(c);
        }

        let mut result: Vec<CompanyContactsGroup> = groups
            .into_iter()
            .map(|(company, mut contacts)| {
                contacts.sort_by(|a, b| {
                    b.relationship_score
                        .partial_cmp(&a.relationship_score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| b.last_timestamp.cmp(&a.last_timestamp))
                });
                let total_emails: i32 = contacts.iter().map(|c| c.email_count).sum();
                let last_timestamp = contacts.iter().filter_map(|c| c.last_timestamp).max();
                CompanyContactsGroup {
                    company,
                    contacts,
                    total_emails,
                    last_timestamp,
                }
            })
            .collect();

        // Sort groups: named companies first by total emails desc, then the
        // unknown/None bucket last so it doesn't bury real groups.
        result.sort_by(|a, b| match (a.company.is_some(), b.company.is_some()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b.total_emails.cmp(&a.total_emails),
        });

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use crate::db::Database;
    use crate::models::ContactsQuery;

    #[test]
    fn list_contacts_aggregates_inbound_and_outbound() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@me.com");

        // Two inbound from alice
        insert_contact_email(&db, "e1", "acc1", "t-a", "Alice", "alice@acme.com", "[]", "inbox", 1000);
        insert_contact_email(&db, "e2", "acc1", "t-a", "Alice", "alice@acme.com", "[]", "inbox", 2000);

        // One outbound from me to alice + bob
        insert_contact_email(
            &db,
            "e3",
            "acc1",
            "t-a",
            "Me",
            "me@me.com",
            "[\"Alice <alice@acme.com>\", \"bob@beta.com\"]",
            "sent",
            3000,
        );

        // One inbound from a self-loop (must be excluded)
        insert_contact_email(&db, "e4", "acc1", "t-x", "Me", "me@me.com", "[]", "inbox", 4000);

        let page = db
            .list_contacts(
                "acc1",
                &ContactsQuery {
                    sort: Some("total".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        let alice = page
            .items
            .iter()
            .find(|c| c.email == "alice@acme.com")
            .expect("alice present");
        assert_eq!(alice.received_count, 2, "alice has 2 inbound");
        assert_eq!(alice.sent_count, 1, "alice has 1 outbound");
        assert_eq!(alice.email_count, 3);
        assert_eq!(alice.last_timestamp, Some(3000));
        assert_eq!(alice.first_timestamp, Some(1000));

        let bob = page
            .items
            .iter()
            .find(|c| c.email == "bob@beta.com")
            .expect("bob present");
        assert_eq!(bob.received_count, 0);
        assert_eq!(bob.sent_count, 1);
        assert_eq!(bob.last_timestamp, Some(3000));

        // Self must not appear
        assert!(
            !page.items.iter().any(|c| c.email == "me@me.com"),
            "self address must be excluded, got: {:?}",
            page.items.iter().map(|c| &c.email).collect::<Vec<_>>()
        );
    }

    #[test]
    fn list_contacts_counts_gmail_style_sent_emails() {
        // Gmail stores user-sent emails with mailbox='inbox' (no 'sent' label
        // applied at sync time). The outbound pass must still count them.
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@me.com");

        // One inbound from alice
        insert_contact_email(&db, "e1", "acc1", "t-a", "Alice", "alice@acme.com", "[]", "inbox", 1000);

        // Two user-sent emails to alice — but stored as 'inbox' (Gmail pattern).
        insert_contact_email(
            &db,
            "e2",
            "acc1",
            "t-a",
            "Me",
            "me@me.com",
            "[\"Alice <alice@acme.com>\"]",
            "inbox",
            2000,
        );
        insert_contact_email(
            &db,
            "e3",
            "acc1",
            "t-a",
            "Me",
            "me@me.com",
            "[\"Alice <alice@acme.com>\"]",
            "inbox",
            3000,
        );

        // One classic IMAP-style sent row (mailbox='sent') — must also count.
        insert_contact_email(
            &db,
            "e4",
            "acc1",
            "t-a",
            "Me",
            "me@me.com",
            "[\"Alice <alice@acme.com>\"]",
            "sent",
            4000,
        );

        let page = db.list_contacts("acc1", &ContactsQuery::default()).unwrap();

        let alice = page
            .items
            .iter()
            .find(|c| c.email == "alice@acme.com")
            .expect("alice present");
        assert_eq!(alice.received_count, 1, "1 inbound");
        assert_eq!(alice.sent_count, 3, "3 outbound (2 gmail-style + 1 classic)");

        // Self-edge guard still holds (recipient == self_email is skipped).
        assert!(
            !page.items.iter().any(|c| c.email == "me@me.com"),
            "self address must not appear as a contact"
        );
    }

    #[test]
    fn list_contacts_classifies_kind() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@me.com");

        insert_contact_email(
            &db,
            "e1",
            "acc1",
            "t-a",
            "No Reply",
            "no-reply@news.com",
            "[]",
            "inbox",
            100,
        );
        insert_contact_email(
            &db,
            "e2",
            "acc1",
            "t-b",
            "Real Person",
            "alice@acme.com",
            "[]",
            "inbox",
            200,
        );
        insert_contact_email(
            &db,
            "e3",
            "acc1",
            "t-c",
            "Notify",
            "notifications@acme.com",
            "[]",
            "inbox",
            300,
        );
        // Patterns the user reported as still misclassified.
        insert_contact_email(
            &db,
            "e4",
            "acc1",
            "t-d",
            "NoResponder",
            "noresponder@foo.com",
            "[]",
            "inbox",
            400,
        );
        insert_contact_email(
            &db,
            "e5",
            "acc1",
            "t-e",
            "Substack",
            "writer@blog.substack.com",
            "[]",
            "inbox",
            500,
        );
        insert_contact_email(
            &db,
            "e6",
            "acc1",
            "t-f",
            "Newsletter",
            "newsletter@brand.com",
            "[]",
            "inbox",
            600,
        );
        insert_contact_email(
            &db,
            "e7",
            "acc1",
            "t-g",
            "Order Updates",
            "order-updates@store.com",
            "[]",
            "inbox",
            700,
        );
        insert_contact_email(
            &db,
            "e8",
            "acc1",
            "t-h",
            "Shipment",
            "shipment@store.com",
            "[]",
            "inbox",
            800,
        );
        insert_contact_email(
            &db,
            "e9",
            "acc1",
            "t-i",
            "Confirmar",
            "confirmar@bank.com",
            "[]",
            "inbox",
            900,
        );
        insert_contact_email(
            &db,
            "e10",
            "acc1",
            "t-j",
            "Hola",
            "hola@agency.com",
            "[]",
            "inbox",
            1000,
        );
        insert_contact_email(
            &db,
            "e11",
            "acc1",
            "t-k",
            "Hello",
            "hello@startup.com",
            "[]",
            "inbox",
            1100,
        );
        insert_contact_email(
            &db,
            "e12",
            "acc1",
            "t-l",
            "Sales",
            "sales@vendor.com",
            "[]",
            "inbox",
            1200,
        );
        insert_contact_email(
            &db,
            "e13",
            "acc1",
            "t-m",
            "Tienda",
            "tienda@shop.com",
            "[]",
            "inbox",
            1300,
        );
        insert_contact_email(
            &db,
            "e14",
            "acc1",
            "t-n",
            "Office",
            "office@firm.com",
            "[]",
            "inbox",
            1400,
        );
        insert_contact_email(
            &db,
            "e15",
            "acc1",
            "t-o",
            "Contact",
            "contact@firm.com",
            "[]",
            "inbox",
            1500,
        );
        insert_contact_email(
            &db,
            "e16",
            "acc1",
            "t-p",
            "Ayuda",
            "ayuda@firm.com",
            "[]",
            "inbox",
            1600,
        );
        insert_contact_email(&db, "e17", "acc1", "t-q", "Help", "help@firm.com", "[]", "inbox", 1700);
        insert_contact_email(&db, "e18", "acc1", "t-r", "News", "news@brand.com", "[]", "inbox", 1800);
        insert_contact_email(
            &db,
            "e19",
            "acc1",
            "t-s",
            "Replies",
            "replies@brand.com",
            "[]",
            "inbox",
            1900,
        );
        insert_contact_email(&db, "e20", "acc1", "t-t", "Chat", "chat@brand.com", "[]", "inbox", 2000);
        // Financial / transactional roles
        insert_contact_email(
            &db,
            "e21",
            "acc1",
            "t-u",
            "Invoice",
            "invoice@vendor.com",
            "[]",
            "inbox",
            2100,
        );
        insert_contact_email(
            &db,
            "e22",
            "acc1",
            "t-v",
            "Receipts",
            "receipts@vendor.com",
            "[]",
            "inbox",
            2200,
        );
        insert_contact_email(
            &db,
            "e23",
            "acc1",
            "t-w",
            "Payments",
            "payments@bank.com",
            "[]",
            "inbox",
            2300,
        );
        insert_contact_email(
            &db,
            "e24",
            "acc1",
            "t-x",
            "Billing-Noreply",
            "billing-noreply@vendor.com",
            "[]",
            "inbox",
            2400,
        );
        // Brand pattern: local == domain root
        insert_contact_email(
            &db,
            "e25",
            "acc1",
            "t-y",
            "Sifted",
            "sifted@sifted.eu",
            "[]",
            "inbox",
            2500,
        );
        insert_contact_email(
            &db,
            "e26",
            "acc1",
            "t-z",
            "Medium",
            "medium@medium.com",
            "[]",
            "inbox",
            2600,
        );
        insert_contact_email(
            &db,
            "e27",
            "acc1",
            "t-aa",
            "LinkedIn",
            "linkedin@email.linkedin.com",
            "[]",
            "inbox",
            2700,
        );
        // Negative: same local as person, domain root differs — must stay person.
        insert_contact_email(
            &db,
            "e28",
            "acc1",
            "t-ab",
            "George",
            "george@example.com",
            "[]",
            "inbox",
            2800,
        );

        let page = db.list_contacts("acc1", &ContactsQuery::default()).unwrap();

        let by = |addr: &str| page.items.iter().find(|c| c.email == addr).cloned().unwrap();
        assert_eq!(by("no-reply@news.com").kind, "automated");
        assert_eq!(by("notifications@acme.com").kind, "automated");
        assert_eq!(by("alice@acme.com").kind, "person");
        // New patterns must all classify as automated.
        for addr in [
            "noresponder@foo.com",
            "writer@blog.substack.com",
            "newsletter@brand.com",
            "order-updates@store.com",
            "shipment@store.com",
            "confirmar@bank.com",
            "hola@agency.com",
            "hello@startup.com",
            "sales@vendor.com",
            "tienda@shop.com",
            "office@firm.com",
            "contact@firm.com",
            "ayuda@firm.com",
            "help@firm.com",
            "news@brand.com",
            "replies@brand.com",
            "chat@brand.com",
            "invoice@vendor.com",
            "receipts@vendor.com",
            "payments@bank.com",
            "billing-noreply@vendor.com",
            "sifted@sifted.eu",
            "medium@medium.com",
            "linkedin@email.linkedin.com",
        ] {
            assert_eq!(by(addr).kind, "automated", "{addr} should be automated");
        }
        assert_eq!(
            by("george@example.com").kind,
            "person",
            "local != domain root must stay as person"
        );

        // Kind filter should narrow the set
        let people = db
            .list_contacts(
                "acc1",
                &ContactsQuery {
                    kind: Some("person".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(people.items.iter().all(|c| c.kind == "person"));
        assert!(people.items.iter().any(|c| c.email == "alice@acme.com"));
    }

    #[test]
    fn list_contacts_company_enrichment_and_groups() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@me.com");

        insert_contact_email(&db, "e1", "acc1", "t-a", "Alice", "alice@acme.com", "[]", "inbox", 100);
        insert_contact_email(&db, "e2", "acc1", "t-b", "Bob", "bob@beta.com", "[]", "inbox", 200);
        // Tag e1 with company=acme
        tag_email(&db, "e1", "company", "acme");

        let page = db.list_contacts("acc1", &ContactsQuery::default()).unwrap();
        let alice = page.items.iter().find(|c| c.email == "alice@acme.com").unwrap();
        assert_eq!(alice.company.as_deref(), Some("acme"));
        let bob = page.items.iter().find(|c| c.email == "bob@beta.com").unwrap();
        assert_eq!(bob.company, None);

        // Group by company: acme group + None group.
        let groups = db.list_contacts_by_company("acc1").unwrap();
        let acme = groups
            .iter()
            .find(|g| g.company.as_deref() == Some("acme"))
            .expect("acme group");
        assert_eq!(acme.contacts.len(), 1);
        let none = groups.iter().find(|g| g.company.is_none()).expect("None group");
        assert_eq!(none.contacts.len(), 1);
        // Named groups sort before None
        assert!(groups.first().unwrap().company.is_some());
    }

    #[test]
    fn list_contacts_search_and_pagination() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@me.com");

        for i in 0..25 {
            insert_contact_email(
                &db,
                &format!("e{i}"),
                "acc1",
                &format!("t-{i}"),
                "Sender",
                &format!("user{i}@host.com"),
                "[]",
                "inbox",
                100 + i,
            );
        }

        let p1 = db
            .list_contacts(
                "acc1",
                &ContactsQuery {
                    limit: Some(10),
                    offset: Some(0),
                    sort: Some("name".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(p1.items.len(), 10);
        assert_eq!(p1.total, 25);
        assert!(p1.has_more);

        // Search narrows the set
        let s = db
            .list_contacts(
                "acc1",
                &ContactsQuery {
                    search: Some("user1".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        // user1, user10..user19 → 11 hits
        assert_eq!(s.total, 11, "search should match 11 user1* contacts");
    }

    #[test]
    fn get_contact_detail_returns_aliases_with_same_name() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@me.com");

        insert_contact_email(
            &db,
            "e1",
            "acc1",
            "t-a",
            "John Smith",
            "john@old.com",
            "[]",
            "inbox",
            100,
        );
        insert_contact_email(
            &db,
            "e2",
            "acc1",
            "t-b",
            "John Smith",
            "john@new.com",
            "[]",
            "inbox",
            200,
        );

        let detail = db
            .get_contact_detail("acc1", "john@old.com")
            .unwrap()
            .expect("contact present");
        assert_eq!(detail.contact.email, "john@old.com");
        assert_eq!(detail.aliases, vec!["john@new.com".to_string()]);
    }
}
