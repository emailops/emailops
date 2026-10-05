# Lenses, tasks and attachments

Lenses turn matching mail into table rows with a model; Tasks lists what the mail asks of the
user; Attachments collects files through rules. Every attachment file written to disk carries
the OS quarantine mark, and opening one whose type can run code asks first.

## Sub-features

- `lens.create` create a Lens from a template or from the chat.
- `lens.run` backfill, incremental extraction after each sync; one failing Lens does not stop the others.
- `lens.read` a Lens's table lists its rows with sort and column filters; the chat answers from a Lens's rows.
- `tasks.list` extracted tasks with status.
- `tasks.create` the user adds a task by typing it, or asks the chat to.
- `tasks.status` a task is marked done, snoozed or dismissed.
- `attachments.rules` rules collect files by sender / subject / filename, with suggestions.
- `attachments.open` a stored file opens with the OS default app; a program, script, installer, shortcut or web page is refused until confirmed (`DangerousAttachmentDialog`).
- `attachments.preview` images, PDF and HTML open inside the app; HTML in an iframe with an empty `sandbox`.
- `attachments.download` a file is saved to Downloads (one from an email, several from the Attachments view).
- `attachments.quarantine` every attachment file written to disk carries the OS quarantine mark.

## How to get to it (user POV)

Three entry points, the columns of `## Parity`:

- **Sidebar view** — Sidebar → **Lenses**, **Tasks** or **Attachments**, whichever the row is about.
- **Open email** — the **Attachments (n)** strip under a message, its preview tab and the image lightbox.
- **Chat** — a lens request fills the Create Lens form; attachment chips in answers; tools `list_lenses`, `get_lens_data`, `list_pending_tasks`, `create_task`.

## Parity

| Capability | Sidebar view (Lenses / Tasks / Attachments) | Open email (attachment strip) | Chat |
|---|---|---|---|
| lens.create | vitest:src/components/Lenses/LensCreateModal.template.test.tsx::opens the prefilled form instead of creating the Lens | n/a: a Lens is defined over a scope of mail, not from one open email | e2e:Chat/Formularios/rellenar Crear Lens desde el chat |
| lens.run | integration:lens_on_emails_synced_extracts_matching | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |
| lens.read | vitest:src/components/Lenses/LensColumnFilterMenu.test.tsx::applies the ticked values | n/a: Lens rows are read in the Lens table, not from one email | gap: untested — get_lens_data / list_lenses only have validation and gating tests; none returns rows |
| tasks.list | e2e:Vistas/Tasks | n/a: tasks are listed per account; the Tasks view previews the source email | gap: untested — the list_pending_tasks tool has no test; only the DB query is tested |
| tasks.create | gap: untested — the add-task form has no component test; only the store is tested | gap: missing — an open email offers no add-task action, although create_task accepts a source email | gap: untested — the create_task tool has no test; only the service is tested |
| tasks.status | gap: untested — done / snooze / dismiss have no component test | n/a: status is changed on the task, which an open email does not list | gap: missing — no chat tool changes a task's status |
| attachments.rules | vitest:src/components/Attachments/RuleManagementModal.suggestions.test.tsx::creating the reviewed rule applies it to existing mail and accepts the suggestion | gap: missing — the strip offers no rule; the prefilled rule is only on the inbox row / card ⋮ menu | gap: missing — the chat can fill only the Create Lens form |
| attachments.open | gap: untested — Open externally in the Attachments viewer is never driven | e2e:Adjuntos/tipo peligroso pide confirmación | gap: untested — chat chips open through the same hook with a fallback; no test clicks one |
| attachments.preview | gap: missing — the Attachments view previews HTML in an iframe with sandbox="allow-same-origin", not the empty sandbox | e2e:Adjuntos/página web en vista previa aislada | gap: missing — chat chips always hand the file to the OS; no in-app preview |
| attachments.download | gap: untested — bulk download has no test; the toolbar's only test is the suggestion badge | gap: untested — the strip's download and the preview tab's download have no test | gap: missing — a chat attachment chip can only open |
| attachments.quarantine | rust:src-tauri/src/services/attachment_safety.rs::an_opened_file_carries_the_quarantine_mark | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |

## Driving it with verify.sh

Preconditions: baseline. The demo email *Corrected Larkspur Freight renewal quote* (find it with the search box: `Larkspur`) carries `larkspur-client-portal.webloc` (a shortcut stored on disk) and `larkspur-renewal-terms.html` (inline). Confirmed live on 30/09/2026.

- Confirmation → open the email, `$V wd click 'button[title^="larkspur-client-portal.webloc"]'` → `$V wd exists '[data-testid="cancel-open-attachment"]'` prints `present`; the dialog names the file and says it is a shortcut. `$V wd click '[data-testid="cancel-open-attachment"]'` closes it. **Never** click `confirm-open-attachment`: that hands the file to the OS.
- Preview → `$V wd click 'button[title^="larkspur-renewal-terms.html"]'` → `iframe[title="larkspur-renewal-terms.html"]` has `sandbox=""` and a `data:text/html;base64,` source; close with `button[title="Close"]`.
- Views → `$V wd click 'button*=Lenses'`, `button*=Tasks`, `button*=Attachments` render content.

## Gotchas

- A type the app previews (image, PDF, HTML) never reaches the confirmation dialog: it is not handed to the OS. Use a non-previewable dangerous type to see the dialog.
- The viewer reads inline data (`INLINE::<filename>`) or asks the provider; a demo attachment with only a `file_path` shows "Failed to load attachment" because the demo accounts have no credentials.
- Lens extraction needs the chat model; its quality is measured by `make eval-lenses`, not by the sweep.

| Case | Test kind |
|---|---|
| danger classification, quarantine mark, refusal without confirmation | unit (`services::attachment_safety`, `services::attachments`) |
| rules, suggestions, retroactive apply | unit (`services::attachments`, `services::attachment_suggestions`) |
| Lens scope, extraction, coercion, isolation of a failing Lens, stop from the queue | unit on an in-memory DB (`services::lenses::*`, `db::lenses`) |
| rule collects a file on sync; a dangerous collected file is refused until confirmed | integration (`a_headless_sync_applies_attachment_rules_to_new_mail`, `a_dangerous_attachment_collected_by_a_rule_is_refused_until_confirmed`, …) |
| Lens CRUD and runs | integration (`*lens*`, `backfill_*` — the latter are counted under Accounts by their name) |
| attachment and Lens command arguments (`confirmed` included) | contract (`src/lib/apiContract/lenses.api.test.ts`) |
| dialog, hook, viewer, rule modal, stores | vitest (`useOpenAttachment`, `AttachmentTabView`, `Attachments/*`, `attachmentStore`, `Lenses/*`) |
| confirmation dialog, sandboxed preview, the three views | e2e (`Adjuntos/*`, `Vistas/{Attachments,Tasks,Lenses}`) |
| template extraction quality | eval (`make eval-lenses`) |
