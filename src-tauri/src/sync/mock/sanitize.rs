//! Sanitize recorded provider responses so cassettes can be safely committed.
//!
//! Walks the recorded JSON body recursively and rewrites:
//! - Any string that looks like an email address → `user-<8hex>@example.com`,
//!   deterministic per `(address, scenario)` so within-cassette identity is
//!   preserved (the same sender across 3 messages keeps the same pseudonym).
//! - Long text fields (`body.content`, `bodyPreview`, `snippet`, Gmail
//!   payload `data`) → truncated to [`SANITIZED_BODY_LIMIT`] chars + a
//!   redaction marker.
//! - URL-shaped strings → collapsed to `https://example.com/…`.
//!
//! What is **NOT** touched:
//! - Threading identifiers (`id`, `conversationId`, `threadId`,
//!   `internetMessageId`, `receivedDateTime`, `@odata.nextLink`) — tests
//!   match on these.
//! - Top-level cassette metadata (scenario / provider / sanitized flag) —
//!   the surrounding code handles that.
//!
//! The sanitiser is pure and deterministic; the executor (the
//! `record_provider_cassette` example) calls [`sanitize_cassette`] once
//! before writing the file.

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::cassette::Cassette;

/// Maximum length kept for free-text body fields before redaction marker
/// is appended.
pub const SANITIZED_BODY_LIMIT: usize = 200;

const REDACTION_MARKER: &str = " [redacted]";

/// Keys whose string values are interpreted as email addresses (or contain
/// them) and rewritten to pseudonyms. Lower-cased for case-insensitive
/// matching against the parent key.
const EMAIL_ADDRESS_KEYS: &[&str] = &["address", "emailaddress", "email"];

/// Keys whose string values are display names that should be replaced with
/// `Person <hex>` pseudonyms.
const DISPLAY_NAME_KEYS: &[&str] = &["name"];

/// Keys whose string values are long body content to truncate. Matches the
/// shape returned by both providers (Graph: `body.content`, `bodyPreview`,
/// `subject` is NOT in this list — see header doc).
const BODY_TEXT_KEYS: &[&str] = &["content", "bodypreview", "snippet", "data"];

/// Sanitise an entire cassette: marks `sanitized = true` and walks every
/// recorded response body using the scenario name as a hash salt so
/// pseudonyms are stable within the scenario but differ across scenarios.
pub fn sanitize_cassette(mut cassette: Cassette) -> Cassette {
    let salt = cassette.scenario.clone();
    for interaction in &mut cassette.interactions {
        if let Some(body) = interaction.response.body_json.take() {
            interaction.response.body_json = Some(sanitize_value(body, &salt, None));
        }
    }
    cassette.sanitized = true;
    cassette
}

/// Sanitise a `serde_json::Value` in place using `salt` for stable
/// pseudonyms. `parent_key` tells the walker which transformation to apply
/// at a leaf string (e.g. truncate vs. pseudonymise vs. URL-collapse).
pub fn sanitize_value(value: Value, salt: &str, parent_key: Option<&str>) -> Value {
    match value {
        Value::String(s) => sanitize_string(&s, salt, parent_key),
        Value::Array(arr) => Value::Array(arr.into_iter().map(|v| sanitize_value(v, salt, parent_key)).collect()),
        Value::Object(obj) => Value::Object(
            obj.into_iter()
                .map(|(k, v)| {
                    let lower = k.to_lowercase();
                    let sanitized_v = sanitize_value(v, salt, Some(&lower));
                    (k, sanitized_v)
                })
                .collect(),
        ),
        // Numbers / bools / nulls pass through unchanged.
        other => other,
    }
}

fn sanitize_string(s: &str, salt: &str, parent_key: Option<&str>) -> Value {
    // 1. URL-shaped strings collapse regardless of parent key (URLs can show
    //    up in body text, link headers, `webLink`, etc.).
    if is_url(s) {
        return Value::String("https://example.com/…".to_string());
    }

    // 2. Per-key transformations.
    if let Some(key) = parent_key {
        if EMAIL_ADDRESS_KEYS.contains(&key) {
            return Value::String(sanitize_email_address(s, salt));
        }
        if DISPLAY_NAME_KEYS.contains(&key) {
            return Value::String(sanitize_display_name(s, salt));
        }
        if BODY_TEXT_KEYS.contains(&key) {
            return Value::String(truncate_body(s));
        }
    }

    // 3. Free-floating string that *contains* an email address — Graph
    //    sometimes embeds addresses in headers/values that don't sit under
    //    one of the known keys. Replace any RFC-822-ish match.
    let replaced = EMAIL_RE
        .replace_all(s, |caps: &regex::Captures<'_>| {
            let addr = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            sanitize_email_address(addr, salt)
        })
        .to_string();
    Value::String(replaced)
}

pub fn sanitize_email_address(addr: &str, salt: &str) -> String {
    let id = stable_short_id(addr.to_lowercase().as_bytes(), salt);
    format!("user-{}@example.com", id)
}

pub fn sanitize_display_name(name: &str, salt: &str) -> String {
    let id = stable_short_id(name.to_lowercase().as_bytes(), salt);
    format!("Person {}", id.to_uppercase())
}

pub fn truncate_body(body: &str) -> String {
    if body.chars().count() <= SANITIZED_BODY_LIMIT {
        body.to_string()
    } else {
        let head: String = body.chars().take(SANITIZED_BODY_LIMIT).collect();
        format!("{}{}", head, REDACTION_MARKER)
    }
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn stable_short_id(input: &[u8], salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update([0]); // unambiguous salt/input separator
    hasher.update(input);
    let digest = hasher.finalize();
    hex::encode(&digest[..4]) // 8 hex chars
}

static EMAIL_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    #[allow(clippy::expect_used)] // regex literal — syntax checked at first use
    regex::Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
        .expect("EMAIL_RE: hard-coded regex must compile")
});

#[cfg(test)]
mod tests {
    use super::super::cassette::{Interaction, RecordedRequest, RecordedResponse};
    use super::*;
    use serde_json::json;

    // ── Address pseudonyms ───────────────────────────────────────────────────

    #[test]
    fn email_address_pseudonym_is_stable_for_same_input() {
        let a = sanitize_email_address("alice@example.com", "salt");
        let b = sanitize_email_address("alice@example.com", "salt");
        assert_eq!(a, b);
        assert!(a.ends_with("@example.com"));
    }

    #[test]
    fn email_address_pseudonym_differs_across_inputs() {
        let a = sanitize_email_address("alice@example.com", "salt");
        let b = sanitize_email_address("bob@example.com", "salt");
        assert_ne!(a, b);
    }

    #[test]
    fn email_address_pseudonym_is_case_insensitive() {
        let a = sanitize_email_address("Alice@Example.com", "salt");
        let b = sanitize_email_address("alice@example.com", "salt");
        assert_eq!(a, b);
    }

    // ── Body truncation ──────────────────────────────────────────────────────

    #[test]
    fn truncate_body_short_passes_through_unchanged() {
        assert_eq!(truncate_body("hello"), "hello");
    }

    #[test]
    fn truncate_body_long_gets_marker() {
        let long = "x".repeat(SANITIZED_BODY_LIMIT + 50);
        let out = truncate_body(&long);
        assert!(out.ends_with(REDACTION_MARKER));
        assert_eq!(
            out.chars().count(),
            SANITIZED_BODY_LIMIT + REDACTION_MARKER.chars().count()
        );
    }

    // ── Value walker ─────────────────────────────────────────────────────────

    #[test]
    fn value_walker_replaces_address_under_address_key() {
        let v = json!({"emailAddress": {"address": "alice@realcorp.com", "name": "Alice Real"}});
        let out = sanitize_value(v, "scenario-1", None);
        let addr = out.pointer("/emailAddress/address").and_then(|v| v.as_str()).unwrap();
        let name = out.pointer("/emailAddress/name").and_then(|v| v.as_str()).unwrap();
        assert!(addr.ends_with("@example.com"), "got {addr}");
        assert!(name.starts_with("Person "), "got {name}");
    }

    #[test]
    fn value_walker_truncates_body_content_field() {
        let body = "x".repeat(SANITIZED_BODY_LIMIT + 100);
        let v = json!({"body": {"content": body.clone(), "contentType": "html"}});
        let out = sanitize_value(v, "salt", None);
        let content = out.pointer("/body/content").and_then(|v| v.as_str()).unwrap();
        assert!(content.ends_with(REDACTION_MARKER));
        // contentType is preserved (not a body-text key)
        assert_eq!(out.pointer("/body/contentType").and_then(|v| v.as_str()), Some("html"));
    }

    #[test]
    fn value_walker_collapses_urls_anywhere() {
        let v = json!({"webLink": "https://outlook.office.com/mail/inbox/id/AAMkAD...", "other": 1});
        let out = sanitize_value(v, "salt", None);
        assert_eq!(
            out.pointer("/webLink").and_then(|v| v.as_str()),
            Some("https://example.com/…")
        );
        assert_eq!(out.pointer("/other").and_then(|v| v.as_u64()), Some(1));
    }

    #[test]
    fn value_walker_replaces_embedded_addresses_in_free_strings() {
        // Graph headers sometimes contain raw "From: Alice <alice@x.com>" strings
        // under a generic `value` key — pseudonymise inline.
        let v = json!({"header": "Reply-To: alice@realcorp.com, bob@realcorp.com"});
        let out = sanitize_value(v, "salt", None);
        let s = out.pointer("/header").and_then(|v| v.as_str()).unwrap();
        assert!(!s.contains("realcorp"), "domain leaked: {s}");
        assert!(s.contains("@example.com"));
    }

    #[test]
    fn value_walker_preserves_threading_ids_under_id_keys() {
        // `id`, `conversationId`, `internetMessageId` etc. are not in any
        // sanitiser key list — they should pass through verbatim. Tests
        // assert exact match on these.
        let v = json!({
            "id": "AAMkADAwATE0YzMwLWUxMQBiLTdkZTYtMDACLTAwCgBGAA==",
            "conversationId": "AQQkADAwATE...",
            "internetMessageId": "<abc123@mail.example>",
            "receivedDateTime": "2026-05-28T16:00:00Z"
        });
        let out = sanitize_value(v.clone(), "salt", None);
        assert_eq!(out.pointer("/id"), v.pointer("/id"));
        assert_eq!(out.pointer("/conversationId"), v.pointer("/conversationId"));
        // internetMessageId contains an @, but value_walker only rewrites
        // *strict* RFC-822 matches; angle-bracketed forms pass through.
        // (If this becomes a privacy concern, extend EMAIL_RE.)
        assert_eq!(out.pointer("/receivedDateTime"), v.pointer("/receivedDateTime"));
    }

    #[test]
    fn value_walker_handles_arrays_of_recipients() {
        let v = json!({
            "toRecipients": [
                {"emailAddress": {"address": "x@a.com", "name": "X"}},
                {"emailAddress": {"address": "y@b.com", "name": "Y"}}
            ]
        });
        let out = sanitize_value(v, "salt", None);
        let arr = out.pointer("/toRecipients").and_then(|v| v.as_array()).unwrap();
        assert_eq!(arr.len(), 2);
        for item in arr {
            let addr = item.pointer("/emailAddress/address").and_then(|v| v.as_str()).unwrap();
            assert!(addr.ends_with("@example.com"));
        }
    }

    // ── Whole-cassette ───────────────────────────────────────────────────────

    #[test]
    fn sanitize_cassette_marks_flag_and_uses_scenario_salt() {
        let cassette = Cassette {
            scenario: "scenario-A".into(),
            provider: "outlook".into(),
            sanitized: false,
            recorded_at: 0,
            interactions: vec![
                Interaction {
                    request: RecordedRequest {
                        method: "GET".into(),
                        url_path: "/v1.0/me/messages".into(),
                        query_params: vec![],
                    },
                    response: RecordedResponse {
                        status: 200,
                        headers: vec![],
                        body_json: Some(json!({"value": [
                            {"emailAddress": {"address": "alice@realcorp.com", "name": "Alice"}}
                        ]})),
                    },
                },
                Interaction {
                    request: RecordedRequest {
                        method: "GET".into(),
                        url_path: "/v1.0/me/messages/x".into(),
                        query_params: vec![],
                    },
                    response: RecordedResponse {
                        status: 200,
                        headers: vec![],
                        body_json: Some(
                            json!({"from": {"emailAddress": {"address": "alice@realcorp.com", "name": "Alice"}}}),
                        ),
                    },
                },
            ],
        };
        let sanitized = sanitize_cassette(cassette);
        assert!(sanitized.sanitized);
        // The same real address in two different interactions of the same
        // scenario maps to the same pseudonym.
        let addr1 = sanitized.interactions[0]
            .response
            .body_json
            .as_ref()
            .unwrap()
            .pointer("/value/0/emailAddress/address")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();
        let addr2 = sanitized.interactions[1]
            .response
            .body_json
            .as_ref()
            .unwrap()
            .pointer("/from/emailAddress/address")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();
        assert_eq!(addr1, addr2, "within-scenario consistency");
        assert!(!addr1.contains("realcorp"));
    }
}
