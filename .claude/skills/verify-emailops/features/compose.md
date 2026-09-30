# Compose

The Compose button opens a modal to write a new email for the selected account, with
To/Cc/Bcc, Subject, body, attachments and an AI draft button. Drafts save
automatically; Discard closes without keeping one; Send hands the message to the
provider.

## Sub-features

- `compose.open` Compose opens the modal with empty To, Subject and body.
- `compose.fields` typing fills the fields; Add Cc / Add Bcc reveal extra rows.
- `compose.draft` a partially written email appears under Views → Drafts after closing.
- `compose.discard` Discard asks "Discard this draft?" and removes it.
- `compose.tables` a draft or a reply that holds a table keeps it in the editor (sections, captions, cell styles) and in the HTML that is saved and sent.
- `compose.conflict` a draft edited locally is never dropped by a sync: the dirty marker (V028) makes the local version win and be pushed.
- `compose.send` Send with the demo account fails with "Failed to send" (no credentials); nothing leaves the machine.

## How to get to it (user POV)

- Sidebar → **Compose**; or Reply / Reply All from an open thread; or "Continue editing" from Drafts.

## Driving it with verify.sh

Preconditions: baseline; account `demo-acct-work` selected. Entry confirmed present on 11/09/2026: `button=Compose`, `button=Drafts`.

- Open → `$V wd click 'button=Compose'`, `$V wd shot "$R/compose-open.png"` → `$V wd exists 'input[placeholder="Subject"], input[placeholder^="Email subject"]'` and `$V wd exists 'textarea[placeholder^="Write your message"], [contenteditable="true"]'` print `present`.
- Fill → `$V wd type 'input[placeholder^="To"], input[aria-label="To"]' 'someone@example.com'`, `$V wd type 'input[placeholder="Subject"], input[placeholder^="Email subject"]' 'Verification run'`, then the body field → the PNG shows the text.
- Draft side effect → close with `$V wd click 'aria/Close'` (or `Minimize`), `$V wd click 'button=Drafts'` → `$V wd find '*=Verification run'` prints the draft row; `sqlite3 .emailops-demo-data/emailops.db "select count(*) from drafts where subject='Verification run'"` prints `1`.
- Discard → reopen the draft (`Continue editing`), `$V wd click 'button=Discard'`, confirm the dialog → the drafts row and DB row are gone.
- Send path → fill To/Subject, `$V wd click 'button=Send'` → `$V wd find '*=Failed to send'` prints the error banner (expected: the demo account has no SMTP credentials).
- Table draft → Views → **Drafts**, row *Milestone dates (table)* → **Continue editing** opens it in a compose tab: `$V wd js 'document.querySelectorAll("[contenteditable=true] table tr").length'` prints `3`. Put the caret at the end of a cell and `document.execCommand('insertText', false, ' (tbc)')` → the cell text changes, the table keeps its 3 rows, and within ~2 s `sqlite3 .emailops-demo-data/emailops.db "select body_html from drafts where subject='Milestone dates (table)'"` holds the edited cell inside the same `<table>`. Delete the typed text again and close with the tab's `aria-label="Close tab"` button (hidden until hover: click it through the DOM). Confirmed live on 30/09/2026.

## Gotchas

- The body may be a rich-text editor (`contenteditable`) rather than a textarea; if `wd type` reports the value did not land, click it and use `wd keys` per character, or cua-driver `type_text`.
- The modal is the shared `Modal`: a click that starts inside and ends outside does not close it.
- "Generate with AI" needs the local model and retrieval; treat it as the chat feature's precondition set.
- Always Discard your draft before cleanup, or the next run starts with a stray row in Drafts.
- A draft opened from Drafts is a compose **tab**, not the modal: there is no `.fixed` overlay around it.

| Case | Test kind |
|---|---|
| outgoing HTML: tables, safe inline styles, inline images | unit + vitest (`services::emails::html_sanitizer`, `util::html`, `composeHtml`, `composeEditorExtensions`) |
| draft store, dirty marker, sync plan for every local/upstream combination | unit (`db::drafts`, `sync::draft_plan`, `services::emails::drafts`) |
| draft conflicts, offline saves, replies and sends against the fake provider | integration (`*draft*`, `*reply*`, `send_*`) |
| draft command arguments, `Draft` / `SaveDraftRequest` shapes | contract (`src/lib/apiContract/compose.api.test.ts`) |
| modal, tab, recipients, AI draft | vitest (`ComposeModal.*`, `ComposeTabView.*`, `draftRequest`, `sendWarnings`) |
| open, fill, send without credentials, draft, discard, table draft | e2e (`Compose/*`, `Vistas/Drafts`) |
| drafts and translation quality | eval (`draft*`, `translation_eval`) |
