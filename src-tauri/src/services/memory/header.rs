//! Build the always-on `<memory>...</memory>` context header injected into
//! every chat turn.
//!
//! The header is intentionally cheap: it only reads from SQLite (no LLM, no
//! network, no vec0 KNN unless a caller explicitly uses `hybrid_search_facts`
//! and passes it in). Keeping this synchronous means it can be assembled
//! inside `run_chat_turn` without adding a new async hop to an already-fat
//! critical path.
//!
//! Layout:
//!
//! ```text
//! <memory>
//! Profile:
//!   - Alice is the founder and handles all billing (promoted)
//!   - Alice prefers morning calls (candidate)
//! Contacts matching query:
//!   - alice.smith@emailops.com: Head of ops at Emailops (promoted)
//! Tasks: open=4 overdue=1 due_today=2
//! Awaiting your reply: 2 threads
//!   - "Invoice Q1" — last touched 2d ago
//! Recent activity:
//!   - read 1h ago
//!   - replied 3h ago
//! </memory>
//! ```

use std::sync::Arc;

use crate::db::Database;
use crate::models::error::Result;

/// Maximum characters in the final header. LLMs will gladly take 4k of context
/// from this block — we'd rather hold it to a budget and fail a soft ~400
/// token cap than fight for room with the sources block.
const MAX_HEADER_CHARS: usize = 2_200;

/// Build the `<memory>...</memory>` block for a chat turn. `query` is used
/// only for subject-key contact lookups; it is not fed to any LLM.
///
/// Returns `None` when the resulting header would be empty — so callers can
/// skip `<memory></memory>` entirely rather than inject dead tags.
pub fn build_header(db: &Arc<Database>, account_id: &str, query: &str) -> Result<Option<String>> {
    let mut sections: Vec<String> = Vec::new();

    // ── Profile facts (user self-knowledge) ────────────────────────────────
    let profile = db.list_memory_facts(account_id, Some("promoted"), 8)?;
    let profile_user: Vec<_> = profile.iter().filter(|f| f.subject_kind == "user").take(3).collect();
    if !profile_user.is_empty() {
        let mut s = String::from("Profile:\n");
        for f in &profile_user {
            s.push_str(&format!("  - {} (promoted)\n", one_line(&f.fact)));
        }
        sections.push(s);
    }

    // ── Entity recall via FTS on the user query ────────────────────────────
    // FTS only — vector KNN would make this async. The hybrid variant is
    // available via `services::memory::embeddings::hybrid_search_facts` for
    // callers willing to pay an embed-API round trip (tool dispatcher does).
    let q = query.trim();
    if !q.is_empty() {
        let hits = db.search_memory_facts_fts(account_id, q, 4)?;
        let relevant: Vec<_> = hits
            .into_iter()
            .filter(|(f, _)| f.subject_kind != "user")
            .take(3)
            .collect();
        if !relevant.is_empty() {
            let mut s = String::from("Entities matching query:\n");
            for (f, _) in &relevant {
                s.push_str(&format!(
                    "  - {} [{}]: {} ({})\n",
                    f.subject_kind,
                    f.subject_key,
                    one_line(&f.fact),
                    f.status,
                ));
            }
            sections.push(s);
        }
    }

    // ── Task aggregate counts ──────────────────────────────────────────────
    let (open, overdue, due_today) = db.count_pending_tasks(account_id)?;
    if open > 0 {
        sections.push(format!(
            "Tasks: open={} overdue={} due_today={}\n",
            open, overdue, due_today
        ));
    }

    // ── Threads awaiting the user ──────────────────────────────────────────
    let awaiting_user = db.list_open_thread_states(account_id, Some("user"), 10)?;
    if !awaiting_user.is_empty() {
        let relevant = if q.is_empty() {
            awaiting_user.iter().take(3).cloned().collect::<Vec<_>>()
        } else {
            let toks = tokenize(q);
            let mut matched: Vec<_> = awaiting_user
                .iter()
                .filter(|t| {
                    t.summary
                        .as_deref()
                        .map(|s| toks.iter().any(|tk| s.to_ascii_lowercase().contains(tk)))
                        .unwrap_or(false)
                        || t.participants
                            .iter()
                            .any(|p| toks.iter().any(|tk| p.to_ascii_lowercase().contains(tk)))
                })
                .take(3)
                .cloned()
                .collect();
            if matched.is_empty() {
                matched = awaiting_user.iter().take(3).cloned().collect();
            }
            matched
        };
        let mut s = format!("Awaiting your reply: {} threads\n", awaiting_user.len());
        for t in &relevant {
            let line = t.summary.clone().unwrap_or_else(|| format!("thread {}", t.thread_id));
            s.push_str(&format!("  - {}\n", one_line(&line)));
        }
        sections.push(s);
    }

    // ── Recent activity (3 most recent interaction events) ─────────────────
    let events = db.recent_interaction_events(account_id, 3)?;
    if !events.is_empty() {
        let now = chrono::Utc::now().timestamp();
        let mut s = String::from("Recent activity:\n");
        for e in &events {
            s.push_str(&format!("  - {} {}\n", e.kind, ago_human(now - e.created_at)));
        }
        sections.push(s);
    }

    if sections.is_empty() {
        return Ok(None);
    }

    let mut body = sections.join("");
    if body.len() > MAX_HEADER_CHARS {
        // Hard cap — take as much as fits on a codepoint boundary.
        body = take_chars(&body, MAX_HEADER_CHARS);
        body.push_str("…\n");
    }
    Ok(Some(format!("<memory>\n{body}</memory>\n")))
}

fn one_line(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    // Collapse runs of spaces.
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    out.trim().to_string()
}

fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '@' && c != '.')
        .filter(|t| t.len() >= 3)
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

fn ago_human(secs: i64) -> String {
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    format!("{days}d ago")
}

fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MemoryFact, PendingTask, ThreadState};

    fn mk_fact(id: &str, kind: &str, key: &str, text: &str, status: &str) -> MemoryFact {
        MemoryFact {
            id: id.into(),
            account_id: "a1".into(),
            subject_kind: kind.into(),
            subject_key: key.into(),
            fact: text.into(),
            source: "extraction".into(),
            source_email_id: None,
            confidence: 0.8,
            score: 1.0,
            status: status.into(),
            last_used_at: None,
            domain: None,
            vigency: None,
            company: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn mk_task(id: &str) -> PendingTask {
        PendingTask {
            id: id.into(),
            account_id: "a1".into(),
            title: format!("task {id}"),
            detail: None,
            source: "extracted".into(),
            source_email_id: None,
            source_thread_id: None,
            assignee: "me".into(),
            status: "open".into(),
            priority: "normal".into(),
            due_at: None,
            completed_at: None,
            company: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn mk_state(thread: &str, awaiting: &str, summary: &str) -> ThreadState {
        ThreadState {
            account_id: "a1".into(),
            thread_id: thread.into(),
            awaiting: awaiting.into(),
            last_inbound_at: None,
            last_outbound_at: None,
            last_touched_at: 0,
            summary: Some(summary.into()),
            commitment: None,
            deadline_at: None,
            participants: vec!["alice@ex.com".into()],
            updated_at: 0,
        }
    }

    #[test]
    fn header_is_none_when_nothing_to_say() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        let out = build_header(&db, "a1", "hello").unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn header_includes_profile_and_tasks_and_threads() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("a1");
        db.insert_memory_fact(&mk_fact("u1", "user", "self", "I run a SaaS", "promoted"))
            .unwrap();
        db.insert_memory_fact(&mk_fact(
            "c1",
            "contact",
            "alice@ex.com",
            "Alice leads ops at Acme",
            "promoted",
        ))
        .unwrap();
        db.insert_pending_task(&mk_task("t1")).unwrap();
        db.upsert_thread_state(&mk_state("thread-a", "user", "Invoice Q1"))
            .unwrap();

        let out = build_header(&db, "a1", "alice invoice").unwrap().unwrap();
        assert!(out.contains("<memory>"));
        assert!(out.contains("Profile:"));
        assert!(out.contains("I run a SaaS"));
        assert!(out.contains("Entities matching query:"));
        assert!(out.contains("Alice leads ops"));
        assert!(out.contains("Tasks: open=1"));
        assert!(out.contains("Awaiting your reply: 1"));
        assert!(out.contains("Invoice Q1"));
        assert!(out.ends_with("</memory>\n"));
    }

    #[test]
    fn header_hides_profile_when_only_candidate_facts_exist() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("a1");
        // Candidate-only facts must NOT leak into Profile; the header should
        // stay empty if nothing else qualifies either.
        db.insert_memory_fact(&mk_fact("u1", "user", "self", "I run a SaaS", "candidate"))
            .unwrap();
        let out = build_header(&db, "a1", "").unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn header_truncates_to_max_chars() {
        let db = Arc::new(Database::new_for_testing().unwrap());
        db.seed_test_account("a1");
        let huge = "a".repeat(5000);
        db.insert_memory_fact(&mk_fact("u1", "user", "self", &huge, "promoted"))
            .unwrap();
        let out = build_header(&db, "a1", "").unwrap().unwrap();
        assert!(out.len() < MAX_HEADER_CHARS + 200);
    }
}
