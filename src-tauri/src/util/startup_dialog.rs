//! Best-effort native error dialog for fatal startup failures.
//!
//! When EmailOps cannot start it has no window, no frontend, and — if it was
//! launched from the Finder, a `.desktop` entry, or the Start menu — no visible
//! stderr either. Until now only macOS showed the user anything: the
//! `#[cfg(not(target_os = "macos"))]` arm was an empty function, so a Linux box
//! without a Secret Service daemon (which makes keychain init fail, which is
//! fatal) exited instantly with no explanation at all.
//!
//! Two mechanisms, deliberately ordered by reliability:
//!
//! 1. **A crash-report file**, written unconditionally. It needs no GUI toolkit,
//!    no helper binary, and no display server, so it is the one thing that
//!    works everywhere and is what support can ask the user for.
//! 2. **A native dialog**, attempted afterwards on a best-effort basis. Each
//!    platform gets a chain of candidate helpers; the first that runs
//!    successfully wins and the rest are skipped. Failure here is not fatal —
//!    the message is already on disk and on stderr.
//!
//! The command chain is chosen by a pure function so it can be table-tested for
//! every platform from any platform. That matters because the Linux and Windows
//! arms are never executed on the developer's machine.

use std::path::{Path, PathBuf};

/// Operating systems that get a bespoke dialog chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogOs {
    MacOs,
    Linux,
    Windows,
    Other,
}

impl DialogOs {
    /// The OS this binary was compiled for.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

/// Environment variable used to hand the message to a helper process.
///
/// Passing the text out-of-band rather than interpolating it into a command
/// string sidesteps every shell-quoting hazard — the message contains newlines
/// and arbitrary error text from the OS, which is exactly the kind of input
/// that breaks naive escaping.
pub const MESSAGE_ENV_VAR: &str = "EMAILOPS_STARTUP_ERROR";

/// One candidate way to show a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogCommand {
    pub program: String,
    pub args: Vec<String>,
    /// When true, the message is exported as [`MESSAGE_ENV_VAR`] for the child
    /// process instead of appearing in `args`.
    pub message_via_env: bool,
}

/// Absolute path to a helper under the Windows system directory.
///
/// `%SystemRoot%` is read from the environment (it is always set on Windows) and
/// falls back to the conventional `C:\Windows`. Building the path here rather
/// than relying on PATH lookup is what stops a same-directory `powershell.exe`
/// from being picked up — see the `DialogOs::Windows` arm.
///
/// `relative` is a `System32`-relative path, e.g. `"notepad.exe"`.
fn windows_system32_path(relative: &str) -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    format!("{}\\System32\\{}", root.trim_end_matches('\\'), relative)
}

/// Ordered list of dialog helpers to try for `os`.
///
/// Returns an empty list for platforms with no known helper, which callers must
/// treat as "file and stderr only" rather than as an error.
pub fn dialog_commands(os: DialogOs, message: &str) -> Vec<DialogCommand> {
    match os {
        // `osascript` is present on every macOS install and works before the
        // app has created a window.
        DialogOs::MacOs => {
            let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
            vec![DialogCommand {
                program: "osascript".into(),
                args: vec![
                    "-e".into(),
                    format!("display alert \"EmailOps\" message \"{escaped}\" as critical"),
                ],
                message_via_env: false,
            }]
        }
        // No single dialog helper ships on every distribution, so try the
        // GNOME one, then the KDE one, then a plain desktop notification.
        // Arguments are passed as separate argv entries, so the message needs
        // no escaping.
        DialogOs::Linux => vec![
            DialogCommand {
                program: "zenity".into(),
                args: vec!["--error".into(), "--title=EmailOps".into(), format!("--text={message}")],
                message_via_env: false,
            },
            DialogCommand {
                program: "kdialog".into(),
                args: vec![
                    "--title".into(),
                    "EmailOps".into(),
                    "--error".into(),
                    message.to_string(),
                ],
                message_via_env: false,
            },
            DialogCommand {
                program: "notify-send".into(),
                args: vec!["--urgency=critical".into(), "EmailOps".into(), message.to_string()],
                message_via_env: false,
            },
        ],
        // PowerShell ships with every supported Windows version. The message
        // travels through the environment, so newlines and quotes in the OS
        // error text cannot break the command line.
        //
        // Named by absolute path on purpose: `CreateProcessW` searches the
        // application directory and the current directory before PATH, so a bare
        // "powershell" lets a `powershell.exe` sitting next to the app (or in
        // whatever directory the app happened to be launched from) run instead
        // of the real one.
        DialogOs::Windows => vec![DialogCommand {
            program: windows_system32_path("WindowsPowerShell\\v1.0\\powershell.exe"),
            args: vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                format!(
                    "Add-Type -AssemblyName PresentationFramework; \
                     [System.Windows.MessageBox]::Show($env:{MESSAGE_ENV_VAR}, 'EmailOps', 'OK', 'Error')"
                ),
            ],
            message_via_env: true,
        }],
        DialogOs::Other => Vec::new(),
    }
}

/// Path of the crash report for a given data directory.
pub fn crash_report_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("startup-error.log")
}

/// Write the message where a user or support can find it.
///
/// Tries the app data directory first, since that is what the docs point at,
/// and falls back to the system temp directory when the data directory is
/// itself the thing that is broken. Returns the path actually written.
pub fn write_crash_report(app_data_dir: Option<&Path>, message: &str) -> std::io::Result<PathBuf> {
    let mut last_err = None;

    for dir in [app_data_dir, Some(&std::env::temp_dir())].into_iter().flatten() {
        if std::fs::create_dir_all(dir).is_err() {
            continue;
        }
        let path = crash_report_path(dir);
        match std::fs::write(&path, message) {
            Ok(()) => return Ok(path),
            Err(e) => last_err = Some(e),
        }
    }

    Err(last_err.unwrap_or_else(|| std::io::Error::other("no writable directory for the crash report")))
}

/// Show the fatal-startup message to the user as loudly as the platform allows.
///
/// `app_data_dir` is optional because the data directory may be exactly what
/// could not be resolved.
pub fn show(app_data_dir: Option<&Path>, message: &str) {
    match write_crash_report(app_data_dir, message) {
        Ok(path) => eprintln!("[startup][fatal] details written to {}", path.display()),
        Err(e) => eprintln!("[startup][fatal] could not write a crash report: {e}"),
    }

    let candidates = dialog_commands(DialogOs::current(), message);
    if candidates.is_empty() {
        eprintln!("[startup][fatal] no native dialog helper is known for this platform");
        return;
    }

    for candidate in &candidates {
        let mut cmd = std::process::Command::new(&candidate.program);
        cmd.args(&candidate.args);
        if candidate.message_via_env {
            cmd.env(MESSAGE_ENV_VAR, message);
        }
        match cmd.status() {
            Ok(status) if status.success() => return,
            // A helper that exists but refuses (no display, user dismissed a
            // parent) is worth reporting before falling through to the next.
            Ok(status) => {
                eprintln!("[startup][fatal] {} exited with {status}", candidate.program);
            }
            Err(e) => {
                eprintln!("[startup][fatal] could not run {}: {e}", candidate.program);
            }
        }
    }

    eprintln!("[startup][fatal] every native dialog helper failed; see the crash report above");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_uses_osascript_with_an_escaped_script() {
        let cmds = dialog_commands(DialogOs::MacOs, "he said \"boom\"");
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].program, "osascript");
        assert!(
            cmds[0].args.iter().any(|a| a.contains("\\\"boom\\\"")),
            "quotes must be escaped for AppleScript: {:?}",
            cmds[0].args
        );
        assert!(!cmds[0].message_via_env);
    }

    #[test]
    fn macos_escapes_backslashes_before_quotes() {
        // Escaping in the wrong order turns `\` into `\\\"` and corrupts the
        // script, so pin the ordering.
        let cmds = dialog_commands(DialogOs::MacOs, r"C:\path");
        assert!(
            cmds[0].args.iter().any(|a| a.contains(r"C:\\path")),
            "backslash must be doubled: {:?}",
            cmds[0].args
        );
    }

    // Windows resolves a bare program name against the application directory and
    // the current directory *before* PATH, so `Command::new("powershell")` will
    // happily run a `powershell.exe` that someone dropped next to the app. An
    // absolute path under %SystemRoot% removes the search entirely.
    #[test]
    fn windows_dialog_helper_is_an_absolute_path() {
        let cmds = dialog_commands(DialogOs::Windows, "boom");
        let program = &cmds[0].program;
        assert!(
            program.contains(':') && program.contains('\\'),
            "expected an absolute Windows path, got {program:?}"
        );
        assert!(
            program.to_lowercase().ends_with("powershell.exe"),
            "expected powershell.exe, got {program:?}"
        );
    }

    #[test]
    fn linux_falls_back_across_desktop_environments() {
        let cmds = dialog_commands(DialogOs::Linux, "keyring unavailable");
        let programs: Vec<&str> = cmds.iter().map(|c| c.program.as_str()).collect();
        assert_eq!(programs, vec!["zenity", "kdialog", "notify-send"]);
    }

    #[test]
    fn linux_passes_the_message_as_a_single_argv_entry() {
        // Passing argv directly means a message containing quotes, newlines or
        // `$(...)` cannot be re-interpreted by a shell.
        let nasty = "line one\nline \"two\" $(rm -rf /)";
        for cmd in dialog_commands(DialogOs::Linux, nasty) {
            assert!(
                cmd.args.iter().any(|a| a.contains(nasty)),
                "{} must carry the message verbatim: {:?}",
                cmd.program,
                cmd.args
            );
            assert!(!cmd.message_via_env);
        }
    }

    #[test]
    fn windows_passes_the_message_through_the_environment() {
        let cmds = dialog_commands(DialogOs::Windows, "line one\nline \"two\"");
        assert_eq!(cmds.len(), 1);
        // Absolute path, not a bare name — see
        // `windows_dialog_helper_is_an_absolute_path`.
        assert!(
            cmds[0].program.to_lowercase().ends_with("powershell.exe"),
            "expected powershell.exe, got {:?}",
            cmds[0].program
        );
        assert!(cmds[0].message_via_env, "message must not be inlined into the command");
        assert!(
            cmds[0].args.iter().all(|a| !a.contains("line one")),
            "raw message must never appear in argv: {:?}",
            cmds[0].args
        );
        assert!(
            cmds[0].args.iter().any(|a| a.contains(MESSAGE_ENV_VAR)),
            "the command must read the message from the env var: {:?}",
            cmds[0].args
        );
    }

    #[test]
    fn windows_command_is_non_interactive_and_profile_free() {
        // A PowerShell profile or a prompt would hang the dying process.
        let cmds = dialog_commands(DialogOs::Windows, "boom");
        let args = &cmds[0].args;
        assert!(args.iter().any(|a| a == "-NoProfile"));
        assert!(args.iter().any(|a| a == "-NonInteractive"));
    }

    #[test]
    fn unknown_platforms_get_no_dialog_rather_than_a_bad_one() {
        assert!(dialog_commands(DialogOs::Other, "boom").is_empty());
    }

    #[test]
    fn every_platform_chain_is_non_empty_except_other() {
        for os in [DialogOs::MacOs, DialogOs::Linux, DialogOs::Windows] {
            assert!(
                !dialog_commands(os, "boom").is_empty(),
                "{os:?} must have at least one dialog helper"
            );
        }
    }

    #[test]
    fn crash_report_lands_in_the_data_dir() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let path = write_crash_report(Some(tmp.path()), "the details").expect("write");

        assert_eq!(path.parent(), Some(tmp.path()));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "the details");
    }

    /// Both temp-dir fallbacks in one test: they write to the same well-known
    /// path, so splitting them lets the parallel test runner race them.
    #[test]
    fn crash_report_falls_back_to_temp_when_the_data_dir_is_unusable() {
        // A broken data directory is precisely the case that triggers a fatal
        // startup error, so the report must survive it. A path whose parent is
        // a regular file cannot be turned into a directory.
        let tmp = tempfile::tempdir().expect("temp dir");
        let blocker = tmp.path().join("not-a-dir");
        std::fs::write(&blocker, b"x").expect("write blocker");
        let unusable = blocker.join("data");

        let path = write_crash_report(Some(&unusable), "still recorded").expect("fallback write");
        assert!(
            path.starts_with(std::env::temp_dir()),
            "expected temp fallback, got {path:?}"
        );
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "still recorded");

        // The data directory may not be known at all when resolving it is what
        // failed.
        let path = write_crash_report(None, "no data dir").expect("temp write");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "no data dir");
    }

    #[test]
    fn current_os_matches_the_compilation_target() {
        let expected = if cfg!(target_os = "macos") {
            DialogOs::MacOs
        } else if cfg!(target_os = "linux") {
            DialogOs::Linux
        } else if cfg!(target_os = "windows") {
            DialogOs::Windows
        } else {
            DialogOs::Other
        };
        assert_eq!(DialogOs::current(), expected);
    }
}
