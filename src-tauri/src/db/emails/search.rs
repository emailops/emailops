use super::*;

impl Database {
    /// Get aggregate stats for smart filter suggestions, excluding removed filters
    pub fn get_quick_filter_stats(
        &self,
        account_id: &str,
        excluded_domains: &[String],
        excluded_senders: &[String],
    ) -> Result<QuickFilterStats> {
        let conn = self.reader();

        // Top 10 sender domains, excluding removed ones
        let domain_exclude_clause = if excluded_domains.is_empty() {
            String::new()
        } else {
            let placeholders: Vec<String> = excluded_domains
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect();
            format!("HAVING domain NOT IN ({})", placeholders.join(", "))
        };

        let domain_sql = format!(
            "SELECT sender_domain AS domain,
                    COUNT(*) AS cnt
             FROM emails WHERE account_id = ?1
               AND sender_domain != ''
               AND is_deleted = 0
             GROUP BY domain {}
             ORDER BY cnt DESC LIMIT 10",
            domain_exclude_clause
        );

        let mut domain_params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.to_string())];
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

        // Top 10 individual senders, excluding removed ones
        let sender_exclude_clause = if excluded_senders.is_empty() {
            String::new()
        } else {
            let placeholders: Vec<String> = excluded_senders
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect();
            format!("AND sender_email NOT IN ({})", placeholders.join(", "))
        };

        let sender_sql = format!(
            "SELECT sender_email, COUNT(*) AS cnt
             FROM emails WHERE account_id = ?1
               AND is_deleted = 0 {}
             GROUP BY sender_email ORDER BY cnt DESC LIMIT 10",
            sender_exclude_clause
        );

        let mut sender_params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.to_string())];
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

    /// Get emails filtered by domain or sender, with total count for pagination.
    ///
    /// Uses subquery-based approach to avoid both:
    ///   - O(N²) NOT EXISTS correlated subquery
    ///   - SQLite parameter limit (32,766) for large thread_id sets
    ///
    /// The matching thread_ids stay inside a CTE subquery; only account_id and
    /// the filter value are passed as parameters.
    pub fn get_filtered_emails(
        &self,
        account_id: &str,
        domain: Option<&str>,
        sender_email: Option<&str>,
        tag_type: Option<&str>,
        tag_value: Option<&str>,
        attachment_ext: Option<&str>,
        limit: i32,
        offset: i32,
    ) -> Result<FilteredEmailsResult> {
        let conn = self.reader();
        let order_clause = thread_order_clause("e");

        // ── Tag filter: drive from email_tags (small result set) ─────────────
        // Tags are selective — few emails match. Drive from the tag index, find
        // matching threads, GROUP BY for latest-per-thread. Fast even with 0 matches.
        if let (Some(tt), Some(tv)) = (tag_type, tag_value) {
            // ?1 = account_id, ?2 = tag_type, ?3 = tag_value, ?4 = limit, ?5 = offset
            //
            // Semantics (user-confirmed): a thread matches the filter if ANY email
            // in the thread carries the tag. The row shown for that thread is the
            // thread representative (latest email in the thread). This way,
            // replying to "Globex" doesn't make the thread disappear from the
            // Globex filter just because the user's sent reply is the latest.
            //
            // Same shape as the domain/sender branch below: two CTEs (match → latest)
            // and a final join on (thread_id, timestamp) which lets SQLite use
            // `idx_emails_thread_latest (account_id, thread_id, timestamp DESC, id DESC)`.
            // A 3rd "representative_emails" CTE was tried and was much slower because
            // it forced a join-by-id then a full sort instead of an index-driven
            // (thread_id, timestamp) lookup.
            //
            // `mailbox IN ('inbox', 'sent')` keeps Spam/Trash copies out of Inbox-level
            // filtered views.
            // `INDEXED BY idx_email_tags_type_value` is critical: without it SQLite
            // picks the inverted plan — scan all ~87k emails of the account and
            // probe email_tags by email_id — instead of starting from the tag
            // (which yields ~hundreds of rows). With the hint, matched_threads
            // costs O(emails_tagged_with_this_value), not O(account_emails).
            let select_sql = format!(
                "WITH matched_threads AS (
                     SELECT DISTINCT e2.thread_id
                     FROM email_tags et INDEXED BY idx_email_tags_type_value
                     JOIN emails e2 ON e2.id = et.email_id
                     WHERE et.tag_type = ?2 AND et.tag_value = ?3
                       AND e2.account_id = ?1 AND e2.is_deleted = 0
                       AND e2.mailbox IN ('inbox', 'sent')
                 ),
                 thread_latest AS (
                     SELECT thread_id AS tid, MAX(timestamp) AS max_ts
                     FROM emails
                     WHERE account_id = ?1 AND is_deleted = 0
                       AND mailbox IN ('inbox', 'sent')
                       AND thread_id IN (SELECT thread_id FROM matched_threads)
                     GROUP BY thread_id
                 )
                 SELECT {cols}
                 FROM emails e
                 INNER JOIN thread_latest l ON e.thread_id = l.tid AND e.timestamp = l.max_ts
                 WHERE e.account_id = ?1 AND e.is_deleted = 0 AND e.mailbox IN ('inbox', 'sent')
                 ORDER BY {order}
                 LIMIT ?4 OFFSET ?5",
                cols = EMAIL_COLUMNS,
                order = order_clause,
            );

            let mut stmt = conn.prepare(&select_sql)?;
            let mut emails = Vec::new();
            let mut rows = stmt.query(params![account_id, tt, tv, limit, offset])?;
            while let Some(row) = rows.next()? {
                emails.push(row_to_email(row)?);
            }
            return Ok(FilteredEmailsResult {
                emails,
                total_count: -1,
            });
        }

        // ── Domain/sender filter: three-step CTE ───────────────────────────────
        // Step 1: find matching thread_ids via idx_emails_domain_filter or
        //         idx_emails_sender_filter (covering, no table access needed).
        // Step 2: GROUP BY for latest timestamp per thread — O(matching_threads).
        // Step 3: join back to emails to fetch the representative row.
        // This is O(emails_from_domain) rather than O(all_emails) and avoids
        // scanning the full inbox in timestamp order.
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.to_string())];
        let mut param_idx = 2usize; // ?1 = account_id

        // Inbox-level filtered view: exclude Spam/Trash copies so they don't
        // leak into the main filter UI. Soft-deleted rows are also excluded.
        let mut match_conditions = vec![
            "account_id = ?1".to_string(),
            "is_deleted = 0".to_string(),
            "mailbox IN ('inbox', 'sent')".to_string(),
        ];
        if let Some(d) = domain {
            match_conditions.push(format!("sender_domain = ?{}", param_idx));
            params_vec.push(Box::new(d.to_lowercase()));
            param_idx += 1;
        }
        if let Some(s) = sender_email {
            match_conditions.push(format!("sender_email = ?{}", param_idx));
            params_vec.push(Box::new(s.to_string()));
            param_idx += 1;
        }
        if let Some(ext) = attachment_ext {
            match_conditions.push(format!(
                "EXISTS (SELECT 1 FROM email_attachment_meta am WHERE am.email_id = id AND LOWER(am.filename) LIKE ?{})",
                param_idx
            ));
            params_vec.push(Box::new(format!("%.{}", ext.to_lowercase())));
            param_idx += 1;
        }

        let select_sql = format!(
            "WITH matched_threads AS (
                 SELECT DISTINCT thread_id
                 FROM emails
                 WHERE {match_cond}
             ),
             thread_latest AS (
                 SELECT thread_id AS tid, MAX(timestamp) AS max_ts
                 FROM emails
                 WHERE account_id = ?1 AND is_deleted = 0 AND mailbox IN ('inbox', 'sent')
                   AND thread_id IN (SELECT thread_id FROM matched_threads)
                 GROUP BY thread_id
             )
             SELECT {cols}
             FROM emails e
             INNER JOIN thread_latest l ON e.thread_id = l.tid AND e.timestamp = l.max_ts
             WHERE e.account_id = ?1 AND e.is_deleted = 0 AND e.mailbox IN ('inbox', 'sent')
             ORDER BY {order}
             LIMIT ?{limit_idx} OFFSET ?{offset_idx}",
            match_cond = match_conditions.join(" AND "),
            cols = EMAIL_COLUMNS,
            order = order_clause,
            limit_idx = param_idx,
            offset_idx = param_idx + 1,
        );

        params_vec.push(Box::new(limit));
        params_vec.push(Box::new(offset));

        let mut stmt = conn.prepare(&select_sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let mut emails = Vec::new();
        let mut rows = stmt.query(params_refs.as_slice())?;
        while let Some(row) = rows.next()? {
            emails.push(row_to_email(row)?);
        }

        Ok(FilteredEmailsResult {
            emails,
            total_count: -1,
        })
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
    ) -> Result<Vec<Email>> {
        let conn = self.reader();
        let mut conditions: Vec<String> = vec!["e.account_id = ?1".to_string(), "e.is_deleted = 0".to_string()];
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
            "SELECT {cols} FROM emails e WHERE {where} ORDER BY e.timestamp DESC, e.id DESC LIMIT ?{limit_idx}",
            cols = EMAIL_COLUMNS,
            where = where_clause,
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
        tag_filters: Option<&[String]>,
        limit: i32,
    ) -> Result<Vec<Email>> {
        // ── Date-only fast path (no text filters) ────────────────────────────────
        // When there are no text-based filters (keyword, from, to, subject, tag),
        // thread deduplication is wrong: the user wants ALL emails in the window,
        // not just the latest email per thread. Example: search_emails(since=today)
        // should return every individual email received today, not one per thread.
        let has_text_filter = !query.is_empty()
            || from_filter.is_some()
            || to_filter.is_some()
            || subject_filter.is_some()
            || tag_filters.map(|t| !t.is_empty()).unwrap_or(false);

        if !has_text_filter {
            return self.search_emails_by_date(account_id, categories, after_timestamp, before_timestamp, limit);
        }

        let conn = self.reader();
        let order_clause = thread_order_clause("e");

        // ── CTE: find thread_ids that contain a matching email ────────────────────
        // All filter conditions apply to the same email row (`match_e`) so that
        // `from:alice subject:meeting` requires a single message to satisfy both.
        let mut cte_conditions: Vec<String> = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let mut param_idx = 1usize;
        // Optional pre-CTE for the from filter — populated when both email-prefix
        // and FTS-sender branches are needed so each can use its own index.
        let mut from_match_cte: Option<String> = None;
        // Number of params consumed up to (and including) the `from` block.
        // Step 1 of the fast path only references those params, so we must not
        // pass subject/date/tag params to it — SQLite rejects extra positional
        // params as "Wrong number of parameters". Updated right after the from
        // block is appended to params_vec.
        let mut from_params_end: usize = 0;

        // account_id is ?1 — used in both the CTE and outer query.
        params_vec.push(Box::new(account_id.to_string()));
        cte_conditions.push(format!("match_e.account_id = ?{}", param_idx));
        cte_conditions.push("match_e.is_deleted = 0".to_string());
        param_idx += 1;

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
            if !fts_query.is_empty() {
                cte_conditions.push(format!(
                    "match_e.id IN (SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?{})",
                    param_idx
                ));
                params_vec.push(Box::new(fts_query));
                param_idx += 1;
            }
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

                from_match_cte = Some(format!(
                    "from_match AS (
                         SELECT id AS email_id
                         FROM emails INDEXED BY idx_emails_sender_email_nocase
                         WHERE account_id = ?1
                           AND {range}
                         UNION
                         SELECT email_id FROM emails_fts WHERE emails_fts MATCH ?{fts_idx}
                     )",
                    range = range_clause,
                    fts_idx = param_idx + range_params,
                ));
                params_vec.push(Box::new(from_lower));
                if let Some(ub) = upper_bound {
                    params_vec.push(Box::new(ub));
                }
                params_vec.push(Box::new(fts_sender));
                param_idx += range_params + 1;
                // The JOIN into from_match is handled in query assembly below.
            }
            from_params_end = params_vec.len();
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

        // Tag filters
        if let Some(tags) = tag_filters.filter(|t| !t.is_empty()) {
            for tag_value in tags {
                cte_conditions.push(format!(
                    "EXISTS (SELECT 1 FROM email_tags et WHERE et.email_id = match_e.id AND et.tag_value = ?{})",
                    param_idx
                ));
                params_vec.push(Box::new(tag_value.clone()));
                param_idx += 1;
            }
        }

        // ── Assemble and execute ─────────────────────────────────────────────────
        let cte_where = cte_conditions.join(" AND ");

        // ── Fast path: when from_match_cte is present, use a three-step approach ─
        //
        // Step 1: Materialise from_match email IDs (sender index + FTS).
        // Step 2: PK-lookup those IDs in `emails` to get thread_ids + apply
        //         category / is_deleted / date filters.
        // Step 3: For each matching thread, find the latest email using GROUP BY
        //         (benchmarked at 4 ms vs 3,200 ms for NOT EXISTS on 47k rows).
        //
        // Every step is either an index range-scan or a PK lookup, so we never
        // touch more rows than the result set.
        if let Some(ref from_cte) = from_match_cte {
            // ── Step 1: get matching email IDs from sender index + FTS ────────
            // Only pass params referenced by the from_match CTE (account_id +
            // from-filter bindings). Any subject / date / tag params live later
            // in params_vec; forwarding them would trip SQLite's strict
            // positional-parameter count check.
            let ids_sql = format!("WITH {} SELECT email_id FROM from_match", from_cte);
            let mut ids_stmt = conn.prepare(&ids_sql)?;
            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params_vec[..from_params_end].iter().map(|p| p.as_ref()).collect();
            let email_ids: Vec<String> = ids_stmt
                .query_map(params_refs.as_slice(), |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            if email_ids.is_empty() {
                return Ok(Vec::new());
            }

            // ── Step 2: PK-lookup to get thread_ids with filters ──────────────
            // Build filter conditions for the PK lookup (category, is_deleted, date).
            // We reuse the conditions from cte_conditions but replace match_e with e.
            let mut pk_conditions: Vec<String> = vec!["e.account_id = ?1".to_string(), "e.is_deleted = 0".to_string()];
            let mut pk_params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.to_string())];

            // Category filter
            if let Some(cats) = categories.filter(|c| !c.is_empty()) {
                let start_idx = pk_params.len() + 1;
                let phs: Vec<String> = (0..cats.len()).map(|i| format!("?{}", start_idx + i)).collect();
                pk_conditions.push(format!("e.category IN ({})", phs.join(", ")));
                for cat in cats {
                    pk_params.push(Box::new(cat.clone()));
                }
            }

            // Date filters
            if let Some(after) = after_timestamp {
                pk_conditions.push(format!("e.timestamp >= ?{}", pk_params.len() + 1));
                pk_params.push(Box::new(after));
            }
            if let Some(before) = before_timestamp {
                pk_conditions.push(format!("e.timestamp <= ?{}", pk_params.len() + 1));
                pk_params.push(Box::new(before));
            }

            // email ID IN list
            let id_start = pk_params.len() + 1;
            let id_phs: Vec<String> = (0..email_ids.len()).map(|i| format!("?{}", id_start + i)).collect();
            pk_conditions.push(format!("e.id IN ({})", id_phs.join(",")));
            for eid in &email_ids {
                pk_params.push(Box::new(eid.clone()));
            }

            let pk_where = pk_conditions.join(" AND ");
            // Step 2 now returns the fully-filtered email IDs (after category /
            // date / is_deleted filters) instead of DISTINCT thread_ids. Step 3
            // picks the latest MATCHING email per thread — a thread containing
            // alice's email + the user's later reply must return alice's row,
            // not the reply's. Using thread_id alone in Step 3 ignored whether
            // the latest email matched the filter, producing the wrong row.
            let matched_sql = format!("SELECT e.id FROM emails e WHERE {}", pk_where);
            let mut matched_stmt = conn.prepare(&matched_sql)?;
            let pk_refs: Vec<&dyn rusqlite::ToSql> = pk_params.iter().map(|p| p.as_ref()).collect();
            let matched_ids: Vec<String> = matched_stmt
                .query_map(pk_refs.as_slice(), |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            if matched_ids.is_empty() {
                return Ok(Vec::new());
            }

            // ── Step 3: latest MATCHING email per thread via GROUP BY ─────────
            // GROUP BY + MAX(timestamp) is ~250x faster than NOT EXISTS for
            // finding the latest email per thread (4ms vs 3,200ms benchmarked).
            // Placeholders for matched_ids are reused in two places (the
            // grouping subquery AND the outer JOIN's `e.id IN (...)` guard).
            // SQLite binds reused positional params to a single value — we
            // only push each id once.
            let id_start = 2usize; // ?1 = account_id
            let id_phs: Vec<String> = (0..matched_ids.len()).map(|i| format!("?{}", id_start + i)).collect();
            let limit_idx = id_start + matched_ids.len();
            let final_sql = format!(
                "SELECT {cols}
                 FROM emails e
                 INNER JOIN (
                     SELECT thread_id AS tid, MAX(timestamp) AS max_ts
                     FROM emails
                     WHERE account_id = ?1 AND is_deleted = 0 AND id IN ({phs})
                     GROUP BY thread_id
                 ) l ON e.thread_id = l.tid AND e.timestamp = l.max_ts
                 WHERE e.account_id = ?1 AND e.is_deleted = 0 AND e.id IN ({phs})
                 ORDER BY {order}
                 LIMIT ?{limit_idx}",
                phs = id_phs.join(", "),
                cols = EMAIL_COLUMNS,
                order = order_clause,
                limit_idx = limit_idx,
            );

            let mut final_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            final_params.push(Box::new(account_id.to_string()));
            for eid in &matched_ids {
                final_params.push(Box::new(eid.clone()));
            }
            final_params.push(Box::new(limit));

            let mut final_stmt = conn.prepare(&final_sql)?;
            let final_refs: Vec<&dyn rusqlite::ToSql> = final_params.iter().map(|p| p.as_ref()).collect();
            let emails = final_stmt.query_map(final_refs.as_slice(), row_to_email)?;
            let mut result = Vec::new();
            for email in emails {
                result.push(email?);
            }
            return Ok(result);
        }

        // ── General path (no from_match CTE) ────────────────────────────────────
        // GROUP BY + MAX(timestamp) is ~250x faster than the scalar subquery
        // for finding the latest email per thread (benchmarked on 47k emails).
        // `filter_match` emits id/thread_id/timestamp for matching emails,
        // so `thread_latest` groups by thread over ONLY the matching rows —
        // not all emails in those threads. The outer query then restricts
        // `e.id IN filter_match` so a non-matching email with the same
        // timestamp cannot slip through. See the regression test
        // `search_emails_from_filter_returns_matching_email_not_reply`.
        let sql = format!(
            "WITH filter_match AS (
                 SELECT match_e.id, match_e.thread_id, match_e.timestamp
                 FROM emails match_e
                 WHERE {cte_where}
             ),
             thread_latest AS (
                 SELECT thread_id AS tid, MAX(timestamp) AS max_ts
                 FROM filter_match
                 GROUP BY thread_id
             )
             SELECT {cols}
             FROM emails e
             INNER JOIN thread_latest tl ON e.thread_id = tl.tid AND e.timestamp = tl.max_ts
             WHERE e.account_id = ?1 AND e.is_deleted = 0
               AND e.id IN (SELECT id FROM filter_match)
             ORDER BY {order}
             LIMIT ?{limit_idx}",
            cte_where = cte_where,
            cols = EMAIL_COLUMNS,
            order = order_clause,
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
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use super::super::*;
    use crate::db::Database;

    // Regression for user-confirmed semantics: a tag filter (e.g. company "Globex")
    // must match any thread where AT LEAST ONE email carries the tag, even when the
    // user has replied and their sent message is now the thread representative.
    // The row returned for each matching thread is the latest email in the thread.
    #[test]
    fn tag_filter_matches_thread_if_any_email_tagged_and_returns_latest() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        // Thread A: E1 (old, urgent) + E2 (newer, no urgent tag — represents user reply).
        // The thread should match because E1 is urgent; the row returned is E2 (latest).
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
            .get_filtered_emails(account, None, None, Some("priority"), Some("urgent"), None, 50, 0)
            .unwrap();

        let ids: Vec<&str> = result.emails.iter().map(|e| e.id.as_str()).collect();

        // Thread B is fully represented by its only (urgent) email.
        assert!(
            ids.contains(&"e3"),
            "e3 (urgent thread B) should appear, got: {:?}",
            ids
        );

        // Thread A matches because E1 is urgent. The representative row is E2 (latest),
        // even though E2 itself is tagged "normal" — this is the corrected behavior.
        assert!(
            ids.contains(&"e2"),
            "e2 (latest in urgent-tagged thread A) should appear, got: {:?}",
            ids
        );

        // E1 is not the thread representative — only one row per thread.
        assert!(
            !ids.contains(&"e1"),
            "e1 (non-representative) must not appear, got: {:?}",
            ids
        );

        // Thread C never had an urgent tag.
        assert!(
            !ids.contains(&"e4"),
            "e4 (no urgent tag anywhere in thread C) must not appear, got: {:?}",
            ids
        );

        // Exactly two thread representatives match.
        assert_eq!(
            result.emails.len(),
            2,
            "exactly 2 threads should match urgent filter, got: {} ({:?})",
            result.emails.len(),
            ids
        );

        // Order: newest first (timestamp DESC).
        assert_eq!(ids, vec!["e3", "e2"], "results should be newest-first");
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
}
