pub mod accounts;
pub mod agent_search;
pub mod ai;
pub mod app_handle;
pub mod attachments;
pub mod background_refresh;
pub mod background_tasks;
pub mod calendar;
pub mod chat;
pub mod classification;
pub mod clock;
pub mod connectivity;
pub mod contacts;
pub mod dashboard;
pub mod email_company;
pub mod emails;
pub mod embeddings;
pub mod events;
pub mod filters;
pub mod i18n;
pub mod junk;
pub mod keychain;
pub mod lenses;
pub mod logger;
pub mod memory;
pub mod password;
pub mod prompts;
pub mod retrieval;
pub mod search;
pub mod secrets_vault;
pub mod storage_stats;
pub mod sync_error_dedup;
// Desktop-only: the Tauri background scheduler (IMAP IDLE threads, Gmail polling,
// calendar/meeting loops). A server deployment replaces it with its own supervisor,
// so it is gated to keep `services/` compiling without Tauri.
#[cfg(feature = "desktop")]
pub mod sync_scheduler;
pub mod tag_priority;
pub mod task_queue;
pub mod tasks;
pub mod thread_clean;
pub mod translation;
// Desktop-only: the GitHub-release update checker. Meaningless for a served app.
#[cfg(feature = "desktop")]
pub mod updates;
