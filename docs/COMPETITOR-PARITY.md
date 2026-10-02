# Competitor parity: Gmail and Outlook

**Audited commit:** `68aae91` (`main`)
**Date:** 01/10/2026
**Branch implementing it:** `feature/competitor-parity`

EmailOps is ahead of Gmail and Outlook on everything AI-shaped (chat over the
mailbox, lenses, memory, tag board, local junk model). This audit is about the
other half: the everyday mail-client features users carry over from Gmail and
Outlook and notice immediately when they are missing.

## How it was prioritised

Each gap was scored on three questions:

1. **Expectation.** Do both Gmail *and* Outlook ship it, prominently, so a
   switching user reaches for it in the first day?
2. **Cost of absence.** Is there a workaround inside EmailOps, or does the user
   have to go back to the provider's web client?
3. **Effort and risk.** Can it be built on seams that already exist (provider
   write-back, the task queue, the notification plugin, the composer), or does
   it need a cross-cutting rewrite?

**High** = expected on day one, no workaround, buildable on existing seams.
**Medium** = expected by regular users, or a workaround exists, or a smaller
slice of a larger feature. **Low** = niche, provider-specific, privacy-hostile,
or a large cross-cutting change that deserves its own branch.

## Inventory and priority

| Feature | Gmail | Outlook | EmailOps today | Priority |
|---|---|---|---|---|
| Archive | ✅ | ✅ | ❌ only delete | **High** |
| Mark as unread | ✅ | ✅ | ❌ read-on-open only | **High** |
| Star / flag | ✅ | ✅ | ❌ ("flagged" means junk) | **High** |
| Multi-select + bulk actions | ✅ | ✅ | ❌ | **High** |
| Undo for archive / delete | ✅ | ✅ | ❌ | **High** |
| Undo send | ✅ | ✅ | ❌ | **High** |
| Scheduled send | ✅ | ✅ | ❌ | **High** |
| Snooze | ✅ | ✅ | ❌ | **High** |
| Signatures per account | ✅ | ✅ | ❌ | **High** |
| Keyboard shortcuts + `?` help | ✅ | ✅ | ⚠️ Cmd+K only | **High** |
| One-click unsubscribe | ✅ | ✅ | ⚠️ headers captured, no UI | **High** |
| Block sender | ✅ | ✅ | ⚠️ menu item only hides a filter chip | **High** (misleading today) |
| New-mail desktop notifications | ✅ | ✅ | ❌ plugin wired for meetings only | **High** |
| Templates / quick parts | ✅ | ✅ | ❌ | Medium |
| Mail rules / filters with actions, Sweep | ✅ | ✅ | ⚠️ rules only set tags | Medium |
| Search operators (`has:attachment`, `is:starred`, `filename:`, `larger:`, `in:`) | ✅ | ✅ | ⚠️ from/to/subject/date/tag | Medium |
| Mute thread / ignore conversation | ✅ | ✅ | ❌ | Medium |
| Pin thread to top | — | ✅ | ❌ | Medium |
| Hover quick actions on list rows | ✅ | ✅ | ❌ ⋮ menu only | Medium |
| Drag & drop attachments into composer, Cmd+Enter to send | ✅ | ✅ | ❌ | Medium |
| Mark-as-read delay setting | — | ✅ | ❌ | Medium |
| Report spam on any message | ✅ | ✅ | ⚠️ only on detector-flagged mail | Medium |
| Print | ✅ | ✅ | ❌ | Medium |
| Follow-up nudges (no reply in N days) | ✅ | ✅ | ⚠️ AI tasks, experimental | Medium |
| Dark mode | ✅ | ✅ | ❌ | Low — cross-cutting restyle of every view plus the light-pinned email frame; own branch |
| Send-as aliases (From picker) | ✅ | ✅ | ❌ | Low — per-provider alias discovery |
| Vacation responder | ✅ | ✅ | ❌ | Low — provider-side settings API per provider, no IMAP equivalent |
| Command palette | — | ⚠️ | ❌ | Low — shortcuts + search cover it |
| Show original / save `.eml` | ✅ | ✅ | ❌ | Low — raw headers kept away from the webview by design |
| Conversation-view toggle, density | ✅ | ✅ | ❌ | Low |
| Sender hover card | ✅ | ✅ | ⚠️ Contacts view only | Low |
| User-picked label colours | ✅ | ✅ | ❌ hashed colours | Low |
| S/MIME / PGP | ⚠️ | ⚠️ | ❌ | Low |
| Read receipts | — | ✅ | ❌ | Out of scope (privacy positioning) |

Already at parity: drafts autosave, recipient autocomplete, inline reply,
translation, tasks from email, rich-text composer, IMAP folder management,
calendar with RSVP, junk filtering.

## Design constraints carried into the implementation

- **Provider write-back follows the read-state pattern.** Archive and star are
  applied locally first and pushed to the provider (Gmail labels `INBOX` /
  `STARRED`, Graph move-to-archive / `flag`, IMAP move to the `\Archive`
  special-use folder / `\Flagged`), with the pending marker and retry the
  read-state push already uses (see DECISIONS 2026-09-30).
- **Snooze, mute and pin are local state.** No provider exposes a portable
  snooze; storing it locally keeps it working on IMAP too.
- **Undo send and scheduled send share one outbox.** A message waits in a local
  `outbox` table until its `send_at`; undo send is a scheduled send a few
  seconds in the future. The app must be running for a scheduled send to go out,
  which the UI states.
- **Every irreversible action has a reachable inverse** (implementation-gap
  class H): unblock, unmute, unsnooze, unstar, move back to inbox, cancel a
  scheduled send.
