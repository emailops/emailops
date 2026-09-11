---
name: verify-emailops
description: "Drive the real EmailOps desktop app (Tauri 2 + React, macOS) the way a user does and capture proof: launch an isolated dev instance on the synthetic demo DB with the embedded WebDriver enabled, health-check it, click/type through WebDriver selectors (cua-driver for the native layer), keep screenshots + DOM/AX evidence, tear down only what the run started. Use after any frontend/UI change, before declaring a UI feature or fix done, or when a bug report comes as a screenshot. Never drives the developer's own running instance or the production mailbox."
---

# Verify EmailOps

EmailOps is a Tauri 2 desktop app: Rust backend, React/TypeScript frontend rendered in
a WKWebView. The user touches the **desktop window** (primary surface). Secondary
surfaces, verified elsewhere: `emailops-cli` (headless, `make cli-*`, see root
`CLAUDE.md`) and the iOS build (`scripts/ios.sh`, no tap tooling).

Everything below goes through one helper so the next agent never re-derives it:

```bash
V=.claude/skills/verify-emailops/scripts/verify.sh
```

Feature recipes live in [`features/`](features/README.md). Read the README first,
then the file for the feature you are proving.

## Launch

```bash
$V launch      # ~10 s warm, several minutes on a cold Rust build
```

What it does, and why each part exists:

- Starts `npm run tauri dev -- --features webdriver` with `TAURI_WEBDRIVER_PORT=4445`,
  `EMAILOPS_DATA_DIR=<repo>/.emailops-demo-data` and a
  `--config` override that moves Vite **and** Tauri's `devUrl` to port **1421**.
  `vite.config.ts` pins 1420 with `strictPort`, so without the override a second instance
  cannot start while the developer's own `make dev` is up. Ports and dirs come from
  `VERIFY_PORT` / `VERIFY_DATA_DIR` if you need another pair.
- Builds the demo DB + embeddings first if missing (`scripts/ensure_demo_db.sh`). The demo
  persona is synthetic: two IMAP accounts, `ulises@emailopslabs.dev` (id `demo-acct-work`)
  and `ulises@fastmail.com` (`demo-acct-personal`), 79 emails, onboarding already completed.
- Refuses to start if port 1421 is taken, if another `emailops` process already holds the
  demo DB, or if `VERIFY_DATA_DIR` points at `~/Library/Application Support/com.emailops.app`
  (the production mailbox). Refusing beats double-driving a shared instance.
- Waits until an `emailops` process has `<data_dir>/emailops.db` open (that is how the
  instance is identified: by the file it holds, never by `pgrep -f`), then until
  cua-driver lists a window for that pid, then until the WebDriver `/status` answers. Prints
  `ready: pid=… window_id=… port=… webdriver=… data_dir=… run_dir=…`.
- Records `launcher.pid`, `app.pid`, `window.id`, `port`, `data_dir` and `app.log` under
  the run dir; `src-tauri/reports/verify/current` symlinks to it.

Ready signal in the log: `App running` / Vite `ready in`. A failure shows as the launcher
exiting (`launcher exited before the app came up`, log tail printed) or a timeout.

Expected on screen after launch: the inbox of `ulises@emailopslabs.dev` and a yellow
**"Authentication required for demo-acct-work"** banner. The banner is normal: the demo
accounts have no keychain credentials, so sync cannot run. Everything read-only works.

**Spaces gotcha (seen 11/09/2026):** if the terminal that runs `launch` is in a
full-screen Space, the EmailOps window is created on the desktop Space instead. WindowServer
lists it (`list_windows`) but Accessibility exposes **no** windows for the process, every
snapshot returns only the menu bar, and the screenshot is a blank pane. `doctor` reports this
as a FAIL on "window is on the current Space". WebDriver driving (`$V wd …`) still works in
that state; only the cua-driver layer is blind. For a native-layer check the fix is on the
human side: leave full-screen or switch to the Space holding EmailOps, then re-run `doctor`.
Do not `bring_to_front` on your own; that steals the developer's focus.

Teardown: `$V cleanup` (see below).

## Doctor

```bash
$V doctor      # read-only; exit 1 on any FAIL
```

Checks, in order: launcher alive; recorded app pid is an `emailops` process; that pid has
the demo DB open; that pid does **not** have the production DB open; port has a
listener; cua-driver daemon running; Accessibility + Screen Recording granted to
cua-driver; the recorded window id is still listed. Run it before the first drive, after
any drive that surprised you, and after a failed iteration.

If cua-driver is not running: `open -n -g -a CuaDriver --args serve` then
`cua-driver status`. (This is the daemon, not the target app, so `open` is fine here.)

## Drive

Two transports. **WebDriver first**: `launch` builds the app with the `webdriver` cargo
feature and `TAURI_WEBDRIVER_PORT=4445`, so `tauri-plugin-wdio-webdriver` runs a W3C server
on 127.0.0.1:4445 inside the app (`src-tauri/src/webdriver.rs`). It drives the DOM of the real
WKWebView with real IPC and data, and does not care about Spaces, focus or Accessibility.
`wd.mjs` (node + `webdriverio`, a devDependency) wraps it; one session per call.

```bash
$V wd status                                   # {"ready":true,…}
$V wd find 'button*=Nadia Brunner'             # count + text of matches (exit 1 if none)
$V wd exists 'aria/Close'                      # present/absent, exit code
$V wd click 'button=Tag Board'
$V wd type  'input[placeholder^="Search"]' 'Ollama'
$V wd keys  Enter
$V wd text  'h1'                               # text of first match
$V wd js    'document.title'                   # any expression, printed as JSON
$V wd shot  "$(readlink src-tauri/reports/verify/current)/inbox.png"
```

Selectors are WebdriverIO's: CSS, `button=Inbox` (exact text), `button*=Nadia` (partial
text), `aria/Close chat panel` (accessible name, i.e. the `aria-label` or visible label).

**cua-driver second** (native layer: window screenshots, menus, anything outside the
webview). Daemon at `~/Library/Caches/cua-driver/cua-driver.sock`; it works on a backgrounded
window and never steals focus. Invariant: **snapshot before every element-indexed action**;
the helper does that for you.

```bash
$V snap inbox                      # AX tree + PNG -> <run>/inbox.{json,png,tree.txt}
$V find inbox 'AXButton "Compose"' # element_index of matching role/label/value
$V click 'AXButton "Tag Board"' nav-tagboard      # snap, click first match, snap again
$V type  'AXTextArea' 'What needs my reply?' chat-ask   # click the field, type, snap
```

Raw calls when the composites do not fit (pid/window from `<run>/app.pid`, `<run>/window.id`):

```bash
cua-driver click     '{"pid":P,"window_id":W,"element_index":N}'
cua-driver press_key '{"pid":P,"key":"return"}'        # escape, return, tab…
cua-driver hotkey    '{"pid":P,"keys":["cmd","k"]}'
cua-driver scroll    '{"pid":P,"window_id":W,"direction":"down","amount":10}'
```

Stable handles (from `src/locales/en/*.json`; the AX label is the visible text or the
`aria-label`):

| Where | Handle |
|---|---|
| Sidebar, Views | `AXButton "Inbox"`, `"Tag Board"`, `"Attachments"`, `"Drafts"`, `"Sent"`, `"Calendar"` |
| Sidebar, Other Views | `"Spam"`, `"Deleted"`, `"Contacts"`, `"Dashboard"` |
| Sidebar, AI Features | `AXCheckBox "Chat"` (toggles the right chat panel), `"Tasks"`, `"Lenses"`, `"Memory"` |
| Sidebar, top | `AXButton "Compose"`, `AXButton "Search emails…"`, account rows by address, `"All accounts"` |
| Inbox | category tabs `"Primary" / "Social" / "Updates" / "Forums" / "Promotions"` (group aria `Email categories`); rows are `AXButton` whose label starts with the sender name |
| Email row hover | `"More actions"`, `"Chat about this thread"`, `"Open in new tab"` |
| Reading pane | `"Close"`, `"Reply"`, `"Reply All"`, `"Forward"`, `"Open in tab"`, `"Close tab"` |
| Chat panel | `"Open chat panel"` / `"Close chat panel"`, `"New chat"`, `AXTextArea` placeholder `Send a message…`, `AXButton "Send"` |
| Tag Board | `Search tags…`, range `"All time" / "Today" / "Yesterday" / "Last 7 days" / "Custom"`, block menu `⋮` |
| Compose | fields `To`, `Subject`, `Write your message…`; `"Send"`, `"Discard"`, `"Attach files"`, `"Expand"`, `"Generate with AI"` |

WKWebView quirks: the first snapshot after launch can be sparse, snapshot again. Virtualised
inbox rows off-viewport show with `h:1` frames, scroll before clicking them. Text typed
through the AX path into web inputs reports `unverifiable`; trust the screenshot, not the
echo. If a background click does not land, re-issue it with `"delivery_mode":"foreground"`
and say so in the report (it briefly fronts the window).

## Evidence

`$V wd shot <file>` writes a page screenshot through WebDriver (works off-Space). Every
cua-driver snap writes three files under the run dir (default
`src-tauri/reports/verify/<timestamp>/`, gitignored by `src-tauri/reports/`):
`<name>.png` (window screenshot), `<name>.json` (full AX payload),
`<name>.tree.txt` (Markdown tree). `click`/`type` write a `-before` and `-after` pair.

Proof standard for a feature:

- Drive the **user path** named in the feature file (sidebar button, row click, typed
  text), not a store setter, a Tauri command, or a test-only route.
- Capture the action and the resulting state: the `-before/-after` pair, plus the
  `find` line that proves the expected element/text exists in the after tree.
- Verify side effects where the feature has them: rows in `<data_dir>/emailops.db`
  (`make cli-demo ARGS="… --json"` or `sqlite3`), files written, log lines in
  `<run>/app.log`. Visible state alone is not proof of a write.
- Nothing here is a dry-run: the demo instance really writes drafts, preferences and
  chat conversations to the demo DB. That is fine, it is throwaway. Sending mail is
  not: the demo accounts have no credentials, so a Send ends in an error banner, which
  is the observable state to assert.
- Mocks: none. The only isolated boundary is the mail provider (no credentials, so
  sync/send fail fast); everything else is the real code path.

Reference the run dir and the exact file names in the final report as `file://` links.

## Cleanup

```bash
$V cleanup
```

Kills the launcher process tree recorded in `launcher.pid` (npm → tauri → vite/node) and
the app pid recorded in `app.pid`, and nothing else: it never kills by name, so the
developer's own instance on 1420 is untouched. Waits for the port to free, then lists
the evidence that stays in the run dir. Run it after every failed iteration too, so a
broken attempt does not leave a second instance squatting on 1421.

Do not delete the run dir. If disk matters, prune old runs by hand:
`ls src-tauri/reports/verify/`.

## Helpers

`scripts/verify.sh` (executable) is the entry point; its subcommands are shown above and in
its header (`$V` with no args prints them). `scripts/wd.mjs` is the WebDriver client it calls
for `wd`; it can also run alone with `TAURI_WEBDRIVER_PORT=… node scripts/wd.mjs …`. Environment knobs: `VERIFY_PORT`,
`VERIFY_DATA_DIR`, `VERIFY_RUN_DIR`, `VERIFY_LAUNCH_TIMEOUT`, `VERIFY_SETTLE`
(seconds to wait between an action and its after-snapshot, default 1.5).

Keep the feature map honest with `/maintain-verification-skill` when the UI changes.
