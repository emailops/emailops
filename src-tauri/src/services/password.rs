//! Password hashing and verification using Argon2id, and the throttled check
//! of the main (app-lock) password.

use argon2::{
    // `phc::PasswordHash` rather than the root re-export: password-hash 0.6
    // deprecated the latter, and this crate builds with `-D warnings`.
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};

use crate::db::Database;
use crate::models::error::{AppError, Result};

/// Shortest main password accepted, in characters. The Settings dialog checks
/// the same number for an immediate message; this is the rule that holds.
pub const MIN_MAIN_PASSWORD_CHARS: usize = 6;

/// Reject a new main password shorter than [`MIN_MAIN_PASSWORD_CHARS`].
/// Counts characters, not bytes, so non-ASCII passwords are measured the way
/// the user typed them.
pub fn validate_new_password(password: &str) -> Result<()> {
    if password.chars().count() < MIN_MAIN_PASSWORD_CHARS {
        return Err(AppError::InvalidInput(format!(
            "Password must be at least {MIN_MAIN_PASSWORD_CHARS} characters."
        )));
    }
    Ok(())
}

/// Hash `password` with Argon2id (random salt). Returns a PHC-format string.
pub fn hash_password(password: &str) -> Result<String> {
    // password-hash 0.6 generates the salt itself (getrandom, on by default in
    // argon2 0.6) instead of taking one — same CSPRNG source as the explicit
    // `SaltString::generate(&mut OsRng)` this replaces.
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| AppError::AuthError(format!("Failed to hash password: {e}")))
}

/// Verify `password` against `stored_hash`.
///
/// Only Argon2 PHC strings are accepted; every release has stored those.
///
/// Returns `Ok(true)` on match, `Ok(false)` on mismatch, and `Err` when
/// `stored_hash` matches neither format — a corrupt or truncated record is a
/// different problem from a wrong password and must not masquerade as one.
pub fn verify_password(password: &str, stored_hash: &str) -> Result<bool> {
    if !stored_hash.starts_with("$argon2") {
        return Err(AppError::AuthError(
            "Stored password hash is not in a recognised format. Reset the main password to continue.".to_string(),
        ));
    }
    let parsed =
        PasswordHash::new(stored_hash).map_err(|e| AppError::AuthError(format!("Invalid stored hash: {e}")))?;
    // A PHC string can parse while carrying no hash component at all (e.g.
    // `$argon2id$broken`, where "broken" is read as the salt). `verify_password`
    // reports that as `Error::Password` — indistinguishable from a wrong
    // password — so catch it here instead of telling the user their correct
    // password is wrong, forever.
    if parsed.hash.is_none() {
        return Err(AppError::AuthError(
            "Stored password hash is incomplete. Reset the main password to continue.".to_string(),
        ));
    }
    // Only `Error::Password` means "wrong password". Every other error means the
    // stored PHC string is unusable (missing salt/hash, unknown params), which is
    // a corrupt record — reporting it as a wrong password would lock the user out
    // with no way to tell the difference.
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        // `Error::Password` in password-hash 0.5.
        Err(argon2::password_hash::Error::PasswordInvalid) => Ok(false),
        Err(e) => Err(AppError::AuthError(format!(
            "Stored password hash is unusable ({e}). Reset the main password to continue."
        ))),
    }
}

pub const MAIN_PASSWORD_KEY: &str = "security.main_password_hash";

const FAILED_ATTEMPTS_KEY: &str = "security.main_password_failed_attempts";
const LAST_FAILED_AT_KEY: &str = "security.main_password_last_failed_at";

/// Wrong main passwords allowed before attempts start being delayed.
const FREE_ATTEMPTS: u32 = 5;
const FIRST_LOCKOUT_SECS: i64 = 30;
const MAX_LOCKOUT_SECS: i64 = 15 * 60;

/// Seconds the lock screen refuses attempts after `failures` consecutive
/// wrong main passwords: none for the first few, then doubling up to a cap.
pub fn lockout_secs(failures: u32) -> i64 {
    if failures < FREE_ATTEMPTS {
        return 0;
    }
    let doublings = (failures - FREE_ATTEMPTS).min(8);
    (FIRST_LOCKOUT_SECS << doublings).min(MAX_LOCKOUT_SECS)
}

/// Verify the main password, throttled by consecutive failures. The counter
/// lives in the DB so restarting the app does not reset it. Attempts made
/// during a lockout are refused without being checked.
pub fn verify_main_password(db: &Database, password: &str, now: i64) -> Result<bool> {
    let failures: u32 = db
        .get_preference(FAILED_ATTEMPTS_KEY)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let last_failed_at: i64 = db
        .get_preference(LAST_FAILED_AT_KEY)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let wait = lockout_secs(failures) - (now - last_failed_at);
    if wait > 0 {
        return Err(AppError::AuthError(format!(
            "Too many incorrect attempts. Try again in {wait} s."
        )));
    }

    let Some(stored) = db.get_preference(MAIN_PASSWORD_KEY)?.filter(|v| !v.is_empty()) else {
        return Ok(false);
    };
    if verify_password(password, &stored)? {
        db.set_preference(FAILED_ATTEMPTS_KEY, "0")?;
        Ok(true)
    } else {
        db.set_preference(FAILED_ATTEMPTS_KEY, &failures.saturating_add(1).to_string())?;
        db.set_preference(LAST_FAILED_AT_KEY, &now.to_string())?;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    /// A PHC string produced by argon2 0.5 — the version every existing
    /// install hashed its main password with. Verification reads its
    /// parameters (m=19456, t=2, p=1) out of the string itself rather than
    /// from the crate's current defaults, so a version bump must keep
    /// accepting it. If this ever fails, the upgrade locks every user out of
    /// their vault with no way back.
    const HASH_FROM_ARGON2_0_5: &str =
        "$argon2id$v=19$m=19456,t=2,p=1$Xvs3i2UkHxTx49tD9UKLlw$WHMpw4/HziB9dtLLh4BkIyQvIgQ/g5MES0vHGPs+r4c";
    const PASSWORD_FOR_THAT_HASH: &str = "correct horse battery staple";

    #[test]
    fn new_hashes_keep_the_same_argon2id_parameters() {
        // Deliberate tripwire. Verification reads parameters from the stored
        // string, so a change here would not lock anyone out — but it silently
        // alters the cost of every new password, in either direction. If
        // upstream moves its defaults, that should be an accepted change rather
        // than something noticed later.
        let fresh = super::hash_password("whatever").expect("hash");
        assert!(
            fresh.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "argon2 defaults moved: {fresh}"
        );
    }

    #[test]
    fn a_hash_written_by_the_previous_argon2_still_verifies() {
        assert!(super::verify_password(PASSWORD_FOR_THAT_HASH, HASH_FROM_ARGON2_0_5).expect("verify"));
    }

    #[test]
    fn a_wrong_password_against_a_previous_argon2_hash_is_still_a_mismatch() {
        // Not an error — the caller distinguishes "wrong password" from
        // "unusable record", and conflating them either locks users out or
        // hides corruption.
        assert!(!super::verify_password("wrong password", HASH_FROM_ARGON2_0_5).expect("verify"));
    }

    use super::*;

    /// The unsalted SHA-256 hex digest pre-0.5.0 builds stored.
    fn legacy_hash(password: &str) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(password.as_bytes()))
    }

    #[test]
    fn hash_produces_argon2id_format() {
        let h = hash_password("secret").unwrap();
        assert!(h.starts_with("$argon2id$"), "expected argon2id format, got: {h}");
    }

    #[test]
    fn verify_correct_argon2_password_returns_true() {
        let h = hash_password("correct").unwrap();
        assert!(verify_password("correct", &h).unwrap());
    }

    #[test]
    fn verify_wrong_argon2_password_returns_false() {
        let h = hash_password("correct").unwrap();
        assert!(!verify_password("wrong", &h).unwrap());
    }

    #[test]
    fn different_calls_produce_different_hashes_but_both_verify() {
        let h1 = hash_password("pass").unwrap();
        let h2 = hash_password("pass").unwrap();
        assert_ne!(h1, h2, "random salt means different PHC strings");
        assert!(verify_password("pass", &h1).unwrap());
        assert!(verify_password("pass", &h2).unwrap());
    }

    #[test]
    fn a_new_main_password_needs_at_least_six_characters() {
        for (password, accepted) in [
            ("", false),
            ("abc", false),
            ("abcde", false),
            ("abcdef", true),
            ("a much longer passphrase", true),
            // Characters, not bytes: five accented letters are ten UTF-8 bytes
            // and still too short; six are enough.
            ("ééééé", false),
            ("éééééé", true),
        ] {
            let result = super::validate_new_password(password);
            assert_eq!(result.is_ok(), accepted, "{password:?}");
            if !accepted {
                assert!(matches!(result, Err(super::AppError::InvalidInput(_))), "{password:?}");
            }
        }
    }

    #[test]
    fn a_legacy_unsalted_sha256_digest_is_rejected() {
        // Every release since 0.5.0 stores Argon2id; an unsalted digest is
        // not an acceptable stored form (CASA 1.1.3), so it must not verify.
        let stored = legacy_hash("legacy");
        assert!(verify_password("legacy", &stored).is_err());
    }

    // Regression: anything not starting with `$argon2` was treated as a legacy
    // SHA-256 hex digest, so a truncated or garbled stored hash silently became
    // a comparison that could never match. The user was told "Password is
    // incorrect" forever with no way to tell a wrong password from a corrupt
    // record — and the doc comment claimed an `Err` was returned for exactly
    // this case, which was unreachable.
    #[test]
    fn verify_rejects_a_stored_hash_that_is_neither_format() {
        for stored in [
            "",                 // empty
            "not-a-hash",       // garbage
            "abc123",           // too short to be a sha256 hex digest
            &"a".repeat(63),    // one nibble short
            &"a".repeat(65),    // one nibble long
            &"A".repeat(64),    // uppercase — not the format we ever wrote
            &"z".repeat(64),    // right length, not hex
            "$argon2id$broken", // argon2-shaped but unparseable
        ] {
            let result = verify_password("whatever", stored);
            assert!(
                result.is_err(),
                "a stored hash of {stored:?} must be reported as invalid, not as a failed match"
            );
        }
    }

    // --- main password throttling ---

    fn db_with_main_password(password: &str) -> Database {
        let db = Database::new_for_testing().expect("test db");
        db.set_preference(MAIN_PASSWORD_KEY, &hash_password(password).unwrap())
            .unwrap();
        db
    }

    #[test]
    fn lockout_starts_after_the_free_attempts_and_doubles_up_to_a_cap() {
        assert_eq!(lockout_secs(0), 0);
        assert_eq!(lockout_secs(4), 0);
        assert_eq!(lockout_secs(5), 30);
        assert_eq!(lockout_secs(6), 60);
        assert_eq!(lockout_secs(7), 120);
        assert_eq!(lockout_secs(12), 900);
        assert_eq!(lockout_secs(u32::MAX), 900);
    }

    #[test]
    fn the_right_password_unlocks() {
        let db = db_with_main_password("right");
        assert!(verify_main_password(&db, "right", 1_000).unwrap());
    }

    #[test]
    fn attempts_are_refused_during_a_lockout_even_with_the_right_password() {
        let db = db_with_main_password("right");
        for _ in 0..5 {
            assert!(!verify_main_password(&db, "wrong", 1_000).unwrap());
        }
        let result = verify_main_password(&db, "right", 1_010);
        assert!(matches!(result, Err(AppError::AuthError(_))), "{result:?}");
    }

    #[test]
    fn the_lockout_ends_after_its_delay() {
        let db = db_with_main_password("right");
        for _ in 0..5 {
            assert!(!verify_main_password(&db, "wrong", 1_000).unwrap());
        }
        assert!(verify_main_password(&db, "right", 1_030).unwrap());
    }

    #[test]
    fn a_success_resets_the_failure_count() {
        let db = db_with_main_password("right");
        for _ in 0..4 {
            assert!(!verify_main_password(&db, "wrong", 1_000).unwrap());
        }
        assert!(verify_main_password(&db, "right", 1_000).unwrap());
        for _ in 0..4 {
            assert!(!verify_main_password(&db, "wrong", 1_000).unwrap());
        }
        assert!(verify_main_password(&db, "right", 1_000).unwrap());
    }
}
