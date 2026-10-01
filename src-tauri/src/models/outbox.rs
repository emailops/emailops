//! Types of the local outbox (V032) behind undo send and scheduled send.

use serde::{Deserialize, Serialize};

use crate::sync::provider::EmailAttachment;

/// A message as the composer hands it over: everything needed to repeat the
/// exact send call later, or to reopen the composer with it. Stored as the
/// outbox row's JSON payload — never logged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingMessage {
    pub account_id: String,
    /// The message this answers. `Some` sends a reply (threading headers from
    /// that message); `None` sends a new message, forwards included.
    #[serde(default)]
    pub reply_to_email_id: Option<String>,
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    /// For a reply an empty subject means the parent's ("Re: …"); queueing
    /// fills it in so the stored message is complete.
    #[serde(default)]
    pub subject: String,
    /// Plain-text body.
    pub body: String,
    #[serde(default)]
    pub body_html: Option<String>,
    #[serde(default)]
    pub inline_images: Vec<EmailAttachment>,
    #[serde(default)]
    pub attachments: Vec<EmailAttachment>,
}

impl OutgoingMessage {
    pub fn kind(&self) -> OutboxKind {
        if self.reply_to_email_id.is_some() {
            OutboxKind::Reply
        } else {
            OutboxKind::New
        }
    }
}

/// When a queued message goes out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum OutboxSchedule {
    /// Undo send: `delay_secs` from now, one of [`UNDO_SEND_DELAYS`].
    #[serde(rename_all = "camelCase")]
    Undo { delay_secs: i64 },
    /// Scheduled send at `send_at` (unix seconds, in the future).
    #[serde(rename_all = "camelCase")]
    At { send_at: i64 },
}

/// The undo-send windows the setting offers (0 = off, sends directly).
pub const UNDO_SEND_DELAYS: [i64; 4] = [5, 10, 20, 30];

/// Preference holding the undo-send window in seconds ("0" = off). Unset
/// means the default of 10 s, which the composer applies.
pub const UNDO_SEND_DELAY_PREF: &str = "compose.undo_send_delay_secs";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutboxKind {
    New,
    Reply,
}

impl OutboxKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxKind::New => "new",
            OutboxKind::Reply => "reply",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "new" => Some(OutboxKind::New),
            "reply" => Some(OutboxKind::Reply),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutboxOrigin {
    /// Sent with an undo window: the user already pressed Send.
    Undo,
    /// Scheduled for a chosen time.
    Scheduled,
}

impl OutboxOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxOrigin::Undo => "undo",
            OutboxOrigin::Scheduled => "scheduled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "undo" => Some(OutboxOrigin::Undo),
            "scheduled" => Some(OutboxOrigin::Scheduled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutboxStatus {
    Scheduled,
    Sending,
    Sent,
    Failed,
    Cancelled,
}

impl OutboxStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxStatus::Scheduled => "scheduled",
            OutboxStatus::Sending => "sending",
            OutboxStatus::Sent => "sent",
            OutboxStatus::Failed => "failed",
            OutboxStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "scheduled" => Some(OutboxStatus::Scheduled),
            "sending" => Some(OutboxStatus::Sending),
            "sent" => Some(OutboxStatus::Sent),
            "failed" => Some(OutboxStatus::Failed),
            "cancelled" => Some(OutboxStatus::Cancelled),
            _ => None,
        }
    }
}

/// Why a row failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutboxFailureKind {
    /// The provider refused the send (or it could not be reached).
    Error,
    /// The app stopped while the row was being sent: it may or may not have
    /// gone out, so it is never resent automatically.
    Interrupted,
}

impl OutboxFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OutboxFailureKind::Error => "error",
            OutboxFailureKind::Interrupted => "interrupted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "error" => Some(OutboxFailureKind::Error),
            "interrupted" => Some(OutboxFailureKind::Interrupted),
            _ => None,
        }
    }
}

/// One outbox row as the Scheduled view lists it — no payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxEntry {
    pub id: String,
    pub account_id: String,
    pub kind: OutboxKind,
    pub reply_to_email_id: Option<String>,
    pub origin: OutboxOrigin,
    pub to_addresses: Vec<String>,
    pub cc_addresses: Vec<String>,
    pub subject: String,
    pub attachment_count: i64,
    pub send_at: i64,
    pub status: OutboxStatus,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub failure_kind: Option<OutboxFailureKind>,
    pub created_at: i64,
}
