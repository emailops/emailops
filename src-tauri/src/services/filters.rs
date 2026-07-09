use std::sync::Arc;

use crate::db::Database;
use crate::models::error::Result;
use crate::models::{FilteredEmailsResult, QuickFilterStats, SmartFilterPref, SmartFilterSuggestion};

/// Dedup key for saved suggestions. Sender addresses are case-insensitive
/// identifiers; every other filter value is compared verbatim.
fn suggestion_key(filter_type: &str, filter_value: &str) -> String {
    if filter_type == "sender" {
        format!("{}:{}", filter_type, filter_value.to_lowercase())
    } else {
        format!("{}:{}", filter_type, filter_value)
    }
}

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
        for (value, count) in db.get_tag_stats(account_id, tag_type, 15)? {
            to_save.push(SmartFilterSuggestion {
                filter_type: tag_type.to_string(),
                filter_value: value,
                count,
            });
        }
    }

    // Pinned filters must always carry a count, even when they fall outside
    // the top-N stats (or, for the account owner's own address, are excluded
    // from them). Compute their thread counts directly so the sidebar never
    // shows a pinned filter as 0. Sender keys compare case-insensitively,
    // matching the frontend's filterMatchKey.
    let saved_keys: std::collections::HashSet<String> = to_save
        .iter()
        .map(|s| suggestion_key(&s.filter_type, &s.filter_value))
        .collect();
    for p in prefs.iter().filter(|p| p.status == "pinned") {
        if saved_keys.contains(&suggestion_key(&p.filter_type, &p.filter_value)) {
            continue;
        }
        let count = db.count_filter_threads(account_id, &p.filter_type, &p.filter_value)?;
        to_save.push(SmartFilterSuggestion {
            filter_type: p.filter_type.clone(),
            filter_value: p.filter_value.clone(),
            count,
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_email(db: &Database, id: &str, account: &str, thread: &str, sender_email: &str, mailbox: &str) {
        let domain = sender_email.rsplit_once('@').map(|(_, d)| d.to_lowercase()).unwrap();
        db.connection()
            .execute(
                "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                     VALUES (?1,?2,?3,'subj','sender',?4,?5,'[]','[]','snip',0,0,'primary',?6,0)",
                rusqlite::params![id, account, thread, sender_email, domain, mailbox],
            )
            .unwrap();
    }

    // A pinned filter that falls outside the top-N stats (here: the account
    // owner's own address, which is excluded from sender stats entirely) must
    // still get a real thread count saved — the sidebar showed 0 for it.
    #[test]
    fn refresh_saves_counts_for_pinned_filters_missing_from_stats() {
        // seed_test_account sets email = id, so the account id IS the owner address.
        let me = "me@mymail.com";
        let db = std::sync::Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account(me);

        insert_email(&db, "s1", me, "t1", me, "sent");
        insert_email(&db, "s2", me, "t2", me, "sent");
        pin_filter(&db, me, "sender", me).unwrap();

        refresh_filter_stats(&db, me).unwrap();

        let suggestions = db.get_filter_suggestions(me).unwrap();
        let pinned = suggestions
            .iter()
            .find(|s| s.filter_type == "sender" && s.filter_value.eq_ignore_ascii_case(me))
            .expect("pinned sender must have a saved suggestion even though stats exclude it");
        assert_eq!(pinned.count, 2, "count must be the pinned sender's thread count");
    }

    // A pinned filter that IS already in the stats must not get a duplicate row.
    #[test]
    fn refresh_does_not_duplicate_pinned_filters_already_in_stats() {
        let db = std::sync::Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");

        insert_email(&db, "e1", "acc", "t1", "Alice@Ex.com", "inbox");
        // Pinned under a different casing than the stored/stats value.
        pin_filter(&db, "acc", "sender", "alice@ex.com").unwrap();

        refresh_filter_stats(&db, "acc").unwrap();

        let alice: Vec<_> = db
            .get_filter_suggestions("acc")
            .unwrap()
            .into_iter()
            .filter(|s| s.filter_type == "sender" && s.filter_value.eq_ignore_ascii_case("alice@ex.com"))
            .collect();
        assert_eq!(alice.len(), 1, "one suggestion row, not a stats + pinned duplicate");
    }

    // A failing tag-stats query must surface as an error, not be silently
    // swallowed (which made every tag section quietly vanish from the sidebar).
    #[test]
    fn refresh_filter_stats_propagates_tag_stats_errors() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");
        db.connection().execute("DROP TABLE email_tags", []).unwrap();

        let result = refresh_filter_stats(&db, "acc");

        assert!(
            result.is_err(),
            "refresh must propagate the tag-stats failure instead of dropping tag suggestions"
        );
    }
}
