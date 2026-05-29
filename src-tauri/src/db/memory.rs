//! Low-level SQL for the memory subsystem (facts, thread states, pending
//! tasks, interaction events).
//!
//! This module mirrors the convention set by `db/tags.rs` and `db/chat.rs`:
//! plain `impl Database` methods that do one SQL operation each, returning
//! domain types from `crate::models`. Business logic (scoring, extraction,
//! header assembly) lives in `services/memory/`.

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{InteractionEvent, MemoryFact, PendingTask, ThreadState};
use rusqlite::{params, Row};

const PIPELINE_MEMORY_FACTS: &str = "memory_facts";
const PIPELINE_TASKS: &str = "tasks";

// ── Row helpers ──────────────────────────────────────────────────────────────

fn row_to_fact(row: &Row<'_>) -> rusqlite::Result<MemoryFact> {
    Ok(MemoryFact {
        id: row.get(0)?,
        account_id: row.get(1)?,
        subject_kind: row.get(2)?,
        subject_key: row.get(3)?,
        fact: row.get(4)?,
        source: row.get(5)?,
        source_email_id: row.get(6)?,
        confidence: row.get(7)?,
        score: row.get(8)?,
        status: row.get(9)?,
        last_used_at: row.get(10)?,
        domain: row.get(11)?,
        vigency: row.get(12)?,
        company: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

const FACT_SELECT: &str = "SELECT id, account_id, subject_kind, subject_key, fact,
                                  source, source_email_id, confidence, score, status,
                                  last_used_at, domain, vigency, company, created_at, updated_at
                           FROM memory_facts";

fn row_to_thread_state(row: &Row<'_>) -> rusqlite::Result<ThreadState> {
    let participants_json: String = row.get(9)?;
    let participants: Vec<String> = serde_json::from_str(&participants_json).unwrap_or_default();
    Ok(ThreadState {
        account_id: row.get(0)?,
        thread_id: row.get(1)?,
        awaiting: row.get(2)?,
        last_inbound_at: row.get(3)?,
        last_outbound_at: row.get(4)?,
        last_touched_at: row.get(5)?,
        summary: row.get(6)?,
        commitment: row.get(7)?,
        deadline_at: row.get(8)?,
        participants,
        updated_at: row.get(10)?,
    })
}

const THREAD_SELECT: &str = "SELECT account_id, thread_id, awaiting,
                                    last_inbound_at, last_outbound_at, last_touched_at,
                                    summary, commitment, deadline_at,
                                    participants_json, updated_at
                             FROM thread_states";

fn row_to_task(row: &Row<'_>) -> rusqlite::Result<PendingTask> {
    Ok(PendingTask {
        id: row.get(0)?,
        account_id: row.get(1)?,
        title: row.get(2)?,
        detail: row.get(3)?,
        source: row.get(4)?,
        source_email_id: row.get(5)?,
        source_thread_id: row.get(6)?,
        assignee: row.get(7)?,
        status: row.get(8)?,
        priority: row.get(9)?,
        due_at: row.get(10)?,
        completed_at: row.get(11)?,
        company: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

const TASK_SELECT: &str = "SELECT id, account_id, title, detail, source,
                                  source_email_id, source_thread_id, assignee,
                                  status, priority, due_at, completed_at,
                                  company, created_at, updated_at
                           FROM pending_tasks";

fn row_to_event(row: &Row<'_>) -> rusqlite::Result<InteractionEvent> {
    Ok(InteractionEvent {
        id: row.get(0)?,
        account_id: row.get(1)?,
        kind: row.get(2)?,
        email_id: row.get(3)?,
        thread_id: row.get(4)?,
        payload_json: row.get(5)?,
        created_at: row.get(6)?,
    })
}

// ── Memory facts ─────────────────────────────────────────────────────────────

impl Database {
    /// Insert a new fact. Callers must supply a unique id and timestamps;
    /// this keeps the function side-effect-free and friendly to batch writes.
    /// FTS is populated in the same transaction so search stays consistent.
    pub fn insert_memory_fact(&self, fact: &MemoryFact) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO memory_facts (id, account_id, subject_kind, subject_key, fact,
                                        source, source_email_id, confidence, score, status,
                                        last_used_at, domain, vigency, company, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                fact.id,
                fact.account_id,
                fact.subject_kind,
                fact.subject_key,
                fact.fact,
                fact.source,
                fact.source_email_id,
                fact.confidence,
                fact.score,
                fact.status,
                fact.last_used_at,
                fact.domain,
                fact.vigency,
                fact.company,
                fact.created_at,
                fact.updated_at,
            ],
        )?;
        // Keep FTS in sync. Errors here would leave FTS divergent from the
        // table, so propagate them rather than swallowing.
        conn.execute(
            "INSERT INTO memory_facts_fts (fact_id, fact, subject_key) VALUES (?1, ?2, ?3)",
            params![fact.id, fact.fact, fact.subject_key],
        )?;
        Ok(())
    }

    /// Update fact content. Bumps `updated_at`. Does not touch score/status
    /// (those are managed by the consolidation job). Caller is responsible
    /// for setting updated_at.
    pub fn update_memory_fact_text(&self, fact_id: &str, new_fact: &str, updated_at: i64) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE memory_facts SET fact = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_fact, updated_at, fact_id],
        )?;
        // FTS row must be replaced — FTS5 doesn't support in-place UPDATE without
        // external-content mode, which we explicitly avoided.
        conn.execute("DELETE FROM memory_facts_fts WHERE fact_id = ?1", params![fact_id])?;
        conn.execute(
            "INSERT INTO memory_facts_fts (fact_id, fact, subject_key)
             SELECT id, fact, subject_key FROM memory_facts WHERE id = ?1",
            params![fact_id],
        )?;
        Ok(())
    }

    pub fn set_memory_fact_status(&self, fact_id: &str, status: &str, updated_at: i64) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE memory_facts SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, updated_at, fact_id],
        )?;
        Ok(())
    }

    pub fn bump_memory_fact_score(&self, fact_id: &str, delta: f64, now: i64) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE memory_facts SET score = score + ?1, last_used_at = ?2, updated_at = ?2
             WHERE id = ?3",
            params![delta, now, fact_id],
        )?;
        Ok(())
    }

    /// List facts by status (e.g. all promoted facts for the header).
    /// Use `status = None` to get everything except retired.
    pub fn list_memory_facts(&self, account_id: &str, status: Option<&str>, limit: i32) -> Result<Vec<MemoryFact>> {
        let conn = self.reader();
        let (sql, bound): (String, Vec<Box<dyn rusqlite::ToSql>>) = match status {
            Some(s) => (
                format!(
                    "{FACT_SELECT} WHERE account_id = ?1 AND status = ?2
                         ORDER BY score DESC, updated_at DESC LIMIT ?3"
                ),
                vec![
                    Box::new(account_id.to_string()),
                    Box::new(s.to_string()),
                    Box::new(limit),
                ],
            ),
            None => (
                format!(
                    "{FACT_SELECT} WHERE account_id = ?1 AND status != 'retired'
                         ORDER BY score DESC, updated_at DESC LIMIT ?2"
                ),
                vec![Box::new(account_id.to_string()), Box::new(limit)],
            ),
        };
        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::ToSql> = bound.iter().map(|p| p.as_ref()).collect();
        let facts = stmt
            .query_map(refs.as_slice(), row_to_fact)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(facts)
    }

    pub fn get_memory_facts_by_subject(
        &self,
        account_id: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<Vec<MemoryFact>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "{FACT_SELECT} WHERE account_id = ?1 AND subject_kind = ?2 AND subject_key = ?3
             AND status != 'retired'
             ORDER BY score DESC, updated_at DESC"
        ))?;
        let facts = stmt
            .query_map(params![account_id, subject_kind, subject_key], row_to_fact)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(facts)
    }

    /// FTS5 search over facts. Returns (fact, bm25_score) tuples for the best
    /// matches. Caller is responsible for fusing with vector results.
    pub fn search_memory_facts_fts(&self, account_id: &str, query: &str, limit: i32) -> Result<Vec<(MemoryFact, f64)>> {
        // Sanitize: FTS5 treats punctuation as operators. Strip anything that
        // isn't alphanumeric or whitespace, then quote each token.
        let sanitized: String = query
            .split_whitespace()
            .map(|w| {
                let clean: String = w.chars().filter(|c| c.is_alphanumeric() || *c == '-').collect();
                if clean.is_empty() {
                    String::new()
                } else {
                    format!("\"{clean}\"")
                }
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" OR ");
        if sanitized.is_empty() {
            return Ok(vec![]);
        }
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT f.id, f.account_id, f.subject_kind, f.subject_key, f.fact,
                    f.source, f.source_email_id, f.confidence, f.score, f.status,
                    f.last_used_at, f.domain, f.vigency, f.company,
                    f.created_at, f.updated_at,
                    bm25(memory_facts_fts) AS rank
             FROM memory_facts f
             JOIN memory_facts_fts fts ON fts.fact_id = f.id
             WHERE f.account_id = ?1
               AND f.status != 'retired'
               AND memory_facts_fts MATCH ?2
             ORDER BY rank
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![account_id, sanitized, limit], |row| {
                let fact = row_to_fact(row)?;
                // bm25 returns negative numbers where smaller = better.
                // Convert to a positive 0..1-ish score for RRF fusion.
                let rank: f64 = row.get(16)?;
                let normalized = 1.0 / (1.0 + (-rank).max(0.0));
                Ok((fact, normalized))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn count_memory_facts(&self, account_id: &str, status: Option<&str>) -> Result<i32> {
        let conn = self.reader();
        match status {
            Some(s) => conn
                .query_row(
                    "SELECT COUNT(*) FROM memory_facts WHERE account_id = ?1 AND status = ?2",
                    params![account_id, s],
                    |row| row.get(0),
                )
                .map_err(AppError::from),
            None => conn
                .query_row(
                    "SELECT COUNT(*) FROM memory_facts WHERE account_id = ?1 AND status != 'retired'",
                    params![account_id],
                    |row| row.get(0),
                )
                .map_err(AppError::from),
        }
    }

    // ── Thread states ────────────────────────────────────────────────────────

    pub fn get_thread_state(&self, account_id: &str, thread_id: &str) -> Result<Option<ThreadState>> {
        let conn = self.reader();
        let result = conn.query_row(
            &format!("{THREAD_SELECT} WHERE account_id = ?1 AND thread_id = ?2"),
            params![account_id, thread_id],
            row_to_thread_state,
        );
        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Insert-or-upsert a thread state. Used by the extractor (full row) and
    /// by implicit update helpers (partial row via `touch_thread_state`).
    pub fn upsert_thread_state(&self, state: &ThreadState) -> Result<()> {
        let conn = self.connection();
        let participants_json = serde_json::to_string(&state.participants)
            .map_err(|e| AppError::InvalidInput(format!("participants json: {e}")))?;
        conn.execute(
            "INSERT INTO thread_states (
                account_id, thread_id, awaiting,
                last_inbound_at, last_outbound_at, last_touched_at,
                summary, commitment, deadline_at,
                participants_json, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(account_id, thread_id) DO UPDATE SET
                 awaiting = excluded.awaiting,
                 last_inbound_at = COALESCE(excluded.last_inbound_at, thread_states.last_inbound_at),
                 last_outbound_at = COALESCE(excluded.last_outbound_at, thread_states.last_outbound_at),
                 last_touched_at = excluded.last_touched_at,
                 summary = COALESCE(excluded.summary, thread_states.summary),
                 commitment = COALESCE(excluded.commitment, thread_states.commitment),
                 deadline_at = COALESCE(excluded.deadline_at, thread_states.deadline_at),
                 participants_json = excluded.participants_json,
                 updated_at = excluded.updated_at",
            params![
                state.account_id,
                state.thread_id,
                state.awaiting,
                state.last_inbound_at,
                state.last_outbound_at,
                state.last_touched_at,
                state.summary,
                state.commitment,
                state.deadline_at,
                participants_json,
                state.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Light-weight "mark touched" used by implicit action handlers. Creates
    /// the row if missing so downstream reads never see a null thread state
    /// for a thread the user actually interacted with.
    pub fn touch_thread_state(
        &self,
        account_id: &str,
        thread_id: &str,
        awaiting: Option<&str>,
        last_outbound_at: Option<i64>,
        now: i64,
    ) -> Result<()> {
        let conn = self.connection();
        // Upsert: if missing, seed with the provided awaiting (or "unknown")
        // and the touch timestamp. If present, only update fields we have new
        // info about — preserve whatever the extractor set. We can't rely on
        // `COALESCE(excluded.awaiting, …)` because the INSERT-time default
        // (`COALESCE(?3, 'unknown')`) masks a caller-supplied NULL. Bind the
        // raw awaiting param separately in the UPDATE branch so NULL means
        // "don't change".
        conn.execute(
            "INSERT INTO thread_states (
                account_id, thread_id, awaiting,
                last_inbound_at, last_outbound_at, last_touched_at,
                participants_json, updated_at
             ) VALUES (?1, ?2, COALESCE(?3, 'unknown'), NULL, ?4, ?5, '[]', ?5)
             ON CONFLICT(account_id, thread_id) DO UPDATE SET
                 awaiting = CASE WHEN ?3 IS NULL THEN thread_states.awaiting ELSE ?3 END,
                 last_outbound_at = COALESCE(?4, thread_states.last_outbound_at),
                 last_touched_at = ?5,
                 updated_at = ?5",
            params![account_id, thread_id, awaiting, last_outbound_at, now],
        )?;
        Ok(())
    }

    pub fn list_open_thread_states(
        &self,
        account_id: &str,
        awaiting: Option<&str>,
        limit: i32,
    ) -> Result<Vec<ThreadState>> {
        let conn = self.reader();
        let (sql, bound): (String, Vec<Box<dyn rusqlite::ToSql>>) = match awaiting {
            Some(a) => (
                format!(
                    "{THREAD_SELECT} WHERE account_id = ?1 AND awaiting = ?2
                         ORDER BY last_touched_at DESC LIMIT ?3"
                ),
                vec![
                    Box::new(account_id.to_string()),
                    Box::new(a.to_string()),
                    Box::new(limit),
                ],
            ),
            None => (
                format!(
                    "{THREAD_SELECT} WHERE account_id = ?1 AND awaiting != 'resolved'
                         ORDER BY last_touched_at DESC LIMIT ?2"
                ),
                vec![Box::new(account_id.to_string()), Box::new(limit)],
            ),
        };
        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::ToSql> = bound.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), row_to_thread_state)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn count_open_threads(&self, account_id: &str, awaiting: &str) -> Result<i32> {
        let conn = self.reader();
        conn.query_row(
            "SELECT COUNT(*) FROM thread_states WHERE account_id = ?1 AND awaiting = ?2",
            params![account_id, awaiting],
            |row| row.get(0),
        )
        .map_err(AppError::from)
    }

    // ── Pending tasks ────────────────────────────────────────────────────────

    pub fn insert_pending_task(&self, task: &PendingTask) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO pending_tasks (
                id, account_id, title, detail, source,
                source_email_id, source_thread_id, assignee,
                status, priority, due_at, completed_at,
                company, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                task.id,
                task.account_id,
                task.title,
                task.detail,
                task.source,
                task.source_email_id,
                task.source_thread_id,
                task.assignee,
                task.status,
                task.priority,
                task.due_at,
                task.completed_at,
                task.company,
                task.created_at,
                task.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_pending_task_status(
        &self,
        task_id: &str,
        status: &str,
        completed_at: Option<i64>,
        updated_at: i64,
    ) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE pending_tasks SET status = ?1, completed_at = ?2, updated_at = ?3 WHERE id = ?4",
            params![status, completed_at, updated_at, task_id],
        )?;
        Ok(())
    }

    pub fn list_pending_tasks(
        &self,
        account_id: &str,
        status: Option<&str>,
        due_before: Option<i64>,
        limit: i32,
    ) -> Result<Vec<PendingTask>> {
        let conn = self.reader();
        let mut sql = format!("{TASK_SELECT} WHERE account_id = ?1");
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.to_string())];
        if let Some(s) = status {
            sql.push_str(&format!(" AND status = ?{}", bound.len() + 1));
            bound.push(Box::new(s.to_string()));
        } else {
            sql.push_str(" AND status = 'open'");
        }
        if let Some(d) = due_before {
            sql.push_str(&format!(" AND due_at IS NOT NULL AND due_at <= ?{}", bound.len() + 1));
            bound.push(Box::new(d));
        }
        // Tasks with a due date come first, sorted by priority, then
        // undated tasks at the end. Using CASE keeps the statement portable.
        sql.push_str(
            " ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'normal' THEN 1 ELSE 2 END,
                       CASE WHEN due_at IS NULL THEN 1 ELSE 0 END, due_at ASC,
                       created_at DESC",
        );
        sql.push_str(&format!(" LIMIT ?{}", bound.len() + 1));
        bound.push(Box::new(limit));
        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::ToSql> = bound.iter().map(|p| p.as_ref()).collect();
        let tasks = stmt
            .query_map(refs.as_slice(), row_to_task)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tasks)
    }

    pub fn count_pending_tasks(&self, account_id: &str) -> Result<(i32, i32, i32)> {
        let conn = self.reader();
        let now = chrono::Utc::now().timestamp();
        // One day in seconds — used for "due today" (midnight-to-midnight is
        // overkill; we approximate with a 24-hour window that's good enough
        // for a count badge).
        let soon = now + 86_400;
        let total: i32 = conn.query_row(
            "SELECT COUNT(*) FROM pending_tasks WHERE account_id = ?1 AND status = 'open'",
            params![account_id],
            |row| row.get(0),
        )?;
        let overdue: i32 = conn.query_row(
            "SELECT COUNT(*) FROM pending_tasks
             WHERE account_id = ?1 AND status = 'open' AND due_at IS NOT NULL AND due_at < ?2",
            params![account_id, now],
            |row| row.get(0),
        )?;
        let due_today: i32 = conn.query_row(
            "SELECT COUNT(*) FROM pending_tasks
             WHERE account_id = ?1 AND status = 'open'
               AND due_at IS NOT NULL AND due_at >= ?2 AND due_at < ?3",
            params![account_id, now, soon],
            |row| row.get(0),
        )?;
        Ok((total, overdue, due_today))
    }

    // ── Interaction events ───────────────────────────────────────────────────

    /// Fire-and-forget-ish: best-effort log of a user action. Never blocks the
    /// caller for long — all writes are single-row with an auto-increment id.
    pub fn log_interaction_event(
        &self,
        account_id: &str,
        kind: &str,
        email_id: Option<&str>,
        thread_id: Option<&str>,
        payload_json: Option<&str>,
    ) -> Result<()> {
        let conn = self.connection();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO interaction_events (account_id, kind, email_id, thread_id, payload_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![account_id, kind, email_id, thread_id, payload_json, now],
        )?;
        Ok(())
    }

    pub fn recent_interaction_events(&self, account_id: &str, limit: i32) -> Result<Vec<InteractionEvent>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, kind, email_id, thread_id, payload_json, created_at
             FROM interaction_events WHERE account_id = ?1
             ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![account_id, limit], row_to_event)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Delete events older than `cutoff` (unix seconds). Called by the
    /// consolidation job.
    pub fn prune_interaction_events(&self, cutoff: i64) -> Result<usize> {
        let conn = self.connection();
        let n = conn.execute("DELETE FROM interaction_events WHERE created_at < ?1", params![cutoff])?;
        Ok(n)
    }

    // ── Extractor support ───────────────────────────────────────────────────
    //
    // `email_extraction_status` stores one row per (email, pipeline) after a
    // pipeline has handled that email. Facts and tasks are independent so
    // enabling/resetting/backfilling one pipeline never consumes the other.

    /// Fetch up to `limit` email ids for `account_id` that have not yet been
    /// processed by the memory extractor, ordered newest first. When
    /// `categories` is non-empty, restricts to the given Gmail categories.
    /// `min_timestamp`, when Some, excludes emails older than the cutoff (unix
    /// seconds) — used by the `task_backfill_days` window.
    pub fn get_memory_unextracted_email_ids(
        &self,
        account_id: &str,
        limit: i32,
        categories: &[String],
        min_timestamp: Option<i64>,
    ) -> Result<Vec<String>> {
        self.get_unextracted_email_ids_by_pipeline(account_id, limit, categories, min_timestamp, PIPELINE_MEMORY_FACTS)
    }

    fn get_unextracted_email_ids_by_pipeline(
        &self,
        account_id: &str,
        limit: i32,
        categories: &[String],
        min_timestamp: Option<i64>,
        pipeline: &str,
    ) -> Result<Vec<String>> {
        let conn = self.reader();
        // Compose WHERE clause dynamically, keeping placeholders in step with
        // the args vec so either optional filter can be present or absent.
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        args.push(Box::new(account_id.to_string()));
        args.push(Box::new(pipeline.to_string()));
        let mut sql = String::from(
            "SELECT id FROM emails
             WHERE account_id = ?1
               AND is_deleted = 0
               AND NOT EXISTS (
                   SELECT 1 FROM email_extraction_status s
                   WHERE s.email_id = emails.id AND s.pipeline = ?2
               )",
        );
        if !categories.is_empty() {
            let placeholders: Vec<String> = (0..categories.len())
                .map(|i| format!("?{}", args.len() + 1 + i))
                .collect();
            sql.push_str(&format!(" AND category IN ({})", placeholders.join(",")));
            for c in categories {
                args.push(Box::new(c.clone()));
            }
        }
        if let Some(ts) = min_timestamp {
            sql.push_str(&format!(" AND timestamp >= ?{}", args.len() + 1));
            args.push(Box::new(ts));
        }
        sql.push_str(&format!(" ORDER BY timestamp DESC LIMIT ?{}", args.len() + 1));
        args.push(Box::new(limit));

        let mut stmt = conn.prepare(&sql)?;
        let params_ref: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params_ref.iter()), |row| {
                row.get::<_, String>(0)
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Count rows still waiting for memory fact extraction. Used by the Memory UI
    /// to show "N emails remaining" during backfill. Honours the same
    /// categories + min_timestamp filters as `get_memory_unextracted_email_ids`.
    pub fn count_memory_unextracted_emails(
        &self,
        account_id: &str,
        categories: &[String],
        min_timestamp: Option<i64>,
    ) -> Result<i32> {
        self.count_unextracted_emails_by_pipeline(account_id, categories, min_timestamp, PIPELINE_MEMORY_FACTS)
    }

    fn count_unextracted_emails_by_pipeline(
        &self,
        account_id: &str,
        categories: &[String],
        min_timestamp: Option<i64>,
        pipeline: &str,
    ) -> Result<i32> {
        let conn = self.reader();
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        args.push(Box::new(account_id.to_string()));
        args.push(Box::new(pipeline.to_string()));
        let mut sql = String::from(
            "SELECT COUNT(*) FROM emails
             WHERE account_id = ?1
               AND is_deleted = 0
               AND NOT EXISTS (
                   SELECT 1 FROM email_extraction_status s
                   WHERE s.email_id = emails.id AND s.pipeline = ?2
               )",
        );
        if !categories.is_empty() {
            let placeholders: Vec<String> = (0..categories.len())
                .map(|i| format!("?{}", args.len() + 1 + i))
                .collect();
            sql.push_str(&format!(" AND category IN ({})", placeholders.join(",")));
            for c in categories {
                args.push(Box::new(c.clone()));
            }
        }
        if let Some(ts) = min_timestamp {
            sql.push_str(&format!(" AND timestamp >= ?{}", args.len() + 1));
            args.push(Box::new(ts));
        }
        let mut stmt = conn.prepare(&sql)?;
        let params_ref: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        let n: i32 = stmt.query_row(rusqlite::params_from_iter(params_ref.iter()), |r| r.get(0))?;
        Ok(n)
    }

    /// Mark an email as having been processed by the memory fact extractor.
    /// Idempotent — safe to call on an already-marked row.
    pub fn mark_memory_facts_extracted(&self, email_id: &str, at: i64) -> Result<()> {
        self.mark_email_pipeline_extracted(email_id, PIPELINE_MEMORY_FACTS, at)
    }

    /// Delete memory fact extraction status rows for an account so matching
    /// emails are re-queued on the next backfill or sync tick.
    pub fn reset_memory_extraction(&self, account_id: &str) -> Result<u32> {
        self.reset_email_pipeline_extraction(account_id, PIPELINE_MEMORY_FACTS)
    }

    /// Fetch task-unextracted email ids for the independent task pipeline.
    pub fn get_task_unextracted_email_ids(
        &self,
        account_id: &str,
        limit: i32,
        categories: &[String],
        min_timestamp: Option<i64>,
    ) -> Result<Vec<String>> {
        self.get_unextracted_email_ids_by_pipeline(account_id, limit, categories, min_timestamp, PIPELINE_TASKS)
    }

    pub fn count_task_unextracted_emails(
        &self,
        account_id: &str,
        categories: &[String],
        min_timestamp: Option<i64>,
    ) -> Result<i32> {
        self.count_unextracted_emails_by_pipeline(account_id, categories, min_timestamp, PIPELINE_TASKS)
    }

    pub fn mark_tasks_extracted(&self, email_id: &str, at: i64) -> Result<()> {
        self.mark_email_pipeline_extracted(email_id, PIPELINE_TASKS, at)
    }

    fn mark_email_pipeline_extracted(&self, email_id: &str, pipeline: &str, at: i64) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO email_extraction_status (email_id, pipeline, extracted_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(email_id, pipeline) DO UPDATE SET extracted_at = excluded.extracted_at",
            params![email_id, pipeline, at],
        )?;
        Ok(())
    }

    pub fn reset_task_extraction(&self, account_id: &str) -> Result<u32> {
        self.reset_email_pipeline_extraction(account_id, PIPELINE_TASKS)
    }

    fn reset_email_pipeline_extraction(&self, account_id: &str, pipeline: &str) -> Result<u32> {
        let conn = self.connection();
        let rows = conn.execute(
            "DELETE FROM email_extraction_status
             WHERE pipeline = ?1
               AND email_id IN (SELECT id FROM emails WHERE account_id = ?2)",
            params![pipeline, account_id],
        )?;
        Ok(rows as u32)
    }

    // ── Fact embeddings ─────────────────────────────────────────────────────
    //
    // One embedding row per memory_fact (facts are short — no chunking needed).
    // Mirrors the vec_emails/embedding_chunks split so the vec0 rowid ↔ fact_id
    // indirection can be broken cleanly on deletes.

    /// Fact ids that have a row in `memory_facts` but not yet in
    /// `memory_fact_chunks`. Used to drive batch embedding.
    pub fn list_facts_needing_embedding(&self, account_id: &str, limit: i32) -> Result<Vec<(String, String)>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT f.id, f.fact FROM memory_facts f
             LEFT JOIN memory_fact_chunks c ON c.fact_id = f.id
             WHERE f.account_id = ?1 AND f.status != 'retired' AND c.fact_id IS NULL
             ORDER BY f.created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![account_id, limit], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Insert (or replace) the embedding for a fact.
    pub fn upsert_fact_embedding(&self, fact_id: &str, embedding: &[f32], model: &str) -> Result<()> {
        let conn = self.connection();
        let now = chrono::Utc::now().timestamp();
        // Clear any previous embedding row for this fact.
        conn.execute(
            "DELETE FROM vec_memory_facts WHERE rowid IN (SELECT rowid FROM memory_fact_chunks WHERE fact_id = ?1)",
            params![fact_id],
        )?;
        conn.execute("DELETE FROM memory_fact_chunks WHERE fact_id = ?1", params![fact_id])?;
        conn.execute(
            "INSERT INTO memory_fact_chunks (fact_id, embedding_model, created_at)
             VALUES (?1, ?2, ?3)",
            params![fact_id, model, now],
        )?;
        let rowid = conn.last_insert_rowid();
        let blob = fact_embedding_to_blob(embedding);
        conn.execute(
            "INSERT INTO vec_memory_facts (rowid, embedding) VALUES (?1, ?2)",
            params![rowid, blob],
        )?;
        Ok(())
    }

    /// KNN search over fact embeddings, filtered to `account_id` and excluding
    /// retired facts. Returns (fact, cosine_distance) pairs, best first.
    pub fn vec_search_memory_facts(
        &self,
        query_embedding: &[f32],
        account_id: &str,
        limit: i32,
    ) -> Result<Vec<(MemoryFact, f32)>> {
        let blob = fact_embedding_to_blob(query_embedding);
        // Stage 1: KNN on vec_memory_facts — broad fetch so we can still fill
        // the requested limit after filtering.
        let expand = (limit * 5).max(10);
        let knn: Vec<(i64, f32)> = {
            let conn = self.reader();
            let mut stmt = conn.prepare(
                "SELECT rowid, distance FROM vec_memory_facts WHERE embedding MATCH ?1
                 ORDER BY distance LIMIT ?2",
            )?;
            let collected: Vec<(i64, f32)> = stmt
                .query_map(params![blob, expand], |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            collected
        };
        if knn.is_empty() {
            return Ok(Vec::new());
        }
        let dist_by_rowid: std::collections::HashMap<i64, f32> = knn.into_iter().collect();
        let rowids: Vec<i64> = dist_by_rowid.keys().copied().collect();
        let ph: String = (0..rowids.len())
            .map(|i| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");

        // Stage 2: rowid → fact_id → full row, account-filtered.
        let sql = format!(
            "SELECT {cols}, mfc.rowid FROM memory_facts f
             JOIN memory_fact_chunks mfc ON mfc.fact_id = f.id
             WHERE f.account_id = ?1 AND f.status != 'retired' AND mfc.rowid IN ({ph})",
            cols = "f.id, f.account_id, f.subject_kind, f.subject_key, f.fact,
                    f.source, f.source_email_id, f.confidence, f.score, f.status,
                    f.last_used_at, f.domain, f.vigency, f.company,
                    f.created_at, f.updated_at"
        );
        let conn = self.reader();
        let mut stmt = conn.prepare(&sql)?;
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.to_string())];
        for r in &rowids {
            bound.push(Box::new(*r));
        }
        let refs: Vec<&dyn rusqlite::ToSql> = bound.iter().map(|p| p.as_ref()).collect();
        let mut rows: Vec<(MemoryFact, f32)> = stmt
            .query_map(refs.as_slice(), |row| {
                let fact = row_to_fact(row)?;
                let rowid: i64 = row.get(16)?;
                let d = dist_by_rowid.get(&rowid).copied().unwrap_or(2.0);
                Ok((fact, d))
            })?
            .filter_map(|r| r.ok())
            .collect();
        rows.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        rows.truncate(limit as usize);
        Ok(rows)
    }

    /// Delete a fact row outright. Cascades to FTS via trigger and to
    /// memory_fact_chunks / vec_memory_facts via explicit cleanup here.
    pub fn delete_memory_fact(&self, fact_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM vec_memory_facts WHERE rowid IN (SELECT rowid FROM memory_fact_chunks WHERE fact_id = ?1)",
            params![fact_id],
        )?;
        conn.execute("DELETE FROM memory_fact_chunks WHERE fact_id = ?1", params![fact_id])?;
        conn.execute("DELETE FROM memory_facts WHERE id = ?1", params![fact_id])?;
        Ok(())
    }

    /// Consolidation support: candidates older than `older_than` with a score
    /// below `min_score`. These are the rows the dream job may retire.
    pub fn list_stale_candidate_facts(
        &self,
        account_id: &str,
        older_than: i64,
        min_score: f64,
    ) -> Result<Vec<MemoryFact>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "{FACT_SELECT} WHERE account_id = ?1 AND status = 'candidate'
               AND created_at < ?2 AND score < ?3"
        ))?;
        let rows = stmt
            .query_map(params![account_id, older_than, min_score], row_to_fact)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// All candidate facts for an account, highest score first. Dream job uses
    /// this to decide promotions.
    pub fn list_candidate_facts(&self, account_id: &str, limit: i32) -> Result<Vec<MemoryFact>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "{FACT_SELECT} WHERE account_id = ?1 AND status = 'candidate'
             ORDER BY score DESC, updated_at DESC LIMIT ?2"
        ))?;
        let rows = stmt
            .query_map(params![account_id, limit], row_to_fact)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Distinct (subject_kind, subject_key) groups that currently have two or
    /// more non-retired facts — dream job targets these for dedup.
    pub fn list_subject_groups_with_duplicates(&self, account_id: &str) -> Result<Vec<(String, String)>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT subject_kind, subject_key FROM memory_facts
             WHERE account_id = ?1 AND status != 'retired'
             GROUP BY subject_kind, subject_key
             HAVING COUNT(*) > 1",
        )?;
        let rows = stmt
            .query_map(params![account_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
}

/// Convert f32 embedding to raw little-endian bytes for sqlite-vec.
/// Duplicated from `db/embeddings.rs` to avoid cross-module coupling.
fn fact_embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &v in embedding {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MemoryFact, PendingTask, ThreadState};

    fn new_fact(id: &str, account: &str, kind: &str, key: &str, text: &str) -> MemoryFact {
        let now = 1_700_000_000;
        MemoryFact {
            id: id.to_string(),
            account_id: account.to_string(),
            subject_kind: kind.to_string(),
            subject_key: key.to_string(),
            fact: text.to_string(),
            source: "extraction".to_string(),
            source_email_id: None,
            confidence: 0.7,
            score: 0.0,
            status: "candidate".to_string(),
            last_used_at: None,
            domain: None,
            vigency: None,
            company: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn insert_email(db: &Database, id: &str, timestamp: i64) {
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES ('a1', 'gmail', 'a1', 'Test', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails (
                    id, account_id, thread_id, subject, sender, sender_email,
                    sender_domain, recipients_json, cc_json, snippet,
                    timestamp, is_read, is_deleted, category, mailbox, raw_json, created_at
                ) VALUES (?1, 'a1', 'th1', 'Subject', 'Alice', 'alice@ex.com',
                    'ex.com', '[]', '[]', '', ?2, 0, 0, 'primary', 'inbox', NULL, ?2)",
            rusqlite::params![id, timestamp],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO email_bodies (email_id, body) VALUES (?1, '')",
            rusqlite::params![id],
        )
        .unwrap();
    }

    #[test]
    fn extraction_markers_are_independent_for_memory_and_tasks() {
        let db = Database::new_for_testing().unwrap();
        insert_email(&db, "e1", 100);
        insert_email(&db, "e2", 200);

        db.mark_memory_facts_extracted("e1", 1_000).unwrap();

        let memory_ids = db
            .get_memory_unextracted_email_ids("a1", 10, &["primary".to_string()], None)
            .unwrap();
        let task_ids = db
            .get_task_unextracted_email_ids("a1", 10, &["primary".to_string()], None)
            .unwrap();

        assert_eq!(memory_ids, vec!["e2"]);
        assert_eq!(task_ids, vec!["e2", "e1"]);

        db.mark_tasks_extracted("e2", 2_000).unwrap();
        assert_eq!(db.count_memory_unextracted_emails("a1", &[], None).unwrap(), 1);
        assert_eq!(db.count_task_unextracted_emails("a1", &[], None).unwrap(), 1);

        db.reset_memory_extraction("a1").unwrap();
        assert_eq!(db.count_memory_unextracted_emails("a1", &[], None).unwrap(), 2);
        assert_eq!(
            db.count_task_unextracted_emails("a1", &[], None).unwrap(),
            1,
            "resetting memory must not reset the independent task marker",
        );
    }

    #[test]
    fn insert_and_query_facts_by_subject() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("a1");
        db.insert_memory_fact(&new_fact(
            "f1",
            "a1",
            "contact",
            "alice@ex.com",
            "Alice handles billing",
        ))
        .unwrap();
        db.insert_memory_fact(&new_fact(
            "f2",
            "a1",
            "contact",
            "alice@ex.com",
            "Alice prefers morning calls",
        ))
        .unwrap();
        db.insert_memory_fact(&new_fact("f3", "a1", "contact", "bob@ex.com", "Bob is slow to reply"))
            .unwrap();
        let alice = db.get_memory_facts_by_subject("a1", "contact", "alice@ex.com").unwrap();
        assert_eq!(alice.len(), 2);
        let bob = db.get_memory_facts_by_subject("a1", "contact", "bob@ex.com").unwrap();
        assert_eq!(bob.len(), 1);
    }

    #[test]
    fn fts_finds_facts_by_content() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("a1");
        db.insert_memory_fact(&new_fact(
            "f1",
            "a1",
            "contact",
            "alice@ex.com",
            "Alice handles billing at BigCo",
        ))
        .unwrap();
        db.insert_memory_fact(&new_fact(
            "f2",
            "a1",
            "contact",
            "bob@ex.com",
            "Bob is our lead engineer",
        ))
        .unwrap();
        let hits = db.search_memory_facts_fts("a1", "billing", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0.id, "f1");
    }

    #[test]
    fn fts_ignores_retired_facts() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("a1");
        db.insert_memory_fact(&new_fact(
            "f1",
            "a1",
            "contact",
            "alice@ex.com",
            "Alice handles billing",
        ))
        .unwrap();
        db.set_memory_fact_status("f1", "retired", 1_700_000_100).unwrap();
        let hits = db.search_memory_facts_fts("a1", "billing", 5).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn touch_thread_state_creates_and_updates() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("a1");
        db.touch_thread_state("a1", "t1", Some("them"), Some(1000), 1000)
            .unwrap();
        let s = db.get_thread_state("a1", "t1").unwrap().unwrap();
        assert_eq!(s.awaiting, "them");
        assert_eq!(s.last_outbound_at, Some(1000));

        // Second touch only updates last_touched_at; awaiting stays because
        // we pass None.
        db.touch_thread_state("a1", "t1", None, None, 2000).unwrap();
        let s2 = db.get_thread_state("a1", "t1").unwrap().unwrap();
        assert_eq!(s2.awaiting, "them");
        assert_eq!(s2.last_touched_at, 2000);
    }

    #[test]
    fn list_pending_tasks_orders_by_priority_then_due() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("a1");
        let now = 1_700_000_000;
        let mk = |id: &str, priority: &str, due: Option<i64>| PendingTask {
            id: id.to_string(),
            account_id: "a1".to_string(),
            title: format!("task {id}"),
            detail: None,
            source: "extracted".to_string(),
            source_email_id: None,
            source_thread_id: None,
            assignee: "me".to_string(),
            status: "open".to_string(),
            priority: priority.to_string(),
            due_at: due,
            completed_at: None,
            company: None,
            created_at: now,
            updated_at: now,
        };
        db.insert_pending_task(&mk("a", "low", Some(100))).unwrap();
        db.insert_pending_task(&mk("b", "high", Some(200))).unwrap();
        db.insert_pending_task(&mk("c", "normal", None)).unwrap();
        db.insert_pending_task(&mk("d", "high", None)).unwrap();
        let tasks = db.list_pending_tasks("a1", None, None, 10).unwrap();
        let ids: Vec<_> = tasks.iter().map(|t| t.id.as_str()).collect();
        // high+dated first, then high+undated, then normal+undated, then low+dated.
        assert_eq!(ids, vec!["b", "d", "c", "a"]);
    }

    #[test]
    fn count_pending_tasks_splits_overdue_and_today() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("a1");
        let now = chrono::Utc::now().timestamp();
        let mk = |id: &str, due: i64| PendingTask {
            id: id.to_string(),
            account_id: "a1".to_string(),
            title: id.to_string(),
            detail: None,
            source: "extracted".to_string(),
            source_email_id: None,
            source_thread_id: None,
            assignee: "me".to_string(),
            status: "open".to_string(),
            priority: "normal".to_string(),
            due_at: Some(due),
            completed_at: None,
            company: None,
            created_at: now,
            updated_at: now,
        };
        db.insert_pending_task(&mk("past", now - 3600)).unwrap();
        db.insert_pending_task(&mk("today", now + 3600)).unwrap();
        db.insert_pending_task(&mk("later", now + 86_400 * 3)).unwrap();
        let (total, overdue, today) = db.count_pending_tasks("a1").unwrap();
        assert_eq!(total, 3);
        assert_eq!(overdue, 1);
        assert_eq!(today, 1);
    }

    #[test]
    fn interaction_events_roundtrip_and_prune() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("a1");
        db.log_interaction_event("a1", "read", Some("e1"), Some("t1"), None)
            .unwrap();
        db.log_interaction_event("a1", "reply", Some("e2"), Some("t2"), Some("{\"to\":\"x\"}"))
            .unwrap();
        let events = db.recent_interaction_events("a1", 10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "reply"); // newest first

        // Prune with a future cutoff -> everything deleted.
        let n = db
            .prune_interaction_events(chrono::Utc::now().timestamp() + 10)
            .unwrap();
        assert_eq!(n, 2);
        assert!(db.recent_interaction_events("a1", 10).unwrap().is_empty());
    }

    #[test]
    fn thread_state_upsert_preserves_fields() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("a1");
        let full = ThreadState {
            account_id: "a1".to_string(),
            thread_id: "t1".to_string(),
            awaiting: "user".to_string(),
            last_inbound_at: Some(100),
            last_outbound_at: Some(50),
            last_touched_at: 100,
            summary: Some("Invoice discussion".to_string()),
            commitment: Some("Send invoice by Friday".to_string()),
            deadline_at: Some(9999),
            participants: vec!["alice@ex.com".to_string(), "me@ex.com".to_string()],
            updated_at: 100,
        };
        db.upsert_thread_state(&full).unwrap();

        // Touch (partial update) — summary/commitment/deadline must survive.
        db.touch_thread_state("a1", "t1", Some("them"), Some(200), 200).unwrap();
        let s = db.get_thread_state("a1", "t1").unwrap().unwrap();
        assert_eq!(s.summary.as_deref(), Some("Invoice discussion"));
        assert_eq!(s.commitment.as_deref(), Some("Send invoice by Friday"));
        assert_eq!(s.deadline_at, Some(9999));
        assert_eq!(s.awaiting, "them");
        assert_eq!(s.last_outbound_at, Some(200));
    }
}
