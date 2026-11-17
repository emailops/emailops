# Settings, language and updates

The settings dialog (appearance, signatures, notifications, AI tabs, privacy and security, junk, calendar), the UI
language, and the update check. AI provider settings have their own recipe
([ai-providers.md](./ai-providers.md)).

## Sub-features

- `settings.tabs` each tab renders; Escape and **Close settings** close the dialog.
- `settings.privacy` remote content, trusted senders, password; load and save failures are shown.
- `settings.language` UI language and AI output language; four locales with the same keys.
- `settings.updates` update check and install prompt.
- `settings.guard` the generic preference command refuses `security.*` and `app_data_dir` keys.
- `settings.sidebarDensity` Appearance → Sidebar density: Compact shortens the rows of the sidebar's scrolling list
  at once, survives a relaunch, and leaves the fixed header and footer alone.

## How to get to it (user POV)

- Gear at the bottom of the sidebar (`aria/Application settings`).

## Driving it with verify.sh

Preconditions: baseline.

- Open → `$V wd click 'aria/Application settings'` → the tab list shows Appearance, AI Backend & Models, AI Classification, Privacy & Security, Junk.
- Each tab → click the **last** button containing the tab label (the sidebar has a `Calendar` button too) → content renders.
- Close → `$V wd keys Escape`, or `$V wd click 'aria/Close settings'`.

### `settings.sidebarDensity`

Preconditions: baseline (`demo-acct-work` selected, "Other Views" folded), and the smart filters loaded: the
number of buttons in `[data-testid=sidebar-nav]` stays the same between the two measures.

1. Open Settings on Appearance. The control is a native `<select>` on macOS; set it through the DOM and close:
   `$V wd js "(() => { const s = document.querySelector('[data-testid=sidebar-density-select]'); s.value = 'compact'; s.dispatchEvent(new Event('change', { bubbles: true })); return s.value; })()"`,
   then `$V wd keys Escape`. Expect `$V wd js "document.querySelector('[data-testid=sidebar-nav]').dataset.density"`
   → `"compact"` without a restart.
2. The demo DB row is plain text: `sqlite3 .emailops-demo-data/emailops.db "SELECT value FROM user_preferences WHERE
   key = 'sidebar_density'"` → `compact` (no JSON quotes).
3. Measure the nav's content height (independent of the window height; `scrollHeight` floors at the visible height):
   `$V wd js "(() => { const n = document.querySelector('[data-testid=sidebar-nav]'); const k = [...n.children]; const cs = getComputedStyle(n); return { density: n.dataset.density, buttons: n.querySelectorAll('button').length, content: Math.round(k[k.length - 1].getBoundingClientRect().bottom - k[0].getBoundingClientRect().top + parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom)) }; })()"`.
   Switch back to `comfortable` the same way and measure again. Pass: compact / comfortable ≤ 0.8 with the same
   `buttons` count (17/11/2026: 1618 / 2106 = 0.768 with 60 buttons). Every row the setting changes is ≥ 24 px
   tall (view rows 28, account and filter rows 24).
4. Keyboard focus in Compact, for the first, a middle and the last row of the nav: focus the focusable just before
   the row with `$V wd js` (`.focus()`), then send a real Tab with `cua-driver press_key '{"pid":P,"key":"tab"}'`
   (`$V wd keys Tab` is a synthetic event and does not move focus). Pass: `document.activeElement` is the row.
   The focus ring itself only paints, and `:focus-visible` only matches, while the window is active on the
   current Space; with the window on another Space (`doctor` FAIL "window is on the current Space") the ring
   cannot be seen and the visual check stays open. One theme is enough: the `<aside>` carries `theme-chrome`,
   which paints the sidebar with the same dark palette on the light and the dark theme.
5. With Compact saved, `$V cleanup` and `$V launch` again; once the preference is read the nav shows
   `data-density="compact"` (CUJ 2).
6. Put `comfortable` back at the end, even when a step failed (the demo DB is shared by later runs).

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
