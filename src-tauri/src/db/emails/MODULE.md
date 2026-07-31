# db/emails

## What this module owns

All SQL queries that read or write the `emails`, `email_bodies`, `email_tags`, and `email_attachment_meta` tables.

- **crud.rs** — INSERT / UPDATE / DELETE: `upsert_email`, `mark_as_read`, `delete_email`, `mark_body_downloaded`, etc. Also the optimistic-sent surface (V010 `emails.pending_sync`): `insert_sent_email_local` (single-row insert that mirrors batch.rs — emails row + body + manual FTS — and sets `pending_sync`), `get_pending_sent_emails`, `delete_pending_sent_emails` (hard delete, guarded to `pending_sync = 1` rows), `update_email_thread_id`, `clear_stale_pending_sent`.
- **batch.rs** — batch upsert for sync loops: `upsert_emails_batch(conn, emails)` — always runs in one transaction.
- **search.rs** — FTS5 and structured filter queries: `search_emails`, `get_filtered_emails`, `get_emails_with_cursor`
- **autocomplete.rs** — sender/recipient autocomplete (`autocomplete_senders`, `autocomplete_recipients`). Both scope the contact pool to `is_deleted = 0 AND mailbox NOT IN ('spam','trash')` — a spam sender is not a correspondent, and suggesting one puts a phishing address one keystroke from being mailed. Custom IMAP folders (`mailbox = 'folder:…'`) stay in scope. `autocomplete_recipients` ranks `domain_match → direct_contact → freq → recency`, where `direct_contact` demotes addresses that only ever shared a To/Cc line on *received* mail (strangers on misdirected/harvested mail) below people the user actually corresponds with. Both then drop machine-generated envelope addresses (`is_machine_generated_address`: VERP `=` locals, opaque random tokens, hex/UUID tags, `bounces.` domains); `autocomplete_recipients` additionally drops unattended mailboxes (`is_no_reply_address`), which `autocomplete_senders` keeps because a no-reply sender is a useful search facet. The queries over-fetch 12x so a prefix dominated by filtered addresses still returns a full page.
- **contacts.rs** — contact aggregation queries (sender stats, company grouping)
- **failed.rs** — queries for emails with missing/failed bodies (used by redownload flow)
- **test_helpers.rs** — `insert_email_for_test`, `insert_email_body_for_test` helpers for `#[cfg(test)]` modules
- **mod.rs** — re-exports

## Account scoping (`AccountScope`)

List/count/filter/position reads take a `crate::db::AccountScope` instead of a
bare `account_id`:

- `AccountScope::Account(id)` — the default, single-account behavior.
- `AccountScope::AllEnabled` — the deliberate cross-account path backing the
  unified ("All accounts") inbox. Spans every account with
  `accounts.enabled = 1`. This keeps cross-account reads a visible, greppable
  opt-in per call site (satisfying the "no cross-account read paths by
  default" guardrail) — commands map `account_id: Option<String>` at the IPC
  boundary (`None` = unified).

**Collision rule:** `thread_id` is NOT globally unique (two accounts CC'd on
one provider thread share the same string; Outlook falls back to message ids).
Every dedup/count/CTE under `AllEnabled` must key on `(account_id, thread_id)`
— see `get_emails` (inbox dedup), `count_emails`, `get_filtered_emails`,
`get_quick_filter_stats`, `get_tag_stats`, `count_filter_threads`,
`get_email_inbox_position`. The unified list ordering is served by
`idx_emails_mailbox_active (mailbox, is_deleted, timestamp DESC, id DESC)`
(V009).

## Access rules (enforced by CLAUDE.md)

| Operation | Must use |
|-----------|----------|
| SELECT | `db.reader()` |
| INSERT / UPDATE / DELETE / DDL | `db.connection()` |
| Read-then-write (TOCTOU) | `db.connection()` for both |

## Performance guidelines

- Thread-latest queries use `GROUP BY thread_id, MAX(timestamp)` — never `NOT EXISTS`.
- Broad filters (domain/sender) drive from `idx_emails_account_active` with LIMIT; selective filters (tags) drive from `email_tags`.
- `LIKE 'prefix%'` is converted to `>= / <` range bounds to avoid parameter-plan problems.

## What should NOT live here

- Business logic (classification decisions, draft assembly) — `services/`
- Vector/embedding queries — `db/embeddings.rs`
- Schema migrations — `db/schema.rs`
