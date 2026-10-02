# Mutants ledger

Surviving mutants from `make mutants` runs that were looked at and did **not**
get a new test, with the reason. The workflow is in
[COVERAGE-AND-MUTATION.md](COVERAGE-AND-MUTATION.md). Line numbers are left
out on purpose: they move with every edit, the function and the mutation do
not.

When a later run reports one of these again, it is expected. When a change
makes one observable (a new caller, a new branch), delete the row and write the
test.

## Equivalent or log-only

The mutation changes nothing a caller or a user can observe.

| File | Function | Mutation | Why it survives |
|---|---|---|---|
| src/db/emails/search.rs | `Database::search_emails_inner` | subject token length `>= 2` → `<` | Short or no tokens fall back to `subject LIKE %…%`, which matches the same rows as the FTS path; only the query plan changes. |
| src/services/attachment_safety.rs | `quarantine` (Windows) | body → `Ok(())` | `#[cfg(windows)]`: not compiled on the macOS host that runs mutants, so no test can see it. The macOS variant is caught. |
| src/services/chat/planner.rs | `SearchPlan::normalised` | `limit < 5` → `<= 5` | At a limit of exactly 5 both versions set 5. |
| src/services/chat/research/control.rs | `store_estimate`, `take_estimate` | `elapsed() < ESTIMATE_TTL` → `<=` | `Instant` is continuous; equality with the TTL is not observable. |
| src/services/chat/research/notes.rs | `parse_condensed` | `s < end` → `s <= end` | `s` indexes a `{` and `end` a `}`: they are never the same position. |
| src/services/emails/history_refresh.rs | `Applied::log`, `apply_changes` | body → `()`, counter `+=` → `*=`, the `> 0` / `+` mutations of the summary condition | The counters only feed the "Matched the account" log line. |
| src/services/emails/history_refresh.rs | `implied_labels` | delete the `"spam"` arm | A spam row's mailbox is never changed by the change log (spam belongs to the spam reconciliation), so its implied place label is never compared; read-state changes do not depend on it. |
| src/services/emails/mailbox_state.rs | `retry_pending_read_pushes` | delete `!` in `!plan.give_up.is_empty()`, `pushed +=` → `*=`, `pushed > 0` → `==`/`<`/`>=` | Each only decides whether a log line is emitted. |
| src/services/emails/mailbox_state.rs | `retry_pending_read_pushes`, `retry_pending_star_pushes` | guard `rows.is_empty()` → `false` | With no rows the plan is empty and nothing is pushed or cleared: the early return only skips work. |
| src/services/emails/mailbox_state.rs | `PushedFlag::noun` | → `""` / `"xyzzy"` | Log-only wording. |
| src/services/emails/reconcile.rs | `bare_address` | `start < end` → `<=` | `start` indexes a `<` and `end` a `>`: never the same position. |
| src/services/emails/reconcile.rs | `plan_sent_reconciliation` | `pending.is_empty() \|\| candidates.is_empty()` → `&&` | Early return only: with either list empty both passes find nothing. |
| src/services/emails/state_refresh.rs | `refresh_stored_mail_state` | `+=` → `*=` on the `read_changes`, `star_changes`, `moved`, `removed` counters; `>`/`+` mutations in `read_changes + star_changes + moved + removed > 0` | The counters only decide whether and what the "Matched the account" log line says. |
| src/services/emails/state_refresh.rs | `warn` | body → `()` | Log-only helper. |
| src/services/junk/content.rs | `extract_bare_urls` | `cursor < text.len()` → `<=` | At `cursor == len` the next `find` returns `None` and the loop breaks anyway. |
| src/services/junk/lookalike.rs | `detect` | `reference.is_empty() \|\| reference == candidate` → `&&` | Redundant guard: an equal domain has an equal brand and is skipped by `candidate_brand == reference_brand`; the punycode skeleton of a domain is never within the edit budget of its own `xn--` brand. |
| src/services/junk/mod.rs | `backfill_account` | `total < BACKFILL_CEILING` → `<=` | Differs only when exactly 50,000 messages were scored in one run; accepted rather than seeding 50,000 rows in a unit test. |
| src/services/junk/mod.rs | `log_tag_error` | body → `()` | Log-only helper. |
| src/services/junk/mod.rs | `score_with` | `primary != Legit && any axis flagged` → `\|\|` | `primary` is non-legit exactly when an axis is flagged (judge and `suppress_phishing` both derive it from the bands), so both halves always agree. |
| src/services/junk/model.rs | `score` | `2.0 * ALPHA` → `2.0 / ALPHA` (both classes) | `ALPHA` is 1.0, so both are 2.0. |
| src/services/junk/verdict.rs | `AxisBuilder::finish` | `score < junk_cutoff` → `<=` (unknown branch) | Differs only when a float sum lands exactly on the cutoff; not reachable from realistic weights. |
| src/services/junk/verdict.rs | `JunkAxis::as_str` | → `""` / `"xyzzy"` | Only used in a test's assertion message. |
| src/services/junk/verdict.rs | `JunkSignals::headers_available` | → `true` / `false` | Only called by the eval harness (`src/evals/junk`, `eval` feature), whose own tests cover it; not compiled in the mutants build. |
| src/services/junk/verdict.rs | `JunkSignals::is_first_contact` | every mutant | No caller anywhere: dead code, a candidate for removal. |
| src/services/junk/verdict.rs | `judge` | `caps_ratio > 0.7` → `>=` | Differs only for a subject whose ratio is exactly 0.7 in `f32`. |
| src/db/emails/batch.rs | `Database::insert_emails_batch` | delete `!` in `!released.is_empty()` | Only guards the "returned to the inbox: new mail arrived" log line; the snooze rows are already deleted. |
| src/db/emails/crud.rs | `Database::email_list_query` | `offset > 0` → `>=` | Turns on `OFFSET 0`, which returns the same rows; the date-arm window stays at `limit + 0`. |
| src/db/emails/snoozes.rs | `release_snoozes_for_new_mail` | `n > 0` → `<` / `==` / `>=` | The released list only feeds the log line in `insert_emails_batch`; the DELETE has already happened. |
| src/db/emails/snoozes.rs | `any_snooze` | → `Ok(true)` | Performance shortcut only: with an empty `thread_snoozes` the per-row checks and the release DELETE change nothing. |
| src/db/emails/snoozes.rs | `Database::snooze_threads` | `n +=` → `*=` | The returned count has no production caller (`services::emails::snooze::snooze_threads` discards it). |
| src/models/outbox.rs | `OutboxStatus::as_str`, `OutboxFailureKind::as_str` | → `""` / `"xyzzy"` | No caller anywhere (the SQL uses string literals): dead code, a candidate for removal. |
| src/services/emails/snooze.rs | `unsnooze_threads` | `n > 0` → `<` / `==` / `>=` | Only decides whether the "Unsnoozed" log line is written. |
| src/services/emails/snooze.rs | `wake_due_snoozes` | `pruned > 0` → `<` / `==` / `>=` | Only decides whether the "Dropped … woken snooze record(s)" debug line is written. |
| src/services/emails/thread_actions.rs | `apply_to_account` | `done`/`failed` `+=` → `*=`; `done > 0`, `failed > 0` → `<` / `==` / `>=`; `&&` → `\|\|` | The counters only decide the success and "could not be reached" log lines; failures reach the report through `report.failed`. |
| src/services/emails/thread_actions.rs | `verb`, `past_tense` | → `""` / `"xyzzy"` | Log-only wording. |
| src/services/mail_notifications.rs | `show_all` | `shown += 1` → `*=` | The count only feeds a debug log line. |
| src/services/mail_notifications.rs | `notify_new_mail` | `candidates.is_empty() \|\| watermark.is_none()` → `&&` | Early return only: the planner returns nothing for an empty batch or a `None` watermark (first sync). |
| src/services/mail_notifications.rs | `try_notify_new_mail` | `!prefs.enabled \|\| !prefs.account_enabled` → `&&` | Early return only: `notifications_wanted` in the planner checks both switches again. |
| src/services/notifier.rs | `<impl Notifier for TauriNotifier>::show`, `::app_focused` | → `Ok(())` / `true` / `false` | `#[cfg(feature = "desktop")]`: not compiled in the mutants build. |
| src/services/sender_controls.rs | `file_blocked_arrivals` | delete `!` in `!caught.is_empty()` | Only guards the summary log line. |
| src/services/sender_controls.rs | `log_report` | body → `()`; `>` → `<` / `==` / `>=` (moved and local-only counts) | Log-only helper. |
| src/services/signatures.rs | `check_signature_image` | `approx > MAX_SIGNATURE_IMAGE_BYTES + 3` → `>=` | `approx` is `payload.len() / 4 * 3`, always a multiple of 3, and `MAX + 3` is not, so they are never equal. |
| src/services/unsubscribe.rs | `bracketed_uris` | `end + 1` → `end * 1` | The next slice starts at the closing `>` instead of after it, and the next `find('<')` skips it either way. |
| src/services/unsubscribe.rs | `parse_mailto` | body cap `>` → `>=` / `==` | Unreachable: `MAX_URI_LEN` (2,048 bytes) bounds the whole mailto URI, so a decoded body never reaches `MAX_MAILTO_BODY` (2,048 chars). The `long_body` case of `overlong_values_are_rejected` is refused by the URI cap, not by this one. |
| src/services/unsubscribe.rs | `unsubscribe` | delete the `UnsubscribeKind::Link` arm | Only picks the wording of the success log line; the recorded kind and the return value are unchanged. |
| src/sync/provider.rs | `FakeEmailProvider` methods, `fake_labels` | `==` → `!=`, delete match arms | Test fake, not production code. |

## Gaps that need a seam or a fixture first

Real gaps: the behaviour matters, but a test needs a seam or a fixture the code
does not have yet.

| File | Function | Mutation | What is missing |
|---|---|---|---|
| src/services/chat/research/control.rs | `ESTIMATE_TTL` | `30 * 60` → `30 + 60` | Needs a clock seam (the module reads `Instant::now()`) |
| src/services/emails/mailbox_state.rs | `write_provider` | body → `Ok(None)` | Needs a provider-factory seam; for Gmail/IMAP/Outlook it calls `build_provider` (keychain, OAuth), so only the `*_with_provider` functions and the no-writes path are tested. |
| src/db/emails/search.rs | `Database::search_emails_inner` | bind-index `param_idx` increments after the sender range, relaxed-token and sender-domain branches (`+= 2`, `+= 1`, `+ range_params + 1`), and after the short-subject LIKE fallback | No test combines a sender filter with a later bound filter (subject, tags, dates) in one query, so a wrong placeholder index goes unnoticed. Needs a fixture with FTS-indexed rows (`insert_emails_batch`) and combined filters. |
| src/sync/imap.rs | `ImapClient::archive_uid_blocking`, `spam_uid_blocking`, `<impl EmailProvider for ImapClient>::set_starred`, `archive_message`, `move_to_spam`, `fetch_message_states` | bodies → `Ok(…)`, `==` → `!=` on the source folder, delete `!` on the folder-select and Message-ID checks, `&&` → `\|\|` / delete `!` in the Message-ID header filter | The blocking halves take a TLS `imap::Session`; no test IMAP server exists. Needs an IMAP fake (or a session trait) to cover: source already the target, missing source folder, Message-ID changed on the server. |
| src/services/emails/snooze.rs | `wake_due_snoozes` | `mark_snoozes_woken(..)? > 0` → `>=` | Only matters when another writer re-snoozes the thread between `pending_snoozes` and `mark_snoozes_woken`; needs a hook between the read and the write. |
| src/services/outbox.rs | `wake_dispatcher_at`, `dispatcher_waker` | body → `()`; `send_at - now` → `/` / `+`; `delay == 0` → `!=`; `delay + 1` → `-` / `*`; `dispatcher_waker` → a leaked fresh `Notify` | Timing only: spawns a sleep and notifies the dispatcher loop. Needs a timer seam and a way to observe the `Notify`. |
| src/services/unsubscribe.rs | `one_click_client` | redirect cap `>=` → `<` | Needs an HTTPS test server: wiremock serves plain http, and the policy refuses a non-https hop before the count matters. |
| src/services/sync_scheduler.rs | `SyncScheduler::start`, `snooze_wake_loop`, `outbox_dispatch_loop`, `AppOutboxProviders::provider_for` | bodies → `Default` / `()` / `Ok(Box::new(Default))` | Background loops and provider factory wired to `AppHandle`; the passes they run (`wake_due_snoozes`, `dispatch_due_outbox`) are tested directly. |

## Missed by the fast pass, caught by the whole suite

The fast pass runs one module's unit tests. These survived it and are caught by
tests elsewhere in the library or in `tests/integration.rs` (checked with
`RECHECK=<label> make mutants` or by applying the mutation by hand).

| File | Function | Mutation | Caught by |
|---|---|---|---|
| src/services/emails/reconcile.rs | `reconcile_pending_sent` | body → `()` | 4 tests in `tests/integration.rs` (`sync_reconciles_*`). |
| src/db/emails/search.rs | `Database::junk_condition` | → `String::new()` / `"xyzzy"` | 57 lib tests outside `db::emails::search` (chat and search services). |
| src/db/emails/search.rs | `Database::get_filtered_emails` | window bind index `first_idx + 4` → `*` | 2 lib tests outside `db::emails::search`. |
| src/db/emails/search.rs | `Database::read_thread_people` | → `Ok(())` | 8 lib tests outside `db::emails::search`. |
| src/db/emails/search.rs | `Database::search_emails_inner` | subject-filter routing `\|\|` → `&&`; category filter `!` deleted; category bind index `+=` → `-=` | 1–2 lib tests outside `db::emails::search` each. |
