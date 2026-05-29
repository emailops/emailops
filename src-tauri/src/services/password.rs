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
/// - **Legacy SHA-256** — 64-char lowercase hex (transparently migrated)
///
/// Returns `Ok(true)` on match, `Ok(false)` on mismatch, `Err` only if
/// `stored_hash` is neither format.
pub fn verify_password(password: &str, stored_hash: &str) -> Result<bool> {
    if needs_rehash(stored_hash) {
        return Ok(sha2_hex(password) == stored_hash);
    }
    let parsed =
        PasswordHash::new(stored_hash).map_err(|e| AppError::AuthError(format!("Invalid stored hash: {e}")))?;
    Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}

/// Returns `true` when `stored_hash` is in a legacy (non-Argon2) format and
/// should be upgraded after a successful verify.
pub fn needs_rehash(stored_hash: &str) -> bool {
    !stored_hash.starts_with("$argon2")
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
}
