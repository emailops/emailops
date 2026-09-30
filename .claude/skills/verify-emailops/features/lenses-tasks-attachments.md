# Lenses, tasks and attachments

Lenses turn matching mail into table rows with a model; Tasks lists what the mail asks of the
user; Attachments collects files through rules. Every attachment file written to disk carries
the OS quarantine mark, and opening one whose type can run code asks first.

## Sub-features

- `lens.create` / `lens.run` create from a template or from the chat, backfill, incremental extraction after each sync; one failing Lens does not stop the others.
- `tasks.list` extracted tasks with status.
- `attachments.rules` rules collect files by sender / subject / filename, with suggestions.
- `attachments.open` a stored file opens with the OS default app; a program, script, installer, shortcut or web page is refused until confirmed (`DangerousAttachmentDialog`).
- `attachments.preview` images, PDF and HTML open inside the app; HTML in an iframe with an empty `sandbox`.

## How to get to it (user POV)

- Sidebar → **Lenses**, **Tasks**, **Attachments**.
- An open email → **Attachments (n)** chips under the body.

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
