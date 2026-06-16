use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::models::error::{AppError, Result};

use rusqlite::ffi::sqlite3_auto_extension;

mod embedded {
    refinery::embed_migrations!("migrations");
}

pub mod accounts;
pub mod attachments;
pub mod chat;
pub mod drafts;
pub mod emails;
pub mod embeddings;
pub mod filters;
pub mod lenses;
pub mod memory;
pub mod tags;
pub mod trusted_senders;

/// Number of read connections in the pool. SQLite WAL mode supports unlimited
/// concurrent readers; 4 is enough to keep the UI responsive while background
/// sync and filter stats queries run in parallel.
const READ_POOL_SIZE: usize = 4;

/// Whether `Database::new` prints its per-step `[db-init]` startup timings to
/// stderr. The timings are dev-only (the `timed!` macro is
/// `#[cfg(debug_assertions)]`). Defaults to on so desktop dev builds keep them;
/// the CLI flips it off unless a `--trace` command asked for diagnostics — see
/// `cli::startup_timing_enabled`.
#[cfg(debug_assertions)]
static DB_INIT_TIMING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Enable/disable the `[db-init]` startup-timing stream (see [`DB_INIT_TIMING`]).
/// Must be called before [`Database::new`]. A no-op in release builds, where the
/// timings are compiled out entirely.
pub fn set_db_init_timing(enabled: bool) {
    #[cfg(debug_assertions)]
    DB_INIT_TIMING.store(enabled, std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(debug_assertions))]
    let _ = enabled;
}

pub struct Database {
    db_path: PathBuf,
    write_conn: Mutex<Connection>,
    /// Pool of read connections so concurrent reads don't serialize behind a
    /// single Mutex. Each connection is independently locked; `reader()` picks
    /// the first idle one via `try_lock`, falling back to a blocking lock on
    /// the first connection if the entire pool is busy.
    /// Empty in test mode (in-memory DBs can't share connections).
    read_conns: Vec<Mutex<Connection>>,
}

impl Database {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)
            .map_err(|_| AppError::DbError(rusqlite::Error::InvalidPath(data_dir.clone())))?;

        // Register sqlite-vec extension before opening connection
        unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *mut i8,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> i32,
            >(sqlite_vec::sqlite3_vec_init as *const ())));
        }

        let db_path = data_dir.join("emailops.db");
        let write_conn = Connection::open(&db_path)?;

        let mut read_conns = Vec::with_capacity(READ_POOL_SIZE);
        for _ in 0..READ_POOL_SIZE {
            read_conns.push(Mutex::new(Connection::open(&db_path)?));
        }

        let db = Self {
            db_path: db_path.clone(),
            write_conn: Mutex::new(write_conn),
            read_conns,
        };

        macro_rules! timed {
            ($label:expr, $expr:expr) => {{
                let _t = std::time::Instant::now();
                let _r = $expr;
                // Startup timing runs before the logger seam is installed, so it
                // can only go to stderr — gate it to debug builds, and let the
                // CLI silence it (DB_INIT_TIMING) unless `--trace` is set.
                #[cfg(debug_assertions)]
                if DB_INIT_TIMING.load(std::sync::atomic::Ordering::Relaxed) {
                    eprintln!(
                        "[db-init] [{:.0}ms] {}",
                        _t.elapsed().as_secs_f64() * 1000.0,
                        $label
                    );
                }
                _r
            }};
        }

        timed!("configure_connection", db.configure_connection()?);
        timed!("configure_read_connection", db.configure_read_connection()?);
        timed!("run_migrations", db.run_migrations()?);
        timed!("ensure_virtual_tables", db.ensure_virtual_tables()?);
        timed!("populate_fts_if_empty", db.populate_fts_if_empty()?);
        // Integrity check removed — WAL mode is crash-safe and the check was
        // taking 100+ seconds on large databases, dominating startup time.
        // Corruption will surface as a DB error on the affected query.

        Ok(db)
    }

    fn configure_connection(&self) -> Result<()> {
        let conn = self.connection();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             -- NORMAL is safe against corruption in WAL mode; only risks losing the very
             -- last committed transaction on an OS crash (acceptable for a desktop app).
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             -- 32 MB page cache (negative = kibibytes). Reduces disk I/O for large mailboxes.
             PRAGMA cache_size = -32000;
             -- Memory-map up to 256 MB of the DB file so reads bypass the kernel page cache.
             PRAGMA mmap_size = 268435456;
             -- INCREMENTAL auto-vacuum tracks free pages so we can reclaim disk space
             -- after retention sweeps without running a full VACUUM (which rewrites the
             -- entire 6 GB file and locks writers). NOTE: this PRAGMA only takes effect
             -- on a *fresh* DB (the page-tracking metadata must exist before tables are
             -- created). On databases created with the previous default (auto_vacuum=NONE)
             -- the setting is a no-op until a one-time `VACUUM;` flips the mode.
             PRAGMA auto_vacuum = INCREMENTAL;
             -- Bound the WAL by triggering a PASSIVE checkpoint roughly every 1 000 pages
             -- (~4 MB). PASSIVE checkpoints don't truncate the file — they just reset the
             -- write offset — but they keep the WAL from growing unboundedly during long
             -- sync batches. `checkpoint_wal_truncate()` (below) handles the truncate side.
             PRAGMA wal_autocheckpoint = 1000;",
        )?;
        Ok(())
    }

    /// Run `PRAGMA wal_checkpoint(TRUNCATE)`.
    ///
    /// After a sync batch lands a few MB of new rows, the WAL file holds onto
    /// those pages until SQLite checkpoints them back into the main DB. The
    /// default PASSIVE auto-checkpoint resets the WAL's write offset but does
    /// NOT shrink the file on disk — so after a long initial backfill, the
    /// WAL can sit at hundreds of MB until the process exits. TRUNCATE both
    /// checkpoints AND truncates the WAL file to 0 bytes, reclaiming the
    /// disk space immediately.
    ///
    /// Returns silently on contention (`SQLITE_LOCKED`); callers should treat
    /// this as best-effort and never panic on failure.
    pub fn checkpoint_wal_truncate(&self) -> Result<()> {
        let conn = self.connection();
        // `PRAGMA wal_checkpoint(TRUNCATE)` returns a 3-row result; we don't
        // care about its values, only that the call doesn't error. Use
        // execute_batch so we don't have to bind to the result.
        match conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
            Ok(_) => Ok(()),
            // SQLITE_BUSY / SQLITE_LOCKED: another writer holds the WAL lock.
            // The next checkpoint will catch up; do not surface as an error.
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::DatabaseBusy || e.code == rusqlite::ErrorCode::DatabaseLocked =>
            {
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Run `PRAGMA incremental_vacuum(pages)`.
    ///
    /// Only useful when `auto_vacuum=INCREMENTAL` is in effect (see
    /// `configure_connection`). On a DB still in `auto_vacuum=NONE` mode this
    /// is a no-op — no error is raised, the call just doesn't free anything.
    ///
    /// `pages == 0` means "free all available free pages"; positive values
    /// bound how much work a single call does so a periodic background
    /// caller doesn't lock writers for long.
    pub fn incremental_vacuum_pages(&self, pages: u32) -> Result<()> {
        let conn = self.connection();
        let sql = if pages == 0 {
            "PRAGMA incremental_vacuum;".to_string()
        } else {
            format!("PRAGMA incremental_vacuum({});", pages)
        };
        match conn.execute_batch(&sql) {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::DatabaseBusy || e.code == rusqlite::ErrorCode::DatabaseLocked =>
            {
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    fn configure_read_connection(&self) -> Result<()> {
        // WAL mode is persistent at the file level; set FK enforcement defensively in case
        // a future code path accidentally routes a write through the read connection.
        for rc in &self.read_conns {
            let conn = rc.lock().unwrap_or_else(PoisonError::into_inner);
            conn.execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA busy_timeout = 5000;
                 PRAGMA cache_size = -32000;
                 PRAGMA mmap_size = 268435456;",
            )?;
        }
        Ok(())
    }

    /// Re-create the vec0 virtual tables (`vec_emails`, `vec_memory_facts`)
    /// idempotently. Required because the demo bootstrap copies prod schema
    /// but skips vec0 tables (sqlite-vec isn't loaded in stock Python sqlite3)
    /// and also copies `refinery_schema_history`, which makes refinery skip
    /// V001 on subsequent opens. Without this step the demo DB starts up
    /// missing both vec0 tables and any embedding write fails.
    fn ensure_virtual_tables(&self) -> Result<()> {
        let conn = self.connection();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vec_emails USING vec0(
                 embedding float[768] distance_metric=cosine
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS vec_memory_facts USING vec0(
                 embedding float[768] distance_metric=cosine
             );",
        )?;
        Ok(())
    }

    /// Run all pending versioned SQL migrations in `src-tauri/migrations/`.
    ///
    /// Migrations are embedded at compile time via `refinery::embed_migrations!`
    /// so no filesystem access is needed at runtime. Applied migrations are
    /// recorded in `refinery_schema_history`; subsequent calls are no-ops for
    /// already-applied versions.
    ///
    /// All statements in each migration file use `IF NOT EXISTS`, making them
    /// idempotent on databases that pre-date refinery — V001 running on an
    /// existing developer database skips tables that already exist.
    fn run_migrations(&self) -> Result<()> {
        let mut conn = self.connection();
        embedded::migrations::runner()
            .run(&mut *conn)
            .map_err(|e| AppError::IoError(format!("DB migration failed: {e}")))?;
        Ok(())
    }

    /// Returns the write connection. Use for all INSERT / UPDATE / DELETE / DDL.
    pub fn connection(&self) -> MutexGuard<'_, Connection> {
        self.write_conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns an idle read connection from the pool. Tries each connection
    /// without blocking (`try_lock`); if all are busy, blocks on the first one.
    /// Falls back to the write connection in test mode (empty pool).
    pub fn reader(&self) -> MutexGuard<'_, Connection> {
        // Fast path: grab the first idle connection without blocking
        for rc in &self.read_conns {
            if let Ok(guard) = rc.try_lock() {
                return guard;
            }
        }
        // All busy — block on the first read connection (or write conn in tests)
        if let Some(first) = self.read_conns.first() {
            first.lock().unwrap_or_else(PoisonError::into_inner)
        } else {
            self.write_conn.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }

    /// Stream every email row, strip HTML, and insert into emails_fts — all
    /// inside a single transaction with prepared statements.  At 47k emails
    /// on a 6 GB DB this finishes in ~3 s instead of minutes of individual
    /// auto-commits, and never materialises the full mailbox in memory.
    ///
    /// Assumes emails_fts has already been cleared (or is empty) by the caller.
    pub(crate) fn populate_fts_from_emails(&self) -> Result<u32> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let mut count: u32 = 0;
        {
            let mut read_stmt = tx.prepare(
                "SELECT e.id, e.subject, e.sender, COALESCE(b.body, '')
                 FROM emails e LEFT JOIN email_bodies b ON b.email_id = e.id",
            )?;
            let mut insert_stmt =
                tx.prepare("INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1, ?2, ?3, ?4)")?;
            let mut rows = read_stmt.query([])?;
            while let Some(row) = rows.next()? {
                let id: String = row.get(0)?;
                let subject: String = row.get(1)?;
                let sender: String = row.get(2)?;
                let body: String = row.get(3)?;
                let body_text = crate::util::html::strip_html_for_fts(&body);
                insert_stmt.execute(rusqlite::params![id, subject, sender, body_text])?;
                count += 1;
            }
        }
        tx.commit()?;
        Ok(count)
    }

    /// Populate FTS index from emails table (called after schema init if FTS is empty).
    pub fn populate_fts_if_empty(&self) -> Result<u32> {
        // EXISTS + LIMIT 1 returns instantly; COUNT(*) on FTS5 does a full scan.
        let has_rows: bool =
            self.connection()
                .query_row("SELECT EXISTS(SELECT 1 FROM emails_fts LIMIT 1)", [], |row| row.get(0))?;
        if has_rows {
            return Ok(0);
        }
        self.populate_fts_from_emails()
    }

    /// Path to the on-disk SQLite file. Empty for in-memory test databases.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Copy the live database to `backup_dir/emailops-{timestamp}.db` using
    /// the SQLite Online Backup API.  Uses a dedicated short-lived connection so
    /// neither the write nor read mutex is held during the (potentially slow) copy.
    /// After a successful backup the directory is pruned to keep only the newest
    /// `keep` files.
    pub fn backup(&self, backup_dir: &Path, keep: usize) -> Result<PathBuf> {
        if self.db_path.as_os_str().is_empty() {
            // In-memory / test DB — nothing to back up.
            return Err(crate::models::error::AppError::InvalidInput(
                "backup not supported for in-memory databases".into(),
            ));
        }
        std::fs::create_dir_all(backup_dir).map_err(|e| crate::models::error::AppError::IoError(e.to_string()))?;

        // Bound the directory *before* copying. A previous run that failed
        // (e.g. a full disk) never reached the post-copy prune below, so without
        // this an unbroken string of failures accumulates partial backups
        // forever — exactly the ~1900-file pile seen in production.
        Self::prune_backups(backup_dir, keep);

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let dest_path = backup_dir.join(format!("emailops-{timestamp}.db"));
        // Copy into a temp file whose name does NOT match the `emailops-` prefix,
        // so a partial copy is never mistaken for — or retained as — a real
        // backup. Promote to the final name only once the copy is complete.
        let tmp_path = backup_dir.join(".emailops-backup.tmp");
        Self::remove_backup_temp(&tmp_path);

        let copy = (|| -> Result<()> {
            // Open a fresh source connection — avoids holding either mutex during backup.
            let src = Connection::open(&self.db_path)?;
            let mut dst = Connection::open(&tmp_path)?;
            let backup = rusqlite::backup::Backup::new(&src, &mut dst)?;
            // step(-1) copies the entire DB in one go; for large DBs consider stepping
            // in smaller increments to allow periodic progress events.
            backup.run_to_completion(5, std::time::Duration::from_millis(250), None)?;
            Ok(())
        })();

        if let Err(e) = copy {
            // Delete the partial copy so failures don't accumulate on disk.
            Self::remove_backup_temp(&tmp_path);
            return Err(e);
        }

        std::fs::rename(&tmp_path, &dest_path).map_err(|e| crate::models::error::AppError::IoError(e.to_string()))?;
        Self::prune_backups(backup_dir, keep);
        Ok(dest_path)
    }

    /// Remove a backup temp file plus any SQLite sidecar journals it may have
    /// left behind. Best-effort — a missing file (or a directory occupying the
    /// path in tests) is fine.
    fn remove_backup_temp(tmp_path: &Path) {
        let _ = std::fs::remove_file(tmp_path);
        for suffix in ["-journal", "-wal", "-shm"] {
            let mut sidecar = tmp_path.as_os_str().to_owned();
            sidecar.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(sidecar));
        }
    }

    fn prune_backups(backup_dir: &Path, keep: usize) {
        let Ok(entries) = std::fs::read_dir(backup_dir) else {
            return;
        };
        let mut files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("emailops-"))
            .collect();
        // Sort newest first by file name (timestamp suffix is lexicographically sortable).
        #[allow(clippy::unnecessary_sort_by)]
        files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        for old in files.into_iter().skip(keep) {
            let _ = std::fs::remove_file(old.path());
        }
    }

    pub fn get_preference(&self, key: &str) -> Result<Option<String>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT value FROM user_preferences WHERE key = ?1",
            rusqlite::params![key],
            |row| row.get(0),
        );
        match result {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_preference(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// Master AI enable/disable flag. When `false`, every AI-powered code path
    /// (chat, classification, embeddings, memory/task extraction, AI search)
    /// must short-circuit before doing real work. Stored as a string in
    /// `user_preferences` so it can share the existing key-value plumbing.
    ///
    /// Default: enabled. A missing row preserves prior behaviour for users
    /// upgrading from a release without this flag.
    pub fn is_ai_enabled(&self) -> Result<bool> {
        Ok(self
            .get_preference("ai_enabled")?
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(true))
    }

    /// Per-feature gate for the AI Memory extraction pipeline and the chat's
    /// memory tools (`memory_search`, `recall_entity`, `remember`). Mirrors the
    /// `useMemoryEnabledStore` default in `src/stores/featureToggleStore.ts` —
    /// off by default; users opt in from Settings.
    pub fn is_memory_enabled(&self) -> Result<bool> {
        Ok(self
            .get_preference("memory_enabled")?
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false))
    }

    /// Per-feature gate for the AI Tasks extraction pipeline and the chat's
    /// task tools (`list_pending_tasks`, `create_task`). Pref key is singular
    /// (`task_enabled`) to match the frontend store.
    pub fn is_tasks_enabled(&self) -> Result<bool> {
        Ok(self
            .get_preference("task_enabled")?
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false))
    }

    /// Per-feature gate for AI Lenses and the chat's lens-display tools.
    pub fn is_lenses_enabled(&self) -> Result<bool> {
        Ok(self
            .get_preference("lenses_enabled")?
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false))
    }

    /// Per-feature gate for AI draft generation and the chat's draft tools.
    /// Default `true` to match `AiDraftsSettings.tsx` and the pre-existing
    /// `commands::emails::generate_draft` gate parser.
    pub fn is_ai_drafts_enabled(&self) -> Result<bool> {
        Ok(self
            .get_preference("ai_drafts_enabled")?
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(true))
    }

    /// Maximum age (in days) of emails eligible for AI processing
    /// (embeddings + classification). Default: 365 days. A value of 0 means
    /// "no limit" — process every email regardless of age.
    ///
    /// Returns `Some(min_unix_seconds)` when a cutoff is configured, or
    /// `None` when there is no cutoff. Callers pass this through to the
    /// SQL fetch functions so old emails are never selected for AI work
    /// in the first place.
    pub fn ai_processing_min_timestamp(&self, now_unix_seconds: i64) -> Result<Option<i64>> {
        let days = self
            .get_preference("ai_max_email_age_days")?
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(365);
        if days <= 0 {
            return Ok(None);
        }
        Ok(Some(now_unix_seconds.saturating_sub(days.saturating_mul(86_400))))
    }

    /// Remove a stored preference. Idempotent — clearing an already-absent
    /// key is a no-op. Used by "reset to default" flows so subsequent reads
    /// fall back to the registry default rather than to a stale override.
    pub fn delete_preference(&self, key: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute("DELETE FROM user_preferences WHERE key = ?1", rusqlite::params![key])?;
        Ok(())
    }

    /// Create an in-memory database for unit tests.
    ///
    /// Runs the **same** refinery migrations as production (`migrations/V*.sql`),
    /// so every table, virtual table (FTS5, vec0), trigger, and index from prod
    /// is present in the test DB by construction. This eliminates the drift
    /// class of bugs where a test passes but the same code path fails in
    /// production because a table the test DB lacks.
    ///
    /// When adding new schema, create a new versioned migration file in
    /// `src-tauri/migrations/` — the test DB will pick it up automatically.
    pub fn new_for_testing() -> Result<Self> {
        // Ensure sqlite-vec is registered so `vec_emails` / `vec_memory_facts`
        // (vec0 virtual tables) are available. Matches `Database::new`.
        unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *mut i8,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> i32,
            >(sqlite_vec::sqlite3_vec_init as *const ())));
        }
        let conn = Connection::open_in_memory()?;
        let db = Self {
            db_path: PathBuf::new(),
            write_conn: Mutex::new(conn),
            read_conns: Vec::new(),
        };
        // FK enforcement is per-connection and must be set before any DDL that
        // declares foreign-key constraints. WAL / mmap / cache pragmas from
        // `configure_connection` don't apply to `:memory:` and are skipped.
        db.connection().execute_batch("PRAGMA foreign_keys = ON;")?;
        db.run_migrations()?;
        Ok(db)
    }

    /// Idempotent helper to satisfy the `accounts(id)` FK in tests. The
    /// production schema enforces every email/sync/filter row to point at a
    /// real account; older tests used to skip this because the hand-written
    /// test schema omitted FKs. Calling this once per `account_id` referenced
    /// by a test keeps the in-memory DB consistent with prod.
    #[cfg(test)]
    pub(crate) fn seed_test_account(&self, id: &str) {
        self.connection()
            .execute(
                "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?1, 'Test', 0)",
                rusqlite::params![id],
            )
            .expect("seed test account");
    }

    /// Open an existing database read-only for benchmarks / diagnostics.
    #[cfg(test)]
    pub fn open_readonly(db_path: PathBuf) -> Result<Self> {
        unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *mut i8,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> i32,
            >(sqlite_vec::sqlite3_vec_init as *const ())));
        }
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let write_conn = Connection::open_with_flags(&db_path, flags)?;
        let mut read_conns = Vec::with_capacity(READ_POOL_SIZE);
        for _ in 0..READ_POOL_SIZE {
            let rc = Connection::open_with_flags(&db_path, flags)?;
            rc.execute_batch("PRAGMA cache_size = -32000; PRAGMA mmap_size = 268435456;")?;
            read_conns.push(Mutex::new(rc));
        }
        Ok(Self {
            db_path,
            write_conn: Mutex::new(write_conn),
            read_conns,
        })
    }
}

#[cfg(test)]
mod ai_enabled_tests {
    use super::*;

    #[test]
    fn missing_pref_defaults_to_enabled() {
        // Upgrading users (no `ai_enabled` row yet) must keep the historical
        // behaviour — AI on. Verifies the `unwrap_or(true)` branch.
        let db = Database::new_for_testing().expect("create test db");
        assert!(db.is_ai_enabled().expect("read pref"));
    }

    #[test]
    fn explicit_true_is_enabled() {
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("ai_enabled", "true").expect("write pref");
        assert!(db.is_ai_enabled().expect("read pref"));
    }

    #[test]
    fn explicit_false_is_disabled() {
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("ai_enabled", "false").expect("write pref");
        assert!(!db.is_ai_enabled().expect("read pref"));
    }

    #[test]
    fn case_insensitive_true() {
        // Frontend writes lowercase but tolerate other casings to avoid a
        // silent disable if someone hand-edits the DB.
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("ai_enabled", "TRUE").expect("write pref");
        assert!(db.is_ai_enabled().expect("read pref"));
    }
}

#[cfg(test)]
mod backup_tests {
    //! Regression coverage for the backup retention bug: `backup()` used to
    //! prune *only* after a successful copy, so any failing backup (e.g. a full
    //! disk) left a partial `emailops-*.db` file that was never cleaned up. In
    //! production this accumulated ~1900 partial files. The contract now is:
    //! copy into a non-`emailops-` temp file, rename on success, delete the
    //! partial on failure, and bound the directory on every call.
    use super::*;
    use std::fs;

    fn touch(path: &std::path::Path) {
        fs::write(path, b"x").expect("write fixture backup");
    }

    fn backup_count(dir: &std::path::Path) -> usize {
        fs::read_dir(dir)
            .expect("read backup dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("emailops-"))
            .count()
    }

    fn seed_stale_backups(dir: &std::path::Path, n: usize) {
        for i in 0..n {
            touch(&dir.join(format!("emailops-2020010{i:01}_00000{i:01}.db")));
        }
    }

    #[test]
    fn successful_backup_prunes_to_keep_and_leaves_no_temp() {
        let data_dir = tempfile::tempdir().expect("data dir");
        let db = Database::new(data_dir.path().to_path_buf()).expect("open db");
        let backup_dir = tempfile::tempdir().expect("backup dir");
        seed_stale_backups(backup_dir.path(), 6);

        let out = db.backup(backup_dir.path(), 3).expect("backup should succeed");

        assert!(out.exists(), "the new backup file must exist");
        assert_eq!(backup_count(backup_dir.path()), 3, "directory must be pruned to `keep`");
        assert!(
            !backup_dir.path().join(".emailops-backup.tmp").exists(),
            "no temp file should be left behind after a successful backup"
        );
    }

    #[test]
    fn failed_backup_is_surfaced_and_leaves_no_partial() {
        let data_dir = tempfile::tempdir().expect("data dir");
        let db = Database::new(data_dir.path().to_path_buf()).expect("open db");
        let backup_dir = tempfile::tempdir().expect("backup dir");
        // Force the copy to fail without making the directory read-only (which
        // would also break pruning). Occupying the temp path with a *directory*
        // makes the destination connection fail to open, while the backup dir
        // itself stays writable — modelling a copy that fails mid-flight.
        fs::create_dir(backup_dir.path().join(".emailops-backup.tmp")).expect("occupy temp path");
        seed_stale_backups(backup_dir.path(), 6);

        let result = db.backup(backup_dir.path(), 3);

        assert!(
            result.is_err(),
            "a failed copy must be surfaced as an error, not a bogus backup"
        );
        // Even though the copy failed, the directory must stay bounded so a
        // string of failures cannot accumulate unbounded partial files.
        assert!(
            backup_count(backup_dir.path()) <= 3,
            "failed backups must still bound the directory to `keep`"
        );
    }
}

#[cfg(test)]
mod feature_flag_tests {
    //! Per-feature gates the chat tool registry consults to decide which tools
    //! to advertise. Defaults match `src/stores/featureToggleStore.ts` (memory /
    //! task / lenses default `false` — opt-in experimental features) and
    //! `src/components/Settings/AiDraftsSettings.tsx` (drafts default `true`).
    //! Frontend and backend must agree on defaults so a fresh install behaves
    //! the same whether the chat or the Settings UI reads the pref first.
    use super::*;

    #[test]
    fn is_memory_enabled_defaults_false_when_unset() {
        let db = Database::new_for_testing().expect("create test db");
        assert!(!db.is_memory_enabled().expect("read pref"));
    }

    #[test]
    fn is_memory_enabled_true_when_pref_true() {
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("memory_enabled", "true").expect("write pref");
        assert!(db.is_memory_enabled().expect("read pref"));
    }

    #[test]
    fn is_memory_enabled_false_when_pref_false() {
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("memory_enabled", "false").expect("write pref");
        assert!(!db.is_memory_enabled().expect("read pref"));
    }

    #[test]
    fn is_memory_enabled_case_insensitive() {
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("memory_enabled", "TRUE").expect("write pref");
        assert!(db.is_memory_enabled().expect("read pref"));
    }

    #[test]
    fn is_tasks_enabled_defaults_false_when_unset() {
        let db = Database::new_for_testing().expect("create test db");
        assert!(!db.is_tasks_enabled().expect("read pref"));
    }

    #[test]
    fn is_tasks_enabled_reads_singular_task_enabled_key() {
        // The pref key is `task_enabled` (singular) — that's what the frontend
        // `useTasksEnabledStore` writes. A previous version of this plan
        // proposed `tasks_enabled`; assert the singular key wins so the helper
        // and the Settings UI stay in lockstep.
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("task_enabled", "true").expect("write pref");
        assert!(db.is_tasks_enabled().expect("read pref"));
        // The plural key should be ignored.
        db.set_preference("tasks_enabled", "false").expect("write pref");
        assert!(db.is_tasks_enabled().expect("read pref"));
    }

    #[test]
    fn is_tasks_enabled_false_when_pref_false() {
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("task_enabled", "false").expect("write pref");
        assert!(!db.is_tasks_enabled().expect("read pref"));
    }

    #[test]
    fn is_lenses_enabled_defaults_false_when_unset() {
        let db = Database::new_for_testing().expect("create test db");
        assert!(!db.is_lenses_enabled().expect("read pref"));
    }

    #[test]
    fn is_lenses_enabled_true_when_pref_true() {
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("lenses_enabled", "true").expect("write pref");
        assert!(db.is_lenses_enabled().expect("read pref"));
    }

    #[test]
    fn is_lenses_enabled_false_when_pref_false() {
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("lenses_enabled", "false").expect("write pref");
        assert!(!db.is_lenses_enabled().expect("read pref"));
    }

    #[test]
    fn is_ai_drafts_enabled_defaults_true_when_unset() {
        // Drafts is the odd one out — default on for fresh installs to match
        // the existing UI behaviour (`AiDraftsSettings.tsx` initialises
        // `enabled = true` before the pref load resolves).
        let db = Database::new_for_testing().expect("create test db");
        assert!(db.is_ai_drafts_enabled().expect("read pref"));
    }

    #[test]
    fn is_ai_drafts_enabled_false_when_pref_false() {
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("ai_drafts_enabled", "false").expect("write pref");
        assert!(!db.is_ai_drafts_enabled().expect("read pref"));
    }

    #[test]
    fn is_ai_drafts_enabled_true_when_pref_true() {
        let db = Database::new_for_testing().expect("create test db");
        db.set_preference("ai_drafts_enabled", "true").expect("write pref");
        assert!(db.is_ai_drafts_enabled().expect("read pref"));
    }
}

#[cfg(test)]
mod schema_parity_tests {
    //! Guards against test-vs-prod schema drift. `Database::new_for_testing()`
    //! runs the same refinery migrations as production, so any table / virtual
    //! table / trigger / index the production code relies on must show up in
    //! the in-memory DB too. The set asserted here is a load-bearing subset —
    //! the ones whose absence has historically caused tests to silently pass
    //! while production was broken (`emails_fts`, `vec_emails`, FK targets).
    use super::*;

    fn names_of_kind(db: &Database, kind: &str) -> Vec<String> {
        let conn = db.connection();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = ?1")
            .expect("prepare sqlite_master query");
        let rows: Vec<String> = stmt
            .query_map([kind], |row| row.get::<_, String>(0))
            .expect("query sqlite_master")
            .map(|r| r.expect("read name"))
            .collect();
        rows
    }

    #[test]
    fn test_db_has_critical_tables() {
        let db = Database::new_for_testing().expect("create test db");
        let tables = names_of_kind(&db, "table");
        for required in [
            "accounts",
            "emails",
            "email_bodies",
            "email_extraction_status",
            "email_tags",
            "drafts",
            "sync_state",
            "user_preferences",
            "memory_facts",
            "thread_states",
            "interaction_events",
            "pending_tasks",
            "embedding_chunks",
            "smart_filter_suggestions",
        ] {
            assert!(
                tables.iter().any(|t| t == required),
                "test DB missing required table `{required}`. Tables present: {tables:?}"
            );
        }
    }

    #[test]
    fn test_db_has_critical_virtual_tables() {
        // FTS5 + vec0 virtual tables must exist; without them search / vector
        // queries silently return zero rows and tests pass for the wrong reason.
        let db = Database::new_for_testing().expect("create test db");
        let tables = names_of_kind(&db, "table");
        for required in ["emails_fts", "memory_facts_fts", "vec_emails", "vec_memory_facts"] {
            assert!(
                tables.iter().any(|t| t == required),
                "test DB missing required virtual table `{required}`. Tables present: {tables:?}"
            );
        }
    }

    #[test]
    fn test_db_has_critical_triggers() {
        let db = Database::new_for_testing().expect("create test db");
        let triggers = names_of_kind(&db, "trigger");
        for required in ["emails_fts_delete", "memory_facts_fts_delete"] {
            assert!(
                triggers.iter().any(|t| t == required),
                "test DB missing trigger `{required}`. Triggers present: {triggers:?}"
            );
        }
    }

    #[test]
    fn test_db_enforces_foreign_keys() {
        // The hand-written test schema used to silently drop FK constraints,
        // hiding bugs where prod refused to insert rows that the test happily
        // accepted. Assert here so we never regress.
        let db = Database::new_for_testing().expect("create test db");
        let on: i64 = db
            .connection()
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .expect("read pragma");
        assert_eq!(on, 1, "PRAGMA foreign_keys must be ON in test DB");
    }

    #[test]
    fn ensure_virtual_tables_recreates_dropped_vec_tables() {
        // Regression test for the demo-DB embed pipeline: the demo bootstrap
        // (scripts/generate_demo_db.py) copies prod schema but skips vec0
        // virtual tables AND copies refinery_schema_history, so refinery
        // treats V001 as applied and never re-creates `vec_emails` /
        // `vec_memory_facts`. `Database::new` must therefore re-assert them
        // idempotently on every startup.
        let db = Database::new_for_testing().expect("create test db");
        db.connection()
            .execute_batch("DROP TABLE vec_emails; DROP TABLE vec_memory_facts;")
            .expect("drop vec tables");
        let before = names_of_kind(&db, "table");
        assert!(!before.iter().any(|t| t == "vec_emails"));
        assert!(!before.iter().any(|t| t == "vec_memory_facts"));

        db.ensure_virtual_tables().expect("ensure virtual tables");

        let after = names_of_kind(&db, "table");
        assert!(after.iter().any(|t| t == "vec_emails"));
        assert!(after.iter().any(|t| t == "vec_memory_facts"));
    }
}
