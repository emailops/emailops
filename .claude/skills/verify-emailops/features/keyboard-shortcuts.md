# Keyboard shortcuts

Gmail-style shortcuts: `j`/`k` move a keyboard cursor through the list, `Enter`/`o` open, `u` back,
`x` select, `e` archive, `#` delete, `s` star, `b` snooze, `c`/`r`/`a`/`f` compose, `g` sequences
to switch views, `/` search and `?` the help overlay. They pause while typing and inside dialogs;
Settings → Appearance can turn them off.

## Sub-features

- `shortcuts.help` `?` opens the overlay grouped by Navigation, Conversation actions, Compose, Go to, Application.
- `shortcuts.navigate` `j`/`k` cursor (`[data-cursor=true]` on the row), open/back.
- `shortcuts.act` the action keys reuse the row actions (star, archive, snooze…).
- `shortcuts.hints` toolbar tooltips name the key from the registry (`Archive (E)`, `Delete thread (#)`, `Reply (R)`); gone when shortcuts are off.
- `shortcuts.overlays` no conversation shortcut runs while any overlay is open (dialogs, Settings, the image viewer, the row ⋮ menu, the snooze picker…): overlays register through `useOverlay`. Backspace is never bound.
- `shortcuts.setting` the on/off switch and "Show the list" in Settings → Appearance.
- `shortcuts.send` `Mod+Enter` sends from a composer (Compose modal, Compose tab, inline reply).

## How to get to it (user POV)

Anywhere outside a text field, press `?`. Six entry points, the columns of `## Parity`:

- **Inbox list** — the list, full-width or split, including the bulk bar.
- **Open thread** — the reading pane, where keys arrive as pane commands.
- **Compose modal**, **Compose tab**, **Inline reply** — `Mod+Enter`; other keys are text there.
- **Tag Board** — the board and its reading pane.

## Parity

| Capability | Inbox list | Open thread | Compose modal | Compose tab | Inline reply | Tag Board |
|---|---|---|---|---|---|---|
| shortcuts.help | e2e:Atajos/? abre la ayuda y Escape la cierra | gap: untested — ? with a conversation open is never pressed | n/a: the modal is an overlay and keys in it are text; no app shortcut runs there by design | n/a: keys typed in the tab's editor are text by design | n/a: keys typed in the reply editor are text by design | gap: untested — ? is never pressed on the Tag Board |
| shortcuts.navigate | e2e:Atajos/j y k mueven el cursor | vitest:src/hooks/useGlobalShortcuts.test.tsx::in the split layout j opens the next conversation | n/a: keys in a composer are text by design | n/a: keys in a composer are text by design | n/a: keys in a composer are text by design | gap: missing — the board is not a list view and publishes no list, so j/k/Enter/o/x do nothing on its cards |
| shortcuts.act | e2e:Atajos/s destaca la conversación del cursor | e2e:Atajos/e archiva y abre la siguiente | n/a: keys in a composer are text by design | n/a: keys in a composer are text by design | n/a: keys in a composer are text by design | gap: untested — with a board thread open the keys reach the pane, but nothing presses them on the board |
| shortcuts.hints | vitest:src/components/Inbox/BulkToolbar.test.tsx::names each key from the registry, and none when shortcuts are off | e2e:Atajos/la ayuda emergente nombra la tecla | gap: missing — the Send button names no key although compose.send is Mod+Enter | gap: missing — same, the tab's Send names no key | gap: missing — same, the reply's Send names no key | gap: untested — the board pane's toolbar titles are never read |
| shortcuts.overlays | e2e:Atajos/nada actúa con el menú ⋮ abierto | e2e:Atajos/nada actúa tras el visor de imágenes | vitest:src/hooks/useGlobalShortcuts.overlays.test.tsx::$key does not touch the open conversation | n/a: the tab is page content, not an overlay; overlays it opens are the shared ones covered elsewhere | n/a: the inline reply is page content, not an overlay | gap: untested — no key is pressed with a card ⋮ menu open on the board |
| shortcuts.setting | vitest:src/hooks/useGlobalShortcuts.test.tsx::the setting turns every shortcut off, ⌘K included | vitest:src/components/EmailView/EmailView.tooltips.test.tsx::drops the keys when keyboard shortcuts are off | gap: untested — the send-key hook checks the switch but only the reply composer tests it | gap: untested — same, no tab test | vitest:src/components/EmailView/ReplyCompose.sendShortcut.test.tsx::does nothing when keyboard shortcuts are turned off | gap: untested — the switch is never exercised on the board |
| shortcuts.send | n/a: the list has no composer | n/a: the thread's composer is the inline reply (its own column) | gap: untested — the send-key hook is wired but no test presses Mod+Enter in the modal | gap: untested — wired, no test | vitest:src/components/EmailView/ReplyCompose.sendShortcut.test.tsx::Cmd+Enter sends once and claims the key from the editor | n/a: the board has no composer; a reply in its pane is the inline reply column |

## Driving it with verify.sh

Preconditions: baseline, focus outside any input (`$V wd js 'document.activeElement.blur()'`). Confirmed live on 02/10/2026.

- `$V wd keys '?'` → `[data-testid=shortcut-help]` with 5 `[data-testid=shortcut-group]`; `$V wd keys Escape` closes it.
- `$V wd keys j` → `[data-cursor=true]` moves to the next row; `k` moves it back.
- Open a conversation, `$V wd keys e` → the next row opens (auto-advance) and the toast offers Undo.
- Overlay check: open the row ⋮ menu (or Settings, or an image attachment) and press `#`/`e`/`j` → no toast, no row leaves, the cursor stays; `Escape` closes the menu.
- `$V wd keys s` → the cursor row's `[data-testid=star-toggle]` turns `aria-pressed=true`; `s` again turns it off.

## Gotchas

- A focused search box or composer swallows the keys by design: blur first.
- The cursor is only drawn after the first keyboard move or click in the list.

| Case | Test kind |
|---|---|
| key matching, sequences, scope rules, action planning | vitest (`shortcuts`, `shortcutPlan`, `shortcutStore`, `useGlobalShortcuts`, `Inbox.shortcuts`, `EmailView.shortcuts`, `ReplyCompose.sendShortcut`) |
| help overlay, setting, tooltips | vitest (`ShortcutHelpModal`, `KeyboardShortcutsSetting`, `EmailView.tooltips`, `BulkToolbar`) |
| every real overlay blocks `#`, `Delete`, `e`, `s`, `b`, `j`; Backspace never deletes; overlay registry guard | vitest (`useGlobalShortcuts.overlays`, `overlayStore`) |
| unit (Rust), integration, contract | n/a: the shortcuts are frontend-only and call the same store actions and commands the buttons call, covered under their own features |
| `?` overlay, `j`/`k`, `s`, `e` auto-advance, tooltips, nothing behind the ⋮ menu / image viewer / Settings | e2e (`Atajos/*`) |
| eval | n/a: no model involved |
