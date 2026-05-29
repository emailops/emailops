//! Persisted user configuration for the task extraction subsystem.
//!
//! Task extraction is independent from memory fact extraction. Each pipeline
//! has its own enable flag, sync flag, categories, exclusions, and self-only
//! rule, so disabling one feature does not consume the other's extraction
//! backlog.

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::models::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskConfig {
    pub enabled: bool,
    pub extract_on_sync: bool,
    pub categories: Vec<String>,
    pub excluded_senders: Vec<String>,
    pub excluded_tags: Vec<String>,
    pub max_tasks_per_email: i32,
    pub backfill_days: i32,
    pub extract_from_self_only: bool,
}

impl Default for TaskConfig {
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
            max_tasks_per_email: 1,
            backfill_days: 30,
            extract_from_self_only: true,
        }
    }
}

impl TaskConfig {
    pub fn is_sender_excluded(&self, sender_email: &str) -> bool {
        let s = sender_email.to_ascii_lowercase();
        self.excluded_senders.iter().any(|needle| {
            let n = needle.trim().to_ascii_lowercase();
            !n.is_empty() && s.contains(&n)
        })
    }

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

    pub fn backfill_min_timestamp(&self, now: i64) -> Option<i64> {
        if self.backfill_days <= 0 {
            None
        } else {
            Some(now - (self.backfill_days as i64) * 86_400)
        }
    }
}

fn task_keys_present(db: &Database) -> Result<bool> {
    Ok(db.get_preference("task_enabled")?.is_some())
}

pub fn get_config(db: &Database) -> Result<TaskConfig> {
    if !task_keys_present(db)? {
        let seeded = seed_from_memory(db)?;
        save_config(db, &seeded)?;
        return Ok(seeded);
    }

    let defaults = TaskConfig::default();
    let enabled = db
        .get_preference("task_enabled")?
        .map(|v| v == "true")
        .unwrap_or(defaults.enabled);
    let extract_on_sync = db
        .get_preference("task_extract_on_sync")?
        .map(|v| v == "true")
        .unwrap_or(defaults.extract_on_sync);
    let categories = db
        .get_preference("task_categories")?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or(defaults.categories);
    let excluded_senders = db
        .get_preference("task_excluded_senders")?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or(defaults.excluded_senders);
    let excluded_tags = db
        .get_preference("task_excluded_tags")?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or(defaults.excluded_tags);
    let max_tasks_per_email = db
        .get_preference("task_max_tasks_per_email")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.max_tasks_per_email);
    let backfill_days = db
        .get_preference("task_backfill_days")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.backfill_days);
    let extract_from_self_only = db
        .get_preference("task_extract_from_self_only")?
        .map(|v| v == "true")
        .unwrap_or(defaults.extract_from_self_only);

    Ok(TaskConfig {
        enabled,
        extract_on_sync,
        categories,
        excluded_senders,
        excluded_tags,
        max_tasks_per_email,
        backfill_days,
        extract_from_self_only,
    })
}

pub fn save_config(db: &Database, config: &TaskConfig) -> Result<()> {
    db.set_preference("task_enabled", bool_str(config.enabled))?;
    db.set_preference("task_extract_on_sync", bool_str(config.extract_on_sync))?;
    db.set_preference("task_categories", &serde_json::to_string(&config.categories)?)?;
    db.set_preference(
        "task_excluded_senders",
        &serde_json::to_string(&config.excluded_senders)?,
    )?;
    db.set_preference("task_excluded_tags", &serde_json::to_string(&config.excluded_tags)?)?;
    db.set_preference("task_max_tasks_per_email", &config.max_tasks_per_email.to_string())?;
    db.set_preference("task_backfill_days", &config.backfill_days.to_string())?;
    db.set_preference("task_extract_from_self_only", bool_str(config.extract_from_self_only))?;
    Ok(())
}

fn seed_from_memory(db: &Database) -> Result<TaskConfig> {
    let defaults = TaskConfig::default();
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
    let max_tasks_per_email = db
        .get_preference("memory_max_tasks_per_email")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.max_tasks_per_email);
    let backfill_days = db
        .get_preference("memory_task_backfill_days")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(defaults.backfill_days);
    let extract_from_self_only = db
        .get_preference("memory_extract_from_self_only")?
        .map(|v| v == "true")
        .unwrap_or(defaults.extract_from_self_only);
    Ok(TaskConfig {
        enabled,
        extract_on_sync,
        categories,
        excluded_senders,
        excluded_tags,
        max_tasks_per_email,
        backfill_days,
        extract_from_self_only,
    })
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
    fn save_then_load_roundtrips() {
        let db = Database::new_for_testing().unwrap();
        let cfg = TaskConfig {
            enabled: false,
            extract_on_sync: false,
            categories: vec!["primary".into(), "updates".into()],
            excluded_senders: vec!["noreply@".into()],
            excluded_tags: vec!["promotion".into()],
            max_tasks_per_email: 3,
            backfill_days: 7,
            extract_from_self_only: false,
        };
        save_config(&db, &cfg).unwrap();
        let loaded = get_config(&db).unwrap();
        assert!(!loaded.enabled);
        assert!(!loaded.extract_on_sync);
        assert_eq!(loaded.max_tasks_per_email, 3);
        assert_eq!(loaded.backfill_days, 7);
        assert_eq!(loaded.categories, vec!["primary".to_string(), "updates".to_string()]);
    }

    #[test]
    fn first_read_seeds_from_memory_keys() {
        let db = Database::new_for_testing().unwrap();
        db.set_preference("memory_enabled", "true").unwrap();
        db.set_preference("memory_extract_on_sync", "false").unwrap();
        db.set_preference("memory_categories", r#"["primary","updates"]"#)
            .unwrap();
        db.set_preference("memory_max_tasks_per_email", "5").unwrap();
        db.set_preference("memory_task_backfill_days", "60").unwrap();
        let loaded = get_config(&db).unwrap();
        assert_eq!(loaded.max_tasks_per_email, 5);
        assert_eq!(loaded.backfill_days, 60);
        assert!(!loaded.extract_on_sync);
        assert_eq!(loaded.categories, vec!["primary".to_string(), "updates".to_string()]);
        let again = get_config(&db).unwrap();
        assert_eq!(again.max_tasks_per_email, 5);
    }

    #[test]
    fn backfill_min_timestamp_respects_window() {
        let mut cfg = TaskConfig {
            backfill_days: 30,
            ..TaskConfig::default()
        };
        let now = 1_700_000_000i64;
        assert_eq!(cfg.backfill_min_timestamp(now), Some(now - 30 * 86_400));
        cfg.backfill_days = 0;
        assert_eq!(cfg.backfill_min_timestamp(now), None);
    }
}
