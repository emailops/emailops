//! Shared documents (V036): documents and sheets kept in sync between EmailOps
//! installs by email. The sync itself lives in `services::shared_docs`.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::models::error::{AppError, Result};

/// What kind of editor a document opens in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "lowercase")]
pub enum DocKind {
    Doc,
    Sheet,
}

impl DocKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Doc => "doc",
            Self::Sheet => "sheet",
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "doc" => Ok(Self::Doc),
            "sheet" => Ok(Self::Sheet),
            other => Err(AppError::InvalidInput(format!("Unknown document kind: {other}"))),
        }
    }
}

/// Where a document stands on this install.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "lowercase")]
pub enum DocStatus {
    /// Someone shared it with this account; nothing is sent until accepted.
    Invited,
    /// Editable; changes are mailed once the user has shared or accepted it.
    Active,
    /// Declined or left: kept read-only, later messages ignored.
    Left,
}

impl DocStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invited => "invited",
            Self::Active => "active",
            Self::Left => "left",
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "invited" => Ok(Self::Invited),
            "active" => Ok(Self::Active),
            "left" => Ok(Self::Left),
            other => Err(AppError::InvalidInput(format!("Unknown document status: {other}"))),
        }
    }
}

/// A shared document as the list and the editor header show it. The content
/// itself travels separately, as Yjs bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../src/types/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct SharedDoc {
    pub id: String,
    pub account_id: String,
    pub kind: DocKind,
    pub title: String,
    pub status: DocStatus,
    /// Normalized addresses, this account's own included. A document never
    /// shared has only its owner.
    pub participants: Vec<String>,
    /// When the user agreed to mail its changes (shared or accepted).
    pub consented_at: Option<i64>,
    /// Changes not mailed yet, since this time.
    pub dirty_since: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}
