# Settings, language and updates

The settings dialog (appearance, signatures, notifications, AI tabs, privacy and security, junk, calendar), the UI
language, and the update check. AI provider settings have their own recipe
([ai-providers.md](./ai-providers.md)).

## Sub-features

- `settings.tabs` each tab renders.
- `settings.close` Escape and **Close settings** close the dialog wherever it was opened from, dropping any prefill.
- `settings.aiOff` with AI off, tabs that need a model are hidden and AI deep links fall back to a visible tab.
- `settings.privacy` remote content, trusted senders, password; load and save failures are shown.
- `settings.language` UI language and AI output language; four locales with the same keys.
- `settings.updates` update check and install prompt.
- `settings.guard` the generic preference command refuses `security.*` and `app_data_dir` keys.

## How to get to it (user POV)

Five entry points, the columns of `## Parity`:

- **Sidebar gear** — gear at the bottom of the sidebar (`aria/Application settings`), opens on Appearance.
- **Chat** — the context-budget note's settings button opens the AI tab; a guide answer's `navigateTo settings/<tab>` opens any allowlisted tab.
- **Logs status bar** — the gear next to the model selector ("AI settings") opens the AI tab.
- **Classification shortcut** — the Tag Board empty state ("Open settings") and the row / card ⋮ "Create classification rule" open the Classification tab with a prefill.
- **Update notice** — the startup "update available" toast and the sidebar footer link (not in the dialog).

## Parity

| Capability | Sidebar gear | Chat (budget note, guide link) | Logs status bar | Classification shortcut | Update notice |
|---|---|---|---|---|---|
| settings.tabs | e2e:Ajustes/pestaña AI Backend | vitest:src/components/Chat/MessageBubble.budget.test.tsx::opens the AI settings from the note when a handler is wired | gap: untested — the Logs bar gear is never clicked; IA/barra de Logs only reads the bar | gap: untested — the Tag Board empty state and the row ⋮ classification rule open the Classification tab; no test | n/a: the update notice opens the release page, not Settings |
| settings.close | e2e:Ajustes/Escape cierra el diálogo | gap: untested — a dialog opened on the AI tab from the chat is never closed in a test | gap: untested — a dialog opened from the Logs bar is never closed in a test | gap: untested — closing must also drop the classification prefill; no test | n/a: the update notice opens no dialog |
| settings.privacy | vitest:src/components/Settings/PrivacySettings.errors.test.tsx::rolls the remote-content toggle back and says so when saving fails | gap: untested — a guide answer can open settings/privacy, tested only in the lib helper | n/a: the Logs bar opens the AI tab only | n/a: the shortcut opens the Classification tab only | n/a: the update notice opens no settings tab |
| settings.language | gap: untested — the UI language and AI output language selects have no test through the dialog; only the shared widget is tested | gap: untested — the budget note opens the AI tab, where the AI output language lives; no test reaches it from there | gap: untested — the Logs bar opens the AI tab, where the AI output language lives; no test | n/a: the shortcut opens the Classification tab only | n/a: the update notice opens no settings tab |
| settings.updates | gap: missing — the dialog has no update check, version or update-available entry | n/a: no settings tab hosts updates, so no guide link can open one | n/a: the Logs bar opens the AI tab only | n/a: the shortcut opens the Classification tab only | gap: untested — the toast logic is tested only in lib/store tests; the App listener and sidebar link have no test |
| settings.guard | rust:src-tauri/src/commands/preferences.rs::reserved_pref_keys_are_rejected_for_writes | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point | n/a: backend, same path for every entry point |
| settings.aiOff | vitest:src/components/Settings/SettingsDialog.aiOff.test.tsx::still hides the tabs that need a model | n/a: the chat itself needs AI on | gap: untested — the AI tab the Logs bar opens stays visible with AI off; no test opens it with AI off | gap: missing — with AI off the row ⋮ Create classification rule is still offered but its tab is hidden, so Settings silently opens on Appearance | n/a: the update notice does not depend on AI |

## Driving it with verify.sh

Preconditions: baseline.

- Open → `$V wd click 'aria/Application settings'` → the tab list shows Appearance, AI Backend & Models, AI Classification, Privacy & Security, Junk.
- Each tab → click the **last** button containing the tab label (the sidebar has a `Calendar` button too) → content renders.
- Close → `$V wd keys Escape`, or `$V wd click 'aria/Close settings'`.

## Gotchas

- "Close settings" is a `title`, and the dialog has no `role="dialog"`: scope through `button[title="Close settings"]` → `.closest('.fixed')`.
- Preferences are stored in SQLite (`user_preferences`), never in `localStorage`; a driven change persists in the demo DB, so put it back.

| Case | Test kind |
|---|---|
| preference guard, i18n lookup, update logic, clock | unit (`commands::preferences`, `services::i18n`, `services::updates`, `services::clock`) |
| preference round trips | integration (`*preference*`) |
| locale parity, native names; preference and security command arguments | contract (`src/i18n/i18n.parity.test.ts`, `nativeNames`, `src/lib/apiContract/ajustes.api.test.ts`) |
| panels and their error paths | vitest (`Settings/*`, `hooks/*`) |
| dialog, tabs, Escape | e2e / ui (`Ajustes/*`) |
| eval | n/a: no setting produces a model reply of its own; the ones that steer the chat (output language, routing mode) are exercised by the chat evals |
