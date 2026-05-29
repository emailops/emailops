// Chat conversations & messages persistence.
//
// Conventions (per CLAUDE.md):
//   - SELECTs use `self.reader()` (read pool).
//   - INSERT / UPDATE / DELETE / DDL use `self.connection()` (write conn).

use rusqlite::params;

use crate::models::error::Result;
use crate::models::{ChatConversation, ChatMessage, ChatMessageSource, ChatTrace};

use super::Database;

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

const MSG_COLUMNS: &str = "id, conversation_id, role, content, model, token_count, latency_ms, created_at, trace, referenced_email_ids, referenced_draft_ids";

fn row_to_conversation(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatConversation> {
    Ok(ChatConversation {
        id: row.get(0)?,
        account_id: row.get(1)?,
        title: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatMessage> {
    let trace_json: Option<String> = row.get(8)?;
    // A malformed trace JSON (e.g. older row from a schema transition) must not
    // break message loading — drop it, log, and let the UI render the message
    // without a reasoning section.
    let trace = trace_json.and_then(|s| match serde_json::from_str::<ChatTrace>(&s) {
        Ok(t) => Some(t),
        Err(e) => {
            crate::services::logger::log("debug", "chat", format!("dropping malformed trace JSON: {e}"));
            None
        }
    });
    // referenced_email_ids / referenced_draft_ids: JSON array TEXT (NULL
    // on pre-migration rows). Same drop-and-log degrade as trace — bad
    // JSON must never prevent the message itself from rendering.
    let parse_refs = |col: usize, kind: &str| -> rusqlite::Result<Vec<String>> {
        let json: Option<String> = row.get(col)?;
        Ok(json
            .and_then(|s| match serde_json::from_str::<Vec<String>>(&s) {
                Ok(v) => Some(v),
                Err(e) => {
                    crate::services::logger::log("debug", "chat", format!("dropping malformed {kind} refs JSON: {e}"));
                    None
                }
            })
            .unwrap_or_default())
    };
    let referenced_email_ids = parse_refs(9, "email")?;
    let referenced_draft_ids = parse_refs(10, "draft")?;
    Ok(ChatMessage {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        model: row.get(4)?,
        token_count: row.get(5)?,
        latency_ms: row.get(6)?,
        created_at: row.get(7)?,
        sources: Vec::new(),
        trace,
        referenced_email_ids,
        referenced_draft_ids,
    })
}

impl Database {
    // ── Conversations ───────────────────────────────────────────────────────

    pub fn create_chat_conversation(&self, account_id: &str, title: &str) -> Result<ChatConversation> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_ts();
        let conn = self.connection();
        conn.execute(
            "INSERT INTO chat_conversations (id, account_id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![id, account_id, title, now],
        )?;
        Ok(ChatConversation {
            id,
            account_id: account_id.to_string(),
            title: title.to_string(),
            created_at: now,
            updated_at: now,
        })
    }

    /// Create a conversation seeded with one role='system' message in a single
    /// transaction. Used by the "Chat about this thread" feature so the
    /// conversation row and its context message are committed atomically — if
    /// the system-message insert fails, the conversation row is rolled back
    /// and the user retries cleanly instead of seeing a half-created chat.
    pub fn create_chat_conversation_with_system_message(
        &self,
        account_id: &str,
        title: &str,
        system_content: &str,
    ) -> Result<ChatConversation> {
        let conv_id = uuid::Uuid::new_v4().to_string();
        let msg_id = uuid::Uuid::new_v4().to_string();
        let now = now_ts();
        let conn = self.connection();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO chat_conversations (id, account_id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![conv_id, account_id, title, now],
        )?;
        tx.execute(
            "INSERT INTO chat_messages (id, conversation_id, role, content, created_at)
             VALUES (?1, ?2, 'system', ?3, ?4)",
            params![msg_id, conv_id, system_content, now],
        )?;
        tx.commit()?;
        Ok(ChatConversation {
            id: conv_id,
            account_id: account_id.to_string(),
            title: title.to_string(),
            created_at: now,
            updated_at: now,
        })
    }

    pub fn list_chat_conversations(&self, account_id: &str) -> Result<Vec<ChatConversation>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, title, created_at, updated_at
             FROM chat_conversations
             WHERE account_id = ?1
             ORDER BY updated_at DESC",
        )?;
        let convs = stmt
            .query_map(params![account_id], row_to_conversation)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(convs)
    }

    pub fn rename_chat_conversation(&self, id: &str, title: &str) -> Result<()> {
        let conn = self.connection();
        let now = now_ts();
        conn.execute(
            "UPDATE chat_conversations SET title = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, title, now],
        )?;
        Ok(())
    }

    pub fn delete_chat_conversation(&self, id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute("DELETE FROM chat_conversations WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn touch_chat_conversation(&self, id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE chat_conversations SET updated_at = ?2 WHERE id = ?1",
            params![id, now_ts()],
        )?;
        Ok(())
    }

    /// Fetch a single conversation by id. Used by auto-title logic to check
    /// whether the current title is still the default placeholder.
    pub fn get_chat_conversation(&self, id: &str) -> Result<Option<ChatConversation>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT id, account_id, title, created_at, updated_at
             FROM chat_conversations WHERE id = ?1",
            params![id],
            row_to_conversation,
        );
        match result {
            Ok(c) => Ok(Some(c)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_chat_conversation_account(&self, id: &str) -> Result<Option<String>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT account_id FROM chat_conversations WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    // ── Messages ─────────────────────────────────────────────────────────────

    pub fn insert_chat_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        model: Option<&str>,
    ) -> Result<ChatMessage> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_ts();
        let conn = self.connection();
        conn.execute(
            "INSERT INTO chat_messages (id, conversation_id, role, content, model, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, conversation_id, role, content, model, now],
        )?;
        conn.execute(
            "UPDATE chat_conversations SET updated_at = ?2 WHERE id = ?1",
            params![conversation_id, now],
        )?;
        Ok(ChatMessage {
            id,
            conversation_id: conversation_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            model: model.map(|s| s.to_string()),
            token_count: None,
            latency_ms: None,
            created_at: now,
            sources: Vec::new(),
            trace: None,
            referenced_email_ids: Vec::new(),
            referenced_draft_ids: Vec::new(),
        })
    }

    /// Persist the aggregated email-ref allowlist for a completed assistant
    /// turn — the union of every `ToolOutput.email_refs` produced by tool
    /// calls in that turn. Frontend uses it to validate `email://EMAIL_ID`
    /// markdown links; ids not in the list are dropped (and a warning
    /// logged) so a hallucinated reference can't open the wrong email.
    /// Empty input writes NULL so the row mirrors what a pre-migration row
    /// would look like (no allowlist == no chips rendered).
    pub fn update_chat_message_referenced_emails(&self, message_id: &str, refs: &[String]) -> Result<()> {
        self.update_chat_message_refs("referenced_email_ids", message_id, refs, "email")
    }

    /// Same as `update_chat_message_referenced_emails` but for the
    /// `referenced_draft_ids` column — feeds the `draft://DRAFT_ID`
    /// validator on the frontend.
    pub fn update_chat_message_referenced_drafts(&self, message_id: &str, refs: &[String]) -> Result<()> {
        self.update_chat_message_refs("referenced_draft_ids", message_id, refs, "draft")
    }

    fn update_chat_message_refs(
        &self,
        column: &'static str,
        message_id: &str,
        refs: &[String],
        kind: &str,
    ) -> Result<()> {
        let conn = self.connection();
        if refs.is_empty() {
            conn.execute(
                &format!("UPDATE chat_messages SET {} = NULL WHERE id = ?1", column),
                params![message_id],
            )?;
            return Ok(());
        }
        let json = serde_json::to_string(refs).map_err(|e| {
            crate::models::error::AppError::InvalidInput(format!("failed to serialize {} refs: {}", kind, e))
        })?;
        conn.execute(
            &format!("UPDATE chat_messages SET {} = ?2 WHERE id = ?1", column),
            params![message_id, json],
        )?;
        Ok(())
    }

    /// Persist the reasoning trace for a completed assistant turn. Called from
    /// `run_chat_turn` after retrieval, tool calls, and streaming have all
    /// finished so the trace reflects the full flow.
    pub fn update_chat_message_trace(&self, message_id: &str, trace: &ChatTrace) -> Result<()> {
        let json = serde_json::to_string(trace).map_err(|e| {
            crate::models::error::AppError::InvalidInput(format!("failed to serialize chat trace: {}", e))
        })?;
        let conn = self.connection();
        conn.execute(
            "UPDATE chat_messages SET trace = ?2 WHERE id = ?1",
            params![message_id, json],
        )?;
        Ok(())
    }

    /// Update the assistant message with the final content and generation stats.
    pub fn update_chat_message_completion(
        &self,
        message_id: &str,
        content: &str,
        token_count: Option<i32>,
        latency_ms: Option<i64>,
    ) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE chat_messages SET content = ?2, token_count = ?3, latency_ms = ?4 WHERE id = ?1",
            params![message_id, content, token_count, latency_ms],
        )?;
        Ok(())
    }

    pub fn insert_chat_message_sources(&self, message_id: &str, sources: &[ChatMessageSource]) -> Result<()> {
        if sources.is_empty() {
            return Ok(());
        }
        let conn = self.connection();
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO chat_message_sources
                    (message_id, citation_number, email_id, relevance_score,
                     subject, sender, sender_email, email_timestamp, body_excerpt)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for src in sources {
                stmt.execute(params![
                    message_id,
                    src.citation_number,
                    src.email_id,
                    src.relevance_score,
                    src.subject,
                    src.sender,
                    src.sender_email,
                    src.timestamp,
                    src.body_excerpt,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Fetch all messages (with citations attached) for a conversation, oldest first.
    pub fn get_chat_messages(&self, conversation_id: &str) -> Result<Vec<ChatMessage>> {
        let conn = self.reader();

        let mut stmt = conn.prepare(&format!(
            // `created_at` is second-precision (now_ts), so user + assistant
            // messages inserted in the same second collide. Tiebreaking on
            // `id` (random UUID) shuffles them. `rowid` is monotonically
            // assigned on insert, so it preserves insertion order — and also
            // repairs ordering for pre-existing rows that already shared a
            // second-precision timestamp.
            "SELECT {} FROM chat_messages WHERE conversation_id = ?1 ORDER BY created_at ASC, rowid ASC",
            MSG_COLUMNS,
        ))?;
        let mut messages: Vec<ChatMessage> = stmt
            .query_map(params![conversation_id], row_to_message)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        if messages.is_empty() {
            return Ok(messages);
        }

        let mut src_stmt = conn.prepare(
            "SELECT s.message_id, s.citation_number, s.email_id, s.relevance_score,
                    s.subject, s.sender, s.sender_email, s.email_timestamp, s.body_excerpt
             FROM chat_message_sources s
             JOIN chat_messages m ON m.id = s.message_id
             WHERE m.conversation_id = ?1
             ORDER BY s.citation_number ASC",
        )?;
        let src_rows = src_stmt.query_map(params![conversation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                ChatMessageSource {
                    citation_number: row.get(1)?,
                    email_id: row.get(2)?,
                    relevance_score: row.get(3)?,
                    subject: row.get(4)?,
                    sender: row.get(5)?,
                    sender_email: row.get(6)?,
                    timestamp: row.get(7)?,
                    body_excerpt: row.get(8)?,
                },
            ))
        })?;

        let mut by_message: std::collections::HashMap<String, Vec<ChatMessageSource>> =
            std::collections::HashMap::new();
        for row in src_rows {
            let (msg_id, src) = row?;
            by_message.entry(msg_id).or_default().push(src);
        }

        for msg in &mut messages {
            if let Some(srcs) = by_message.remove(&msg.id) {
                msg.sources = srcs;
            }
        }

        Ok(messages)
    }

    /// Fetch all `system`-role messages for a conversation, in insertion order.
    ///
    /// Used by `run_chat_turn` to detect "thread-bound" conversations: when the
    /// chat was seeded with an email thread (via
    /// `create_chat_conversation_with_thread`), the cleaned thread is stored
    /// once as a system message. The chat service uses its presence to skip
    /// RAG retrieval / tool calls and inject the thread directly into the
    /// system prompt.
    pub fn get_chat_system_messages(&self, conversation_id: &str) -> Result<Vec<ChatMessage>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM chat_messages
             WHERE conversation_id = ?1 AND role = 'system'
             ORDER BY created_at ASC, rowid ASC",
            MSG_COLUMNS
        ))?;
        let rows = stmt
            .query_map(params![conversation_id], row_to_message)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Fetch the last `n` turns (user + assistant) for prompting, in chronological
    /// order. Does not include sources — prompt assembly doesn't need them.
    pub fn get_recent_chat_turns(&self, conversation_id: &str, n: usize) -> Result<Vec<ChatMessage>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM chat_messages
             WHERE conversation_id = ?1 AND role IN ('user', 'assistant')
             ORDER BY created_at DESC, rowid DESC LIMIT ?2",
            MSG_COLUMNS
        ))?;
        let mut rows: Vec<ChatMessage> = stmt
            .query_map(params![conversation_id, n as i64], row_to_message)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.reverse();
        Ok(rows)
    }
}
