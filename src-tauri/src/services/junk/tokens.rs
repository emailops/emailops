//! Feature extraction for the statistical layer.
//!
//! Tokens are hashed into a fixed number of buckets rather than kept in a
//! vocabulary map. That keeps the model a pair of flat arrays — no dictionary to
//! version, migrate or keep in sync between training and scoring — at the cost
//! of occasional collisions, which a classifier of this kind tolerates well.
//!
//! Training and scoring MUST derive features from the same projection of a
//! message. Both use subject + snippet + sender: the snippet is already on the
//! `emails` row, so a full retrain never has to load 20k message bodies.

/// 2^15 buckets: 128 KB per count array, 256 KB per model. Large enough that
/// collisions between meaningful tokens stay rare at mailbox scale, small enough
/// that the blob is cheap to read once per scoring batch.
pub const BUCKET_BITS: u32 = 15;
pub const BUCKETS: usize = 1 << BUCKET_BITS;

/// Characters of the snippet considered. Beyond this, spam and ham look alike.
const MAX_TEXT: usize = 2_000;

/// Tokens shorter than this carry no signal and inflate every message equally.
const MIN_TOKEN_LEN: usize = 3;

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// Hash a namespaced token into a bucket.
///
/// The namespace prefix keeps `from:acme.example` and the word `acme.example`
/// in a message body from colliding into the same evidence — they mean
/// different things.
fn bucket(namespace: &str, token: &str) -> u32 {
    let mut buf = Vec::with_capacity(namespace.len() + token.len() + 1);
    buf.extend_from_slice(namespace.as_bytes());
    buf.push(b':');
    buf.extend_from_slice(token.as_bytes());
    (fnv1a(&buf) % BUCKETS as u64) as u32
}

/// Split text into lowercase word tokens.
///
/// Unicode letters and digits are kept, everything else splits. Currency and
/// punctuation are deliberately dropped: they are near-universal and would only
/// add noise shared by every message.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in text.chars().take(MAX_TEXT) {
        if c.is_alphanumeric() {
            current.extend(c.to_lowercase());
        } else if !current.is_empty() {
            if current.chars().count() >= MIN_TOKEN_LEN {
                out.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
    }
    if current.chars().count() >= MIN_TOKEN_LEN {
        out.push(current);
    }
    out
}

/// Message projection the model sees. Identical at training and scoring time.
#[derive(Debug, Clone, Default)]
pub struct FeatureInput<'a> {
    pub subject: &'a str,
    /// Short plain-text preview — `emails.snippet`, not the full body.
    pub snippet: &'a str,
    pub sender_email: &'a str,
    pub x_mailer: Option<&'a str>,
}

/// Extract the bucket set for one message.
///
/// Returns **unique** buckets: this is a Bernoulli-style presence model, so a
/// word repeated forty times in a marketing blast counts once. Repetition is a
/// property of the template, not evidence about the sender.
pub fn features(input: &FeatureInput<'_>) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();

    for word in words(input.subject) {
        out.push(bucket("subj", &word));
    }
    for word in words(input.snippet) {
        out.push(bucket("body", &word));
    }

    let sender = input.sender_email.trim().to_lowercase();
    if let Some((local, domain)) = sender.split_once('@') {
        out.push(bucket("from_domain", domain));
        // The local part matters on its own: `noreply@` and `newsletter@` are
        // strong bulk markers regardless of which domain they sit on.
        if local.chars().count() >= MIN_TOKEN_LEN {
            out.push(bucket("from_local", local));
        }
    }

    if let Some(mailer) = input.x_mailer {
        // The whole header value, not word-split: ESP fingerprints are the
        // point, and splitting them loses the identity.
        let mailer = mailer.trim().to_lowercase();
        if !mailer.is_empty() {
            out.push(bucket("mailer", &mailer));
        }
    }

    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(subject: &'a str, snippet: &'a str, sender: &'a str) -> FeatureInput<'a> {
        FeatureInput {
            subject,
            snippet,
            sender_email: sender,
            x_mailer: None,
        }
    }

    #[test]
    fn every_bucket_is_in_range() {
        let f = features(&input(
            "Quarterly invoice",
            "Payment is due Friday",
            "billing@acme.example",
        ));
        assert!(!f.is_empty());
        assert!(f.iter().all(|b| (*b as usize) < BUCKETS));
    }

    #[test]
    fn identical_messages_produce_identical_features() {
        let a = features(&input("Hello there", "Body text", "a@x.example"));
        let b = features(&input("Hello there", "Body text", "a@x.example"));
        assert_eq!(a, b, "feature extraction must be deterministic across runs");
    }

    #[test]
    fn repetition_does_not_multiply_evidence() {
        // A presence model on purpose: a word repeated forty times in a
        // marketing template says something about the template, not about
        // whether the sender is junk.
        let once = features(&input("", "sale", "a@x.example"));
        let many = features(&input("", "sale sale sale sale sale", "a@x.example"));
        assert_eq!(once, many);
    }

    #[test]
    fn the_same_word_in_subject_and_body_lands_in_different_buckets() {
        // Namespacing keeps "invoice in the subject line" distinguishable from
        // "invoice mentioned somewhere in the text".
        let subject_only = features(&input("invoice", "", "a@x.example"));
        let body_only = features(&input("", "invoice", "a@x.example"));
        assert_ne!(subject_only, body_only);
    }

    #[test]
    fn the_sender_domain_and_local_part_are_separate_features() {
        let a = features(&input("", "", "noreply@acme.example"));
        let b = features(&input("", "", "noreply@other.example"));
        // Same local part, different domain: they share exactly one bucket.
        let shared: Vec<_> = a.iter().filter(|x| b.contains(x)).collect();
        assert_eq!(shared.len(), 1, "the from_local bucket should be the only overlap");
    }

    #[test]
    fn very_short_tokens_are_dropped() {
        // "a", "of", "to" appear in every message and separate nothing.
        let f = features(&input("a of to", "", ""));
        assert!(f.is_empty(), "got {f:?}");
    }

    #[test]
    fn case_and_punctuation_do_not_change_the_token() {
        let a = features(&input("Invoice!", "", "x@y.example"));
        let b = features(&input("invoice", "", "x@y.example"));
        assert_eq!(a, b);
    }

    #[test]
    fn accented_and_non_latin_text_still_tokenizes() {
        // The user's mail is multilingual; dropping non-ASCII would blind the
        // model to most of it.
        let f = features(&input("Reunión mañana", "", ""));
        assert_eq!(f.len(), 2, "expected two subject tokens, got {f:?}");
    }

    #[test]
    fn an_empty_message_yields_no_features() {
        assert!(features(&input("", "", "")).is_empty());
    }

    #[test]
    fn a_mailer_fingerprint_is_kept_whole() {
        let with = FeatureInput {
            subject: "",
            snippet: "",
            sender_email: "",
            x_mailer: Some("MailChimp Mailer - **CID**"),
        };
        assert_eq!(features(&with).len(), 1);
    }
}
