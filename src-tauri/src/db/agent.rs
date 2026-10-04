//! The email agent's tables (V035): rules, runs with their actions, and the
//! stats panels with the emails each one matched.
//!
//! Action status changes are guarded like the outbox's: each UPDATE names the
//! status it starts from, so approving twice (two clicks, two windows) runs an
//! action once.

use rusqlite::{params, OptionalExtension, Row};

use crate::db::Database;
use crate::models::agent::{
    AgentAction, AgentActionKind, AgentActionStatus, AgentPanel, AgentRule, AgentRun, AgentRunStatus, AgentTrigger,
    PanelWindow, ReviewOutcome,
};
use crate::models::error::Result;

fn bad_column(column: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("unknown agent {column} {value:?}").into(),
    )
}

const RULE_COLUMNS: &str = "id, name, trigger_kind, account_id, match_prompt, action_prompt, always_approve, enabled, \
     created_at, updated_at";

fn rule_from_row(row: &Row<'_>) -> rusqlite::Result<AgentRule> {
    let trigger: String = row.get(2)?;
    Ok(AgentRule {
        id: row.get(0)?,
        name: row.get(1)?,
        trigger: AgentTrigger::parse(&trigger).ok_or_else(|| bad_column("trigger", &trigger))?,
        account_id: row.get(3)?,
        match_prompt: row.get(4)?,
        action_prompt: row.get(5)?,
        always_approve: row.get(6)?,
        enabled: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

/// A reply draft of the agent the user has not sent or discarded, and that
/// still exists (one sent or deleted from the composer has nothing to review).
macro_rules! needs_review_sql {
    () => {
        "(a.kind = 'draft_reply' AND a.status = 'done' AND a.review_outcome IS NULL \
         AND EXISTS (SELECT 1 FROM drafts d WHERE d.id = a.result))"
    };
}
const NEEDS_REVIEW: &str = needs_review_sql!();

const ACTION_COLUMNS: &str = concat!(
    "a.id, a.run_id, a.rule_id, a.rule_name, a.kind, a.detail, a.status, a.requires_approval, \
     a.result, a.error, a.created_at, a.decided_at, r.title, a.review_outcome, a.reviewed_at, ",
    needs_review_sql!(),
    " AS needs_review"
);

fn action_from_row(row: &Row<'_>) -> rusqlite::Result<AgentAction> {
    let kind: String = row.get(4)?;
    let status: String = row.get(6)?;
    Ok(AgentAction {
        id: row.get(0)?,
        run_id: row.get(1)?,
        rule_id: row.get(2)?,
        rule_name: row.get(3)?,
        kind: AgentActionKind::parse(&kind).ok_or_else(|| bad_column("action kind", &kind))?,
        detail: row.get(5)?,
        status: AgentActionStatus::parse(&status).ok_or_else(|| bad_column("action status", &status))?,
        requires_approval: row.get(7)?,
        result: row.get(8)?,
        error: row.get(9)?,
        created_at: row.get(10)?,
        decided_at: row.get(11)?,
        run_title: row.get(12)?,
        review_outcome: row
            .get::<_, Option<String>>(13)?
            .map(|v| ReviewOutcome::parse(&v).ok_or_else(|| bad_column("review outcome", &v)))
            .transpose()?,
        reviewed_at: row.get(14)?,
        needs_review: row.get(15)?,
    })
}

const RUN_COLUMNS: &str =
    "id, account_id, trigger_kind, trigger_ref, title, sender, status, summary, error, created_at";

fn run_from_row(row: &Row<'_>) -> rusqlite::Result<AgentRun> {
    let trigger: String = row.get(2)?;
    let status: String = row.get(6)?;
    Ok(AgentRun {
        id: row.get(0)?,
        account_id: row.get(1)?,
        trigger: AgentTrigger::parse(&trigger).ok_or_else(|| bad_column("trigger", &trigger))?,
        trigger_ref: row.get(3)?,
        title: row.get(4)?,
        sender: row.get(5)?,
        status: AgentRunStatus::parse(&status).ok_or_else(|| bad_column("run status", &status))?,
        summary: row.get(7)?,
        error: row.get(8)?,
        created_at: row.get(9)?,
        actions: Vec::new(),
    })
}

fn panel_from_row(row: &Row<'_>) -> rusqlite::Result<AgentPanel> {
    let window: String = row.get(3)?;
    Ok(AgentPanel {
        id: row.get(0)?,
        title: row.get(1)?,
        prompt: row.get(2)?,
        window: PanelWindow::parse(&window).ok_or_else(|| bad_column("panel window", &window))?,
        created_at: row.get(4)?,
        count: 0,
    })
}

impl Database {
    // ── Rules ───────────────────────────────────────────────────────────────

    pub fn insert_agent_rule(&self, rule: &AgentRule) -> Result<()> {
        self.connection().execute(
            &format!("INSERT INTO agent_rules ({RULE_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"),
            params![
                rule.id,
                rule.name,
                rule.trigger.as_str(),
                rule.account_id,
                rule.match_prompt,
                rule.action_prompt,
                rule.always_approve,
                rule.enabled,
                rule.created_at,
                rule.updated_at
            ],
        )?;
        Ok(())
    }

    /// Replace a rule's editable fields. `false` when it does not exist.
    pub fn update_agent_rule(&self, rule: &AgentRule) -> Result<bool> {
        let n = self.connection().execute(
            "UPDATE agent_rules SET name = ?2, trigger_kind = ?3, account_id = ?4, match_prompt = ?5,
                    action_prompt = ?6, always_approve = ?7, enabled = ?8, updated_at = ?9
             WHERE id = ?1",
            params![
                rule.id,
                rule.name,
                rule.trigger.as_str(),
                rule.account_id,
                rule.match_prompt,
                rule.action_prompt,
                rule.always_approve,
                rule.enabled,
                rule.updated_at
            ],
        )?;
        Ok(n > 0)
    }

    pub fn delete_agent_rule(&self, id: &str) -> Result<bool> {
        let n = self
            .connection()
            .execute("DELETE FROM agent_rules WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    pub fn get_agent_rule(&self, id: &str) -> Result<Option<AgentRule>> {
        let rule = self
            .reader()
            .query_row(
                &format!("SELECT {RULE_COLUMNS} FROM agent_rules WHERE id = ?1"),
                params![id],
                rule_from_row,
            )
            .optional()?;
        Ok(rule)
    }

    /// Every rule, oldest first — the order the user created them in.
    pub fn list_agent_rules(&self) -> Result<Vec<AgentRule>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT {RULE_COLUMNS} FROM agent_rules ORDER BY created_at, id"
        ))?;
        let rules = stmt
            .query_map([], rule_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rules)
    }

    // ── Runs and actions ────────────────────────────────────────────────────

    /// Store a run with its actions in one transaction. `false` when the
    /// trigger already has a run — it was evaluated before.
    pub fn insert_agent_run(&self, run: &AgentRun) -> Result<bool> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let inserted = tx.execute(
            &format!(
                "INSERT OR IGNORE INTO agent_runs ({RUN_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            ),
            params![
                run.id,
                run.account_id,
                run.trigger.as_str(),
                run.trigger_ref,
                run.title,
                run.sender,
                run.status.as_str(),
                run.summary,
                run.error,
                run.created_at
            ],
        )?;
        if inserted == 0 {
            return Ok(false);
        }
        for a in &run.actions {
            tx.execute(
                "INSERT INTO agent_actions (id, run_id, rule_id, rule_name, kind, detail, status, requires_approval,
                                            result, error, created_at, decided_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    a.id,
                    run.id,
                    a.rule_id,
                    a.rule_name,
                    a.kind.as_str(),
                    a.detail,
                    a.status.as_str(),
                    a.requires_approval,
                    a.result,
                    a.error,
                    a.created_at,
                    a.decided_at
                ],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn agent_run_exists(&self, trigger: AgentTrigger, trigger_ref: &str) -> Result<bool> {
        let found = self
            .reader()
            .query_row(
                "SELECT 1 FROM agent_runs WHERE trigger_kind = ?1 AND trigger_ref = ?2",
                params![trigger.as_str(), trigger_ref],
                |_| Ok(()),
            )
            .optional()?;
        Ok(found.is_some())
    }

    pub fn get_agent_run(&self, id: &str) -> Result<Option<AgentRun>> {
        let run = self
            .reader()
            .query_row(
                &format!("SELECT {RUN_COLUMNS} FROM agent_runs WHERE id = ?1"),
                params![id],
                run_from_row,
            )
            .optional()?;
        Ok(run)
    }

    /// The feed: runs that matched a rule (or failed), newest first, each
    /// with its actions.
    pub fn list_agent_feed(&self, limit: i64) -> Result<Vec<AgentRun>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT {RUN_COLUMNS} FROM agent_runs WHERE status <> 'no_match'
             ORDER BY created_at DESC, rowid DESC LIMIT ?1"
        ))?;
        let mut runs = stmt
            .query_map(params![limit], run_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut actions = conn.prepare(&format!(
            "SELECT {ACTION_COLUMNS} FROM agent_actions a JOIN agent_runs r ON r.id = a.run_id
             WHERE a.run_id = ?1 ORDER BY a.created_at, a.rowid"
        ))?;
        for run in &mut runs {
            run.actions = actions
                .query_map(params![run.id], action_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }
        Ok(runs)
    }

    /// The side panel: every pending action (oldest first, the queue to
    /// review), then the latest decided ones.
    pub fn list_agent_actions(&self, recent_limit: i64) -> Result<Vec<AgentAction>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT * FROM (
                 SELECT {ACTION_COLUMNS}, 0 AS grp, a.created_at AS sort_key
                 FROM agent_actions a JOIN agent_runs r ON r.id = a.run_id
                 WHERE a.status = 'pending' OR {NEEDS_REVIEW}
             )
             UNION ALL
             SELECT * FROM (
                 SELECT {ACTION_COLUMNS}, 1 AS grp, -COALESCE(a.decided_at, a.created_at) AS sort_key
                 FROM agent_actions a JOIN agent_runs r ON r.id = a.run_id
                 WHERE a.status <> 'pending' AND NOT {NEEDS_REVIEW}
                 ORDER BY sort_key, a.id DESC LIMIT ?1
             )
             ORDER BY grp, sort_key, id"
        ))?;
        let actions = stmt
            .query_map(params![recent_limit], action_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(actions)
    }

    pub fn get_agent_action(&self, id: &str) -> Result<Option<AgentAction>> {
        let action = self
            .reader()
            .query_row(
                &format!(
                    "SELECT {ACTION_COLUMNS} FROM agent_actions a JOIN agent_runs r ON r.id = a.run_id WHERE a.id = ?1"
                ),
                params![id],
                action_from_row,
            )
            .optional()?;
        Ok(action)
    }

    /// `pending → done`, before the approved action runs, so a second
    /// approval loses. `false` when the action is no longer pending.
    pub fn claim_agent_action(&self, id: &str, now: i64) -> Result<bool> {
        let n = self.connection().execute(
            "UPDATE agent_actions SET status = 'done', decided_at = ?2 WHERE id = ?1 AND status = 'pending'",
            params![id, now],
        )?;
        Ok(n > 0)
    }

    /// Record what the user did with a reply draft of the agent. `false`
    /// when the action is not a done draft, or was reviewed already.
    pub fn review_agent_draft(&self, id: &str, outcome: ReviewOutcome, now: i64) -> Result<bool> {
        let n = self.connection().execute(
            "UPDATE agent_actions SET review_outcome = ?2, reviewed_at = ?3
             WHERE id = ?1 AND kind = 'draft_reply' AND status = 'done' AND review_outcome IS NULL",
            params![id, outcome.as_str(), now],
        )?;
        Ok(n > 0)
    }

    /// `pending → rejected`.
    pub fn reject_agent_action(&self, id: &str, now: i64) -> Result<bool> {
        let n = self.connection().execute(
            "UPDATE agent_actions SET status = 'rejected', decided_at = ?2 WHERE id = ?1 AND status = 'pending'",
            params![id, now],
        )?;
        Ok(n > 0)
    }

    /// Record what an approved action produced, or why it failed.
    pub fn finish_agent_action(&self, id: &str, outcome: std::result::Result<Option<&str>, &str>) -> Result<()> {
        let (status, result, error) = match outcome {
            Ok(result) => ("done", result, None),
            Err(error) => ("failed", None, Some(error)),
        };
        self.connection().execute(
            "UPDATE agent_actions SET status = ?2, result = ?3, error = ?4 WHERE id = ?1",
            params![id, status, result, error],
        )?;
        Ok(())
    }

    /// Emails of `account_id` the agent has not looked at: inbox mail from
    /// others that arrived at or after `since`, not junk, oldest first.
    pub fn agent_candidate_emails(&self, account_id: &str, since: i64, limit: i64) -> Result<Vec<String>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT e.id FROM emails e
             WHERE e.account_id = ?1 AND e.is_deleted = 0 AND e.mailbox = 'inbox' AND e.is_sent = 0
               AND e.timestamp >= ?2
               AND NOT EXISTS (SELECT 1 FROM agent_runs r WHERE r.trigger_kind = 'email' AND r.trigger_ref = e.id)
               {junk}
             ORDER BY e.timestamp, e.id LIMIT ?3",
            junk = crate::db::exclude_junk_sql("e", false)
        ))?;
        let ids = stmt
            .query_map(params![account_id, since, limit], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    /// The latest inbox or sent messages of `account_id` exchanged with
    /// `addresses` (as sender, or among the recipients), newest first:
    /// `(timestamp, sender_email, subject)`.
    pub fn agent_recent_with(
        &self,
        account_id: &str,
        addresses: &[&str],
        limit: i64,
    ) -> Result<Vec<(i64, String, String)>> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.to_string()), Box::new(limit)];
        let mut matches = Vec::new();
        for address in addresses {
            let address = address.trim().to_lowercase();
            let escaped = address.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            args.push(Box::new(address));
            let exact = args.len();
            args.push(Box::new(format!("%{escaped}%")));
            let like = args.len();
            matches.push(format!(
                "lower(e.sender_email) = ?{exact} OR lower(e.recipients_json) LIKE ?{like} ESCAPE '\\'"
            ));
        }
        let sql = format!(
            "SELECT e.timestamp, e.sender_email, e.subject FROM emails e
             WHERE e.account_id = ?1 AND e.is_deleted = 0 AND e.mailbox IN ('inbox', 'sent') AND ({})
             ORDER BY e.timestamp DESC, e.id LIMIT ?2",
            matches.join(" OR ")
        );
        let conn = self.reader();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())), |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ── Panels ──────────────────────────────────────────────────────────────

    pub fn insert_agent_panel(&self, panel: &AgentPanel) -> Result<()> {
        self.connection().execute(
            "INSERT INTO agent_panels (id, title, prompt, time_window, sort_order, created_at)
             VALUES (?1, ?2, ?3, ?4, (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM agent_panels), ?5)",
            params![
                panel.id,
                panel.title,
                panel.prompt,
                panel.window.as_str(),
                panel.created_at
            ],
        )?;
        Ok(())
    }

    /// Change a panel. A new prompt drops the emails the old one matched.
    pub fn update_agent_panel(&self, panel: &AgentPanel) -> Result<bool> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let old_prompt: Option<String> = tx
            .query_row(
                "SELECT prompt FROM agent_panels WHERE id = ?1",
                params![panel.id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(old_prompt) = old_prompt else {
            return Ok(false);
        };
        tx.execute(
            "UPDATE agent_panels SET title = ?2, prompt = ?3, time_window = ?4 WHERE id = ?1",
            params![panel.id, panel.title, panel.prompt, panel.window.as_str()],
        )?;
        if old_prompt != panel.prompt {
            tx.execute("DELETE FROM agent_panel_hits WHERE panel_id = ?1", params![panel.id])?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn delete_agent_panel(&self, id: &str) -> Result<bool> {
        let n = self
            .connection()
            .execute("DELETE FROM agent_panels WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    pub fn get_agent_panel(&self, id: &str) -> Result<Option<AgentPanel>> {
        let panel = self
            .reader()
            .query_row(
                "SELECT id, title, prompt, time_window, created_at FROM agent_panels WHERE id = ?1",
                params![id],
                panel_from_row,
            )
            .optional()?;
        Ok(panel)
    }

    /// Every panel in display order, `count` left at 0.
    pub fn list_agent_panels(&self) -> Result<Vec<AgentPanel>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, title, prompt, time_window, created_at FROM agent_panels ORDER BY sort_order, created_at",
        )?;
        let panels = stmt
            .query_map([], panel_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(panels)
    }

    pub fn add_agent_panel_hit(&self, panel_id: &str, email_id: &str, email_timestamp: i64) -> Result<()> {
        self.connection().execute(
            "INSERT OR IGNORE INTO agent_panel_hits (panel_id, email_id, email_timestamp) VALUES (?1, ?2, ?3)",
            params![panel_id, email_id, email_timestamp],
        )?;
        Ok(())
    }

    /// Emails a panel matched that arrived at or after `since`.
    pub fn count_agent_panel_hits(&self, panel_id: &str, since: i64) -> Result<i64> {
        let n = self.reader().query_row(
            "SELECT COUNT(*) FROM agent_panel_hits h JOIN emails e ON e.id = h.email_id
             WHERE h.panel_id = ?1 AND h.email_timestamp >= ?2 AND e.is_deleted = 0",
            params![panel_id, since],
            |row| row.get(0),
        )?;
        Ok(n)
    }

    /// Inbox emails of the enabled accounts that arrived at or after
    /// `since`, newest first — what a new panel counts over its window.
    pub fn agent_panel_window_emails(&self, since: i64, limit: i64) -> Result<Vec<String>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT e.id FROM emails e JOIN accounts a ON a.id = e.account_id
             WHERE a.enabled = 1 AND e.is_deleted = 0 AND e.mailbox = 'inbox' AND e.is_sent = 0
               AND e.timestamp >= ?1
               {junk}
             ORDER BY e.timestamp DESC, e.id LIMIT ?2",
            junk = crate::db::exclude_junk_sql("e", false)
        ))?;
        let ids = stmt
            .query_map(params![since, limit], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn db() -> Database {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db
    }

    fn seed_email(db: &Database, id: &str, timestamp: i64, mailbox: &str, is_sent: bool) {
        db.connection()
            .execute(
                "INSERT INTO emails (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                                     recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox,
                                     is_sent, created_at)
                 VALUES (?1, 'acc-1', ?1, 'Subject', 'Ana', 'ana@example.com', 'example.com', '[]', '[]', '',
                         ?2, 0, 'primary', ?3, ?4, 0)",
                params![id, timestamp, mailbox, is_sent],
            )
            .unwrap();
    }

    fn rule(id: &str, created_at: i64) -> AgentRule {
        AgentRule {
            id: id.into(),
            name: format!("Rule {id}"),
            trigger: AgentTrigger::Email,
            account_id: None,
            match_prompt: "support requests".into(),
            action_prompt: "draft a reply".into(),
            always_approve: false,
            enabled: true,
            created_at,
            updated_at: created_at,
        }
    }

    fn action(id: &str, kind: AgentActionKind, status: AgentActionStatus, created_at: i64) -> AgentAction {
        AgentAction {
            id: id.into(),
            run_id: String::new(),
            rule_id: None,
            rule_name: "Rule r1".into(),
            kind,
            detail: String::new(),
            status,
            requires_approval: status == AgentActionStatus::Pending,
            result: None,
            error: None,
            created_at,
            decided_at: None,
            run_title: String::new(),
            review_outcome: None,
            reviewed_at: None,
            needs_review: false,
        }
    }

    fn run(
        id: &str,
        trigger_ref: &str,
        status: AgentRunStatus,
        created_at: i64,
        actions: Vec<AgentAction>,
    ) -> AgentRun {
        AgentRun {
            id: id.into(),
            account_id: "acc-1".into(),
            trigger: AgentTrigger::Email,
            trigger_ref: trigger_ref.into(),
            title: format!("Title {id}"),
            sender: "Ana".into(),
            status,
            summary: "A support request".into(),
            error: None,
            created_at,
            actions,
        }
    }

    #[test]
    fn a_rule_round_trips_updates_and_deletes() {
        let db = db();
        db.insert_agent_rule(&rule("r2", NOW + 1)).unwrap();
        db.insert_agent_rule(&rule("r1", NOW)).unwrap();
        let ids: Vec<_> = db.list_agent_rules().unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["r1", "r2"], "oldest first");

        let mut changed = rule("r1", NOW);
        changed.always_approve = true;
        changed.enabled = false;
        changed.trigger = AgentTrigger::Event;
        changed.account_id = Some("acc-1".into());
        assert!(db.update_agent_rule(&changed).unwrap());
        assert_eq!(db.get_agent_rule("r1").unwrap().unwrap(), changed);

        assert!(db.delete_agent_rule("r1").unwrap());
        assert!(!db.delete_agent_rule("r1").unwrap());
        assert!(!db.update_agent_rule(&changed).unwrap());
    }

    #[test]
    fn a_trigger_gets_one_run_only() {
        let db = db();
        let first = run("run-1", "e-1", AgentRunStatus::NoMatch, NOW, vec![]);
        assert!(db.insert_agent_run(&first).unwrap());
        assert!(db.agent_run_exists(AgentTrigger::Email, "e-1").unwrap());
        assert!(!db.agent_run_exists(AgentTrigger::Event, "e-1").unwrap());
        let again = run(
            "run-2",
            "e-1",
            AgentRunStatus::Matched,
            NOW,
            vec![action("a1", AgentActionKind::Star, AgentActionStatus::Pending, NOW)],
        );
        assert!(!db.insert_agent_run(&again).unwrap(), "already evaluated");
        assert!(db.get_agent_action("a1").unwrap().is_none(), "the loser stores nothing");
    }

    #[test]
    fn runs_of_the_same_second_keep_the_order_they_were_made_in() {
        let db = db();
        // Ids sort the other way round from the order the runs were made in.
        for id in ["zz-first", "mm-second", "aa-third"] {
            db.insert_agent_run(&run(id, &format!("e-{id}"), AgentRunStatus::Matched, NOW, vec![]))
                .unwrap();
        }
        let ids: Vec<_> = db.list_agent_feed(10).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["aa-third", "mm-second", "zz-first"], "newest first");
    }

    #[test]
    fn the_feed_shows_matched_and_failed_runs_newest_first_with_their_actions() {
        let db = db();
        db.insert_agent_run(&run("quiet", "e-1", AgentRunStatus::NoMatch, NOW + 3, vec![]))
            .unwrap();
        db.insert_agent_run(&run(
            "old",
            "e-2",
            AgentRunStatus::Matched,
            NOW,
            vec![
                action("a1", AgentActionKind::DraftReply, AgentActionStatus::Done, NOW),
                action("a2", AgentActionKind::Archive, AgentActionStatus::Pending, NOW + 1),
            ],
        ))
        .unwrap();
        db.insert_agent_run(&run("broken", "e-3", AgentRunStatus::Failed, NOW + 2, vec![]))
            .unwrap();
        let feed = db.list_agent_feed(10).unwrap();
        let ids: Vec<_> = feed.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["broken", "old"]);
        let kinds: Vec<_> = feed[1].actions.iter().map(|a| a.kind).collect();
        assert_eq!(kinds, vec![AgentActionKind::DraftReply, AgentActionKind::Archive]);
        assert_eq!(feed[1].actions[0].run_id, "old");
        assert_eq!(feed[1].actions[0].run_title, "Title old");
        assert_eq!(db.list_agent_feed(1).unwrap().len(), 1);
    }

    fn seed_draft(db: &Database, id: &str) {
        db.connection()
            .execute(
                "INSERT INTO drafts (id, account_id, to_addresses_json, subject, body, created_at, updated_at)
                 VALUES (?1, 'acc-1', '[]', 'Re: Subject', 'Thanks', 0, 0)",
                params![id],
            )
            .unwrap();
    }

    fn draft_action(id: &str, draft_id: &str) -> AgentAction {
        let mut a = action(id, AgentActionKind::DraftReply, AgentActionStatus::Done, NOW);
        a.result = Some(draft_id.into());
        a
    }

    #[test]
    fn a_done_draft_waits_for_review_while_the_draft_exists() {
        let db = db();
        seed_draft(&db, "d-kept");
        db.insert_agent_run(&run(
            "run",
            "e-1",
            AgentRunStatus::Matched,
            NOW,
            vec![draft_action("kept", "d-kept"), draft_action("gone", "d-gone")],
        ))
        .unwrap();
        assert!(db.get_agent_action("kept").unwrap().unwrap().needs_review);
        assert!(
            !db.get_agent_action("gone").unwrap().unwrap().needs_review,
            "a draft sent or deleted elsewhere has nothing left to review"
        );
    }

    #[test]
    fn a_reviewed_draft_records_its_outcome_once() {
        let db = db();
        seed_draft(&db, "d1");
        db.insert_agent_run(&run(
            "run",
            "e-1",
            AgentRunStatus::Matched,
            NOW,
            vec![
                draft_action("draft", "d1"),
                action("task", AgentActionKind::CreateTask, AgentActionStatus::Done, NOW),
            ],
        ))
        .unwrap();
        assert!(db.review_agent_draft("draft", ReviewOutcome::Sent, NOW + 3).unwrap());
        assert!(!db
            .review_agent_draft("draft", ReviewOutcome::Discarded, NOW + 4)
            .unwrap());
        assert!(
            !db.review_agent_draft("task", ReviewOutcome::Sent, NOW).unwrap(),
            "only drafts are reviewed"
        );
        let a = db.get_agent_action("draft").unwrap().unwrap();
        assert_eq!(a.review_outcome, Some(ReviewOutcome::Sent));
        assert_eq!(a.reviewed_at, Some(NOW + 3));
        assert!(!a.needs_review);
    }

    #[test]
    fn drafts_to_review_are_listed_with_the_pending_actions() {
        let db = db();
        seed_draft(&db, "d1");
        db.insert_agent_run(&run(
            "run",
            "e-1",
            AgentRunStatus::Matched,
            NOW,
            vec![
                action("pending", AgentActionKind::Archive, AgentActionStatus::Pending, NOW + 2),
                draft_action("draft", "d1"),
                action("task", AgentActionKind::CreateTask, AgentActionStatus::Done, NOW + 1),
            ],
        ))
        .unwrap();
        let ids: Vec<_> = db.list_agent_actions(10).unwrap().into_iter().map(|a| a.id).collect();
        assert_eq!(ids, vec!["draft", "pending", "task"]);
    }

    #[test]
    fn the_side_panel_lists_pending_actions_oldest_first_then_recent_decisions() {
        let db = db();
        let mut decided = action("done", AgentActionKind::CreateTask, AgentActionStatus::Done, NOW);
        decided.decided_at = Some(NOW + 50);
        let mut older_decided = action("rejected", AgentActionKind::Star, AgentActionStatus::Rejected, NOW);
        older_decided.decided_at = Some(NOW + 10);
        db.insert_agent_run(&run(
            "run",
            "e-1",
            AgentRunStatus::Matched,
            NOW,
            vec![
                action("p-new", AgentActionKind::Archive, AgentActionStatus::Pending, NOW + 5),
                action("p-old", AgentActionKind::MarkRead, AgentActionStatus::Pending, NOW + 1),
                decided,
                older_decided,
            ],
        ))
        .unwrap();
        let ids: Vec<_> = db.list_agent_actions(10).unwrap().into_iter().map(|a| a.id).collect();
        assert_eq!(ids, vec!["p-old", "p-new", "done", "rejected"]);
        let capped: Vec<_> = db.list_agent_actions(1).unwrap().into_iter().map(|a| a.id).collect();
        assert_eq!(
            capped,
            vec!["p-old", "p-new", "done"],
            "pending actions are never capped"
        );
    }

    #[test]
    fn a_pending_action_is_claimed_or_rejected_once() {
        let db = db();
        db.insert_agent_run(&run(
            "run",
            "e-1",
            AgentRunStatus::Matched,
            NOW,
            vec![
                action("a", AgentActionKind::Archive, AgentActionStatus::Pending, NOW),
                action("b", AgentActionKind::Star, AgentActionStatus::Pending, NOW),
            ],
        ))
        .unwrap();
        assert!(db.claim_agent_action("a", NOW + 1).unwrap());
        assert!(!db.claim_agent_action("a", NOW + 2).unwrap(), "a second approval loses");
        assert!(!db.reject_agent_action("a", NOW + 2).unwrap());
        db.finish_agent_action("a", Err("offline")).unwrap();
        let a = db.get_agent_action("a").unwrap().unwrap();
        assert_eq!(a.status, AgentActionStatus::Failed);
        assert_eq!(a.error.as_deref(), Some("offline"));
        assert_eq!(a.decided_at, Some(NOW + 1));

        assert!(db.reject_agent_action("b", NOW).unwrap());
        assert!(
            !db.claim_agent_action("b", NOW).unwrap(),
            "a rejected action never runs"
        );
    }

    #[test]
    fn a_deleted_rule_leaves_its_actions_with_the_rule_name() {
        let db = db();
        db.insert_agent_rule(&rule("r1", NOW)).unwrap();
        let mut from_rule = action("a", AgentActionKind::Star, AgentActionStatus::Pending, NOW);
        from_rule.rule_id = Some("r1".into());
        db.insert_agent_run(&run("run", "e-1", AgentRunStatus::Matched, NOW, vec![from_rule]))
            .unwrap();
        db.delete_agent_rule("r1").unwrap();
        let a = db.get_agent_action("a").unwrap().unwrap();
        assert_eq!(a.rule_id, None);
        assert_eq!(a.rule_name, "Rule r1");
    }

    #[test]
    fn candidates_are_new_inbox_mail_from_others_not_yet_evaluated_oldest_first() {
        let db = db();
        seed_email(&db, "before", NOW - 1, "inbox", false);
        seed_email(&db, "second", NOW + 20, "inbox", false);
        seed_email(&db, "first", NOW + 10, "inbox", false);
        seed_email(&db, "sent", NOW + 10, "sent", true);
        seed_email(&db, "spam", NOW + 10, "spam", false);
        seed_email(&db, "done", NOW + 10, "inbox", false);
        db.insert_agent_run(&run("r", "done", AgentRunStatus::NoMatch, NOW, vec![]))
            .unwrap();
        assert_eq!(
            db.agent_candidate_emails("acc-1", NOW, 10).unwrap(),
            vec!["first", "second"]
        );
        assert_eq!(db.agent_candidate_emails("acc-1", NOW, 1).unwrap(), vec!["first"]);
        assert!(db.agent_candidate_emails("acc-2", NOW, 10).unwrap().is_empty());
    }

    #[test]
    fn junk_is_never_a_candidate() {
        let db = db();
        seed_email(&db, "junk", NOW + 1, "inbox", false);
        db.connection()
            .execute(
                "INSERT INTO email_junk (email_id, account_id, spam_score, phish_score, gray_score, band,
                                         primary_kind, reasons_json, method, scored_at)
                 VALUES ('junk', 'acc-1', 1.0, 0.0, 0.0, 'junk', 'spam', '[]', 'deterministic', 0)",
                [],
            )
            .unwrap();
        assert!(db.agent_candidate_emails("acc-1", NOW, 10).unwrap().is_empty());
    }

    fn panel(id: &str, prompt: &str) -> AgentPanel {
        AgentPanel {
            id: id.into(),
            title: format!("Panel {id}"),
            prompt: prompt.into(),
            window: PanelWindow::Today,
            created_at: NOW,
            count: 0,
        }
    }

    #[test]
    fn panels_list_in_creation_order_and_count_hits_in_their_window() {
        let db = db();
        db.insert_agent_panel(&panel("p1", "support")).unwrap();
        db.insert_agent_panel(&panel("p2", "invoices")).unwrap();
        let ids: Vec<_> = db.list_agent_panels().unwrap().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec!["p1", "p2"]);

        seed_email(&db, "old", NOW - 100, "inbox", false);
        seed_email(&db, "new", NOW + 5, "inbox", false);
        db.add_agent_panel_hit("p1", "old", NOW - 100).unwrap();
        db.add_agent_panel_hit("p1", "new", NOW + 5).unwrap();
        db.add_agent_panel_hit("p1", "new", NOW + 5).unwrap();
        assert_eq!(db.count_agent_panel_hits("p1", NOW).unwrap(), 1);
        assert_eq!(db.count_agent_panel_hits("p1", NOW - 100).unwrap(), 2);
        assert_eq!(db.count_agent_panel_hits("p2", 0).unwrap(), 0);
    }

    #[test]
    fn a_new_panel_prompt_forgets_old_hits_but_a_new_title_does_not() {
        let db = db();
        db.insert_agent_panel(&panel("p1", "support")).unwrap();
        seed_email(&db, "e", NOW, "inbox", false);
        db.add_agent_panel_hit("p1", "e", NOW).unwrap();

        let mut renamed = panel("p1", "support");
        renamed.title = "Support today".into();
        renamed.window = PanelWindow::Last7Days;
        assert!(db.update_agent_panel(&renamed).unwrap());
        assert_eq!(db.count_agent_panel_hits("p1", 0).unwrap(), 1);
        let stored = db.get_agent_panel("p1").unwrap().unwrap();
        assert_eq!(
            (stored.title.as_str(), stored.window),
            ("Support today", PanelWindow::Last7Days)
        );

        assert!(db.update_agent_panel(&panel("p1", "complaints")).unwrap());
        assert_eq!(db.count_agent_panel_hits("p1", 0).unwrap(), 0);
        assert!(!db.update_agent_panel(&panel("missing", "x")).unwrap());
    }

    #[test]
    fn deleting_a_panel_or_its_email_removes_the_hits() {
        let db = db();
        db.insert_agent_panel(&panel("p1", "support")).unwrap();
        seed_email(&db, "e1", NOW, "inbox", false);
        db.add_agent_panel_hit("p1", "e1", NOW).unwrap();
        db.connection()
            .execute("DELETE FROM emails WHERE id = 'e1'", [])
            .unwrap();
        assert_eq!(db.count_agent_panel_hits("p1", 0).unwrap(), 0);
        assert!(db.delete_agent_panel("p1").unwrap());
        assert!(db.get_agent_panel("p1").unwrap().is_none());
    }

    #[test]
    fn recent_mail_with_attendees_is_newest_first_in_both_directions() {
        let db = db();
        seed_email(&db, "from-ana", NOW, "inbox", false);
        seed_email(&db, "to-ana", NOW + 10, "sent", true);
        db.connection()
            .execute(
                "UPDATE emails SET sender_email = 'me@example.com', recipients_json = '[\"Ana <ANA@example.com>\"]'
                 WHERE id = 'to-ana'",
                [],
            )
            .unwrap();
        seed_email(&db, "other", NOW + 20, "inbox", false);
        db.connection()
            .execute(
                "UPDATE emails SET sender_email = 'zoe@example.com' WHERE id = 'other'",
                [],
            )
            .unwrap();
        let found = db.agent_recent_with("acc-1", &["ana@example.com"], 10).unwrap();
        let senders: Vec<_> = found.iter().map(|(ts, s, _)| (*ts, s.as_str())).collect();
        assert_eq!(senders, vec![(NOW + 10, "me@example.com"), (NOW, "ana@example.com")]);
        assert_eq!(db.agent_recent_with("acc-1", &["ana@example.com"], 1).unwrap().len(), 1);
        assert!(db.agent_recent_with("acc-1", &[], 10).unwrap().is_empty());
    }

    #[test]
    fn a_panel_window_lists_inbox_mail_newest_first() {
        let db = db();
        seed_email(&db, "before", NOW - 1, "inbox", false);
        seed_email(&db, "a", NOW, "inbox", false);
        seed_email(&db, "b", NOW + 1, "inbox", false);
        seed_email(&db, "sent", NOW + 2, "sent", true);
        assert_eq!(db.agent_panel_window_emails(NOW, 10).unwrap(), vec!["b", "a"]);
        assert_eq!(db.agent_panel_window_emails(NOW, 1).unwrap(), vec!["b"]);
    }
}
