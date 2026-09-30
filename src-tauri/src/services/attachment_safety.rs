//! Safety of attachment files handed to the operating system.
//!
//! An attachment is written under the sender's own extension, so opening it
//! with the default app does whatever that type does. Two protections live
//! here:
//!
//! - [`quarantine`] marks a file as received from outside, so the platform's
//!   own first-open checks apply (Gatekeeper on macOS, Mark-of-the-Web on
//!   Windows). Only attachment files are marked, never the app's other files.
//! - [`classify`] names the types that run code or open another location when
//!   opened, and [`open_attachment_file`] refuses those without an explicit
//!   confirmation from the user.

use std::path::Path;

use crate::models::error::{AppError, Result};

/// Why opening an attachment needs the user's confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DangerKind {
    /// Runs as a program.
    Program,
    /// Runs commands through an interpreter or changes system settings.
    Script,
    /// Installs software or mounts a volume that can.
    Installer,
    /// Opens another file, app or address.
    Shortcut,
    /// Opens in the browser as a local page, outside the app's sanitizer.
    WebPage,
}

impl DangerKind {
    /// Stable identifier sent to the frontend (translation key suffix).
    pub fn as_str(self) -> &'static str {
        match self {
            DangerKind::Program => "program",
            DangerKind::Script => "script",
            DangerKind::Installer => "installer",
            DangerKind::Shortcut => "shortcut",
            DangerKind::WebPage => "web_page",
        }
    }
}

/// Extensions whose default action, on a double-click or `open`, runs code or
/// opens a location the file names — on macOS, Windows or a Linux desktop.
/// That is the only criterion: a type belongs here when *opening* it is
/// enough to act, and not when it is merely a document that a vulnerable
/// reader could mishandle (PDF, Office files, archives). Lowercase, no dot.
const DANGEROUS_EXTENSIONS: &[(&str, DangerKind)] = &[
    // Programs.
    ("app", DangerKind::Program),
    ("exe", DangerKind::Program),
    ("com", DangerKind::Program),
    ("scr", DangerKind::Program),
    ("pif", DangerKind::Program),
    ("jar", DangerKind::Program),
    ("appimage", DangerKind::Program),
    ("run", DangerKind::Program),
    // Scripts and files that carry commands for an interpreter.
    ("command", DangerKind::Script),
    ("terminal", DangerKind::Script),
    ("tool", DangerKind::Script),
    ("workflow", DangerKind::Script),
    ("action", DangerKind::Script),
    ("scpt", DangerKind::Script),
    ("scptd", DangerKind::Script),
    ("applescript", DangerKind::Script),
    ("sh", DangerKind::Script),
    ("zsh", DangerKind::Script),
    ("bash", DangerKind::Script),
    ("py", DangerKind::Script),
    ("rb", DangerKind::Script),
    ("pl", DangerKind::Script),
    ("bat", DangerKind::Script),
    ("cmd", DangerKind::Script),
    ("ps1", DangerKind::Script),
    ("vbs", DangerKind::Script),
    ("js", DangerKind::Script),
    ("jse", DangerKind::Script),
    ("wsf", DangerKind::Script),
    ("hta", DangerKind::Script),
    ("reg", DangerKind::Script),
    // Installers and disk images.
    ("pkg", DangerKind::Installer),
    ("mpkg", DangerKind::Installer),
    ("dmg", DangerKind::Installer),
    ("msi", DangerKind::Installer),
    ("deb", DangerKind::Installer),
    ("rpm", DangerKind::Installer),
    // Shortcuts to another file, app or address.
    ("fileloc", DangerKind::Shortcut),
    ("inetloc", DangerKind::Shortcut),
    ("webloc", DangerKind::Shortcut),
    ("url", DangerKind::Shortcut),
    ("lnk", DangerKind::Shortcut),
    ("desktop", DangerKind::Shortcut),
    // Local pages: scripts in them run in the browser with a file origin.
    ("html", DangerKind::WebPage),
    ("htm", DangerKind::WebPage),
    ("xhtml", DangerKind::WebPage),
    ("svg", DangerKind::WebPage),
];

/// MIME types that declare one of the kinds above. Checked after the name, so
/// a file the sender labelled as a program is confirmed even under a name
/// that says nothing. Lowercase.
const DANGEROUS_MIME_TYPES: &[(&str, DangerKind)] = &[
    ("application/x-msdownload", DangerKind::Program),
    ("application/x-msdos-program", DangerKind::Program),
    ("application/vnd.microsoft.portable-executable", DangerKind::Program),
    ("application/x-executable", DangerKind::Program),
    ("application/x-mach-binary", DangerKind::Program),
    ("application/java-archive", DangerKind::Program),
    ("application/x-sh", DangerKind::Script),
    ("application/x-shellscript", DangerKind::Script),
    ("text/x-shellscript", DangerKind::Script),
    ("application/x-applescript", DangerKind::Script),
    ("application/javascript", DangerKind::Script),
    ("text/javascript", DangerKind::Script),
    ("application/hta", DangerKind::Script),
    ("application/x-apple-diskimage", DangerKind::Installer),
    ("application/x-msi", DangerKind::Installer),
    ("application/vnd.apple.installer+xml", DangerKind::Installer),
    ("application/x-ms-shortcut", DangerKind::Shortcut),
    ("application/internet-shortcut", DangerKind::Shortcut),
    ("text/html", DangerKind::WebPage),
    ("application/xhtml+xml", DangerKind::WebPage),
    ("image/svg+xml", DangerKind::WebPage),
];

/// Classify an attachment by its filename and, when known, its MIME type.
/// `None` means opening it needs no confirmation.
///
/// The last extension of the last path component decides, compared without
/// case and after dropping trailing dots, spaces and control characters —
/// Windows removes those when it writes the file, so `setup.exe.` lands on
/// disk as `setup.exe`.
pub fn classify(filename: &str, mime_type: Option<&str>) -> Option<DangerKind> {
    classify_name(filename).or_else(|| mime_type.and_then(classify_mime))
}

fn classify_name(filename: &str) -> Option<DangerKind> {
    let name = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let name = name.trim_end_matches(|c: char| c == '.' || c.is_whitespace() || c.is_control());
    let (_, extension) = name.rsplit_once('.')?;
    let extension = extension.to_lowercase();
    DANGEROUS_EXTENSIONS
        .iter()
        .find(|(listed, _)| *listed == extension)
        .map(|(_, kind)| *kind)
}

fn classify_mime(mime_type: &str) -> Option<DangerKind> {
    // Drop parameters: `text/html; charset=utf-8`.
    let essence = mime_type.split(';').next().unwrap_or(mime_type).trim().to_lowercase();
    DANGEROUS_MIME_TYPES
        .iter()
        .find(|(listed, _)| *listed == essence)
        .map(|(_, kind)| *kind)
}

/// Name recorded as the agent that received the file.
const QUARANTINE_AGENT: &str = "EmailOps";

/// `0x0001` (downloaded) | `0x0080`: the flags a non-sandboxed downloader
/// leaves on a file, without `0x0040` (user approved) — so Gatekeeper
/// assesses the file the first time it is opened.
const QUARANTINE_FLAGS: u16 = 0x0081;

/// The `com.apple.quarantine` value for a file received at `timestamp`
/// (seconds since the epoch), identified by `event_id`:
/// `flags;hex timestamp;agent;event UUID`.
pub fn quarantine_value(timestamp: i64, event_id: &str) -> String {
    format!("{QUARANTINE_FLAGS:04x};{timestamp:x};{QUARANTINE_AGENT};{event_id}")
}

/// Mark `path` as received from outside so the platform checks it on first
/// open. A file that already carries a mark keeps it: macOS records the
/// user's approval in the same attribute, and replacing it would ask again.
#[cfg(target_os = "macos")]
pub fn quarantine(path: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    const ATTRIBUTE: &std::ffi::CStr = c"com.apple.quarantine";
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

    // SAFETY: both pointers are NUL-terminated strings that outlive the call;
    // a null buffer of size 0 only asks for the attribute's length.
    let existing = unsafe { libc::getxattr(c_path.as_ptr(), ATTRIBUTE.as_ptr(), std::ptr::null_mut(), 0, 0, 0) };
    if existing >= 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() != Some(libc::ENOATTR) {
        return Err(err);
    }

    let event_id = uuid::Uuid::new_v4().to_string().to_uppercase();
    let value = quarantine_value(chrono::Utc::now().timestamp(), &event_id);
    // SAFETY: as above; `value` is a live buffer of exactly `value.len()` bytes.
    let rc = unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            ATTRIBUTE.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Mark `path` as received from outside so the platform checks it on first
/// open: the Mark-of-the-Web, a `Zone.Identifier` alternate data stream
/// naming the Internet zone (3), which SmartScreen and Office read.
#[cfg(windows)]
pub fn quarantine(path: &Path) -> std::io::Result<()> {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\n")
}

/// Linux (and other platforms) have no file-level "received from outside"
/// mark that desktops act on, so there is nothing to write. The confirmation
/// in [`open_attachment_file`] is the protection there.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn quarantine(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Mark a file the app just saved. A failure is reported to the output panel
/// and the save stands: the file is not opened here, and
/// [`open_attachment_file`] marks it again (and refuses on failure) before
/// any open.
pub fn quarantine_saved_file(path: &Path) {
    if let Err(e) = quarantine(path) {
        crate::services::logger::log(
            "error",
            "system",
            format!("Could not mark '{}' as received by email: {e}", display_name(path)),
        );
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// Open a stored attachment with the default app.
///
/// `filename` and `mime_type` are what the email declared; `path` is the file
/// on disk, whose own name is classified as well because that is the name the
/// OS acts on. A dangerous type is refused with
/// [`AppError::AttachmentConfirmationRequired`] unless `confirmed`. No file
/// is opened without the quarantine mark: if it cannot be written the open is
/// refused, since the OS would then launch the file unchecked. `launch` is
/// the OS hand-off (`open::that` in production).
pub fn open_attachment_file(
    path: &Path,
    filename: &str,
    mime_type: Option<&str>,
    confirmed: bool,
    launch: &dyn Fn(&Path) -> std::io::Result<()>,
) -> Result<()> {
    use crate::services::logger::log;

    let kind = classify(filename, mime_type).or_else(|| classify(&display_name(path), None));
    if let Some(kind) = kind {
        if !confirmed {
            return Err(AppError::AttachmentConfirmationRequired {
                filename: filename.to_string(),
                kind: kind.as_str(),
            });
        }
    }

    if let Err(e) = quarantine(path) {
        let message = format!("'{filename}' was not opened: it could not be marked as received by email ({e})");
        log("error", "system", message.clone());
        return Err(AppError::IoError(message));
    }

    match kind {
        Some(kind) => log(
            "info",
            "system",
            format!(
                "Opening attachment '{filename}' ({}) in the default app after confirmation",
                kind.as_str()
            ),
        ),
        None => log(
            "info",
            "system",
            format!("Opening attachment '{filename}' in the default app"),
        ),
    }
    launch(path).map_err(|e| {
        log(
            "error",
            "system",
            format!("Could not open attachment '{filename}': {e}"),
        );
        AppError::IoError(format!("Failed to open file: {e}"))
    })
}

/// The quarantine mark on `path` as `xattr` reports it, for tests; `None`
/// also when `xattr` cannot be run, which fails the assertions on it.
#[cfg(all(test, target_os = "macos"))]
pub(crate) fn read_quarantine(path: &Path) -> Option<String> {
    let out = std::process::Command::new("/usr/bin/xattr")
        .args(["-p", "com.apple.quarantine"])
        .arg(path)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;

    #[test]
    fn classifies_types_that_act_when_opened() {
        let cases: &[(&str, Option<&str>, Option<DangerKind>)] = &[
            // Benign documents.
            ("report.pdf", Some("application/pdf"), None),
            ("photo.JPG", Some("image/jpeg"), None),
            ("notes.txt", None, None),
            ("archive.zip", Some("application/zip"), None),
            ("sheet.xlsx", None, None),
            ("README", None, None),
            ("", None, None),
            // macOS launchers.
            ("Installer.app", None, Some(DangerKind::Program)),
            ("run.command", None, Some(DangerKind::Script)),
            ("profile.terminal", None, Some(DangerKind::Script)),
            ("build.tool", None, Some(DangerKind::Script)),
            ("resize.workflow", None, Some(DangerKind::Script)),
            ("resize.action", None, Some(DangerKind::Script)),
            ("hello.scpt", None, Some(DangerKind::Script)),
            ("hello.scptd", None, Some(DangerKind::Script)),
            ("hello.applescript", None, Some(DangerKind::Script)),
            ("setup.pkg", None, Some(DangerKind::Installer)),
            ("setup.mpkg", None, Some(DangerKind::Installer)),
            ("setup.dmg", None, Some(DangerKind::Installer)),
            ("tool.jar", None, Some(DangerKind::Program)),
            ("share.fileloc", None, Some(DangerKind::Shortcut)),
            ("share.inetloc", None, Some(DangerKind::Shortcut)),
            ("site.webloc", None, Some(DangerKind::Shortcut)),
            ("site.url", None, Some(DangerKind::Shortcut)),
            ("deploy.sh", None, Some(DangerKind::Script)),
            ("deploy.zsh", None, Some(DangerKind::Script)),
            ("deploy.bash", None, Some(DangerKind::Script)),
            ("deploy.py", None, Some(DangerKind::Script)),
            ("deploy.rb", None, Some(DangerKind::Script)),
            ("deploy.pl", None, Some(DangerKind::Script)),
            ("page.html", None, Some(DangerKind::WebPage)),
            ("page.htm", None, Some(DangerKind::WebPage)),
            ("page.xhtml", None, Some(DangerKind::WebPage)),
            ("logo.svg", None, Some(DangerKind::WebPage)),
            // Windows.
            ("setup.exe", None, Some(DangerKind::Program)),
            ("setup.msi", None, Some(DangerKind::Installer)),
            ("go.bat", None, Some(DangerKind::Script)),
            ("go.cmd", None, Some(DangerKind::Script)),
            ("go.com", None, Some(DangerKind::Program)),
            ("saver.scr", None, Some(DangerKind::Program)),
            ("old.pif", None, Some(DangerKind::Program)),
            ("docs.lnk", None, Some(DangerKind::Shortcut)),
            ("go.ps1", None, Some(DangerKind::Script)),
            ("go.vbs", None, Some(DangerKind::Script)),
            ("go.js", None, Some(DangerKind::Script)),
            ("go.jse", None, Some(DangerKind::Script)),
            ("go.wsf", None, Some(DangerKind::Script)),
            ("go.hta", None, Some(DangerKind::Script)),
            ("keys.reg", None, Some(DangerKind::Script)),
            // Linux desktops.
            ("app.desktop", None, Some(DangerKind::Shortcut)),
            ("app.AppImage", None, Some(DangerKind::Program)),
            // The last extension decides, whatever precedes it.
            ("invoice.pdf.exe", Some("application/pdf"), Some(DangerKind::Program)),
            ("setup.exe.pdf", Some("application/pdf"), None),
            // Case, trailing dots and spaces (Windows drops them on disk).
            ("SETUP.EXE", None, Some(DangerKind::Program)),
            ("Page.HtMl", None, Some(DangerKind::WebPage)),
            ("setup.exe.", None, Some(DangerKind::Program)),
            ("setup.exe ", None, Some(DangerKind::Program)),
            ("setup.exe . .", None, Some(DangerKind::Program)),
            ("setup.exe\u{a0}", None, Some(DangerKind::Program)),
            // Only the last path component counts.
            ("folder.app/readme.txt", None, None),
            ("C:\\tools.exe\\readme.txt", None, None),
            ("docs/setup.exe", None, Some(DangerKind::Program)),
            // The declared type is enough under a name that says nothing.
            ("download", Some("application/x-msdownload"), Some(DangerKind::Program)),
            ("page.bin", Some("TEXT/HTML; charset=utf-8"), Some(DangerKind::WebPage)),
            ("script.txt", Some("application/x-sh"), Some(DangerKind::Script)),
            ("image.dat", Some("image/svg+xml"), Some(DangerKind::WebPage)),
            (
                "disk.bin",
                Some("application/x-apple-diskimage"),
                Some(DangerKind::Installer),
            ),
            // The name wins when both speak.
            ("setup.exe", Some("text/html"), Some(DangerKind::Program)),
        ];
        for (filename, mime, expected) in cases {
            assert_eq!(classify(filename, *mime), *expected, "{filename:?} / {mime:?}");
        }
    }

    #[test]
    fn every_listed_extension_and_mime_type_is_lowercase_and_unique() {
        for table in [DANGEROUS_EXTENSIONS, DANGEROUS_MIME_TYPES] {
            let mut seen = std::collections::HashSet::new();
            for (key, _) in table {
                assert_eq!(*key, key.to_lowercase(), "{key}");
                assert!(!key.starts_with('.'), "{key}");
                assert!(seen.insert(*key), "duplicate {key}");
            }
        }
    }

    #[test]
    fn the_quarantine_value_asks_gatekeeper_to_check_a_download() {
        assert_eq!(
            quarantine_value(0x66fa_1b2c, "0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0"),
            "0081;66fa1b2c;EmailOps;0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn quarantine_writes_the_attribute_macos_reads_back() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("invoice.command");
        std::fs::write(&file, b"#!/bin/sh\n").expect("write");
        assert_eq!(read_quarantine(&file), None);

        quarantine(&file).expect("quarantine");

        let value = read_quarantine(&file).expect("attribute present");
        let parts: Vec<&str> = value.split(';').collect();
        assert_eq!(parts.len(), 4, "{value}");
        assert_eq!(parts[0], "0081");
        assert!(i64::from_str_radix(parts[1], 16).expect("hex timestamp") > 0x6000_0000);
        assert_eq!(parts[2], "EmailOps");
        assert!(uuid::Uuid::parse_str(parts[3]).is_ok(), "{value}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn quarantine_keeps_a_mark_the_file_already_carries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("report.pdf");
        std::fs::write(&file, b"%PDF").expect("write");
        quarantine(&file).expect("first");
        let first = read_quarantine(&file).expect("attribute present");

        quarantine(&file).expect("second");

        assert_eq!(read_quarantine(&file).as_deref(), Some(first.as_str()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn quarantine_reports_a_file_it_cannot_mark() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(quarantine(&tmp.path().join("missing.pdf")).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_failed_mark_on_a_saved_file_is_logged_not_swallowed() {
        let _seam = crate::services::events::seam_test_lock();
        let logger = crate::services::logger::install_for_testing();
        let tmp = tempfile::tempdir().expect("tempdir");

        quarantine_saved_file(&tmp.path().join("missing-saved-file.pdf"));

        assert!(logger
            .events()
            .iter()
            .any(|e| e.level == "error" && e.source == "system" && e.message.contains("missing-saved-file.pdf")));
    }

    /// A temp file plus a launcher that records what it was asked to open.
    struct Fixture {
        _tmp: tempfile::TempDir,
        path: PathBuf,
        launched: RefCell<Vec<PathBuf>>,
    }

    impl Fixture {
        fn new(disk_name: &str) -> Self {
            let tmp = tempfile::tempdir().expect("tempdir");
            let path = tmp.path().join(disk_name);
            std::fs::write(&path, b"x").expect("write");
            Self {
                _tmp: tmp,
                path,
                launched: RefCell::new(Vec::new()),
            }
        }

        fn open(&self, filename: &str, mime: Option<&str>, confirmed: bool) -> Result<()> {
            open_attachment_file(&self.path, filename, mime, confirmed, &|p| {
                self.launched.borrow_mut().push(p.to_path_buf());
                Ok(())
            })
        }
    }

    #[test]
    fn a_benign_attachment_opens_without_confirmation() {
        let f = Fixture::new("1b2c.pdf");
        f.open("report.pdf", Some("application/pdf"), false).expect("opens");
        assert_eq!(*f.launched.borrow(), vec![f.path.clone()]);
    }

    #[test]
    fn a_dangerous_attachment_is_refused_without_confirmation() {
        let f = Fixture::new("1b2c.command");
        let err = f.open("run.command", None, false).expect_err("refused");
        assert_eq!(err.code(), "attachment_confirmation_required");
        let params = err.params();
        assert_eq!(params.get("filename").map(String::as_str), Some("run.command"));
        assert_eq!(params.get("kind").map(String::as_str), Some("script"));
        assert!(f.launched.borrow().is_empty());
    }

    #[test]
    fn a_dangerous_attachment_opens_once_confirmed() {
        let f = Fixture::new("1b2c.command");
        f.open("run.command", None, true).expect("opens");
        assert_eq!(f.launched.borrow().len(), 1);
    }

    #[test]
    fn the_file_on_disk_is_classified_too() {
        // The row's filename is benign but the stored file is not.
        let f = Fixture::new("1b2c.terminal");
        let err = f.open("notes.txt", None, false).expect_err("refused");
        assert_eq!(err.code(), "attachment_confirmation_required");
        assert!(f.launched.borrow().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_opened_file_carries_the_quarantine_mark() {
        let f = Fixture::new("1b2c.pdf");
        f.open("report.pdf", None, false).expect("opens");
        assert!(read_quarantine(&f.path).is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_file_that_cannot_be_marked_is_not_opened() {
        let f = Fixture::new("1b2c.pdf");
        std::fs::remove_file(&f.path).expect("remove");
        let err = f.open("report.pdf", None, false).expect_err("refused");
        assert_eq!(err.code(), "io");
        assert!(f.launched.borrow().is_empty());
    }

    #[test]
    fn a_launcher_failure_is_returned() {
        let f = Fixture::new("1b2c.pdf");
        let err = open_attachment_file(&f.path, "report.pdf", None, false, &|_| {
            Err(std::io::Error::other("no handler"))
        })
        .expect_err("fails");
        assert!(err.to_string().contains("no handler"));
    }

    #[test]
    fn opening_is_reported_to_the_output_panel_by_name() {
        let _seam = crate::services::events::seam_test_lock();
        let logger = crate::services::logger::install_for_testing();
        let f = Fixture::new("1b2c.pdf");
        f.open("panel-log-report.pdf", None, false).expect("opens");
        assert!(logger
            .events()
            .iter()
            .any(|e| e.source == "system" && e.message.contains("panel-log-report.pdf")));
    }
}
