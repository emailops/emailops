# Lenses, tasks and attachments

Lenses turn matching mail into table rows with a model; Tasks lists what the mail asks of the
user; Attachments collects files through rules. Every attachment file written to disk carries
the OS quarantine mark, and opening one whose type can run code asks first.

## Sub-features

- `lens.create` / `lens.run` create from a template or from the chat, backfill, incremental extraction after each sync; one failing Lens does not stop the others.
- `tasks.list` extracted tasks with status.
- `attachments.rules` rules collect files by sender / subject / filename, with suggestions.
- `attachments.open` a stored file opens with the OS default app; a program, script, installer, shortcut or web page is refused until confirmed (`DangerousAttachmentDialog`).
- `attachments.preview` images, PDF, Word (.docx) and HTML open inside the app; HTML and Word render like an email body (sanitized, `sandbox="allow-scripts"` frame, remote content by consent), the Word page stays white on the dark theme; any other framed type gets an empty `sandbox`.
- `attachments.chat` an open PDF or Word tab (or one selected in **Attachments**) is the chat's context: the chip names the file, the document starters show, and the answer comes from its text.

## How to get to it (user POV)

- Sidebar → **Lenses**, **Tasks**, **Attachments**.
- An open email → **Attachments (n)** chips under the body.

## Driving it with verify.sh

Preconditions: baseline. The demo email *Corrected Larkspur Freight renewal quote* (find it with the search box: `Larkspur`) carries `larkspur-client-portal.webloc` (a shortcut stored on disk) and `larkspur-renewal-terms.html` (inline). Confirmed live on 30/09/2026.

- Confirmation → open the email, `$V wd click 'button[title^="larkspur-client-portal.webloc"]'` → `$V wd exists '[data-testid="cancel-open-attachment"]'` prints `present`; the dialog names the file and says it is a shortcut. `$V wd click '[data-testid="cancel-open-attachment"]'` closes it. **Never** click `confirm-open-attachment`: that hands the file to the OS.
- Preview → `$V wd click 'button[title^="larkspur-renewal-terms.html"]'` → the tab shows the page like an email body: `iframe[title="Email content"]` with `sandbox="allow-scripts"`, its remote image without a `src`, and *Remote images were blocked…* with **Show images** (never click it on a real sender); close with `button[title="Close"]`. Driven live 31/10/2026.
- Word → open *Kickoff minutes: dispatch portal* (`//div[@role="button"][contains(., "Ines Okafor")][contains(., "Kickoff minutes")]`), `$V wd click 'button[title^="larkspur-kickoff-minutes.docx"]'` (its title ends `click to view`) → no `cancel-open-attachment` dialog; one `iframe[title="Email content"]` whose `srcdoc` holds `docx-wrapper`, `17 March 2026` and `<meta name="color-scheme" content="light">`, no `invert(1)` even on the dark theme; the chat chip reads `larkspur-kickoff-minutes.docx` · **Using as context**. Asking *When is the dispatch board prototype due, and who delivers it?* answers 17 March 2026 / Ulises. Driven live 18/11/2026.
- Views → `$V wd click 'button*=Lenses'`, `button*=Tasks`, `button*=Attachments` render content.

## Gotchas

- A type the app previews (image, PDF, HTML) never reaches the confirmation dialog: it is not handed to the OS. Use a non-previewable dangerous type to see the dialog.
- The viewer reads the downloaded file (`file_path`), then inline data (`INLINE::<filename>`), then asks the provider (`fetch_meta_bytes`); only a demo attachment with none of the first two fails, because the demo accounts have no credentials.
- Lens extraction needs the chat model; its quality is measured by `make eval-lenses`, not by the sweep.

| Case | Test kind |
|---|---|
| danger classification, quarantine mark, refusal without confirmation | unit (`services::attachment_safety`, `services::attachments`) |
| rules, suggestions, retroactive apply | unit (`services::attachments`, `services::attachment_suggestions`) |
| Lens scope, extraction, coercion, isolation of a failing Lens, stop from the queue | unit on an in-memory DB (`services::lenses::*`, `db::lenses`) |
| rule collects a file on sync; a dangerous collected file is refused until confirmed | integration (`a_headless_sync_applies_attachment_rules_to_new_mail`, `a_dangerous_attachment_collected_by_a_rule_is_refused_until_confirmed`, …) |
| an HTML attachment renders like an email body, with remote content by consent | vitest (`EmailView/AttachmentTabView.render`, `Attachments/AttachmentViewer.render`) + e2e (`Adjuntos/página web como el cuerpo de un correo`) |
| a Word attachment opens in the app, sanitized and paper-white; .doc/.docm still go to the OS | vitest (`lib/docxPreview`, `Attachments/DocxViewer`, `EmailView/EmailAttachments.view`, `EmailView/AttachmentTabView.render`) |
| the chat reads an open Word document | unit (`services::docx_text`, `services::chat::open_document`) + chat eval (`demo_open_docx_table_fact`, `demo_open_docx_budget_es`) |
| the chat's Lens tools leave out a row of deleted mail | unit (`db::lenses`) + eval (`lens_rows_leave_out_trashed_mail` in `retrieval_exclusions.yaml`) |
| Lens CRUD and runs | integration (`*lens*`, `backfill_*` — the latter are counted under Accounts by their name) |
| attachment and Lens command arguments (`confirmed` included) | contract (`src/lib/apiContract/lenses.api.test.ts`) |
| dialog, hook, viewer, rule modal, stores | vitest (`useOpenAttachment`, `AttachmentTabView`, `Attachments/*`, `attachmentStore`, `Lenses/*`) |
| confirmation dialog, sandboxed preview, the three views | e2e (`Adjuntos/*`, `Vistas/{Attachments,Tasks,Lenses}`) |
| template extraction quality | eval (`make eval-lenses`) |
