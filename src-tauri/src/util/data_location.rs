//! Where EmailOps keeps its data folder.
//!
//! The data folder holds everything: the SQLite database, `models/`,
//! `attachments/`, `backups/`, `skills/` and the instance lock. The user can
//! move it (Settings → Privacy, or the first onboarding step), so its location
//! cannot live inside the database: it has to be known before the database is
//! opened. It is kept in a small pointer file next to the platform default
//! data folder (`com.emailops.app.data-location.json` beside
//! `com.emailops.app/`), the one place the app can always find on launch.
//! Beside it, not inside it: after moving away from the default the user is
//! told to delete the old folder, and that must not take the pointer with it.
//!
//! Resolution order, shared by the desktop app and `emailops-cli`:
//! `--data-dir` (CLI only) → `EMAILOPS_DATA_DIR` → pointer file → default.

use std::path::{Path, PathBuf};

/// Suffix of the pointer's file name, appended to the default folder's name.
const POINTER_SUFFIX: &str = ".data-location.json";

/// The pointer file for `default_dir`: a sibling named after it.
pub fn pointer_file(default_dir: &Path) -> PathBuf {
    let mut name = default_dir.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(POINTER_SUFFIX);
    default_dir.with_file_name(name)
}

/// Which input decided the data folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataDirSource {
    /// The CLI `--data-dir` flag.
    Flag,
    /// The `EMAILOPS_DATA_DIR` environment variable.
    Env,
    /// A folder the user chose from the app.
    Pointer,
    /// The platform default for this app id.
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDataDir {
    pub path: PathBuf,
    pub source: DataDirSource,
}

/// Pick the data folder from its inputs. Pure: the caller reads the env var
/// and the pointer file. A blank env var counts as unset.
pub fn resolve(
    flag: Option<PathBuf>,
    env: Option<String>,
    pointer: Option<PathBuf>,
    default: PathBuf,
) -> ResolvedDataDir {
    if let Some(path) = flag {
        return ResolvedDataDir {
            path,
            source: DataDirSource::Flag,
        };
    }
    if let Some(dir) = env.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        return ResolvedDataDir {
            path: PathBuf::from(dir),
            source: DataDirSource::Env,
        };
    }
    if let Some(path) = pointer {
        return ResolvedDataDir {
            path,
            source: DataDirSource::Pointer,
        };
    }
    ResolvedDataDir {
        path: default,
        source: DataDirSource::Default,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PointerError {
    #[error("could not read the data location file {path}: {detail}")]
    Unreadable { path: PathBuf, detail: String },
    #[error("could not save the data location file {path}: {detail}")]
    Unwritable { path: PathBuf, detail: String },
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Pointer {
    path: PathBuf,
}

/// The folder the user chose, if any. A missing pointer file means "the
/// default". A malformed one is an error rather than a silent fallback: falling
/// back would open an empty database and look as if all mail had vanished.
pub fn read_pointer(default_dir: &Path) -> Result<Option<PathBuf>, PointerError> {
    let file = pointer_file(default_dir);
    let unreadable = |detail: String| PointerError::Unreadable {
        path: file.clone(),
        detail,
    };
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(unreadable(e.to_string())),
    };
    let pointer: Pointer = serde_json::from_str(&text).map_err(|e| unreadable(e.to_string()))?;
    if !pointer.path.is_absolute() {
        return Err(unreadable(format!(
            "{} is not an absolute path",
            pointer.path.display()
        )));
    }
    Ok(Some(pointer.path))
}

/// Point the app at `target` from the next launch. Choosing the default folder
/// removes the pointer instead, so the default needs no file at all.
pub fn write_pointer(default_dir: &Path, target: &Path) -> Result<(), PointerError> {
    let file = pointer_file(default_dir);
    let unwritable = |detail: String| PointerError::Unwritable {
        path: file.clone(),
        detail,
    };
    if target == default_dir {
        return match std::fs::remove_file(&file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(unwritable(e.to_string())),
        };
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| unwritable(e.to_string()))?;
    }
    let json = serde_json::to_string_pretty(&Pointer {
        path: target.to_path_buf(),
    })
    .map_err(|e| unwritable(e.to_string()))?;
    // Write a sibling and rename over the pointer, so a crash mid-write never
    // leaves a truncated file that would block the next launch.
    let mut tmp_name = file.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = file.with_file_name(tmp_name);
    std::fs::write(&tmp, json).map_err(|e| unwritable(e.to_string()))?;
    std::fs::rename(&tmp, &file).map_err(|e| unwritable(e.to_string()))
}

/// A chosen folder that is gone (an unplugged disk, a deleted folder) must stop
/// the launch: creating it would put an empty database on the system disk, or
/// under an unmounted volume's mount point. Only a chosen folder is checked —
/// the default and the dev overrides are created on demand as before.
pub fn unavailable_chosen_dir(resolved: &ResolvedDataDir) -> Option<&Path> {
    (resolved.source == DataDirSource::Pointer && !resolved.path.is_dir()).then_some(resolved.path.as_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_dir() -> PathBuf {
        PathBuf::from("/default/com.emailops.app")
    }

    #[test]
    fn flag_wins_over_everything() {
        let resolved = resolve(
            Some(PathBuf::from("/flag")),
            Some("/env".into()),
            Some(PathBuf::from("/chosen")),
            default_dir(),
        );
        assert_eq!(
            resolved,
            ResolvedDataDir {
                path: "/flag".into(),
                source: DataDirSource::Flag
            }
        );
    }

    #[test]
    fn env_wins_over_the_pointer() {
        let resolved = resolve(
            None,
            Some(" /env ".into()),
            Some(PathBuf::from("/chosen")),
            default_dir(),
        );
        assert_eq!(
            resolved,
            ResolvedDataDir {
                path: "/env".into(),
                source: DataDirSource::Env
            }
        );
    }

    #[test]
    fn blank_env_falls_through_to_the_pointer() {
        let resolved = resolve(None, Some("  ".into()), Some(PathBuf::from("/chosen")), default_dir());
        assert_eq!(
            resolved,
            ResolvedDataDir {
                path: "/chosen".into(),
                source: DataDirSource::Pointer
            }
        );
    }

    #[test]
    fn no_inputs_means_the_default() {
        let resolved = resolve(None, None, None, default_dir());
        assert_eq!(
            resolved,
            ResolvedDataDir {
                path: default_dir(),
                source: DataDirSource::Default
            }
        );
    }

    /// A default data folder inside a scratch parent, like
    /// `~/Library/Application Support/com.emailops.app`.
    fn scratch_default() -> (tempfile::TempDir, PathBuf) {
        let parent = tempfile::tempdir().unwrap();
        let default = parent.path().join("com.emailops.app");
        (parent, default)
    }

    #[test]
    fn pointer_sits_beside_the_default_folder() {
        assert_eq!(
            pointer_file(Path::new("/support/com.emailops.app")),
            PathBuf::from("/support/com.emailops.app.data-location.json")
        );
    }

    #[test]
    fn missing_pointer_file_reads_as_none() {
        let (_parent, default) = scratch_default();
        assert_eq!(read_pointer(&default).unwrap(), None);
    }

    #[test]
    fn deleting_the_default_folder_keeps_the_pointer() {
        let (parent, default) = scratch_default();
        std::fs::create_dir_all(&default).unwrap();
        let target = parent.path().join("elsewhere");
        write_pointer(&default, &target).unwrap();
        std::fs::remove_dir_all(&default).unwrap();
        assert_eq!(read_pointer(&default).unwrap(), Some(target));
    }

    #[test]
    fn written_pointer_reads_back() {
        let (parent, default) = scratch_default();
        let target = parent.path().join("elsewhere");
        write_pointer(&default, &target).unwrap();
        assert_eq!(read_pointer(&default).unwrap(), Some(target));
    }

    #[test]
    fn choosing_the_default_removes_the_pointer() {
        let (parent, default) = scratch_default();
        write_pointer(&default, &parent.path().join("elsewhere")).unwrap();
        write_pointer(&default, &default).unwrap();
        assert!(!pointer_file(&default).exists());
        assert_eq!(read_pointer(&default).unwrap(), None);
    }

    #[test]
    fn malformed_pointer_is_an_error_not_the_default() {
        let (_parent, default) = scratch_default();
        std::fs::write(pointer_file(&default), "{ not json").unwrap();
        assert!(matches!(read_pointer(&default), Err(PointerError::Unreadable { .. })));
    }

    #[test]
    fn relative_pointer_path_is_an_error() {
        let (_parent, default) = scratch_default();
        std::fs::write(pointer_file(&default), r#"{"path":"relative/dir"}"#).unwrap();
        assert!(matches!(read_pointer(&default), Err(PointerError::Unreadable { .. })));
    }

    #[test]
    fn missing_chosen_folder_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("unplugged-disk");
        let resolved = ResolvedDataDir {
            path: gone.clone(),
            source: DataDirSource::Pointer,
        };
        assert_eq!(unavailable_chosen_dir(&resolved), Some(gone.as_path()));
    }

    #[test]
    fn existing_chosen_folder_is_available() {
        let dir = tempfile::tempdir().unwrap();
        let resolved = ResolvedDataDir {
            path: dir.path().to_path_buf(),
            source: DataDirSource::Pointer,
        };
        assert_eq!(unavailable_chosen_dir(&resolved), None);
    }

    #[test]
    fn missing_default_folder_is_created_on_demand_not_unavailable() {
        let resolved = ResolvedDataDir {
            path: "/does/not/exist".into(),
            source: DataDirSource::Default,
        };
        assert_eq!(unavailable_chosen_dir(&resolved), None);
    }
}
