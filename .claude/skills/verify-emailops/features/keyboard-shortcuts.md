# Keyboard shortcuts

Gmail-style shortcuts: `j`/`k` move a keyboard cursor through the list, `Enter`/`o` open, `u` back,
`x` select, `e` archive, `#` delete, `s` star, `b` snooze, `c`/`r`/`a`/`f` compose, `g` sequences
to switch views, `/` search and `?` the help overlay. They pause while typing and inside dialogs;
Settings → Appearance can turn them off.

## Sub-features

- `shortcuts.help` `?` opens the overlay grouped by Navigation, Conversation actions, Compose, Go to, Application.
- `shortcuts.navigate` `j`/`k` cursor (`[data-cursor=true]` on the row), open/back.
- `shortcuts.act` the action keys reuse the row actions (star, archive, snooze…).
- `shortcuts.setting` the on/off switch and "Show the list" in Settings → Appearance; `Mod+Enter` sends from a composer.

## How to get to it (user POV)

- Anywhere outside a text field: press `?`.

## Driving it with verify.sh

Preconditions: baseline, focus outside any input (`$V wd js 'document.activeElement.blur()'`). Confirmed live on 02/10/2026.

- `$V wd keys '?'` → `[data-testid=shortcut-help]` with 5 `[data-testid=shortcut-group]`; `$V wd keys Escape` closes it.
- `$V wd keys j` → `[data-cursor=true]` moves to the next row; `k` moves it back.
- `$V wd keys s` → the cursor row's `[data-testid=star-toggle]` turns `aria-pressed=true`; `s` again turns it off.

## Gotchas

- A focused search box or composer swallows the keys by design: blur first.
- The cursor is only drawn after the first keyboard move or click in the list.

| Case | Test kind |
|---|---|
| key matching, sequences, scope rules, action planning | vitest (`shortcuts`, `shortcutPlan`, `shortcutStore`, `useGlobalShortcuts`, `Inbox.shortcuts`, `EmailView.shortcuts`, `ReplyCompose.sendShortcut`) |
| help overlay, setting | vitest (`ShortcutHelpModal`, `KeyboardShortcutsSetting`) |
| unit (Rust), integration, contract | n/a: the shortcuts are frontend-only and call the same store actions and commands the buttons call, covered under their own features |
| `?` overlay, `j`/`k`, `s` | e2e (`Atajos/*`) |
| eval | n/a: no model involved |
