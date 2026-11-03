# EO Docs

Documents and sheets several EmailOps users edit together with no server: each install keeps a
Yjs CRDT per document, and changes travel as an `.eodoc` attachment by email. Sharing asks for
consent per document; a change goes out after two minutes without editing, on **Send now** or
when the document closes. The design, limits and gaps are in `docs/EO-DOCS.md`.

## Sub-features

- `docs.create` **New** (`shared-doc-new`, `shared-doc-new-title`, `shared-doc-create`) makes a document or a sheet, which opens in its editor (`shared-doc-editor` for text).
- `docs.edit` text through TipTap bound to the `Y.Doc`, saved to `shared_docs.state` in batches; undo only reaches this person's own changes.
- `docs.links` a link in a document never navigates: it renders with no `target`, since tauri-plugin-shell opens any `target=_blank` link on its own, a remote image keeps no address to fetch, and a click in any view opens `ExternalLinkDialog` with the real address.
- `docs.history` the panel (`shared-doc-history-toggle`) lists versions (`shared-doc-version-<id>`); a version opens read-only with `shared-doc-version-banner`.
- `docs.share` **Share** (`shared-doc-share`) names the recipients and asks for consent; an invitation carries the full state and a readable HTML copy.
- `docs.arrivals` sync hands every `.eodoc` message to the pure planner: apply, create an invitation or ignore; updates are marked read and archived, invitations stay in the inbox.
- `docs.send` the outbox loop sends per document with backoff, to every participant.
- `docs.organize` folders, move, search, delete (a tombstone stops it coming back).
- `docs.import` Word and spreadsheet files, from **Import** or from an email attachment.

## How to get to it (user POV)

- Sidebar → **EO Docs** (`sidebar-documents`), on unless Settings → EO Docs turns it off.
- A composer → attach menu → from EO Docs.
- An email with an `.eodoc` invitation or an Office attachment → **Open in EO Docs**.

## Driving it with verify.sh

Preconditions: baseline; the demo DB has no documents. Handles confirmed live on 31/10/2026.

- Create → `$V wd click '[data-testid="sidebar-documents"]'`, `$V wd click '[data-testid="shared-doc-new"]'`, `$V wd type '[data-testid="shared-doc-new-title"]' 'Verification document'`, `$V wd click '[data-testid="shared-doc-create"]'` → `[data-testid="shared-doc-editor"]` renders and `select title, status from shared_docs` has the row.
- Link → paste HTML by dispatching a `ClipboardEvent('paste')` with a `DataTransfer` on the editor (`sweep.mjs`, `EO Docs/enlace e imagen de otra persona`), open **History**, pick the version and click the link → `[data-testid="external-link-open"]`. Click **Cancel**, never **Open in browser**.
- Delete → `$V wd click '[data-testid="shared-doc-delete"]'`, `$V wd click '[data-testid="delete-doc-confirm"]'`.

## Gotchas

- A click on a link asks in every view, editable or read-only. Seeing the dialog does not prove the browser stayed shut: tauri-plugin-shell opens any click on a `target="_blank"` link itself, past the dialog, so the rendered `<a>` must carry no `target`.
- Sharing and arrivals need a mail provider: the demo accounts have no credentials, so they are proven against the fake provider and the planner, never in the sweep.
- A document saves in batches of 400 ms, and the first version appears a moment after typing: wait before opening History.

| Case | Test kind |
|---|---|
| arrival, flush, recipients, backoff, sender checks | unit on the planner (`services::shared_docs::planner`) |
| arrivals, merge race, send backoff, archive of one message | unit on an in-memory DB (`services::shared_docs::tests`, `db::shared_docs`) |
| a sync applies shared document messages and archives the updates | integration (`sync_applies_shared_document_messages_and_archives_the_updates`) |
| shared document command arguments and shapes | contract (`src/lib/apiContract/sharedDocs.api.test.ts`) |
| editors, undo, history, share and delete dialogs, schema of links and images, conflicts, sheets, import | vitest (`Documents/*`, `docSchema`, `sheet*`, `officeImport`, `sharedDocsStore`, `useSharedYDoc`) |
| create and open, link and image from someone else, read-only link dialog, delete | e2e (`EO Docs/*` in `sweep.mjs`) |
| eval | n/a: no model reads or writes a document |
