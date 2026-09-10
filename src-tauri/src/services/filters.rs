use std::sync::Arc;

use crate::db::{AccountScope, Database};
use crate::models::error::{AppError, Result};
use crate::models::{
    Account, EmailWindow, FilteredEmailsResult, QuickFilterStats, SmartFilterPref, SmartFilterSuggestion, TagStat,
    ThreadParticipants,
};

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
    window: &EmailWindow,
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
        window,
        limit,
        offset,
    )
}

/// Tag types the classifier emits, and the only values `get_tag_stats` accepts.
/// Mirrors the loop in `refresh_filter_stats` and the frontend's
/// `TAG_BOARD_TYPES`.
pub const CLASSIFIED_TAG_TYPES: &[&str] = &["company", "intent", "topic", "priority"];

/// Default and maximum number of tag values a single `get_tag_stats` call
/// returns. The board renders one column per value, so the cap is a UI budget
/// as much as a query one.
const TAG_STATS_DEFAULT_LIMIT: i32 = 15;
const TAG_STATS_MAX_LIMIT: i32 = 50;

/// Board blocks are (account × tag) pairs, so the ceiling has to be higher
/// than the per-tag one — five accounts sharing a dozen tags is ordinary.
const TAG_BOARD_DEFAULT_LIMIT: i32 = 24;
const TAG_BOARD_MAX_LIMIT: i32 = 120;

/// Live per-tag thread counts for one `tag_type`, newest stats every call —
/// unlike `get_saved_suggestions`, which serves whatever the last
/// `refresh_filter_stats` wrote. `account_id: None` aggregates across every
/// enabled account.
pub fn get_tag_stats(db: &Arc<Database>, account_id: Option<&str>, tag_type: &str, limit: i32) -> Result<Vec<TagStat>> {
    if !CLASSIFIED_TAG_TYPES.contains(&tag_type) {
        return Err(AppError::InvalidInput(format!(
            "unknown tag type '{tag_type}' (expected one of {})",
            CLASSIFIED_TAG_TYPES.join(", ")
        )));
    }
    // A non-positive limit is SQLite's "no rows", which would silently render
    // an empty board for a caller that just omitted the argument.
    let limit = if limit <= 0 {
        TAG_STATS_DEFAULT_LIMIT
    } else {
        limit.min(TAG_STATS_MAX_LIMIT)
    };

    Ok(db
        .get_tag_stats(scope_of(account_id), tag_type, limit)?
        .into_iter()
        .map(|(tag_value, count)| TagStat {
            account_id: None,
            tag_value,
            count,
            sent_share: 0.0,
            read_share: 0.0,
            last_activity_at: None,
            score: 0.0,
        })
        .collect())
}

/// Neutral rank for a tag value the vocabulary tables below don't mention —
/// a classifier vocabulary change must not silently bury new values under
/// promotional noise.
const ACTIONABILITY_DEFAULT: i32 = 2;

/// Tag values that usually mean the user owes someone a reply or a decision.
const ACTIONABILITY_HIGH: &[&str] = &[
    // intent
    "request",
    "approval",
    "question",
    "complaint",
    "scheduling",
    // topic
    "contract",
    "project",
    "billing",
    "legal",
    "hiring",
    "finance",
    "security",
    // priority
    "urgent",
];

/// Broadcast mail: it arrives in bulk and almost never needs an answer.
const ACTIONABILITY_NOISE: &[&str] = &[
    // intent
    "promotion",
    "newsletter",
    "notification",
    // topic
    "marketing",
    // priority
    "low",
];

/// How likely a tag is to need the user to *do* something, 0 (noise) to 3
/// (needs a reply or a decision). Drives the board's block order ahead of raw
/// thread count, so a three-thread `contract` block leads a 6000-thread
/// `marketing` one.
///
/// `company` is deliberately flat: its values are an open vocabulary (one per
/// organisation) with no inherent actionability, so those blocks fall through
/// to thread count.
pub fn tag_actionability(tag_type: &str, tag_value: &str) -> i32 {
    if tag_type == "company" {
        return ACTIONABILITY_DEFAULT;
    }
    let value = tag_value.trim().to_ascii_lowercase();
    if ACTIONABILITY_HIGH.contains(&value.as_str()) {
        3
    } else if ACTIONABILITY_NOISE.contains(&value.as_str()) {
        0
    } else {
        ACTIONABILITY_DEFAULT
    }
}

/// Sent share at which the reply signal saturates. Above this, more outbound
/// mail says nothing new — and it stops a two-message tag from topping the
/// board on ratio alone.
const SENT_SATURATION: f64 = 0.25;

/// Weights for the two signals. Sending is deliberate; reading is passive but
/// still separates mail the user consumes from mail they ignore.
const W_SENT: f64 = 0.65;
const W_READ: f64 = 0.35;

/// Threads of history needed before the learned signal fully replaces the
/// hand-written prior. Below it the two are weighted equally.
const CONFIDENCE_THREADS: i32 = 20;

/// How engaged the user is with a tag, 0..1 — the board's ordering key.
///
/// Combines two signals read off the tag's own messages: the share the user
/// **sent** (an explicit act) and the share they have **read** (passive, but it
/// tells a listings digest they open daily apart from a social network they
/// never touch).
///
/// `prior_tier` is [`tag_actionability`]'s hand-written guess, used only while
/// there is too little history to trust the mailbox — a tag with three threads
/// proves nothing. Past [`CONFIDENCE_THREADS`] the mailbox wins outright.
pub fn tag_engagement_score(prior_tier: i32, threads: i32, sent_share: f64, read_share: f64) -> f64 {
    let sent = (sent_share / SENT_SATURATION).clamp(0.0, 1.0);
    let read = read_share.clamp(0.0, 1.0);
    let learned = W_SENT * sent + W_READ * read;

    let prior = f64::from(prior_tier.clamp(0, 3)) / 3.0;

    // Deliberately a step, not a ramp. A proportional blend made the score fall
    // as thread count rose whenever the learned signal was flat — so two
    // equally-ignored tags sorted by ASCENDING count, and a fresh mailbox
    // (nothing read, nothing replied) came out backwards. Weighting the prior
    // the same for every under-evidenced tag keeps them tied on score, which
    // lets thread count break the tie the way round.
    if threads >= CONFIDENCE_THREADS {
        learned
    } else {
        0.5 * learned + 0.5 * prior
    }
}

/// Rank of a tag value on an ordinal scale, or 0 when the dimension has no
/// inherent order.
///
/// `priority` is the only ordinal dimension: its values *are* an importance
/// scale, so `urgent` outranks `normal` by definition and no amount of reading
/// habit should reorder them. Company, topic and intent values have no such
/// order — they are ranked entirely by engagement.
///
/// This is deliberately separate from [`tag_actionability`], which is a
/// *guess* the learned signal is allowed to overrule. An ordinal rank is a
/// fact about the vocabulary and always wins.
pub fn tag_ordinal(tag_type: &str, tag_value: &str) -> i32 {
    if tag_type != "priority" {
        return 0;
    }
    match tag_value.trim().to_ascii_lowercase().as_str() {
        "urgent" => 3,
        "normal" => 2,
        "low" => 1,
        // An unrecognised level sits between low and normal rather than on top.
        _ => 1,
    }
}

/// Thread count at which volume reaches its maximum.
const VOLUME_SATURATION: f64 = 2_000.0;

/// How much of the mailbox a tag accounts for, 0..1.
///
/// Engagement is a *ratio*, so without this a 138-thread digest read 57% of the
/// time outranked a 2,212-thread one read 44% of the time — near-identical
/// habits, one fifteen times more of the user's mail.
///
/// The curve is logarithmic, not linear, because thread counts here span 1 to
/// ~10,000. A linear factor generous enough to separate 138 from 2,212 also let
/// **one-thread** tags sit near the top of the board; one that crushed those
/// flattened everything above a few hundred into a tie. Log scaling does both:
/// it drops a singleton to ~0.09 while still separating the hundreds from the
/// thousands.
pub fn tag_volume_factor(threads: i32) -> f64 {
    let scale = (1.0 + VOLUME_SATURATION).ln();
    (f64::from(threads.max(0)) + 1.0).ln().min(scale) / scale
}

/// Floor on the recency factor. A dormant tag sinks; it never disappears.
const RECENCY_FLOOR: f64 = 0.05;

/// Days after which the non-floor part of the recency factor halves.
const RECENCY_HALF_LIFE_DAYS: f64 = 45.0;

/// How live a tag is, [`RECENCY_FLOOR`]..1, from the newest message carrying it.
///
/// Without this the board led with companies last heard from years ago: every
/// message read, every one answered, so engagement scored a perfect 1.0 forever.
/// Engagement says a tag *mattered*; recency says whether it still does.
///
/// A future timestamp — clock skew in a provider's headers — is treated as now
/// rather than scoring above 1.0.
pub fn tag_recency_factor(now_ts: i64, last_activity_ts: i64) -> f64 {
    let age_days = ((now_ts - last_activity_ts).max(0) as f64) / 86_400.0;
    RECENCY_FLOOR + (1.0 - RECENCY_FLOOR) / (1.0 + age_days / RECENCY_HALF_LIFE_DAYS)
}

/// Per-`(account, tag_value)` thread counts for the tag board.
///
/// One row per block the board will render, ordered by thread count so the
/// busiest blocks come first, then capped. `account_id: None` spans every
/// enabled account — which is the point: the board splits a shared tag into
/// one block per mailbox rather than merging them.
pub fn get_tag_board_stats(
    db: &Arc<Database>,
    account_id: Option<&str>,
    tag_type: &str,
    window: &EmailWindow,
    limit: i32,
) -> Result<Vec<TagStat>> {
    if !CLASSIFIED_TAG_TYPES.contains(&tag_type) {
        return Err(AppError::InvalidInput(format!(
            "unknown tag type '{tag_type}' (expected one of {})",
            CLASSIFIED_TAG_TYPES.join(", ")
        )));
    }
    let limit = if limit <= 0 {
        TAG_BOARD_DEFAULT_LIMIT
    } else {
        limit.min(TAG_BOARD_MAX_LIMIT)
    };

    // Over-fetch, then rank by actionability before truncating: ordering by
    // count alone in SQL would drop a small high-signal block (a three-thread
    // `contract`) off the end in favour of bulk mail.
    // One wall-clock read for the whole ranking, so every block — and every
    // message inside it — decays against the same instant.
    let now_ts = chrono::Utc::now().timestamp();
    let candidates = db.get_tag_board_stats(scope_of(account_id), tag_type, window, now_ts, limit.saturating_mul(4))?;

    let mut stats: Vec<TagStat> = candidates
        .into_iter()
        .map(|row| {
            let engagement = tag_engagement_score(
                tag_actionability(tag_type, &row.tag_value),
                row.count,
                row.sent_share,
                row.read_share,
            );
            // Engagement says the tag mattered; recency says whether it still
            // does. A dormant tag keeps its rank order but sinks below live mail.
            let recency = row
                .last_activity_at
                .map_or(RECENCY_FLOOR, |ts| tag_recency_factor(now_ts, ts));
            // Volume separates two tags with similar habits by how much of the
            // mailbox each one actually is.
            let volume = tag_volume_factor(row.count);
            TagStat {
                account_id: Some(row.account_id),
                tag_value: row.tag_value,
                count: row.count,
                sent_share: row.sent_share,
                read_share: row.read_share,
                last_activity_at: row.last_activity_at,
                score: engagement * recency * volume,
            }
        })
        .collect();

    stats.sort_by(|a, b| {
        // An ordinal dimension (priority) groups by level first; engagement
        // only orders blocks *within* a level.
        tag_ordinal(tag_type, &b.tag_value)
            .cmp(&tag_ordinal(tag_type, &a.tag_value))
            .then_with(|| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then(b.count.cmp(&a.count))
            // Stable tiebreak so the order doesn't wobble between refreshes.
            .then(a.tag_value.cmp(&b.tag_value))
    });
    stats.truncate(limit as usize);
    Ok(stats)
}

/// Threads read per call. Sync batches are far smaller, but a caller looping a
/// large board shouldn't be able to blow SQLite's 32,766-parameter ceiling.
const PARTICIPANT_THREAD_CHUNK: usize = 200;

/// Everyone other than the account owner taking part in each thread.
///
/// The board lists one representative message per thread, so the card alone
/// can't say who else is in the conversation. Batched by thread id — a page of
/// eight cards is one query, not eight.
///
/// A person is listed once, keyed on their address (case-insensitively),
/// preferring a display name over a bare address, ordered by most recent
/// activity so whoever is currently talking comes first.
pub fn get_thread_participants(
    db: &Arc<Database>,
    account_id: &str,
    thread_ids: &[String],
) -> Result<Vec<ThreadParticipants>> {
    if thread_ids.is_empty() {
        return Ok(Vec::new());
    }

    let owner = db
        .get_account(account_id)?
        .map(|a| a.email.trim().to_ascii_lowercase())
        .unwrap_or_default();

    let mut per_thread: std::collections::HashMap<String, Vec<(String, String)>> = std::collections::HashMap::new();
    for chunk in thread_ids.chunks(PARTICIPANT_THREAD_CHUNK) {
        db.read_thread_people(account_id, chunk, &mut per_thread)?;
    }

    Ok(thread_ids
        .iter()
        .filter_map(|tid| {
            let people = per_thread.get(tid)?;
            // Rows arrive newest-first; first sighting of an address wins its
            // position, and a later display name upgrades a bare address.
            let mut order: Vec<String> = Vec::new();
            let mut best: std::collections::HashMap<String, String> = std::collections::HashMap::new();

            for (name, address) in people {
                let key = address.trim().to_ascii_lowercase();
                if key.is_empty() || key == owner {
                    continue;
                }
                let display = if name.trim().is_empty() {
                    address.trim().to_string()
                } else {
                    name.trim().to_string()
                };
                match best.get(&key) {
                    None => {
                        order.push(key.clone());
                        best.insert(key, display);
                    }
                    Some(existing) if existing.contains('@') && !display.contains('@') => {
                        best.insert(key, display);
                    }
                    Some(_) => {}
                }
            }

            Some(ThreadParticipants {
                thread_id: tid.clone(),
                names: order.into_iter().filter_map(|k| best.remove(&k)).collect(),
            })
        })
        .collect())
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

    fn insert_email_tagged(
        db: &Database,
        id: &str,
        account: &str,
        thread: &str,
        mailbox: &str,
        tag_type: &str,
        tag_value: &str,
    ) {
        insert_email_tagged_at(db, id, account, thread, mailbox, tag_type, tag_value, "primary", 0);
    }

    fn insert_email_priority(db: &Database, id: &str, account: &str, thread: &str, level: &str, is_read: bool) {
        insert_email_tagged(db, id, account, thread, "inbox", "priority", level);
        db.connection()
            .execute(
                "UPDATE emails SET is_read = ?2, timestamp = ?3 WHERE id = ?1",
                rusqlite::params![id, is_read as i32, chrono::Utc::now().timestamp()],
            )
            .unwrap();
    }

    fn set_timestamp(db: &Database, id: &str, ts: i64) {
        db.connection()
            .execute(
                "UPDATE emails SET timestamp = ?2 WHERE id = ?1",
                rusqlite::params![id, ts],
            )
            .unwrap();
    }

    /// Tag an email and set the two engagement signals on it.
    fn insert_email_engaged(
        db: &Database,
        id: &str,
        account: &str,
        thread: &str,
        tag_value: &str,
        is_read: bool,
        is_sent: bool,
    ) {
        insert_email_tagged(db, id, account, thread, "inbox", "company", tag_value);
        db.connection()
            .execute(
                "UPDATE emails SET is_read = ?2, is_sent = ?3 WHERE id = ?1",
                rusqlite::params![id, is_read as i32, is_sent as i32],
            )
            .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_email_tagged_at(
        db: &Database,
        id: &str,
        account: &str,
        thread: &str,
        mailbox: &str,
        tag_type: &str,
        tag_value: &str,
        category: &str,
        timestamp: i64,
    ) {
        insert_email(db, id, account, thread, "someone@example.com", mailbox);
        db.connection()
            .execute(
                "UPDATE emails SET category = ?2, timestamp = ?3 WHERE id = ?1",
                rusqlite::params![id, category, timestamp],
            )
            .unwrap();
        db.connection()
            .execute(
                "INSERT INTO email_tags (email_id, tag_type, tag_value, confidence, created_at)
                 VALUES (?1, ?2, ?3, NULL, 0)",
                rusqlite::params![id, tag_type, tag_value],
            )
            .unwrap();
    }

    // ── get_tag_stats ────────────────────────────────────────────────────────

    #[test]
    fn get_tag_stats_returns_thread_counts_ordered_by_count_desc() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");

        insert_email_tagged(&db, "e1", "acc", "t1", "inbox", "company", "globex");
        insert_email_tagged(&db, "e2", "acc", "t2", "inbox", "company", "globex");
        insert_email_tagged(&db, "e3", "acc", "t3", "inbox", "company", "initech");

        let stats = get_tag_stats(&db, Some("acc"), "company", 10).unwrap();

        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].tag_value, "globex");
        assert_eq!(stats[0].count, 2);
        assert_eq!(stats[1].tag_value, "initech");
        assert_eq!(stats[1].count, 1);
    }

    #[test]
    fn get_tag_stats_scopes_to_the_requested_tag_type() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");

        insert_email_tagged(&db, "e1", "acc", "t1", "inbox", "company", "globex");
        insert_email_tagged(&db, "e2", "acc", "t2", "inbox", "topic", "billing");

        let topics = get_tag_stats(&db, Some("acc"), "topic", 10).unwrap();
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].tag_value, "billing");
    }

    #[test]
    fn get_tag_stats_rejects_an_unknown_tag_type() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");

        // Guards the board's tag-type selector against arbitrary values from
        // the frontend — an unrecognised type is a bug, not an empty board.
        let err = get_tag_stats(&db, Some("acc"), "not-a-tag-type", 10).unwrap_err();
        assert!(matches!(err, crate::models::error::AppError::InvalidInput(_)));
    }

    #[test]
    fn get_tag_stats_clamps_the_limit_to_a_sane_range() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");

        insert_email_tagged(&db, "e1", "acc", "t1", "inbox", "company", "globex");
        insert_email_tagged(&db, "e2", "acc", "t2", "inbox", "company", "initech");

        // A non-positive limit would otherwise mean "no rows" in SQLite, which
        // would render an empty board for a caller that just forgot the arg.
        assert_eq!(get_tag_stats(&db, Some("acc"), "company", 0).unwrap().len(), 2);
        assert_eq!(get_tag_stats(&db, Some("acc"), "company", -5).unwrap().len(), 2);
    }

    #[test]
    fn get_tag_stats_unified_scope_aggregates_enabled_accounts() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        db.seed_test_account("acc2");

        insert_email_tagged(&db, "e1", "acc1", "t1", "inbox", "company", "globex");
        insert_email_tagged(&db, "e2", "acc2", "t2", "inbox", "company", "globex");

        let stats = get_tag_stats(&db, None, "company", 10).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].count, 2, "both accounts' threads count toward the tag");
    }

    // ── tag_engagement_score ─────────────────────────────────────────────────

    /// Score with full confidence (enough threads that the prior is ignored).
    fn score(sent: f64, read: f64) -> f64 {
        tag_engagement_score(ACTIONABILITY_DEFAULT, 500, sent, read)
    }

    #[test]
    fn engagement_rewards_replying_more_than_reading() {
        // Both signals count, but sending is an explicit act and reading is
        // passive, so a modest reply rate must beat a high read rate.
        assert!(score(0.20, 0.10) > score(0.0, 0.90));
    }

    #[test]
    fn engagement_credits_a_tag_you_always_read_but_never_answer() {
        // The listings site you read every day and never reply to still beats
        // the social network you never open.
        let always_read = score(0.0, 0.50);
        let never_opened = score(0.0, 0.02);
        assert!(always_read > never_opened);
    }

    #[test]
    fn engagement_is_zero_for_mail_that_is_neither_read_nor_answered() {
        assert_eq!(score(0.0, 0.0), 0.0);
    }

    #[test]
    fn engagement_rises_with_each_signal_independently() {
        assert!(score(0.10, 0.30) > score(0.05, 0.30));
        assert!(score(0.10, 0.60) > score(0.10, 0.30));
    }

    #[test]
    fn engagement_saturates_a_very_high_reply_share() {
        // Beyond the saturation point extra sent share adds nothing, so a
        // two-message tag can't outrank a real correspondence on ratio alone.
        assert_eq!(score(0.60, 0.5), score(0.90, 0.5));
    }

    #[test]
    fn engagement_stays_within_zero_and_one() {
        for (sent, read) in [(0.0, 0.0), (1.0, 1.0), (0.5, 0.5), (0.0, 1.0), (1.0, 0.0)] {
            let s = score(sent, read);
            assert!((0.0..=1.0).contains(&s), "score {s} out of range for {sent}/{read}");
        }
    }

    #[test]
    fn engagement_leans_on_the_prior_when_there_is_little_history() {
        // One thread proves nothing; the hand-written tier should still lead.
        let noisy_prior = tag_engagement_score(0, 1, 0.9, 0.9);
        let action_prior = tag_engagement_score(3, 1, 0.9, 0.9);
        assert!(action_prior > noisy_prior);
    }

    #[test]
    fn engagement_ignores_the_prior_once_there_is_enough_history() {
        // With a real history the mailbox wins over the shipped guess — the
        // point of learning it at all.
        let a = tag_engagement_score(0, 500, 0.25, 0.9);
        let b = tag_engagement_score(3, 500, 0.25, 0.9);
        assert!((a - b).abs() < 1e-9);
    }

    #[test]
    fn board_stats_ranks_a_read_but_unanswered_tag_above_ignored_bulk() {
        // Regression from a real mailbox: a listings digest (never replied
        // to, about half read) must outrank a social-notification feed
        // (never replied to, never read), even with fewer threads.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        for i in 0..6 {
            // Read half of the listings mail.
            insert_email_engaged(
                &db,
                &format!("i{i}"),
                "acc1",
                &format!("ti{i}"),
                "listingsdaily",
                i % 2 == 0,
                false,
            );
        }
        for i in 0..10 {
            insert_email_engaged(
                &db,
                &format!("w{i}"),
                "acc1",
                &format!("tw{i}"),
                "socialpings",
                false,
                false,
            );
        }

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats[0].tag_value, "listingsdaily");
        assert_eq!(stats[1].tag_value, "socialpings");
    }

    #[test]
    fn board_stats_ranks_a_conversation_above_a_tag_you_only_read() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        for i in 0..6 {
            // Mail I actually answered.
            insert_email_engaged(
                &db,
                &format!("c{i}"),
                "acc1",
                &format!("tc{i}"),
                "client",
                true,
                i % 3 == 0,
            );
        }
        for i in 0..6 {
            insert_email_engaged(&db, &format!("n{i}"), "acc1", &format!("tn{i}"), "news", true, false);
        }

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats[0].tag_value, "client");
    }

    #[test]
    fn board_stats_exposes_the_signals_behind_the_rank() {
        // The UI explains the ordering, so the shares travel with the row.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_engaged(&db, "a", "acc1", "t1", "acme", true, false);
        insert_email_engaged(&db, "b", "acc1", "t2", "acme", true, true);

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats[0].read_share, 1.0);
        assert_eq!(stats[0].sent_share, 0.5);
    }

    // ── junk ─────────────────────────────────────────────────────────────────

    fn mark_junk(db: &Database, email_id: &str, kind: &str, band: &str, override_: Option<&str>) {
        db.connection()
            .execute(
                "INSERT INTO email_junk
                 (email_id, account_id, spam_score, phish_score, gray_score, band, primary_kind,
                  reasons_json, method, model_version, scored_at, user_override)
                 VALUES (?1, 'acc1', 0.9, 0.0, 0.0, ?2, ?3, '[]', 'deterministic', 1, 0, ?4)",
                rusqlite::params![email_id, band, kind, override_],
            )
            .unwrap();
    }

    #[test]
    fn board_stats_exclude_spam_and_phishing() {
        // Mail the junk detector called spam or phishing has no business on a
        // board meant to show what deserves attention.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "ok", "acc1", "t1", "inbox", "company", "acme");
        insert_email_tagged(&db, "spam", "acc1", "t2", "inbox", "company", "acme");
        mark_junk(&db, "spam", "spam", "junk", None);
        insert_email_tagged(&db, "phish", "acc1", "t3", "inbox", "company", "acme");
        mark_junk(&db, "phish", "phishing", "junk", None);

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats[0].count, 1, "only the clean thread counts");
    }

    #[test]
    fn board_stats_keep_graymail() {
        // Graymail is newsletters and receipts — bulk, but the user's own mail,
        // and most of what a company block legitimately holds. Dropping it
        // would gut the board.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "gray", "acc1", "t1", "inbox", "company", "acme");
        mark_junk(&db, "gray", "graymail", "junk", None);

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats.len(), 1);
    }

    #[test]
    fn board_stats_hide_graymail_when_asked() {
        // The "Hide junk" toggle. Spam and phishing are always gone; graymail
        // is the user's call, because it is most of what a company block holds.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "ok", "acc1", "t1", "inbox", "company", "acme");
        insert_email_tagged(&db, "gray", "acc1", "t2", "inbox", "company", "acme");
        mark_junk(&db, "gray", "graymail", "junk", None);

        let window = EmailWindow {
            hide_graymail: true,
            ..Default::default()
        };
        let stats = get_tag_board_stats(&db, None, "company", &window, 10).unwrap();
        assert_eq!(stats[0].count, 1);
    }

    #[test]
    fn hiding_graymail_still_respects_a_not_junk_override() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "rescued", "acc1", "t1", "inbox", "company", "acme");
        mark_junk(&db, "rescued", "graymail", "junk", Some("not_junk"));

        let window = EmailWindow {
            hide_graymail: true,
            ..Default::default()
        };
        assert_eq!(get_tag_board_stats(&db, None, "company", &window, 10).unwrap().len(), 1);
    }

    #[test]
    fn filtered_emails_hide_graymail_when_asked() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "ok", "acc1", "t1", "inbox", "company", "acme");
        insert_email_tagged(&db, "gray", "acc1", "t2", "inbox", "company", "acme");
        mark_junk(&db, "gray", "graymail", "junk", None);

        let window = EmailWindow {
            hide_graymail: true,
            ..Default::default()
        };
        let got = get_filtered_emails(
            &db,
            Some("acc1"),
            None,
            None,
            Some("company"),
            Some("acme"),
            None,
            &window,
            50,
            0,
        )
        .unwrap();
        assert_eq!(got.emails.len(), 1);
        assert_eq!(got.emails[0].id, "ok");
    }

    #[test]
    fn board_stats_respect_a_not_junk_override() {
        // The user overruled the detector; the board must not overrule them.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "rescued", "acc1", "t1", "inbox", "company", "acme");
        mark_junk(&db, "rescued", "spam", "junk", Some("not_junk"));

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats.len(), 1);
    }

    #[test]
    fn board_stats_keep_mail_the_detector_was_unsure_about() {
        // Only the `junk` band is excluded. `uncertain` and `unknown` stay —
        // suppressing those would hide real mail on a maybe.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "maybe", "acc1", "t1", "inbox", "company", "acme");
        mark_junk(&db, "maybe", "legit", "uncertain", None);

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats.len(), 1);
    }

    #[test]
    fn filtered_emails_exclude_spam_and_phishing() {
        // The block's list has to agree with the count in its header.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "ok", "acc1", "t1", "inbox", "company", "acme");
        insert_email_tagged(&db, "spam", "acc1", "t2", "inbox", "company", "acme");
        mark_junk(&db, "spam", "spam", "junk", None);

        let got = get_filtered_emails(
            &db,
            Some("acc1"),
            None,
            None,
            Some("company"),
            Some("acme"),
            None,
            &EmailWindow::default(),
            50,
            0,
        )
        .unwrap();
        assert_eq!(got.emails.len(), 1);
        assert_eq!(got.emails[0].id, "ok");
    }

    // ── ordinal dimensions ───────────────────────────────────────────────────

    #[test]
    fn priority_is_an_ordinal_scale() {
        assert!(tag_ordinal("priority", "urgent") > tag_ordinal("priority", "normal"));
        assert!(tag_ordinal("priority", "normal") > tag_ordinal("priority", "low"));
    }

    #[test]
    fn only_priority_is_ordinal() {
        // Company, topic and intent values have no inherent order — they are
        // ranked by how the user engages with them.
        assert_eq!(tag_ordinal("company", "globex"), tag_ordinal("company", "acme"));
        assert_eq!(tag_ordinal("topic", "contract"), tag_ordinal("topic", "marketing"));
        assert_eq!(tag_ordinal("intent", "request"), tag_ordinal("intent", "promotion"));
    }

    #[test]
    fn board_stats_puts_every_urgent_block_above_every_normal_one() {
        // Reported case: with all accounts on the Priority board, a 425-thread
        // `urgent` block sat below 10-thread `normal` ones because engagement
        // had replaced the prior entirely above the confidence threshold.
        // `urgent` outranks `normal` by definition, not by reading habits.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        db.seed_test_account("acc2");

        // A big, well-read `normal` block, and a small ignored `urgent` one.
        for i in 0..40 {
            insert_email_priority(&db, &format!("n{i}"), "acc1", &format!("tn{i}"), "normal", true);
        }
        for i in 0..3 {
            insert_email_priority(&db, &format!("u{i}"), "acc2", &format!("tu{i}"), "urgent", false);
        }

        let stats = get_tag_board_stats(&db, None, "priority", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats[0].tag_value, "urgent");
        assert_eq!(stats[1].tag_value, "normal");
    }

    #[test]
    fn board_stats_groups_all_accounts_by_priority_level() {
        // Blocks are per (account, tag), so each level appears once per
        // account; the levels must not interleave.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        db.seed_test_account("acc2");
        for (i, (acct, level)) in [
            ("acc1", "urgent"),
            ("acc1", "low"),
            ("acc2", "urgent"),
            ("acc2", "low"),
            ("acc1", "normal"),
            ("acc2", "normal"),
        ]
        .iter()
        .enumerate()
        {
            insert_email_priority(&db, &format!("e{i}"), acct, &format!("t{i}"), level, true);
        }

        let stats = get_tag_board_stats(&db, None, "priority", &EmailWindow::default(), 10).unwrap();
        let levels: Vec<&str> = stats.iter().map(|s| s.tag_value.as_str()).collect();
        assert_eq!(levels, vec!["urgent", "urgent", "normal", "normal", "low", "low"]);
    }

    #[test]
    fn board_stats_still_ranks_within_a_priority_level_by_engagement() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        db.seed_test_account("acc2");
        // Same level, different engagement: read beats ignored.
        for i in 0..5 {
            insert_email_priority(&db, &format!("r{i}"), "acc1", &format!("tr{i}"), "normal", true);
            insert_email_priority(&db, &format!("i{i}"), "acc2", &format!("ti{i}"), "normal", false);
        }

        let stats = get_tag_board_stats(&db, None, "priority", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats[0].account_id.as_deref(), Some("acc1"));
    }

    // ── volume ───────────────────────────────────────────────────────────────

    #[test]
    fn volume_lifts_a_tag_that_is_more_of_the_mailbox() {
        assert!(tag_volume_factor(2000) > tag_volume_factor(140));
    }

    #[test]
    fn volume_saturates_so_bulk_cannot_buy_the_top_of_the_board() {
        // Past the cap, more mail says nothing: a 10k-thread tag you ignore
        // must not climb over one you answer.
        assert_eq!(tag_volume_factor(2_000), tag_volume_factor(10_000));
    }

    #[test]
    fn volume_drops_a_one_thread_tag_out_of_contention() {
        // A linear factor left single-thread tags in the top ten, because the
        // cold-start prior alone put them mid-table.
        assert!(tag_volume_factor(1) < 0.15);
    }

    #[test]
    fn volume_still_separates_hundreds_from_thousands() {
        // The other half of the same trade-off: a curve steep enough to bury
        // singletons must not flatten everything above a few hundred into a tie.
        assert!(tag_volume_factor(2_212) > tag_volume_factor(138) * 1.4);
    }

    #[test]
    fn volume_breaks_a_tie_between_identical_habits() {
        // What volume is actually for: same engagement profile, different
        // footprint — the bigger slice of the mailbox wins.
        let big = tag_engagement_score(2, 2212, 0.02, 0.5) * tag_volume_factor(2212);
        let small = tag_engagement_score(2, 138, 0.02, 0.5) * tag_volume_factor(138);
        assert!(big > small);
    }

    #[test]
    fn volume_keeps_a_floor_so_a_small_real_correspondence_still_ranks() {
        // A five-thread client you reply to every time is worth more than a
        // huge digest you skim — volume adjusts the ranking, it doesn't set it.
        let tiny_engaged = tag_engagement_score(2, 500, 0.30, 1.0) * tag_volume_factor(5);
        let huge_skimmed = tag_engagement_score(2, 500, 0.0, 0.44) * tag_volume_factor(10_000);
        assert!(tiny_engaged > huge_skimmed);
    }

    #[test]
    fn volume_closes_the_gap_on_the_reported_pair() {
        // The reported case: a 138-thread digest read 57% of the time sat well
        // above a 2,212-thread one read 44% of the time. The small one keeps a
        // real edge — it is read more AND occasionally replied to — but volume
        // has to bring them close enough that footprint is visibly counted.
        let eng_small = tag_engagement_score(2, 138, 0.026, 0.571);
        let eng_large = tag_engagement_score(2, 2212, 0.0, 0.442);
        let before = eng_large / eng_small;
        let after = (eng_large * tag_volume_factor(2212)) / (eng_small * tag_volume_factor(138));
        assert!(after > before * 1.4, "volume should close the gap: {before} → {after}");
    }

    #[test]
    fn volume_still_ranks_real_correspondence_above_any_digest() {
        // Guard against volume swamping engagement: a real correspondence
        // (36.7% sent, 96.9% read, 546 threads) must stay above a
        // 9,803-thread feed.
        let client = tag_engagement_score(2, 546, 0.367, 0.969) * tag_volume_factor(546);
        let feed = tag_engagement_score(2, 9803, 0.0, 0.172) * tag_volume_factor(9803);
        assert!(client > feed);
    }

    #[test]
    fn volume_is_bounded() {
        for threads in [0, 1, 100, 10_000, i32::MAX] {
            let v = tag_volume_factor(threads);
            assert!((0.0..=1.0).contains(&v), "volume {v} out of range for {threads}");
        }
    }

    // ── interaction weighting ────────────────────────────────────────────────

    #[test]
    fn board_stats_weights_recent_replies_above_old_ones() {
        // Two tags, identical raw reply counts. One was answered lately and
        // ignored back then; the other the reverse. Both have the same newest
        // message, so the block-level recency multiplier is equal and this
        // isolates the per-interaction weighting.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        let now = chrono::Utc::now().timestamp();
        let old = now - 3 * 365 * 86_400;
        let recent = now - 3 * 86_400;

        for i in 0..5 {
            // "warming": ignored years ago, answered lately.
            insert_email_engaged(
                &db,
                &format!("wa{i}"),
                "acc1",
                &format!("twa{i}"),
                "warming",
                false,
                false,
            );
            set_timestamp(&db, &format!("wa{i}"), old);
            insert_email_engaged(
                &db,
                &format!("wb{i}"),
                "acc1",
                &format!("twb{i}"),
                "warming",
                true,
                true,
            );
            set_timestamp(&db, &format!("wb{i}"), recent);

            // "cooling": answered years ago, ignored lately.
            insert_email_engaged(
                &db,
                &format!("ca{i}"),
                "acc1",
                &format!("tca{i}"),
                "cooling",
                true,
                true,
            );
            set_timestamp(&db, &format!("ca{i}"), old);
            insert_email_engaged(
                &db,
                &format!("cb{i}"),
                "acc1",
                &format!("tcb{i}"),
                "cooling",
                false,
                false,
            );
            set_timestamp(&db, &format!("cb{i}"), recent);
        }

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        let warming = stats.iter().find(|s| s.tag_value == "warming").unwrap();
        let cooling = stats.iter().find(|s| s.tag_value == "cooling").unwrap();

        assert!(
            warming.sent_share > cooling.sent_share,
            "recent replies ({}) should outweigh old ones ({})",
            warming.sent_share,
            cooling.sent_share
        );
        assert!(warming.read_share > cooling.read_share);
        assert_eq!(stats[0].tag_value, "warming");
    }

    #[test]
    fn board_stats_weighting_leaves_a_uniform_history_unchanged() {
        // When every message is the same age the weights cancel, so the shares
        // are still the plain proportions.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        let ts = chrono::Utc::now().timestamp() - 10 * 86_400;
        for i in 0..4 {
            insert_email_engaged(&db, &format!("e{i}"), "acc1", &format!("t{i}"), "acme", true, i < 2);
            set_timestamp(&db, &format!("e{i}"), ts);
        }

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert!((stats[0].sent_share - 0.5).abs() < 1e-6);
        assert!((stats[0].read_share - 1.0).abs() < 1e-6);
    }

    // ── recency ──────────────────────────────────────────────────────────────

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_800_000_000;

    #[test]
    fn recency_is_full_for_mail_that_arrived_today() {
        assert!((tag_recency_factor(NOW, NOW) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn recency_falls_off_as_the_tag_goes_quiet() {
        let fresh = tag_recency_factor(NOW, NOW - DAY);
        let month = tag_recency_factor(NOW, NOW - 30 * DAY);
        let year = tag_recency_factor(NOW, NOW - 365 * DAY);
        let ancient = tag_recency_factor(NOW, NOW - 5 * 365 * DAY);
        assert!(fresh > month && month > year && year > ancient);
    }

    #[test]
    fn recency_keeps_a_floor_so_a_dormant_tag_is_ranked_not_erased() {
        // Old correspondence should sink, not vanish — the board still lists it
        // below live traffic.
        let ancient = tag_recency_factor(NOW, NOW - 20 * 365 * DAY);
        assert!(ancient > 0.0, "a dormant tag must keep a non-zero factor");
        assert!(ancient < 0.15);
    }

    #[test]
    fn recency_treats_a_future_timestamp_as_now() {
        // Clock skew on a provider's headers must not score above 1.0.
        assert!((tag_recency_factor(NOW, NOW + 10 * DAY) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn recency_sinks_a_years_old_correspondence_below_live_bulk() {
        // The reported bug: companies worked with years ago — every message
        // read and replied to — sat above everything current.
        let old_client = tag_engagement_score(2, 200, 0.30, 1.0) * tag_recency_factor(NOW, NOW - 5 * 365 * DAY);
        let live_newsletter = tag_engagement_score(2, 200, 0.0, 0.30) * tag_recency_factor(NOW, NOW - 2 * DAY);
        assert!(
            live_newsletter > old_client,
            "live bulk {live_newsletter} should outrank a dormant client {old_client}"
        );
    }

    #[test]
    fn recency_keeps_a_live_correspondence_on_top_of_live_bulk() {
        // Decay must not flatten the engagement signal for anything current.
        let live_client = tag_engagement_score(2, 200, 0.30, 1.0) * tag_recency_factor(NOW, NOW - 2 * DAY);
        let live_newsletter = tag_engagement_score(2, 200, 0.0, 0.30) * tag_recency_factor(NOW, NOW - 2 * DAY);
        assert!(live_client > live_newsletter);
    }

    #[test]
    fn board_stats_ranks_a_live_tag_above_a_dormant_better_engaged_one() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        let now = chrono::Utc::now().timestamp();
        // Dormant but perfectly engaged: read and answered, five years ago.
        for i in 0..8 {
            insert_email_engaged(
                &db,
                &format!("o{i}"),
                "acc1",
                &format!("to{i}"),
                "oldclient",
                true,
                i % 2 == 0,
            );
            set_timestamp(&db, &format!("o{i}"), now - 5 * 365 * 86_400);
        }
        // Live but barely engaged.
        for i in 0..8 {
            insert_email_engaged(
                &db,
                &format!("n{i}"),
                "acc1",
                &format!("tn{i}"),
                "livenews",
                i == 0,
                false,
            );
            set_timestamp(&db, &format!("n{i}"), now - 2 * 86_400);
        }

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats[0].tag_value, "livenews");
        assert_eq!(stats[1].tag_value, "oldclient");
    }

    #[test]
    fn board_stats_reports_last_activity_for_each_block() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_engaged(&db, "a", "acc1", "t1", "acme", true, false);
        set_timestamp(&db, "a", 1_700_000_000);
        insert_email_engaged(&db, "b", "acc1", "t2", "acme", true, false);
        set_timestamp(&db, "b", 1_700_000_500);

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats[0].last_activity_at, Some(1_700_000_500));
    }

    // ── search ───────────────────────────────────────────────────────────────

    #[test]
    fn board_stats_search_matches_anywhere_in_the_tag_value() {
        // 2,500+ company values can't fit a grid, so search has to reach the
        // ones ranking would never surface.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "e1", "acc1", "t1", "inbox", "company", "globex-industries");
        insert_email_tagged(&db, "e2", "acc1", "t2", "inbox", "company", "initech");

        let window = EmailWindow {
            search: Some("dustr".into()),
            ..Default::default()
        };
        let stats = get_tag_board_stats(&db, None, "company", &window, 10).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].tag_value, "globex-industries");
    }

    #[test]
    fn board_stats_search_is_case_insensitive() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "e1", "acc1", "t1", "inbox", "company", "Globex");

        let window = EmailWindow {
            search: Some("GLOB".into()),
            ..Default::default()
        };
        assert_eq!(get_tag_board_stats(&db, None, "company", &window, 10).unwrap().len(), 1);
    }

    #[test]
    fn board_stats_search_escapes_wildcards_so_they_match_literally() {
        // A user typing "%" is searching for a percent sign, not for anything.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "e1", "acc1", "t1", "inbox", "company", "acme");
        insert_email_tagged(&db, "e2", "acc1", "t2", "inbox", "company", "50%-off");

        let window = EmailWindow {
            search: Some("%".into()),
            ..Default::default()
        };
        let stats = get_tag_board_stats(&db, None, "company", &window, 10).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].tag_value, "50%-off");
    }

    #[test]
    fn board_stats_blank_search_is_no_filter() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "e1", "acc1", "t1", "inbox", "company", "acme");
        insert_email_tagged(&db, "e2", "acc1", "t2", "inbox", "company", "initech");

        let window = EmailWindow {
            search: Some("   ".into()),
            ..Default::default()
        };
        assert_eq!(get_tag_board_stats(&db, None, "company", &window, 10).unwrap().len(), 2);
    }

    // ── tag_actionability ────────────────────────────────────────────────────

    #[test]
    fn actionability_ranks_work_topics_above_noise() {
        // The board should lead with tags that imply the user owes someone
        // something, not with the bulk of the mailbox.
        assert!(tag_actionability("topic", "contract") > tag_actionability("topic", "marketing"));
        assert!(tag_actionability("topic", "project") > tag_actionability("topic", "marketing"));
        assert!(tag_actionability("intent", "request") > tag_actionability("intent", "promotion"));
        assert!(tag_actionability("intent", "approval") > tag_actionability("intent", "newsletter"));
    }

    #[test]
    fn actionability_puts_broadcast_intents_at_the_bottom() {
        assert_eq!(tag_actionability("intent", "promotion"), 0);
        assert_eq!(tag_actionability("intent", "newsletter"), 0);
        assert_eq!(tag_actionability("topic", "marketing"), 0);
    }

    #[test]
    fn actionability_ranks_priority_by_urgency() {
        assert!(tag_actionability("priority", "urgent") > tag_actionability("priority", "normal"));
        assert!(tag_actionability("priority", "normal") > tag_actionability("priority", "low"));
    }

    #[test]
    fn actionability_is_flat_for_company_tags() {
        // Company values are an open vocabulary — no vocabulary-based signal
        // exists, so they must all tie and fall through to thread count.
        assert_eq!(
            tag_actionability("company", "globex"),
            tag_actionability("company", "initech")
        );
    }

    #[test]
    fn actionability_gives_unknown_values_a_neutral_middle_rank() {
        // A classifier vocabulary change must not bury new values under noise.
        let unknown = tag_actionability("topic", "some-new-topic");
        assert!(unknown > tag_actionability("topic", "marketing"));
        assert!(unknown < tag_actionability("topic", "contract"));
    }

    #[test]
    fn board_stats_orders_actionable_tags_first_despite_lower_counts() {
        // The whole point: a small "contract" block outranks a huge
        // "marketing" one.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        for i in 0..5 {
            insert_email_tagged(
                &db,
                &format!("m{i}"),
                "acc1",
                &format!("tm{i}"),
                "inbox",
                "topic",
                "marketing",
            );
        }
        insert_email_tagged(&db, "c1", "acc1", "tc1", "inbox", "topic", "contract");

        let stats = get_tag_board_stats(&db, None, "topic", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats[0].tag_value, "contract");
        assert_eq!(stats[0].count, 1);
        assert_eq!(stats[1].tag_value, "marketing");
        assert_eq!(stats[1].count, 5);
    }

    #[test]
    fn board_stats_falls_back_to_count_within_a_tier() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "c1", "acc1", "t1", "inbox", "topic", "contract");
        for i in 0..3 {
            insert_email_tagged(
                &db,
                &format!("p{i}"),
                "acc1",
                &format!("tp{i}"),
                "inbox",
                "topic",
                "project",
            );
        }

        let stats = get_tag_board_stats(&db, None, "topic", &EmailWindow::default(), 10).unwrap();
        // Same tier, so the busier block leads.
        assert_eq!(stats[0].tag_value, "project");
        assert_eq!(stats[1].tag_value, "contract");
    }

    #[test]
    fn board_stats_company_still_orders_purely_by_count() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "a1", "acc1", "t1", "inbox", "company", "aaa");
        for i in 0..3 {
            insert_email_tagged(
                &db,
                &format!("z{i}"),
                "acc1",
                &format!("tz{i}"),
                "inbox",
                "company",
                "zzz",
            );
        }
        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats[0].tag_value, "zzz");
    }

    // ── get_filtered_emails window ───────────────────────────────────────────

    #[test]
    fn filtered_emails_respects_the_category_filter() {
        // A block's list has to obey the same category chip its count does, or
        // the header says 1 thread and the body shows two.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged_at(&db, "e1", "acc1", "t1", "inbox", "company", "globex", "primary", 100);
        insert_email_tagged_at(&db, "e2", "acc1", "t2", "inbox", "company", "globex", "promotions", 100);

        let window = EmailWindow {
            categories: vec!["primary".into()],
            ..Default::default()
        };
        let got = get_filtered_emails(
            &db,
            Some("acc1"),
            None,
            None,
            Some("company"),
            Some("globex"),
            None,
            &window,
            50,
            0,
        )
        .unwrap();
        assert_eq!(got.emails.len(), 1);
        assert_eq!(got.emails[0].id, "e1");
    }

    #[test]
    fn filtered_emails_respects_the_time_window() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged_at(&db, "old", "acc1", "t1", "inbox", "company", "globex", "primary", 1_000);
        insert_email_tagged_at(&db, "new", "acc1", "t2", "inbox", "company", "globex", "primary", 5_000);

        let window = EmailWindow {
            since: Some(2_000),
            ..Default::default()
        };
        let got = get_filtered_emails(
            &db,
            Some("acc1"),
            None,
            None,
            Some("company"),
            Some("globex"),
            None,
            &window,
            50,
            0,
        )
        .unwrap();
        assert_eq!(got.emails.len(), 1);
        assert_eq!(got.emails[0].id, "new");
    }

    #[test]
    fn filtered_emails_default_window_changes_nothing() {
        // Every existing caller passes the default; it must stay a no-op.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged_at(&db, "e1", "acc1", "t1", "inbox", "company", "globex", "promotions", 1);
        insert_email_tagged_at(&db, "e2", "acc1", "t2", "inbox", "company", "globex", "social", 2);

        let got = get_filtered_emails(
            &db,
            Some("acc1"),
            None,
            None,
            Some("company"),
            Some("globex"),
            None,
            &EmailWindow::default(),
            50,
            0,
        )
        .unwrap();
        assert_eq!(got.emails.len(), 2);
    }

    // ── get_tag_board_stats ──────────────────────────────────────────────────

    #[test]
    fn board_stats_splits_one_tag_into_a_column_per_account() {
        // The board shows one block per (account, tag) so a block's title can
        // name the mailbox its threads actually live in.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        db.seed_test_account("acc2");
        insert_email_tagged(&db, "e1", "acc1", "t1", "inbox", "company", "globex");
        insert_email_tagged(&db, "e2", "acc1", "t2", "inbox", "company", "globex");
        insert_email_tagged(&db, "e3", "acc2", "t3", "inbox", "company", "globex");

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();

        assert_eq!(stats.len(), 2, "one block per account, not one merged block");
        assert_eq!(stats[0].account_id.as_deref(), Some("acc1"));
        assert_eq!(stats[0].count, 2);
        assert_eq!(stats[1].account_id.as_deref(), Some("acc2"));
        assert_eq!(stats[1].count, 1);
    }

    #[test]
    fn board_stats_orders_blocks_by_thread_count_desc() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged(&db, "e1", "acc1", "t1", "inbox", "company", "small");
        insert_email_tagged(&db, "e2", "acc1", "t2", "inbox", "company", "big");
        insert_email_tagged(&db, "e3", "acc1", "t3", "inbox", "company", "big");

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats[0].tag_value, "big");
        assert_eq!(stats[1].tag_value, "small");
    }

    #[test]
    fn board_stats_filters_by_category() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged_at(&db, "e1", "acc1", "t1", "inbox", "company", "globex", "primary", 100);
        insert_email_tagged_at(&db, "e2", "acc1", "t2", "inbox", "company", "globex", "promotions", 100);

        let window = EmailWindow {
            categories: vec!["primary".into()],
            ..Default::default()
        };
        let stats = get_tag_board_stats(&db, None, "company", &window, 10).unwrap();
        assert_eq!(stats[0].count, 1, "only the primary-category thread counts");
    }

    #[test]
    fn board_stats_empty_category_list_means_every_category() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged_at(&db, "e1", "acc1", "t1", "inbox", "company", "globex", "primary", 100);
        insert_email_tagged_at(&db, "e2", "acc1", "t2", "inbox", "company", "globex", "promotions", 100);

        let stats = get_tag_board_stats(&db, None, "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats[0].count, 2);
    }

    #[test]
    fn board_stats_applies_the_time_window() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        insert_email_tagged_at(&db, "old", "acc1", "t1", "inbox", "company", "globex", "primary", 1_000);
        insert_email_tagged_at(&db, "new", "acc1", "t2", "inbox", "company", "globex", "primary", 5_000);

        // `since` is inclusive, `until` exclusive — a half-open window so
        // adjacent day ranges can't double-count a thread on the boundary.
        let window = EmailWindow {
            since: Some(2_000),
            ..Default::default()
        };
        assert_eq!(
            get_tag_board_stats(&db, None, "company", &window, 10).unwrap()[0].count,
            1
        );

        let window = EmailWindow {
            until: Some(5_000),
            ..Default::default()
        };
        assert_eq!(
            get_tag_board_stats(&db, None, "company", &window, 10).unwrap()[0].count,
            1
        );

        let window = EmailWindow {
            since: Some(1_000),
            until: Some(5_001),
            ..Default::default()
        };
        assert_eq!(
            get_tag_board_stats(&db, None, "company", &window, 10).unwrap()[0].count,
            2
        );
    }

    #[test]
    fn board_stats_scoped_to_one_account_returns_only_its_blocks() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        db.seed_test_account("acc2");
        insert_email_tagged(&db, "e1", "acc1", "t1", "inbox", "company", "globex");
        insert_email_tagged(&db, "e2", "acc2", "t2", "inbox", "company", "globex");

        let stats = get_tag_board_stats(&db, Some("acc1"), "company", &EmailWindow::default(), 10).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].account_id.as_deref(), Some("acc1"));
    }

    #[test]
    fn board_stats_rejects_an_unknown_tag_type() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc1");
        let err = get_tag_board_stats(&db, None, "nope", &EmailWindow::default(), 10).unwrap_err();
        assert!(matches!(err, crate::models::error::AppError::InvalidInput(_)));
    }

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

#[cfg(test)]
mod participant_tests {
    use super::*;
    use rusqlite::params;

    fn seed(db: &Database, id: &str, thread: &str, sender: &str, sender_email: &str, to: &[&str], ts: i64) {
        db.connection()
            .execute(
                "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                 VALUES (?1,'acc',?2,'subj',?3,?4,'ex.com',?5,'[]','snip',?6,1,'primary','inbox',0)",
                params![id, thread, sender, sender_email, serde_json::to_string(to).unwrap(), ts],
            )
            .unwrap();
    }

    #[test]
    fn lists_every_other_person_in_the_thread() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");
        seed(&db, "e1", "t1", "Alice", "alice@ex.com", &["acc", "bob@ex.com"], 100);
        seed(&db, "e2", "t1", "Bob", "bob@ex.com", &["acc", "alice@ex.com"], 200);

        let out = get_thread_participants(&db, "acc", &["t1".to_string()]).unwrap();
        let names = &out.iter().find(|p| p.thread_id == "t1").unwrap().names;
        assert!(names.contains(&"Bob".to_string()));
        assert!(names.contains(&"Alice".to_string()));
    }

    #[test]
    fn excludes_the_account_owner() {
        // The user knows they are in their own thread; the point is who else is.
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");
        seed(&db, "e1", "t1", "Me", "acc", &["alice@ex.com"], 100);

        let out = get_thread_participants(&db, "acc", &["t1".to_string()]).unwrap();
        let names = &out[0].names;
        assert!(
            !names.iter().any(|n| n == "Me" || n == "acc"),
            "own address must not appear: {names:?}"
        );
        assert_eq!(names, &vec!["alice@ex.com".to_string()]);
    }

    #[test]
    fn prefers_a_display_name_over_a_bare_address() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");
        // Alice appears as a recipient address first, then as a named sender.
        seed(&db, "e1", "t1", "Me", "acc", &["alice@ex.com"], 100);
        seed(&db, "e2", "t1", "Alice Smith", "alice@ex.com", &["acc"], 200);

        let out = get_thread_participants(&db, "acc", &["t1".to_string()]).unwrap();
        assert_eq!(out[0].names, vec!["Alice Smith".to_string()]);
    }

    #[test]
    fn lists_the_most_recently_active_person_first() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");
        seed(&db, "e1", "t1", "Old", "old@ex.com", &["acc"], 100);
        seed(&db, "e2", "t1", "Recent", "recent@ex.com", &["acc"], 300);

        let out = get_thread_participants(&db, "acc", &["t1".to_string()]).unwrap();
        assert_eq!(out[0].names.first(), Some(&"Recent".to_string()));
    }

    #[test]
    fn counts_a_person_once_however_often_they_appear() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");
        for (i, ts) in [100, 200, 300].iter().enumerate() {
            seed(&db, &format!("e{i}"), "t1", "Alice", "alice@ex.com", &["acc"], *ts);
        }
        let out = get_thread_participants(&db, "acc", &["t1".to_string()]).unwrap();
        assert_eq!(out[0].names, vec!["Alice".to_string()]);
    }

    #[test]
    fn matches_addresses_case_insensitively() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");
        seed(&db, "e1", "t1", "Alice", "Alice@Ex.com", &["acc"], 100);
        seed(&db, "e2", "t1", "Me", "acc", &["ALICE@ex.com"], 200);

        let out = get_thread_participants(&db, "acc", &["t1".to_string()]).unwrap();
        assert_eq!(out[0].names.len(), 1, "one person, not two casings: {:?}", out[0].names);
    }

    #[test]
    fn strips_angle_brackets_from_recipient_headers() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");
        seed(&db, "e1", "t1", "Me", "acc", &["Bob Jones <bob@ex.com>"], 100);

        let out = get_thread_participants(&db, "acc", &["t1".to_string()]).unwrap();
        assert_eq!(out[0].names, vec!["Bob Jones".to_string()]);
    }

    #[test]
    fn returns_a_row_per_requested_thread_and_ignores_others() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");
        seed(&db, "e1", "t1", "Alice", "alice@ex.com", &["acc"], 100);
        seed(&db, "e2", "t2", "Bob", "bob@ex.com", &["acc"], 100);
        seed(&db, "e3", "t3", "Carol", "carol@ex.com", &["acc"], 100);

        let out = get_thread_participants(&db, "acc", &["t1".to_string(), "t3".to_string()]).unwrap();
        let ids: Vec<&str> = out.iter().map(|p| p.thread_id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"t1") && ids.contains(&"t3"));
    }

    #[test]
    fn an_empty_request_makes_no_query() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("acc");
        assert!(get_thread_participants(&db, "acc", &[]).unwrap().is_empty());
    }
}
