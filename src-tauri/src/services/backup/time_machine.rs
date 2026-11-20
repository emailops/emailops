//! Keep the full database copies out of Time Machine: Time Machine already
//! backs up the live database, and every full copy is as large as it.

use std::path::Path;

use crate::models::error::Result;

/// Mark `dir` so Time Machine skips it and everything in it. `tmutil` stores
/// the mark as an extended attribute on the folder, so it needs no
/// administrator rights and moves with the data folder.
#[cfg(target_os = "macos")]
pub fn exclude(dir: &Path) -> Result<()> {
    use crate::models::error::AppError;
    let out = std::process::Command::new("/usr/bin/tmutil")
        .arg("addexclusion")
        .arg(dir)
        .output()
        .map_err(|e| AppError::IoError(format!("Could not run tmutil: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(AppError::IoError(format!(
            "tmutil could not keep the backups folder out of Time Machine: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// Other platforms have no Time Machine to keep the copies out of.
#[cfg(not(target_os = "macos"))]
pub fn exclude(_dir: &Path) -> Result<()> {
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn an_excluded_folder_is_skipped_by_time_machine() {
        let dir = tempfile::tempdir().expect("dir");

        super::exclude(dir.path()).expect("exclude");

        let out = std::process::Command::new("/usr/bin/tmutil")
            .arg("isexcluded")
            .arg(dir.path())
            .output()
            .expect("tmutil isexcluded");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("[Excluded]"),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}
