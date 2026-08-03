//! Password hashing and verification using Argon2id.
//!
//! Legacy SHA-256 hashes (stored as 64-char lowercase hex) are transparently
//! verified so existing users are not locked out. Callers should call
//! `needs_rehash` after a successful verify and upgrade to Argon2 when true.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use sha2::{Digest, Sha256};

use crate::models::error::{AppError, Result};

/// Hash `password` with Argon2id (random salt). Returns a PHC-format string.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::AuthError(format!("Failed to hash password: {e}")))
}

/// Verify `password` against `stored_hash`.
///
/// Supports both:
/// - **Argon2id** — PHC string starting with `$argon2`
/// - **Legacy SHA-256** — exactly 64 lowercase hex chars (transparently migrated)
///
/// Returns `Ok(true)` on match, `Ok(false)` on mismatch, and `Err` when
/// `stored_hash` matches neither format — a corrupt or truncated record is a
/// different problem from a wrong password and must not masquerade as one.
pub fn verify_password(password: &str, stored_hash: &str) -> Result<bool> {
    if is_legacy_sha256(stored_hash) {
        return Ok(legacy_matches(password, stored_hash));
    }
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
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(AppError::AuthError(format!(
            "Stored password hash is unusable ({e}). Reset the main password to continue."
        ))),
    }
}

/// Returns `true` when `stored_hash` is a legacy hash that should be upgraded
/// after a successful verify.
///
/// Only says `true` for something that really is a legacy digest — a malformed
/// value is neither format and is rejected by [`verify_password`] instead of
/// being silently compared against.
pub fn needs_rehash(stored_hash: &str) -> bool {
    is_legacy_sha256(stored_hash)
}

/// Exactly 64 lowercase hex characters — the shape [`sha2_hex`] emits. Anything
/// else is not a legacy hash, however superficially similar.
fn is_legacy_sha256(stored_hash: &str) -> bool {
    stored_hash.len() == 64
        && stored_hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Compare a candidate password against a legacy digest in constant time.
///
/// `==` on `str` short-circuits at the first differing byte, which leaks how much
/// of the digest matched. Argon2's own `verify_password` is already constant
/// time; this brings the legacy path in line.
fn legacy_matches(password: &str, stored_hash: &str) -> bool {
    use subtle::ConstantTimeEq;
    let computed = sha2_hex(password);
    // Length is already pinned by `is_legacy_sha256`, but compare defensively so
    // this helper is safe to call directly (the tests do).
    if computed.len() != stored_hash.len() {
        return false;
    }
    computed.as_bytes().ct_eq(stored_hash.as_bytes()).into()
}

fn sha2_hex(password: &str) -> String {
    hex::encode(Sha256::digest(password.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_hash(password: &str) -> String {
        sha2_hex(password)
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
    fn verify_legacy_sha256_correct_password_returns_true() {
        let stored = legacy_hash("legacy");
        assert!(verify_password("legacy", &stored).unwrap());
    }

    #[test]
    fn verify_legacy_sha256_wrong_password_returns_false() {
        let stored = legacy_hash("legacy");
        assert!(!verify_password("wrong", &stored).unwrap());
    }

    #[test]
    fn needs_rehash_true_for_sha256_hex() {
        assert!(needs_rehash(&legacy_hash("x")));
    }

    #[test]
    fn needs_rehash_false_for_argon2_hash() {
        let h = hash_password("x").unwrap();
        assert!(!needs_rehash(&h));
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

    #[test]
    fn legacy_hex_digests_are_still_accepted_case_sensitively() {
        // The legacy writer emitted lowercase hex; that must keep verifying.
        let stored = legacy_hash("legacy");
        assert_eq!(stored.len(), 64);
        assert!(verify_password("legacy", &stored).unwrap());
    }

    // A near-miss on the stored hash must not leak how far the comparison got.
    #[test]
    fn legacy_comparison_is_constant_time() {
        let stored = legacy_hash("legacy");
        // Behavioural proxy for the property: the comparison is routed through a
        // constant-time primitive, so a first-byte mismatch and a last-byte
        // mismatch are both simply "false".
        let mut first_byte_differs = stored.clone().into_bytes();
        first_byte_differs[0] ^= 0x01;
        let mut last_byte_differs = stored.clone().into_bytes();
        let last = last_byte_differs.len() - 1;
        last_byte_differs[last] ^= 0x01;

        assert!(!legacy_matches(
            "legacy",
            std::str::from_utf8(&first_byte_differs).unwrap()
        ));
        assert!(!legacy_matches(
            "legacy",
            std::str::from_utf8(&last_byte_differs).unwrap()
        ));
        assert!(legacy_matches("legacy", &stored));
    }
}
