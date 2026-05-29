//! CRUD operations for the Lenses feature.
//!
//! Writes go through `connection()`. Reads use `reader()` so concurrent
//! Lens queries don't serialize behind the write mutex.

use rusqlite::{params, OptionalExtension};

use crate::models::error::{AppError, Result};
use crate::models::lens::{
    CreateLensInput, Lens, LensRow, LensRowsPage, LensRunKind, LensSchema, LensScope, LensStatus, LensSummary,
    SortSpec, UpdateLensInput,
};

use super::Database;

/// (run_id, kind, processed, total, succeeded, failed)
pub type LensRunProgress = (String, String, i64, i64, i64, i64);

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn row_to_lens(row: &rusqlite::Row<'_>) -> rusqlite::Result<Lens> {
    let scope_json: String = row.get(5)?;
    let schema_json: String = row.get(6)?;
    let scope: LensScope = serde_json::from_str(&scope_json)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e)))?;
    let schema: LensSchema = serde_json::from_str(&schema_json)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e)))?;
    Ok(Lens {
        id: row.get(0)?,
        name: row.get(1)?,
        icon: row.get(2)?,
        template_key: row.get(3)?,
        account_id: row.get(4)?,
        scope,
        schema,
        prompt_text: row.get(7)?,
        prompt_version: row.get(8)?,
        model_provider: row.get(9)?,
        model_name: row.get(10)?,
        is_enabled: row.get::<_, i64>(11)? != 0,
        sort_order: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

const LENS_COLS: &str = "id, name, icon, template_key, account_id, scope_json, schema_json, \
    prompt_text, prompt_version, model_provider, model_name, is_enabled, sort_order, created_at, updated_at";

impl Database {
    // ── Lens CRUD ──────────────────────────────────────────────────────────

    pub fn list_lenses(&self) -> Result<Vec<LensSummary>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT l.id, l.name, l.icon, l.template_key, l.account_id, l.is_enabled, \
                    l.sort_order, l.prompt_version, \
                    (SELECT COUNT(*) FROM lens_rows r WHERE r.lens_id = l.id AND r.status = 'ok') AS row_count \
             FROM lenses l \
             ORDER BY l.sort_order ASC, l.created_at ASC",
        )?;
        let items = stmt
            .query_map([], |row| {
                Ok(LensSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    icon: row.get(2)?,
                    template_key: row.get(3)?,
                    account_id: row.get(4)?,
                    is_enabled: row.get::<_, i64>(5)? != 0,
                    sort_order: row.get(6)?,
                    prompt_version: row.get(7)?,
                    row_count: row.get(8)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(items)
    }

    pub fn get_lens(&self, lens_id: &str) -> Result<Lens> {
        let conn = self.reader();
        let sql = format!("SELECT {LENS_COLS} FROM lenses WHERE id = ?1");
        let mut stmt = conn.prepare(&sql)?;
        let lens = stmt
            .query_row(params![lens_id], row_to_lens)
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("Lens {lens_id} not found")))?;
        Ok(lens)
    }

    pub fn create_lens(&self, input: &CreateLensInput) -> Result<Lens> {
        let conn = self.connection();
        let now = now_secs();
        let id = uuid::Uuid::new_v4().to_string();
        let scope_json = serde_json::to_string(&input.scope)?;
        let schema_json = serde_json::to_string(&input.schema)?;

        // Append to the end by default.
        let next_order: i64 = conn
            .query_row("SELECT COALESCE(MAX(sort_order), 0) + 1 FROM lenses", [], |row| {
                row.get(0)
            })
            .unwrap_or(1);

        conn.execute(
            "INSERT INTO lenses (id, name, icon, template_key, account_id, scope_json, schema_json, \
                                 prompt_text, prompt_version, model_provider, model_name, \
                                 is_enabled, sort_order, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10, 1, ?11, ?12, ?12)",
            params![
                id,
                input.name,
                input.icon,
                input.template_key,
                input.account_id,
                scope_json,
                schema_json,
                input.prompt_text,
                input.model_provider,
                input.model_name,
                next_order,
                now,
            ],
        )?;
        drop(conn);
        self.get_lens(&id)
    }

    pub fn update_lens(&self, lens_id: &str, input: &UpdateLensInput) -> Result<Lens> {
        let existing = self.get_lens(lens_id)?;

        // Bump prompt_version when prompt or schema changes — drives the
        // "N rows need re-extraction" banner.
        let new_prompt = input.prompt_text.clone().unwrap_or(existing.prompt_text.clone());
        let new_schema = input.schema.clone().unwrap_or(existing.schema.clone());
        let prompt_changed = new_prompt != existing.prompt_text;
        let schema_changed = serde_json::to_string(&new_schema)? != serde_json::to_string(&existing.schema)?;
        let new_version = if prompt_changed || schema_changed {
            existing.prompt_version + 1
        } else {
            existing.prompt_version
        };

        let new_name = input.name.clone().unwrap_or(existing.name.clone());
        let new_icon = match &input.icon {
            Some(v) => Some(v.clone()),
            None => existing.icon.clone(),
        };
        let new_account_id = match &input.account_id {
            Some(opt) => opt.clone(),
            None => existing.account_id.clone(),
        };
        let new_scope = input.scope.clone().unwrap_or(existing.scope);
        let new_provider = match &input.model_provider {
            Some(opt) => opt.clone(),
            None => existing.model_provider.clone(),
        };
        let new_model = match &input.model_name {
            Some(opt) => opt.clone(),
            None => existing.model_name.clone(),
        };
        let new_enabled = input.is_enabled.unwrap_or(existing.is_enabled);
        let new_order = input.sort_order.unwrap_or(existing.sort_order);

        let scope_json = serde_json::to_string(&new_scope)?;
        let schema_json = serde_json::to_string(&new_schema)?;

        let conn = self.connection();
        conn.execute(
            "UPDATE lenses SET name = ?1, icon = ?2, account_id = ?3, scope_json = ?4, \
                                schema_json = ?5, prompt_text = ?6, prompt_version = ?7, \
                                model_provider = ?8, model_name = ?9, is_enabled = ?10, \
                                sort_order = ?11, updated_at = ?12 \
             WHERE id = ?13",
            params![
                new_name,
                new_icon,
                new_account_id,
                scope_json,
                schema_json,
                new_prompt,
                new_version,
                new_provider,
                new_model,
                new_enabled as i64,
                new_order,
                now_secs(),
                lens_id,
            ],
        )?;
        drop(conn);
        self.get_lens(lens_id)
    }

    pub fn delete_lens(&self, lens_id: &str) -> Result<()> {
        let mut conn = self.connection();
        // The production inline schema does not declare ON DELETE CASCADE on
        // the lens_* child tables, so we clean them up explicitly inside a
        // single transaction to keep state consistent if any step fails.
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM lens_rows WHERE lens_id = ?1", params![lens_id])?;
        tx.execute("DELETE FROM lens_exclusions WHERE lens_id = ?1", params![lens_id])?;
        tx.execute("DELETE FROM lens_runs WHERE lens_id = ?1", params![lens_id])?;
        let affected = tx.execute("DELETE FROM lenses WHERE id = ?1", params![lens_id])?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("Lens {lens_id} not found")));
        }
        tx.commit()?;
        Ok(())
    }

    // ── Rows ───────────────────────────────────────────────────────────────

    /// Upsert an extracted row. `overrides_json` on the existing row is preserved
    /// — this is the rule that lets users edit cells without fearing re-extraction.
    pub fn upsert_lens_row(
        &self,
        lens_id: &str,
        email_id: &str,
        account_id: &str,
        extracted_json: &str,
        prompt_version: i64,
        email_timestamp: i64,
        status: &str,
        error_message: Option<&str>,
    ) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO lens_rows (lens_id, email_id, account_id, extracted_json, overrides_json, \
                                    prompt_version, email_timestamp, extracted_at, status, error_message) \
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(lens_id, email_id) DO UPDATE SET \
                extracted_json = excluded.extracted_json, \
                prompt_version = excluded.prompt_version, \
                email_timestamp = excluded.email_timestamp, \
                extracted_at = excluded.extracted_at, \
                status = excluded.status, \
                error_message = excluded.error_message",
            params![
                lens_id,
                email_id,
                account_id,
                extracted_json,
                prompt_version,
                email_timestamp,
                now_secs(),
                status,
                error_message,
            ],
        )?;
        Ok(())
    }

    /// Returns rows for a Lens with overrides merged. Sort happens client-side
    /// over the page for v1 (PRD §7.1: extracted columns aren't indexed).
    /// When the schema has a column with `is_unique_key = true`, rows are
    /// deduplicated by that column's value — only the most recent email per
    /// unique value is returned (null/empty values each keep their own row).
    /// `total = -1` when not computed.
    pub fn get_lens_rows(
        &self,
        lens_id: &str,
        sort: Option<&SortSpec>,
        limit: i64,
        offset: i64,
    ) -> Result<LensRowsPage> {
        // Resolve unique-key column + schema columns from the lens schema.
        // The columns vec is used to whitelist `sort.key` so user-supplied input
        // can't be spliced into the ORDER BY clause, and to pick the right
        // numeric/text comparator for the sort key.
        use crate::models::lens::LensColumnType;
        let (unique_key_path, schema_cols): (Option<String>, Vec<(String, LensColumnType)>) = {
            let conn = self.reader();
            let schema_json: Option<String> = conn
                .query_row(
                    "SELECT schema_json FROM lenses WHERE id = ?1",
                    params![lens_id],
                    |row| row.get(0),
                )
                .optional()?;
            match schema_json.and_then(|s| serde_json::from_str::<crate::models::lens::LensSchema>(&s).ok()) {
                Some(schema) => {
                    let ukey = schema
                        .columns
                        .iter()
                        .find(|c| c.is_unique_key)
                        .map(|c| format!("$.{}", c.key));
                    let cols = schema.columns.into_iter().map(|c| (c.key, c.column_type)).collect();
                    (ukey, cols)
                }
                None => (None, Vec::new()),
            }
        };

        // Build the ORDER BY fragment. The column expression is one of:
        //   - `email_timestamp` (default / "emailTimestamp" key)
        //   - `json_extract(COALESCE(overrides, extracted), '$.<key>')` with an
        //     optional `CAST(... AS REAL)` so numeric / currency columns sort
        //     numerically instead of lexicographically ("93.12" vs "217.80").
        //   - For Currency columns, the path is `$.<key>.amount` (the value is
        //     stored as `{ amount: number, currency: string }`).
        // Direction is restricted to ASC/DESC (no parameterized direction in SQL).
        // Tiebreak by email_timestamp DESC so equal values are still deterministic.
        let (order_expr, tiebreak): (String, &str) = match sort {
            Some(s) if s.key == "emailTimestamp" || s.key == "email_timestamp" => {
                let dir = if s.desc { "DESC" } else { "ASC" };
                (format!("email_timestamp {dir}"), "")
            }
            Some(s) => match schema_cols.iter().find(|(k, _)| k == &s.key) {
                Some((_, col_type)) => {
                    let dir = if s.desc { "DESC" } else { "ASC" };
                    // `s.key` is whitelisted against the schema; safe to inline
                    // as a JSON path. COALESCE so an override sorts before the
                    // extracted value when both exist.
                    let path = match col_type {
                        LensColumnType::Currency => format!("$.{}.amount", s.key),
                        _ => format!("$.{}", s.key),
                    };
                    let raw = format!(
                        "COALESCE(json_extract(overrides_json, '{path}'), json_extract(extracted_json, '{path}'))",
                    );
                    let comparable = match col_type {
                        LensColumnType::Number | LensColumnType::Currency => {
                            format!("CAST({raw} AS REAL)")
                        }
                        LensColumnType::Boolean => format!("CAST({raw} AS INTEGER)"),
                        // String / Text / Date (ISO 8601) / Enum / Email / Url
                        // all compare correctly as text.
                        _ => raw,
                    };
                    (format!("{comparable} {dir}"), ", email_timestamp DESC")
                }
                None => ("email_timestamp DESC".to_string(), ""),
            },
            None => ("email_timestamp DESC".to_string(), ""),
        };

        let conn = self.reader();
        let limit = limit.clamp(1, 1000);

        let rows: Vec<LensRow> = if let Some(ref ukey_path) = unique_key_path {
            // Deduplicate by unique-key column value using ROW_NUMBER().
            // COALESCE(NULLIF(..., ''), email_id) keeps null/empty values as
            // separate rows rather than collapsing them all into one.
            let sql = format!(
                "WITH ranked AS ( \
                   SELECT r.lens_id, r.email_id, r.account_id, r.extracted_json, r.overrides_json, \
                          r.prompt_version, r.email_timestamp, r.extracted_at, r.status, r.error_message, \
                          e.subject, e.sender, e.sender_email, \
                          ROW_NUMBER() OVER ( \
                            PARTITION BY COALESCE(NULLIF(json_extract(r.extracted_json, ?4), ''), r.email_id) \
                            ORDER BY r.email_timestamp DESC \
                          ) AS rn \
                   FROM lens_rows r \
                   JOIN emails e ON e.id = r.email_id \
                   WHERE r.lens_id = ?1 AND r.status = 'ok' \
                 ) \
                 SELECT lens_id, email_id, account_id, extracted_json, overrides_json, \
                        prompt_version, email_timestamp, extracted_at, status, error_message, \
                        subject, sender, sender_email \
                 FROM ranked WHERE rn = 1 \
                 ORDER BY {order_expr}{tiebreak} \
                 LIMIT ?2 OFFSET ?3",
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![lens_id, limit, offset.max(0), ukey_path], map_lens_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        } else {
            let sql = format!(
                "SELECT r.lens_id, r.email_id, r.account_id, r.extracted_json, r.overrides_json, \
                        r.prompt_version, r.email_timestamp, r.extracted_at, r.status, r.error_message, \
                        e.subject, e.sender, e.sender_email \
                 FROM lens_rows r \
                 JOIN emails e ON e.id = r.email_id \
                 WHERE r.lens_id = ?1 AND r.status = 'ok' \
                 ORDER BY {order_expr}{tiebreak} \
                 LIMIT ?2 OFFSET ?3",
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![lens_id, limit, offset.max(0)], map_lens_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        Ok(LensRowsPage { rows, total: -1 })
    }

    /// `email_ids` whose `lens_rows.prompt_version < lens.prompt_version` —
    /// the set of rows that need re-extraction after a prompt/schema edit.
    pub fn list_stale_row_email_ids(&self, lens_id: &str) -> Result<Vec<String>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT r.email_id \
             FROM lens_rows r \
             JOIN lenses l ON l.id = r.lens_id \
             WHERE r.lens_id = ?1 AND r.prompt_version < l.prompt_version",
        )?;
        let ids = stmt
            .query_map(params![lens_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Count rows whose prompt_version is behind the Lens's current prompt_version.
    pub fn count_stale_rows(&self, lens_id: &str) -> Result<i64> {
        let conn = self.reader();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM lens_rows r \
             JOIN lenses l ON l.id = r.lens_id \
             WHERE r.lens_id = ?1 AND r.prompt_version < l.prompt_version",
            params![lens_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// True if an extracted row exists OR the email is in `lens_exclusions`.
    /// Used by `runner` to skip already-processed emails during backfill.
    pub fn lens_row_exists(&self, lens_id: &str, email_id: &str) -> Result<bool> {
        let conn = self.reader();
        let exists: bool = conn.query_row(
            "SELECT EXISTS( \
                SELECT 1 FROM lens_rows WHERE lens_id = ?1 AND email_id = ?2 \
                UNION ALL \
                SELECT 1 FROM lens_exclusions WHERE lens_id = ?1 AND email_id = ?2 \
             )",
            params![lens_id, email_id],
            |row| row.get(0),
        )?;
        Ok(exists)
    }

    /// Like `lens_row_exists` but ignores rows with `status = 'failed'`, so a
    /// subsequent backfill picks them back up. Returns `true` when there is a
    /// row in `ok` / `excluded` state, or a matching exclusion entry.
    pub fn lens_row_completed_or_excluded(&self, lens_id: &str, email_id: &str) -> Result<bool> {
        let conn = self.reader();
        let exists: bool = conn.query_row(
            "SELECT EXISTS( \
                SELECT 1 FROM lens_rows \
                  WHERE lens_id = ?1 AND email_id = ?2 AND status != 'failed' \
                UNION ALL \
                SELECT 1 FROM lens_exclusions WHERE lens_id = ?1 AND email_id = ?2 \
             )",
            params![lens_id, email_id],
            |row| row.get(0),
        )?;
        Ok(exists)
    }

    pub fn set_lens_row_override(&self, lens_id: &str, email_id: &str, overrides: &serde_json::Value) -> Result<()> {
        let conn = self.connection();
        let json = serde_json::to_string(overrides)?;
        let affected = conn.execute(
            "UPDATE lens_rows SET overrides_json = ?1 WHERE lens_id = ?2 AND email_id = ?3",
            params![json, lens_id, email_id],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("No lens row for ({lens_id}, {email_id})")));
        }
        Ok(())
    }

    pub fn add_lens_exclusion(&self, lens_id: &str, email_id: &str) -> Result<()> {
        let conn = self.connection();
        // Use a transaction so the exclusion + row-marking happen atomically.
        conn.execute(
            "INSERT OR REPLACE INTO lens_exclusions (lens_id, email_id, excluded_at) \
             VALUES (?1, ?2, ?3)",
            params![lens_id, email_id, now_secs()],
        )?;
        conn.execute(
            "UPDATE lens_rows SET status = 'excluded' WHERE lens_id = ?1 AND email_id = ?2",
            params![lens_id, email_id],
        )?;
        Ok(())
    }

    pub fn remove_lens_exclusion(&self, lens_id: &str, email_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM lens_exclusions WHERE lens_id = ?1 AND email_id = ?2",
            params![lens_id, email_id],
        )?;
        // Don't auto-revive the row's status — re-extraction will overwrite it.
        Ok(())
    }

    // ── Runs ───────────────────────────────────────────────────────────────

    /// Insert a new lens_runs row. `total` is the scope size known up-front
    /// (number of candidate emails) so the UI can show a progress bar without
    /// waiting for the run to finish; pass 0 when unknown.
    pub fn insert_lens_run(&self, lens_id: &str, kind: LensRunKind, total: i64) -> Result<String> {
        let conn = self.connection();
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO lens_runs (id, lens_id, kind, started_at, status, processed, total, succeeded, failed) \
             VALUES (?1, ?2, ?3, ?4, 'running', 0, ?5, 0, 0)",
            params![id, lens_id, kind.as_str(), now_secs(), total],
        )?;
        Ok(id)
    }

    pub fn update_lens_run_progress(&self, run_id: &str, processed: i64, succeeded: i64, failed: i64) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE lens_runs SET processed = ?1, succeeded = ?2, failed = ?3 WHERE id = ?4",
            params![processed, succeeded, failed, run_id],
        )?;
        Ok(())
    }

    pub fn finish_lens_run(&self, run_id: &str, status: &str, error_message: Option<&str>) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE lens_runs SET status = ?1, finished_at = ?2, error_message = ?3 WHERE id = ?4",
            params![status, now_secs(), error_message, run_id],
        )?;
        Ok(())
    }

    /// Mark any `lens_runs` still in `running` status as `failed`. Called at
    /// startup so a crashed/restarted app doesn't leave the UI showing a
    /// permanently-running Lens. Returns the number of rows recovered.
    pub fn reset_orphan_lens_runs(&self) -> Result<usize> {
        let conn = self.connection();
        let n = conn.execute(
            "UPDATE lens_runs SET status = 'failed', finished_at = ?1, \
                                  error_message = COALESCE(error_message, 'interrupted by app restart') \
             WHERE status = 'running'",
            params![now_secs()],
        )?;
        Ok(n)
    }

    /// One-time, idempotent schema patch: for built-in templates whose
    /// canonical schema acquired an `is_unique_key` column after lenses were
    /// already created (e.g. invoices_received / invoices_sent → invoice_number),
    /// ensure existing lenses derived from those templates have the flag set.
    ///
    /// Without this, users created from older builds keep seeing duplicates
    /// because dedup in `get_lens_rows` only kicks in when a column carries
    /// `is_unique_key = true`. We patch the JSON in place (no prompt_version
    /// bump) so previously-extracted rows stay valid — dedup happens at read
    /// time and doesn't require re-extraction.
    ///
    /// Returns the number of lenses whose schema_json was updated.
    pub fn migrate_template_unique_keys(&self) -> Result<usize> {
        // (template_key, column key that should be marked unique).
        let mappings: &[(&str, &str)] = &[
            ("invoices_received", "invoice_number"),
            ("invoices_sent", "invoice_number"),
        ];
        let conn = self.connection();
        let mut updated = 0usize;
        for (template_key, col_key) in mappings {
            let rows: Vec<(String, String)> = {
                let mut stmt = conn.prepare("SELECT id, schema_json FROM lenses WHERE template_key = ?1")?;
                let collected = stmt
                    .query_map(params![template_key], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                collected
            };
            for (id, schema_json) in rows {
                let mut schema: LensSchema = match serde_json::from_str(&schema_json) {
                    Ok(s) => s,
                    Err(_) => continue, // corrupt schema — leave it alone
                };
                let mut changed = false;
                for c in schema.columns.iter_mut() {
                    if c.key == *col_key && !c.is_unique_key {
                        c.is_unique_key = true;
                        changed = true;
                    }
                }
                if changed {
                    let new_json = serde_json::to_string(&schema)?;
                    conn.execute(
                        "UPDATE lenses SET schema_json = ?1, updated_at = ?2 WHERE id = ?3",
                        params![new_json, now_secs(), id],
                    )?;
                    updated += 1;
                }
            }
        }
        Ok(updated)
    }

    /// Idempotently add the `total` column to `lens_runs` on databases that
    /// were created before the column existed. New databases ship with it via
    /// `schema.rs`, so this is a no-op there.
    pub fn ensure_lens_runs_total_column(&self) -> Result<bool> {
        let conn = self.connection();
        let mut stmt = conn.prepare("PRAGMA table_info(lens_runs)")?;
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if cols.iter().any(|c| c == "total") {
            return Ok(false);
        }
        drop(stmt);
        conn.execute("ALTER TABLE lens_runs ADD COLUMN total INTEGER NOT NULL DEFAULT 0", [])?;
        Ok(true)
    }

    /// Force-mark any currently `running` run for a Lens as `cancelled`. Used
    /// by `cancel_lens_run` to recover from orphaned rows whose in-memory
    /// worker is no longer alive (so the cancel flag has no reader).
    pub fn force_cancel_running_lens_runs(&self, lens_id: &str) -> Result<usize> {
        let conn = self.connection();
        let n = conn.execute(
            "UPDATE lens_runs SET status = 'cancelled', finished_at = ?1, \
                                  error_message = COALESCE(error_message, 'cancelled by user (no live worker)') \
             WHERE lens_id = ?2 AND status = 'running'",
            params![now_secs(), lens_id],
        )?;
        Ok(n)
    }

    /// Current running run for a Lens, if any. Returns `(run_id, kind, processed, total, succeeded, failed)`.
    pub fn current_lens_run(&self, lens_id: &str) -> Result<Option<LensRunProgress>> {
        let conn = self.reader();
        let row = conn
            .query_row(
                "SELECT id, kind, processed, total, succeeded, failed FROM lens_runs \
                 WHERE lens_id = ?1 AND status = 'running' \
                 ORDER BY started_at DESC LIMIT 1",
                params![lens_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn last_lens_run_error(&self, lens_id: &str) -> Result<Option<String>> {
        let conn = self.reader();
        let row = conn
            .query_row(
                "SELECT error_message FROM lens_runs \
                 WHERE lens_id = ?1 AND status = 'failed' \
                 ORDER BY started_at DESC LIMIT 1",
                params![lens_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(row.flatten())
    }

    /// Most recent runs for a Lens (any status), newest first.
    pub fn list_lens_runs(&self, lens_id: &str, limit: i64) -> Result<Vec<crate::models::lens::LensRunHistoryEntry>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, kind, status, started_at, finished_at, processed, succeeded, failed, error_message \
             FROM lens_runs WHERE lens_id = ?1 ORDER BY started_at DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![lens_id, limit], |row| {
                Ok(crate::models::lens::LensRunHistoryEntry {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    status: row.get(2)?,
                    started_at: row.get(3)?,
                    finished_at: row.get(4)?,
                    processed: row.get(5)?,
                    succeeded: row.get(6)?,
                    failed: row.get(7)?,
                    error_message: row.get(8)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Compose a `LensStatus` snapshot for the frontend.
    pub fn get_lens_status(&self, lens_id: &str) -> Result<LensStatus> {
        let current = self.current_lens_run(lens_id)?;
        let pending = self.count_stale_rows(lens_id)?;
        let last_error = self.last_lens_run_error(lens_id)?;
        let (state, run_id, kind, processed, total, succeeded, failed) = match current {
            // `total` is the candidate count captured by the runner when the
            // run started — 0 means unknown (legacy rows) and the UI falls back
            // to "running…" without a progress bar.
            Some((id, k, p, t, s, f)) => ("running".to_string(), Some(id), Some(k), p, t, s, f),
            None => (
                if last_error.is_some() {
                    "error".to_string()
                } else {
                    "idle".to_string()
                },
                None,
                None,
                0,
                -1,
                0,
                0,
            ),
        };
        Ok(LensStatus {
            lens_id: lens_id.to_string(),
            state,
            current_run_id: run_id,
            current_run_kind: kind,
            processed,
            total,
            succeeded,
            failed,
            pending_reextract: pending,
            last_error,
        })
    }
}

/// Row mapper shared by both query paths in `get_lens_rows`.
/// Must be a free function (not a closure) so it has no captured lifetime and
/// can be passed to `query_map` without borrow-checker complaints.
fn map_lens_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LensRow> {
    let extracted: String = row.get(3)?;
    let overrides: Option<String> = row.get(4)?;
    let mut data: serde_json::Value = serde_json::from_str(&extracted)
        .ok()
        .filter(|v: &serde_json::Value| v.is_object())
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let mut has_overrides = false;
    if let Some(ov_str) = overrides.as_deref() {
        if let Ok(ov) = serde_json::from_str::<serde_json::Value>(ov_str) {
            merge_json(&mut data, &ov);
            has_overrides = !ov.is_null() && !matches!(&ov, serde_json::Value::Object(m) if m.is_empty());
        }
    }
    Ok(LensRow {
        lens_id: row.get(0)?,
        email_id: row.get(1)?,
        account_id: row.get(2)?,
        data,
        has_overrides,
        prompt_version: row.get(5)?,
        email_timestamp: row.get(6)?,
        extracted_at: row.get(7)?,
        status: row.get(8)?,
        error_message: row.get(9)?,
        email_subject: row.get(10)?,
        email_sender: row.get(11)?,
        email_sender_email: row.get(12)?,
    })
}

/// Deep-merge JSON object `patch` into `target` (overrides win). For arrays
/// and scalars, `patch` replaces `target` wholesale (consistent with how
/// users edit a cell — the new value is the value, not a merge).
fn merge_json(target: &mut serde_json::Value, patch: &serde_json::Value) {
    match (target, patch) {
        (serde_json::Value::Object(t), serde_json::Value::Object(p)) => {
            for (k, v) in p {
                merge_json(t.entry(k.clone()).or_insert(serde_json::Value::Null), v);
            }
        }
        (slot, val) => {
            *slot = val.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::lens::{CreateLensInput, LensColumn, LensColumnType, LensSchema, LensScope};

    fn sample_input() -> CreateLensInput {
        CreateLensInput {
            name: "Test Lens".into(),
            icon: None,
            template_key: None,
            account_id: None,
            scope: LensScope::default(),
            schema: LensSchema {
                columns: vec![LensColumn {
                    key: "vendor".into(),
                    label: "Vendor".into(),
                    column_type: LensColumnType::String,
                    description: "Who sent the invoice".into(),
                    enum_values: None,
                    required: false,
                    is_unique_key: false,
                }],
            },
            prompt_text: "Extract the vendor name.".into(),
            model_provider: None,
            model_name: None,
        }
    }

    #[test]
    fn create_and_get_lens_roundtrip() {
        let db = Database::new_for_testing().expect("test db");
        let lens = db.create_lens(&sample_input()).expect("create");
        assert_eq!(lens.name, "Test Lens");
        assert_eq!(lens.prompt_version, 1);
        assert!(lens.is_enabled);

        let fetched = db.get_lens(&lens.id).expect("get");
        assert_eq!(fetched.id, lens.id);
        assert_eq!(fetched.schema.columns.len(), 1);
        assert_eq!(fetched.schema.columns[0].key, "vendor");
    }

    #[test]
    fn update_lens_bumps_prompt_version_on_prompt_change() {
        let db = Database::new_for_testing().expect("test db");
        let lens = db.create_lens(&sample_input()).unwrap();
        let v1 = lens.prompt_version;

        let updated = db
            .update_lens(
                &lens.id,
                &UpdateLensInput {
                    prompt_text: Some("New prompt".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.prompt_version, v1 + 1);

        // Name-only edit must NOT bump prompt_version.
        let again = db
            .update_lens(
                &lens.id,
                &UpdateLensInput {
                    name: Some("Renamed".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(again.prompt_version, v1 + 1);
        assert_eq!(again.name, "Renamed");
    }

    #[test]
    fn delete_unknown_lens_returns_not_found() {
        let db = Database::new_for_testing().expect("test db");
        match db.delete_lens("nope") {
            Err(AppError::NotFound(_)) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    /// Regression: scope edits coming from the frontend's `LensScopeEditor`
    /// must round-trip through `update_lens` and survive a subsequent
    /// `get_lens`. Uses the exact JSON shape `buildScope()` emits — camelCase
    /// keys, `null` for cleared fields, ISO direction string — so that any
    /// serde/DB mistake is caught here instead of in the UI.
    #[test]
    fn update_lens_persists_scope_edits_from_frontend_payload() {
        let db = Database::new_for_testing().expect("test db");

        // Seed with the invoices template's wide default scope (lastDays=365,
        // no mailbox filter, etc).
        let initial = CreateLensInput {
            name: "Invoices received".into(),
            icon: Some("🧾".into()),
            template_key: Some("invoices_received".into()),
            account_id: None,
            scope: serde_json::from_value(serde_json::json!({
                "direction": "inbound",
                "dateRange": { "lastDays": 365 },
                "query": "invoice OR receipt"
            }))
            .unwrap(),
            schema: sample_input().schema,
            prompt_text: "Extract invoices.".into(),
            model_provider: None,
            model_name: None,
        };
        let lens = db.create_lens(&initial).expect("create");
        assert_eq!(lens.scope.date_range.as_ref().unwrap().last_days, Some(365));

        // Build the exact payload the UI emits when the user narrows the
        // scope to: inbox only, Primary, 5 days, one sender domain.
        let payload = serde_json::json!({
            "scope": {
                "accountIds": null,
                "mailboxes": ["inbox"],
                "categories": ["Primary"],
                "direction": "inbound",
                "query": "invoice OR receipt",
                "senderDomains": ["madrid.barcelo@impacthub.net"],
                "senderEmails": null,
                "dateRange": { "lastDays": 5 }
            }
        });
        let input: UpdateLensInput = serde_json::from_value(payload).expect("deserialize UpdateLensInput");

        let returned = db.update_lens(&lens.id, &input).expect("update");
        // The value the command would return to the frontend must reflect the
        // edits — the UI uses this to refresh `activeLens` without re-fetching.
        assert_eq!(returned.scope.date_range.as_ref().unwrap().last_days, Some(5));
        assert_eq!(returned.scope.mailboxes.as_deref(), Some(&["inbox".into()][..]));
        assert_eq!(returned.scope.categories.as_deref(), Some(&["Primary".into()][..]));
        assert_eq!(
            returned.scope.sender_domains.as_deref(),
            Some(&["madrid.barcelo@impacthub.net".into()][..])
        );
        // Scope-only edits MUST NOT bump prompt_version (existing extracted
        // rows should stay valid — only the next backfill pulls new emails).
        assert_eq!(returned.prompt_version, lens.prompt_version);

        // And a fresh `get_lens` must see the same values — proves we wrote to
        // the DB and didn't just mutate the returned struct.
        let fetched = db.get_lens(&lens.id).expect("get after update");
        assert_eq!(fetched.scope.date_range.as_ref().unwrap().last_days, Some(5));
        assert_eq!(fetched.scope.mailboxes.as_deref(), Some(&["inbox".into()][..]));
        assert_eq!(fetched.scope.categories.as_deref(), Some(&["Primary".into()][..]));
        assert_eq!(
            fetched.scope.sender_domains.as_deref(),
            Some(&["madrid.barcelo@impacthub.net".into()][..])
        );
        assert!(fetched.scope.sender_emails.is_none());
        assert!(fetched.scope.account_ids.is_none());

        // Now widen back: the user clears mailboxes and bumps the window to
        // 60 days. Empty arrays in the UI are sent as `null` (per
        // buildScope), so we mirror that here.
        let widen = serde_json::json!({
            "scope": {
                "accountIds": null,
                "mailboxes": null,
                "categories": null,
                "direction": "either",
                "query": null,
                "senderDomains": null,
                "senderEmails": null,
                "dateRange": { "lastDays": 60 }
            }
        });
        let input: UpdateLensInput = serde_json::from_value(widen).unwrap();
        db.update_lens(&lens.id, &input).expect("widen");
        let fetched = db.get_lens(&lens.id).expect("get after widen");
        assert!(fetched.scope.mailboxes.is_none(), "mailboxes should clear back to None");
        assert!(fetched.scope.categories.is_none());
        assert!(fetched.scope.sender_domains.is_none());
        assert!(fetched.scope.query.is_none());
        assert_eq!(fetched.scope.date_range.as_ref().unwrap().last_days, Some(60));
        // `direction: "either"` from the UI is sent literally — confirm it
        // round-trips so the editor's "Either" option survives a save.
        use crate::models::lens::Direction;
        assert!(matches!(fetched.scope.direction, Some(Direction::Either)));
    }
}
