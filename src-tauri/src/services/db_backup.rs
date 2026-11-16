//! The automatic database backup the app makes shortly after launch: how often
//! it runs, how many copies it keeps, and the copies already in `backups/`.
//!
//! The copy itself is `Database::startup_backup`; this module owns the user's
//! settings for it and what Settings shows and deletes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::models::error::{AppError, Result};

pub const FREQUENCY_PREF: &str = "db_backup.frequency";
pub const KEEP_PREF: &str = "db_backup.keep";

/// Most copies a user can keep: each one is the size of the whole database.
pub const MAX_KEEP: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupFrequency {
    Off,
    Daily,
    Weekly,
}

impl BackupFrequency {
    /// How long a copy stays current; `None` when backups are off.
    pub fn interval_secs(self) -> Option<i64> {
        const DAY: i64 = 24 * 60 * 60;
        match self {
            Self::Off => None,
            Self::Daily => Some(DAY),
            Self::Weekly => Some(7 * DAY),
        }
    }

    fn from_pref(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            _ => None,
        }
    }

    fn as_pref(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupSettings {
    pub frequency: BackupFrequency,
    pub keep: usize,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            frequency: BackupFrequency::Daily,
            keep: MAX_KEEP,
        }
    }
}

/// Pure: the stored preferences, or today's default for one that is missing or
/// not a value Settings writes.
pub fn parse_settings(frequency: Option<&str>, keep: Option<&str>) -> BackupSettings {
    let default = BackupSettings::default();
    BackupSettings {
        frequency: frequency
            .and_then(BackupFrequency::from_pref)
            .unwrap_or(default.frequency),
        keep: keep
            .and_then(|k| k.parse().ok())
            .and_then(|k| validate_keep(k).ok())
            .unwrap_or(default.keep),
    }
}

pub fn validate_keep(keep: usize) -> Result<usize> {
    if (1..=MAX_KEEP).contains(&keep) {
        Ok(keep)
    } else {
        Err(AppError::InvalidInput(format!(
            "copies to keep must be between 1 and {MAX_KEEP}, got: {keep}"
        )))
    }
}

pub fn load_settings(db: &Database) -> Result<BackupSettings> {
    Ok(parse_settings(
        db.get_preference(FREQUENCY_PREF)?.as_deref(),
        db.get_preference(KEEP_PREF)?.as_deref(),
    ))
}

/// Store the settings and drop the copies beyond the new `keep` right away,
/// so lowering it frees the disk now rather than at the next backup.
pub fn save_settings(db: &Database, backup_dir: &Path, settings: BackupSettings) -> Result<()> {
    let keep = validate_keep(settings.keep)?;
    db.set_preference(FREQUENCY_PREF, settings.frequency.as_pref())?;
    db.set_preference(KEEP_PREF, &keep.to_string())?;
    Database::prune_backups(backup_dir, keep);
    Ok(())
}

/// The backup settings and the copies on disk, as Settings shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    pub frequency: BackupFrequency,
    pub keep: usize,
    pub count: usize,
    pub total_bytes: u64,
    /// When the newest copy was taken, in unix seconds.
    pub newest_at: Option<i64>,
}

pub fn status(db: &Database, backup_dir: &Path) -> Result<BackupStatus> {
    let settings = load_settings(db)?;
    let copies = copies_in(backup_dir)?;
    let mut total_bytes = 0u64;
    for copy in &copies {
        let len = std::fs::metadata(copy)
            .map_err(|e| AppError::IoError(format!("Could not read the size of a backup: {e}")))?
            .len();
        total_bytes = total_bytes.saturating_add(len);
    }
    let newest_at = copies
        .iter()
        .filter_map(|c| crate::db::backup_taken_at(&c.file_name()?.to_string_lossy()))
        .max();
    Ok(BackupStatus {
        frequency: settings.frequency,
        keep: settings.keep,
        count: copies.len(),
        total_bytes,
        newest_at,
    })
}

/// Delete every copy in `backup_dir` and return how many there were. A copy
/// still being written is left alone: its temp name is not a copy's.
pub fn delete_all(backup_dir: &Path) -> Result<usize> {
    let copies = copies_in(backup_dir)?;
    for copy in &copies {
        std::fs::remove_file(copy).map_err(|e| AppError::IoError(format!("Could not delete a backup: {e}")))?;
    }
    Ok(copies.len())
}

/// The copies in `backup_dir`: the files `Database::backup` names and prunes.
/// None before the first backup has created the folder.
fn copies_in(backup_dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(backup_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(AppError::IoError(format!("Could not read the backups folder: {e}"))),
    };
    let mut copies = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| AppError::IoError(format!("Could not read the backups folder: {e}")))?;
        if entry.file_name().to_string_lossy().starts_with("emailops-") {
            copies.push(entry.path());
        }
    }
    Ok(copies)
}

pub fn backup_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("backups")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn copy_name(taken_at: i64) -> String {
        let stamp = chrono::DateTime::from_timestamp(taken_at, 0)
            .expect("valid time")
            .format("%Y%m%d_%H%M%S");
        format!("emailops-{stamp}.db")
    }

    fn copies_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("read backup dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("emailops-"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn frequency_sets_how_long_a_copy_stays_current() {
        assert_eq!(BackupFrequency::Off.interval_secs(), None);
        assert_eq!(BackupFrequency::Daily.interval_secs(), Some(24 * 60 * 60));
        assert_eq!(BackupFrequency::Weekly.interval_secs(), Some(7 * 24 * 60 * 60));
    }

    #[test]
    fn parse_settings_reads_what_settings_writes_and_defaults_the_rest() {
        let default = BackupSettings::default();
        let cases = [
            ("nothing stored", None, None, default),
            (
                "off, one copy",
                Some("off"),
                Some("1"),
                BackupSettings {
                    frequency: BackupFrequency::Off,
                    keep: 1,
                },
            ),
            (
                "weekly, two copies",
                Some("weekly"),
                Some("2"),
                BackupSettings {
                    frequency: BackupFrequency::Weekly,
                    keep: 2,
                },
            ),
            ("unknown frequency", Some("hourly"), None, default),
            ("no copies", None, Some("0"), default),
            ("too many copies", None, Some("7"), default),
            ("not a number", None, Some("three"), default),
        ];
        for (name, frequency, keep, expected) in cases {
            assert_eq!(parse_settings(frequency, keep), expected, "{name}");
        }
    }

    #[test]
    fn default_settings_are_daily_with_three_copies() {
        assert_eq!(
            BackupSettings::default(),
            BackupSettings {
                frequency: BackupFrequency::Daily,
                keep: 3
            }
        );
    }

    #[test]
    fn keep_is_one_to_three_copies() {
        for ok in 1..=3 {
            assert_eq!(validate_keep(ok).expect("valid keep"), ok);
        }
        for bad in [0, 4, 100] {
            assert!(validate_keep(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn saved_settings_are_loaded_back() {
        let db = Database::new_for_testing().expect("test db");
        let dir = tempfile::tempdir().expect("backup dir");
        let settings = BackupSettings {
            frequency: BackupFrequency::Weekly,
            keep: 2,
        };

        save_settings(&db, dir.path(), settings).expect("save");

        assert_eq!(load_settings(&db).expect("load"), settings);
    }

    #[test]
    fn saving_fewer_copies_removes_the_oldest_now() {
        let db = Database::new_for_testing().expect("test db");
        let dir = tempfile::tempdir().expect("backup dir");
        let now = 1_800_000_000;
        for age_days in [3, 2, 1] {
            fs::write(dir.path().join(copy_name(now - age_days * 86_400)), b"x").expect("seed copy");
        }

        save_settings(
            &db,
            dir.path(),
            BackupSettings {
                frequency: BackupFrequency::Daily,
                keep: 1,
            },
        )
        .expect("save");

        assert_eq!(copies_in(dir.path()), vec![copy_name(now - 86_400)]);
    }

    #[test]
    fn saving_an_invalid_keep_changes_nothing() {
        let db = Database::new_for_testing().expect("test db");
        let dir = tempfile::tempdir().expect("backup dir");
        fs::write(dir.path().join(copy_name(1_800_000_000)), b"x").expect("seed copy");

        let result = save_settings(
            &db,
            dir.path(),
            BackupSettings {
                frequency: BackupFrequency::Off,
                keep: 0,
            },
        );

        assert!(result.is_err());
        assert_eq!(load_settings(&db).expect("load"), BackupSettings::default());
        assert_eq!(copies_in(dir.path()).len(), 1);
    }

    #[test]
    fn status_counts_the_copies_their_size_and_the_newest() {
        let db = Database::new_for_testing().expect("test db");
        let dir = tempfile::tempdir().expect("backup dir");
        let now = 1_800_000_000;
        fs::write(dir.path().join(copy_name(now - 86_400)), vec![0u8; 1_000]).expect("seed copy");
        fs::write(dir.path().join(copy_name(now)), vec![0u8; 2_000]).expect("seed copy");
        fs::write(dir.path().join(".emailops-backup.tmp"), vec![0u8; 500]).expect("seed temp");

        let status = status(&db, dir.path()).expect("status");

        assert_eq!(status.count, 2);
        assert_eq!(status.total_bytes, 3_000);
        assert_eq!(status.newest_at, Some(now));
    }

    #[test]
    fn status_before_the_first_backup_has_no_copies() {
        let db = Database::new_for_testing().expect("test db");
        let dir = tempfile::tempdir().expect("data dir");

        let status = status(&db, &dir.path().join("backups")).expect("status");

        assert_eq!((status.count, status.total_bytes, status.newest_at), (0, 0, None));
    }

    #[test]
    fn delete_all_removes_every_copy_and_leaves_a_copy_in_progress() {
        let dir = tempfile::tempdir().expect("backup dir");
        let now = 1_800_000_000;
        fs::write(dir.path().join(copy_name(now - 86_400)), b"x").expect("seed copy");
        fs::write(dir.path().join(copy_name(now)), b"x").expect("seed copy");
        fs::write(dir.path().join(".emailops-backup.tmp"), b"x").expect("seed temp");

        assert_eq!(delete_all(dir.path()).expect("delete"), 2);

        assert!(copies_in(dir.path()).is_empty());
        assert!(dir.path().join(".emailops-backup.tmp").exists());
    }

    #[test]
    fn delete_all_before_the_first_backup_deletes_nothing() {
        let dir = tempfile::tempdir().expect("data dir");

        assert_eq!(delete_all(&dir.path().join("backups")).expect("delete"), 0);
    }
}
