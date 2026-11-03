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
    ("cpl", DangerKind::Program),
    ("xll", DangerKind::Program),
    ("jar", DangerKind::Program),
    ("class", DangerKind::Program),
    ("jnlp", DangerKind::Program),
    ("appimage", DangerKind::Program),
    ("run", DangerKind::Program),
    // The rest of what Launch Services ranks LSRiskCategoryUnsafeExecutable.
    ("cin", DangerKind::Program),
    ("gcx", DangerKind::Program),
    ("help", DangerKind::Program),
    ("ibplugin", DangerKind::Program),
    ("icp", DangerKind::Program),
    ("inputplugin", DangerKind::Program),
    ("ipg", DangerKind::Program),
    ("menu", DangerKind::Program),
    ("nib", DangerKind::Program),
    ("service", DangerKind::Program),
    ("x11app", DangerKind::Program),
    // Scripts and files that carry commands for an interpreter.
    ("command", DangerKind::Script),
    ("terminal", DangerKind::Script),
    ("term", DangerKind::Script),
    ("tool", DangerKind::Script),
    ("workflow", DangerKind::Script),
    ("wflow", DangerKind::Script),
    ("action", DangerKind::Script),
    ("scpt", DangerKind::Script),
    ("scptd", DangerKind::Script),
    ("applescript", DangerKind::Script),
    ("shortcut", DangerKind::Script),
    ("qtz", DangerKind::Script),
    ("sh", DangerKind::Script),
    ("zsh", DangerKind::Script),
    ("bash", DangerKind::Script),
    ("csh", DangerKind::Script),
    ("tcsh", DangerKind::Script),
    ("ksh", DangerKind::Script),
    ("fish", DangerKind::Script),
    ("py", DangerKind::Script),
    ("pyw", DangerKind::Script),
    ("pyc", DangerKind::Script),
    ("pyo", DangerKind::Script),
    ("pyz", DangerKind::Script),
    ("pyzw", DangerKind::Script),
    ("rb", DangerKind::Script),
    ("rbw", DangerKind::Script),
    ("pl", DangerKind::Script),
    ("bat", DangerKind::Script),
    ("cmd", DangerKind::Script),
    ("ps1", DangerKind::Script),
    ("psc1", DangerKind::Script),
    ("vbs", DangerKind::Script),
    ("vbe", DangerKind::Script),
    ("js", DangerKind::Script),
    ("jse", DangerKind::Script),
    ("ws", DangerKind::Script),
    ("wsf", DangerKind::Script),
    ("wsh", DangerKind::Script),
    ("wsc", DangerKind::Script),
    ("sct", DangerKind::Script),
    ("wsb", DangerKind::Script),
    ("hta", DangerKind::Script),
    ("chm", DangerKind::Script),
    ("msc", DangerKind::Script),
    ("scf", DangerKind::Script),
    ("reg", DangerKind::Script),
    ("diagcab", DangerKind::Script),
    ("diagcfg", DangerKind::Script),
    ("diagpkg", DangerKind::Script),
    ("theme", DangerKind::Script),
    ("themepack", DangerKind::Script),
    ("deskthemepack", DangerKind::Script),
    // Installers and disk images.
    ("pkg", DangerKind::Installer),
    ("mpkg", DangerKind::Installer),
    ("dmg", DangerKind::Installer),
    ("cdr", DangerKind::Installer),
    ("sparseimage", DangerKind::Installer),
    ("sparsebundle", DangerKind::Installer),
    ("iso", DangerKind::Installer),
    ("img", DangerKind::Installer),
    ("vhd", DangerKind::Installer),
    ("vhdx", DangerKind::Installer),
    ("ipa", DangerKind::Installer),
    ("mobileconfig", DangerKind::Installer),
    ("prefpane", DangerKind::Installer),
    ("saver", DangerKind::Installer),
    ("msi", DangerKind::Installer),
    ("msp", DangerKind::Installer),
    ("msu", DangerKind::Installer),
    ("msix", DangerKind::Installer),
    ("msixbundle", DangerKind::Installer),
    ("appx", DangerKind::Installer),
    ("appxbundle", DangerKind::Installer),
    ("appinstaller", DangerKind::Installer),
    ("application", DangerKind::Installer),
    ("ppkg", DangerKind::Installer),
    ("deb", DangerKind::Installer),
    ("rpm", DangerKind::Installer),
    ("flatpak", DangerKind::Installer),
    ("flatpakref", DangerKind::Installer),
    ("flatpakrepo", DangerKind::Installer),
    ("snap", DangerKind::Installer),
    // Shortcuts to another file, app or address.
    ("fileloc", DangerKind::Shortcut),
    ("inetloc", DangerKind::Shortcut),
    ("webloc", DangerKind::Shortcut),
    ("afploc", DangerKind::Shortcut),
    ("ftploc", DangerKind::Shortcut),
    ("mailloc", DangerKind::Shortcut),
    ("newsloc", DangerKind::Shortcut),
    ("vncloc", DangerKind::Shortcut),
    ("url", DangerKind::Shortcut),
    ("lnk", DangerKind::Shortcut),
    ("appref-ms", DangerKind::Shortcut),
    ("library-ms", DangerKind::Shortcut),
    ("search-ms", DangerKind::Shortcut),
    ("searchconnector-ms", DangerKind::Shortcut),
    ("settingcontent-ms", DangerKind::Shortcut),
    ("rdp", DangerKind::Shortcut),
    ("desktop", DangerKind::Shortcut),
    // Local pages: scripts in them run in the browser with a file origin.
    ("html", DangerKind::WebPage),
    ("htm", DangerKind::WebPage),
    ("shtml", DangerKind::WebPage),
    ("shtm", DangerKind::WebPage),
    ("xhtml", DangerKind::WebPage),
    ("xhtm", DangerKind::WebPage),
    ("xht", DangerKind::WebPage),
    ("svg", DangerKind::WebPage),
    ("svgz", DangerKind::WebPage),
    ("mht", DangerKind::WebPage),
    ("mhtml", DangerKind::WebPage),
    ("webarchive", DangerKind::WebPage),
];

/// MIME types that declare one of the kinds above. Checked after the name, so
/// a file the sender labelled as a program is confirmed even under a name
/// that says nothing. Lowercase.
const DANGEROUS_MIME_TYPES: &[(&str, DangerKind)] = &[
    ("application/x-msdownload", DangerKind::Program),
    ("application/x-msdos-program", DangerKind::Program),
    ("application/x-dosexec", DangerKind::Program),
    ("application/x-ms-dos-executable", DangerKind::Program),
    ("application/vnd.microsoft.portable-executable", DangerKind::Program),
    ("application/x-executable", DangerKind::Program),
    ("application/x-pie-executable", DangerKind::Program),
    ("application/x-elf", DangerKind::Program),
    ("application/x-mach-binary", DangerKind::Program),
    ("application/java-archive", DangerKind::Program),
    ("application/x-java-jnlp-file", DangerKind::Program),
    ("application/x-sh", DangerKind::Script),
    ("application/x-shellscript", DangerKind::Script),
    ("text/x-shellscript", DangerKind::Script),
    ("application/x-csh", DangerKind::Script),
    ("application/x-applescript", DangerKind::Script),
    ("application/javascript", DangerKind::Script),
    ("text/javascript", DangerKind::Script),
    ("application/x-powershell", DangerKind::Script),
    ("application/x-perl", DangerKind::Script),
    ("application/x-ruby", DangerKind::Script),
    ("text/x-python", DangerKind::Script),
    ("text/vbscript", DangerKind::Script),
    ("application/hta", DangerKind::Script),
    ("application/vnd.ms-htmlhelp", DangerKind::Script),
    ("application/x-apple-diskimage", DangerKind::Installer),
    ("application/x-iso9660-image", DangerKind::Installer),
    ("application/x-cd-image", DangerKind::Installer),
    ("application/x-msi", DangerKind::Installer),
    ("application/x-ms-installer", DangerKind::Installer),
    ("application/x-ms-application", DangerKind::Installer),
    ("application/vnd.apple.installer+xml", DangerKind::Installer),
    ("application/x-apple-aspen-config", DangerKind::Installer),
    ("application/vnd.debian.binary-package", DangerKind::Installer),
    ("application/x-debian-package", DangerKind::Installer),
    ("application/x-rpm", DangerKind::Installer),
    ("application/vnd.flatpak", DangerKind::Installer),
    ("application/vnd.flatpak.ref", DangerKind::Installer),
    ("application/x-ms-shortcut", DangerKind::Shortcut),
    ("application/internet-shortcut", DangerKind::Shortcut),
    ("application/x-mswinurl", DangerKind::Shortcut),
    ("application/x-desktop", DangerKind::Shortcut),
    ("application/x-rdp", DangerKind::Shortcut),
    ("text/html", DangerKind::WebPage),
    ("application/xhtml+xml", DangerKind::WebPage),
    ("image/svg+xml", DangerKind::WebPage),
    ("application/x-mimearchive", DangerKind::WebPage),
    ("application/x-webarchive", DangerKind::WebPage),
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
    fn classifies_what_windows_acts_on_when_opened() {
        let cases: &[(&str, DangerKind)] = &[
            ("panel.cpl", DangerKind::Program),
            ("addin.xll", DangerKind::Program),
            ("console.msc", DangerKind::Script),
            ("help.chm", DangerKind::Script),
            ("shell.scf", DangerKind::Script),
            ("go.vbe", DangerKind::Script),
            ("go.ws", DangerKind::Script),
            ("go.wsh", DangerKind::Script),
            ("go.wsc", DangerKind::Script),
            ("go.sct", DangerKind::Script),
            ("go.psc1", DangerKind::Script),
            ("go.pyw", DangerKind::Script),
            ("go.pyo", DangerKind::Script),
            ("go.pyz", DangerKind::Script),
            ("go.pyzw", DangerKind::Script),
            ("box.wsb", DangerKind::Script),
            ("fix.diagcab", DangerKind::Script),
            ("fix.diagcfg", DangerKind::Script),
            ("fix.diagpkg", DangerKind::Script),
            ("look.theme", DangerKind::Script),
            ("look.themepack", DangerKind::Script),
            ("look.deskthemepack", DangerKind::Script),
            ("patch.msp", DangerKind::Installer),
            ("update.msu", DangerKind::Installer),
            ("app.msix", DangerKind::Installer),
            ("app.msixbundle", DangerKind::Installer),
            ("app.appx", DangerKind::Installer),
            ("app.appxbundle", DangerKind::Installer),
            ("app.appinstaller", DangerKind::Installer),
            ("app.application", DangerKind::Installer),
            ("setup.ppkg", DangerKind::Installer),
            ("disk.iso", DangerKind::Installer),
            ("disk.img", DangerKind::Installer),
            ("disk.vhd", DangerKind::Installer),
            ("disk.vhdx", DangerKind::Installer),
            ("app.appref-ms", DangerKind::Shortcut),
            ("share.library-ms", DangerKind::Shortcut),
            ("find.search-ms", DangerKind::Shortcut),
            ("find.searchconnector-ms", DangerKind::Shortcut),
            ("panel.settingcontent-ms", DangerKind::Shortcut),
            ("desk.rdp", DangerKind::Shortcut),
            ("page.mht", DangerKind::WebPage),
            ("page.mhtml", DangerKind::WebPage),
        ];
        for (filename, expected) in cases {
            assert_eq!(classify(filename, None), Some(*expected), "{filename:?}");
        }
    }

    #[test]
    fn classifies_what_macos_acts_on_when_opened() {
        // The extensions Launch Services ranks LSRiskCategoryUnsafeExecutable,
        // then what the same data says by content type, and the types that
        // install or mount.
        let cases: &[(&str, DangerKind)] = &[
            ("a.cin", DangerKind::Program),
            ("a.gcx", DangerKind::Program),
            ("a.help", DangerKind::Program),
            ("a.ibplugin", DangerKind::Program),
            ("a.icp", DangerKind::Program),
            ("a.inputplugin", DangerKind::Program),
            ("a.ipg", DangerKind::Program),
            ("a.menu", DangerKind::Program),
            ("a.nib", DangerKind::Program),
            ("a.service", DangerKind::Program),
            ("a.x11app", DangerKind::Program),
            ("a.class", DangerKind::Program),
            ("a.jnlp", DangerKind::Program),
            ("a.term", DangerKind::Script),
            ("a.wflow", DangerKind::Script),
            ("a.qtz", DangerKind::Script),
            ("a.shortcut", DangerKind::Script),
            ("a.csh", DangerKind::Script),
            ("a.tcsh", DangerKind::Script),
            ("a.ksh", DangerKind::Script),
            ("a.pyc", DangerKind::Script),
            ("a.rbw", DangerKind::Script),
            ("a.ipa", DangerKind::Installer),
            ("a.mobileconfig", DangerKind::Installer),
            ("a.prefPane", DangerKind::Installer),
            ("a.saver", DangerKind::Installer),
            ("a.cdr", DangerKind::Installer),
            ("a.sparseimage", DangerKind::Installer),
            ("a.sparsebundle", DangerKind::Installer),
            ("a.afploc", DangerKind::Shortcut),
            ("a.ftploc", DangerKind::Shortcut),
            ("a.mailloc", DangerKind::Shortcut),
            ("a.newsloc", DangerKind::Shortcut),
            ("a.vncloc", DangerKind::Shortcut),
            ("a.webarchive", DangerKind::WebPage),
        ];
        for (filename, expected) in cases {
            assert_eq!(classify(filename, None), Some(*expected), "{filename:?}");
        }
    }

    #[test]
    fn classifies_what_a_linux_desktop_acts_on_when_opened() {
        let cases: &[(&str, DangerKind)] = &[
            ("a.flatpak", DangerKind::Installer),
            ("a.flatpakref", DangerKind::Installer),
            ("a.flatpakrepo", DangerKind::Installer),
            ("a.snap", DangerKind::Installer),
            ("a.fish", DangerKind::Script),
            ("page.shtml", DangerKind::WebPage),
            ("page.shtm", DangerKind::WebPage),
            ("page.xht", DangerKind::WebPage),
            ("page.xhtm", DangerKind::WebPage),
            ("logo.svgz", DangerKind::WebPage),
        ];
        for (filename, expected) in cases {
            assert_eq!(classify(filename, None), Some(*expected), "{filename:?}");
        }
    }

    #[test]
    fn a_declared_type_names_the_same_kinds_as_the_extensions() {
        let cases: &[(&str, DangerKind)] = &[
            ("application/x-dosexec", DangerKind::Program),
            ("application/x-ms-dos-executable", DangerKind::Program),
            ("application/x-pie-executable", DangerKind::Program),
            ("application/x-elf", DangerKind::Program),
            ("application/x-java-jnlp-file", DangerKind::Program),
            ("application/x-csh", DangerKind::Script),
            ("application/x-powershell", DangerKind::Script),
            ("application/x-perl", DangerKind::Script),
            ("application/x-ruby", DangerKind::Script),
            ("text/x-python", DangerKind::Script),
            ("text/vbscript", DangerKind::Script),
            ("application/vnd.ms-htmlhelp", DangerKind::Script),
            ("application/x-ms-application", DangerKind::Installer),
            ("application/x-ms-installer", DangerKind::Installer),
            ("application/vnd.debian.binary-package", DangerKind::Installer),
            ("application/x-debian-package", DangerKind::Installer),
            ("application/x-rpm", DangerKind::Installer),
            ("application/x-iso9660-image", DangerKind::Installer),
            ("application/x-cd-image", DangerKind::Installer),
            ("application/vnd.flatpak", DangerKind::Installer),
            ("application/vnd.flatpak.ref", DangerKind::Installer),
            ("application/x-apple-aspen-config", DangerKind::Installer),
            ("application/x-mswinurl", DangerKind::Shortcut),
            ("application/x-desktop", DangerKind::Shortcut),
            ("application/x-rdp", DangerKind::Shortcut),
            ("application/x-mimearchive", DangerKind::WebPage),
            ("application/x-webarchive", DangerKind::WebPage),
        ];
        for (mime, expected) in cases {
            assert_eq!(classify("download.bin", Some(mime)), Some(*expected), "{mime:?}");
        }
    }

    #[test]
    fn a_document_stays_out_even_if_a_reader_could_mishandle_it() {
        // Lists built for mail gateways carry these too, but opening one runs
        // nothing on its own: no default action, a text editor, or a document
        // reader. Asking on every one would teach the user to click through.
        for filename in [
            "lib.dll",
            "driver.sys",
            "setup.inf",
            "report.docm",
            "sheet.xlsm",
            "bundle.cab",
            "transform.mst",
            "source.vb",
        ] {
            assert_eq!(classify(filename, None), None, "{filename:?}");
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
    /// Opening reports to the output panel through the process logger, whose
    /// events other tests count, so the fixture holds the seam lock.
    struct Fixture {
        _seam: std::sync::MutexGuard<'static, ()>,
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
                _seam: crate::services::events::seam_test_lock(),
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
        let f = Fixture::new("1b2c.pdf");
        let logger = crate::services::logger::install_for_testing();
        f.open("panel-log-report.pdf", None, false).expect("opens");
        assert!(logger
            .events()
            .iter()
            .any(|e| e.source == "system" && e.message.contains("panel-log-report.pdf")));
    }
}
