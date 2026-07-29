//! Captured RFC 5322 headers.
//!
//! A deliberately narrow, typed subset — not the full header set. Two reasons:
//! privacy (we store the minimum the detector needs, not everything the sender
//! wrote) and query cost (typed columns beat a key/value table that needs a
//! join per message, and beat a JSON blob that needs parsing on every read).
//!
//! Populated by `sync::header_capture::capture`. Persisted to the
//! `email_headers` table.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The subset of headers junk detection needs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawHeaders {
    /// The **topmost** `Authentication-Results` only.
    ///
    /// This is a security boundary, not a convenience. Your own MTA prepends
    /// its verdict above everything the sender wrote, and an attacker can paste
    /// a forged `spf=pass; dmarc=pass` line into their message. Keeping any
    /// instance but the first would let the attacker choose their own
    /// authentication result.
    pub auth_results: Option<String>,
    /// The `authserv-id` from `auth_results` — the identity of the MTA that
    /// made the claim. A verdict is only trustworthy when this matches the
    /// server we expect for the account.
    pub authserv_id: Option<String>,
    pub received_spf: Option<String>,
    /// `d=` values from every `DKIM-Signature`, in header order.
    pub dkim_domains: Vec<String>,
    pub return_path: Option<String>,
    pub reply_to: Option<String>,
    /// Raw `From`, display name included — the lookalike and impersonation
    /// checks need the unparsed form.
    pub from_raw: Option<String>,
    pub to_raw: Option<String>,
    pub list_id: Option<String>,
    pub list_unsubscribe: Option<String>,
    /// RFC 8058 one-click. Presence means a compliant bulk sender.
    pub list_unsubscribe_post: Option<String>,
    pub precedence: Option<String>,
    pub x_mailer: Option<String>,
    pub content_type: Option<String>,
    pub received_count: usize,
    /// The **bottom-most** `Received` — the origin hop, the one the sender
    /// could not forge by prepending.
    pub first_received: Option<String>,
    /// Every `X-Spam-*` header, joined. On IMAP this carries the server's own
    /// verdict and is the single highest-recall signal available.
    pub spam_headers: Option<String>,
    /// Allowlisted long tail, so a new signal doesn't need a migration.
    pub extra: BTreeMap<String, String>,
}

impl RawHeaders {
    /// Did the provider give us anything at all?
    ///
    /// Drives `Band::Unknown` rather than a confident `Clean`: absence of
    /// evidence is not evidence of innocence.
    pub fn is_empty(&self) -> bool {
        *self == RawHeaders::default()
    }
}
