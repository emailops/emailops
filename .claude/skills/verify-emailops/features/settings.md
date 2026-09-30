# Settings, language and updates

The settings dialog (appearance, AI tabs, privacy and security, junk, calendar), the UI
language, and the update check. AI provider settings have their own recipe
([ai-providers.md](./ai-providers.md)).

## Sub-features

- `settings.tabs` each tab renders; Escape and **Close settings** close the dialog.
- `settings.privacy` remote content, trusted senders, password; load and save failures are shown.
- `settings.language` UI language and AI output language; four locales with the same keys.
- `settings.updates` update check and install prompt.
- `settings.guard` the generic preference command refuses `security.*` and `app_data_dir` keys.

## How to get to it (user POV)

- Gear at the bottom of the sidebar (`aria/Application settings`).

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
