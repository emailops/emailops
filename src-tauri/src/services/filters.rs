use std::sync::Arc;

use crate::db::{AccountScope, Database};
use crate::models::error::Result;
use crate::models::{Account, FilteredEmailsResult, QuickFilterStats, SmartFilterPref, SmartFilterSuggestion};

/// Dedup key for saved suggestions. Sender addresses are case-insensitive
/// identifiers; every other filter value is compared verbatim.
fn suggestion_key(filter_type: &str, filter_value: &str) -> String {
    if filter_type == "sender" {
        format!("{}:{}", filter_type, filter_value.to_lowercase())
    } else {
        format!("{}:{}", filter_type, filter_value)
    }
}

fn scope_of(account_id: Option<&str>) -> AccountScope<'_> {
    match account_id {
        Some(id) => AccountScope::Account(id),
        None => AccountScope::AllEnabled,
    }
}

/// Pure planner: which accounts a filter-pref write (pin/remove/delete)
/// applies to. Single-account mode targets that account; unified mode fans
/// out to every ENABLED account so the preference holds wherever the filter's
/// emails live.
fn pref_write_targets(account_id: Option<&str>, accounts: &[Account]) -> Vec<String> {
    match account_id {
        Some(id) => vec![id.to_string()],
        None => accounts.iter().filter(|a| a.enabled).map(|a| a.id.clone()).collect(),
    }
}

/// Calculate fresh suggestions, persist them to DB, and return stats.
///
/// `account_id: None` (unified "All accounts") refreshes each enabled account
/// exactly as the per-account path would — persisting per-account suggestion
/// rows — then returns stats aggregated across all of them.
pub fn refresh_filter_stats(db: &Arc<Database>, account_id: Option<&str>) -> Result<QuickFilterStats> {
    match account_id {
        // Single-account path (unchanged behavior).
        Some(id) => refresh_account_filter_stats(db, id),
        None => {
            let accounts = db.list_accounts()?;
            for account in accounts.iter().filter(|a| a.enabled) {
                refresh_account_filter_stats(db, &account.id)?;
            }
            // Aggregated read-back: exclusions come from the deduped unified
            // prefs (pinned-beats-removed), so a filter pinned in one account
            // isn't suppressed by another account's removal.
            let prefs = db.get_filter_prefs_all_enabled()?;
            let (excluded_domains, excluded_senders) = removed_exclusions(&prefs);
            db.get_quick_filter_stats(AccountScope::AllEnabled, &excluded_domains, &excluded_senders)
        }
    }
}

fn removed_exclusions(prefs: &[SmartFilterPref]) -> (Vec<String>, Vec<String>) {
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
    (excluded_domains, excluded_senders)
}

/// Per-account refresh: compute stats, persist suggestion rows (domain/sender
/// + tag groups + pinned-filter counts) for this account.
fn refresh_account_filter_stats(db: &Arc<Database>, account_id: &str) -> Result<QuickFilterStats> {
    // Read removed prefs to exclude from suggestions
    let prefs = db.get_filter_prefs(account_id)?;
    let (excluded_domains, excluded_senders) = removed_exclusions(&prefs);

    let stats = db.get_quick_filter_stats(AccountScope::Account(account_id), &excluded_domains, &excluded_senders)?;

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
        for (value, count) in db.get_tag_stats(AccountScope::Account(account_id), tag_type, 15)? {
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
        let count = db.count_filter_threads(AccountScope::Account(account_id), &p.filter_type, &p.filter_value)?;
        to_save.push(SmartFilterSuggestion {
            filter_type: p.filter_type.clone(),
            filter_value: p.filter_value.clone(),
            count,
        });
    }

    db.save_filter_suggestions(account_id, &to_save)?;

    Ok(stats)
}

/// Load previously calculated suggestions from DB.
/// `None` aggregates across every enabled account (counts summed, sender
/// values merged case-insensitively).
pub fn get_saved_suggestions(db: &Arc<Database>, account_id: Option<&str>) -> Result<Vec<SmartFilterSuggestion>> {
    match account_id {
        Some(id) => db.get_filter_suggestions(id),
        None => db.get_filter_suggestions_all_enabled(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn get_filtered_emails(
    db: &Arc<Database>,
    account_id: Option<&str>,
    domain: Option<&str>,
    sender_email: Option<&str>,
    tag_type: Option<&str>,
    tag_value: Option<&str>,
    attachment_ext: Option<&str>,
    limit: i32,
    offset: i32,
) -> Result<FilteredEmailsResult> {
    db.get_filtered_emails(
        scope_of(account_id),
        domain,
        sender_email,
        tag_type,
        tag_value,
        attachment_ext,
        limit,
        offset,
    )
}

/// `None` returns the union of prefs across enabled accounts, deduped with
/// pinned-beats-removed precedence.
pub fn get_filter_prefs(db: &Arc<Database>, account_id: Option<&str>) -> Result<Vec<SmartFilterPref>> {
    match account_id {
        Some(id) => db.get_filter_prefs(id),
        None => db.get_filter_prefs_all_enabled(),
    }
}

pub fn pin_filter(db: &Arc<Database>, account_id: Option<&str>, filter_type: &str, filter_value: &str) -> Result<()> {
    for target in pref_write_targets(account_id, &db.list_accounts()?) {
        let id = format!("{}:{}:{}", target, filter_type, filter_value);
        db.upsert_filter_pref(&id, filter_type, filter_value, "pinned", &target)?;
    }
    Ok(())
}

pub fn remove_filter(
    db: &Arc<Database>,
    account_id: Option<&str>,
    filter_type: &str,
    filter_value: &str,
) -> Result<()> {
    for target in pref_write_targets(account_id, &db.list_accounts()?) {
        let id = format!("{}:{}:{}", target, filter_type, filter_value);
        db.upsert_filter_pref(&id, filter_type, filter_value, "removed", &target)?;
    }
    Ok(())
}

pub fn delete_filter_pref(
    db: &Arc<Database>,
    account_id: Option<&str>,
    filter_type: &str,
    filter_value: &str,
) -> Result<()> {
    for target in pref_write_targets(account_id, &db.list_accounts()?) {
        let id = format!("{}:{}:{}", target, filter_type, filter_value);
        db.delete_filter_pref(&id, &target)?;
    }
    Ok(())
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
        pin_filter(&db, Some(me), "sender", me).unwrap();

        refresh_filter_stats(&db, Some(me)).unwrap();

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
        pin_filter(&db, Some("acc"), "sender", "alice@ex.com").unwrap();

        refresh_filter_stats(&db, Some("acc")).unwrap();

        let alice: Vec<_> = db
            .get_filter_suggestions("acc")
            .unwrap()
            .into_iter()
            .filter(|s| s.filter_type == "sender" && s.filter_value.eq_ignore_ascii_case("alice@ex.com"))
            .collect();
        assert_eq!(alice.len(), 1, "one suggestion row, not a stats + pinned duplicate");
    }

    // ── unified (None) mode ──────────────────────────────────────────────────

    fn make_account(id: &str, enabled: bool) -> Account {
        Account {
            id: id.to_string(),
            provider: "gmail".to_string(),
            email: format!("{id}@example.com"),
            name: id.to_string(),
            created_at: 0,
            sort_order: 0,
            enabled,
            sync_from_timestamp: None,
        }
    }

    #[test]
    fn pref_write_targets_single_account_targets_it_regardless_of_enabled() {
        let accounts = vec![make_account("a", false), make_account("b", true)];
        assert_eq!(pref_write_targets(Some("a"), &accounts), vec!["a".to_string()]);
    }

    #[test]
    fn pref_write_targets_unified_fans_out_to_enabled_accounts_only() {
        let accounts = vec![
            make_account("a", true),
            make_account("b", false),
            make_account("c", true),
        ];
        assert_eq!(
            pref_write_targets(None, &accounts),
            vec!["a".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn pref_write_targets_unified_with_no_accounts_is_empty() {
        assert!(pref_write_targets(None, &[]).is_empty());
    }

    #[test]
    fn pin_filter_unified_fans_out_to_all_enabled_accounts() {
        let db = std::sync::Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        db.seed_test_account("acc2");
        db.seed_test_account("acc3");
        db.connection()
            .execute("UPDATE accounts SET enabled = 0 WHERE id = 'acc3'", [])
            .unwrap();

        pin_filter(&db, None, "domain", "acme.com").unwrap();

        assert_eq!(
            db.get_filter_prefs("acc1").unwrap().len(),
            1,
            "acc1 must receive the pin"
        );
        assert_eq!(
            db.get_filter_prefs("acc2").unwrap().len(),
            1,
            "acc2 must receive the pin"
        );
        assert!(
            db.get_filter_prefs("acc3").unwrap().is_empty(),
            "disabled acc3 must NOT receive the pin"
        );
    }

    // A failing tag-stats query must surface as an error, not be silently
    // swallowed (which made every tag section quietly vanish from the sidebar).
    #[test]
    fn refresh_filter_stats_propagates_tag_stats_errors() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");
        db.connection().execute("DROP TABLE email_tags", []).unwrap();

        let result = refresh_filter_stats(&db, Some("acc"));

        assert!(
            result.is_err(),
            "refresh must propagate the tag-stats failure instead of dropping tag suggestions"
        );
    }
}
