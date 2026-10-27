//! Move the data folder somewhere else (Settings → Privacy, onboarding).
//!
//! The move is a copy: the database goes as a consistent SQLite snapshot (the
//! live file cannot be copied byte for byte while the app writes to it), every
//! other file is copied as is, the copy is checked, and only then is the
//! pointer written that makes the next launch open it (`util::data_location`).
//! The caller restarts the app. The old folder is left untouched; the moved
//! database records where it is so Settings can tell the user to delete it.
//!
//! A refused or failed move changes nothing: the pointer is written last, and
//! whatever was copied is removed again.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::util::data_location::{self, DataDirSource};

#[cfg(test)]
mod tests;

/// Preference set in the moved database: the folder the data was copied from.
pub const PREVIOUS_LOCATION_PREF: &str = "data_location_previous";

/// The folder created inside the one the user picks, so picking a whole disk
/// or a busy folder still gives the data a place of its own.
const PICKED_SUBFOLDER: &str = "EmailOps";

const DB_FILE: &str = "emailops.db";

/// Top-level entries never copied: the database travels as a snapshot (which
/// folds in its WAL), and the instance lock belongs to the running process.
const NOT_COPIED: &[&str] = &[DB_FILE, "emailops.db-wal", "emailops.db-shm", "emailops.lock"];

/// Files the OS drops into any folder it shows; a folder holding only these
/// still counts as empty.
const OS_METADATA: &[&str] = &[".DS_Store", "Thumbs.db", "desktop.ini"];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MoveProblem {
    #[error("the data folder is set by EMAILOPS_DATA_DIR; unset it to choose a folder in the app")]
    EnvOverride,
    #[error("EmailOps already keeps its data there")]
    SameFolder,
    #[error("the new folder cannot be inside the current data folder")]
    InsideCurrent,
    #[error("{} already has files in it; choose an empty folder", path.display())]
    NotEmpty { path: PathBuf },
    #[error("EmailOps cannot write to {}", path.display())]
    NotWritable { path: PathBuf },
    #[error("not enough free space there: {} needed, {} available", size_label(*needed_bytes), size_label(*available_bytes))]
    NotEnoughSpace { needed_bytes: u64, available_bytes: u64 },
    #[error("the copy in {} does not match the original ({detail}); nothing was changed", path.display())]
    CopyMismatch { path: PathBuf, detail: String },
}

impl MoveProblem {
    /// Stable translation key suffix; renaming one breaks translations.
    pub fn code(&self) -> &'static str {
        match self {
            MoveProblem::EnvOverride => "data_move_env_override",
            MoveProblem::SameFolder => "data_move_same_folder",
            MoveProblem::InsideCurrent => "data_move_inside_current",
            MoveProblem::NotEmpty { .. } => "data_move_not_empty",
            MoveProblem::NotWritable { .. } => "data_move_not_writable",
            MoveProblem::NotEnoughSpace { .. } => "data_move_not_enough_space",
            MoveProblem::CopyMismatch { .. } => "data_move_copy_mismatch",
        }
    }

    /// Values the translated message interpolates.
    pub fn params(&self) -> BTreeMap<&'static str, String> {
        let mut p = BTreeMap::new();
        match self {
            MoveProblem::NotEmpty { path } | MoveProblem::NotWritable { path } => {
                p.insert("path", path.display().to_string());
            }
            MoveProblem::NotEnoughSpace {
                needed_bytes,
                available_bytes,
            } => {
                p.insert("needed", size_label(*needed_bytes));
                p.insert("available", size_label(*available_bytes));
            }
            MoveProblem::CopyMismatch { path, detail } => {
                p.insert("path", path.display().to_string());
                p.insert("detail", detail.clone());
            }
            MoveProblem::EnvOverride | MoveProblem::SameFolder | MoveProblem::InsideCurrent => {}
        }
        p
    }
}

/// Whole megabytes or gigabytes, rounded up, so the label reads the same in
/// every language (no decimal separator) and never understates what is needed.
fn size_label(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    if bytes >= GB {
        format!("{} GB", bytes.div_ceil(GB))
    } else {
        format!("{} MB", bytes.div_ceil(MB))
    }
}

/// What the planner needs to know about the target, gathered by `target_facts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetFacts {
    pub exists: bool,
    /// True when the folder is missing or holds only OS metadata files.
    pub is_empty: bool,
    pub writable: bool,
    pub free_bytes: u64,
}

/// A folder set with `EMAILOPS_DATA_DIR` belongs to whoever set the variable;
/// a pointer written from the app would be ignored on the next launch anyway.
pub fn ensure_movable(source: DataDirSource) -> std::result::Result<(), MoveProblem> {
    match source {
        DataDirSource::Env | DataDirSource::Flag => Err(MoveProblem::EnvOverride),
        DataDirSource::Pointer | DataDirSource::Default => Ok(()),
    }
}

/// Where the data goes when the user picks `picked` in the folder dialog.
pub fn target_for_picked(picked: &Path) -> PathBuf {
    picked.join(PICKED_SUBFOLDER)
}

/// Decide whether the data may move from `current` to `target`. Pure; both
/// paths must already be normalized the same way (see `normalize`).
pub fn plan_move(
    current: &Path,
    target: &Path,
    facts: &TargetFacts,
    needed_bytes: u64,
) -> std::result::Result<(), MoveProblem> {
    if target == current {
        return Err(MoveProblem::SameFolder);
    }
    if target.starts_with(current) {
        return Err(MoveProblem::InsideCurrent);
    }
    if facts.exists && !facts.is_empty {
        return Err(MoveProblem::NotEmpty {
            path: target.to_path_buf(),
        });
    }
    if !facts.writable {
        return Err(MoveProblem::NotWritable {
            path: target.to_path_buf(),
        });
    }
    if facts.free_bytes < needed_bytes {
        return Err(MoveProblem::NotEnoughSpace {
            needed_bytes,
            available_bytes: facts.free_bytes,
        });
    }
    Ok(())
}

/// Look at the target on disk.
pub fn target_facts(target: &Path) -> TargetFacts {
    let exists = target.is_dir();
    let is_empty = !exists || only_os_metadata(target);
    TargetFacts {
        exists,
        is_empty,
        writable: can_write_near(target),
        // An unknown free space must not block the move; a full disk then
        // fails the copy, which is cleaned up like any other failure.
        free_bytes: crate::util::system::available_disk_bytes(target).unwrap_or(u64::MAX),
    }
}

fn only_os_metadata(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .all(|e| OS_METADATA.iter().any(|name| e.file_name() == *name)),
        // Unreadable: do not treat it as empty and write into it.
        Err(_) => false,
    }
}

/// Probe by writing a file in the target, or in its nearest existing ancestor
/// when the target is still to be created.
fn can_write_near(target: &Path) -> bool {
    let Some(dir) = target.ancestors().find(|p| p.is_dir()) else {
        return false;
    };
    let probe = dir.join(".emailops-write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            if let Err(e) = std::fs::remove_file(&probe) {
                crate::services::logger::log("error", "system", format!("could not remove {}: {e}", probe.display()));
            }
            true
        }
        Err(_) => false,
    }
}

/// Resolve symlinks (`/var` → `/private/var` on macOS) through the nearest
/// existing ancestor, so a target that does not exist yet compares correctly
/// with the current folder.
fn normalize(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut base = path;
    loop {
        if let Ok(real) = base.canonicalize() {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (base.file_name(), base.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_os_string());
                base = parent;
            }
            _ => return path.components().filter(|c| !matches!(c, Component::CurDir)).collect(),
        }
    }
}

/// A file relative to the data folder, with its size.
type File = (PathBuf, u64);
/// A link relative to the data folder, with the absolute path it points to.
type Link = (PathBuf, PathBuf);

/// Every file to copy, relative to `root`, with its size, and every link with
/// what it points to. A link is a model adopted with "Use existing file…"
/// (`model_manager::link_local_model`): it moves as a link to the same file,
/// never as a copy of it, and a dangling one moves as it is.
fn files_to_copy(root: &Path) -> Result<(Vec<File>, Vec<Link>)> {
    let mut files = Vec::new();
    let mut links = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let dir = root.join(&rel);
        let entries =
            std::fs::read_dir(&dir).map_err(|e| AppError::IoError(format!("read_dir({}): {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| AppError::IoError(format!("read_dir({}): {e}", dir.display())))?;
            let name = entry.file_name();
            if rel.as_os_str().is_empty() && NOT_COPIED.iter().any(|n| name == *n) {
                continue;
            }
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path)
                .map_err(|e| AppError::IoError(format!("metadata({}): {e}", path.display())))?;
            if meta.file_type().is_symlink() {
                let points_to = std::fs::read_link(&path)
                    .map_err(|e| AppError::IoError(format!("read_link({}): {e}", path.display())))?;
                // A relative link would point somewhere else from the new folder.
                links.push((rel.join(&name), dir.join(points_to)));
            } else if meta.is_dir() {
                stack.push(rel.join(&name));
            } else if meta.is_file() {
                files.push((rel.join(&name), meta.len()));
            }
        }
    }
    Ok((files, links))
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// What a move from the current folder would copy.
struct CopyPlan {
    /// Every file besides the database, relative to the folder, with its size.
    files: Vec<(PathBuf, u64)>,
    links: Vec<Link>,
    /// The database's own bytes: the file plus its WAL.
    db_bytes: u64,
    total: u64,
}

fn measure(current: &Path) -> Result<CopyPlan> {
    let (files, links) = files_to_copy(current)?;
    let db_bytes = file_len(&current.join(DB_FILE)) + file_len(&current.join("emailops.db-wal"));
    let total = db_bytes + files.iter().map(|(_, len)| len).sum::<u64>();
    Ok(CopyPlan {
        files,
        links,
        db_bytes,
        total,
    })
}

/// Check that the data may move from `current` to `target` without copying
/// anything. Returns the bytes the move needs.
pub fn check_move(current: &Path, target: &Path) -> Result<u64> {
    let total = measure(current)?.total;
    plan_move(&normalize(current), &normalize(target), &target_facts(target), total)?;
    Ok(total)
}

/// Copy the data folder from `current` to `target` and point the next launch
/// at it. `progress(copied, total)` is called as bytes are copied.
pub fn move_data_dir(
    db: &Database,
    current: &Path,
    target: &Path,
    default_dir: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let plan = measure(current)?;
    plan_move(
        &normalize(current),
        &normalize(target),
        &target_facts(target),
        plan.total,
    )?;

    let target_existed = target.is_dir();
    let copied = copy_and_verify(db, current, target, &plan, progress)
        .and_then(|()| data_location::write_pointer(default_dir, target).map_err(|e| AppError::IoError(e.to_string())));
    if copied.is_err() {
        remove_partial_copy(target, target_existed);
    }
    copied
}

fn copy_and_verify(
    db: &Database,
    current: &Path,
    target: &Path,
    plan: &CopyPlan,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let CopyPlan {
        files,
        links,
        db_bytes,
        total,
    } = plan;
    let (db_bytes, total) = (*db_bytes, *total);
    let io = |what: &str, path: &Path, e: std::io::Error| AppError::IoError(format!("{what}({}): {e}", path.display()));
    std::fs::create_dir_all(target).map_err(|e| io("create_dir_all", target, e))?;
    // The default folder sits in the user's private Library; on another disk
    // nothing else keeps other users of this machine out of the mail.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| io("set_permissions", target, e))?;
    }

    let db_copy = target.join(DB_FILE);
    db.snapshot_to(&db_copy, &mut |pages, of| {
        if let Some(copied) = db_bytes.saturating_mul(pages).checked_div(of) {
            progress(copied, total);
        }
    })?;
    let conn = rusqlite::Connection::open(&db_copy)?;
    conn.execute(
        "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?1, ?2)",
        rusqlite::params![PREVIOUS_LOCATION_PREF, current.to_string_lossy()],
    )?;
    let mut done = db_bytes;
    progress(done, total);

    for (rel, len) in files {
        let (from, to) = (current.join(rel), target.join(rel));
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io("create_dir_all", parent, e))?;
        }
        std::fs::copy(&from, &to).map_err(|e| io("copy", &from, e))?;
        done += len;
        progress(done, total);
    }
    for (rel, points_to) in links {
        let to = target.join(rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io("create_dir_all", parent, e))?;
        }
        crate::util::fs_link::link_file(points_to, &to).map_err(|e| io("link", &to, e))?;
    }

    let mismatch = |detail: String| MoveProblem::CopyMismatch {
        path: target.to_path_buf(),
        detail,
    };
    for (rel, len) in files {
        let copied = file_len(&target.join(rel));
        if copied != *len {
            return Err(mismatch(format!("{} has {copied} of {len} bytes", rel.display())).into());
        }
    }
    for (rel, points_to) in links {
        // `link_file` may fall back to a hard link on Windows, which has no target to read.
        if let Ok(copied) = std::fs::read_link(target.join(rel)) {
            if copied != *points_to {
                return Err(mismatch(format!("{} points to {}", rel.display(), copied.display())).into());
            }
        }
    }
    let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(mismatch(format!("database check: {check}")).into());
    }
    Ok(())
}

/// Undo a failed copy. The target was empty before (the planner refuses
/// anything else), so everything in it but OS metadata is ours to remove.
fn remove_partial_copy(target: &Path, target_existed: bool) {
    let log = |path: &Path, e: std::io::Error| {
        crate::services::logger::log(
            "error",
            "system",
            format!("could not remove the partial copy {}: {e}", path.display()),
        );
    };
    if !target_existed {
        if let Err(e) = std::fs::remove_dir_all(target) {
            log(target, e);
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(target) else {
        return;
    };
    for entry in entries.flatten() {
        if OS_METADATA.iter().any(|name| entry.file_name() == *name) {
            continue;
        }
        let path = entry.path();
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(e) = removed {
            log(&path, e);
        }
    }
}
