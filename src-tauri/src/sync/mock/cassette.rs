//! On-disk format for recorded provider HTTP interactions.
//!
//! One [`Cassette`] file = one test scenario (`happy_path`, `paginated`,
//! `401_refresh`, …). The interactions inside are stored in capture order
//! so the replay server can register them as ordered `wiremock` stubs.
//!
//! The format intentionally matches what `wiremock` needs to register a
//! stub with no further massaging: `method`, `url_path` (no query),
//! `query_params` (as key-value pairs to allow per-key matching), and a
//! `response.body_json` that's serialised back into the HTTP body verbatim.
//! See `src-tauri/tests/common/mock_server.rs` (the replay-side helper —
//! lives under `tests/` because `wiremock` is a dev-dependency).

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::models::error::{AppError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cassette {
    /// Human-readable scenario label, e.g. `outlook_paginated_inbox_sync`.
    /// Used in test failure messages and as the on-disk filename stem.
    pub scenario: String,
    /// `gmail` / `outlook` — picks the matching base URL when the replay
    /// server boots, and acts as a guard against loading a Gmail cassette
    /// into an Outlook test by mistake.
    pub provider: String,
    /// `true` when the sanitiser ran. Integration tests should refuse to
    /// load cassettes with `sanitized = false` unless explicitly opted in.
    pub sanitized: bool,
    pub recorded_at: i64,
    pub interactions: Vec<Interaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Interaction {
    pub request: RecordedRequest,
    pub response: RecordedResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedRequest {
    /// HTTP method, uppercased (`GET`, `POST`, `DELETE`, …).
    pub method: String,
    /// Path without scheme/host/query, e.g. `/v1.0/me/mailFolders/inbox/messages`.
    pub url_path: String,
    /// Query parameters as ordered `(key, value)` pairs. Multiple values for
    /// the same key are allowed (mirrors `wiremock::matchers::query_param`).
    #[serde(default)]
    pub query_params: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedResponse {
    pub status: u16,
    /// Response headers as ordered `(name, value)` pairs.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// Parsed JSON body. `None` when the response had no body (e.g. 204) or
    /// the body wasn't JSON (rare for Graph/Gmail).
    pub body_json: Option<serde_json::Value>,
}

impl Cassette {
    /// Write the cassette to `<dir>/<scenario>.json`. Returns the path.
    pub fn write_to(&self, dir: &Path) -> Result<std::path::PathBuf> {
        fs::create_dir_all(dir)
            .map_err(|e| AppError::IoError(format!("create cassette dir {}: {}", dir.display(), e)))?;
        let path = dir.join(format!("{}.json", self.scenario));
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::IoError(format!("serialize cassette {}: {}", self.scenario, e)))?;
        fs::write(&path, json).map_err(|e| AppError::IoError(format!("write {}: {}", path.display(), e)))?;
        Ok(path)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path).map_err(|e| AppError::IoError(format!("read {}: {}", path.display(), e)))?;
        serde_json::from_str(&raw).map_err(|e| AppError::IoError(format!("parse {}: {}", path.display(), e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Cassette {
        Cassette {
            scenario: "outlook_happy_path".into(),
            provider: "outlook".into(),
            sanitized: true,
            recorded_at: 1_700_000_000,
            interactions: vec![Interaction {
                request: RecordedRequest {
                    method: "GET".into(),
                    url_path: "/v1.0/me/mailFolders/inbox/messages".into(),
                    query_params: vec![("$top".into(), "10".into())],
                },
                response: RecordedResponse {
                    status: 200,
                    headers: vec![("content-type".into(), "application/json".into())],
                    body_json: Some(serde_json::json!({"value": []})),
                },
            }],
        }
    }

    #[test]
    fn cassette_roundtrips_through_write_and_load() {
        let tmp = tempfile::tempdir().unwrap();
        let original = sample();
        let path = original.write_to(tmp.path()).expect("write");
        let loaded = Cassette::load_from(&path).expect("load");
        assert_eq!(loaded.scenario, original.scenario);
        assert_eq!(loaded.provider, original.provider);
        assert_eq!(loaded.sanitized, original.sanitized);
        assert_eq!(loaded.interactions.len(), 1);
        assert_eq!(loaded.interactions[0].request.method, "GET");
        assert_eq!(
            loaded.interactions[0].response.body_json,
            Some(serde_json::json!({"value": []}))
        );
    }

    #[test]
    fn cassette_filename_uses_scenario_stem() {
        let tmp = tempfile::tempdir().unwrap();
        let path = sample().write_to(tmp.path()).unwrap();
        assert_eq!(
            path.file_name().and_then(|s| s.to_str()),
            Some("outlook_happy_path.json")
        );
    }
}
