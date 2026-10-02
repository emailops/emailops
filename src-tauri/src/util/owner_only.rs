//! Owner-only file-system permissions for the app's per-user data.
//!
//! The data directory holds the mail database, cached attachments and
//! backups. On macOS and Windows the per-user location is already private
//! (`~/Library` is 0700; `%APPDATA%` carries a user-only ACL), but on Linux
//! `~/.local/share` follows the umask, which commonly leaves new directories
//! world-readable. Restricting the data directory itself closes that for
//! every file beneath it (CASA/DASA 1.9.1). No-ops off Unix, where the
//! profile ACL already applies.

use std::path::Path;

/// Make `dir` accessible to its owner only (mode 0700 on Unix).
pub fn restrict_dir_to_owner(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Write `contents` to `path`, creating the file readable and writable by its
/// owner only (mode 0600 on Unix) — for files that may land outside the
/// private data directory, such as the temp-dir crash-report fallback.
pub fn write_owner_only(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    // `mode` only applies when the file is created; tighten one that existed.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(contents.as_bytes())
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    #[test]
    fn data_dir_becomes_owner_only() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("data");
        std::fs::create_dir(&dir).expect("mkdir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        restrict_dir_to_owner(&dir).expect("restrict");

        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn owner_only_file_is_not_readable_by_others() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("startup-error.log");

        write_owner_only(&path, "boom").expect("write");

        assert_eq!(mode(&path), 0o600);
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "boom");
    }

    #[test]
    fn rewriting_an_existing_file_tightens_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("startup-error.log");
        std::fs::write(&path, "old").expect("seed");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");

        write_owner_only(&path, "new").expect("write");

        assert_eq!(mode(&path), 0o600);
    }
}
