use super::*;

/// One classifier tag to filter by.
///
/// `tag_type` is optional because the two callers know different things: the
/// chat tool filters an `intent` or a `topic` and must not match a company
/// that happens to share the name, while the search box's `tag:` operator
/// takes a bare value the user typed, with no type attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagQuery {
    pub tag_type: Option<String>,
    pub value: String,
}

impl TagQuery {
    /// Any tag type with this value — the `tag:` operator's historical meaning.
    pub fn any_type(value: impl Into<String>) -> Self {
        Self {
            tag_type: None,
            value: value.into(),
        }
    }

    /// One specific tag type and value.
    pub fn typed(tag_type: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            tag_type: Some(tag_type.into()),
            value: value.into(),
        }
    }
}

/// Split a `Name <addr@host>` header into its display name and address.
/// A bare address yields an empty name.
fn split_addressee(raw: &str) -> (String, String) {
    let raw = raw.trim();
    if let Some(open) = raw.rfind('<') {
        if let Some(close) = raw[open..].find('>') {
            let address = raw[open + 1..open + close].trim().to_string();
            let name = raw[..open].trim().trim_matches('"').trim().to_string();
            return (name, address);
        }
    }
    (String::new(), raw.to_string())
}

impl Database {
    /// Get aggregate stats for smart filter suggestions, excluding removed filters.
    ///
    /// Counts are DISTINCT threads over inbox/sent mail only, so the sidebar
    /// number matches the rows `get_filtered_emails` shows when the suggestion
    /// is clicked (which is thread-representative and mailbox-scoped the same
    /// way). Sender grouping/exclusion is case-insensitive because providers
    /// vary the casing of one address across messages, and the account owner's
    /// own address is excluded (sent mail would always rank it first).
    ///
    /// Under `AllEnabled` (unified inbox): counts dedup by
    /// `(account_id, thread_id)` — thread ids are not globally unique — and
    /// EVERY enabled account's own address is excluded from sender stats.
    pub fn get_quick_filter_stats(
        &self,
        scope: crate::db::AccountScope<'_>,
        excluded_domains: &[String],
        excluded_senders: &[String],
    ) -> Result<QuickFilterStats> {
        let conn = self.reader();
        let live = crate::db::live_mailboxes_sql!();

        // Scope-dependent SQL fragments. The account id (when present) binds
        // as ?1; exclusion placeholders start after it.
        let (scope_cond, own_address_cond, account_param): (&str, &str, Option<&str>) = match scope {
            crate::db::AccountScope::Account(id) => (
                "account_id = ?1",
                // COALESCE guards the (test-only) case of a missing accounts
                // row — `<> NULL` would otherwise filter out every sender.
                "sender_email COLLATE NOCASE <> COALESCE((SELECT email FROM accounts WHERE id = ?1), '')",
                Some(id),
            ),
            crate::db::AccountScope::AllEnabled => (
                "account_id IN (SELECT id FROM accounts WHERE enabled = 1)",
                "NOT EXISTS (SELECT 1 FROM accounts a WHERE a.enabled = 1 \
                 AND LOWER(a.email) = LOWER(sender_email))",
                None,
            ),
        };
        let first_exclude_idx = if account_param.is_some() { 2 } else { 1 };
        // Thread count that stays correct across accounts: thread ids are only
        // unique per account, so dedup on the (account_id, thread_id) pair.
        let thread_cnt = "COUNT(DISTINCT account_id || ':' || thread_id)";

        // Top 10 sender domains, excluding removed ones
        let domain_exclude_clause = if excluded_domains.is_empty() {
            String::new()
        } else {
            let placeholders: Vec<String> = excluded_domains
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + first_exclude_idx))
                .collect();
            format!("AND sender_domain NOT IN ({})", placeholders.join(", "))
        };

        let domain_sql = format!(
            "SELECT sender_domain AS domain,
                    {thread_cnt} AS cnt
             FROM emails WHERE {scope_cond}
               AND sender_domain != ''
               AND is_deleted = 0
               AND mailbox IN {live} {domain_exclude_clause}
             GROUP BY domain
             ORDER BY cnt DESC LIMIT 10"
        );

        let mut domain_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(id) = account_param {
            domain_params.push(Box::new(id.to_string()));
        }
        for d in excluded_domains {
            domain_params.push(Box::new(d.clone()));
        }
        let domain_refs: Vec<&dyn rusqlite::ToSql> = domain_params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&domain_sql)?;
        let top_domains: Vec<FilterSuggestion> = stmt
            .query_map(domain_refs.as_slice(), |row| {
                Ok(FilterSuggestion {
                    value: row.get(0)?,
                    count: row.get(1)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Top 10 individual senders, excluding removed ones (case-insensitively)
        let sender_exclude_clause = if excluded_senders.is_empty() {
            String::new()
        } else {
            let placeholders: Vec<String> = excluded_senders
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + first_exclude_idx))
                .collect();
            format!("AND sender_email COLLATE NOCASE NOT IN ({})", placeholders.join(", "))
        };

        // MIN(sender_email) picks a deterministic representative casing for
        // each NOCASE group.
        let sender_sql = format!(
            "SELECT MIN(sender_email), {thread_cnt} AS cnt
             FROM emails WHERE {scope_cond}
               AND is_deleted = 0
               AND mailbox IN {live}
               AND {own_address_cond} {sender_exclude_clause}
             GROUP BY sender_email COLLATE NOCASE
             ORDER BY cnt DESC LIMIT 10"
        );

        let mut sender_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(id) = account_param {
            sender_params.push(Box::new(id.to_string()));
        }
        for s in excluded_senders {
            sender_params.push(Box::new(s.clone()));
        }
        let sender_refs: Vec<&dyn rusqlite::ToSql> = sender_params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sender_sql)?;
        let top_senders: Vec<FilterSuggestion> = stmt
            .query_map(sender_refs.as_slice(), |row| {
                Ok(FilterSuggestion {
                    value: row.get(0)?,
                    count: row.get(1)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(QuickFilterStats {
            top_domains,
            top_senders,
        })
    }

    /// The junk detector's spam/phishing exclusion as a bare WHERE term
    /// (`exclude_junk_sql` returns it with a leading `AND` for callers that
    /// append it to a finished clause).
    fn junk_condition(alias: &str) -> String {
        Self::junk_condition_with(alias, false)
    }

    /// [`Self::junk_condition`], also dropping graymail when `hide_graymail`.
    fn junk_condition_with(alias: &str, hide_graymail: bool) -> String {
        crate::db::exclude_junk_sql(alias, hide_graymail)
            .trim_start()
            .trim_start_matches("AND ")
            .to_string()
    }

    /// Mailboxes a smart filter reaches: the live ones plus custom folders,
    /// never Spam or Trash. The same list the search box uses, so a filter
    /// and its `tag:` / `from:` token list the same mail.
    fn filterable_mailbox(alias: &str) -> String {
        format!(
            "({alias}.mailbox IN {live} OR {alias}.mailbox LIKE 'folder:%')",
            live = crate::db::live_mailboxes_sql!()
        )
    }

    /// Everything after the `matched (aid, tid, mid, mts)` CTE both
    /// `get_filtered_emails` branches build: one row per thread — its newest
    /// MATCHING email — sorted by the thread's newest email of any kind, so an
    /// old thread that got a reply today sorts as today.
    ///
    /// `ROW_NUMBER` picks the representative in one pass over the matches with
    /// the inbox's `timestamp DESC, id DESC` order, so two matches stamped in
    /// the same second still yield exactly one row.
    ///
    /// The thread's latest timestamp MUST be an indexed scalar subquery per
    /// thread. A `emails JOIN matched GROUP BY` shape regressed to 138s on a
    /// 90k-email DB (unified intent filter): SQLite scanned each matched
    /// thread against `idx_emails_account_mailbox` — which lacks `thread_id`.
    /// The `INDEXED BY idx_emails_thread_latest` hint is load-bearing: with it
    /// each lookup is a single (account_id, thread_id) seek.
    ///
    /// CROSS JOIN pins the join order (SQLite never reorders CROSS JOIN) so
    /// the probe drives from the small `thread_latest` set into the emails
    /// primary key, never the reverse. The thread timestamp is the last column.
    fn representative_tail(limit_idx: usize, offset_idx: usize) -> String {
        format!(
            "picked AS (
                 SELECT aid, tid, mid,
                        ROW_NUMBER() OVER (PARTITION BY aid, tid ORDER BY mts DESC, mid DESC) AS rn
                 FROM matched
             ),
             thread_latest AS (
                 SELECT p.mid AS rep_id,
                        (SELECT e3.timestamp
                         FROM emails e3 INDEXED BY idx_emails_thread_latest
                         WHERE e3.account_id = p.aid AND e3.thread_id = p.tid
                           AND e3.is_deleted = 0 AND {filterable}
                         ORDER BY e3.timestamp DESC, e3.id DESC
                         LIMIT 1) AS thread_ts
                 FROM picked p
                 WHERE p.rn = 1
             )
             SELECT {cols}, l.thread_ts
             FROM thread_latest l
             CROSS JOIN emails e
             WHERE e.id = l.rep_id
             ORDER BY l.thread_ts DESC, e.timestamp DESC, e.id DESC
             LIMIT ?{limit_idx} OFFSET ?{offset_idx}",
            filterable = Self::filterable_mailbox("e3"),
            cols = EMAIL_COLUMNS,
        )
    }

    /// Read the rows [`Self::representative_tail`] returns.
    fn read_representatives(rows: &mut rusqlite::Rows<'_>) -> Result<FilteredEmailsResult> {
        let thread_ts_idx = EMAIL_COLUMNS.split(',').count();
        let mut emails = Vec::new();
        let mut thread_latest_at = std::collections::HashMap::new();
        while let Some(row) = rows.next()? {
            let email = row_to_email(row)?;
            let thread_ts: i64 = row.get(thread_ts_idx)?;
            thread_latest_at.insert(email.id.clone(), thread_ts);
            emails.push(email);
        }
        Ok(FilteredEmailsResult {
            emails,
            total_count: -1,
            thread_latest_at,
        })
    }

    /// Get emails filtered by domain or sender, with total count for pagination.
    ///
    /// Uses subquery-based approach to avoid both:
    ///   - O(N²) NOT EXISTS correlated subquery
    ///   - SQLite parameter limit (32,766) for large thread_id sets
    ///
    /// The matching thread_ids stay inside a CTE subquery; only the scope's
    /// account id (when single-account) and the filter value are parameters.
    ///
    /// Thread dedup keys on `(account_id, thread_id)` in BOTH scopes — a no-op
    /// for single-account queries (account is constant) and required under
    /// `AllEnabled`, where two accounts CC'd on one provider thread share the
    /// same thread_id string and must each keep their own representative row.
    pub fn get_filtered_emails(
        &self,
        scope: crate::db::AccountScope<'_>,
        domain: Option<&str>,
        sender_email: Option<&str>,
        tag_type: Option<&str>,
        tag_value: Option<&str>,
        attachment_ext: Option<&str>,
        window: &crate::models::EmailWindow,
        limit: i32,
        offset: i32,
    ) -> Result<FilteredEmailsResult> {
        let conn = self.reader();

        // Scope condition for an aliased emails table. Under Account the id
        // binds as ?1 in every query built below.
        let scope_cond = |alias: &str| -> String {
            match scope {
                crate::db::AccountScope::Account(_) => format!("{alias}.account_id = ?1"),
                crate::db::AccountScope::AllEnabled => {
                    format!("{alias}.account_id IN (SELECT id FROM accounts WHERE enabled = 1)")
                }
            }
        };
        let account_param: Option<&str> = match scope {
            crate::db::AccountScope::Account(id) => Some(id),
            crate::db::AccountScope::AllEnabled => None,
        };

        // ── Tag filter: drive from email_tags (small result set) ─────────────
        // Tags are selective — few emails match. Drive from the tag index, find
        // matching threads, GROUP BY for latest-per-thread. Fast even with 0 matches.
        if let (Some(tt), Some(tv)) = (tag_type, tag_value) {
            // Semantics (user-confirmed): a thread matches the filter if ANY email
            // in the thread carries the tag, so replying to "Globex" doesn't make
            // the thread disappear from the Globex filter. The row shown is the
            // thread's newest email that carries the tag; the thread sorts by its
            // newest email of any kind (see `representative_tail`).
            //
            // `filterable_mailbox` keeps Spam/Trash copies out of the filtered
            // views while reaching archived mail and custom folders.
            // `INDEXED BY idx_email_tags_type_value` is critical: without it SQLite
            // picks the inverted plan — scan all ~87k emails of the account and
            // probe email_tags by email_id — instead of starting from the tag
            // (which yields ~hundreds of rows). With the hint, matched_threads
            // costs O(emails_tagged_with_this_value), not O(account_emails).
            let first_idx = if account_param.is_some() { 2 } else { 1 };
            // Window binds land after limit/offset so the fixed indices above
            // keep their positions.
            let junk_sql = crate::db::exclude_junk_sql("e2", window.hide_graymail);
            // The board asks for `latest_tag_only`; the sidebar filter never
            // does, so the "ANY email in the thread" rule above is untouched.
            let latest_sql = crate::db::latest_tagged_in_thread_sql("e2", first_idx, window.latest_tag_only);
            let mut window_idx = first_idx + 4;
            let (window_sql, window_binds) = window.sql("e2", &mut window_idx);
            let select_sql = format!(
                "WITH matched AS (
                     SELECT e2.account_id AS aid, e2.thread_id AS tid, e2.id AS mid, e2.timestamp AS mts
                     FROM email_tags et INDEXED BY idx_email_tags_type_value
                     JOIN emails e2 ON e2.id = et.email_id
                     WHERE et.tag_type = ?{tt_idx} AND et.tag_value = ?{tv_idx}
                       AND {scope_e2} AND e2.is_deleted = 0
                       AND {filterable}
                       {junk_sql}
                       {latest_sql}
                       {window_sql}
                 ),
                 {tail}",
                tt_idx = first_idx,
                tv_idx = first_idx + 1,
                scope_e2 = scope_cond("e2"),
                filterable = Self::filterable_mailbox("e2"),
                tail = Self::representative_tail(first_idx + 2, first_idx + 3),
            );

            let mut stmt = conn.prepare(&select_sql)?;
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            if let Some(id) = account_param {
                params_vec.push(Box::new(id.to_string()));
            }
            params_vec.push(Box::new(tt.to_string()));
            params_vec.push(Box::new(tv.to_string()));
            params_vec.push(Box::new(limit));
            params_vec.push(Box::new(offset));
            params_vec.extend(window_binds);
            let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
            let mut rows = stmt.query(params_refs.as_slice())?;
            return Self::read_representatives(&mut rows);
        }

        // ── Domain/sender filter: three-step CTE ───────────────────────────────
        // Step 1: find matching (account_id, thread_id) pairs via
        //         idx_emails_domain_filter or idx_emails_sender_filter
        //         (covering, no table access needed).
        // Step 2: pick each thread's newest match and its latest timestamp —
        //         O(matching_threads) (see `representative_tail`).
        // Step 3: join back to emails to fetch the representative row.
        // This is O(emails_from_domain) rather than O(all_emails) and avoids
        // scanning the full inbox in timestamp order.
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let mut param_idx = 1usize;

        // Inbox-level filtered view: exclude Spam/Trash copies and junk so they
        // don't leak into the main filter UI. Soft-deleted rows are also excluded.
        let mut match_conditions = vec![
            "is_deleted = 0".to_string(),
            Self::filterable_mailbox("emails"),
            Self::junk_condition_with("emails", window.hide_graymail),
        ];
        match scope {
            crate::db::AccountScope::Account(id) => {
                match_conditions.push(format!("account_id = ?{param_idx}"));
                params_vec.push(Box::new(id.to_string()));
                param_idx += 1;
            }
            crate::db::AccountScope::AllEnabled => {
                match_conditions.push("account_id IN (SELECT id FROM accounts WHERE enabled = 1)".to_string());
            }
        }
        if let Some(d) = domain {
            match_conditions.push(format!("sender_domain = ?{}", param_idx));
            params_vec.push(Box::new(d.to_lowercase()));
            param_idx += 1;
        }
        if let Some(s) = sender_email {
            // NOCASE: the clicked suggestion (or a blocked header address) may
            // differ in case from the stored rows. Served by
            // idx_emails_sender_email_nocase.
            match_conditions.push(format!("sender_email = ?{} COLLATE NOCASE", param_idx));
            params_vec.push(Box::new(s.to_string()));
            param_idx += 1;
        }
        if let Some(ext) = attachment_ext {
            // `emails.id` must be qualified: a bare `id` binds to the
            // innermost table (`am.id`) and the filter would never match.
            match_conditions.push(format!(
                "EXISTS (SELECT 1 FROM email_attachment_meta am WHERE am.email_id = emails.id AND LOWER(am.filename) LIKE ?{})",
                param_idx
            ));
            params_vec.push(Box::new(format!("%.{}", ext.to_lowercase())));
            param_idx += 1;
        }

        let (window_sql, window_binds) = window.sql("emails", &mut param_idx);
        let select_sql = format!(
            "WITH matched AS (
                 SELECT account_id AS aid, thread_id AS tid, id AS mid, timestamp AS mts
                 FROM emails
                 WHERE {match_cond}{window_sql}
             ),
             {tail}",
            match_cond = match_conditions.join(" AND "),
            tail = Self::representative_tail(param_idx, param_idx + 1),
        );

        params_vec.extend(window_binds);
        params_vec.push(Box::new(limit));
        params_vec.push(Box::new(offset));

        let mut stmt = conn.prepare(&select_sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let mut rows = stmt.query(params_refs.as_slice())?;
        Self::read_representatives(&mut rows)
    }

    /// Sender and recipient pairs for each of the given threads, newest message
    /// first, appended into `out` keyed by thread id.
    ///
    /// Returns `(display_name, address)` pairs — the caller dedupes by address
    /// and decides which name to show. Kept as a separate query rather than
    /// bolted onto `get_filtered_emails`: that one is shared with the sidebar,
    /// and its join order is load-bearing (see the CROSS JOIN note in
    /// `db::tags`).
    pub fn read_thread_people(
        &self,
        account_id: &str,
        thread_ids: &[String],
        out: &mut std::collections::HashMap<String, Vec<(String, String)>>,
    ) -> Result<()> {
        if thread_ids.is_empty() {
            return Ok(());
        }
        let placeholders = std::iter::repeat_n("?", thread_ids.len()).collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT thread_id, sender, sender_email, recipients_json, cc_json
             FROM emails
             WHERE account_id = ?1 AND is_deleted = 0
               AND thread_id IN ({placeholders})
             ORDER BY timestamp DESC, id DESC"
        );

        let conn = self.reader();
        let mut stmt = conn.prepare(&sql)?;
        let mut bound: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(1 + thread_ids.len());
        bound.push(&account_id);
        for t in thread_ids {
            bound.push(t);
        }

        let mut rows = stmt.query(bound.as_slice())?;
        while let Some(row) = rows.next()? {
            let thread_id: String = row.get(0)?;
            let sender: String = row.get(1)?;
            let sender_email: String = row.get(2)?;
            let recipients: Vec<String> =
                serde_json::from_str(&row.get::<_, String>(3).unwrap_or_default()).unwrap_or_default();
            let cc: Vec<String> =
                serde_json::from_str(&row.get::<_, String>(4).unwrap_or_default()).unwrap_or_default();

            let people = out.entry(thread_id).or_default();
            people.push((sender, sender_email));
            for raw in recipients.into_iter().chain(cc) {
                people.push(split_addressee(&raw));
            }
        }
        Ok(())
    }

    /// Date-only search: return all individual emails in the given window,
    /// ordered newest-first. Used when there are no text-based filters so that
    /// thread deduplication does NOT hide emails — `since=today` should list
    /// every email received today, not just one representative per thread.
    fn search_emails_by_date(
        &self,
        account_id: &str,
        categories: Option<&[String]>,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
        limit: i32,
        ascending: bool,
        exclude_spam: bool,
        unread_only: bool,
        received_only: bool,
    ) -> Result<Vec<Email>> {
        let conn = self.reader();
        let mut conditions: Vec<String> = vec![
            "e.account_id = ?1".to_string(),
            "e.is_deleted = 0".to_string(),
            // Spam/trash never surface in search (see search_emails_inner).
            "e.mailbox NOT IN ('spam', 'trash')".to_string(),
        ];
        if exclude_spam {
            conditions.push(Self::junk_condition("e"));
        }
        if unread_only {
            conditions.push("e.is_read = 0".to_string());
        }
        if received_only {
            conditions.push("e.is_sent = 0".to_string());
        }
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.to_string())];
        let mut param_idx = 2usize;

        if let Some(cats) = categories.filter(|c| !c.is_empty()) {
            let placeholders: Vec<String> = (0..cats.len()).map(|i| format!("?{}", param_idx + i)).collect();
            conditions.push(format!("e.category IN ({})", placeholders.join(", ")));
            for cat in cats {
                params_vec.push(Box::new(cat.clone()));
            }
            param_idx += cats.len();
        }
        if let Some(after) = after_timestamp {
            conditions.push(format!("e.timestamp >= ?{}", param_idx));
            params_vec.push(Box::new(after));
            param_idx += 1;
        }
        if let Some(before) = before_timestamp {
            conditions.push(format!("e.timestamp <= ?{}", param_idx));
            params_vec.push(Box::new(before));
            param_idx += 1;
        }

        let where_clause = conditions.join(" AND ");
        let sql = format!(
            "SELECT {cols} FROM emails e WHERE {where} ORDER BY {order} LIMIT ?{limit_idx}",
            cols = EMAIL_COLUMNS,
            where = where_clause,
            order = thread_order_clause("e", ascending),
            limit_idx = param_idx,
        );
        params_vec.push(Box::new(limit));

        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let emails = stmt.query_map(params_refs.as_slice(), row_to_email)?;
        let mut result = Vec::new();
        for email in emails {
            result.push(email?);
        }
        Ok(result)
    }

    /// Search emails using text matching.
    /// Supports structured filters like from:, to:, subject: as well as plain keyword search.
    ///
    /// Query strategy: a CTE (`filter_match`) finds the distinct thread_ids that contain at
    /// least one email satisfying all filter conditions.  The outer query then locates the
    /// thread-representative email (latest, non-deleted) for each matching thread.
    ///
    /// This is significantly faster than the old approach at scale because:
    /// - `from:` uses a prefix match on the indexed `sender_email` column plus an FTS5
    ///   sender-field lookup, instead of `LIKE '%value%'` (full table scan).
    /// - `subject:` routes through the FTS5 subject column rather than `LIKE '%value%'`.
    /// - The correlated NOT EXISTS predicate (find latest in thread) only evaluates for
    ///   the small set of emails in matching threads, not for every row in the account.
    #[allow(clippy::too_many_arguments)]
    pub fn search_emails(
        &self,
        account_id: &str,
        query: &str,
        categories: Option<&[String]>,
        from_filter: Option<&str>,
        to_filter: Option<&str>,
        subject_filter: Option<&str>,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
        tag_filters: Option<&[TagQuery]>,
        limit: i32,
    ) -> Result<Vec<Email>> {
        // Default ordering is newest-first (the long-standing behaviour every
        // existing caller relies on). "First / oldest" queries use
        // `search_emails_ordered(.., true)`.
        self.search_emails_inner(
            account_id,
            query,
            categories,
            from_filter,
            to_filter,
            subject_filter,
            after_timestamp,
            before_timestamp,
            tag_filters,
            limit,
            false,
            false,
            false,
            false,
            None,
            SearchShape::Lookup,
        )
        .map(drop_sort_keys)
    }

    /// Same as [`search_emails`](Self::search_emails) but with an explicit sort
    /// direction. `ascending == true` returns oldest-first — the only way to
    /// surface the *first* email matching a filter ("primer correo", "first email
    /// I sent to X"). Default callers should keep using `search_emails` (newest).
    #[allow(clippy::too_many_arguments)]
    pub fn search_emails_ordered(
        &self,
        account_id: &str,
        query: &str,
        categories: Option<&[String]>,
        from_filter: Option<&str>,
        to_filter: Option<&str>,
        subject_filter: Option<&str>,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
        tag_filters: Option<&[TagQuery]>,
        limit: i32,
        ascending: bool,
        // `true` drops mail the junk detector called spam or phishing (unless
        // the user overrode it) — what the chat wants; the app's own search
        // box keeps everything reachable.
        exclude_spam: bool,
        // `true` keeps only mail the user has not read. Applied in SQL, so an
        // `ascending` + `limit` query returns the oldest UNREAD email.
        unread_only: bool,
        // `true` keeps only mail the user received (not their own sent mail).
        // Applied in SQL like `unread_only`, so a limit and a count see the
        // same set.
        received_only: bool,
        // "Emails exchanged with X": an email matches when any of these terms
        // is in its sender (name or address), recipients or cc. Pass the
        // person's name and the addresses it resolves to (see
        // `sender_addresses_matching`).
        participants: Option<&[String]>,
    ) -> Result<Vec<Email>> {
        self.search_emails_inner(
            account_id,
            query,
            categories,
            from_filter,
            to_filter,
            subject_filter,
            after_timestamp,
            before_timestamp,
            tag_filters,
            limit,
            ascending,
            exclude_spam,
            unread_only,
            received_only,
            participants,
            SearchShape::Lookup,
        )
        .map(drop_sort_keys)
    }

    /// The app's search box: the same match as [`Self::search_emails`], listed
    /// the way a smart filter lists it — one row per thread (its newest
    /// matching email), threads sorted by their newest email of any kind (the
    /// returned timestamp), reaching every live mailbox and custom folder but
    /// never Spam or Trash, never junk, narrowed by `scope` (the view the
    /// search runs in and the search box's own operators).
    #[allow(clippy::too_many_arguments)]
    pub fn search_box_emails(
        &self,
        account_id: &str,
        query: &str,
        from_filter: Option<&str>,
        to_filter: Option<&str>,
        subject_filter: Option<&str>,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
        tag_filters: Option<&[TagQuery]>,
        scope: SearchBoxScope<'_>,
        limit: i32,
    ) -> Result<Vec<(Email, i64)>> {
        self.search_emails_inner(
            account_id,
            query,
            None,
            from_filter,
            to_filter,
            subject_filter,
            after_timestamp,
            before_timestamp,
            tag_filters,
            limit,
            false,
            true,
            false,
            false,
            None,
            SearchShape::SearchBox(scope),
        )
    }

    /// The addresses a person has written from, found by name or address
    /// fragment among the account's senders, most frequent first. Resolves
    /// "emails with Ana" to the addresses her mail comes from, so mail the
    /// user sent to those addresses — which rarely carries her name — is
    /// found too.
    pub fn sender_addresses_matching(&self, account_id: &str, needle: &str, limit: usize) -> Result<Vec<String>> {
        let needle = needle.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT lower(trim(sender_email)) AS addr FROM emails
             WHERE account_id = ?1 AND trim(sender_email) != ''
               AND (lower(sender) LIKE ?2 OR lower(sender_email) LIKE ?2)
             GROUP BY addr ORDER BY COUNT(*) DESC, addr LIMIT ?3",
        )?;
        let pattern = format!("%{needle}%");
        let rows = stmt.query_map(rusqlite::params![account_id, pattern, limit as i64], |r| {
            r.get::<_, String>(0)
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn search_emails_inner(
        &self,
        account_id: &str,
        query: &str,
        categories: Option<&[String]>,
        from_filter: Option<&str>,
        to_filter: Option<&str>,
        subject_filter: Option<&str>,
        after_timestamp: Option<i64>,
        before_timestamp: Option<i64>,
        tag_filters: Option<&[TagQuery]>,
        limit: i32,
        ascending: bool,
        exclude_spam: bool,
        unread_only: bool,
        received_only: bool,
        participants: Option<&[String]>,
        shape: SearchShape<'_>,
    ) -> Result<Vec<(Email, i64)>> {
        let participants: Vec<String> = participants
            .unwrap_or_default()
            .iter()
            .map(|p| p.trim().to_lowercase())
            .filter(|p| !p.is_empty())
            .collect();
        // ── Date-only fast path (no text filters) ────────────────────────────────
        // When there are no text-based filters (keyword, from, to, subject, tag),
        // thread deduplication is wrong: the user wants ALL emails in the window,
        // not just the latest email per thread. Example: search_emails(since=today)
        // should return every individual email received today, not one per thread.
        let has_text_filter = !query.is_empty()
            || from_filter.is_some()
            || to_filter.is_some()
            || subject_filter.is_some()
            || !participants.is_empty()
            || tag_filters.map(|t| !t.is_empty()).unwrap_or(false);

        // The search box lists threads even for a date-only query.
        if !has_text_filter && matches!(shape, SearchShape::Lookup) {
            return self
                .search_emails_by_date(
                    account_id,
                    categories,
                    after_timestamp,
                    before_timestamp,
                    limit,
                    ascending,
                    exclude_spam,
                    unread_only,
                    received_only,
                )
                .map(with_own_timestamps);
        }

        let conn = self.reader();
        let order_clause = match shape {
            SearchShape::Lookup => thread_order_clause("e", ascending),
            SearchShape::SearchBox(_) => "thread_ts DESC, e.timestamp DESC, e.id DESC".to_string(),
        };
        // Each thread is represented by one matching email: the latest one
        // newest-first, the earliest one oldest-first — otherwise a thread the
        // user started long ago and replied to yesterday sorts by yesterday.
        let thread_pick = if ascending { "MIN" } else { "MAX" };

        // ── CTE: find thread_ids that contain a matching email ────────────────────
        // All filter conditions apply to the same email row (`match_e`) so that
        // `from:alice subject:meeting` requires a single message to satisfy both.
        let mut cte_conditions: Vec<String> = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let mut param_idx = 1usize;
        // Optional pre-CTE for the from filter — populated when both email-prefix
        // and FTS-sender branches are needed so each can use its own index.
        let mut from_match_cte: Option<String> = None;

        // account_id is ?1 — used in both the CTE and outer query.
        params_vec.push(Box::new(account_id.to_string()));
        cte_conditions.push(format!("match_e.account_id = ?{}", param_idx));
        cte_conditions.push("match_e.is_deleted = 0".to_string());
        // Spam/trash never surface in search — a spam email classified
        // `primary` must not sail through the category filter.
        match shape {
            SearchShape::Lookup => cte_conditions.push("match_e.mailbox NOT IN ('spam', 'trash')".to_string()),
            SearchShape::SearchBox(_) => cte_conditions.push(Self::filterable_mailbox("match_e")),
        }
        if exclude_spam {
            cte_conditions.push(Self::junk_condition("match_e"));
        }
        // Read state rides with the other per-email conditions, so the thread
        // representative is the latest UNREAD matching email of the thread.
        if unread_only {
            cte_conditions.push("match_e.is_read = 0".to_string());
        }
        if received_only {
            cte_conditions.push("match_e.is_sent = 0".to_string());
        }
        param_idx += 1;

        if let SearchShape::SearchBox(scope) = shape {
            fn present(v: Option<&str>) -> Option<&str> {
                v.map(str::trim).filter(|v| !v.is_empty())
            }
            // The view a search-box query runs in: only that mailbox's mail.
            if let Some(mailbox) = present(scope.mailbox) {
                cte_conditions.push(format!("match_e.mailbox = ?{param_idx}"));
                params_vec.push(Box::new(mailbox.to_string()));
                param_idx += 1;
            }
            // Same match as the sidebar's domain filter (`get_filtered_emails`).
            if let Some(domain) = present(scope.domain) {
                cte_conditions.push(format!("match_e.sender_domain = ?{param_idx}"));
                params_vec.push(Box::new(domain.to_lowercase()));
                param_idx += 1;
            }
            // Same match as the sidebar's attachment filter.
            if let Some(ext) = present(scope.attachment_ext) {
                cte_conditions.push(format!(
                    "EXISTS (SELECT 1 FROM email_attachment_meta am WHERE am.email_id = match_e.id AND LOWER(am.filename) LIKE ?{param_idx})"
                ));
                params_vec.push(Box::new(format!("%.{}", ext.to_lowercase())));
                param_idx += 1;
            }
        }

        // Category filter
        if let Some(cats) = categories.filter(|c| !c.is_empty()) {
            let placeholders: Vec<String> = (0..cats.len()).map(|i| format!("?{}", param_idx + i)).collect();
            cte_conditions.push(format!("match_e.category IN ({})", placeholders.join(", ")));
            for cat in cats {
                params_vec.push(Box::new(cat.clone()));
            }
            param_idx += cats.len();
        }

        // Keyword search via FTS5 (already indexed — no change needed here)
        if !query.is_empty() {
            let fts_query = sanitize_fts_query(query);
            // A keyword of only symbols ("?!") can match nothing; dropping
            // the condition instead would match every thread.
            if fts_query.is_empty() {
                return Ok(Vec::new());
            }
            cte_conditions.push(format!(
                "match_e.id IN (SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?{})",
                param_idx
            ));
            params_vec.push(Box::new(fts_query));
            param_idx += 1;
        }

        // From filter — two-pronged:
        //   1. Prefix match on `sender_email` (uses idx_emails_sender_email_nocase).
        //      A case-folded range scan so mixed-case stored addresses still match
        //      the lowercased needle (the porter/unicode FTS branch is already
        //      case-insensitive; this keeps the address branch consistent).
        //   2. FTS5 sender-column search for display-name matches (e.g. "from:Alice").
        //
        // When both branches are needed we materialise a separate `from_match` CTE
        // with UNION so each branch can use its own index independently.  The old
        // approach (OR in a single WHERE clause) prevented SQLite from using either
        // index, causing a full table scan on every from: query.
        if let Some(from) = from_filter {
            let from_lower = from.to_lowercase();
            // Build an FTS5 sender-column query: "sender:word1* sender:word2* ..."
            let fts_sender: String = from
                .split(|c: char| !c.is_alphanumeric())
                .filter(|t| t.len() >= 2)
                .map(|t| format!("sender:\"{}\"*", t))
                .collect::<Vec<_>>()
                .join(" ");

            // Convert prefix to explicit >= / < range bounds so SQLite can use
            // idx_emails_sender_email as a B-tree range scan.  Parameterised
            // LIKE ? prevents the optimiser from knowing there is no leading
            // wildcard, so it falls back to a full table scan.
            let upper_bound = prefix_upper_bound(&from_lower);

            if fts_sender.is_empty() {
                // Very short or symbol-only input — range scan only.
                // COLLATE NOCASE so a mixed-case stored address still falls in
                // range against the lowercased needle (idx_emails_sender_email_nocase).
                if let Some(ref ub) = upper_bound {
                    cte_conditions.push(format!(
                        "(match_e.sender_email >= ?{lo} COLLATE NOCASE AND match_e.sender_email < ?{hi} COLLATE NOCASE)",
                        lo = param_idx,
                        hi = param_idx + 1,
                    ));
                    params_vec.push(Box::new(from_lower.clone()));
                    params_vec.push(Box::new(ub.clone()));
                    param_idx += 2;
                } else {
                    cte_conditions.push(format!("match_e.sender_email >= ?{} COLLATE NOCASE", param_idx));
                    params_vec.push(Box::new(from_lower.clone()));
                    param_idx += 1;
                }
            } else {
                // UNION CTE: each branch uses its own index independently.
                //   Branch 1 (>= / <) → idx_emails_sender_email_nocase B-tree range scan.
                //   Branch 2 (MATCH)   → emails_fts inverted index (display-name hits).
                //
                // is_deleted is intentionally omitted — filter_match enforces it.
                // COLLATE NOCASE on both bounds so a mixed-case stored address
                // still falls in range against the lowercased needle; the
                // idx_emails_sender_email_nocase index serves the case-folded scan.
                let range_clause = if let Some(ref _ub) = upper_bound {
                    format!(
                        "sender_email >= ?{lo} COLLATE NOCASE AND sender_email < ?{hi} COLLATE NOCASE",
                        lo = param_idx,
                        hi = param_idx + 1,
                    )
                } else {
                    format!("sender_email >= ?{} COLLATE NOCASE", param_idx)
                };
                let range_params = if upper_bound.is_some() { 2 } else { 1 };

                // Branch 3 (relaxed, multi-token needles only): every
                // meaningful token must appear SOMEWHERE in "display name +
                // address". Neither indexed branch can span the two fields —
                // the FTS `sender` column holds only the display name, and the
                // address branch is a prefix scan pinned to the local part — so
                // a needle like "nadia de northwind" (a person AND their
                // company) matched nothing at all. This branch is a scan, which
                // is why it is gated on a multi-token needle: single-token
                // lookups (the overwhelming majority) keep the pure index path.
                let relaxed_tokens = relaxed_sender_tokens(from);
                let relaxed_branch = if relaxed_tokens.is_empty() {
                    String::new()
                } else {
                    let first = param_idx + range_params + 1;
                    let conds: Vec<String> = (0..relaxed_tokens.len())
                        .map(|i| format!("haystack LIKE ?{}", first + i))
                        .collect();
                    format!(
                        "
                         UNION
                         SELECT email_id FROM (
                             SELECT id AS email_id,
                                    lower(sender || ' ' || sender_email) AS haystack
                             FROM emails
                             WHERE account_id = ?1
                         ) WHERE {}",
                        conds.join(" AND "),
                    )
                };

                // Branch 4 (company / domain needles): the user names the
                // COMPANY, not the person — "northwind", "de northwind". The domain
                // is in no display name, and it is not a prefix of the address
                // (that starts with the local part), so branches 1-2 find only
                // the few senders who spell the company into their display
                // name. Prefix-matched against the indexed `sender_domain`
                // column, so this is a B-tree range scan, not a table scan.
                let domain_needle = sender_domain_needle(from);
                let domain_branch = match domain_needle {
                    None => String::new(),
                    Some(ref d) => {
                        let lo = param_idx + range_params + 1 + relaxed_tokens.len();
                        match prefix_upper_bound(d) {
                            Some(_) => format!(
                                "
                         UNION
                         SELECT id AS email_id
                         FROM emails INDEXED BY idx_emails_sender_domain
                         WHERE account_id = ?1
                           AND sender_domain >= ?{lo} AND sender_domain < ?{hi}",
                                lo = lo,
                                hi = lo + 1,
                            ),
                            None => format!(
                                "
                         UNION
                         SELECT id AS email_id
                         FROM emails INDEXED BY idx_emails_sender_domain
                         WHERE account_id = ?1
                           AND sender_domain >= ?{lo}",
                                lo = lo,
                            ),
                        }
                    }
                };

                from_match_cte = Some(format!(
                    "from_match AS (
                         SELECT id AS email_id
                         FROM emails INDEXED BY idx_emails_sender_email_nocase
                         WHERE account_id = ?1
                           AND {range}
                         UNION
                         SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?{fts_idx}{relaxed}{domain}
                     )",
                    range = range_clause,
                    fts_idx = param_idx + range_params,
                    relaxed = relaxed_branch,
                    domain = domain_branch,
                ));
                params_vec.push(Box::new(from_lower));
                if let Some(ub) = upper_bound {
                    params_vec.push(Box::new(ub));
                }
                params_vec.push(Box::new(fts_sender));
                param_idx += range_params + 1;
                for tok in &relaxed_tokens {
                    params_vec.push(Box::new(format!("%{}%", tok)));
                    param_idx += 1;
                }
                if let Some(d) = domain_needle {
                    let ub = prefix_upper_bound(&d);
                    params_vec.push(Box::new(d));
                    param_idx += 1;
                    if let Some(ub) = ub {
                        params_vec.push(Box::new(ub));
                        param_idx += 1;
                    }
                }
                // The JOIN into from_match is handled in query assembly below.
            }
        }

        // To filter — recipients are stored as a JSON array; LIKE is unavoidable
        // without a separate recipients table.  Run it in the CTE (once) rather than
        // inside a correlated subquery (once per representative email).
        if let Some(to) = to_filter {
            let to_pattern = format!("%{}%", to);
            cte_conditions.push(format!("match_e.recipients_json LIKE ?{}", param_idx));
            params_vec.push(Box::new(to_pattern));
            param_idx += 1;
        }

        // Participants: any term in the sender (name or address), recipients
        // or cc. JSON arrays again, so LIKE — same cost as the `to` filter.
        if !participants.is_empty() {
            let mut any_term = Vec::with_capacity(participants.len());
            for term in &participants {
                any_term.push(format!(
                    "(lower(match_e.sender) LIKE ?{i} OR lower(match_e.sender_email) LIKE ?{i} \
                     OR lower(match_e.recipients_json) LIKE ?{i} OR lower(match_e.cc_json) LIKE ?{i})",
                    i = param_idx
                ));
                params_vec.push(Box::new(format!("%{term}%")));
                param_idx += 1;
            }
            cte_conditions.push(format!("({})", any_term.join(" OR ")));
        }

        // Subject filter — route through FTS5 subject column instead of LIKE '%…%'.
        // FTS5 `subject:{term}*` uses the inverted index on the subject field.
        if let Some(subj) = subject_filter {
            let fts_subject: String = subj
                .split(|c: char| !c.is_alphanumeric())
                .filter(|t| t.len() >= 2)
                .map(|t| format!("subject:\"{}\"*", t))
                .collect::<Vec<_>>()
                .join(" ");

            if fts_subject.is_empty() {
                // Fallback for very short subjects
                let subj_pattern = format!("%{}%", subj);
                cte_conditions.push(format!("match_e.subject LIKE ?{}", param_idx));
                params_vec.push(Box::new(subj_pattern));
                param_idx += 1;
            } else {
                cte_conditions.push(format!(
                    "match_e.id IN (SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?{})",
                    param_idx
                ));
                params_vec.push(Box::new(fts_subject));
                param_idx += 1;
            }
        }

        // Date range filters
        if let Some(after) = after_timestamp {
            cte_conditions.push(format!("match_e.timestamp >= ?{}", param_idx));
            params_vec.push(Box::new(after));
            param_idx += 1;
        }
        if let Some(before) = before_timestamp {
            cte_conditions.push(format!("match_e.timestamp <= ?{}", param_idx));
            params_vec.push(Box::new(before));
            param_idx += 1;
        }

        // Tag filters. A typed filter also binds `tag_type`, which keeps
        // `intent=billing` from matching the company tag "billing" and lets
        // SQLite use idx_email_tags_type_value instead of scanning by value.
        if let Some(tags) = tag_filters.filter(|t| !t.is_empty()) {
            for tag in tags {
                let type_clause = match &tag.tag_type {
                    Some(_) => format!(" AND et.tag_type = ?{}", param_idx + 1),
                    None => String::new(),
                };
                cte_conditions.push(format!(
                    "EXISTS (SELECT 1 FROM email_tags et WHERE et.email_id = match_e.id AND et.tag_value = ?{}{})",
                    param_idx, type_clause
                ));
                params_vec.push(Box::new(tag.value.clone()));
                param_idx += 1;
                if let Some(tag_type) = &tag.tag_type {
                    params_vec.push(Box::new(tag_type.clone()));
                    param_idx += 1;
                }
            }
        }

        // ── Assemble and execute ─────────────────────────────────────────────────
        let cte_where = cte_conditions.join(" AND ");

        // GROUP BY + MAX(timestamp) is ~250x faster than the scalar subquery
        // for finding the latest email per thread (benchmarked on 47k emails).
        // `filter_match` emits id/thread_id/timestamp for matching emails,
        // so the dedup groups by thread over ONLY the matching rows — not all
        // emails in those threads, and the representative id always comes
        // from `filter_match`. See the regression test
        // `search_emails_from_filter_returns_matching_email_not_reply`.
        //
        // With a `from_match` CTE, `filter_match` drives from it (CROSS JOIN
        // pins the order): the sender index + FTS produce the candidate ids,
        // and each is a PK lookup that must pass every other filter. The ids
        // stay inside SQL — binding them one parameter each broke past
        // SQLite's 32,766-variable limit on a prolific sender.
        let (from_cte, filter_source) = match &from_match_cte {
            Some(cte) => (
                format!("{cte},"),
                "from_match fm CROSS JOIN emails match_e ON match_e.id = fm.email_id",
            ),
            None => (String::new(), "emails match_e"),
        };
        let sql = format!(
            "WITH {from_cte}
             filter_match AS (
                 SELECT match_e.id, match_e.thread_id, match_e.timestamp
                 FROM {filter_source}
                 WHERE {cte_where}
             ),
             {dedup}
             ORDER BY {order}
             LIMIT ?{limit_idx}",
            dedup = thread_representative_sql(thread_pick, &sort_key_sql(shape)),
            order = order_clause,
            limit_idx = param_idx,
        );

        params_vec.push(Box::new(limit));

        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let sort_key_idx = EMAIL_COLUMNS.split(',').count();
        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            Ok((row_to_email(row)?, row.get::<_, i64>(sort_key_idx)?))
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }

        Ok(result)
    }
}

/// Thread dedup tail for `search_emails_inner`, run over a preceding
/// `filter_match (id, thread_id, timestamp)` CTE: one representative per
/// thread, picked with `thread_pick` (`MAX` newest-first, `MIN` oldest-first)
/// on the timestamp and then on the id. The id tie-break keeps two matching
/// emails stamped in the same second from both coming back (matching the
/// inbox's `timestamp DESC, id DESC` order); the final lookup is by primary key.
fn thread_representative_sql(thread_pick: &str, sort_key: &str) -> String {
    format!(
        "thread_latest AS (
             SELECT thread_id AS tid, {thread_pick}(timestamp) AS max_ts
             FROM filter_match
             GROUP BY thread_id
         ),
         thread_rep AS (
             SELECT {thread_pick}(fm.id) AS rep_id
             FROM filter_match fm
             JOIN thread_latest tl ON fm.thread_id = tl.tid AND fm.timestamp = tl.max_ts
             GROUP BY fm.thread_id
         )
         SELECT {cols}, {sort_key} AS thread_ts
         FROM thread_rep r
         CROSS JOIN emails e
         WHERE e.id = r.rep_id",
        cols = EMAIL_COLUMNS
    )
}

/// How `search_emails_inner` lists its matches.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SearchShape<'a> {
    /// Programmatic lookups (chat tools, research, agent search): rows sorted
    /// by the matching email; the caller's own spam/category choices apply.
    Lookup,
    /// The app's search box: listed like a smart filter (see
    /// [`Database::search_box_emails`]).
    SearchBox(SearchBoxScope<'a>),
}

/// What only the search box narrows by: the view it runs in and the
/// operators the sender-domain and attachment smart filters write.
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchBoxScope<'a> {
    /// `inbox`, `sent`, `archive` or `folder:<path>`; `None` = every live mailbox.
    pub mailbox: Option<&'a str>,
    /// `domain:` — the sender's domain, exactly.
    pub domain: Option<&'a str>,
    /// `ext:` — an attachment file extension, without the dot.
    pub attachment_ext: Option<&'a str>,
}

/// The column a shape sorts threads by: the matching email itself, or — in
/// the search box — the thread's newest email of any kind, one indexed seek
/// per thread (see `representative_tail` for why the hint is load-bearing).
fn sort_key_sql(shape: SearchShape<'_>) -> String {
    match shape {
        SearchShape::Lookup => "e.timestamp".to_string(),
        SearchShape::SearchBox(_) => format!(
            "(SELECT e3.timestamp
              FROM emails e3 INDEXED BY idx_emails_thread_latest
              WHERE e3.account_id = e.account_id AND e3.thread_id = e.thread_id
                AND e3.is_deleted = 0 AND {filterable}
              ORDER BY e3.timestamp DESC, e3.id DESC
              LIMIT 1)",
            filterable = Database::filterable_mailbox("e3"),
        ),
    }
}

fn drop_sort_keys(rows: Vec<(Email, i64)>) -> Vec<Email> {
    rows.into_iter().map(|(email, _)| email).collect()
}

fn with_own_timestamps(emails: Vec<Email>) -> Vec<(Email, i64)> {
    emails
        .into_iter()
        .map(|email| {
            let ts = email.timestamp;
            (email, ts)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use super::super::*;
    use crate::db::Database;

    // Regression for user-confirmed semantics: a tag filter (e.g. company "Globex")
    // must match any thread where AT LEAST ONE email carries the tag, even when the
    // user has replied and their sent message is now the thread's latest email.
    // The row returned for each matching thread is its newest MATCHING email.
    #[test]
    fn tag_filter_matches_thread_if_any_email_tagged_and_returns_newest_match() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        // Thread A: E1 (old, urgent) + E2 (newer, no urgent tag — represents user reply).
        // The thread matches because E1 is urgent; the row returned is E1.
        insert_email(&db, "e1", account, "thread-a", 100);
        insert_email(&db, "e2", account, "thread-a", 200);
        tag_email(&db, "e1", "priority", "urgent");
        tag_email(&db, "e2", "priority", "normal");

        // Thread B: E3 (only email, urgent). Should appear.
        insert_email(&db, "e3", account, "thread-b", 300);
        tag_email(&db, "e3", "priority", "urgent");

        // Thread C: E4 (only email, no urgent). Should NOT appear.
        insert_email(&db, "e4", account, "thread-c", 400);

        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::Account(account),
                None,
                None,
                Some("priority"),
                Some("urgent"),
                None,
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();

        let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();

        // Thread B is fully represented by its only (urgent) email.
        assert!(
            ids.contains(&"e3"),
            "e3 (urgent thread B) should appear, got: {:?}",
            ids
        );

        // Thread A matches because E1 is urgent, and E1 is what matched: the
        // row is E1, not the newer `normal` reply.
        assert!(
            ids.contains(&"e1"),
            "e1 (urgent match in thread A) should appear, got: {:?}",
            ids
        );
        assert!(!ids.contains(&"e2"), "one row per thread, the match: {:?}", ids);
        assert!(
            !ids.contains(&"e4"),
            "e4 (non-urgent thread C) must not appear, got: {:?}",
            ids
        );
        assert_eq!(ids.len(), 2);
    }

    // ── search: spam/trash mailbox exclusion ─────────────────────────────────────

    // Regression: chat's search_emails tool (and the app search bar) surfaced
    // emails sitting in the spam mailbox because search only filtered by
    // category — a spam email classified `primary` sailed through. Search must
    // never return spam/trash rows, matching get_emails / quick-filter stats.
    #[test]
    fn search_keyword_excludes_spam_and_trash_mailboxes() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email_in_mailbox(
            &db,
            "e-in",
            account,
            "t-in",
            "Alice",
            "alice@good.com",
            "Alignerr opportunities",
            "body",
            100,
            "inbox",
        );
        insert_search_email_in_mailbox(
            &db,
            "e-spam",
            account,
            "t-spam",
            "Bot",
            "bot@bad.com",
            "Alignerr opportunities",
            "body",
            200,
            "spam",
        );
        insert_search_email_in_mailbox(
            &db,
            "e-trash",
            account,
            "t-trash",
            "Bot",
            "bot@bad.com",
            "Alignerr opportunities",
            "body",
            300,
            "trash",
        );

        let results = db
            .search_emails(account, "Alignerr", None, None, None, None, None, None, None, 20)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(ids.contains(&"e-in"), "inbox email must match, got {:?}", ids);
        assert!(!ids.contains(&"e-spam"), "spam email must be excluded, got {:?}", ids);
        assert!(!ids.contains(&"e-trash"), "trash email must be excluded, got {:?}", ids);
    }

    // Same leak on the date-only fast path (no text filters) — the exact path
    // the "summarise this week" chat shortcut takes (since/until only).
    #[test]
    fn search_date_only_excludes_spam_and_trash_mailboxes() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_contact_email(
            &db,
            "e-in",
            account,
            "t-in",
            "Alice",
            "alice@good.com",
            "[]",
            "inbox",
            100,
        );
        insert_contact_email(&db, "e-sent", account, "t-sent", "Me", "me@me.com", "[]", "sent", 150);
        insert_contact_email(
            &db,
            "e-spam",
            account,
            "t-spam",
            "Bot",
            "bot@bad.com",
            "[]",
            "spam",
            200,
        );
        insert_contact_email(
            &db,
            "e-trash",
            account,
            "t-trash",
            "Bot",
            "bot@bad.com",
            "[]",
            "trash",
            300,
        );

        let results = db
            .search_emails(account, "", None, None, None, None, Some(50), Some(500), None, 20)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(ids.contains(&"e-in"), "inbox email must appear, got {:?}", ids);
        assert!(ids.contains(&"e-sent"), "sent email must appear, got {:?}", ids);
        assert!(!ids.contains(&"e-spam"), "spam email must be excluded, got {:?}", ids);
        assert!(!ids.contains(&"e-trash"), "trash email must be excluded, got {:?}", ids);
    }

    // ── quick filter stats: mailbox scoping + count semantics ────────────────────

    // Suggestion stats must only count inbox/sent mail, matching what
    // get_filtered_emails shows when the user clicks the suggestion. Spam and
    // trash used to inflate counts and could push a spam domain into the top 10.
    #[test]
    fn quick_filter_stats_excludes_spam_and_trash_mailboxes() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_account(&db, account, "me@mymail.com");

        insert_contact_email(&db, "g1", account, "t-g1", "Gina", "gina@good.com", "[]", "inbox", 100);
        // Three spam/trash emails from spam.com — would outrank good.com if counted.
        insert_contact_email(&db, "s1", account, "t-s1", "Bot", "bot@spam.com", "[]", "spam", 200);
        insert_contact_email(&db, "s2", account, "t-s2", "Bot", "bot@spam.com", "[]", "spam", 300);
        insert_contact_email(&db, "s3", account, "t-s3", "Bot", "bot@spam.com", "[]", "trash", 400);

        let stats = db
            .get_quick_filter_stats(crate::db::AccountScope::Account(account), &[], &[])
            .unwrap();

        let domains: Vec<&str> = stats.top_domains.iter().map(|d| d.value.as_str()).collect();
        assert!(
            domains.contains(&"good.com"),
            "inbox mail must be counted, got {:?}",
            domains
        );
        assert!(
            !domains.contains(&"spam.com"),
            "spam/trash mail must not produce domain suggestions, got {:?}",
            domains
        );

        let senders: Vec<&str> = stats.top_senders.iter().map(|s| s.value.as_str()).collect();
        assert!(
            !senders.contains(&"bot@spam.com"),
            "spam/trash mail must not produce sender suggestions, got {:?}",
            senders
        );
    }

    // The account owner's own address dominates sender stats via sent mail —
    // suggesting "filter by yourself" is noise. It must be excluded, in any case
    // variant of the configured account email.
    #[test]
    fn quick_filter_stats_excludes_account_owner_address() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_account(&db, account, "me@mymail.com");

        insert_contact_email(
            &db,
            "s1",
            account,
            "t1",
            "Me",
            "me@mymail.com",
            "[\"a@b.com\"]",
            "sent",
            100,
        );
        insert_contact_email(
            &db,
            "s2",
            account,
            "t2",
            "Me",
            "Me@MyMail.com",
            "[\"a@b.com\"]",
            "sent",
            200,
        );
        insert_contact_email(&db, "i1", account, "t3", "Alice", "alice@ex.com", "[]", "inbox", 300);

        let stats = db
            .get_quick_filter_stats(crate::db::AccountScope::Account(account), &[], &[])
            .unwrap();
        let senders: Vec<String> = stats.top_senders.iter().map(|s| s.value.to_lowercase()).collect();

        assert!(
            !senders.contains(&"me@mymail.com".to_string()),
            "own address must not be suggested as a sender filter, got {:?}",
            senders
        );
        assert!(
            senders.contains(&"alice@ex.com".to_string()),
            "other senders must still be suggested, got {:?}",
            senders
        );
    }

    // Sidebar counts must match what clicking the filter shows: one row per
    // thread. Domain/sender stats used to count emails while tag stats counted
    // threads and the filtered list shows thread representatives.
    #[test]
    fn quick_filter_stats_counts_threads_not_emails() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_account(&db, account, "me@mymail.com");

        // Three emails in one thread + one email in another, all bob@acme.com.
        insert_contact_email(&db, "e1", account, "t-a", "Bob", "bob@acme.com", "[]", "inbox", 100);
        insert_contact_email(&db, "e2", account, "t-a", "Bob", "bob@acme.com", "[]", "inbox", 200);
        insert_contact_email(&db, "e3", account, "t-a", "Bob", "bob@acme.com", "[]", "inbox", 300);
        insert_contact_email(&db, "e4", account, "t-b", "Bob", "bob@acme.com", "[]", "inbox", 400);

        let stats = db
            .get_quick_filter_stats(crate::db::AccountScope::Account(account), &[], &[])
            .unwrap();

        let acme = stats
            .top_domains
            .iter()
            .find(|d| d.value == "acme.com")
            .expect("acme.com must be suggested");
        assert_eq!(acme.count, 2, "domain count must be threads (2), not emails (4)");

        let bob = stats
            .top_senders
            .iter()
            .find(|s| s.value == "bob@acme.com")
            .expect("bob@acme.com must be suggested");
        assert_eq!(bob.count, 2, "sender count must be threads (2), not emails (4)");
    }

    // ── quick filter stats: sender case-insensitivity ────────────────────────────

    // Providers vary the case of the same address across messages. BINARY
    // grouping split one sender into two suggestions with divided counts.
    #[test]
    fn quick_filter_stats_merges_sender_case_variants() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_account(&db, account, "me@mymail.com");

        insert_contact_email(&db, "e1", account, "t1", "Alice", "Alice@Ex.com", "[]", "inbox", 100);
        insert_contact_email(&db, "e2", account, "t2", "Alice", "alice@ex.com", "[]", "inbox", 200);

        let stats = db
            .get_quick_filter_stats(crate::db::AccountScope::Account(account), &[], &[])
            .unwrap();
        let alice_entries: Vec<_> = stats
            .top_senders
            .iter()
            .filter(|s| s.value.eq_ignore_ascii_case("alice@ex.com"))
            .collect();

        assert_eq!(
            alice_entries.len(),
            1,
            "case variants of the same address must merge into one suggestion, got {:?}",
            stats.top_senders
        );
        assert_eq!(alice_entries[0].count, 2, "merged suggestion must count both threads");
    }

    // Blocking/removing a sender must suppress it regardless of the stored casing.
    #[test]
    fn quick_filter_stats_excluded_senders_match_case_insensitively() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_account(&db, account, "me@mymail.com");

        insert_contact_email(&db, "e1", account, "t1", "Alice", "Alice@Ex.com", "[]", "inbox", 100);

        let stats = db
            .get_quick_filter_stats(
                crate::db::AccountScope::Account(account),
                &[],
                &["alice@ex.com".to_string()],
            )
            .unwrap();
        let senders: Vec<&str> = stats.top_senders.iter().map(|s| s.value.as_str()).collect();

        assert!(
            !senders.iter().any(|s| s.eq_ignore_ascii_case("alice@ex.com")),
            "removed sender must be excluded regardless of case, got {:?}",
            senders
        );
    }

    // Clicking a sender suggestion (or blocking from an email whose header casing
    // differs from the stored rows) must still match the stored address.
    #[test]
    fn filtered_emails_sender_filter_is_case_insensitive() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_account(&db, account, "me@mymail.com");

        insert_contact_email(
            &db,
            "e1",
            account,
            "t1",
            "Billing",
            "EMEA_Billing@Ex.com",
            "[]",
            "inbox",
            100,
        );

        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::Account(account),
                None,
                Some("emea_billing@ex.com"),
                None,
                None,
                None,
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();

        assert_eq!(
            result.emails.len(),
            1,
            "sender filter must match the stored mixed-case address"
        );
    }

    // ── AllEnabled (unified) scope ───────────────────────────────────────────────

    fn set_enabled(db: &Database, account: &str, enabled: bool) {
        db.connection()
            .execute(
                "UPDATE accounts SET enabled = ?2 WHERE id = ?1",
                rusqlite::params![account, enabled as i32],
            )
            .unwrap();
    }

    #[test]
    fn filtered_emails_all_enabled_sender_filter_spans_accounts() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me1@my.com");
        insert_account(&db, "acc2", "me2@my.com");
        insert_account(&db, "acc3", "me3@my.com");

        insert_contact_email(&db, "e1", "acc1", "t1", "Bob", "bob@acme.com", "[]", "inbox", 100);
        insert_contact_email(&db, "e2", "acc2", "t2", "Bob", "bob@acme.com", "[]", "inbox", 200);
        // Disabled account must not contribute rows.
        insert_contact_email(&db, "e3", "acc3", "t3", "Bob", "bob@acme.com", "[]", "inbox", 300);
        set_enabled(&db, "acc3", false);

        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::AllEnabled,
                None,
                Some("bob@acme.com"),
                None,
                None,
                None,
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();
        let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["e2", "e1"], "both enabled accounts' threads, newest first");
    }

    #[test]
    fn filtered_emails_all_enabled_thread_collision_no_cross_account_merge() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me1@my.com");
        insert_account(&db, "acc2", "me2@my.com");

        // Same thread_id string in both accounts; both match the domain filter.
        // Each account must keep its OWN latest row — the newer acc2 email must
        // not swallow acc1's thread.
        insert_contact_email(&db, "e1a", "acc1", "shared", "Bob", "bob@acme.com", "[]", "inbox", 100);
        insert_contact_email(&db, "e1b", "acc1", "shared", "Bob", "bob@acme.com", "[]", "inbox", 150);
        insert_contact_email(&db, "e2a", "acc2", "shared", "Ann", "ann@acme.com", "[]", "inbox", 200);

        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::AllEnabled,
                Some("acme.com"),
                None,
                None,
                None,
                None,
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();
        let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["e2a", "e1b"],
            "one representative per (account, thread) — not per thread_id string"
        );
    }

    #[test]
    fn filtered_emails_all_enabled_tag_filter_spans_accounts() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me1@my.com");
        insert_account(&db, "acc2", "me2@my.com");

        insert_email(&db, "e1", "acc1", "t1", 100);
        insert_email(&db, "e2", "acc2", "t2", 200);
        insert_email(&db, "e3", "acc2", "t3", 300);
        tag_email(&db, "e1", "company", "Acme");
        tag_email(&db, "e2", "company", "Acme");
        tag_email(&db, "e3", "company", "Globex");

        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::AllEnabled,
                None,
                None,
                Some("company"),
                Some("Acme"),
                None,
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();
        let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["e2", "e1"], "Acme threads from both accounts, newest first");
    }

    #[test]
    fn quick_filter_stats_all_enabled_excludes_every_own_address_and_counts_pairs() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me1@my.com");
        insert_account(&db, "acc2", "me2@my.com");

        // Each account's own sent mail — neither owner address may be suggested.
        insert_contact_email(&db, "s1", "acc1", "ts1", "Me1", "me1@my.com", "[]", "sent", 100);
        insert_contact_email(&db, "s2", "acc2", "ts2", "Me2", "Me2@My.com", "[]", "sent", 200);
        // Same external sender in both accounts, same thread_id string —
        // 2 (account, thread) pairs.
        insert_contact_email(&db, "e1", "acc1", "shared", "Bob", "bob@acme.com", "[]", "inbox", 300);
        insert_contact_email(&db, "e2", "acc2", "shared", "Bob", "bob@acme.com", "[]", "inbox", 400);

        let stats = db
            .get_quick_filter_stats(crate::db::AccountScope::AllEnabled, &[], &[])
            .unwrap();

        let senders: Vec<String> = stats.top_senders.iter().map(|s| s.value.to_lowercase()).collect();
        assert!(
            !senders.contains(&"me1@my.com".to_string()) && !senders.contains(&"me2@my.com".to_string()),
            "every enabled account's own address must be excluded, got {:?}",
            senders
        );

        let bob = stats
            .top_senders
            .iter()
            .find(|s| s.value.eq_ignore_ascii_case("bob@acme.com"))
            .expect("bob must be suggested");
        assert_eq!(bob.count, 2, "counts (account, thread) pairs, not thread_id strings");

        let acme = stats
            .top_domains
            .iter()
            .find(|d| d.value == "acme.com")
            .expect("acme.com must be suggested");
        assert_eq!(acme.count, 2, "domain counts (account, thread) pairs too");
    }

    // ── from: filter correctness ──────────────────────────────────────────────────

    #[test]
    fn search_from_exact_email_finds_thread() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "thread-alice",
            "Alice Smith",
            "alice@example.com",
            "Hello",
            "body text",
            100,
        );
        insert_search_email(
            &db,
            "e2",
            account,
            "thread-bob",
            "Bob Jones",
            "bob@other.com",
            "Hi",
            "body text",
            200,
        );

        let results = db
            .search_emails(
                account,
                "",
                None,
                Some("alice@example.com"),
                None,
                None,
                None,
                None,
                None,
                50,
            )
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(
            ids.contains(&"e1"),
            "exact email must find Alice's thread, got: {:?}",
            ids
        );
        assert!(!ids.contains(&"e2"), "Bob must not appear, got: {:?}", ids);
    }

    // Regression: a `from:` search must match the sender address regardless of
    // case. Providers can send mixed-case local parts (e.g. the user-reported
    // "EMEA_Invoicing@email.apple.com"). The filter lowercases the needle, so the
    // case-sensitive (BINARY) range scan over the stored mixed-case sender_email
    // returned 0 results — `from:EMEA_Invoicing@email.apple.com` found nothing.
    #[test]
    fn search_from_filter_address_is_case_insensitive() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "thread-apple",
            "EMEA Invoicing",
            "EMEA_Invoicing@email.apple.com",
            "Your invoice",
            "see attached",
            100,
        );

        // Needle differs only in case from the stored address.
        let results = db
            .search_emails(
                account,
                "",
                None,
                Some("emea_invoicing@email.apple.com"),
                None,
                None,
                None,
                None,
                None,
                50,
            )
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(
            ids.contains(&"e1"),
            "case-insensitive from: must find the mixed-case sender, got: {:?}",
            ids
        );
    }

    // Regression: a `from:` filter combined with a residual keyword
    // (e.g. `from:alice@example.com presupuesto`) must intersect both — return
    // only the sender's emails that ALSO match the keyword. The from_match fast
    // path used to re-apply only category/date filters in its Step 2 PK lookup,
    // silently dropping the keyword (and to/subject/tag) filters, so every email
    // from the sender came back regardless of the keyword.
    #[test]
    fn search_from_filter_plus_keyword_intersects_both() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        // Same sender, two threads — only one mentions the keyword.
        insert_search_email(
            &db,
            "e1",
            account,
            "thread-budget",
            "Alice Smith",
            "alice@example.com",
            "Re: presupuesto Q3",
            "adjunto el presupuesto revisado",
            100,
        );
        insert_search_email(
            &db,
            "e2",
            account,
            "thread-lunch",
            "Alice Smith",
            "alice@example.com",
            "Lunch tomorrow?",
            "want to grab lunch",
            200,
        );
        // Different sender that DOES mention the keyword — must not match the from filter.
        insert_search_email(
            &db,
            "e3",
            account,
            "thread-other",
            "Bob Jones",
            "bob@other.com",
            "presupuesto draft",
            "here is the presupuesto",
            300,
        );

        let results = db
            .search_emails(
                account,
                "presupuesto",
                None,
                Some("alice@example.com"),
                None,
                None,
                None,
                None,
                None,
                50,
            )
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(
            ids.contains(&"e1"),
            "from:alice + 'presupuesto' must return her budget email, got: {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e2"),
            "from:alice + 'presupuesto' must NOT return her unrelated lunch email, got: {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e3"),
            "from:alice + 'presupuesto' must NOT return Bob's email, got: {:?}",
            ids
        );
        assert_eq!(
            results.len(),
            1,
            "exactly one email matches both filters, got: {:?}",
            ids
        );
    }

    // Regression: a `from:` needle that spans the display NAME and the address
    // DOMAIN ("nadia de northwind" — Spanish "Nadia from Northwind") matched
    // nothing. The FTS branch ANDs every token against the `sender` column,
    // which only holds the display name, so the domain token could never hit;
    // the address branch is a PREFIX scan, so the name token pinned it to the
    // local part. Neither branch can span both fields, and connector words
    // ("de", "from") are in no field at all.
    #[test]
    fn search_from_filter_spans_display_name_and_address_domain() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "thread-nadia",
            "Nadia Brookes",
            "nadia.brookes@northwind.example",
            "Contract review",
            "attached the revised draft",
            100,
        );
        // Same first name, different company — must NOT match.
        insert_search_email(
            &db,
            "e2",
            account,
            "thread-other",
            "Nadia Ferrer",
            "nadia.ferrer@seabright.example",
            "Lunch tomorrow?",
            "want to grab lunch",
            200,
        );

        let results = db
            .search_emails(
                account,
                "",
                None,
                Some("nadia de northwind"),
                None,
                None,
                None,
                None,
                None,
                50,
            )
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(
            ids.contains(&"e1"),
            "name + domain needle must find the sender at that domain, got: {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e2"),
            "the same first name at another domain must not match, got: {:?}",
            ids
        );
    }

    // The relaxed multi-token branch must still be an AND across the meaningful
    // tokens — it broadens WHERE each token may appear (display name or
    // address), never WHICH senders qualify.
    #[test]
    fn search_from_filter_multi_token_requires_every_token() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "thread-nadia",
            "Nadia Brookes",
            "nadia.brookes@northwind.example",
            "Contract review",
            "attached the revised draft",
            100,
        );

        let results = db
            .search_emails(
                account,
                "",
                None,
                Some("nadia seabright"),
                None,
                None,
                None,
                None,
                None,
                50,
            )
            .unwrap();

        assert!(
            results.is_empty(),
            "a token matching no field must exclude the sender, got: {:?}",
            results.iter().map(|e| e.id.as_str()).collect::<Vec<_>>()
        );
    }

    // A multi-word DISPLAY NAME needle must keep working — the relaxed branch is
    // additive, never a replacement for the existing FTS name match.
    #[test]
    fn search_from_filter_multi_token_display_name_still_matches() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "thread-nadia",
            "Nadia Brookes",
            "nadia.brookes@northwind.example",
            "Contract review",
            "attached the revised draft",
            100,
        );

        let results = db
            .search_emails(
                account,
                "",
                None,
                Some("Nadia Brookes"),
                None,
                None,
                None,
                None,
                None,
                50,
            )
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(ids.contains(&"e1"), "full display name must match, got: {:?}", ids);
    }

    // ── `from:` filter contract ─────────────────────────────────────────────
    //
    // The tests above this point each pin ONE needle shape, added reactively
    // after a user hit it. That is how the "person + company" bug survived:
    // nobody had written down what the filter is supposed to accept, so every
    // new phrasing was an untested shape. These two tests are that contract —
    // one seeded sender, and the space of needles a human or the query planner
    // can plausibly produce for them. Add a row here before adding a branch to
    // the `from` filter.
    //
    // The sender is deliberately synthetic (`.example` is the RFC 2606 reserved
    // TLD) so the table can live in git.

    /// The one sender every contract row is aimed at.
    const CONTRACT_SENDER: (&str, &str) = ("Nadia Brookes", "nadia.brookes@northwind.example");
    /// A decoy sharing the first name, at a different company.
    const CONTRACT_DECOY: (&str, &str) = ("Nadia Ferrer", "nadia.ferrer@seabright.example");

    fn contract_db() -> Database {
        let db = Database::new_for_testing().unwrap();
        insert_search_email(
            &db,
            "e1",
            "acc1",
            "thread-brookes",
            CONTRACT_SENDER.0,
            CONTRACT_SENDER.1,
            "Contract review",
            "attached the revised draft",
            100,
        );
        insert_search_email(
            &db,
            "e2",
            "acc1",
            "thread-ferrer",
            CONTRACT_DECOY.0,
            CONTRACT_DECOY.1,
            "Lunch tomorrow?",
            "want to grab lunch",
            200,
        );
        db
    }

    fn from_search(db: &Database, needle: &str) -> Vec<String> {
        db.search_emails("acc1", "", None, Some(needle), None, None, None, None, None, 50)
            .unwrap()
            .iter()
            .map(|e| e.id.clone())
            .collect()
    }

    #[test]
    fn from_filter_contract_finds_the_sender_for_every_plausible_needle() {
        let db = contract_db();
        // (needle, why it is a shape a real caller produces)
        let must_match: &[(&str, &str)] = &[
            ("nadia", "first name alone"),
            ("brookes", "surname alone"),
            ("Nadia Brookes", "full display name"),
            ("nadia brookes", "display name, lowercased"),
            ("nadia.brookes@northwind.example", "the verbatim address"),
            ("nadia.brookes", "the address local part"),
            (
                "northwind",
                "the COMPANY alone — the domain, which is in no display name",
            ),
            ("northwind.example", "the bare domain with its TLD"),
            ("de northwind", "Spanish 'from <company>' — connector plus domain"),
            ("nadia northwind", "person AND company, no connector"),
            ("nadia de northwind", "person AND company, Spanish connector"),
            ("brookes northwind", "surname AND company"),
        ];
        let mut failures = Vec::new();
        for (needle, why) in must_match {
            let ids = from_search(&db, needle);
            if !ids.contains(&"e1".to_string()) {
                failures.push(format!("  from={:?} ({}) → {:?}", needle, why, ids));
            }
        }
        assert!(
            failures.is_empty(),
            "these needles must all reach {} <{}>:\n{}",
            CONTRACT_SENDER.0,
            CONTRACT_SENDER.1,
            failures.join("\n")
        );
    }

    #[test]
    fn from_filter_contract_does_not_leak_the_other_sender() {
        // Broadening recall must not cost precision: nothing that names only
        // the decoy's company may return the contract sender, and vice versa.
        let db = contract_db();
        let must_not_match: &[(&str, &str)] = &[
            ("seabright", "the decoy's company alone"),
            ("ferrer", "the decoy's surname"),
            ("nadia seabright", "shared first name, WRONG company"),
            ("brookes seabright", "right surname, wrong company"),
        ];
        let mut failures = Vec::new();
        for (needle, why) in must_not_match {
            let ids = from_search(&db, needle);
            if ids.contains(&"e1".to_string()) {
                failures.push(format!("  from={:?} ({}) → {:?}", needle, why, ids));
            }
        }
        assert!(
            failures.is_empty(),
            "these needles must NOT reach {} <{}>:\n{}",
            CONTRACT_SENDER.0,
            CONTRACT_SENDER.1,
            failures.join("\n")
        );
    }

    #[test]
    fn search_emails_ordered_ascending_returns_oldest_first() {
        // Regression for "primer correo / first email": the only way to surface
        // the FIRST matching email is ascending sort. Default stays newest-first.
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_search_email(
            &db,
            "old",
            account,
            "t-old",
            "Alice",
            "alice@example.com",
            "First",
            "b",
            100,
        );
        insert_search_email(
            &db,
            "mid",
            account,
            "t-mid",
            "Alice",
            "alice@example.com",
            "Second",
            "b",
            200,
        );
        insert_search_email(
            &db,
            "new",
            account,
            "t-new",
            "Alice",
            "alice@example.com",
            "Third",
            "b",
            300,
        );

        let newest = db
            .search_emails(
                account,
                "",
                None,
                Some("alice@example.com"),
                None,
                None,
                None,
                None,
                None,
                1,
            )
            .unwrap();
        assert_eq!(
            newest.first().map(|e| e.id.as_str()),
            Some("new"),
            "default search_emails must stay newest-first"
        );

        let oldest = db
            .search_emails_ordered(
                account,
                "",
                None,
                Some("alice@example.com"),
                None,
                None,
                None,
                None,
                None,
                1,
                true,
                false,
                false,
                false,
                None,
            )
            .unwrap();
        assert_eq!(
            oldest.first().map(|e| e.id.as_str()),
            Some("old"),
            "ascending must surface the oldest (first) matching email"
        );
    }

    /// A thread the user started long ago and replied to recently.
    fn seed_long_running_thread(db: &Database, account: &str) {
        insert_search_email(
            db,
            "opener",
            account,
            "t-long",
            "Me",
            "me@example.com",
            "Kickoff",
            "b",
            100,
        );
        insert_search_email(
            db,
            "reply",
            account,
            "t-long",
            "Me",
            "me@example.com",
            "Re: Kickoff",
            "b",
            900,
        );
        insert_search_email(
            db,
            "single",
            account,
            "t-single",
            "Me",
            "me@example.com",
            "Invoice",
            "b",
            500,
        );
    }

    fn oldest_first(db: &Database, from: Option<&str>, subject: Option<&str>) -> Vec<String> {
        db.search_emails_ordered(
            "acc1", "", None, from, None, subject, None, None, None, 1, true, false, false, false, None,
        )
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect()
    }

    #[test]
    fn oldest_first_ranks_a_thread_by_its_earliest_match_not_its_latest_reply() {
        // "The first email I sent" was answered with a September email because
        // a June thread's latest reply made it sort last.
        let db = Database::new_for_testing().unwrap();
        seed_long_running_thread(&db, "acc1");
        assert_eq!(oldest_first(&db, Some("me@example.com"), None), vec!["opener"]);
    }

    #[test]
    fn oldest_first_ranks_by_earliest_match_on_the_general_path_too() {
        // A subject filter without `from` takes the general (CTE) path.
        let db = Database::new_for_testing().unwrap();
        seed_long_running_thread(&db, "acc1");
        assert_eq!(oldest_first(&db, None, Some("Kickoff")), vec!["opener"]);
    }

    #[test]
    fn search_from_display_name_finds_thread() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "thread-alice",
            "Alice Smith",
            "alice@example.com",
            "Hello",
            "body text",
            100,
        );
        insert_search_email(
            &db,
            "e2",
            account,
            "thread-bob",
            "Bob Jones",
            "bob@other.com",
            "Hi",
            "body text",
            200,
        );

        // "Alice" is a display-name query — only FTS sender-field can match it
        let results = db
            .search_emails(account, "", None, Some("Alice"), None, None, None, None, None, 50)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(
            ids.contains(&"e1"),
            "display-name 'Alice' must find her thread, got: {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e2"),
            "Bob must not appear in Alice search, got: {:?}",
            ids
        );
    }

    #[test]
    fn search_from_prefix_matches_partial_address() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "thread-alice",
            "Alice Smith",
            "alice@example.com",
            "Hello",
            "body text",
            100,
        );
        insert_search_email(
            &db,
            "e2",
            account,
            "thread-bob",
            "Bob Jones",
            "bob@other.com",
            "Hi",
            "body text",
            200,
        );

        // "alice" as prefix must match alice@example.com via LIKE 'alice%'
        let results = db
            .search_emails(account, "", None, Some("alice"), None, None, None, None, None, 50)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(
            ids.contains(&"e1"),
            "prefix 'alice' must match alice@example.com, got: {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e2"),
            "Bob must not appear in prefix search, got: {:?}",
            ids
        );
    }

    #[test]
    fn search_from_returns_latest_matching_email_per_thread() {
        // `from:alice` must return Alice's actual email — not Bob's later reply
        // from the same thread. Showing the thread-latest row even when it did
        // not match the filter was confusing: chat tool callers interpreted
        // Bob's reply as "from Alice", and inbox users saw unrelated replies
        // surface under a sender filter. The current behaviour picks the
        // latest MATCHING email per thread.
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        // Alice's original at t=100
        insert_search_email(
            &db,
            "e1",
            account,
            "thread-conv",
            "Alice Smith",
            "alice@example.com",
            "Project update",
            "Let's meet",
            100,
        );
        // Bob's reply at t=200 — thread-latest, but not from alice
        insert_search_email(
            &db,
            "e2",
            account,
            "thread-conv",
            "Bob Jones",
            "bob@other.com",
            "Re: Project update",
            "Sounds good",
            200,
        );
        // Unrelated thread
        insert_search_email(
            &db,
            "e3",
            account,
            "thread-other",
            "Carol Lee",
            "carol@other.com",
            "Invoice",
            "see attached",
            300,
        );

        let results = db
            .search_emails(account, "", None, Some("alice"), None, None, None, None, None, 50)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        // Alice's email (e1) is the only one matching `from:alice` — it must
        // represent the thread even though Bob's e2 is newer.
        assert!(
            ids.contains(&"e1"),
            "Alice's matching email must appear for from:alice, got: {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e2"),
            "Bob's reply must NOT appear — it does not match from:alice, got: {:?}",
            ids
        );
        // Carol's thread is unrelated
        assert!(
            !ids.contains(&"e3"),
            "Carol's unrelated thread must not appear, got: {:?}",
            ids
        );
    }

    #[test]
    fn search_from_no_false_positives() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "thread-alice",
            "Alice Smith",
            "alice@example.com",
            "Hello",
            "body text",
            100,
        );

        let results = db
            .search_emails(
                account,
                "",
                None,
                Some("nobody@unknown.com"),
                None,
                None,
                None,
                None,
                None,
                50,
            )
            .unwrap();

        assert!(
            results.is_empty(),
            "unknown sender must return empty, got {} results",
            results.len()
        );
    }

    #[test]
    fn search_from_deleted_email_excluded() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "thread-alice",
            "Alice Smith",
            "alice@example.com",
            "Hello",
            "body text",
            100,
        );
        db.delete_email("e1").unwrap();

        let results = db
            .search_emails(account, "", None, Some("alice"), None, None, None, None, None, 50)
            .unwrap();

        assert!(
            results.is_empty(),
            "deleted emails must not appear in from: search, got {} results",
            results.len()
        );
    }

    #[test]
    fn search_from_cross_account_isolation() {
        let db = Database::new_for_testing().unwrap();

        // Two accounts, Alice exists only in acc1
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at) VALUES ('acc1','gmail','a@a.com','A',0)",
                [],
            )
            .unwrap();
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at) VALUES ('acc2','gmail','b@b.com','B',0)",
                [],
            )
            .unwrap();

        insert_search_email(
            &db,
            "e1",
            "acc1",
            "thread-a",
            "Alice Smith",
            "alice@example.com",
            "Hello",
            "body",
            100,
        );

        // acc2 must not see acc1's emails
        let results = db
            .search_emails("acc2", "", None, Some("alice"), None, None, None, None, None, 50)
            .unwrap();

        assert!(
            results.is_empty(),
            "acc2 must not see acc1 emails in from: search, got {} results",
            results.len()
        );
    }

    #[test]
    fn search_from_large_mailbox_finds_correct_emails() {
        // Insert 500 emails from various senders to verify correctness under load.
        // (Performance on disk with 35k rows is validated at runtime, not here.)
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        for i in 0..490 {
            insert_search_email(
                &db,
                &format!("noise-{i}"),
                account,
                &format!("thread-noise-{i}"),
                "Noise Sender",
                &format!("noise{i}@noise.com"),
                &format!("Noise {i}"),
                "noise body",
                i as i64,
            );
        }
        // 10 emails from alice
        for i in 0..10 {
            insert_search_email(
                &db,
                &format!("alice-{i}"),
                account,
                &format!("thread-alice-{i}"),
                "Alice Smith",
                "alice@example.com",
                &format!("Alice msg {i}"),
                "alice body",
                (500 + i) as i64,
            );
        }

        let results = db
            .search_emails(
                account,
                "",
                None,
                Some("alice@example.com"),
                None,
                None,
                None,
                None,
                None,
                100,
            )
            .unwrap();

        // Must find exactly the 10 Alice threads (single-email threads, so representative = alice's email)
        assert_eq!(
            results.len(),
            10,
            "must find exactly 10 Alice threads, got {} results",
            results.len()
        );
        for r in &results {
            assert_eq!(
                r.sender_email, "alice@example.com",
                "every result must be from alice, got: {}",
                r.sender_email
            );
        }
    }

    // ── Keyword (FTS) search correctness ────────────────────────────────────

    #[test]
    fn search_keyword_finds_by_subject() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "t1",
            "Alice",
            "alice@ex.com",
            "Project invoice for Q4",
            "body text",
            100,
        );
        insert_search_email(
            &db,
            "e2",
            account,
            "t2",
            "Bob",
            "bob@ex.com",
            "Meeting notes",
            "body text",
            200,
        );

        let results = db
            .search_emails(account, "invoice", None, None, None, None, None, None, None, 50)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(
            ids.contains(&"e1"),
            "email with 'invoice' in subject must be found, got: {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e2"),
            "email without 'invoice' must not appear, got: {:?}",
            ids
        );
    }

    #[test]
    fn search_keyword_finds_by_body() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "t1",
            "Alice",
            "alice@ex.com",
            "Hello",
            "Please review the contract details",
            100,
        );
        insert_search_email(
            &db,
            "e2",
            account,
            "t2",
            "Bob",
            "bob@ex.com",
            "Hi",
            "Nothing relevant here",
            200,
        );

        let results = db
            .search_emails(account, "contract", None, None, None, None, None, None, None, 50)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(
            ids.contains(&"e1"),
            "email with 'contract' in body must be found, got: {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e2"),
            "email without 'contract' must not appear, got: {:?}",
            ids
        );
    }

    #[test]
    fn search_keyword_multi_word_requires_all() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "t1",
            "Alice",
            "alice@ex.com",
            "Meeting notes",
            "from today's standup",
            100,
        );
        insert_search_email(
            &db,
            "e2",
            account,
            "t2",
            "Bob",
            "bob@ex.com",
            "Meeting agenda",
            "tomorrow's plan",
            200,
        );

        let results = db
            .search_emails(account, "meeting notes", None, None, None, None, None, None, None, 50)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(
            ids.contains(&"e1"),
            "email with both 'meeting' and 'notes' must be found, got: {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e2"),
            "email with only 'meeting' must not appear, got: {:?}",
            ids
        );
    }

    #[test]
    fn search_keyword_no_match_returns_empty() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "t1",
            "Alice",
            "alice@ex.com",
            "Hello",
            "body text",
            100,
        );

        let results = db
            .search_emails(account, "xyznonexistent", None, None, None, None, None, None, None, 50)
            .unwrap();
        assert!(
            results.is_empty(),
            "non-matching keyword must return empty, got {} results",
            results.len()
        );
    }

    #[test]
    fn search_keyword_deleted_excluded() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "t1",
            "Alice",
            "alice@ex.com",
            "Invoice reminder",
            "body",
            100,
        );
        db.delete_email("e1").unwrap();

        let results = db
            .search_emails(account, "invoice", None, None, None, None, None, None, None, 50)
            .unwrap();
        assert!(
            results.is_empty(),
            "deleted email must not appear in keyword search, got {} results",
            results.len()
        );
    }

    #[test]
    fn search_keyword_cross_account_isolation() {
        let db = Database::new_for_testing().unwrap();

        insert_search_email(&db, "e1", "acc1", "t1", "Alice", "alice@ex.com", "Invoice", "body", 100);

        let results = db
            .search_emails("acc2", "invoice", None, None, None, None, None, None, None, 50)
            .unwrap();
        assert!(
            results.is_empty(),
            "acc2 must not see acc1 emails in keyword search, got {} results",
            results.len()
        );
    }

    #[test]
    fn search_keyword_returns_latest_matching_email_per_thread() {
        // Keyword search must surface the email that actually matches, not a
        // newer non-matching reply in the same thread.
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        // e1 (older) has "invoice" in subject/body; e2 (newer reply) does not.
        insert_search_email(
            &db,
            "e1",
            account,
            "thread-conv",
            "Alice",
            "alice@ex.com",
            "Invoice attached",
            "see invoice details",
            100,
        );
        insert_search_email(
            &db,
            "e2",
            account,
            "thread-conv",
            "Bob",
            "bob@ex.com",
            "Got it",
            "thanks for sending that",
            200,
        );

        let results = db
            .search_emails(account, "invoice", None, None, None, None, None, None, None, 50)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(
            ids.contains(&"e1"),
            "e1 (the matching email) must appear, got: {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e2"),
            "e2 (newer reply, not matching 'invoice') must NOT appear, got: {:?}",
            ids
        );
    }

    #[test]
    fn search_keyword_html_tags_not_matched() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        // Email body contains <table> tag but no actual "table" content
        insert_search_email(
            &db,
            "e1",
            account,
            "t1",
            "Newsletter",
            "news@ex.com",
            "Weekly Update",
            "<div><table><tr><td>Important content here</td></tr></table></div>",
            100,
        );
        // Email body actually mentions "table" as content
        insert_search_email(
            &db,
            "e2",
            account,
            "t2",
            "Alice",
            "alice@ex.com",
            "Office Setup",
            "Please reserve the conference table for tomorrow",
            200,
        );

        let results = db
            .search_emails(account, "table", None, None, None, None, None, None, None, 50)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();

        assert!(
            ids.contains(&"e2"),
            "email with 'table' as content must be found, got: {:?}",
            ids
        );
        assert!(
            !ids.contains(&"e1"),
            "email with 'table' only in HTML tags must NOT match, got: {:?}",
            ids
        );
    }

    #[test]
    fn search_keyword_html_style_block_stripped() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db, "e1", account, "t1", "Newsletter", "news@ex.com",
            "Weekly",
            "<html><head><style>.display{color:red} .hidden{visibility:hidden}</style></head><body><p>Hello world</p></body></html>",
            100,
        );

        // CSS class names like "display", "hidden", "visibility" must not match
        let results = db
            .search_emails(account, "display", None, None, None, None, None, None, None, 50)
            .unwrap();
        assert!(
            results.is_empty(),
            "'display' from CSS must not match, got {} results",
            results.len()
        );

        // Actual content "Hello" should match
        let results = db
            .search_emails(account, "hello", None, None, None, None, None, None, None, 50)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"e1"), "'hello' from content must match, got: {:?}", ids);
    }

    #[test]
    fn search_keyword_prefix_matching() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(
            &db,
            "e1",
            account,
            "t1",
            "Alice",
            "alice@ex.com",
            "Invoice attached",
            "body",
            100,
        );

        // "inv" prefix should match "invoice"
        let results = db
            .search_emails(account, "inv", None, None, None, None, None, None, None, 50)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"e1"), "prefix 'inv' must match 'invoice', got: {:?}", ids);
    }

    #[test]
    fn search_keyword_whitespace_only_does_not_crash() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_search_email(&db, "e1", account, "t1", "Alice", "alice@ex.com", "Hello", "body", 100);

        // Whitespace-only query must not crash
        let result = db.search_emails(account, "   ", None, None, None, None, None, None, None, 50);
        assert!(
            result.is_ok(),
            "whitespace-only query must not crash: {:?}",
            result.err()
        );
    }

    #[test]
    fn strip_html_for_fts_strips_tags() {
        assert_eq!(strip_html_for_fts("hello world"), "hello world");
        assert_eq!(strip_html_for_fts("<p>hello</p>"), "hello");
        assert_eq!(
            strip_html_for_fts("<div><table><tr><td>data</td></tr></table></div>"),
            "data"
        );
    }

    #[test]
    fn strip_html_for_fts_strips_style_blocks() {
        let html = "<html><head><style>.foo{color:red}</style></head><body>Content</body></html>";
        let result = strip_html_for_fts(html);
        assert!(!result.contains("foo"), "CSS class names must be stripped: {}", result);
        assert!(!result.contains("color"), "CSS properties must be stripped: {}", result);
        assert!(result.contains("Content"), "Body content must be preserved: {}", result);
    }

    #[test]
    fn strip_html_for_fts_decodes_entities() {
        assert_eq!(strip_html_for_fts("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(strip_html_for_fts("a &lt; b &gt; c"), "a < b > c");
    }

    /// Benchmark `search_emails` with from: filter against the real production DB.
    /// Run with: cargo test -p emailops bench_from_search_prod -- --nocapture --ignored
    #[test]
    #[ignore] // only runs manually — requires the production DB to exist
    fn bench_from_search_prod() {
        use std::path::PathBuf;

        let db_path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.emailops.app")
            .join("emailops.db");

        if !db_path.exists() {
            eprintln!("Production DB not found at {:?}, skipping", db_path);
            return;
        }

        eprintln!("\n=== Benchmark: from: search on production DB ===");
        eprintln!("DB path: {:?}", db_path);
        eprintln!(
            "DB size: {:.1} MB",
            std::fs::metadata(&db_path).unwrap().len() as f64 / 1_000_000.0
        );

        let db = Database::open_readonly(db_path).expect("Failed to open production DB");

        // Find the account with the most emails
        let (account_id, email_count): (String, i64) = db
            .reader()
            .query_row(
                "SELECT account_id, COUNT(*) as cnt FROM emails GROUP BY account_id ORDER BY cnt DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        eprintln!("Account: {} ({} emails)", account_id, email_count);

        // Find a sender with some emails
        let from_name: String = db
            .reader()
            .query_row(
                "SELECT SUBSTR(sender_email, 1, INSTR(sender_email, '@') - 1) \
                 FROM emails WHERE account_id = ?1 \
                 GROUP BY sender_email ORDER BY COUNT(*) DESC LIMIT 1 OFFSET 2",
                rusqlite::params![account_id],
                |row| row.get(0),
            )
            .unwrap();
        eprintln!("Searching from:{}\n", from_name);

        // --- Warm-up run ---
        let _ = db.search_emails(
            &account_id,
            "",
            None,
            Some(&from_name),
            None,
            None,
            None,
            None,
            None,
            100,
        );

        // --- Timed runs ---
        let categories = ["primary".to_string()];
        let cat_refs: Vec<&str> = categories.iter().map(|s| s.as_str()).collect();

        for run in 1..=3 {
            let t = std::time::Instant::now();
            let results = db
                .search_emails(
                    &account_id,
                    "",
                    Some(&cat_refs.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
                    Some(&from_name),
                    None,
                    None,
                    None,
                    None,
                    None,
                    100,
                )
                .unwrap();
            eprintln!(
                "Run {}: {:.0}ms — {} results",
                run,
                t.elapsed().as_secs_f64() * 1000.0,
                results.len()
            );
        }

        // --- Component timing ---
        eprintln!("\n--- Component breakdown ---");
        let conn = db.reader();

        // 1. from_match: sender_email range scan
        let from_lower = from_name.to_lowercase();
        let upper = prefix_upper_bound(&from_lower).unwrap();
        let t = std::time::Instant::now();
        let sender_ids: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM emails INDEXED BY idx_emails_sender_email \
                 WHERE account_id = ?1 AND sender_email >= ?2 AND sender_email < ?3",
                )
                .unwrap();
            stmt.query_map(rusqlite::params![account_id, from_lower, upper], |row| row.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        eprintln!(
            "  sender_email range scan: {:.0}ms ({} rows)",
            t.elapsed().as_secs_f64() * 1000.0,
            sender_ids.len()
        );

        // 2. FTS sender search
        let fts_query = format!("sender:\"{}\"*", from_name);
        let t = std::time::Instant::now();
        let fts_ids: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?1")
                .unwrap();
            stmt.query_map(rusqlite::params![fts_query], |row| row.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        eprintln!(
            "  FTS sender search: {:.0}ms ({} rows)",
            t.elapsed().as_secs_f64() * 1000.0,
            fts_ids.len()
        );

        // 3. Combine and get thread_ids
        let all_ids: std::collections::HashSet<String> = sender_ids.into_iter().chain(fts_ids).collect();
        let t = std::time::Instant::now();
        let mut thread_ids: Vec<String> = Vec::new();
        if !all_ids.is_empty() {
            let placeholders: String = (0..all_ids.len())
                .map(|i| format!("?{}", i + 2))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT DISTINCT thread_id FROM emails \
                 WHERE account_id = ?1 AND id IN ({}) AND is_deleted = 0 AND category = 'primary'",
                placeholders
            );
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.clone())];
            for id in &all_ids {
                params.push(Box::new(id.clone()));
            }
            let mut stmt = conn.prepare(&sql).unwrap();
            let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            thread_ids = stmt
                .query_map(refs.as_slice(), |row| row.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect();
        }
        eprintln!(
            "  thread_id lookup: {:.0}ms ({} threads)",
            t.elapsed().as_secs_f64() * 1000.0,
            thread_ids.len()
        );

        // 4. Latest-per-thread via NOT EXISTS
        if !thread_ids.is_empty() {
            let placeholders: String = (0..thread_ids.len())
                .map(|i| format!("?{}", i + 2))
                .collect::<Vec<_>>()
                .join(",");
            let latest_pred = latest_thread_email_predicate("e");
            let sql = format!(
                "SELECT COUNT(*) FROM emails e \
                 WHERE e.account_id = ?1 AND e.is_deleted = 0 \
                   AND e.thread_id IN ({placeholders}) \
                   AND {latest}",
                placeholders = placeholders,
                latest = latest_pred,
            );
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.clone())];
            for tid in &thread_ids {
                params.push(Box::new(tid.clone()));
            }
            let t = std::time::Instant::now();
            let mut stmt = conn.prepare(&sql).unwrap();
            let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let count: i64 = stmt.query_row(refs.as_slice(), |row| row.get(0)).unwrap();
            eprintln!(
                "  latest-per-thread (NOT EXISTS): {:.0}ms ({} rows)",
                t.elapsed().as_secs_f64() * 1000.0,
                count
            );

            // 4b. Try alternative: MAX(timestamp) GROUP BY
            let sql2 = format!(
                "SELECT COUNT(*) FROM (\
                    SELECT thread_id, MAX(timestamp) as max_ts \
                    FROM emails \
                    WHERE account_id = ?1 AND is_deleted = 0 AND thread_id IN ({placeholders}) \
                    GROUP BY thread_id\
                 )",
                placeholders = placeholders,
            );
            let t = std::time::Instant::now();
            let mut stmt2 = conn.prepare(&sql2).unwrap();
            let count2: i64 = stmt2.query_row(refs.as_slice(), |row| row.get(0)).unwrap();
            eprintln!(
                "  latest-per-thread (GROUP BY): {:.0}ms ({} rows)",
                t.elapsed().as_secs_f64() * 1000.0,
                count2
            );

            // 4c. Try: subquery per thread_id
            let sql3 = format!(
                "SELECT {cols} FROM emails e WHERE e.id IN (\
                    SELECT (\
                        SELECT id FROM emails \
                        WHERE account_id = ?1 AND thread_id = t.thread_id AND is_deleted = 0 \
                        ORDER BY timestamp DESC, id DESC LIMIT 1\
                    ) FROM (SELECT DISTINCT thread_id FROM emails WHERE account_id = ?1 AND thread_id IN ({placeholders})) t\
                 ) ORDER BY e.timestamp DESC, e.id DESC LIMIT 100",
                cols = EMAIL_COLUMNS,
                placeholders = placeholders,
            );
            let t = std::time::Instant::now();
            let mut stmt3 = conn.prepare(&sql3).unwrap();
            let results: Vec<Email> = stmt3
                .query_map(refs.as_slice(), row_to_email)
                .unwrap()
                .filter_map(|r| r.ok())
                .collect();
            eprintln!(
                "  latest-per-thread (scalar subquery): {:.0}ms ({} rows)",
                t.elapsed().as_secs_f64() * 1000.0,
                results.len()
            );
        }

        eprintln!("\n=== Done ===\n");
    }

    /// Production FTS diagnostic: benchmarks keyword search and reports HTML stripping impact.
    /// Run with: cargo test -p emailops report_fts_diagnostic -- --nocapture --ignored
    #[test]
    #[ignore]
    fn report_fts_diagnostic() {
        use std::path::PathBuf;

        let db_path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.emailops.app")
            .join("emailops.db");

        if !db_path.exists() {
            eprintln!("Production DB not found at {:?}, skipping", db_path);
            return;
        }

        let db_size_mb = std::fs::metadata(&db_path).unwrap().len() as f64 / 1_000_000.0;
        let db = Database::open_readonly(db_path.clone()).expect("Failed to open production DB");

        let (account_id, email_count): (String, i64) = db
            .reader()
            .query_row(
                "SELECT account_id, COUNT(*) as cnt FROM emails GROUP BY account_id ORDER BY cnt DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        let fts_count: i64 = db
            .reader()
            .query_row("SELECT COUNT(*) FROM emails_fts", [], |row| row.get(0))
            .unwrap();

        eprintln!("\n╔══════════════════════════════════════════════════════════════╗");
        eprintln!("║            FTS Search Diagnostic Report                     ║");
        eprintln!("╠══════════════════════════════════════════════════════════════╣");
        eprintln!("║ DB path : {:?}", db_path);
        eprintln!("║ DB size : {:.1} MB", db_size_mb);
        eprintln!("║ Emails  : {}", email_count);
        eprintln!("║ FTS rows: {}", fts_count);
        eprintln!("║ Account : {}", account_id);
        eprintln!("╚══════════════════════════════════════════════════════════════╝");

        // ── 1. HTML pollution check ──────────────────────────────────────────
        eprintln!("\n━━━ 1. HTML Pollution in FTS Index ━━━");
        let html_tags = ["table", "div", "style", "span", "class", "font", "display", "hidden"];
        let conn = db.reader();
        for tag in &html_tags {
            let query = format!("\"{}\"", tag);
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM emails_fts WHERE emails_fts MATCH ?1",
                    rusqlite::params![query],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            let marker = if count > 100 { " ← POLLUTED" } else { "" };
            eprintln!("  FTS MATCH '{}' → {} hits{}", tag, count, marker);
        }

        // ── 2. Sample: HTML in body vs stripped ──────────────────────────────
        eprintln!("\n━━━ 2. HTML Stripping Sample ━━━");
        let sample: Option<(String, String)> = conn
            .query_row(
                "SELECT e.id, eb.body FROM emails e JOIN email_bodies eb ON eb.email_id = e.id WHERE eb.body LIKE '%<table%' AND e.account_id = ?1 LIMIT 1",
                rusqlite::params![account_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        if let Some((id, body)) = sample {
            let raw_len = body.len();
            let stripped = strip_html_for_fts(&body);
            let stripped_len = stripped.len();
            eprintln!("  Email ID  : {}", id);
            eprintln!("  Raw body  : {} chars", raw_len);
            eprintln!(
                "  Stripped  : {} chars ({:.0}% reduction)",
                stripped_len,
                (1.0 - stripped_len as f64 / raw_len as f64) * 100.0
            );
            eprintln!("  Preview   : {}...", &stripped[..stripped.len().min(120)]);
        } else {
            eprintln!("  (no HTML emails found)");
        }

        // ── 3. Keyword search benchmarks ─────────────────────────────────────
        eprintln!("\n━━━ 3. Keyword Search Benchmarks (GROUP BY path) ━━━");
        let keywords = ["invoice", "meeting", "project", "update", "report"];
        for kw in &keywords {
            // Warm up
            let _ = db.search_emails(&account_id, kw, None, None, None, None, None, None, None, 50);

            let mut times = Vec::new();
            let mut result_count = 0;
            for _ in 0..3 {
                let t = std::time::Instant::now();
                let results = db
                    .search_emails(&account_id, kw, None, None, None, None, None, None, None, 50)
                    .unwrap();
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                result_count = results.len();
            }
            let avg = times.iter().sum::<f64>() / times.len() as f64;
            let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
            eprintln!(
                "  '{}'{} → {} results | avg {:.0}ms, best {:.0}ms",
                kw,
                " ".repeat(10 - kw.len()),
                result_count,
                avg,
                min,
            );
        }

        // ── 4. GROUP BY vs scalar subquery comparison ────────────────────────
        eprintln!("\n━━━ 4. GROUP BY vs Scalar Subquery Comparison ━━━");
        // Pick a keyword with decent results
        let test_kw = "invoice";
        let fts_query = sanitize_fts_query(test_kw);
        if fts_query.is_empty() {
            eprintln!("  (skipped — empty FTS query)");
        } else {
            // Step 1: get matching thread_ids via FTS
            let thread_ids: Vec<String> = {
                let mut stmt = conn
                    .prepare(
                        "SELECT DISTINCT e.thread_id FROM emails e
                         WHERE e.account_id = ?1 AND e.is_deleted = 0
                           AND e.id IN (SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?2)",
                    )
                    .unwrap();
                stmt.query_map(rusqlite::params![account_id, fts_query], |row| row.get(0))
                    .unwrap()
                    .filter_map(|r| r.ok())
                    .collect()
            };
            eprintln!("  FTS match → {} threads for '{}'", thread_ids.len(), test_kw);

            if !thread_ids.is_empty() && thread_ids.len() <= 32766 {
                let tid_start = 2usize;
                let tid_phs: Vec<String> = (0..thread_ids.len()).map(|i| format!("?{}", tid_start + i)).collect();
                let phs = tid_phs.join(", ");

                let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.clone())];
                for tid in &thread_ids {
                    params.push(Box::new(tid.clone()));
                }
                let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

                // Method A: GROUP BY + MAX(timestamp)
                let sql_group = format!(
                    "SELECT COUNT(*) FROM (
                        SELECT thread_id, MAX(timestamp) as max_ts
                        FROM emails
                        WHERE account_id = ?1 AND is_deleted = 0 AND thread_id IN ({phs})
                        GROUP BY thread_id
                    )"
                );
                let t = std::time::Instant::now();
                let mut stmt = conn.prepare(&sql_group).unwrap();
                let count_a: i64 = stmt.query_row(refs.as_slice(), |row| row.get(0)).unwrap();
                let time_group = t.elapsed().as_secs_f64() * 1000.0;

                // Method B: Scalar subquery (old approach)
                let latest_pred = latest_thread_email_predicate("e");
                let sql_scalar = format!(
                    "SELECT COUNT(*) FROM emails e
                     WHERE e.account_id = ?1 AND e.is_deleted = 0
                       AND e.thread_id IN ({phs})
                       AND {latest}",
                    latest = latest_pred,
                );
                let t = std::time::Instant::now();
                let mut stmt = conn.prepare(&sql_scalar).unwrap();
                let count_b: i64 = stmt.query_row(refs.as_slice(), |row| row.get(0)).unwrap();
                let time_scalar = t.elapsed().as_secs_f64() * 1000.0;

                let speedup = if time_group > 0.0 {
                    time_scalar / time_group
                } else {
                    0.0
                };
                eprintln!("  GROUP BY        : {:.0}ms ({} threads)", time_group, count_a);
                eprintln!("  Scalar subquery : {:.0}ms ({} threads)", time_scalar, count_b);
                eprintln!("  Speedup         : {:.1}x", speedup);
            }
        }

        // ── 5. Edge cases ────────────────────────────────────────────────────
        eprintln!("\n━━━ 5. Edge Cases ━━━");
        // Whitespace-only
        let t = std::time::Instant::now();
        let result = db.search_emails(&account_id, "   ", None, None, None, None, None, None, None, 50);
        eprintln!(
            "  Whitespace '   '  : {} ({:.0}ms)",
            match &result {
                Ok(r) => format!("{} results", r.len()),
                Err(e) => format!("ERROR: {e}"),
            },
            t.elapsed().as_secs_f64() * 1000.0,
        );

        // Special chars
        let t = std::time::Instant::now();
        let result = db.search_emails(&account_id, "***", None, None, None, None, None, None, None, 50);
        eprintln!(
            "  Special '***'     : {} ({:.0}ms)",
            match &result {
                Ok(r) => format!("{} results", r.len()),
                Err(e) => format!("ERROR: {e}"),
            },
            t.elapsed().as_secs_f64() * 1000.0,
        );

        // Multi-word
        let t = std::time::Instant::now();
        let results = db
            .search_emails(
                &account_id,
                "meeting notes",
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                50,
            )
            .unwrap();
        eprintln!(
            "  Multi 'meeting notes' : {} results ({:.0}ms)",
            results.len(),
            t.elapsed().as_secs_f64() * 1000.0,
        );

        eprintln!("\n══════════════════════════════════════════════════════════════");
        eprintln!("  Report complete");
        eprintln!("══════════════════════════════════════════════════════════════\n");
    }

    // ── participants ("emails exchanged with X") ──

    fn set_recipients(db: &Database, id: &str, recipients: &[&str]) {
        db.connection()
            .execute(
                "UPDATE emails SET recipients_json = ?1 WHERE id = ?2",
                rusqlite::params![serde_json::to_string(recipients).unwrap(), id],
            )
            .unwrap();
    }

    fn exchanged_with(db: &Database, participants: &[String]) -> Vec<String> {
        let mut ids: Vec<String> = db
            .search_emails_ordered(
                "acc1",
                "",
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                50,
                false,
                false,
                false,
                false,
                Some(participants),
            )
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        ids.sort();
        ids
    }

    fn seed_exchange(db: &Database) {
        insert_search_email(db, "e1", "acc1", "t1", "Ana Ruiz", "ana@x.example", "Hola", "body", 100);
        insert_search_email(
            db,
            "e2",
            "acc1",
            "t2",
            "Me",
            "me@mine.example",
            "Propuesta",
            "body",
            200,
        );
        set_recipients(db, "e2", &["ana@x.example"]);
        insert_search_email(db, "e3", "acc1", "t3", "Bob", "bob@y.example", "Otro", "body", 300);
        insert_search_email(
            db,
            "e4",
            "acc1",
            "t4",
            "Me",
            "me@mine.example",
            "Sin nombre",
            "body",
            400,
        );
        set_recipients(db, "e4", &["gm@we.example"]);
    }

    #[test]
    fn a_participant_matches_whether_they_sent_or_received_the_email() {
        let db = Database::new_for_testing().unwrap();
        seed_exchange(&db);
        assert_eq!(exchanged_with(&db, &["ana".to_string()]), vec!["e1", "e2"]);
    }

    #[test]
    fn a_participant_named_only_by_name_needs_their_address_to_match_mail_to_them() {
        let db = Database::new_for_testing().unwrap();
        seed_exchange(&db);
        // e4 went to gm@we.example: the name alone is nowhere in it.
        assert!(exchanged_with(&db, &["genoveva".to_string()]).is_empty());
        assert_eq!(
            exchanged_with(&db, &["genoveva".to_string(), "gm@we.example".to_string()]),
            vec!["e4"]
        );
    }

    #[test]
    fn a_name_resolves_to_the_addresses_it_has_written_from() {
        let db = Database::new_for_testing().unwrap();
        insert_search_email(
            &db,
            "e1",
            "acc1",
            "t1",
            "Genoveva Mendoza",
            "gm@we.example",
            "a",
            "b",
            1,
        );
        insert_search_email(
            &db,
            "e2",
            "acc1",
            "t2",
            "Genoveva M.",
            "genoveva@home.example",
            "a",
            "b",
            2,
        );
        insert_search_email(&db, "e3", "acc1", "t3", "Bob", "bob@y.example", "a", "b", 3);
        insert_search_email(&db, "e4", "acc2", "t4", "Genoveva", "other@acct.example", "a", "b", 4);
        let mut found = db.sender_addresses_matching("acc1", "genoveva", 10).unwrap();
        found.sort();
        assert_eq!(found, vec!["genoveva@home.example", "gm@we.example"]);
    }

    // Regression: the attachment-extension EXISTS used a bare `id`, which
    // SQLite binds to the innermost table (email_attachment_meta.id), so the
    // filter compared an attachment's email_id to its own id and never matched.
    #[test]
    fn filtered_emails_attachment_ext_matches_email_with_that_attachment() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_account(&db, account, "me@example.com");
        insert_contact_email(&db, "e1", account, "t1", "Bob", "bob@example.com", "[]", "inbox", 100);
        insert_contact_email(&db, "e2", account, "t2", "Bob", "bob@example.com", "[]", "inbox", 200);
        db.connection()
            .execute(
                "INSERT INTO email_attachment_meta (id, email_id, account_id, filename, mime_type)
                 VALUES ('att1', 'e1', ?1, 'Report.PDF', 'application/pdf')",
                rusqlite::params![account],
            )
            .unwrap();

        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::Account(account),
                None,
                Some("bob@example.com"),
                None,
                None,
                Some("pdf"),
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();
        let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["e1"], "only the email carrying a .pdf attachment matches");
    }

    // ── Thread dedup: same-second ties ──────────────────────────────────────────

    // Regression: thread dedup joined back on `timestamp = MAX(timestamp)`, so
    // two emails of one thread stamped in the same second both came back and
    // the thread showed twice. The representative must be exactly one row,
    // tie-broken like the inbox (`timestamp DESC, id DESC`).
    #[test]
    fn filtered_emails_same_second_thread_tie_returns_one_row() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_account(&db, account, "me@example.com");
        insert_contact_email(&db, "e-a", account, "t1", "Bob", "bob@example.com", "[]", "inbox", 100);
        insert_contact_email(&db, "e-b", account, "t1", "Bob", "bob@example.com", "[]", "inbox", 100);

        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::Account(account),
                None,
                Some("bob@example.com"),
                None,
                None,
                None,
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();
        let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["e-b"]);
    }

    fn insert_same_second_thread(db: &Database, account: &str) {
        for id in ["e-a", "e-b"] {
            insert_search_email(
                db,
                id,
                account,
                "t1",
                "Alice",
                "alice@example.com",
                "invoice",
                "body",
                100,
            );
        }
    }

    #[test]
    fn search_keyword_same_second_thread_tie_returns_one_row() {
        let db = Database::new_for_testing().unwrap();
        insert_same_second_thread(&db, "acc1");
        let results = db
            .search_emails("acc1", "invoice", None, None, None, None, None, None, None, 50)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["e-b"]);
    }

    #[test]
    fn search_from_same_second_thread_tie_returns_one_row() {
        let db = Database::new_for_testing().unwrap();
        insert_same_second_thread(&db, "acc1");
        let results = db
            .search_emails("acc1", "", None, Some("alice"), None, None, None, None, None, 50)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["e-b"]);
    }

    // Regression: the `from:` fast path bound every matched id as its own
    // parameter, so a sender with more emails than SQLite's variable limit
    // (32,766) made the search fail outright.
    #[test]
    fn search_from_with_more_matches_than_the_sql_variable_limit() {
        let db = Database::new_for_testing().unwrap();
        let mut conn = db.connection();
        ensure_account(&conn, "acc1");
        let tx = conn.transaction().unwrap();
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO emails
                         (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                          recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                     VALUES (?1, 'acc1', ?1, 's', 'Alice', 'alice@example.com', 'example.com',
                             '[]', '[]', '', ?2, 0, 'primary', 'inbox', 0)",
                )
                .unwrap();
            for i in 0..33_000_i64 {
                stmt.execute(rusqlite::params![format!("e{i:05}"), i]).unwrap();
            }
        }
        tx.commit().unwrap();
        drop(conn);

        let results = db
            .search_emails("acc1", "", None, Some("alice"), None, None, None, None, None, 5)
            .unwrap();
        let ids: Vec<&str> = results.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["e32999", "e32998", "e32997", "e32996", "e32995"]);
    }

    // Regression: a keyword made only of symbols ("?!") sanitized to an empty
    // FTS query, the keyword condition was dropped, and every thread matched.
    #[test]
    fn search_symbol_only_query_returns_nothing() {
        let db = Database::new_for_testing().unwrap();
        insert_search_email(
            &db,
            "e1",
            "acc1",
            "t1",
            "Alice",
            "alice@example.com",
            "hello",
            "body",
            100,
        );
        let results = db
            .search_emails("acc1", "?!", None, None, None, None, None, None, None, 50)
            .unwrap();
        assert!(
            results.is_empty(),
            "got {:?}",
            results.iter().map(|e| &e.id).collect::<Vec<_>>()
        );
    }

    // ── archive is live mail ─────────────────────────────────────────────────────
    // Archived mail left the Inbox view only: sender/domain/tag filters and their
    // suggestion counts reach it, as Gmail and Outlook search and labels do.

    #[test]
    fn quick_filter_stats_count_archived_mail() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_account(&db, account, "me@mymail.com");
        insert_contact_email(
            &db,
            "a1",
            account,
            "t-a1",
            "Ana",
            "ana@archived.example",
            "[]",
            "archive",
            100,
        );

        let stats = db
            .get_quick_filter_stats(crate::db::AccountScope::Account(account), &[], &[])
            .unwrap();
        let domains: Vec<&str> = stats.top_domains.iter().map(|d| d.value.as_str()).collect();
        let senders: Vec<&str> = stats.top_senders.iter().map(|s| s.value.as_str()).collect();
        assert_eq!(domains, vec!["archived.example"]);
        assert_eq!(senders, vec!["ana@archived.example"]);
    }

    #[test]
    fn filtered_emails_sender_filter_includes_archived_mail() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_account(&db, account, "me@mymail.com");
        insert_contact_email(&db, "in", account, "t1", "Ana", "ana@example.com", "[]", "inbox", 100);
        insert_contact_email(
            &db,
            "arch",
            account,
            "t2",
            "Ana",
            "ana@example.com",
            "[]",
            "archive",
            200,
        );
        insert_contact_email(&db, "junk", account, "t3", "Ana", "ana@example.com", "[]", "spam", 300);

        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::Account(account),
                None,
                Some("ana@example.com"),
                None,
                None,
                None,
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();
        let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["arch", "in"]);
    }

    #[test]
    fn filtered_emails_tag_filter_includes_archived_mail() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_account(&db, account, "me@mymail.com");
        insert_contact_email(
            &db,
            "arch",
            account,
            "t1",
            "Ana",
            "ana@example.com",
            "[]",
            "archive",
            100,
        );
        tag_email(&db, "arch", "company", "Acme");

        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::Account(account),
                None,
                None,
                Some("company"),
                Some("Acme"),
                None,
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();
        let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["arch"]);
    }

    // A thread whose newest message was archived shows that message, not an
    // older inbox one, as the filtered list's row for the thread.
    #[test]
    fn filtered_emails_thread_row_is_the_newest_live_message_even_when_archived() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_account(&db, account, "me@mymail.com");
        insert_contact_email(&db, "old", account, "t1", "Ana", "ana@example.com", "[]", "inbox", 100);
        insert_contact_email(
            &db,
            "new",
            account,
            "t1",
            "Ana",
            "ana@example.com",
            "[]",
            "archive",
            200,
        );

        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::Account(account),
                Some("example.com"),
                None,
                None,
                None,
                None,
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();
        let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["new"]);
    }

    // The tag path skips COUNT(*) for the infinite-scroll list and says so
    // with -1, which the frontend reads as "unknown", not as a count.
    #[test]
    fn a_tag_filter_reports_its_total_as_unknown() {
        let db = Database::new_for_testing().unwrap();
        insert_email(&db, "e1", "acc1", "thread-a", 100);
        tag_email(&db, "e1", "priority", "urgent");

        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::Account("acc1"),
                None,
                None,
                Some("priority"),
                Some("urgent"),
                None,
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();

        assert_eq!(result.emails.len(), 1);
        assert_eq!(result.total_count, -1);
    }

    #[test]
    fn a_sender_or_domain_filter_reports_its_total_as_unknown() {
        let db = Database::new_for_testing().unwrap();
        insert_email(&db, "e1", "acc1", "thread-a", 100);

        let result = db
            .get_filtered_emails(
                crate::db::AccountScope::Account("acc1"),
                Some("s.com"),
                None,
                None,
                None,
                None,
                &crate::models::EmailWindow::default(),
                50,
                0,
            )
            .unwrap();

        assert_eq!(result.emails.len(), 1);
        assert_eq!(result.total_count, -1);
    }

    // The category placeholders shift every later bind: the date bound must
    // still land on its own placeholder.
    #[test]
    fn a_date_search_combines_categories_with_the_date_window() {
        let db = Database::new_for_testing().unwrap();
        insert_email_with_category(&db, "old", "acc1", "t1", 100, "primary");
        insert_email_with_category(&db, "promo", "acc1", "t2", 200, "promotions");
        insert_email_with_category(&db, "new", "acc1", "t3", 300, "primary");
        let primary = vec!["primary".to_string()];

        let found = db
            .search_emails_by_date("acc1", Some(&primary), Some(150), None, 50, false, false, false, false)
            .unwrap();

        let ids: Vec<&str> = found.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["new"]);
    }

    #[test]
    fn an_addressee_splits_into_its_display_name_and_address() {
        assert_eq!(
            super::split_addressee(r#" "Ada L" <ada@x.example> "#),
            ("Ada L".to_string(), "ada@x.example".to_string())
        );
        assert_eq!(
            super::split_addressee("b@y.example"),
            (String::new(), "b@y.example".to_string())
        );
        assert_eq!(
            super::split_addressee("Ada <ada@x.example"),
            (String::new(), "Ada <ada@x.example".to_string())
        );
    }
}
