//! Cross-platform single-instance lock.
//!
//! A second launch must notice that EmailOps is already running and exit,
//! rather than opening a second window against the same SQLite file. The lock
//! is held by an open file handle for the lifetime of the process; every
//! supported OS releases it automatically when the process exits, including
//! after a crash.
//!
//! Portability lives entirely in `fd_lock`, which maps to `flock(2)` on Unix
//! and `LockFileEx` on Windows. The previous implementation called
//! `libc::flock` through `std::os::unix::io::AsRawFd` directly, which does not
//! exist on Windows — the crate did not compile there at all.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// File name of the lock, inside the app data directory.
const LOCK_FILE_NAME: &str = "emailops.lock";

/// Why the single-instance lock could not be taken.
///
/// The two variants demand different responses, which is why this is not a
/// bare `String`: `AlreadyRunning` is the expected, benign outcome of
/// double-clicking the app icon and should exit quietly, while `Unavailable`
/// means the data directory is genuinely unusable and the user needs to be
/// told. The old code collapsed both into one string and exited 0 for each,
/// so a read-only or full data directory looked exactly like a normal
/// second launch.
#[derive(Debug)]
pub enum LockError {
    /// Another EmailOps process holds the lock.
    AlreadyRunning,
    /// The lock file could not be created, opened, or locked.
    Unavailable(String),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning => {
                write!(f, "Another instance of EmailOps is already running. Exiting.")
            }
            Self::Unavailable(detail) => write!(f, "{detail}"),
        }
    }
}

/// Holds the lock for the lifetime of the process.
///
/// The guard borrows a deliberately leaked `fd_lock::RwLock`, which is what
/// makes it `'static` and therefore storable in Tauri's managed state. Leaking
/// one file handle is correct here rather than wasteful: the lock must outlive
/// every other startup step and is only meant to be released by process exit.
pub struct InstanceLock {
    _guard: fd_lock::RwLockWriteGuard<'static, File>,
    path: PathBuf,
}

impl InstanceLock {
    /// Path of the lock file being held. Used by tests and diagnostics.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Debug for InstanceLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceLock").field("path", &self.path).finish()
    }
}

/// Resolve the lock file path for an app data directory.
pub fn lock_file_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(LOCK_FILE_NAME)
}

/// Try to become the single running instance.
///
/// Returns `Err(LockError::AlreadyRunning)` when another process holds the
/// lock, and `Err(LockError::Unavailable)` when the lock file itself could not
/// be created or locked.
pub fn acquire(app_data_dir: &Path) -> Result<InstanceLock, LockError> {
    std::fs::create_dir_all(app_data_dir)
        .map_err(|e| LockError::Unavailable(format!("Cannot create app data dir: {e}")))?;

    let lock_path = lock_file_path(app_data_dir);

    // Deliberately no `.truncate(true)`: on Windows, opening with truncation
    // while another handle holds a byte-range lock fails at `open` time with a
    // lock violation, which would surface as `Unavailable` instead of the
    // `AlreadyRunning` this case actually is. Truncate after locking instead.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| LockError::Unavailable(format!("Cannot open lock file: {e}")))?;

    // The guard must outlive this function and every later startup step, so the
    // `RwLock` it borrows is leaked to obtain a `'static` lifetime.
    let lock: &'static mut fd_lock::RwLock<File> = Box::leak(Box::new(fd_lock::RwLock::new(file)));

    let mut guard = match lock.try_write() {
        Ok(guard) => guard,
        Err(e) if e.kind() == ErrorKind::WouldBlock => return Err(LockError::AlreadyRunning),
        Err(e) => {
            return Err(LockError::Unavailable(format!(
                "Cannot lock {}: {e}",
                lock_path.display()
            )))
        }
    };

    // Record our PID so `cat emailops.lock` identifies the holder. Best-effort:
    // the lock is already held at this point, so a write failure must not fail
    // startup — but it does get logged rather than silently dropped.
    if let Err(e) = write_pid(&mut guard) {
        eprintln!(
            "[startup] Warning: could not record PID in {}: {e}",
            lock_path.display()
        );
    }

    Ok(InstanceLock {
        _guard: guard,
        path: lock_path,
    })
}

/// Overwrite the lock file's contents with the current PID.
fn write_pid(file: &mut File) -> std::io::Result<()> {
    file.set_len(0)?;
    file.write_all(std::process::id().to_string().as_bytes())?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_file_lives_inside_the_app_data_dir() {
        let dir = Path::new("some").join("data").join("dir");
        let path = lock_file_path(&dir);
        assert_eq!(path.parent(), Some(dir.as_path()));
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some(LOCK_FILE_NAME));
    }

    #[test]
    fn acquiring_creates_a_missing_data_dir() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let nested = tmp.path().join("not").join("created").join("yet");
        let lock = acquire(&nested).expect("first acquire should succeed");
        assert!(nested.is_dir(), "acquire must create the data dir");
        assert!(lock.path().is_file(), "lock file must exist");
    }

    #[test]
    fn first_acquire_succeeds_and_records_the_pid() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let lock = acquire(tmp.path()).expect("first acquire should succeed");

        let contents = std::fs::read_to_string(lock.path()).expect("lock file readable");
        assert_eq!(
            contents.trim(),
            std::process::id().to_string(),
            "lock file should name the holding process"
        );
    }

    /// The behaviour the whole module exists for. This test is the real
    /// cross-platform gate: it exercises `flock` on Unix and `LockFileEx` on
    /// Windows, so running it on a Windows CI runner is what proves the
    /// Windows path works.
    #[test]
    fn second_acquire_reports_already_running() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let _held = acquire(tmp.path()).expect("first acquire should succeed");

        match acquire(tmp.path()) {
            Err(LockError::AlreadyRunning) => {}
            Err(other) => panic!("expected AlreadyRunning, got {other:?}"),
            Ok(_) => panic!("second acquire must not succeed while the first is held"),
        }
    }

    #[test]
    fn releasing_allows_a_later_acquire() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let first = acquire(tmp.path()).expect("first acquire should succeed");
        drop(first);

        acquire(tmp.path()).expect("acquire should succeed once the previous lock is dropped");
    }

    #[test]
    fn already_running_and_unavailable_render_differently() {
        let running = LockError::AlreadyRunning.to_string();
        assert!(running.contains("already running"), "got {running}");

        let unavailable = LockError::Unavailable("disk full".into()).to_string();
        assert!(unavailable.contains("disk full"), "got {unavailable}");
        assert!(
            !unavailable.contains("already running"),
            "a real I/O failure must not be reported as a second instance"
        );
    }
}
