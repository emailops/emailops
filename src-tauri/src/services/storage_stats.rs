//! Disk usage stats for the dashboard.
//!
//! Computes the on-disk footprint of EmailOps data without touching SQL.
//! Reports:
//! - the SQLite file and its `-wal` / `-shm` sidecars,
//! - the attachments directory (recursive walk),
//! - everything else under `app_data_dir` rolled up into `otherBytes` so the
//!   dashboard total matches `du -s` on the app data folder.
//!
//! Per-table / per-account breakdowns are intentionally out of scope here:
//! `dbstat` is not exposed by rusqlite 0.31, and the user explicitly opted
//! out of running SUM/length queries on every dashboard load.

use std::path::Path;

use serde::Serialize;

use crate::models::error::Result;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStats {
    /// Sum of every regular file under `app_data_dir` (recursive). This is
    /// the number that should match a user-facing "EmailOps is using N MB".
    pub total_bytes: u64,
    /// Size of the main SQLite file (`emailops.db`). 0 if the file is
    /// missing (e.g. in-memory test DB).
    pub db_file_bytes: u64,
    /// Size of the SQLite write-ahead log (`emailops.db-wal`). 0 when the
    /// WAL has been checkpointed and removed.
    pub wal_bytes: u64,
    /// Size of the SQLite shared-memory file (`emailops.db-shm`). 0 when
    /// not present.
    pub shm_bytes: u64,
    /// Size of the `attachments/` subtree under `app_data_dir`. Counts
    /// every regular file recursively so per-account / per-rule layouts
    /// are both captured.
    pub attachments_bytes: u64,
    /// Size of the `models/` subtree — local LLM weights downloaded for
    /// the embedded llama-cpp backend. Often the largest bucket.
    pub models_bytes: u64,
    /// Size of the `backups/` subtree — periodic SQLite backups created
    /// by `Database::backup()`.
    pub backups_bytes: u64,
    /// Everything under `app_data_dir` not accounted for above. Lock
    /// files, OAuth scratch, future caches, etc. Should be small.
    pub other_bytes: u64,
    /// Unix seconds when this snapshot was taken.
    pub computed_at: i64,
}

/// Walk every regular file under `root` and return the byte total. Missing
/// directories are treated as 0 (fresh installs that haven't created the
/// folder yet shouldn't error). Symlinks are not followed; we only count
/// regular files so attachment hardlinks / OS-managed metadata don't get
/// double-counted.
fn dir_size_bytes(root: &Path) -> Result<u64> {
    let mut total: u64 = 0;
    let mut stack: Vec<std::path::PathBuf> = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(crate::models::error::AppError::IoError(format!(
                    "read_dir({}): {err}",
                    dir.display()
                )))
            }
        };
        for entry in entries.flatten() {
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue, // race: file deleted between readdir and stat
            };
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}

/// File size in bytes, or 0 if the file doesn't exist. Any other IO error
/// (permission denied, etc.) bubbles up — we want the user to see those.
fn file_size_or_zero(path: &Path) -> Result<u64> {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => Ok(m.len()),
        Ok(_) => Ok(0),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(crate::models::error::AppError::IoError(format!(
            "metadata({}): {err}",
            path.display()
        ))),
    }
}

/// Build a `StorageStats` snapshot. `db_path` is the path to the live
/// `emailops.db` (so we can subtract it from `otherBytes`); `app_data_dir`
/// is the directory whose total we want to report.
///
/// This is a pure filesystem operation — no DB locks taken, no SQL run.
pub fn collect_storage_stats(app_data_dir: &Path, db_path: &Path) -> Result<StorageStats> {
    let total_bytes = dir_size_bytes(app_data_dir)?;

    let (db_file_bytes, wal_bytes, shm_bytes) = if db_path.as_os_str().is_empty() {
        (0, 0, 0)
    } else {
        let db = file_size_or_zero(db_path)?;
        // SQLite sidecars sit next to the main file with `-wal` / `-shm`
        // suffixes appended to the *full* filename (including extension).
        let mut wal_path = db_path.as_os_str().to_owned();
        wal_path.push("-wal");
        let mut shm_path = db_path.as_os_str().to_owned();
        shm_path.push("-shm");
        let wal = file_size_or_zero(Path::new(&wal_path))?;
        let shm = file_size_or_zero(Path::new(&shm_path))?;
        (db, wal, shm)
    };

    let attachments_bytes = dir_size_bytes(&app_data_dir.join("attachments"))?;
    let models_bytes = dir_size_bytes(&app_data_dir.join("models"))?;
    let backups_bytes = dir_size_bytes(&app_data_dir.join("backups"))?;

    let accounted = db_file_bytes
        .saturating_add(wal_bytes)
        .saturating_add(shm_bytes)
        .saturating_add(attachments_bytes)
        .saturating_add(models_bytes)
        .saturating_add(backups_bytes);
    let other_bytes = total_bytes.saturating_sub(accounted);

    Ok(StorageStats {
        total_bytes,
        db_file_bytes,
        wal_bytes,
        shm_bytes,
        attachments_bytes,
        models_bytes,
        backups_bytes,
        other_bytes,
        computed_at: chrono::Utc::now().timestamp(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write `bytes` random-ish bytes to `path`, creating parent dirs.
    fn write_file(path: &Path, size: usize) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(&vec![0u8; size]).unwrap();
    }

    #[test]
    fn empty_dir_reports_zeros() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("emailops.db");
        let stats = collect_storage_stats(tmp.path(), &db_path).unwrap();
        assert_eq!(stats.total_bytes, 0);
        assert_eq!(stats.db_file_bytes, 0);
        assert_eq!(stats.wal_bytes, 0);
        assert_eq!(stats.shm_bytes, 0);
        assert_eq!(stats.attachments_bytes, 0);
        assert_eq!(stats.models_bytes, 0);
        assert_eq!(stats.backups_bytes, 0);
        assert_eq!(stats.other_bytes, 0);
    }

    #[test]
    fn buckets_db_attachments_models_backups_and_other() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_file(&root.join("emailops.db"), 1000);
        write_file(&root.join("emailops.db-wal"), 200);
        write_file(&root.join("emailops.db-shm"), 32);
        write_file(&root.join("attachments/acct-1/a.bin"), 500);
        write_file(&root.join("attachments/acct-2/nested/b.bin"), 700);
        write_file(&root.join("models/qwen-3b.gguf"), 9_000);
        write_file(&root.join("backups/emailops-20260101.db"), 2_000);
        write_file(&root.join("logs/app.log"), 50);
        write_file(&root.join("emailops.lock"), 4);

        let stats = collect_storage_stats(root, &root.join("emailops.db")).unwrap();
        assert_eq!(stats.db_file_bytes, 1000);
        assert_eq!(stats.wal_bytes, 200);
        assert_eq!(stats.shm_bytes, 32);
        assert_eq!(stats.attachments_bytes, 1200);
        assert_eq!(stats.models_bytes, 9_000);
        assert_eq!(stats.backups_bytes, 2_000);
        assert_eq!(stats.other_bytes, 50 + 4);
        assert_eq!(stats.total_bytes, 1000 + 200 + 32 + 1200 + 9_000 + 2_000 + 50 + 4);
    }

    #[test]
    fn missing_db_path_zeroes_sqlite_buckets() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(&tmp.path().join("attachments/x.bin"), 42);
        let stats = collect_storage_stats(tmp.path(), Path::new("")).unwrap();
        assert_eq!(stats.db_file_bytes, 0);
        assert_eq!(stats.wal_bytes, 0);
        assert_eq!(stats.shm_bytes, 0);
        assert_eq!(stats.attachments_bytes, 42);
        assert_eq!(stats.models_bytes, 0);
        assert_eq!(stats.backups_bytes, 0);
        assert_eq!(stats.total_bytes, 42);
        assert_eq!(stats.other_bytes, 0);
    }
}
