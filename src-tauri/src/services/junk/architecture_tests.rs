//! Architectural guard: junk detection never touches the mail server.
//!
//! The central promise of this feature is that the detector only ever writes a
//! local flag. A false positive that moves a message hides it in every client
//! the user owns, which is why the design forbids the detector from acting on
//! the provider at all — only an explicit user action may do that, through
//! `commands::junk::report_junk_to_provider`.
//!
//! Today that holds by construction: nothing under `services/junk/` so much as
//! imports the provider trait, so there is no runtime call to intercept and a
//! behavioural test would be vacuous. What can change is the *source*: a future
//! edit could add the import and start moving mail without any test noticing.
//!
//! So the invariant is guarded where it actually lives — in the module's
//! dependencies. This is the same shape as a layering rule ("module X must not
//! depend on Y"), and it is the only place a check can be both meaningful and
//! non-vacuous.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// Symbols that would mean this module had gained the ability to act on the
    /// mail server.
    const FORBIDDEN: &[&str] = &[
        "EmailProvider",
        "MailProvider",
        "move_message",
        "MoveTarget",
        "send_new_email",
        "send_reply",
        "delete_draft",
        "provider_for_account",
    ];

    fn junk_module_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/services/junk")
    }

    fn rust_sources(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(rust_sources(&path));
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn the_detector_cannot_act_on_the_mail_server() {
        let files = rust_sources(&junk_module_dir());
        assert!(!files.is_empty(), "found no sources under services/junk");

        let mut violations: Vec<String> = Vec::new();
        for path in files {
            // This file names the forbidden symbols in order to check for them.
            if path.file_name().is_some_and(|n| n == "architecture_tests.rs") {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in source.lines() {
                // Comments and docs discuss the rule; only real code breaks it.
                let code = line.trim();
                if code.starts_with("//") || code.starts_with("///") || code.starts_with("//!") {
                    continue;
                }
                for symbol in FORBIDDEN {
                    if code.contains(symbol) {
                        violations.push(format!("{}: {}", path.display(), code.trim()));
                    }
                }
            }
        }

        assert!(
            violations.is_empty(),
            "junk detection must never act on the mail server — a verdict is a local flag, \
             and only an explicit user action may move mail. Found:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn the_guard_would_notice_a_violation() {
        // A guard that cannot fail is decoration. This proves the matcher does
        // catch the shape it is meant to catch.
        let sample = "        provider.move_message(&id, None, &MoveTarget::Inbox).await?;";
        assert!(FORBIDDEN.iter().any(|s| sample.contains(s)));
    }
}
