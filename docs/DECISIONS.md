# Decision Log

Durable product and architecture decisions for EmailOps. Append-only, chronological
(newest at the bottom). Each entry records what was decided, the context, and the
alternatives rejected — so future work doesn't relitigate settled questions.

**For agents and developers:** consult this file before proposing changes that touch a
recorded decision. When the developer makes a durable decision in a session (a product
direction, an architecture choice, a deliberate trade-off), append an entry here in the
same change. Operational gotchas and in-flight work do **not** belong here — only
decisions that should still bind six months from now.

## Entry format

```markdown
## YYYY-MM-DD — Short decision title

**Decision:** What was decided, in one or two sentences.
**Context:** Why the question came up and what constraints shaped the answer.
**Rejected:** Alternatives considered and why they lost.
```

---

## 2026-07-22 — Keep decisions in a git-tracked log, not only agent memory

**Decision:** Durable decisions are recorded in this file (`docs/DECISIONS.md`),
versioned with the code. Agent auto-memory remains for session-level operational
context only (gotchas, in-flight work state).
**Context:** Agent memory lives outside the repo, is per-machine, and is invisible to
collaborators; decisions need a shared, versioned, authoritative home.
**Rejected:** ADR-per-file directory (`docs/decisions/`) — more ceremony than a
single-developer project needs today; can be migrated to later if the log grows large.

## 2026-07-22 — Calendar is per-account only (no unified/combined view)

**Decision:** The Calendar screen shows one account's calendar at a time, chosen via an
account selector like other screens. The unified "all accounts" sentinel
(`ALL_ACCOUNTS_ID`) is deliberately **not** offered on the calendar surface.
**Context:** Unified inbox exists for mail, but merging calendars across accounts
creates ambiguity (overlapping events, per-account colors, notification duplication)
without clear value for the current user base.
**Rejected:** Combined multi-account calendar view — deferred, not planned.

## 2026-07-23 — Calendar is read + write (event creation), not read-only

**Decision:** The calendar can create events (double-click an empty slot → new-event
dialog). OAuth scopes are therefore `calendar.events` (Google) and
`Calendars.ReadWrite` (Graph). Gmail-created events request a generated Google Meet
link; Graph events are created plain (Teams meeting creation needs a work/school
tenant — out of scope).
**Context:** v1 shipped read-only; creating events from the week view was requested
immediately after first use, and the write scopes are the same consent tier the app
already occupies.
**Rejected:** Staying read-only with a "open provider calendar to create" link —
breaks the flow the calendar view exists for. Editing events remains out of scope;
deletion IS in scope: delete/cancel with optional attendee notification (Graph
carries a custom cancellation comment; Google's API only sends its standard
cancellation email — a custom message there is not supported and is not faked).
Recurrence is preset-based (daily / weekly / weekdays / monthly / yearly), never
free-form RRULEs.

## 2026-07-22 — Upcoming-meeting notifications carry a direct join link

**Decision:** The app notifies before upcoming meetings, and the notification links
directly to the meeting (click → open join URL in the default browser). Join URLs are
extracted from structured provider fields first (Google `conferenceData`, Graph
`onlineMeeting.joinUrl`) with a regex fallback over location/description for common
platforms (Teams, Google Meet, Webex, Zoom, …).
**Context:** The main value of a meeting notification is getting into the meeting in
one click; provider structured fields are authoritative but not always populated
(links often live only in the event body).
**Rejected:** Notification without join action (forces the user back into the app);
extracting links only via regex (misses structured data, more false positives).

## 2026-07-23 — Calendar integration is per-account, on by default, auto-disabled without permission

**Decision:** Every calendar surface (calendar view + sidebar entry, invite cards with
RSVP, meeting notifications, background calendar sync, the chat `list_calendar_events`
tool and the weekly-report calendar section) is gated on the per-account
`calendar.enabled:<account_id>` preference. Default is **on** for calendar-capable
accounts (Gmail/Outlook); only an explicit `"false"` disables. The pref is written two
ways: the user's toggle in Settings → Calendar, or the scheduler's **auto-disable**
when the provider reports the account never granted calendar permission (403 with
scope-denial markers → `AppError::CalendarPermissionDenied`, emitted as the
`calendar-integration-changed` event so the UI hides immediately). Transient auth
failures (401 / expired token → `NeedsReauth`) never flip the toggle. The backend is
authoritative (scheduler, notifier, and chat tool re-check the pref per tick/turn, so
toggling needs no app restart); the frontend only hides surfaces. Settings offers an
inline re-auth that re-enables + re-syncs after consent is granted.
**Context:** Calendar requires extra OAuth scopes; accounts that never granted them
were getting endless sync attempts (5-minute error noise), invite cards whose RSVP
could only fail, and a chat tool advertising data that didn't exist. But calendar is a
headline feature — requiring an opt-in for accounts that *did* grant access would hide
it from everyone by default.
**Rejected:** Opt-in default (hides a working feature behind a toggle nobody knows
about); deriving enablement from provider capability alone (noisy for non-granted
accounts); auto-disabling on any auth failure (an expired token would silently kill
the calendar — only explicit scope denial may auto-disable).

## 2026-07-23 — IMAP custom folders sync automatically; localized well-known folder detection

**Decision:** For IMAP accounts, ALL user-created folders discovered via `LIST` are
synced automatically (no opt-in UI), stored as `emails.mailbox = 'folder:<server_path>'`
plus a `folders` table (V013), and shown in a per-account collapsible "Folders" sidebar
section (hidden in the unified All-Accounts view). Custom-folder mail flows through the
AI pipelines (classification/embeddings/memory) exactly like inbox mail. Well-known
folder detection (Sent/Spam/Trash) uses SPECIAL-USE (RFC 6154) attributes first, then
localized name candidates (en/de/es/fr — "Gesendete Objekte", "Papierkorb",
"Spamverdacht", …) matched with Unicode case-folding on UTF-7-decoded names; the same
resolver picks the APPEND target for sent copies. Scope is IMAP-only: Gmail labels and
Outlook custom folders keep current behavior via default-empty trait methods.
**Context:** A German IONOS user reported "folders from an IMAP account are not
synchronized, every time" — the historic candidate lists were English-only, so on
German servers nothing but INBOX synced, and custom folders were never enumerated at
all. Bounded by a 50-folder cap and a shared 20-page backfill budget per sync run.
**Rejected:** Opt-in folder selection in Settings (reporter would still see "nothing
syncs" until visiting Settings; an opt-out list can come later); excluding
custom-folder mail from AI pipelines (folders usually hold deliberately-filed,
high-value mail — and spam/trash already flow through them today); Gmail/Outlook
folder sync in the same change (larger scope, no bug report driving it). Drafts
folders and virtual views (`\All`, `\Flagged`) are excluded from sync; `\Archive` is
included as a custom folder. Cross-folder duplicate rows (same Message-ID in INBOX and
a folder) are accepted for now — a Message-ID-header dedup is a possible follow-up.

## 2026-07-23 — In-app IMAP folder management: create/rename/delete + move with drag-and-drop

**Decision:** IMAP accounts get full in-app folder management: create, rename, and
delete custom folders (sidebar "Folders" section: "+" affordance, hover actions, delete
confirmation), and moving messages between the inbox and custom folders (kebab-menu
picker and drag-and-drop of email rows onto sidebar targets). Every operation is
provider-first (server mutation before any local change), then local state migrates in
place: rename re-prefixes all `FOLDER::` message ids and carries sync watermarks so
nothing re-downloads and tags/embeddings/FTS survive; move re-keys the message row to
its new provider id (resolved via `UID MOVE` + `UID SEARCH HEADER Message-ID`); delete
hard-deletes local copies (the mail is gone server-side too). Role folders
(Sent/Spam/Trash) are not user-manageable, and Sent/Spam/Trash messages cannot be moved.
**Context:** Follow-up to the 2026-07-23 custom-folder sync decision — once folders
sync, users need to manage them and file mail without leaving the app (the IONOS
reporter manages folders in webmail today). Rename-in-place assumes servers keep UIDs
stable across RENAME (true for mainstream servers); if one doesn't, the id dedup simply
re-fetches under new ids.
**Rejected:** Rename/delete as "drop local + full re-sync" (loses AI
tags/embeddings and re-downloads entire folders for a cosmetic rename); moving
messages by delete + re-ingest (loses local AI state, message invisible until next
sync); rewriting `chat_messages.referenced_email_ids` JSON on id migration (renderer
already degrades gracefully on unknown ids); Gmail/Outlook support (trait defaults
return a typed "unsupported" error; UI is gated to IMAP accounts).
