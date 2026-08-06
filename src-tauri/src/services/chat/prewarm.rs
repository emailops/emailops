//! Idle-time chat prompt-prefix prewarm.
//!
//! The embedded llama.cpp actor keeps a never-evicted KV anchor of the
//! invariant system prefix (~2.3k tokens). Seeding it costs one prefill pass;
//! doing that at idle time (app startup, chat panel open, model/account
//! switch) instead of on the user's first turn cuts that turn's prefill from
//! ~14s to ~4s on M1-class hardware. See `ai/llama_cpp/actor.rs`.

use crate::ai::provider::{AIProvider, AiMessage};
use crate::db::Database;
use crate::models::error::Result;

use super::tools::ToolRegistry;

/// Build the invariant chat prompt prefix for `account_id` and hand it to the
/// provider's prompt cache. The messages are produced by the SAME
/// `build_prompt` call a real turn uses (empty sources/history/question), so
/// the cached system-prefix bytes match a real turn byte-for-byte by
/// construction.
///
/// Best-effort: callers treat errors as log-and-continue — a failed prewarm
/// must never block startup or a chat turn (the turn just pays the normal
/// cold prefill).
pub async fn prewarm_chat(
    db: &Database,
    registry: &ToolRegistry,
    provider: &dyn AIProvider,
    account_id: &str,
) -> Result<()> {
    // Resolve exactly the inputs `run_chat_turn` feeds `build_prompt` so the
    // prewarmed prefix stays byte-identical with real turns.
    let ai_language = crate::services::i18n::resolve_ai_language(db)?;
    let system_template = crate::services::prompts::get_template(db, "chat.system")?;
    let tools_section = registry.render_system_prompt_section(db);
    // Same graceful degradation as `run_chat_turn`: a lookup failure just
    // drops the identity line rather than failing the prewarm.
    let user_email = db
        .get_account(account_id)
        .ok()
        .flatten()
        .map(|a| a.email)
        .unwrap_or_default();

    let messages: Vec<AiMessage> = super::build_prompt(
        &[],
        &[],
        "",
        ai_language.english_name(),
        &user_email,
        &system_template,
        &tools_section,
        // Prewarm carries no sources, so the budget cannot change the bytes it
        // produces — but it must match what a real turn will send, or the KV
        // prefix it warms would not be the one the turn reuses.
        super::context_budget::FULL_BUDGET,
    )
    .into_iter()
    .map(|(role, content)| AiMessage {
        role,
        content,
        tool_calls: None,
    })
    .collect();

    provider.prewarm_chat_prefix(messages).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::provider::FakeAiProvider;
    use crate::models::Account;
    use crate::services::chat::tools::default_registry;

    fn test_account() -> Account {
        Account {
            id: "acc-1".to_string(),
            provider: "gmail".to_string(),
            email: "user@example.com".to_string(),
            name: "Test User".to_string(),
            created_at: 0,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: None,
        }
    }

    #[tokio::test]
    async fn prewarm_sends_the_same_prefix_bytes_as_a_real_turn() {
        let db = Database::new_for_testing().expect("test db");
        db.insert_account(&test_account()).expect("insert account");
        let registry = default_registry();
        let fake = FakeAiProvider::new();

        prewarm_chat(&db, &registry, &fake, "acc-1").await.expect("prewarm");

        // Exactly one prewarm call, carrying the messages a real turn with no
        // sources/history/question would produce — byte-identical system
        // message is the property the KV anchor depends on.
        let calls = fake.prewarm_calls();
        assert_eq!(calls.len(), 1, "expected exactly one prewarm call");
        let sent = &calls[0];

        let ai_language = crate::services::i18n::resolve_ai_language(&db).expect("lang");
        let system_template = crate::services::prompts::get_template(&db, "chat.system").expect("template");
        let tools_section = registry.render_system_prompt_section(&db);
        let expected: Vec<AiMessage> = crate::services::chat::build_prompt(
            &[],
            &[],
            "",
            ai_language.english_name(),
            "user@example.com",
            &system_template,
            &tools_section,
            crate::services::chat::context_budget::FULL_BUDGET,
        )
        .into_iter()
        .map(|(role, content)| AiMessage {
            role,
            content,
            tool_calls: None,
        })
        .collect();

        assert_eq!(sent.len(), expected.len(), "message count mismatch");
        for (s, e) in sent.iter().zip(expected.iter()) {
            assert_eq!(s.role, e.role);
            assert_eq!(s.content, e.content);
        }
        assert_eq!(sent[0].role, "system");
    }

    #[tokio::test]
    async fn prewarm_degrades_to_blank_identity_on_unknown_account() {
        let db = Database::new_for_testing().expect("test db");
        let registry = default_registry();
        let fake = FakeAiProvider::new();

        prewarm_chat(&db, &registry, &fake, "nope").await.expect("prewarm");

        let calls = fake.prewarm_calls();
        assert_eq!(calls.len(), 1);
        // No account → no identity line, same degradation as a real turn.
        assert!(!calls[0][0].content.contains("YOUR USER'S IDENTITY"));
    }
}
