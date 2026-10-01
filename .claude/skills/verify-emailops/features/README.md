# EmailOps verification map

Maintained source for proving the user-facing behaviour of the EmailOps desktop app.
Read this index, then the matching feature file, before driving anything. The helper
is `.claude/skills/verify-emailops/scripts/verify.sh` (`$V` below); `$V wd …` is the
WebDriver transport (DOM inside the real app), `$V snap/click/type` the cua-driver one.

## Baseline preconditions

- Instance started by `$V launch`: Vite/Tauri on port 1421, WebDriver on 4445, data dir
  `.emailops-demo-data`, synthetic persona `ulises@emailopslabs.dev` (`demo-acct-work`,
  selected by default) and `ulises@fastmail.com` (`demo-acct-personal`), both IMAP, about 100 emails,
  embeddings present, AI on (embedded llama.cpp, `qwen3.5-4b-q4_k_m`). A third,
  credential-less Gmail account `ulises.emailopslabs@gmail.com` (`demo-acct-calendar`) owns
  the demo calendar `demo-cal-work` (six events around today; tomorrow 10:00 is "Sprint 6
  planning — Faro Logistics") because calendar features are only offered to Gmail/Outlook.
- The instance holds no remote-provider key: `launch` starts it with `OPENROUTER_API_KEY` set and empty, so the
  developer's key in `.env.local` is not loaded and nothing in the UI can reach OpenRouter.
- Verification fixtures (`insert_verification_fixtures` in `scripts/generate_demo_db.py`): a trashed quote and its
  correction (Larkspur Freight, with a shortcut and a web-page attachment), a renewal notice and a junk-marked
  lookalike (Tessellate Hosting), an email with a remote image (Harborlight Weekly) whose `email_headers` carry
  RFC 8058 one-click `List-Unsubscribe` on `.example` hosts, a draft with a table, the starred correction quote and one
  archived message (*Studio key handover confirmed*).
- The schema comes from this checkout's migrations (`init_db` example), not from the production DB; `ensure_demo_db.sh`
  rebuilds the demo DB when a migration is newer than it.
- `$V doctor` passes. Never drive an instance this run did not start (the developer's
  own app on port 1420 usually holds the production mailbox).
- The red "Authentication required for account …" banners are expected on all three accounts.
- The chat panel is docked open on the right by default; the inbox list sits in the middle.
- UI language English (the demo DB default). Handles below are the English strings.

## Driving conventions

- Start every recipe from the inbox of `demo-acct-work` unless its preconditions say
  otherwise; `$V wd click 'button=Inbox'` gets you there.
- Address elements by WebdriverIO selectors: `button=Inbox` (exact text), `button*=Nadia`
  (partial), `aria/Close chat panel` (accessible name), CSS, or XPath starting with `//`.
  Inbox rows are `div[role="button"]`; match them with
  `//div[@role="button"][contains(., "<sender>")][contains(., "<subject fragment>")]`.
- Use pixel coordinates only for what is outside the webview (native menus, dialogs).
- Leave the app as you found it: go Back from open threads, discard drafts you started.

## Proof and skip reporting

- A proof is a `-before` / `-after` screenshot pair (`$V wd shot`) plus a `find`/`exists`
  line showing the expected element or text, plus a DB/log read for any side effect.
- Report an unreachable path with the command tried and the unmet precondition. A path
  driven through a different entry point than the one listed is not "verified".
- Say which transport landed the action if it was not the default WebDriver one.

## Feature entry contract

Each file has an H1, one paragraph of user-visible behaviour, then exactly four H2s:
`Sub-features`, `How to get to it (user POV)`, `Driving it with verify.sh`, `Gotchas`, and ends
with a "test kind per case" table. A layer a feature does not have is a row of that table
that says `n/a` and why.

## Features

- [Inbox: open and close a thread](./inbox-open-email.md) — list, open, reading pane, Back. **Driven live 11/09/2026.**
- [Search emails](./search.md) — search box, results, empty, clear. **Driven live 11/09/2026.**
- [Chat with the inbox](./chat.md) — panel, ask, sources, new chat. Selectors confirmed present, not yet driven.
- [Tag Board](./tag-board.md) — blocks per classified tag, range filter, tag search. Entry button confirmed, not yet driven.
- [Compose](./compose.md) — new email, fields, discard, send-without-credentials path. Entry button confirmed, not yet driven.
- [AI providers and models](./ai-providers.md) — backend tabs, OpenRouter fields, Logs status bar. **Driven live 30/09/2026.**
- [Accounts and sync](./accounts-sync.md) — account switching; sync itself is proven against the fake provider.
- [Junk mail](./junk.md) — chip, feedback, exclusion from views and retrieval. **Chip driven live 30/09/2026.**
- [Lenses, tasks and attachments](./lenses-tasks-attachments.md) — views, dangerous-type confirmation, sandboxed preview. **Attachments driven live 30/09/2026.**
- [Calendar](./calendar.md) — view, invite card.
- [Contacts and Dashboard](./contacts-dashboard.md) — the two views.
- [Settings, language and updates](./settings.md) — dialog, tabs, Escape.
- [CLI and agents](./cli.md) — envelope, read-only open, exit codes (no window).
- [Archive, star, snooze and multi-select](./organize-archive-star-snooze.md) — star, Starred/Archive/Snoozed, bulk toolbar, undo window, snooze. **Driven live 02/10/2026.**
- [Undo send and scheduled send](./outbox-undo-schedule.md) — Send + Undo, Schedule send, Scheduled view, failure without credentials. **Driven live 02/10/2026.**
- [Signatures](./signatures.md) — Settings → Signatures, insertion in the composer. **Driven live 02/10/2026.**
- [Keyboard shortcuts](./keyboard-shortcuts.md) — `?` overlay, `j`/`k` cursor, action keys. **Driven live 02/10/2026.**
- [Unsubscribe and block sender](./sender-controls.md) — Unsubscribe dialog (Cancel only), Block/Unblock, Settings → Junk list. **Driven live 02/10/2026.**
- [New-mail notifications](./new-mail-notifications.md) — Settings → Notifications persistence; delivery via the fake provider. **Driven live 02/10/2026.**
- [Chat skills](./skills.md) — toggle, Skills view, new/save/delete, `/` suggestions. Handles confirmed live 29/09/2026.
