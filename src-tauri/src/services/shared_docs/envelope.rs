//! The `.eodoc` attachment that carries a shared document's changes between
//! EmailOps installs. Pure: encode, decode and validate.
//!
//! A message is recognised as a document message by this attachment alone —
//! its file name ends in [`ENVELOPE_EXTENSION`] — not by a custom header:
//! none of the three send paths can set one today, and the sync keeps only an
//! allowlist of headers.
//!
//! The sender is not part of the envelope: it is the message's own `From`,
//! which the receiving side checks against the participants.

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::models::error::{AppError, Result};
use crate::models::shared_docs::DocKind;
use crate::services::sender_controls::normalize_address;

/// File-name suffix of the envelope attachment.
pub const ENVELOPE_EXTENSION: &str = ".eodoc";
/// MIME type the envelope is sent with.
pub const ENVELOPE_MIME: &str = "application/vnd.emailops.doc+json";
/// Largest envelope accepted or sent. Providers cap a message near 25 MB and
/// base64 grows it by a third; a document past this is refused, not split.
pub const MAX_ENVELOPE_BYTES: usize = 8 * 1024 * 1024;
/// Most people one document is shared with, owner included.
pub const MAX_PARTICIPANTS: usize = 50;
pub const MAX_TITLE_CHARS: usize = 200;

const VERSION: u32 = 1;

/// Why a message carries an envelope, which decides what the receiving inbox
/// does with the message itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Purpose {
    /// "X shared a document with you": stays in the inbox.
    Invitation,
    /// Changes only, mailed in the background: archived once applied.
    Update,
    /// A document the user attached to an email they wrote: the email is
    /// theirs, so it stays in the inbox like any other.
    Message,
}

/// A decoded, validated envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub purpose: Purpose,
    pub doc_id: String,
    pub kind: DocKind,
    pub title: String,
    /// Normalized addresses, owner included, no duplicates.
    pub participants: Vec<String>,
    /// The sender's state vector after this update: what it has seen.
    pub state_vector: Vec<u8>,
    /// Yjs v1 update: the changes the receivers may lack.
    pub update: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Wire {
    v: u32,
    purpose: Purpose,
    doc_id: String,
    kind: DocKind,
    title: String,
    participants: Vec<String>,
    state_vector: String,
    update: String,
}

/// The attachment file name for a document.
pub fn file_name(doc_id: &str) -> String {
    format!("{doc_id}{ENVELOPE_EXTENSION}")
}

/// Whether an attachment is a document envelope.
pub fn is_envelope_file(filename: &str) -> bool {
    filename.to_ascii_lowercase().ends_with(ENVELOPE_EXTENSION)
}

/// Pure: the title as stored — trimmed, one line, at most [`MAX_TITLE_CHARS`].
pub fn normalize_title(raw: &str) -> Result<String> {
    let title = raw.trim();
    if title.is_empty() {
        return Err(AppError::InvalidInput("A document needs a title".into()));
    }
    if title.chars().any(char::is_control) {
        return Err(AppError::InvalidInput("A document title must be one line".into()));
    }
    Ok(title.chars().take(MAX_TITLE_CHARS).collect())
}

/// Pure: participants normalized, deduplicated and bounded.
pub fn normalize_participants(raw: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for address in raw {
        let address = normalize_address(address)?;
        if !out.contains(&address) {
            out.push(address);
        }
    }
    if out.is_empty() || out.len() > MAX_PARTICIPANTS {
        return Err(AppError::InvalidInput(format!(
            "A shared document has between 1 and {MAX_PARTICIPANTS} participants"
        )));
    }
    Ok(out)
}

fn valid_doc_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}

pub fn encode(envelope: &Envelope) -> Result<Vec<u8>> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let bytes = serde_json::to_vec(&Wire {
        v: VERSION,
        purpose: envelope.purpose,
        doc_id: envelope.doc_id.clone(),
        kind: envelope.kind,
        title: envelope.title.clone(),
        participants: envelope.participants.clone(),
        state_vector: b64.encode(&envelope.state_vector),
        update: b64.encode(&envelope.update),
    })?;
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(AppError::InvalidInput(
            "This document has grown too large to send by email".into(),
        ));
    }
    Ok(bytes)
}

/// Decode and validate an envelope received from anyone. Everything in it is
/// untrusted: an envelope that fails any check is refused whole.
pub fn decode(bytes: &[u8]) -> Result<Envelope> {
    let bad = |why: &str| AppError::InvalidInput(format!("Not a valid shared document message: {why}"));
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(bad("too large"));
    }
    let wire: Wire = serde_json::from_slice(bytes).map_err(|_| bad("unreadable"))?;
    if wire.v != VERSION {
        return Err(bad("unsupported version"));
    }
    if !valid_doc_id(&wire.doc_id) {
        return Err(bad("document id"));
    }
    let b64 = base64::engine::general_purpose::STANDARD;
    let state_vector = b64.decode(&wire.state_vector).map_err(|_| bad("state vector"))?;
    let update = b64.decode(&wire.update).map_err(|_| bad("update"))?;
    super::crdt::validate_update(&update)?;
    super::crdt::lacks(&state_vector, &state_vector)?;
    Ok(Envelope {
        purpose: wire.purpose,
        doc_id: wire.doc_id,
        kind: wire.kind,
        title: normalize_title(&wire.title)?,
        participants: normalize_participants(&wire.participants)?,
        state_vector,
        update,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::shared_docs::crdt;

    const DOC_ID: &str = "6f1c2a7e-3b4d-4e5f-8a9b-0c1d2e3f4a5b";

    fn envelope() -> Envelope {
        Envelope {
            purpose: Purpose::Update,
            doc_id: DOC_ID.into(),
            kind: DocKind::Sheet,
            title: "Budget 2027".into(),
            participants: vec!["ana@example.com".into(), "ben@example.org".into()],
            state_vector: crdt::empty_state_vector(),
            update: crdt::empty_state(),
        }
    }

    fn wire_with(field: &str, value: serde_json::Value) -> Vec<u8> {
        let mut json: serde_json::Value = serde_json::from_slice(&encode(&envelope()).unwrap()).unwrap();
        json[field] = value;
        serde_json::to_vec(&json).unwrap()
    }

    #[test]
    fn an_envelope_survives_the_round_trip() {
        assert_eq!(decode(&encode(&envelope()).unwrap()).unwrap(), envelope());
    }

    #[test]
    fn an_envelope_with_a_bad_field_is_refused() {
        for (field, value) in [
            ("v", serde_json::json!(2)),
            ("docId", serde_json::json!("../../etc")),
            ("kind", serde_json::json!("slides")),
            ("purpose", serde_json::json!("spam")),
            ("title", serde_json::json!("   ")),
            ("title", serde_json::json!("two\nlines")),
            ("participants", serde_json::json!([])),
            ("participants", serde_json::json!(["not an address"])),
            ("update", serde_json::json!("%%%")),
            ("update", serde_json::json!("/////w==")),
            ("stateVector", serde_json::json!("/////w==")),
            ("extra", serde_json::json!(1)),
        ] {
            assert!(decode(&wire_with(field, value.clone())).is_err(), "{field} = {value}");
        }
        assert!(decode(b"not json").is_err());
    }

    #[test]
    fn participants_are_normalized_and_deduplicated() {
        let raw = vec![
            "Ana@Example.com".to_string(),
            " ana@example.com".into(),
            "ben@example.org".into(),
        ];
        assert_eq!(
            normalize_participants(&raw).unwrap(),
            vec!["ana@example.com".to_string(), "ben@example.org".into()]
        );
        let too_many: Vec<String> = (0..=MAX_PARTICIPANTS).map(|i| format!("p{i}@example.com")).collect();
        assert!(normalize_participants(&too_many).is_err());
    }

    #[test]
    fn titles_are_trimmed_and_capped() {
        assert_eq!(normalize_title("  Notes  ").unwrap(), "Notes");
        assert_eq!(
            normalize_title(&"x".repeat(500)).unwrap().chars().count(),
            MAX_TITLE_CHARS
        );
    }

    #[test]
    fn envelope_files_are_recognised_by_extension() {
        assert!(is_envelope_file(&file_name(DOC_ID)));
        assert!(is_envelope_file("X.EODOC"));
        assert!(!is_envelope_file("budget.xlsx"));
    }

    #[test]
    fn an_oversized_envelope_is_refused_both_ways() {
        let mut big = envelope();
        big.update = vec![0; MAX_ENVELOPE_BYTES];
        assert!(encode(&big).is_err());
        assert!(decode(&vec![b' '; MAX_ENVELOPE_BYTES + 1]).is_err());
    }
}
