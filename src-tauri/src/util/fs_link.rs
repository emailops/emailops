//! Cross-platform file linking for "link a model already on disk".
//!
//! Linking exists so a user who already has a 5 GB GGUF somewhere does not get
//! a second copy in the app data directory. `std::os::unix::fs::symlink` is not
//! available on Windows, so calling it directly made the crate uncompilable
//! there — the same class of bug as the single-instance lock.
//!
//! Windows can create symlinks, but only with Developer Mode enabled or from an
//! elevated process; an ordinary user double-clicking the app has neither. A
//! hard link needs no privileges and serves the purpose just as well here:
//! it consumes no extra space, reports the target's size, and deleting it
//! leaves the user's original file untouched. So Windows tries a symlink first
//! and falls back to a hard link.
//!
//! Copying is deliberately NOT a fallback — silently duplicating several
//! gigabytes when the user asked to *link* would be a worse outcome than a
//! clear error.

use std::io;
use std::path::Path;

/// Create a link at `dest` pointing to `source`.
///
/// Returns the underlying I/O error if no link could be created. Callers should
/// surface [`link_failure_hint`] alongside it so the user knows what to do.
pub fn link_file(source: &Path, dest: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, dest)
    }
    #[cfg(windows)]
    {
        // Preferred: a real symlink, so `is_linked` reporting and dangling-link
        // detection behave exactly as they do on Unix.
        match std::os::windows::fs::symlink_file(source, dest) {
            Ok(()) => Ok(()),
            Err(symlink_err) => {
                // Almost always ERROR_PRIVILEGE_NOT_HELD. A hard link needs no
                // privileges, but only works within a single volume.
                match std::fs::hard_link(source, dest) {
                    Ok(()) => Ok(()),
                    // Report the symlink error, not the hard-link one: it is
                    // the actionable failure (enable Developer Mode), whereas
                    // the hard-link error is usually just "cross-device link".
                    Err(_) => Err(symlink_err),
                }
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (source, dest);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "linking files is not supported on this platform",
        ))
    }
}

/// Platform-appropriate remediation advice for a [`link_file`] failure.
///
/// Kept pure and OS-parameterised so the Windows wording is testable from any
/// host.
pub fn link_failure_hint(os: &str) -> &'static str {
    match os {
        "windows" => {
            "Windows needs Developer Mode enabled to create links \
             (Settings → System → For developers), or the file must be on the \
             same drive as EmailOps' data folder. You can also copy the model \
             into place instead of linking it."
        }
        _ => "Check that the file exists and that EmailOps can write to its data folder.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn source_file(dir: &Path, contents: &[u8]) -> std::path::PathBuf {
        let path = dir.join("source.gguf");
        let mut f = std::fs::File::create(&path).expect("create source");
        f.write_all(contents).expect("write source");
        path
    }

    /// The behaviour every platform must provide: after linking, the
    /// destination reads back the source's bytes without a second copy.
    #[test]
    fn linked_file_exposes_the_source_contents() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let source = source_file(tmp.path(), b"model weights");
        let dest = tmp.path().join("linked.gguf");

        link_file(&source, &dest).expect("link should succeed");

        assert_eq!(
            std::fs::read(&dest).expect("read through link"),
            b"model weights",
            "the link must resolve to the source's contents"
        );
    }

    #[test]
    fn linked_file_reports_the_targets_size() {
        // `list_local_models` relies on `metadata` following the link so a
        // linked model shows its real size rather than 0.
        let tmp = tempfile::tempdir().expect("temp dir");
        let source = source_file(tmp.path(), &[7u8; 4096]);
        let dest = tmp.path().join("linked.gguf");

        link_file(&source, &dest).expect("link should succeed");

        let meta = std::fs::metadata(&dest).expect("metadata");
        assert_eq!(meta.len(), 4096);
    }

    #[test]
    fn removing_the_link_preserves_the_source() {
        // The whole point of linking: deleting a linked model from EmailOps
        // must never destroy the user's own file.
        let tmp = tempfile::tempdir().expect("temp dir");
        let source = source_file(tmp.path(), b"precious");
        let dest = tmp.path().join("linked.gguf");
        link_file(&source, &dest).expect("link should succeed");

        std::fs::remove_file(&dest).expect("remove link");

        assert!(source.is_file(), "source must survive removal of the link");
        assert_eq!(std::fs::read(&source).expect("read source"), b"precious");
    }

    #[test]
    fn linking_onto_an_existing_path_fails() {
        // Callers clear `dest` first; this pins the fact that they must.
        let tmp = tempfile::tempdir().expect("temp dir");
        let source = source_file(tmp.path(), b"a");
        let dest = tmp.path().join("taken.gguf");
        std::fs::write(&dest, b"b").expect("occupy dest");

        assert!(link_file(&source, &dest).is_err());
    }

    #[test]
    fn windows_hint_names_developer_mode() {
        let hint = link_failure_hint("windows");
        assert!(hint.contains("Developer Mode"), "got {hint}");
        assert!(hint.contains("same drive"), "hard-link limitation must be explained");
    }

    #[test]
    fn other_platforms_get_a_generic_hint() {
        for os in ["macos", "linux"] {
            let hint = link_failure_hint(os);
            assert!(
                !hint.contains("Developer Mode"),
                "{os} should not mention Windows settings"
            );
            assert!(!hint.is_empty());
        }
    }
}
