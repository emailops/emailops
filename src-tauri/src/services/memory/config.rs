//! Persisted user configuration for the memory subsystem.
//!
//! Every field is surfaced in the Memory Settings tab. Defaults are tuned for
//! a typical personal mailbox — the user can override per-mailbox if needed.
//! Config is stored as scalar values in `user_preferences` (same pattern as
//! `ClassificationConfig`), so adding / renaming fields is backward-compatible.

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::models::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConfig {
    /// Master switch. When false, sync-time extraction, consolidation tick,
    /// and backfill are all skipped. Existing facts/tasks still render in
    /// the UI so the user can inspect history.
    pub enabled: bool,

    /// Run the per-email extractor automatically after each sync. Off disables
    /// background learning; backfill is still available manually.
    pub extract_on_sync: bool,

    /// Gmail inbox categories that feed the memory extractor. Empty = all.
    /// Default: `["primary"]` — promotions/social rarely carry durable signal.
    pub categories: Vec<String>,

    /// Sender-email patterns (case-insensitive substring match) whose emails
    /// are skipped by the extractor. Useful for noreply/notifications.
    pub excluded_senders: Vec<String>,

    /// Tag values (from `email_tags.tag_value`, any tag_type) that cause an
    /// email to be skipped by the extractor. Matches against already-classified
    /// intents and topics — default excludes categories with very low task/fact
    /// yield (marketing/newsletter/promotion) and domains that typically carry
    /// transactional-but-not-personal signal (sales/hiring).
    pub excluded_tags: Vec<String>,

    /// How often the dream-consolidation job runs in the background. 0 disables
    /// the scheduled tick (it still runs right after sync-time extraction).
    pub consolidation_interval_minutes: i32,

    /// Score threshold above which a candidate fact is promoted to `'promoted'`.
    /// Range 0.0–1.0. Default 0.75.
    pub promote_threshold: f32,

    /// Retire candidate facts older than this many days whose score is still
    /// below `retire_below_score`. Default 14.
    pub candidate_ttl_days: i32,

    /// Delete `interaction_events` older than this many days. Default 30.
    pub event_retention_days: i32,

    /// Upper bound on how many emails one extractor run processes. Applied to
    /// both the per-sync run and each backfill tick. Default 50.
    pub backfill_batch_size: i32,

    /// When true (default), BOTH memory-fact and task extraction only run on
    /// emails the user authored themselves (sender_email == account.email).
    /// The user's own outbound mail is where durable preferences, decisions,
    /// real commitments, and communication-style signal live — inbound
    /// promotions/notifications add mostly noise. Off treats every email
    /// (subject to category/sender/tag filters) as a candidate.
    pub extract_from_self_only: bool,

    /// Preferred natural language for all LLM-generated output (fact text,
    /// task titles, thread summaries, chat answers). Injected into every
    /// extraction/consolidation prompt so the model produces consistent
    /// localized output. Default: "Spanish".
    pub ai_output_language: String,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            extract_on_sync: false,
            categories: vec!["primary".to_string()],
            excluded_senders: Vec::new(),
            excluded_tags: vec![
                "marketing".to_string(),
                "sales".to_string(),
                "hiring".to_string(),
                "newsletter".to_string(),
                "promotion".to_string(),
            ],
            consolidation_interval_minutes: 30,
            promote_threshold: 0.75,
            candidate_ttl_days: 14,
            event_retention_days: 30,
            backfill_batch_size: 50,
            extract_from_self_only: true,
            ai_output_language: "English".to_string(),
        }
    }
}

impl MemoryConfig {
    /// Returns true when `sender_email` matches any excluded pattern. Empty
    /// patterns are ignored (treated as "no exclusion").
    pub fn is_sender_excluded(&self, sender_email: &str) -> bool {
        let s = sender_email.to_ascii_lowercase();
        self.excluded_senders.iter().any(|needle| {
            let n = needle.trim().to_ascii_lowercase();
            !n.is_empty() && s.contains(&n)
        })
    }

    /// Returns true when `category` is allowed. Empty `categories` means allow all.
    pub fn is_category_allowed(&self, category: &str) -> bool {
        self.categories.is_empty() || self.categories.iter().any(|c| c.eq_ignore_ascii_case(category))
    }

    /// Returns true when any value in `tag_values` matches an excluded tag
    /// (case-insensitive, exact match on normalised values).
    pub fn is_tag_excluded<'a, I: IntoIterator<Item = &'a str>>(&self, tag_values: I) -> bool {
        if self.excluded_tags.is_empty() {
            return false;
        }
        let excluded: Vec<String> = self
            .excluded_tags
            .iter()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        if excluded.is_empty() {
            return false;
        }
        for v in tag_values {
            let n = v.trim().to_ascii_lowercase();
            if excluded.contains(&n) {
                return true;
            }
        }
        false
    }
}

pub fn get_config(db: &Database) -> Result<MemoryConfig> {
    let defaults = MemoryConfig::default();
    let enabled = db
        .get_preference("memory_enabled")?
        .map(|v| v == "true")
        .unwrap_or(defaults.enabled);
    let extract_on_sync = db
        .get_preference("memory_extract_on_sync")?
        .map(|v| v == "true")
        .unwrap_or(defaults.extract_on_sync);
    let categories = db
        .get_preference("memory_categories")?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or(defaults.categories);
    let excluded_senders = db
        .get_preference("memory_excluded_senders")?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or(defaults.excluded_senders);
    let excluded_tags = db
        .get_preference("memory_excluded_tags")?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or(defaults.excluded_tags);
    let consolidation_interval_minutes = db
        .get_preference("memory_consolidation_interval_minutes")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.consolidation_interval_minutes);
    let promote_threshold = db
        .get_preference("memory_promote_threshold")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.promote_threshold);
    let candidate_ttl_days = db
        .get_preference("memory_candidate_ttl_days")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.candidate_ttl_days);
    let event_retention_days = db
        .get_preference("memory_event_retention_days")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.event_retention_days);
    let backfill_batch_size = db
        .get_preference("memory_backfill_batch_size")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.backfill_batch_size);
    let extract_from_self_only = db
        .get_preference("memory_extract_from_self_only")?
        .map(|v| v == "true")
        .unwrap_or(defaults.extract_from_self_only);
    let ai_output_language = db
        .get_preference("ai_output_language")?
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(defaults.ai_output_language);

    Ok(MemoryConfig {
        enabled,
        extract_on_sync,
        categories,
        excluded_senders,
        excluded_tags,
        consolidation_interval_minutes,
        promote_threshold,
        candidate_ttl_days,
        event_retention_days,
        backfill_batch_size,
        extract_from_self_only,
        ai_output_language,
    })
}

pub fn save_config(db: &Database, config: &MemoryConfig) -> Result<()> {
    db.set_preference("memory_enabled", bool_str(config.enabled))?;
    db.set_preference("memory_extract_on_sync", bool_str(config.extract_on_sync))?;
    db.set_preference("memory_categories", &serde_json::to_string(&config.categories)?)?;
    db.set_preference(
        "memory_excluded_senders",
        &serde_json::to_string(&config.excluded_senders)?,
    )?;
    db.set_preference("memory_excluded_tags", &serde_json::to_string(&config.excluded_tags)?)?;
    db.set_preference(
        "memory_consolidation_interval_minutes",
        &config.consolidation_interval_minutes.to_string(),
    )?;
    db.set_preference("memory_promote_threshold", &config.promote_threshold.to_string())?;
    db.set_preference("memory_candidate_ttl_days", &config.candidate_ttl_days.to_string())?;
    db.set_preference("memory_event_retention_days", &config.event_retention_days.to_string())?;
    db.set_preference("memory_backfill_batch_size", &config.backfill_batch_size.to_string())?;
    db.set_preference("memory_extract_from_self_only", bool_str(config.extract_from_self_only))?;
    db.set_preference("ai_output_language", &config.ai_output_language)?;
    Ok(())
}

fn bool_str(v: bool) -> &'static str {
    if v {
        "true"
    } else {
        "false"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_sender_excluded_substring_match() {
        let cfg = MemoryConfig {
            excluded_senders: vec!["noreply@".into(), "@bounces.example.com".into()],
            ..MemoryConfig::default()
        };
        assert!(cfg.is_sender_excluded("noreply@stripe.com"));
        assert!(cfg.is_sender_excluded("x@bounces.example.com"));
        assert!(!cfg.is_sender_excluded("alice@example.com"));
    }

    #[test]
    fn is_category_allowed_empty_means_all() {
        let mut cfg = MemoryConfig::default();
        cfg.categories.clear();
        assert!(cfg.is_category_allowed("primary"));
        assert!(cfg.is_category_allowed("social"));
    }

    #[test]
    fn is_category_allowed_checks_membership() {
        let cfg = MemoryConfig::default();
        assert!(cfg.is_category_allowed("primary"));
        assert!(!cfg.is_category_allowed("promotions"));
    }

    #[test]
    fn save_then_load_roundtrips() {
        let db = Database::new_for_testing().unwrap();
        let cfg = MemoryConfig {
            enabled: false,
            extract_on_sync: false,
            categories: vec!["primary".into(), "updates".into()],
            excluded_senders: vec!["noreply@".into()],
            excluded_tags: vec!["promotion".into(), "newsletter".into()],
            consolidation_interval_minutes: 60,
            promote_threshold: 0.8,
            candidate_ttl_days: 7,
            event_retention_days: 14,
            backfill_batch_size: 25,
            extract_from_self_only: false,
            ai_output_language: "English".to_string(),
        };
        save_config(&db, &cfg).unwrap();
        let loaded = get_config(&db).unwrap();
        assert!(!loaded.enabled);
        assert!(!loaded.extract_from_self_only);
        assert!(!loaded.extract_on_sync);
        assert_eq!(loaded.categories, vec!["primary".to_string(), "updates".to_string()]);
        assert_eq!(loaded.excluded_senders, vec!["noreply@".to_string()]);
        assert_eq!(
            loaded.excluded_tags,
            vec!["promotion".to_string(), "newsletter".to_string()]
        );
        assert_eq!(loaded.consolidation_interval_minutes, 60);
        assert!((loaded.promote_threshold - 0.8).abs() < 1e-4);
        assert_eq!(loaded.candidate_ttl_days, 7);
        assert_eq!(loaded.event_retention_days, 14);
        assert_eq!(loaded.backfill_batch_size, 25);
        assert_eq!(loaded.ai_output_language, "English");
    }

    #[test]
    fn is_tag_excluded_matches_defaults() {
        let cfg = MemoryConfig::default();
        assert!(cfg.is_tag_excluded(["marketing"]));
        assert!(cfg.is_tag_excluded(["Newsletter"])); // case-insensitive
        assert!(cfg.is_tag_excluded(["personal", "sales"]));
        assert!(!cfg.is_tag_excluded(["personal"]));
        assert!(!cfg.is_tag_excluded::<[&str; 0]>([]));
    }
}
