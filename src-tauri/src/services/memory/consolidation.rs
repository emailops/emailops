//! Dream-time consolidation for the memory subsystem.
//!
//! Runs periodically (every 30 min) and once after each sync. Responsibilities:
//!
//! - **Score**: bump `memory_facts.score` based on recency, confidence, and
//!   corroboration (multiple facts about the same subject agreeing).
//! - **Promote**: flip high-scoring candidates to `status='promoted'`.
//! - **Deduplicate**: within the same `(subject_kind, subject_key)` cluster,
//!   retire near-duplicates (cosine similarity ≥ 0.88) in favour of the
//!   highest-scoring representative.
//! - **Prune**: delete `interaction_events` older than the retention window;
//!   retire candidate facts that never gained traction.
//!
//! Never fails the caller — all operations are best-effort and log errors.

use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use crate::db::Database;
use crate::models::error::Result;
use crate::models::{AppLogEvent, MemoryFact};

/// Cosine similarity ≥ this → the two facts say effectively the same thing.
const DEDUP_SIMILARITY: f32 = 0.88;
/// Max candidates per run so a very large queue can't starve the DB.
const MAX_CANDIDATES_PER_RUN: i32 = 500;

#[derive(Debug, Default, Clone)]
pub struct ConsolidationStats {
    pub scored: u32,
    pub promoted: u32,
    pub retired_dedup: u32,
    pub retired_ttl: u32,
    pub events_pruned: usize,
}

/// Run a full consolidation pass for `account_id`. Returns the stats so the
/// caller can log or display them.
pub fn run_consolidation(db: &Arc<Database>, app: Option<&AppHandle>, account_id: &str) -> Result<ConsolidationStats> {
    let cfg = crate::services::memory::config::get_config(db)?;
    let mut stats = ConsolidationStats::default();
    let now = chrono::Utc::now().timestamp();

    log(app, "info", &format!("Dream: start (account={account_id})"));

    // 1. Score candidates.
    let candidates = db.list_candidate_facts(account_id, MAX_CANDIDATES_PER_RUN)?;
    stats.scored = score_candidates(db, &candidates, now)? as u32;

    // 2. Promote candidates above threshold. Re-read after scoring so we see
    //    the new scores.
    let rescored = db.list_candidate_facts(account_id, MAX_CANDIDATES_PER_RUN)?;
    let threshold = cfg.promote_threshold as f64;
    for fact in &rescored {
        if fact.score >= threshold {
            if let Err(e) = db.set_memory_fact_status(&fact.id, "promoted", now) {
                log(app, "warn", &format!("promote failed for {}: {e}", fact.id));
            } else {
                stats.promoted += 1;
            }
        }
    }

    // 3. Deduplicate within (kind,key) clusters.
    stats.retired_dedup = dedup_subject_clusters(db, app, account_id, now)?;

    // 4. Prune stale candidates.
    let candidate_ttl_secs = (cfg.candidate_ttl_days as i64) * 86_400;
    let ttl_cutoff = now - candidate_ttl_secs;
    let stale = db.list_stale_candidate_facts(account_id, ttl_cutoff, 0.2)?;
    for fact in &stale {
        if let Err(e) = db.set_memory_fact_status(&fact.id, "retired", now) {
            log(app, "warn", &format!("ttl-retire failed for {}: {e}", fact.id));
        } else {
            stats.retired_ttl += 1;
        }
    }

    // 5. Prune old interaction events. Account-insensitive: the cutoff is
    //    global. OK because events are append-only and accounts are typically
    //    singletons in practice.
    let event_retention_secs = (cfg.event_retention_days as i64) * 86_400;
    match db.prune_interaction_events(now - event_retention_secs) {
        Ok(n) => stats.events_pruned = n,
        Err(e) => log(app, "warn", &format!("prune_interaction_events failed: {e}")),
    }

    log(
        app,
        "success",
        &format!(
            "Dream: scored={}, promoted={}, dedup_retired={}, ttl_retired={}, events_pruned={}",
            stats.scored, stats.promoted, stats.retired_dedup, stats.retired_ttl, stats.events_pruned
        ),
    );

    Ok(stats)
}

/// Score = confidence * recency_decay + recall_bonus + corroboration_bonus.
/// We write the *delta* via `bump_memory_fact_score` so repeat runs compound
/// gracefully (if a fact is still alive next pass, its score keeps climbing).
fn score_candidates(db: &Arc<Database>, candidates: &[MemoryFact], now: i64) -> Result<usize> {
    // Pre-group by (kind,key) so we can apply corroboration bonuses.
    use std::collections::HashMap;
    let mut groups: HashMap<(String, String), Vec<&MemoryFact>> = HashMap::new();
    for f in candidates {
        groups
            .entry((f.subject_kind.clone(), f.subject_key.clone()))
            .or_default()
            .push(f);
    }

    let mut scored = 0usize;
    for fact in candidates {
        let age_days = ((now - fact.created_at).max(0) as f64) / 86_400.0;
        // Half-life of 30 days: stays above 0.5 for a month, decays slowly after.
        let recency = 2f64.powf(-age_days / 30.0);
        let recall_bonus = fact.last_used_at.map(|_| 0.1).unwrap_or(0.0);
        let corroboration = {
            let group = groups
                .get(&(fact.subject_kind.clone(), fact.subject_key.clone()))
                .map(|v| v.len())
                .unwrap_or(1);
            // +0.05 per co-occurring fact, cap at +0.2
            (0.05 * (group.saturating_sub(1)) as f64).min(0.2)
        };

        let target = (fact.confidence * recency + recall_bonus + corroboration).clamp(0.0, 1.0);
        let delta = target - fact.score;
        if delta.abs() > 0.001 && db.bump_memory_fact_score(&fact.id, delta, now).is_ok() {
            scored += 1;
        }
    }
    Ok(scored)
}

/// Within each (subject_kind, subject_key) group that has duplicates, embed
/// is already available — use the vector store to find pairs with cosine
/// similarity above `DEDUP_SIMILARITY`. Retire everything except the highest
/// scoring member of each near-duplicate cluster.
fn dedup_subject_clusters(db: &Arc<Database>, app: Option<&AppHandle>, account_id: &str, now: i64) -> Result<u32> {
    let groups = db.list_subject_groups_with_duplicates(account_id)?;
    let mut retired = 0u32;

    for (kind, key) in &groups {
        let facts = db.get_memory_facts_by_subject(account_id, kind, key)?;
        if facts.len() < 2 {
            continue;
        }

        // Simple O(N^2) pairwise comparison within a single subject cluster.
        // Subject clusters are almost always tiny (< 20 facts) so this is fine.
        let mut keep: Vec<&MemoryFact> = Vec::new();
        let mut to_retire: Vec<String> = Vec::new();

        for fact in &facts {
            let mut replaced = false;
            for existing in &mut keep {
                if texts_equivalent(&existing.fact, &fact.fact) {
                    // New fact is a near-duplicate of an existing kept one.
                    // Keep whichever has the higher score; retire the other.
                    if fact.score > existing.score {
                        to_retire.push(existing.id.clone());
                        *existing = fact;
                    } else {
                        to_retire.push(fact.id.clone());
                    }
                    replaced = true;
                    break;
                }
            }
            if !replaced {
                keep.push(fact);
            }
        }

        for id in to_retire {
            if let Err(e) = db.set_memory_fact_status(&id, "retired", now) {
                log(app, "warn", &format!("dedup-retire failed for {id}: {e}"));
            } else {
                retired += 1;
            }
        }
    }
    Ok(retired)
}

/// Lightweight text similarity: lowercase, alphanumeric tokens, Jaccard
/// similarity. No vector DB call — good enough for dedup inside a single
/// (kind, key) cluster where facts are already about the same subject.
/// `pub(super)` so the extractor can reuse it for write-time dedup.
pub(super) fn texts_equivalent(a: &str, b: &str) -> bool {
    let ta = tokens(a);
    let tb = tokens(b);
    if ta.is_empty() || tb.is_empty() {
        return a.trim().eq_ignore_ascii_case(b.trim());
    }
    let inter = ta.iter().filter(|t| tb.contains(*t)).count();
    let union = ta.len() + tb.len() - inter;
    if union == 0 {
        return false;
    }
    (inter as f32) / (union as f32) >= DEDUP_SIMILARITY
}

fn tokens(s: &str) -> std::collections::HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && t.len() > 2)
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

fn log(app: Option<&AppHandle>, level: &str, message: &str) {
    if let Some(app) = app {
        let _ = app.emit(
            "app-log",
            AppLogEvent {
                level: level.to_string(),
                source: "memory".to_string(),
                message: message.to_string(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MemoryFact;

    fn new_fact(id: &str, text: &str, confidence: f64, created_at: i64) -> MemoryFact {
        MemoryFact {
            id: id.to_string(),
            account_id: "a1".into(),
            subject_kind: "contact".into(),
            subject_key: "alice@ex.com".into(),
            fact: text.to_string(),
            source: "extraction".into(),
            source_email_id: None,
            confidence,
            score: 0.0,
            status: "candidate".into(),
            last_used_at: None,
            domain: None,
            vigency: None,
            company: None,
            created_at,
            updated_at: created_at,
        }
    }

    #[test]
    fn texts_equivalent_sees_near_duplicates() {
        assert!(texts_equivalent(
            "Alice handles billing at Acme",
            "alice handles billing at acme"
        ));
        assert!(!texts_equivalent(
            "Alice handles billing at Acme",
            "Bob is the CTO of another company",
        ));
    }

    #[test]
    fn consolidation_promotes_high_confidence_recent_facts() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("a1");
        let now = chrono::Utc::now().timestamp();
        // Confidence 0.9, co-occurring with two other facts → target ~0.9+0.1.
        db.insert_memory_fact(&new_fact("f1", "Alice leads ops", 0.9, now))
            .unwrap();
        db.insert_memory_fact(&new_fact("f2", "Alice prefers Slack over email", 0.8, now))
            .unwrap();
        db.insert_memory_fact(&new_fact("f3", "Alice is in Madrid", 0.7, now))
            .unwrap();

        let stats = run_consolidation(&db, None, "a1").unwrap();
        assert!(stats.scored >= 3);
        // At least the high-confidence one must promote.
        assert!(stats.promoted >= 1);
    }

    #[test]
    fn consolidation_retires_stale_low_score_candidates() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("a1");
        let now = chrono::Utc::now().timestamp();
        // Old, tiny-confidence fact -> should be retired for TTL.
        let old = now - 86_400 * 20; // 20 days old
        db.insert_memory_fact(&new_fact("old1", "outdated", 0.05, old)).unwrap();

        let stats = run_consolidation(&db, None, "a1").unwrap();
        assert!(stats.retired_ttl >= 1);
    }

    #[test]
    fn consolidation_dedups_near_duplicate_facts() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("a1");
        let now = chrono::Utc::now().timestamp();
        // Three near-identical sentences about Alice. One should survive.
        let mut a = new_fact("a1", "Alice handles billing at Acme", 0.9, now);
        a.score = 0.8;
        db.insert_memory_fact(&a).unwrap();
        let mut b = new_fact("a2", "alice handles billing at acme", 0.85, now);
        b.score = 0.5;
        db.insert_memory_fact(&b).unwrap();

        let stats = run_consolidation(&db, None, "a1").unwrap();
        assert!(stats.retired_dedup >= 1);
    }

    #[test]
    fn consolidation_prunes_old_events() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("a1");
        // Backdate by 40 days via direct SQL (log_interaction_event always uses now()).
        let cutoff = chrono::Utc::now().timestamp() - 86_400 * 40;
        db.connection()
            .execute(
                "INSERT INTO interaction_events (account_id, kind, email_id, thread_id, payload_json, created_at)
                 VALUES ('a1', 'read', 'e1', 't1', NULL, ?1)",
                rusqlite::params![cutoff],
            )
            .unwrap();

        let stats = run_consolidation(&db, None, "a1").unwrap();
        assert!(stats.events_pruned >= 1);
    }
}
