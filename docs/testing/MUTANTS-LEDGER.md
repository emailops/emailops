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
| src/services/emails/mailbox_state.rs | `retry_pending_read_pushes` | guard `rows.is_empty()` → `false` | With no rows the plan is empty and nothing is pushed or cleared: the early return only skips work. |
| src/services/emails/reconcile.rs | `bare_address` | `start < end` → `<=` | `start` indexes a `<` and `end` a `>`: never the same position. |
| src/services/emails/reconcile.rs | `plan_sent_reconciliation` | `pending.is_empty() \|\| candidates.is_empty()` → `&&` | Early return only: with either list empty both passes find nothing. |
| src/services/emails/state_refresh.rs | `refresh_stored_mail_state` | `+=` → `*=` on the `read_changes`, `moved`, `removed` counters; `>`/`+` mutations in `read_changes + moved + removed > 0` | The counters only decide whether and what the "Matched the account" log line says. |
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

## Gaps that need a seam or a fixture first

Real gaps: the behaviour matters, but a test needs a seam or a fixture the code
does not have yet.

| File | Function | Mutation | What is missing |
|---|---|---|---|
| src/services/chat/research/control.rs | `ESTIMATE_TTL` | `30 * 60` → `30 + 60` | Needs a clock seam (the module reads `Instant::now()`) |
| src/services/emails/mailbox_state.rs | `write_provider` | body → `Ok(None)` | Needs a provider-factory seam; for Gmail/IMAP/Outlook it calls `build_provider` (keychain, OAuth), so only the `*_with_provider` functions and the no-writes path are tested. |
| src/db/emails/search.rs | `Database::search_emails_inner` | bind-index `param_idx` increments after the sender range, relaxed-token and sender-domain branches (`+= 2`, `+= 1`, `+ range_params + 1`), and after the short-subject LIKE fallback | No test combines a sender filter with a later bound filter (subject, tags, dates) in one query, so a wrong placeholder index goes unnoticed. Needs a fixture with FTS-indexed rows (`insert_emails_batch`) and combined filters. |

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
