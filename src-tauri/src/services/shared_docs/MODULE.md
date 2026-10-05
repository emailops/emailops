# services/shared_docs

Shared documents and sheets that several EmailOps users edit together, with
email as the only transport: no server, no cloud.

## What it owns

- **The CRDT** (`crdt.rs`): a document is one merged Yjs v1 update, handled
  with `yrs` so the webview's `yjs` writes the same bytes. Merging works on
  encoded updates, so an update that arrives before the one it depends on is
  kept and integrates once the gap is filled.
- **The envelope** (`envelope.rs`, pure): the `.eodoc` JSON attachment a
  message carries — document id, kind, title, participants, the sender's state
  vector and the update. A message is a document message because of this
  attachment's file name; no custom header is needed (the send paths cannot
  set one, and the sync keeps an allowlist).
- **The decisions** (`planner.rs`, pure): what an arriving envelope does
  (`plan_arrival`), who a document's mail goes to, and which documents are
  due to be mailed (`due_flushes`, 2-minute pause after the last edit).
- **The executors** (`mod.rs`): create, share, accept, leave, delete, local edits,
  `flush` / `flush_due` (mail pending changes) and `ingest_arrivals` (the sync
  hook).

## Folders, history and search (EO Docs)

- Folders (`create_folder`, `move_doc`, …) are this install's own; nothing
  about them is mailed.
- Every change is recorded as a version (`record_doc_version`): local edits by
  the same author within 5 minutes collapse into one, each arrival is its own.
  Versions are read-only (`versions`, `version_state`).
- `reindex` refreshes the FTS5 entry (title + `crdt::plain_text`) after every
  content change; `search` matches every word as a prefix.
- `delete` removes the document, its history and its search entry, and keeps a
  tombstone (`shared_doc_tombstones`, V038) so `ingest_arrivals` ignores later
  mail about it instead of storing it as a new invitation. Nothing is mailed:
  the other participants keep their copies.

## How changes travel

1. Sharing mails an invitation with the whole document. Sharing (or accepting
   an invitation) is the user's consent to the automatic mails that follow.
2. Local edits mark the document dirty. After a pause, `flush` mails what the
   least up-to-date recipient lacks (diff against the minimum of the known
   state vectors), then assumes every recipient has it.
3. `ingest_arrivals` first refuses a message whose sender fails the
   provider's authentication (`planner::sender_rejection` over
   `junk::auth::assess`: DMARC fail, or SPF fail with no DMARC policy and no
   valid DKIM; only Gmail and Outlook verdicts can be attributed, so IMAP is
   not covered). Then it merges each envelope, records the sender's state vector,
   and marks the document dirty when the sender lacks something or this
   install is missing a dependency — so the next flush catches the other side
   up. A lost message is made good this way; duplicates are harmless.
4. Update messages are marked read and archived; an invitation stays in the
   inbox. The owner's other installs pick their own messages up from Sent.

## Depends on

`db::shared_docs` (V036), `services::emails` (send, thread archive),
`services::outbox::OutboxProviders` (provider per account for the background
flush, run by `sync_scheduler::outbox_dispatch_loop`), `sync::provider`.

## Not here

- Rendering or editing content: the webview owns the Yjs document and the
  editors; the backend never interprets a document's content.
- Encryption: messages are as private as the rest of the user's mail
  (provider + TLS). See DECISIONS 2026-10-04.
