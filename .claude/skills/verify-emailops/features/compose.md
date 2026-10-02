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
- `compose.send` Send queues the message in the local outbox for the undo window (see [Undo send and scheduled send](./outbox-undo-schedule.md)); on the demo account it then fails with "A message could not be sent: … Authentication required" (no credentials) and the outbox row is `failed`; nothing leaves the machine.
- `compose.flush-on-close` closing or leaving the composer within the autosave delay still saves the last edit.
- `compose.send-warnings` before sending, a warning appears for a promised but missing attachment and similar slips.
- `compose.ai` the composer offers AI drafting or rewriting.
- `compose.escape` Escape closes the composer.
- `compose.recipients` an address typed but not turned into a chip is still sent to.
- `compose.attachments` files can be attached and are sent.
- `compose.translate` the body can be translated before sending.

## How to get to it (user POV)

Five entry points, the columns of `## Parity`:

- **Compose modal** — Sidebar → **Compose** (`ComposeModal`).
- **Compose tab** — the modal's open-in-tab button, or Drafts → **Continue editing** (`ComposeTabView`).
- **Reply**, **Reply all**, **Forward** — the buttons of an open thread (`ReplyCompose` with `mode`, inline under the thread).

## Parity

| Capability | Compose modal | Compose tab | Reply | Reply all | Forward |
|---|---|---|---|---|---|
| compose.open | e2e:Compose/abrir | e2e:Compose/borrador con tabla | gap: untested — no test clicks Reply or checks the To it opens with | gap: untested — nothing renders mode="reply-all" or clicks Reply all | gap: untested — no test clicks Forward or checks the quoted body and empty To |
| compose.fields | gap: missing — no Bcc row; compose:showBcc is unused | gap: missing — no Bcc row | gap: missing — no Bcc row | gap: missing — no Bcc row | gap: missing — no Bcc row |
| compose.draft | e2e:Compose/cerrar y borrador | vitest:src/components/EmailView/ComposeTabView.pendingRecipient.test.tsx::autosaves the draft with the pending address | vitest:src/components/EmailView/ReplyCompose.draft.test.tsx::reopens a saved draft and keeps editing the same draft | gap: untested — keeps a draft but every draft test renders mode="reply" | gap: missing — a forward is never saved as a draft (keepsDraft = mode !== 'forward') |
| compose.discard | gap: missing — Cancel, X, backdrop and Escape only close and the close flush saves; no delete, no prompt | gap: missing — Discard only closes the tab; the draft row stays, no prompt | gap: missing — Cancel deletes the draft without the "Discard this draft?" prompt | gap: missing — same Cancel path, no prompt | gap: missing — a forward keeps no draft to discard |
| compose.tables | gap: untested — the shared editor keeps a pasted table, no test drives one through the modal | e2e:Compose/borrador con tabla | gap: untested — a restored reply draft holding a table is never tested | gap: untested — nothing renders mode="reply-all" | gap: missing — forwardQuote flattens the original to plain text, a forwarded table is lost |
| compose.conflict | integration:when_both_sides_edited_a_draft_the_local_one_wins_and_is_pushed | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |
| compose.send | e2e:Compose/enviar sin credenciales | vitest:src/components/EmailView/ComposeTabView.pendingRecipient.test.tsx::sends to the pending address | vitest:src/components/EmailView/ReplyCompose.draft.test.tsx::hands the draft id to the send so it is removed with the reply | gap: untested — no test renders mode="reply-all" and sends | vitest:src/components/EmailView/ReplyCompose.pendingRecipient.test.tsx::enables Send and sends to the pending To and Cc addresses |
| compose.flush-on-close | gap: untested — the close flush exists but only the open-in-tab flush is tested | gap: missing — autosave is a bare setTimeout cleared when the tab closes; the last edit is dropped | vitest:src/components/EmailView/ReplyCompose.draft.test.tsx::saves what was typed when the user leaves before the autosave fires | gap: untested — same code, no mode="reply-all" test | gap: missing — a forward never autosaves |
| compose.send-warnings | gap: missing — handleSend never calls findSendWarnings | gap: missing — handleSend never calls findSendWarnings | gap: untested — wired, but SendWarningBanner is only tested on its own | gap: untested — same path, no reply-all test | gap: missing — warnings are skipped for forwards |
| compose.ai | vitest:src/components/ComposeModal.aiDraft.test.tsx::stops generating when the failure arrives before the request id | gap: untested — the AI draft button is wired but no test clicks it | gap: untested — AiInstructionBar is wired but only tested on its own | gap: untested — no reply-all render drives the instruction bar | gap: missing — the instruction bar is hidden for forwards |
| compose.escape | gap: untested — handler exists, no test dispatches Escape | gap: missing — no Escape handler | gap: missing — no Escape handler | gap: missing — no Escape handler | gap: missing — no Escape handler |
| compose.recipients | gap: untested — the send merges the typed address, tests only cover the hand-over to the tab | vitest:src/components/EmailView/ComposeTabView.pendingRecipient.test.tsx::sends to the pending address | gap: untested — only mode="forward" is tested | gap: untested — only mode="forward" is tested | vitest:src/components/EmailView/ReplyCompose.pendingRecipient.test.tsx::enables Send and sends to the pending To and Cc addresses |
| compose.attachments | vitest:src/components/ComposeModal.openInTab.test.tsx::carries attached files | vitest:src/components/EmailView/ComposeTabView.pendingRecipient.test.tsx::starts with the files attached in the modal and sends them | gap: untested — file input and chips are never driven | gap: untested — no reply-all test | gap: untested — the original's files are pre-attached, no test checks they are sent |
| compose.translate | gap: untested — TranslateComposeControl is mocked to null in every test | gap: untested — mocked to null | gap: untested — mocked to null | gap: untested — mocked to null | gap: untested — mocked to null |

## Driving it with verify.sh

Preconditions: baseline; account `demo-acct-work` selected. Entry confirmed present on 11/09/2026: `button=Compose`, `button=Drafts`.

- Open → `$V wd click 'button=Compose'`, `$V wd shot "$R/compose-open.png"` → `$V wd exists 'input[placeholder="Subject"], input[placeholder^="Email subject"]'` and `$V wd exists 'textarea[placeholder^="Write your message"], [contenteditable="true"]'` print `present`.
- Fill → `$V wd type 'input[placeholder^="To"], input[aria-label="To"]' 'someone@example.com'`, `$V wd type 'input[placeholder="Subject"], input[placeholder^="Email subject"]' 'Verification run'`, then the body field → the PNG shows the text.
- Draft side effect → close with `$V wd click 'aria/Close'` (or `Minimize`), `$V wd click 'button=Drafts'` → `$V wd find '*=Verification run'` prints the draft row; `sqlite3 .emailops-demo-data/emailops.db "select count(*) from drafts where subject='Verification run'"` prints `1`.
- Discard → reopen the draft (`Continue editing`), `$V wd click 'button=Discard'`, confirm the dialog → the drafts row and DB row are gone.
- Send path → fill To/Subject, `$V wd click '[data-testid=compose-send]'` → the composer closes with a *Sending… · Undo* toast; ~10 s later the toast *A message could not be sent: … Authentication required …* appears and **Scheduled** lists the message as *Not sent* (expected: the demo account has no SMTP credentials). Delete it there. Confirmed live on 02/10/2026.
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
