use std::sync::Arc;

use crate::db::Database;
use crate::models::error::Result;
use crate::models::{FilteredEmailsResult, QuickFilterStats, SmartFilterPref, SmartFilterSuggestion};

/// Calculate fresh suggestions, persist them to DB, and return stats
pub fn refresh_filter_stats(db: &Arc<Database>, account_id: &str) -> Result<QuickFilterStats> {
    // Read removed prefs to exclude from suggestions
    let prefs = db.get_filter_prefs(account_id)?;

    let excluded_domains: Vec<String> = prefs
        .iter()
        .filter(|p| p.status == "removed" && p.filter_type == "domain")
        .map(|p| p.filter_value.clone())
        .collect();

    let excluded_senders: Vec<String> = prefs
        .iter()
        .filter(|p| p.status == "removed" && p.filter_type == "sender")
        .map(|p| p.filter_value.clone())
        .collect();

    let stats = db.get_quick_filter_stats(account_id, &excluded_domains, &excluded_senders)?;

    // Persist suggestions to DB
    let mut to_save: Vec<SmartFilterSuggestion> = Vec::new();
    for d in &stats.top_domains {
        to_save.push(SmartFilterSuggestion {
            filter_type: "domain".to_string(),
            filter_value: d.value.clone(),
            count: d.count,
        });
    }
    for s in &stats.top_senders {
        to_save.push(SmartFilterSuggestion {
            filter_type: "sender".to_string(),
            filter_value: s.value.clone(),
            count: s.count,
        });
    }

    // Add tag-based suggestions (company, intent, topic, priority).
    // `company` is first so the sidebar renders the Companies section above
    // the other tag groups — `Object.entries(tagGroups)` preserves insertion
    // order in the frontend's `SmartFilters.tsx`.
    for tag_type in ["company", "intent", "topic", "priority"] {
        if let Ok(tag_stats) = db.get_tag_stats(account_id, tag_type, 15) {
            for (value, count) in tag_stats {
                to_save.push(SmartFilterSuggestion {
                    filter_type: tag_type.to_string(),
                    filter_value: value,
                    count,
                });
            }
        }
    }

    db.save_filter_suggestions(account_id, &to_save)?;

    Ok(stats)
}

/// Load previously calculated suggestions from DB
pub fn get_saved_suggestions(db: &Arc<Database>, account_id: &str) -> Result<Vec<SmartFilterSuggestion>> {
    db.get_filter_suggestions(account_id)
}

pub fn get_filtered_emails(
    db: &Arc<Database>,
    account_id: &str,
    domain: Option<&str>,
    sender_email: Option<&str>,
    tag_type: Option<&str>,
    tag_value: Option<&str>,
    attachment_ext: Option<&str>,
    limit: i32,
    offset: i32,
) -> Result<FilteredEmailsResult> {
    db.get_filtered_emails(
        account_id,
        domain,
        sender_email,
        tag_type,
        tag_value,
        attachment_ext,
        limit,
        offset,
    )
}

pub fn get_filter_prefs(db: &Arc<Database>, account_id: &str) -> Result<Vec<SmartFilterPref>> {
    db.get_filter_prefs(account_id)
}

pub fn pin_filter(db: &Arc<Database>, account_id: &str, filter_type: &str, filter_value: &str) -> Result<()> {
    let id = format!("{}:{}:{}", account_id, filter_type, filter_value);
    db.upsert_filter_pref(&id, filter_type, filter_value, "pinned", account_id)
}

pub fn remove_filter(db: &Arc<Database>, account_id: &str, filter_type: &str, filter_value: &str) -> Result<()> {
    let id = format!("{}:{}:{}", account_id, filter_type, filter_value);
    db.upsert_filter_pref(&id, filter_type, filter_value, "removed", account_id)
}

pub fn delete_filter_pref(db: &Arc<Database>, account_id: &str, filter_type: &str, filter_value: &str) -> Result<()> {
    let id = format!("{}:{}:{}", account_id, filter_type, filter_value);
    db.delete_filter_pref(&id, account_id)
}
