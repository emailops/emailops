//! Incremental tag-priority scoring.
//!
//! `tag_priority` stores raw signals per (account, tag_type, tag_value):
//! `sent_count`, `received_count`, and `last_activity_at`. The actual priority
//! score is computed at read time (§ [`get_priorities`]) so the weights can be
//! tuned without a migration.
//!
//! The sync loop calls [`update_from_new_emails`] with the IDs of emails it
//! just inserted. This function reads only the tags attached to those IDs,
//! aggregates per `tag_value`, and upserts the delta — so a sync of 20 emails
//! updates at most 20 tag rows, not the full company set.

use std::collections::HashMap;
use std::sync::Arc;

use rusqlite::params_from_iter;

use crate::db::Database;
use crate::models::error::Result;
use crate::models::TagPriority;

/// Upper bound on the IDs-per-query. SQLite's parameter limit is 32,766 and
/// our sync batches are well under that; this is a defensive chunk size so
/// that a one-off backfill over a giant ID set never hits the limit.
const ID_CHUNK: usize = 500;

/// Incrementally update `tag_priority` for the company tags attached to the
/// given email IDs. Read-then-write: one read to gather (tag_value,
/// sender_email, timestamp) triples, one write transaction to UPSERT deltas.
///
/// Touches only the tags that appear on `new_email_ids` — never a full
/// recomputation. Safe to call with an empty list (returns `Ok(0)`).
///
/// Returns the number of distinct tag rows updated.
pub fn update_from_new_emails(
    db: &Arc<Database>,
    account_id: &str,
    owner_email: &str,
    new_email_ids: &[String],
) -> Result<u32> {
    if new_email_ids.is_empty() {
        return Ok(0);
    }

    let owner_lc = owner_email.trim().to_ascii_lowercase();

    // Aggregate across all chunks so a single sync produces one UPSERT per
    // touched tag, even when the ID list spans multiple reads.
    let mut acc: HashMap<String, (i64, i64, i64)> = HashMap::new(); // tag_value -> (sent, recv, max_ts)

    for chunk in new_email_ids.chunks(ID_CHUNK) {
        read_tag_activity(db, account_id, chunk, &owner_lc, &mut acc)?;
    }

    if acc.is_empty() {
        return Ok(0);
    }

    write_priority_deltas(db, account_id, "company", &acc)?;
    Ok(acc.len() as u32)
}

/// Read the (tag_value, sender_email, timestamp) triples for the given email
/// IDs and fold them into `acc`. Reused across chunked calls.
fn read_tag_activity(
    db: &Arc<Database>,
    account_id: &str,
    email_ids: &[String],
    owner_lc: &str,
    acc: &mut HashMap<String, (i64, i64, i64)>,
) -> Result<()> {
    let placeholders = std::iter::repeat_n("?", email_ids.len()).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT t.tag_value, e.sender_email, e.timestamp
         FROM email_tags t
         JOIN emails e ON e.id = t.email_id
         WHERE t.tag_type = 'company'
           AND e.account_id = ?1
           AND e.is_deleted = 0
           AND t.email_id IN ({placeholders})"
    );

    let conn = db.reader();
    let mut stmt = conn.prepare(&sql)?;

    // Bind account_id as ?1, then the email IDs.
    let mut bound: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(1 + email_ids.len());
    bound.push(&account_id);
    for id in email_ids {
        bound.push(id);
    }

    let mut rows = stmt.query(params_from_iter(bound.iter()))?;
    while let Some(row) = rows.next()? {
        let tag_value: String = row.get(0)?;
        let sender_email: String = row.get(1)?;
        let timestamp: i64 = row.get(2)?;

        let is_outbound = sender_email.trim().to_ascii_lowercase() == owner_lc;
        let entry = acc.entry(tag_value).or_insert((0, 0, 0));
        if is_outbound {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
        if timestamp > entry.2 {
            entry.2 = timestamp;
        }
    }
    Ok(())
}

/// Write every aggregated delta in a single transaction. UPSERT keeps the
/// logic idempotent on retry: counts accumulate, `last_activity_at` takes the
/// MAX of existing and incoming.
fn write_priority_deltas(
    db: &Arc<Database>,
    account_id: &str,
    tag_type: &str,
    acc: &HashMap<String, (i64, i64, i64)>,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let mut conn = db.connection();
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO tag_priority
                (account_id, tag_type, tag_value, sent_count, received_count, last_activity_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(account_id, tag_type, tag_value) DO UPDATE SET
                 sent_count       = sent_count + excluded.sent_count,
                 received_count   = received_count + excluded.received_count,
                 last_activity_at = MAX(COALESCE(last_activity_at, 0), COALESCE(excluded.last_activity_at, 0)),
                 updated_at       = excluded.updated_at",
        )?;
        for (tag_value, (sent, recv, max_ts)) in acc {
            let last_activity: Option<i64> = if *max_ts > 0 { Some(*max_ts) } else { None };
            stmt.execute(rusqlite::params![
                account_id,
                tag_type,
                tag_value,
                sent,
                recv,
                last_activity,
                now,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// One-shot backfill for an account that has no priority rows yet. Aggregates
/// every existing `company` tag on this account's non-deleted emails and
/// populates `tag_priority` in a single write transaction. Idempotent: if the
/// account already has rows, returns 0 without touching the DB.
///
/// Called after `email_company::backfill_account` when the feature is first
/// installed — by then there are company tags to aggregate, whereas the DB
/// init's one-shot runs earlier (before any company tags exist) and sees
/// nothing.
pub fn backfill_account(db: &Arc<Database>, account_id: &str) -> Result<u32> {
    // Guard: skip accounts that already have priorities. Cheap existence check.
    {
        let conn = db.reader();
        let has_rows: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tag_priority WHERE account_id = ?1 LIMIT 1)",
            rusqlite::params![account_id],
            |row| row.get(0),
        )?;
        if has_rows {
            return Ok(0);
        }
    }

    let now = chrono::Utc::now().timestamp();
    let conn = db.connection();
    let tx = conn.unchecked_transaction()?;
    // TRIM both sides so the backfill matches `update_from_new_emails`'s
    // trim+lowercase comparison — otherwise a sender_email with trailing
    // whitespace would be miscounted as inbound here but outbound in the
    // incremental path.
    let written = tx.execute(
        "INSERT INTO tag_priority
             (account_id, tag_type, tag_value, sent_count, received_count, last_activity_at, updated_at)
         SELECT e.account_id, t.tag_type, t.tag_value,
                SUM(CASE WHEN LOWER(TRIM(e.sender_email)) = LOWER(TRIM(a.email)) THEN 1 ELSE 0 END),
                SUM(CASE WHEN LOWER(TRIM(e.sender_email)) = LOWER(TRIM(a.email)) THEN 0 ELSE 1 END),
                MAX(e.timestamp),
                ?2
         FROM email_tags t
         JOIN emails   e ON e.id = t.email_id
         JOIN accounts a ON a.id = e.account_id
         WHERE t.tag_type = 'company'
           AND e.account_id = ?1
           AND e.is_deleted = 0
         GROUP BY e.account_id, t.tag_type, t.tag_value",
        rusqlite::params![account_id, now],
    )?;
    tx.commit()?;
    Ok(written as u32)
}

/// One-shot rebuild for a single (account, tag_type) slice. Unlike
/// [`backfill_account`] this is **not** guarded: it deletes every existing
/// `tag_priority` row for the given account+tag_type, then reaggregates from
/// `email_tags` + `emails`. Use after a migration that invalidates aggregates
/// (e.g. [`crate::services::email_company::retag_personal_domains`] which
/// changes the `tag_value` vocabulary).
///
/// The whole operation runs in one transaction so a concurrent read never
/// sees a partial rebuild.
pub fn rebuild_account_tag_type(db: &Arc<Database>, account_id: &str, tag_type: &str) -> Result<u32> {
    let now = chrono::Utc::now().timestamp();
    let conn = db.connection();
    let tx = conn.unchecked_transaction()?;

    tx.execute(
        "DELETE FROM tag_priority WHERE account_id = ?1 AND tag_type = ?2",
        rusqlite::params![account_id, tag_type],
    )?;

    let written = tx.execute(
        "INSERT INTO tag_priority
             (account_id, tag_type, tag_value, sent_count, received_count, last_activity_at, updated_at)
         SELECT e.account_id, t.tag_type, t.tag_value,
                SUM(CASE WHEN LOWER(TRIM(e.sender_email)) = LOWER(TRIM(a.email)) THEN 1 ELSE 0 END),
                SUM(CASE WHEN LOWER(TRIM(e.sender_email)) = LOWER(TRIM(a.email)) THEN 0 ELSE 1 END),
                MAX(e.timestamp),
                ?3
         FROM email_tags t
         JOIN emails   e ON e.id = t.email_id
         JOIN accounts a ON a.id = e.account_id
         WHERE t.tag_type = ?2
           AND e.account_id = ?1
           AND e.is_deleted = 0
         GROUP BY e.account_id, t.tag_type, t.tag_value",
        rusqlite::params![account_id, tag_type, now],
    )?;

    tx.commit()?;
    Ok(written as u32)
}

/// Read-side: compute the priority score in SQL and return rows ordered by
/// score descending.
///
/// # Formula
///
/// ```text
/// mutual_ratio     = min(sent, received) / max(sent, received)   // in [0, 1]
/// volume_cap       = min(sent + received, VOLUME_SATURATION)
/// age_days         = max(0, (now - last_activity_at) / 86400)
///
/// engagement_decay = 1 / (1 + age_days / ENGAGEMENT_HALF_LIFE_DAYS)
/// recency_decay    = 1 / (1 + age_days / RECENCY_HALF_LIFE_DAYS)
///
/// engagement       = mutual_ratio * volume_cap * RATIO_WEIGHT * engagement_decay
/// engagement_gate  = ENGAGEMENT_FLOOR + (1 - ENGAGEMENT_FLOOR) * mutual_ratio
/// recency          = RECENCY_MAX * recency_decay * engagement_gate
///
/// score            = engagement + recency
/// ```
///
/// Intent — **two-way interaction is what counts, and it fades with time**:
/// - **Mutual ratio** (`min/max`) is 1 only when sent ≈ received and 0
///   whenever traffic is one-way *in either direction*. A newsletter
///   (`0 sent / 30 recv`) and a service you blast docs to
///   (`148 sent / 0 recv`) both score 0 — neither is a real relationship.
/// - **Engagement decays too, not just recency.** A perfectly balanced 4/4
///   exchange from 2007 used to score 16 because the engagement term had no
///   age component. It now decays at a longer half-life (1 year) than the
///   recency boost (30 days), so dormant relationships fade gradually rather
///   than rank forever.
/// - **Engagement-gated recency**: a one-way recent sender gets only
///   `ENGAGEMENT_FLOOR` (30 %) of the recency boost; a two-way conversation
///   gets the full boost.
/// - **Volume saturates at `VOLUME_SATURATION`** so a 200-mail thread does
///   not permanently outrank a healthy 50-mail relationship.
///
/// Tunables (kept inline in the SQL so they're inspectable from a sqlite3
/// prompt; promote to Rust constants if/when we expose tuning):
///   VOLUME_SATURATION         = 50    -- emails after which volume stops adding
///   RATIO_WEIGHT              = 2.0   -- multiplier on (mutual * capped volume)
///   RECENCY_MAX               = 50.0  -- recency score at age 0 with full gate
///   RECENCY_HALF_LIFE_DAYS    = 30.0  -- recency halves at this age
///   ENGAGEMENT_HALF_LIFE_DAYS = 365.0 -- engagement halves at this age (slower)
///   ENGAGEMENT_FLOOR          = 0.3   -- minimum recency multiplier for one-way
pub fn get_priorities(db: &Arc<Database>, account_id: &str, tag_type: &str, limit: i32) -> Result<Vec<TagPriority>> {
    let conn = db.reader();
    let mut stmt = conn.prepare(
        "SELECT tag_type, tag_value, sent_count, received_count, last_activity_at,
                (
                  -- engagement: mutual_ratio * volume_cap * RATIO_WEIGHT * decay
                  -- mutual_ratio = min(sent,recv)/max(sent,recv) — peaks at 1 for
                  -- balanced traffic, 0 for one-way (either direction).
                  -- engagement_decay = 1/(1+age_days/365): 1-year half-life so old
                  -- relationships fade (a perfectly balanced 2007 exchange shouldn't
                  -- score the same as a 2026 one). When last_activity_at is NULL
                  -- the decay multiplier is 1 (no time info → don't penalise).
                  CASE WHEN max(sent_count, received_count) > 0 THEN
                    (CAST(min(sent_count, received_count) AS REAL)
                       / CAST(max(sent_count, received_count) AS REAL))
                    * CAST(min(sent_count + received_count, 50) AS REAL)
                    * 2.0
                    *
                    CASE WHEN last_activity_at IS NULL THEN 1.0
                         ELSE 1.0 / (1.0 + max(0.0, (CAST(strftime('%s','now') AS REAL) - CAST(last_activity_at AS REAL)) / 86400.0) / 365.0)
                    END
                  ELSE 0.0 END
                  +
                  -- recency: 50/(1+age_days/30), multiplied by engagement gate
                  CASE WHEN last_activity_at IS NULL THEN 0.0 ELSE
                    (50.0 / (1.0 + max(0.0, (CAST(strftime('%s','now') AS REAL) - CAST(last_activity_at AS REAL)) / 86400.0) / 30.0))
                    *
                    (0.3 + 0.7 * CASE WHEN max(sent_count, received_count) > 0
                                      THEN CAST(min(sent_count, received_count) AS REAL)
                                           / CAST(max(sent_count, received_count) AS REAL)
                                      ELSE 0.0 END)
                  END
                ) AS priority_score
         FROM tag_priority
         WHERE account_id = ?1 AND tag_type = ?2
         ORDER BY priority_score DESC, last_activity_at DESC
         LIMIT ?3",
    )?;

    let rows = stmt
        .query_map(rusqlite::params![account_id, tag_type, limit], |row| {
            Ok(TagPriority {
                tag_type: row.get(0)?,
                tag_value: row.get(1)?,
                sent_count: row.get(2)?,
                received_count: row.get(3)?,
                last_activity_at: row.get(4)?,
                priority_score: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn seed_account(db: &Database, id: &str, email: &str) {
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?2, 'Test', 0)",
                params![id, email],
            )
            .unwrap();
    }

    fn seed_email(db: &Database, id: &str, account_id: &str, sender: &str, timestamp: i64) {
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?1, 'Test', 0)",
            params![account_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails
                    (id, account_id, thread_id, subject, sender, sender_email,
                     sender_domain, recipients_json, cc_json, snippet, timestamp,
                     is_read, is_deleted, category, created_at)
                 VALUES (?1, ?2, ?3, 'subj', ?4, ?4, '', '[]', '[]', '', ?5,
                         0, 0, 'primary', 0)",
            params![id, account_id, format!("thread-{id}"), sender, timestamp],
        )
        .unwrap();
        conn.execute("INSERT INTO email_bodies (email_id, body) VALUES (?1, '')", params![id])
            .unwrap();
    }

    fn seed_tag(db: &Database, email_id: &str, tag_type: &str, tag_value: &str) {
        db.connection()
            .execute(
                "INSERT INTO email_tags (email_id, tag_type, tag_value, confidence, created_at)
                 VALUES (?1, ?2, ?3, NULL, 0)",
                params![email_id, tag_type, tag_value],
            )
            .unwrap();
    }

    fn get_row(db: &Database, account_id: &str, tag_value: &str) -> (i64, i64, Option<i64>) {
        let conn = db.reader();
        conn.query_row(
            "SELECT sent_count, received_count, last_activity_at
             FROM tag_priority WHERE account_id = ?1 AND tag_type = 'company' AND tag_value = ?2",
            params![account_id, tag_value],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
    }

    #[test]
    fn update_from_new_emails_counts_outbound_vs_inbound() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        // Outbound: I am the sender → acme gets a sent_count bump.
        seed_email(&db, "e1", "acc1", "me@mine.com", 1000);
        seed_tag(&db, "e1", "company", "acme");
        // Inbound: alice wrote to me, tagged acme too.
        seed_email(&db, "e2", "acc1", "alice@acme.com", 2000);
        seed_tag(&db, "e2", "company", "acme");

        let n = update_from_new_emails(&db, "acc1", "me@mine.com", &["e1".into(), "e2".into()]).unwrap();
        assert_eq!(n, 1);

        let (sent, recv, last) = get_row(&db, "acc1", "acme");
        assert_eq!(sent, 1);
        assert_eq!(recv, 1);
        assert_eq!(last, Some(2000));
    }

    #[test]
    fn update_from_new_emails_is_incremental() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        seed_email(&db, "e1", "acc1", "me@mine.com", 1000);
        seed_tag(&db, "e1", "company", "acme");
        seed_email(&db, "e2", "acc1", "me@mine.com", 1500);
        seed_tag(&db, "e2", "company", "acme");

        // First batch.
        update_from_new_emails(&db, "acc1", "me@mine.com", &["e1".into()]).unwrap();
        let (sent, _, last) = get_row(&db, "acc1", "acme");
        assert_eq!(sent, 1);
        assert_eq!(last, Some(1000));

        // Second batch — counts accumulate, last_activity_at advances.
        update_from_new_emails(&db, "acc1", "me@mine.com", &["e2".into()]).unwrap();
        let (sent, _, last) = get_row(&db, "acc1", "acme");
        assert_eq!(sent, 2);
        assert_eq!(last, Some(1500));
    }

    #[test]
    fn update_from_new_emails_ignores_other_tag_types() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        seed_email(&db, "e1", "acc1", "me@mine.com", 1000);
        // Only an `intent` tag — no company tag — should produce no row.
        seed_tag(&db, "e1", "intent", "request");

        let n = update_from_new_emails(&db, "acc1", "me@mine.com", &["e1".into()]).unwrap();
        assert_eq!(n, 0);

        let conn = db.reader();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tag_priority", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn get_priorities_orders_by_score() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");

        // Two-way scoring with engagement decay (1-year half-life):
        //   engagement = mutual_ratio * min(total,50) * 2 * 1/(1+age/365)
        //   recency    = 50/(1 + age_days/30) * (0.3 + 0.7*mutual_ratio)
        //
        // engaged_old   (10/10, 200d, mutual=1, eng_dec=1/(1+0.548)=0.646):
        //   eng=40*0.646=25.8, rec_raw≈6.52, gate=1.0  → rec≈6.52   → ~32.3
        // newsletter    ( 0/30,   3d, mutual=0):
        //   eng=0,             rec_raw≈45.45, gate=0.3 → rec≈13.6   → ~13.6
        // engaged_recent( 5/ 5,   3d, mutual=1, eng_dec≈0.992):
        //   eng=20*0.992≈19.8, rec_raw≈45.45, gate=1.0 → rec≈45.45  → ~65.3
        // one_off_recent( 1/ 0,   1d, mutual=0):
        //   eng=0,             rec_raw≈48.39, gate=0.3 → rec≈14.5   → ~14.5
        let now = chrono::Utc::now().timestamp();
        let day = 86_400i64;
        db.connection()
            .execute_batch(&format!(
                "INSERT INTO tag_priority VALUES
                   ('acc1', 'company', 'engaged_old',     10, 10, {now_200d}, {now}),
                   ('acc1', 'company', 'newsletter',       0, 30, {now_3d},   {now}),
                   ('acc1', 'company', 'engaged_recent',   5,  5, {now_3d},   {now}),
                   ('acc1', 'company', 'one_off_recent',   1,  0, {now_1d},   {now});",
                now = now,
                now_1d = now - day,
                now_3d = now - 3 * day,
                now_200d = now - 200 * day,
            ))
            .unwrap();

        let rows = get_priorities(&db, "acc1", "company", 10).unwrap();
        let order: Vec<&str> = rows.iter().map(|r| r.tag_value.as_str()).collect();
        // Balanced two-way conversations dominate, regardless of recency.
        // One-way patterns (newsletter, one_off_recent) score 0 on engagement
        // and only 30 % recency, falling below any real relationship.
        assert_eq!(
            order,
            vec!["engaged_recent", "engaged_old", "one_off_recent", "newsletter"]
        );
    }

    #[test]
    fn get_priorities_engagement_beats_one_way_mail() {
        // The whole point of the reply-ratio reweighting: an actively replied
        // conversation must rank above a higher-volume newsletter, regardless
        // of recency parity.
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        let now = chrono::Utc::now().timestamp();
        db.connection()
            .execute_batch(&format!(
                "INSERT INTO tag_priority VALUES
                   ('acc1','company','newsletter', 0, 200, {now}, {now}),
                   ('acc1','company','client',     5,   5, {now}, {now});",
                now = now,
            ))
            .unwrap();

        let rows = get_priorities(&db, "acc1", "company", 10).unwrap();
        let client = rows.iter().find(|r| r.tag_value == "client").unwrap();
        let newsletter = rows.iter().find(|r| r.tag_value == "newsletter").unwrap();
        assert!(
            client.priority_score > newsletter.priority_score,
            "client {} should outrank newsletter {}",
            client.priority_score,
            newsletter.priority_score
        );
    }

    #[test]
    fn get_priorities_volume_saturates() {
        // Above the volume saturation cap (50), adding more emails of the
        // same ratio should not change the engagement score.
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        let now = chrono::Utc::now().timestamp();
        db.connection()
            .execute_batch(&format!(
                "INSERT INTO tag_priority VALUES
                   ('acc1','company','at_cap',   25,  25, NULL, {now}),
                   ('acc1','company','far_above',500, 500, NULL, {now});",
                now = now,
            ))
            .unwrap();

        let rows = get_priorities(&db, "acc1", "company", 10).unwrap();
        let at_cap = rows.iter().find(|r| r.tag_value == "at_cap").unwrap().priority_score;
        let far_above = rows.iter().find(|r| r.tag_value == "far_above").unwrap().priority_score;
        assert_eq!(at_cap, far_above, "score must saturate at the volume cap");
    }

    #[test]
    fn update_from_new_emails_case_insensitive_outbound() {
        // Gmail sometimes returns the From header with a different case
        // than what's stored in accounts.email. The comparison must be
        // case-insensitive or sent emails silently become inbound.
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "Me@Mine.com");
        seed_email(&db, "e1", "acc1", "me@mine.com", 1000);
        seed_tag(&db, "e1", "company", "acme");

        update_from_new_emails(&db, "acc1", "Me@Mine.com", &["e1".into()]).unwrap();
        let (sent, recv, _) = get_row(&db, "acc1", "acme");
        assert_eq!(sent, 1, "uppercase owner must still match lowercase sender");
        assert_eq!(recv, 0);
    }

    #[test]
    fn update_from_new_emails_trims_sender_whitespace() {
        // Defensive: some providers leave whitespace around the address.
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        seed_email(&db, "e1", "acc1", "  me@mine.com  ", 1000);
        seed_tag(&db, "e1", "company", "acme");

        update_from_new_emails(&db, "acc1", "me@mine.com", &["e1".into()]).unwrap();
        let (sent, recv, _) = get_row(&db, "acc1", "acme");
        assert_eq!(sent, 1);
        assert_eq!(recv, 0);
    }

    // ── backfill_account ────────────────────────────────────────────────

    #[test]
    fn backfill_account_aggregates_existing_tags() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        // 3 outbound to acme, 2 inbound from acme, 1 inbound from beta.
        for (i, (sender, tag, ts)) in [
            ("me@mine.com", "acme", 1000i64),
            ("me@mine.com", "acme", 1100),
            ("me@mine.com", "acme", 1200),
            ("alice@acme.com", "acme", 900),
            ("bob@acme.com", "acme", 1500),
            ("carol@beta.com", "beta", 800),
        ]
        .iter()
        .enumerate()
        {
            let id = format!("e{i}");
            seed_email(&db, &id, "acc1", sender, *ts);
            seed_tag(&db, &id, "company", tag);
        }

        let n = backfill_account(&db, "acc1").unwrap();
        assert_eq!(n, 2, "two distinct company tags should produce two rows");

        let (sent, recv, last) = get_row(&db, "acc1", "acme");
        assert_eq!(sent, 3);
        assert_eq!(recv, 2);
        assert_eq!(last, Some(1500));

        let (sent, recv, last) = get_row(&db, "acc1", "beta");
        assert_eq!(sent, 0);
        assert_eq!(recv, 1);
        assert_eq!(last, Some(800));
    }

    #[test]
    fn backfill_account_is_idempotent() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        seed_email(&db, "e1", "acc1", "me@mine.com", 1000);
        seed_tag(&db, "e1", "company", "acme");

        let first = backfill_account(&db, "acc1").unwrap();
        assert_eq!(first, 1);
        // Second call must not double-count — the account already has rows.
        let second = backfill_account(&db, "acc1").unwrap();
        assert_eq!(second, 0);

        let (sent, recv, _) = get_row(&db, "acc1", "acme");
        assert_eq!(sent, 1);
        assert_eq!(recv, 0);
    }

    #[test]
    fn backfill_account_isolates_accounts() {
        // Two accounts, both tagging "acme". The backfill for acc1 must not
        // include acc2's emails.
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        seed_account(&db, "acc2", "other@other.com");
        seed_email(&db, "e1", "acc1", "me@mine.com", 1000);
        seed_tag(&db, "e1", "company", "acme");
        seed_email(&db, "e2", "acc2", "other@other.com", 2000);
        seed_tag(&db, "e2", "company", "acme");

        backfill_account(&db, "acc1").unwrap();
        let (sent, _, last) = get_row(&db, "acc1", "acme");
        assert_eq!(sent, 1);
        assert_eq!(last, Some(1000), "acc2's email must not bleed into acc1");

        // acc2 still untouched.
        let conn = db.reader();
        let cnt: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tag_priority WHERE account_id = 'acc2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cnt, 0);
    }

    #[test]
    fn backfill_account_trims_whitespace_in_sender() {
        // Regression: backfill's SQL and update_from_new_emails's Rust must
        // agree on outbound detection even with stray whitespace.
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        seed_email(&db, "e1", "acc1", "  me@mine.com  ", 1000);
        seed_tag(&db, "e1", "company", "acme");

        backfill_account(&db, "acc1").unwrap();
        let (sent, recv, _) = get_row(&db, "acc1", "acme");
        assert_eq!(sent, 1, "whitespace must not flip sent→received");
        assert_eq!(recv, 0);
    }

    #[test]
    fn backfill_account_case_insensitive_outbound() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "Me@Mine.com");
        seed_email(&db, "e1", "acc1", "me@mine.com", 1000);
        seed_tag(&db, "e1", "company", "acme");

        backfill_account(&db, "acc1").unwrap();
        let (sent, recv, _) = get_row(&db, "acc1", "acme");
        assert_eq!(sent, 1);
        assert_eq!(recv, 0);
    }

    #[test]
    fn backfill_and_incremental_agree_on_same_data() {
        // Regression: the SQL backfill and the Rust incremental path must
        // produce identical counts when given the same emails.
        let make_db = |tag_fn: &dyn Fn(&Arc<Database>)| {
            let db = Arc::new(Database::new_for_testing().unwrap());
            seed_account(&db, "acc1", "me@mine.com");
            seed_email(&db, "e1", "acc1", "me@mine.com", 1000);
            seed_tag(&db, "e1", "company", "acme");
            seed_email(&db, "e2", "acc1", "ALICE@acme.com", 1500);
            seed_tag(&db, "e2", "company", "acme");
            seed_email(&db, "e3", "acc1", " bob@acme.com ", 2000);
            seed_tag(&db, "e3", "company", "acme");
            tag_fn(&db);
            db
        };

        let a = make_db(&|db| {
            backfill_account(db, "acc1").unwrap();
        });
        let b = make_db(&|db| {
            update_from_new_emails(db, "acc1", "me@mine.com", &["e1".into(), "e2".into(), "e3".into()]).unwrap();
        });

        assert_eq!(get_row(&a, "acc1", "acme"), get_row(&b, "acc1", "acme"));
    }

    #[test]
    fn backfill_account_skips_deleted_emails() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        seed_email(&db, "e1", "acc1", "me@mine.com", 1000);
        seed_tag(&db, "e1", "company", "acme");
        // Mark e1 deleted — it must not count toward priorities.
        db.connection()
            .execute("UPDATE emails SET is_deleted = 1 WHERE id = 'e1'", [])
            .unwrap();
        seed_email(&db, "e2", "acc1", "alice@acme.com", 2000);
        seed_tag(&db, "e2", "company", "acme");

        backfill_account(&db, "acc1").unwrap();
        let (sent, recv, last) = get_row(&db, "acc1", "acme");
        assert_eq!(sent, 0, "deleted outbound must be excluded");
        assert_eq!(recv, 1);
        assert_eq!(last, Some(2000));
    }

    // ── score / recency edge cases ──────────────────────────────────────

    #[test]
    fn get_priorities_applies_continuous_decay() {
        // Pin counts to 0/0 so the gate is at the floor (0.3) and recency
        // dominates the score. Verifies the hyperbolic decay
        //   raw = 50/(1 + age_days/30)
        //   score = raw * 0.3        (engagement gate when ratio=0)
        // is monotonically decreasing with age, smooth (not stepped), and
        // hits 0 for NULL last_activity_at.
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        let now = chrono::Utc::now().timestamp();
        let day = 86_400i64;
        db.connection()
            .execute_batch(&format!(
                "INSERT INTO tag_priority VALUES
               ('acc1','company','d3',   0, 0, {d3},   {now}),
               ('acc1','company','d20',  0, 0, {d20},  {now}),
               ('acc1','company','d60',  0, 0, {d60},  {now}),
               ('acc1','company','d200', 0, 0, {d200}, {now}),
               ('acc1','company','never',0, 0, NULL,   {now});",
                now = now,
                d3 = now - 3 * day,
                d20 = now - 20 * day,
                d60 = now - 60 * day,
                d200 = now - 200 * day,
            ))
            .unwrap();

        let rows = get_priorities(&db, "acc1", "company", 10).unwrap();
        let score = |tag: &str| rows.iter().find(|r| r.tag_value == tag).unwrap().priority_score;

        // Expected: raw * 0.3 for ratio=0.
        //   d3:   50/(1+3/30)   *0.3 ≈ 13.636
        //   d20:  50/(1+20/30)  *0.3 ≈  9.000
        //   d60:  50/(1+60/30)  *0.3 ≈  5.000
        //   d200: 50/(1+200/30) *0.3 ≈  1.957
        let approx = |actual: f64, expected: f64| {
            assert!((actual - expected).abs() < 0.05, "expected ≈ {expected}, got {actual}");
        };
        approx(score("d3"), 13.636);
        approx(score("d20"), 9.000);
        approx(score("d60"), 5.000);
        approx(score("d200"), 1.957);
        assert_eq!(score("never"), 0.0);

        // Monotone: older → smaller score.
        assert!(score("d3") > score("d20"));
        assert!(score("d20") > score("d60"));
        assert!(score("d60") > score("d200"));
        assert!(score("d200") > score("never"));
    }

    #[test]
    fn get_priorities_engagement_decays_with_age() {
        // Regression: a perfectly balanced but very old exchange must not
        // outrank a recent two-way conversation.
        //
        // Before the engagement-decay fix, neo-metrics (4/4 from 2007) scored
        // engagement=16 forever because that term had no age component — it
        // was timeless. Now engagement decays at a 1-year half-life so an
        // 18-year-old 4-email exchange becomes ≈ 0.85.
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        let now = chrono::Utc::now().timestamp();
        let day = 86_400i64;
        db.connection()
            .execute_batch(&format!(
                "INSERT INTO tag_priority VALUES
               ('acc1','company','ancient_balanced', 4, 4, {ancient}, {now}),
               ('acc1','company','recent_balanced',  4, 4, {recent},  {now});",
                now = now,
                ancient = now - 6700 * day, // ~18 years
                recent = now - 3 * day,
            ))
            .unwrap();

        let rows = get_priorities(&db, "acc1", "company", 10).unwrap();
        let score = |tag: &str| rows.iter().find(|r| r.tag_value == tag).unwrap().priority_score;
        // ancient: eng=16 * 1/(1+6700/365) ≈ 16*0.052 ≈ 0.83
        //          rec=50/(1+6700/30)*1 ≈ 0.22 → total ≈ 1.1
        // recent:  eng=16 * 1/(1+3/365)  ≈ 15.87
        //          rec=50/(1+3/30)*1 ≈ 45.45 → total ≈ 61.3
        assert!(
            score("recent_balanced") > 50.0 * score("ancient_balanced"),
            "recent balanced ({}) must vastly outrank ancient balanced ({})",
            score("recent_balanced"),
            score("ancient_balanced"),
        );
        assert!(
            score("ancient_balanced") < 2.0,
            "ancient 4/4 balance must score near zero, got {}",
            score("ancient_balanced"),
        );
    }

    #[test]
    fn get_priorities_engagement_gate_dampens_one_way_recency() {
        // Same recency, three traffic patterns:
        //   newsletter:   one-way inbound  (0/3) → mutual=0
        //   broadcast:    one-way outbound (3/0) → mutual=0 (kindle-style)
        //   conversation: two-way          (3/3) → mutual=1
        //
        // The two-way conversation gets the full recency boost; both one-way
        // patterns get the engagement-floor share (~30 %). Direction doesn't
        // matter — what matters is that the other party engages back.
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        let now = chrono::Utc::now().timestamp();
        let day = 86_400i64;
        db.connection()
            .execute_batch(&format!(
                "INSERT INTO tag_priority VALUES
               ('acc1','company','newsletter',   0, 3, {ts}, {now}),
               ('acc1','company','broadcast',    3, 0, {ts}, {now}),
               ('acc1','company','conversation', 3, 3, {ts}, {now});",
                now = now,
                ts = now - 3 * day,
            ))
            .unwrap();

        let rows = get_priorities(&db, "acc1", "company", 10).unwrap();
        let score = |tag: &str| rows.iter().find(|r| r.tag_value == tag).unwrap().priority_score;

        // conversation:  eng=2*6*1.0=12,  rec≈45.45*1.0=45.45  → ≈57.5
        // newsletter:    eng=0,           rec≈45.45*0.3=13.64  → ≈13.6
        // broadcast:     eng=0,           rec≈45.45*0.3=13.64  → ≈13.6  (same as newsletter)
        assert!(score("conversation") > 3.0 * score("newsletter"));
        assert!(
            (score("newsletter") - score("broadcast")).abs() < 0.01,
            "kindle-style one-way outbound must score the same as a one-way newsletter"
        );
    }

    #[test]
    fn get_priorities_mutual_ratio_zeros_one_way_traffic() {
        // Two-way scoring: any one-way pattern (in either direction) gets
        // engagement=0. Direction doesn't matter — only mutual exchange does.
        // Regression: previously `1 sent / 0 recv` (kindle-style) scored full
        // engagement because reply_ratio was sent/total.
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        let now = chrono::Utc::now().timestamp();
        db.connection()
            .execute_batch(&format!(
                "INSERT INTO tag_priority VALUES
               ('acc1','company','only_sent',  148, 0, NULL, {now}),
               ('acc1','company','only_recv',    0, 3, NULL, {now}),
               ('acc1','company','two_way',      3, 3, NULL, {now});",
                now = now,
            ))
            .unwrap();

        let rows = get_priorities(&db, "acc1", "company", 10).unwrap();
        let score = |tag: &str| rows.iter().find(|r| r.tag_value == tag).unwrap().priority_score;
        assert_eq!(score("only_sent"), 0.0, "kindle-style one-way must score 0");
        assert_eq!(score("only_recv"), 0.0, "newsletter-style one-way must score 0");
        assert!(score("two_way") > 0.0);
    }

    #[test]
    fn get_priorities_respects_limit_and_scope() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");
        seed_account(&db, "acc2", "other@other.com");
        let now = chrono::Utc::now().timestamp();
        // All outbound (ratio=1) so each gets a positive engagement score,
        // ordered by volume up to the saturation cap.
        db.connection()
            .execute_batch(&format!(
                "INSERT INTO tag_priority VALUES
               ('acc1','company','a', 5, 0, NULL, {now}),
               ('acc1','company','b', 3, 0, NULL, {now}),
               ('acc1','company','c', 1, 0, NULL, {now}),
               ('acc2','company','a', 9, 0, NULL, {now});",
                now = now,
            ))
            .unwrap();

        // limit caps results.
        let rows = get_priorities(&db, "acc1", "company", 2).unwrap();
        assert_eq!(rows.len(), 2);
        // Scoped to acc1 — acc2's higher-ranked 'a' must not leak in.
        assert!(rows.iter().all(|r| r.tag_value != "a" || r.sent_count == 5));
    }

    #[test]
    fn update_from_new_emails_touches_only_given_ids() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        seed_account(&db, "acc1", "me@mine.com");

        // Seed 5 emails all tagged 'acme', but only call with 2 of their IDs.
        for i in 0..5 {
            let id = format!("e{i}");
            seed_email(&db, &id, "acc1", "me@mine.com", 1000 + i as i64);
            seed_tag(&db, &id, "company", "acme");
        }

        update_from_new_emails(&db, "acc1", "me@mine.com", &["e0".into(), "e1".into()]).unwrap();

        let (sent, recv, _) = get_row(&db, "acc1", "acme");
        // Only 2 of the 5 emails counted — regression guard for the
        // "no full recalculation" requirement.
        assert_eq!(sent, 2);
        assert_eq!(recv, 0);
    }
}
