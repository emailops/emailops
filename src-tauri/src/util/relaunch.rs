//! Start a fresh copy of the app as this one exits (after moving the data
//! folder, which only takes effect on the next launch).
//!
//! Tauri's own `AppHandle::restart`, called from a command thread, relaunches
//! only after `RunEvent::Exit` has been handled — and our `Exit` handler never
//! returns: it `_exit`s to dodge ggml's Metal abort in C++ destructors
//! (`services::ai::shutdown_and_exit`). So the restart is requested here and
//! carried out by that handler, right before it exits.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Ask for a relaunch when the app exits.
pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// How to start the app again from the binary that is running now. Inside a
/// macOS bundle the app goes through LaunchServices (`open -n`), so it starts
/// as a normal app with its own session; anywhere else the binary itself runs.
pub fn plan(current_binary: &Path) -> (PathBuf, Vec<OsString>) {
    if let Some(bundle) = macos_bundle(current_binary) {
        return (
            PathBuf::from("/usr/bin/open"),
            vec!["-n".into(), bundle.into_os_string()],
        );
    }
    (current_binary.to_path_buf(), Vec::new())
}

/// `Foo.app` for `Foo.app/Contents/MacOS/foo`.
fn macos_bundle(binary: &Path) -> Option<PathBuf> {
    let macos = binary.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension().is_some_and(|ext| ext == "app");
    is_bundle.then(|| bundle.to_path_buf())
}

/// Start the new copy, detached from this process's stdio: ours goes away in
/// a moment, and a child writing to a closed pipe would die on its first print.
pub fn spawn(current_binary: &Path) -> std::io::Result<()> {
    let (program, args) = plan(current_binary);
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_macos_bundle_relaunches_through_launch_services() {
        let (program, args) = plan(Path::new("/Applications/EmailOps.app/Contents/MacOS/emailops"));
        assert_eq!(program, PathBuf::from("/usr/bin/open"));
        assert_eq!(
            args,
            vec![OsString::from("-n"), OsString::from("/Applications/EmailOps.app")]
        );
    }

    #[test]
    fn a_bare_binary_runs_itself_again() {
        let (program, args) = plan(Path::new("/opt/emailops/emailops"));
        assert_eq!(program, PathBuf::from("/opt/emailops/emailops"));
        assert!(args.is_empty());
    }

    #[test]
    fn a_macos_folder_outside_a_bundle_is_not_a_bundle() {
        let (program, _) = plan(Path::new("/tmp/Contents/MacOS/emailops"));
        assert_eq!(program, PathBuf::from("/tmp/Contents/MacOS/emailops"));
    }

    #[test]
    fn nothing_is_requested_until_asked() {
        // The flag is process-global; this is the only test that touches it.
        assert!(!requested());
        request();
        assert!(requested());
    }
}
