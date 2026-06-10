use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    pub account_id: String,
    pub status: String,
    pub current: u32,
    pub total: u32,
    pub message: String,
}

/// Emit a `sync-progress` event to the frontend via the global EventSink seam.
///
/// Progress events drive the UI progress bar only — they are intentionally
/// NOT written to the output-panel log seam. The per-email "Downloading X of
/// Y" tick fires once per message, so logging every progress update would
/// flood the panel on a first sync. Output-panel sync lines come from the
/// explicit (and throttled) `emit_account_log` calls instead.
pub(super) fn emit_progress(account_id: &str, status: &str, current: u32, total: u32, message: &str) {
    crate::services::events::emit(
        "sync-progress",
        SyncProgress {
            account_id: account_id.to_string(),
            status: status.to_string(),
            current,
            total,
            message: message.to_string(),
        },
    );
}

/// Route to the global Logger seam. The `AppHandle` is no longer needed
/// since logging goes through the seam rather than `app.emit`.
pub(super) fn emit_log(level: &str, source: &str, message: &str) {
    crate::services::logger::log(level, source, message);
}

/// Same as `emit_log`, but prepends `[account_email]` to the message so the
/// output panel makes it obvious which account produced the log line.
pub(super) fn emit_account_log(level: &str, source: &str, account_email: &str, message: &str) {
    emit_log(level, source, &format!("[{}] {}", account_email, message));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::{install as install_sink, seam_test_lock, NoopEventSink};
    use crate::services::logger::{
        install as install_logger, install_for_testing as install_logger_for_testing, NoopLogger,
    };
    use std::sync::Arc;

    // `emit_progress` drives the UI progress bar only. It routes to the event
    // sink (`sync-progress`) and must NOT write to the output-panel log seam:
    // the per-email "Downloading X of Y" tick fires once per message, so logging
    // here floods the panel during a first sync. The panel gets its sync lines
    // from explicit (throttled) `emit_account_log` calls instead.
    #[test]
    fn emit_progress_routes_to_event_sink_not_log_seam() {
        let _g = seam_test_lock();
        let sink = crate::services::events::install_for_testing();
        let logger = install_logger_for_testing();

        emit_progress("acc", "syncing", 1, 10, "Downloading 1 of 10 new emails");

        // Goes to the UI event seam as a `sync-progress` event...
        assert_eq!(sink.count("sync-progress"), 1);
        // ...and never to the output-panel log seam.
        assert!(
            logger.events().is_empty(),
            "progress events must not hit the output-panel log seam, got: {:?}",
            logger.events()
        );

        install_sink(Arc::new(NoopEventSink));
        install_logger(Arc::new(NoopLogger));
    }
}
