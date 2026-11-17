//! The two backups the app makes while it is open, at the hour the user picks
//! in Settings → Privacy & Security → Backups:
//!
//! - the local data copy ([`local`]): only the tables that exist nowhere else
//!   ([`tables`]), the newest [`local::KEEP`] in `local-backups/`. Settings
//!   restores from it, and Time Machine copies it with the rest of the data
//!   folder.
//! - the full copy: the whole database through SQLite's online backup API
//!   (`Database::backup`), the newest 1–[`MAX_KEEP`] in `backups/`, a folder
//!   Time Machine skips.
//!
//! A background thread calls [`run_due`] every [`CHECK_EVERY`]; whether a copy
//! is due is the pure [`schedule::is_due`].

pub mod local;
pub mod schedule;
pub mod tables;
mod time_machine;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::models::error::{AppError, Result};

pub const HOUR_PREF: &str = "backup.hour";
pub const LOCAL_PREF: &str = "backup.local";
pub const FULL_PREF: &str = "backup.full";
pub const KEEP_PREF: &str = "backup.keep";

/// Most full copies a user can keep: each one is the size of the whole database.
pub const MAX_KEEP: usize = 3;

/// How often the background thread checks whether a copy is due.
pub const CHECK_EVERY: Duration = Duration::from_secs(5 * 60);

const FULL_DIR: &str = "backups";
const FULL_PREFIX: &str = "emailops-";
const LOCAL_DIR: &str = "local-backups";

/// Disk a full copy leaves free: it must never take the room the live database
/// needs to keep writing.
const FREE_RESERVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupFrequency {
    Off,
    Daily,
    Weekly,
}

impl BackupFrequency {
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
    /// Local hour, 0–23, from which a due copy is made.
    pub hour: u32,
    pub local: BackupFrequency,
    pub full: BackupFrequency,
    /// Full copies kept, 1–[`MAX_KEEP`].
    pub keep: usize,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            hour: 3,
            local: BackupFrequency::Daily,
            full: BackupFrequency::Daily,
            keep: 1,
        }
    }
}

/// Pure: the stored preferences, or the default for one that is missing or not
/// a value Settings writes.
pub fn parse_settings(
    hour: Option<&str>,
    local: Option<&str>,
    full: Option<&str>,
    keep: Option<&str>,
) -> BackupSettings {
    let default = BackupSettings::default();
    BackupSettings {
        hour: hour
            .and_then(|h| h.parse().ok())
            .filter(|h| *h < 24)
            .unwrap_or(default.hour),
        local: local.and_then(BackupFrequency::from_pref).unwrap_or(default.local),
        full: full.and_then(BackupFrequency::from_pref).unwrap_or(default.full),
        keep: keep
            .and_then(|k| k.parse().ok())
            .filter(|k| (1..=MAX_KEEP).contains(k))
            .unwrap_or(default.keep),
    }
}

pub fn validate(settings: BackupSettings) -> Result<BackupSettings> {
    if settings.hour >= 24 {
        return Err(AppError::InvalidInput(format!(
            "the backup hour must be between 0 and 23, got: {}",
            settings.hour
        )));
    }
    if !(1..=MAX_KEEP).contains(&settings.keep) {
        return Err(AppError::InvalidInput(format!(
            "copies to keep must be between 1 and {MAX_KEEP}, got: {}",
            settings.keep
        )));
    }
    Ok(settings)
}

pub fn load_settings(db: &Database) -> Result<BackupSettings> {
    Ok(parse_settings(
        db.get_preference(HOUR_PREF)?.as_deref(),
        db.get_preference(LOCAL_PREF)?.as_deref(),
        db.get_preference(FULL_PREF)?.as_deref(),
        db.get_preference(KEEP_PREF)?.as_deref(),
    ))
}

/// Store the settings and drop the full copies beyond the new `keep` right
/// away, so lowering it frees the disk now rather than at the next copy.
pub fn save_settings(db: &Database, data_dir: &Path, settings: BackupSettings) -> Result<()> {
    let settings = validate(settings)?;
    db.set_preference(HOUR_PREF, &settings.hour.to_string())?;
    db.set_preference(LOCAL_PREF, settings.local.as_pref())?;
    db.set_preference(FULL_PREF, settings.full.as_pref())?;
    db.set_preference(KEEP_PREF, &settings.keep.to_string())?;
    prune(&data_dir.join(FULL_DIR), FULL_PREFIX, settings.keep)
}

/// One copy on disk, as Settings lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupCopy {
    pub name: String,
    /// When the copy was taken, in unix seconds.
    pub taken_at: i64,
    pub bytes: u64,
}

/// The backup settings and the copies on disk, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    pub hour: u32,
    pub local: BackupFrequency,
    pub full: BackupFrequency,
    pub keep: usize,
    pub local_copies: Vec<BackupCopy>,
    pub full_copies: Vec<BackupCopy>,
}

pub fn status(db: &Database, data_dir: &Path) -> Result<BackupStatus> {
    let settings = load_settings(db)?;
    Ok(BackupStatus {
        hour: settings.hour,
        local: settings.local,
        full: settings.full,
        keep: settings.keep,
        local_copies: copies(&data_dir.join(LOCAL_DIR), local::PREFIX)?,
        full_copies: copies(&data_dir.join(FULL_DIR), FULL_PREFIX)?,
    })
}

/// Delete every full copy and return how many there were. A copy still being
/// written is left alone: its temp name is not a copy's.
pub fn delete_full(data_dir: &Path) -> Result<usize> {
    let dir = data_dir.join(FULL_DIR);
    let found = copies(&dir, FULL_PREFIX)?;
    for copy in &found {
        std::fs::remove_file(dir.join(&copy.name))
            .map_err(|e| AppError::IoError(format!("Could not delete a backup: {e}")))?;
    }
    Ok(found.len())
}

/// Restore the local data copy called `name`, one of those [`status`] lists.
/// The name is matched against the folder, so the webview never names a path.
pub fn restore_local(db: &Database, data_dir: &Path, name: &str) -> Result<usize> {
    let dir = data_dir.join(LOCAL_DIR);
    if !copies(&dir, local::PREFIX)?.iter().any(|c| c.name == name) {
        return Err(AppError::NotFound(format!("No local backup called {name}")));
    }
    local::restore(db, &dir.join(name))
}

/// What one check did with one kind of copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ran {
    NotDue,
    Written,
    NoRoom { needed: u64, free: u64 },
}

/// What one check did with each kind of copy.
#[derive(Debug)]
pub struct Checked {
    pub local: Result<Ran>,
    pub full: Result<Ran>,
}

static PAUSED: AtomicBool = AtomicBool::new(false);
static EXCLUSION_TRIED: AtomicBool = AtomicBool::new(false);

/// Start no copy from now on. For a data move or a restore, which restart the
/// app when they finish.
pub fn pause() {
    PAUSED.store(true, Ordering::SeqCst);
}

/// Make each copy that is due at `now`, the small local one first.
pub fn run_due<Tz: TimeZone>(db: &Database, data_dir: &Path, now: &DateTime<Tz>) -> Result<Checked> {
    if PAUSED.load(Ordering::SeqCst) {
        return Ok(Checked {
            local: Ok(Ran::NotDue),
            full: Ok(Ran::NotDue),
        });
    }
    let settings = load_settings(db)?;
    let full_dir = data_dir.join(FULL_DIR);
    // Copies made before this release are in Time Machine's way too.
    if full_dir.exists() {
        exclude_from_time_machine_once(&full_dir);
    }
    Ok(Checked {
        local: local_copy(db, &data_dir.join(LOCAL_DIR), settings, now),
        full: full_copy(db, &full_dir, settings, now),
    })
}

fn local_copy<Tz: TimeZone>(db: &Database, dir: &Path, settings: BackupSettings, now: &DateTime<Tz>) -> Result<Ran> {
    if !schedule::is_due(now, settings.hour, settings.local, newest(dir, local::PREFIX)?) {
        return Ok(Ran::NotDue);
    }
    local::write_copy(db, dir, now.with_timezone(&Utc))?;
    Ok(Ran::Written)
}

fn full_copy<Tz: TimeZone>(db: &Database, dir: &Path, settings: BackupSettings, now: &DateTime<Tz>) -> Result<Ran> {
    if !schedule::is_due(now, settings.hour, settings.full, newest(dir, FULL_PREFIX)?) {
        return Ok(Ran::NotDue);
    }
    let db_bytes = std::fs::metadata(db.db_path())
        .map_err(|e| AppError::IoError(format!("Could not read the database size: {e}")))?
        .len();
    if let Some((needed, free)) = no_room(db_bytes, crate::util::system::available_disk_bytes(dir)) {
        return Ok(Ran::NoRoom { needed, free });
    }
    std::fs::create_dir_all(dir).map_err(|e| AppError::IoError(format!("Could not create the backups folder: {e}")))?;
    exclude_from_time_machine_once(dir);
    crate::services::logger::log("info", "system", "Backing up the database");
    db.backup(dir, settings.keep)?;
    Ok(Ran::Written)
}

/// Pure: `Some((needed, free))` when the disk cannot take a copy of a
/// `db_bytes` database and keep the reserve. `free_bytes` is `None` when the
/// disk probe failed, and the copy then finds out for itself.
fn no_room(db_bytes: u64, free_bytes: Option<u64>) -> Option<(u64, u64)> {
    let needed = db_bytes.saturating_add(FREE_RESERVE_BYTES);
    match free_bytes {
        Some(free) if free < needed => Some((needed, free)),
        _ => None,
    }
}

/// Once per run of the app, so a failing `tmutil` writes one line to Logs
/// rather than one every check.
fn exclude_from_time_machine_once(dir: &Path) {
    if EXCLUSION_TRIED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(e) = time_machine::exclude(dir) {
        crate::services::logger::log(
            "warn",
            "system",
            format!("Database backups may be copied by Time Machine: {e}"),
        );
    }
}

/// When a copy was taken, from the UTC time in its file name.
fn taken_at(prefix: &str, name: &str) -> Option<i64> {
    let stamp = name.strip_prefix(prefix)?.strip_suffix(".db")?;
    let taken = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%d_%H%M%S").ok()?;
    Some(taken.and_utc().timestamp())
}

/// The finished copies in `dir`, newest first. None before the first copy has
/// created the folder; a temp file of a copy in progress does not count.
fn copies(dir: &Path, prefix: &str) -> Result<Vec<BackupCopy>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(AppError::IoError(format!("Could not read a backups folder: {e}"))),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| AppError::IoError(format!("Could not read a backups folder: {e}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(taken_at) = taken_at(prefix, &name) else {
            continue;
        };
        let bytes = entry
            .metadata()
            .map_err(|e| AppError::IoError(format!("Could not read the size of a backup: {e}")))?
            .len();
        found.push(BackupCopy { name, taken_at, bytes });
    }
    found.sort_by_key(|c| std::cmp::Reverse(c.taken_at));
    Ok(found)
}

fn newest(dir: &Path, prefix: &str) -> Result<Option<i64>> {
    Ok(copies(dir, prefix)?.first().map(|c| c.taken_at))
}

/// Delete the copies in `dir` beyond the newest `keep`.
fn prune(dir: &Path, prefix: &str, keep: usize) -> Result<()> {
    for old in copies(dir, prefix)?.iter().skip(keep) {
        std::fs::remove_file(dir.join(&old.name))
            .map_err(|e| AppError::IoError(format!("Could not delete an old backup: {e}")))?;
    }
    Ok(())
}

pub fn local_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(LOCAL_DIR)
}

pub fn full_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(FULL_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn copy_name(prefix: &str, taken_at: i64) -> String {
        let stamp = DateTime::from_timestamp(taken_at, 0)
            .expect("valid time")
            .format("%Y%m%d_%H%M%S");
        format!("{prefix}{stamp}.db")
    }

    fn names(dir: &Path, prefix: &str) -> Vec<String> {
        copies(dir, prefix)
            .expect("copies")
            .into_iter()
            .map(|c| c.name)
            .collect()
    }

    fn off() -> BackupSettings {
        BackupSettings {
            local: BackupFrequency::Off,
            full: BackupFrequency::Off,
            ..BackupSettings::default()
        }
    }

    #[test]
    fn default_settings_copy_daily_at_three_and_keep_one_full_copy() {
        assert_eq!(
            BackupSettings::default(),
            BackupSettings {
                hour: 3,
                local: BackupFrequency::Daily,
                full: BackupFrequency::Daily,
                keep: 1
            }
        );
    }

    #[test]
    fn parse_settings_reads_what_settings_writes_and_defaults_the_rest() {
        let default = BackupSettings::default();
        let cases = [
            ("nothing stored", [None, None, None, None], default),
            (
                "everything stored",
                [Some("22"), Some("weekly"), Some("off"), Some("3")],
                BackupSettings {
                    hour: 22,
                    local: BackupFrequency::Weekly,
                    full: BackupFrequency::Off,
                    keep: 3,
                },
            ),
            ("hour out of range", [Some("24"), None, None, None], default),
            (
                "unknown frequency",
                [None, Some("hourly"), Some("monthly"), None],
                default,
            ),
            ("no copies", [None, None, None, Some("0")], default),
            ("too many copies", [None, None, None, Some("7")], default),
            ("not a number", [Some("three"), None, None, Some("one")], default),
        ];
        for (name, [hour, local, full, keep], expected) in cases {
            assert_eq!(parse_settings(hour, local, full, keep), expected, "{name}");
        }
    }

    #[test]
    fn validate_takes_hours_0_to_23_and_1_to_3_copies() {
        for (hour, keep) in [(0, 1), (23, 3), (3, 2)] {
            let settings = BackupSettings {
                hour,
                keep,
                ..BackupSettings::default()
            };
            assert_eq!(validate(settings).expect("valid"), settings);
        }
        for (hour, keep) in [(24, 1), (3, 0), (3, 4)] {
            let settings = BackupSettings {
                hour,
                keep,
                ..BackupSettings::default()
            };
            assert!(validate(settings).is_err(), "{hour} {keep}");
        }
    }

    #[test]
    fn saved_settings_are_loaded_back() {
        let db = Database::new_for_testing().expect("test db");
        let data = tempfile::tempdir().expect("data dir");
        let settings = BackupSettings {
            hour: 13,
            local: BackupFrequency::Weekly,
            full: BackupFrequency::Off,
            keep: 2,
        };

        save_settings(&db, data.path(), settings).expect("save");

        assert_eq!(load_settings(&db).expect("load"), settings);
    }

    #[test]
    fn saving_fewer_copies_removes_the_oldest_full_copies_now() {
        let db = Database::new_for_testing().expect("test db");
        let data = tempfile::tempdir().expect("data dir");
        let dir = full_dir(data.path());
        fs::create_dir_all(&dir).expect("dir");
        let now = 1_800_000_000;
        for age_days in [3, 2, 1] {
            fs::write(dir.join(copy_name(FULL_PREFIX, now - age_days * 86_400)), b"x").expect("seed copy");
        }

        save_settings(&db, data.path(), BackupSettings::default()).expect("save");

        assert_eq!(names(&dir, FULL_PREFIX), vec![copy_name(FULL_PREFIX, now - 86_400)]);
    }

    #[test]
    fn saving_invalid_settings_changes_nothing() {
        let db = Database::new_for_testing().expect("test db");
        let data = tempfile::tempdir().expect("data dir");

        let result = save_settings(&db, data.path(), BackupSettings { hour: 25, ..off() });

        assert!(result.is_err());
        assert_eq!(load_settings(&db).expect("load"), BackupSettings::default());
    }

    #[test]
    fn status_lists_both_kinds_of_copies_newest_first_with_their_size() {
        let db = Database::new_for_testing().expect("test db");
        let data = tempfile::tempdir().expect("data dir");
        let now = 1_800_000_000;
        let full = full_dir(data.path());
        let local = local_dir(data.path());
        fs::create_dir_all(&full).expect("dir");
        fs::create_dir_all(&local).expect("dir");
        fs::write(full.join(copy_name(FULL_PREFIX, now)), vec![0u8; 2_000]).expect("seed");
        fs::write(full.join(".emailops-backup.tmp"), vec![0u8; 500]).expect("seed temp");
        fs::write(local.join(copy_name(local::PREFIX, now - 86_400)), vec![0u8; 10]).expect("seed");
        fs::write(local.join(copy_name(local::PREFIX, now)), vec![0u8; 20]).expect("seed");

        let status = status(&db, data.path()).expect("status");

        assert_eq!(
            status.full_copies,
            vec![BackupCopy {
                name: copy_name(FULL_PREFIX, now),
                taken_at: now,
                bytes: 2_000
            }]
        );
        let local_times: Vec<i64> = status.local_copies.iter().map(|c| c.taken_at).collect();
        assert_eq!(local_times, vec![now, now - 86_400]);
    }

    #[test]
    fn status_before_the_first_copy_lists_none() {
        let db = Database::new_for_testing().expect("test db");
        let data = tempfile::tempdir().expect("data dir");

        let status = status(&db, data.path()).expect("status");

        assert!(status.local_copies.is_empty() && status.full_copies.is_empty());
    }

    #[test]
    fn delete_full_removes_every_full_copy_and_leaves_a_copy_in_progress() {
        let data = tempfile::tempdir().expect("data dir");
        let dir = full_dir(data.path());
        fs::create_dir_all(&dir).expect("dir");
        let now = 1_800_000_000;
        fs::write(dir.join(copy_name(FULL_PREFIX, now - 86_400)), b"x").expect("seed");
        fs::write(dir.join(copy_name(FULL_PREFIX, now)), b"x").expect("seed");
        fs::write(dir.join(".emailops-backup.tmp"), b"x").expect("seed temp");

        assert_eq!(delete_full(data.path()).expect("delete"), 2);

        assert!(names(&dir, FULL_PREFIX).is_empty());
        assert!(dir.join(".emailops-backup.tmp").exists());
    }

    #[test]
    fn delete_full_before_the_first_copy_deletes_nothing() {
        let data = tempfile::tempdir().expect("data dir");

        assert_eq!(delete_full(data.path()).expect("delete"), 0);
    }

    #[test]
    fn no_room_keeps_two_gib_free() {
        let db = 11 * GIB;
        let needed = db + FREE_RESERVE_BYTES;
        let cases = [
            ("plenty of room", Some(100 * GIB), None),
            ("exactly enough", Some(needed), None),
            ("too little", Some(needed - 1), Some((needed, needed - 1))),
            ("free space unknown", None, None),
        ];
        for (name, free, expected) in cases {
            assert_eq!(no_room(db, free), expected, "{name}");
        }
    }

    #[test]
    fn a_check_with_both_copies_off_writes_nothing() {
        let data = tempfile::tempdir().expect("data dir");
        let db = Database::new(data.path().to_path_buf()).expect("open db");
        save_settings(&db, data.path(), off()).expect("save");

        let checked = run_due(&db, data.path(), &Utc::now()).expect("check");

        assert_eq!(checked.local.expect("local"), Ran::NotDue);
        assert_eq!(checked.full.expect("full"), Ran::NotDue);
        assert!(!local_dir(data.path()).exists() && !full_dir(data.path()).exists());
    }

    #[test]
    fn the_first_check_makes_both_copies_and_the_next_one_none() {
        let data = tempfile::tempdir().expect("data dir");
        let db = Database::new(data.path().to_path_buf()).expect("open db");

        let first = run_due(&db, data.path(), &Utc::now()).expect("check");
        let second = run_due(&db, data.path(), &Utc::now()).expect("check");

        assert_eq!(
            (first.local.expect("local"), first.full.expect("full")),
            (Ran::Written, Ran::Written)
        );
        assert_eq!(
            (second.local.expect("local"), second.full.expect("full")),
            (Ran::NotDue, Ran::NotDue)
        );
        assert_eq!(names(&local_dir(data.path()), local::PREFIX).len(), 1);
        assert_eq!(names(&full_dir(data.path()), FULL_PREFIX).len(), 1);
    }

    #[test]
    fn a_full_copy_keeps_the_copies_the_user_chose() {
        let data = tempfile::tempdir().expect("data dir");
        let db = Database::new(data.path().to_path_buf()).expect("open db");
        let dir = full_dir(data.path());
        fs::create_dir_all(&dir).expect("dir");
        for day in 1..=3 {
            fs::write(dir.join(copy_name(FULL_PREFIX, 1_800_000_000 - day * 86_400)), b"x").expect("seed");
        }
        db.set_preference(KEEP_PREF, "2").expect("keep two");

        let checked = run_due(&db, data.path(), &Utc::now()).expect("check");

        assert_eq!(checked.full.expect("full"), Ran::Written);
        assert_eq!(names(&dir, FULL_PREFIX).len(), 2);
    }

    #[test]
    fn restore_local_only_takes_the_name_of_a_listed_copy() {
        let data = tempfile::tempdir().expect("data dir");
        let db = Database::new(data.path().to_path_buf()).expect("open db");
        fs::write(data.path().join("local-20270115_080000.db"), b"x").expect("file outside the folder");

        for name in ["local-20270115_080000.db", "../local-20270115_080000.db", "emailops.db"] {
            let err = restore_local(&db, data.path(), name).expect_err("refused");
            assert!(matches!(err, AppError::NotFound(_)), "{name}: {err}");
        }
    }
}
