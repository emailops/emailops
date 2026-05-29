use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::Connection;

use crate::models::error::{AppError, Result};

static TEMP_COPY_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalDbMode {
    CopyToTemp,
    InPlaceDangerous,
}

#[derive(Debug)]
pub struct PreparedEvalDb {
    db_dir: PathBuf,
    db_path: PathBuf,
    copied_from: Option<PathBuf>,
    cleanup_dir: Option<PathBuf>,
}

impl PreparedEvalDb {
    pub fn db_dir(&self) -> &Path {
        &self.db_dir
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn copied_from(&self) -> Option<&Path> {
        self.copied_from.as_deref()
    }
}

impl Drop for PreparedEvalDb {
    fn drop(&mut self) {
        if let Some(dir) = self.cleanup_dir.as_deref() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

pub fn prepare_eval_db(prod_db_path: &Path, mode: EvalDbMode, label: &str) -> Result<PreparedEvalDb> {
    if !prod_db_path.exists() {
        return Err(AppError::InvalidInput(format!(
            "prod DB not found at {}",
            prod_db_path.display()
        )));
    }

    let prod_db_dir = prod_db_path
        .parent()
        .ok_or_else(|| AppError::InvalidInput("prod-db path has no parent".into()))?
        .to_path_buf();

    if mode == EvalDbMode::InPlaceDangerous {
        eprintln!(
            "[eval-db] DANGER: operating on production DB in place at {}",
            prod_db_path.display()
        );
        return Ok(PreparedEvalDb {
            db_dir: prod_db_dir,
            db_path: prod_db_path.to_path_buf(),
            copied_from: None,
            cleanup_dir: None,
        });
    }

    let copy_dir = temp_copy_dir(label);
    std::fs::create_dir_all(&copy_dir).map_err(|e| AppError::IoError(e.to_string()))?;
    let copy_path = copy_dir.join("emailops.db");
    backup_sqlite_db(prod_db_path, &copy_path)?;

    eprintln!(
        "[eval-db] copied {} to isolated private eval DB {}",
        prod_db_path.display(),
        copy_path.display()
    );

    Ok(PreparedEvalDb {
        db_dir: copy_dir.clone(),
        db_path: copy_path,
        copied_from: Some(prod_db_path.to_path_buf()),
        cleanup_dir: Some(copy_dir),
    })
}

fn backup_sqlite_db(source_path: &Path, dest_path: &Path) -> Result<()> {
    let source = Connection::open(source_path)?;
    let mut dest = Connection::open(dest_path)?;
    let backup = rusqlite::backup::Backup::new(&source, &mut dest)?;
    backup.run_to_completion(5, std::time::Duration::from_millis(250), None)?;
    drop(backup);
    drop(dest);
    drop(source);
    Ok(())
}

fn temp_copy_dir(label: &str) -> PathBuf {
    let sanitized_label: String = label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let counter = TEMP_COPY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
    std::env::temp_dir().join("emailops-private-evals").join(format!(
        "{}-{}-{}-{}",
        sanitized_label,
        std::process::id(),
        timestamp,
        counter
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use tempfile::tempdir;

    fn create_source_db() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().expect("temp source dir");
        let db_path = dir.path().join("emailops.db");
        let conn = Connection::open(&db_path).expect("open source db");
        conn.execute("CREATE TABLE marker (value TEXT NOT NULL)", [])
            .expect("create marker table");
        conn.execute("INSERT INTO marker (value) VALUES (?1)", params!["prod"])
            .expect("insert source marker");
        drop(conn);
        (dir, db_path)
    }

    #[test]
    fn copy_to_temp_opens_an_isolated_database_copy() {
        let (_dir, source_path) = create_source_db();

        let prepared = prepare_eval_db(&source_path, EvalDbMode::CopyToTemp, "chat").expect("prepare db");

        assert_ne!(prepared.db_path(), source_path.as_path());
        assert_eq!(prepared.copied_from(), Some(source_path.as_path()));

        let copy = Connection::open(prepared.db_path()).expect("open copied db");
        copy.execute("INSERT INTO marker (value) VALUES (?1)", params!["copy"])
            .expect("mutate copied db");
        drop(copy);

        let source = Connection::open(&source_path).expect("reopen source db");
        let source_count: i64 = source
            .query_row("SELECT COUNT(*) FROM marker", [], |row| row.get(0))
            .expect("count source rows");
        assert_eq!(source_count, 1);
    }

    #[test]
    fn in_place_dangerous_returns_the_original_database_directory() {
        let (_dir, source_path) = create_source_db();

        let prepared = prepare_eval_db(&source_path, EvalDbMode::InPlaceDangerous, "chat").expect("prepare db");

        assert_eq!(prepared.db_path(), source_path.as_path());
        assert_eq!(prepared.db_dir(), source_path.parent().expect("source parent"));
        assert_eq!(prepared.copied_from(), None);
    }
}
