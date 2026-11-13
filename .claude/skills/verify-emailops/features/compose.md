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
- `compose.sender` a new blank message starts from Settings → Appearance → **Send new messages from** (default: the selected view's account); replies, forwards and drafts keep their own account; every From list puts the account with the most stored sent mail (`is_sent`) first.
- `compose.send` Send queues the message in the local outbox for the undo window (see [Undo send and scheduled send](./outbox-undo-schedule.md)); on the demo account it then fails with "A message could not be sent: … Authentication required" (no credentials) and the outbox row is `failed`; nothing leaves the machine.

## How to get to it (user POV)

- Sidebar → **Compose**; or Reply / Reply All from an open thread; or "Continue editing" from Drafts.

## Driving it with verify.sh

Preconditions: baseline; account `demo-acct-work` selected. Entry confirmed present on 11/09/2026: `button=Compose`, `button=Drafts`.

- Open → `$V wd click 'button=Compose'`, `$V wd shot "$R/compose-open.png"` → `$V wd exists 'input[placeholder="Subject"], input[placeholder^="Email subject"]'` and `$V wd exists 'textarea[placeholder^="Write your message"], [contenteditable="true"]'` print `present`.
- Fill → `$V wd type 'input[placeholder^="To"], input[aria-label="To"]' 'someone@example.com'`, `$V wd type 'input[placeholder="Subject"], input[placeholder^="Email subject"]' 'Verification run'`, then the body field → the PNG shows the text.
- Draft side effect → close with `$V wd click 'aria/Close'` (or `Minimize`), `$V wd click 'button=Drafts'` → `$V wd find '*=Verification run'` prints the draft row; `sqlite3 .emailops-demo-data/emailops.db "select count(*) from drafts where subject='Verification run'"` prints `1`.
- Discard → reopen the draft (`Continue editing`), `$V wd click 'button=Discard'`, confirm the dialog → the drafts row and DB row are gone.
- Send path → fill To/Subject, `$V wd click '[data-testid=compose-send]'` → the composer closes with a *Sending… · Undo* toast; ~10 s later the toast *A message could not be sent: … Authentication required …* appears and **Scheduled** lists the message as *Not sent* (expected: the demo account has no SMTP credentials). Delete it there. Confirmed live on 02/10/2026.
- Table draft → Views → **Drafts**, row *Milestone dates (table)* → **Continue editing** opens it in a compose tab: `$V wd js 'document.querySelectorAll("[contenteditable=true] table tr").length'` prints `3`. Put the caret at the end of a cell and `document.execCommand('insertText', false, ' (tbc)')` → the cell text changes, the table keeps its 3 rows, and within ~2 s `sqlite3 .emailops-demo-data/emailops.db "select body_html from drafts where subject='Milestone dates (table)'"` holds the edited cell inside the same `<table>`. Delete the typed text again and close with the tab's `aria-label="Close tab"` button (hidden until hover: click it through the DOM). Confirmed live on 30/09/2026.

- Default sender and From order → precondition: the demo generator writes its 2 Sent rows without `is_sent`, so run V014's backfill on the throwaway DB (`sqlite3 .emailops-demo-data/emailops.db "UPDATE emails SET is_sent = 1 WHERE mailbox = 'sent'"`); to see the reorder, also put the work account last (`UPDATE accounts SET sort_order = …`) and `$V wd js 'location.reload()'`. Select `ulises@fastmail.com` in the sidebar, `$V wd click 'button=Compose'` → `$V wd js 'document.querySelector("select[aria-label=From]").value'` is `demo-acct-personal` and the options start with `demo-acct-work` (the only account with sent mail). Settings → Appearance: the native `select[aria-label="Send new messages from"]` cannot be clicked through WebDriver, so set its value and dispatch `change` with `wd js` → `user_preferences` holds `compose.default_from_account = 'demo-acct-calendar'`; Compose then opens from `demo-acct-calendar` with the personal view still selected, and **Reply** on a personal-account message stays on `demo-acct-personal`. Setting it back to the view stores `''`. Confirmed live on 13/11/2026 (T-0061), evidence in `src-tauri/reports/verify/20261113-090015/`.

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
| default sender for new messages, From order by stored sent mail | unit (`db::accounts::sent_count_tests`) + vitest (`composeSender`, `useSenderOptions`, `DefaultSenderSetting`, `ComposeModal.senders`) |
| open, fill, send without credentials, draft, discard, table draft | e2e (`Compose/*`, `Vistas/Drafts`) |
| closing a composer or a compose tab that holds files asks; Escape closes only the question | vitest (`ComposeModal.close`, `ReadingPane.composeClose`) + e2e (`Compose/cerrar con archivos pregunta`, `Compose/cerrar una pestaña con archivos pregunta`) |
| a compose tab keeps its text and files behind a thread; Discard removes the draft | vitest (`ReadingPane.composeTabs`, `ComposeTabView.discard`) + e2e (`Compose/la pestaña conserva texto y archivos`, `Compose/Discard en una pestaña borra el borrador`) |
| the reply opened from a chat draft saves over that draft and leaves none once sent | vitest (`ReplyCompose.draft`, `composeDraft`, `chatToolEffects`) + e2e (`Compose/la respuesta de un borrador del chat no deja borrador`) |
| drafts and translation quality | eval (`draft*`, `translation_eval`) |
