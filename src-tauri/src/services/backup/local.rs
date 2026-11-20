//! The local data copy: a small SQLite file holding only the tables in
//! [`BACKED_UP`], and restoring one into the live database.
//!
//! A copy keeps each table's own `CREATE TABLE` statement and a `backup_meta`
//! table with the schema version it was made at, so a copy from an older
//! version restores the columns both share, and one from a newer version is
//! refused.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use super::tables::{BackedUp, BACKED_UP};
use crate::db::Database;
use crate::models::error::{AppError, Result};

pub const PREFIX: &str = "local-";
const TEMP: &str = ".local-backup.tmp";

/// Copies kept: each one is small, and a week of them leaves a copy from
/// before a problem the user only notices days later.
pub const KEEP: usize = 7;

/// Write the backed-up tables to `dir/local-<UTC time>.db` and keep the newest
/// [`KEEP`] copies. The rows are read in one snapshot from a connection of its
/// own, so the app keeps writing meanwhile.
pub fn write_copy(db: &Database, dir: &Path, now: DateTime<Utc>) -> Result<PathBuf> {
    if db.db_path().as_os_str().is_empty() {
        return Err(AppError::InvalidInput(
            "an in-memory database has no file to copy from".into(),
        ));
    }
    std::fs::create_dir_all(dir)
        .map_err(|e| AppError::IoError(format!("Could not create the local backups folder: {e}")))?;
    let temp = dir.join(TEMP);
    remove_temp(&temp)?;
    if let Err(e) = copy_tables(db.db_path(), &temp) {
        if let Err(cleanup) = remove_temp(&temp) {
            crate::services::logger::log(
                "warn",
                "system",
                format!("Could not remove a partial local backup: {cleanup}"),
            );
        }
        return Err(e);
    }
    let dest = dir.join(format!("{PREFIX}{}.db", now.format("%Y%m%d_%H%M%S")));
    std::fs::rename(&temp, &dest).map_err(|e| AppError::IoError(format!("Could not name the local backup: {e}")))?;
    super::prune(dir, PREFIX, KEEP)?;
    Ok(dest)
}

fn copy_tables(live: &Path, dest: &Path) -> Result<()> {
    let mut conn = Connection::open(dest)?;
    // The copy keeps each table's foreign keys, and most point at tables it
    // leaves out (`emails`, `accounts` rows of another kind): enforcing them
    // here would refuse every row.
    conn.execute_batch("PRAGMA foreign_keys = OFF")?;
    conn.execute("ATTACH DATABASE ?1 AS live", params![live.to_string_lossy()])?;
    let tx = conn.transaction()?;
    for table in BACKED_UP {
        let create: String = tx.query_row(
            "SELECT sql FROM live.sqlite_master WHERE type = 'table' AND name = ?1",
            params![table.table],
            |r| r.get(0),
        )?;
        tx.execute_batch(&create)?;
        let columns = column_list(&tx, "live", table.table)?;
        tx.execute(
            &format!(
                "INSERT INTO main.\"{name}\" ({columns}) SELECT {columns} FROM live.\"{name}\"{filter}",
                name = table.table,
                filter = where_clause(table),
            ),
            [],
        )?;
    }
    let schema_version: i64 = tx.query_row("SELECT MAX(version) FROM live.refinery_schema_history", [], |r| {
        r.get(0)
    })?;
    tx.execute_batch("CREATE TABLE backup_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)")?;
    tx.execute(
        "INSERT INTO backup_meta (key, value) VALUES ('schema_version', ?1), ('app_version', ?2)",
        params![schema_version.to_string(), env!("CARGO_PKG_VERSION")],
    )?;
    tx.commit()?;
    conn.execute("DETACH DATABASE live", [])?;
    Ok(())
}

/// Write every row of the copy at `file` over the live one with the same key,
/// leaving rows the copy does not have alone, then refresh the search indexes
/// those rows feed. Returns how many rows were written.
///
/// Refused when the copy comes from a newer schema, and when an account in it
/// was added again since under a new id: its mail is downloaded under that
/// new id, so the copy's rows would point at nothing.
pub fn restore(db: &Database, file: &Path) -> Result<usize> {
    let (rows, doc_ids) = {
        let conn = db.connection();
        conn.execute("ATTACH DATABASE ?1 AS bk", params![file.to_string_lossy()])?;
        let restored = restore_attached(&conn);
        let detached = conn.execute("DETACH DATABASE bk", []);
        let restored = restored?;
        detached?;
        restored
    };
    // The rows are in by now; a document whose content cannot be read stays
    // out of search rather than failing a restore that already happened.
    for doc_id in doc_ids {
        if let Err(e) = crate::services::shared_docs::reindex(db, &doc_id) {
            crate::services::logger::log(
                "warn",
                "system",
                format!("A restored shared document could not be added to search: {e}"),
            );
        }
    }
    Ok(rows)
}

fn restore_attached(conn: &Connection) -> Result<(usize, Vec<String>)> {
    let not_a_copy = || AppError::InvalidInput("This file is not an EmailOps local data backup".into());
    if !table_exists(conn, "bk", "backup_meta")? {
        return Err(not_a_copy());
    }
    let copy_version: i64 = conn
        .query_row(
            "SELECT value FROM bk.backup_meta WHERE key = 'schema_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|v| v.parse().ok())
        .ok_or_else(not_a_copy)?;
    let live_version: i64 = conn.query_row("SELECT MAX(version) FROM main.refinery_schema_history", [], |r| {
        r.get(0)
    })?;
    if copy_version > live_version {
        return Err(AppError::InvalidInput(
            "This backup was made by a newer version of EmailOps; update the app to restore it".into(),
        ));
    }
    let clash: Option<String> = conn
        .query_row(
            "SELECT b.email FROM bk.accounts b JOIN main.accounts m ON m.email = b.email AND m.id <> b.id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(address) = clash {
        return Err(AppError::InvalidInput(format!(
            "Remove the account {address} before restoring: it was added again after this backup was made"
        )));
    }

    let foreign_keys: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
    // Off for the restore: a restored row may point at mail that is downloaded
    // again only after it, and REPLACE must not cascade into the live rows.
    conn.execute_batch("PRAGMA foreign_keys = OFF")?;
    let written = write_rows(conn);
    conn.execute_batch(&format!("PRAGMA foreign_keys = {foreign_keys}"))?;
    written
}

fn write_rows(conn: &Connection) -> Result<(usize, Vec<String>)> {
    let tx = conn.unchecked_transaction()?;
    let mut rows = 0;
    for table in BACKED_UP {
        if !table_exists(&tx, "bk", table.table)? {
            continue;
        }
        let columns = shared_column_list(&tx, table.table)?;
        rows += tx.execute(
            &format!(
                "INSERT OR REPLACE INTO main.\"{name}\" ({columns}) SELECT {columns} FROM bk.\"{name}\"{filter}",
                name = table.table,
                filter = where_clause(table),
            ),
            [],
        )?;
    }
    // Memory search is a plain FTS table the code fills on insert.
    tx.execute_batch(
        "DELETE FROM memory_facts_fts;
         INSERT INTO memory_facts_fts (fact_id, fact, subject_key) SELECT id, fact, subject_key FROM memory_facts;",
    )?;
    let doc_ids = if table_exists(&tx, "bk", "shared_docs")? {
        let mut stmt = tx.prepare("SELECT id FROM bk.shared_docs")?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        ids
    } else {
        Vec::new()
    };
    tx.commit()?;
    Ok((rows, doc_ids))
}

fn where_clause(table: &BackedUp) -> String {
    table.only.map(|only| format!(" WHERE {only}")).unwrap_or_default()
}

fn table_exists(conn: &Connection, schema: &str, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            &format!("SELECT 1 FROM {schema}.sqlite_master WHERE type = 'table' AND name = ?1"),
            params![table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn columns(conn: &Connection, schema: &str, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("SELECT name FROM {schema}.pragma_table_info(?1)"))?;
    let names = stmt
        .query_map(params![table], |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(names)
}

fn quoted(names: &[String]) -> String {
    names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ")
}

fn column_list(conn: &Connection, schema: &str, table: &str) -> Result<String> {
    Ok(quoted(&columns(conn, schema, table)?))
}

/// The columns the live table and the copy's table both have: a copy from an
/// older version lacks the columns added since, which keep their defaults.
fn shared_column_list(conn: &Connection, table: &str) -> Result<String> {
    let in_copy = columns(conn, "bk", table)?;
    let shared: Vec<String> = columns(conn, "main", table)?
        .into_iter()
        .filter(|c| in_copy.contains(c))
        .collect();
    Ok(quoted(&shared))
}

/// Remove a temp file and the journals SQLite may have left beside it.
fn remove_temp(temp: &Path) -> Result<()> {
    for suffix in ["", "-journal", "-wal", "-shm"] {
        let mut path = temp.as_os_str().to_owned();
        path.push(suffix);
        match std::fs::remove_file(PathBuf::from(path)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(AppError::IoError(format!(
                    "Could not remove a partial local backup: {e}"
                )))
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_db(dir: &Path) -> Database {
        Database::new(dir.to_path_buf()).expect("open db")
    }

    /// Seed values the generic rule below cannot guess: a check that ties two
    /// columns together. A new table whose seed fails a check gets its entry
    /// here.
    const SEED_OVERRIDES: &[(&str, &str, &str)] = &[("outbox", "reply_to_email_id", "NULL")];

    /// One synthetic row in every backed-up table, whatever its columns: a
    /// column limited by `CHECK (col IN (…))` gets the first allowed value,
    /// any other a value of its declared type. Foreign keys are off for the
    /// seed, so the rows need no parents.
    fn seed_every_backed_up_table(db: &Database) {
        let conn = db.connection();
        conn.execute_batch("PRAGMA foreign_keys = OFF")
            .expect("relax foreign keys");
        for table in BACKED_UP {
            let create: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table.table],
                    |r| r.get(0),
                )
                .expect("create statement");
            let mut stmt = conn
                .prepare("SELECT name, upper(type) FROM pragma_table_info(?1)")
                .expect("prepare");
            let cols: Vec<(String, String)> = stmt
                .query_map(params![table.table], |r| Ok((r.get(0)?, r.get(1)?)))
                .expect("columns")
                .collect::<std::result::Result<_, _>>()
                .expect("columns");
            let names = cols
                .iter()
                .map(|(n, _)| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let values = cols
                .iter()
                .map(|(name, ty)| seed_value(table.table, &create, name, ty))
                .collect::<Vec<_>>()
                .join(", ");
            conn.execute(
                &format!("INSERT INTO \"{}\" ({names}) VALUES ({values})", table.table),
                [],
            )
            .unwrap_or_else(|e| panic!("seed {}: {e} (add a SEED_OVERRIDES entry)", table.table));
        }
        conn.execute_batch("PRAGMA foreign_keys = ON")
            .expect("restore foreign keys");
    }

    fn seed_value(table: &str, create: &str, column: &str, ty: &str) -> String {
        if let Some((_, _, value)) = SEED_OVERRIDES.iter().find(|(t, c, _)| *t == table && *c == column) {
            return (*value).to_string();
        }
        let allowed = regex::Regex::new(&format!(r"\b{column}\s+IN\s*\(\s*('[^']*'|-?\d+)")).expect("valid regex");
        if let Some(first) = allowed.captures(create).and_then(|c| c.get(1)) {
            return first.as_str().to_string();
        }
        if ty.contains("INT") {
            "1".into()
        } else if ty.contains("REAL") || ty.contains("FLOA") || ty.contains("DOUB") {
            "1.5".into()
        } else if ty.contains("BLOB") {
            "x'00'".into()
        } else {
            "'x'".into()
        }
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |r| r.get(0))
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    fn filtered_count(conn: &Connection, table: &BackedUp) -> i64 {
        conn.query_row(
            &format!("SELECT COUNT(*) FROM \"{}\"{}", table.table, where_clause(table)),
            [],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("count {}: {e}", table.table))
    }

    fn copies(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(PREFIX))
            .collect();
        names.sort();
        names
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("valid time")
    }

    #[test]
    fn every_backed_up_table_reaches_the_copy() {
        let data = tempfile::tempdir().expect("data dir");
        let db = file_db(data.path());
        seed_every_backed_up_table(&db);
        let dir = data.path().join("local-backups");

        let copy = write_copy(&db, &dir, at(1_800_000_000)).expect("write copy");

        let copied = Connection::open(&copy).expect("open copy");
        let live = db.reader();
        for table in BACKED_UP {
            let in_copy = count(&copied, table.table);
            assert!(in_copy >= 1, "{} did not reach the copy", table.table);
            assert_eq!(in_copy, filtered_count(&live, table), "{} lost rows", table.table);
        }
    }

    #[test]
    fn the_copy_holds_nothing_from_the_tables_left_out() {
        let data = tempfile::tempdir().expect("data dir");
        let db = file_db(data.path());

        let copy = write_copy(&db, &data.path().join("local-backups"), at(1_800_000_000)).expect("write copy");

        let conn = Connection::open(&copy).expect("open copy");
        for (table, _) in super::super::tables::NOT_BACKED_UP {
            assert!(
                !table_exists(&conn, "main", table).expect("look up"),
                "{table} is in the copy"
            );
        }
    }

    #[test]
    fn secret_preferences_and_detector_scores_stay_out_of_the_copy() {
        let data = tempfile::tempdir().expect("data dir");
        let db = file_db(data.path());
        db.set_preference("theme", "dark").expect("pref");
        db.set_preference(crate::services::ai::OPENROUTER_DEV_KEY_PREF, "sk-placeholder")
            .expect("pref");
        db.set_preference(crate::services::password::FAILED_ATTEMPTS_KEY, "2")
            .expect("pref");
        {
            let conn = db.connection();
            conn.execute_batch(
                "PRAGMA foreign_keys = OFF;
                 INSERT INTO email_junk (email_id, account_id, spam_score, phish_score, gray_score, band,
                     primary_kind, reasons_json, method, model_version, scored_at, user_override)
                 VALUES ('acct::1', 'acct', 0.9, 0, 0, 'junk', 'spam', '[]', 'statistical', 1, 1, 'not_junk'),
                        ('acct::2', 'acct', 0.9, 0, 0, 'junk', 'spam', '[]', 'statistical', 1, 1, NULL);
                 PRAGMA foreign_keys = ON;",
            )
            .expect("seed junk");
        }

        let copy = write_copy(&db, &data.path().join("local-backups"), at(1_800_000_000)).expect("write copy");

        let conn = Connection::open(&copy).expect("open copy");
        let keys: Vec<String> = conn
            .prepare("SELECT key FROM user_preferences ORDER BY key")
            .expect("prepare")
            .query_map([], |r| r.get(0))
            .expect("keys")
            .collect::<std::result::Result<_, _>>()
            .expect("keys");
        assert!(keys.contains(&"theme".to_string()));
        assert!(!keys.contains(&crate::services::ai::OPENROUTER_DEV_KEY_PREF.to_string()));
        assert!(!keys.contains(&crate::services::password::FAILED_ATTEMPTS_KEY.to_string()));
        let junk: Vec<String> = conn
            .prepare("SELECT email_id FROM email_junk")
            .expect("prepare")
            .query_map([], |r| r.get(0))
            .expect("rows")
            .collect::<std::result::Result<_, _>>()
            .expect("rows");
        assert_eq!(junk, vec!["acct::1".to_string()]);
    }

    #[test]
    fn a_copy_keeps_the_newest_seven() {
        let data = tempfile::tempdir().expect("data dir");
        let db = file_db(data.path());
        let dir = data.path().join("local-backups");

        for day in 0..9 {
            write_copy(&db, &dir, at(1_800_000_000 + day * 86_400)).expect("write copy");
        }

        let names = copies(&dir);
        assert_eq!(names.len(), KEEP);
        assert_eq!(names.first().map(String::as_str), Some("local-20270117_080000.db"));
    }

    #[test]
    fn restoring_brings_every_backed_up_table_back_into_an_empty_install() {
        let source_dir = tempfile::tempdir().expect("source dir");
        let source = file_db(source_dir.path());
        seed_every_backed_up_table(&source);
        let copy = write_copy(&source, &source_dir.path().join("local-backups"), at(1_800_000_000)).expect("copy");
        let target_dir = tempfile::tempdir().expect("target dir");
        let target = file_db(target_dir.path());

        let rows = restore(&target, &copy).expect("restore");

        let check = Connection::open(target.db_path()).expect("open target");
        check
            .execute("ATTACH DATABASE ?1 AS bk", params![copy.to_string_lossy()])
            .expect("attach copy");
        let mut in_copy = 0;
        for table in BACKED_UP {
            in_copy += check
                .query_row(&format!("SELECT COUNT(*) FROM bk.\"{}\"", table.table), [], |r| {
                    r.get::<_, i64>(0)
                })
                .expect("count copy");
            let columns = shared_column_list(&check, table.table).expect("columns");
            let missing: i64 = check
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM (SELECT {columns} FROM bk.\"{t}\" EXCEPT SELECT {columns} FROM main.\"{t}\")",
                        t = table.table
                    ),
                    [],
                    |r| r.get(0),
                )
                .expect("compare");
            assert_eq!(missing, 0, "{} was not restored", table.table);
        }
        assert_eq!(i64::try_from(rows).expect("row count"), in_copy);
    }

    #[test]
    fn restoring_keeps_rows_the_copy_does_not_have_and_overwrites_the_ones_it_has() {
        let data = tempfile::tempdir().expect("data dir");
        let db = file_db(data.path());
        db.set_preference("theme", "dark").expect("pref");
        let copy = write_copy(&db, &data.path().join("local-backups"), at(1_800_000_000)).expect("copy");
        db.set_preference("theme", "light").expect("pref");
        db.set_preference("language", "es").expect("pref");

        restore(&db, &copy).expect("restore");

        assert_eq!(db.get_preference("theme").expect("read").as_deref(), Some("dark"));
        assert_eq!(db.get_preference("language").expect("read").as_deref(), Some("es"));
    }

    #[test]
    fn restored_memory_is_searchable() {
        let source_dir = tempfile::tempdir().expect("source dir");
        let source = file_db(source_dir.path());
        source
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 INSERT INTO memory_facts (id, account_id, subject_kind, subject_key, fact, source, confidence,
                     score, status, created_at, updated_at)
                 VALUES ('f1', 'acct', 'contact', 'nadia@example.com', 'Prefers calls on Tuesdays', 'extracted',
                     0.9, 1.0, 'active', 1, 1);
                 PRAGMA foreign_keys = ON;",
            )
            .expect("seed fact");
        let copy = write_copy(&source, &source_dir.path().join("local-backups"), at(1_800_000_000)).expect("copy");
        let target_dir = tempfile::tempdir().expect("target dir");
        let target = file_db(target_dir.path());

        restore(&target, &copy).expect("restore");

        let hits: i64 = target
            .reader()
            .query_row(
                "SELECT COUNT(*) FROM memory_facts_fts WHERE memory_facts_fts MATCH 'tuesdays'",
                [],
                |r| r.get(0),
            )
            .expect("search");
        assert_eq!(hits, 1);
    }

    #[test]
    fn a_copy_from_a_newer_version_is_refused() {
        let data = tempfile::tempdir().expect("data dir");
        let db = file_db(data.path());
        let copy = write_copy(&db, &data.path().join("local-backups"), at(1_800_000_000)).expect("copy");
        Connection::open(&copy)
            .expect("open copy")
            .execute("UPDATE backup_meta SET value = '9999' WHERE key = 'schema_version'", [])
            .expect("age the copy forward");

        let err = restore(&db, &copy).expect_err("refused");

        assert!(err.to_string().contains("newer version"), "{err}");
    }

    #[test]
    fn a_file_that_is_not_a_copy_is_refused() {
        let data = tempfile::tempdir().expect("data dir");
        let db = file_db(data.path());
        let other = data.path().join("other.db");
        Connection::open(&other)
            .expect("open")
            .execute_batch("CREATE TABLE t (x)")
            .expect("create");

        let err = restore(&db, &other).expect_err("refused");

        assert!(err.to_string().contains("not an EmailOps local data backup"), "{err}");
    }

    #[test]
    fn an_account_added_again_under_a_new_id_blocks_the_restore() {
        let data = tempfile::tempdir().expect("data dir");
        let db = file_db(data.path());
        let insert_account = |id: &str| {
            db.connection()
                .execute(
                    "INSERT INTO accounts (id, provider, email, name, created_at) \
                     VALUES (?1, 'imap', 'nadia@example.com', 'Nadia', 1)",
                    params![id],
                )
                .expect("account");
        };
        insert_account("old-id");
        let copy = write_copy(&db, &data.path().join("local-backups"), at(1_800_000_000)).expect("copy");
        db.connection()
            .execute("DELETE FROM accounts WHERE id = 'old-id'", [])
            .expect("remove");
        insert_account("new-id");

        let err = restore(&db, &copy).expect_err("refused");

        assert!(err.to_string().contains("nadia@example.com"), "{err}");
    }

    #[test]
    fn restoring_leaves_foreign_keys_as_they_were() {
        let data = tempfile::tempdir().expect("data dir");
        let db = file_db(data.path());
        let copy = write_copy(&db, &data.path().join("local-backups"), at(1_800_000_000)).expect("copy");

        restore(&db, &copy).expect("restore");

        let foreign_keys: i64 = db
            .connection()
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .expect("pragma");
        assert_eq!(foreign_keys, 1);
    }
}
