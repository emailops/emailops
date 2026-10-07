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

## 2026-07-24 — Update notification: in-app toast to GitHub release page, no auto-updater

**Decision:** The app detects new releases by polling the GitHub Releases API
(`/repos/emailops/emailops/releases/latest`) at most once per 24h (checked hourly,
gated on the persisted `app_update_last_check_at` pref, skipped offline and in debug
builds unless `EMAILOPS_UPDATE_CHECK=1`). A newer version surfaces on two in-app
channels, both opening the GitHub release page in the external browser: (1) a sticky
toast (never auto-dismisses; user-close only) emitted once per version via
`app-update-available` (`app_update_notified_version` pref), and (2) a persistent
link in the sidebar footer, below the syncing message, that survives restarts (the
check persists `app_update_latest_version`/`_url` prefs; `get_available_update`
re-derives the link at startup) and disappears only once the user actually upgrades.
**Context:** Releases ship as DMGs on GitHub (plus Homebrew cask); users had no
in-app signal that a new version exists. macOS native notification clicks proved
undeliverable back to the app during the calendar feature, so in-app surfaces are
the actionable channel. A transient toast alone was deemed too easy to miss for a
notice that stays relevant until the user upgrades.
**Rejected:** `tauri-plugin-updater` / self-updating binaries (signing +
auto-replace complexity, and Homebrew-managed installs shouldn't self-mutate);
native OS notification (click-through unreliable); top-of-app banner (too intrusive
for a non-urgent nudge — the sidebar footer link carries the persistent state
instead); direct per-arch DMG download link (breaks if asset naming changes; release
page also shows the notes); a Settings toggle (automatic-only keeps the surface
minimal — trivial to add later since checks read prefs every tick).

## 2026-07-24 — AI email translation: LLM detection, session-only cache, plain-text fidelity

**Decision:** Add AI translation at three surfaces — a Translate button on emails whose
detected language differs from the preferred AI language (reading view, per-email with an
original/translation toggle), "Translate to <thread language>" in reply compose, and a
free-text "Translate to…" target in new compose. Language detection is LLM-based (tiny
400-char sample prompt, `max_tokens 16`, ISO-code-only answer, fail-closed to `und` = no
button) through the configured provider. Nothing is persisted: detections live in a
process-static map on the Rust side + a Zustand session store; translated bodies are
session-only. Translation is a plain-text roundtrip — `body_to_plain_text` → model →
plain rendering (reading view) or `plainTextToHtml` back into Tiptap (compose, with an
"Undo translation" snapshot) — capped at 9,000 input chars (`truncated` surfaced in the
UI). Feature-gated by `ai_translation_enabled` (default on) with a Settings tab; prompts
are user-editable registry entries (`translate.detect_language`, `translate.email`);
free-text targets are sanitized (40 chars, letters/space/hyphen/apostrophe/parens only).
**Context:** Users receive mail in languages other than their preferred one; the app
already resolves a preferred AI language (`resolve_ai_language`). Detection must be lazy
(on email expand) and cheap so the embedded model isn't taxed during sync.
**Rejected:** DB persistence of translations (new migration + release coupling for a
cache that local re-generation covers); heuristic detection crate (user chose LLM
detection — no new dependency, handles mixed content); HTML-preserving translation
(small local models mangle markup and marketing HTML blows the 8k context window);
per-feature model override (would evict the chat KV cache — uses the main provider).

## 2026-07-28 — Gmail OAuth: request `gmail.modify` only, never `gmail.readonly`

**Decision:** `GMAIL_SCOPES` requests `gmail.send`, `gmail.modify`, and the two
`userinfo` scopes — never `gmail.readonly`. `gmail.modify` is a strict superset of
read access, so requesting both widened the declared scope set for zero capability.
A unit test (`gmail_scopes_omit_redundant_readonly` in `sync/oauth.rs`) fails if it
is ever re-added. The scope list published in the privacy policy (section 2, all four
locales in the `emailops_web` site repo) and the scopes declared on the GCP consent
screen must match this constant exactly.
**Context:** EmailOps is going through Google restricted-scope verification to lift
the 100-user OAuth cap. Google's review applies a narrowest-scope requirement, and a
mismatch between the code's scopes, the consent screen, and the privacy policy is a
documented rejection trigger. Carrying a redundant restricted scope meant one more
scope to justify in the review and in the demo video, with nothing gained.
**Rejected:** Keeping `gmail.readonly` for "explicitness" or as a fallback if
`gmail.modify` were ever narrowed (it isn't, and unused breadth is exactly what the
review penalises); dropping to `gmail.readonly` + `gmail.send` and giving up
archive/label/read-state writes (those are core inbox actions the app already ships).

## 2026-07-28 — Record provider Sent state in an `is_sent` column, not by inference

**Decision:** The `emails` table carries an `is_sent` flag (V014) set from the
provider's own signal — Gmail's `SENT` label, the Sent folder for IMAP/Outlook — and
the Sent view matches it first, falling back to `mailbox = 'sent'` and to
sender-equals-account for rows written before the column existed. `mailbox` stays
single-valued and keeps recording 'inbox' for self-sent mail so those threads remain
in the inbox view. Because the sync skips message ids it already stores, the Sent
pass also repairs the flag in place on rows it re-lists, and V015 clears the Sent
backfill watermarks once so existing databases walk their history and self-heal.
**Context:** Mail sent through a Gmail send-as alias was invisible in the Sent view.
Gmail labels self-sent mail INBOX *and* SENT, so the single `mailbox` column recorded
'inbox', and the sender was the alias rather than the account address — so neither
the mailbox check nor the sender check matched. No amount of local inference could
recover the fact; only the provider knows.
**Rejected:** Fetching the account's send-as addresses from Gmail's `sendAs` API into
preferences and matching senders against that list (no migration, but Gmail-only,
leaves Outlook/IMAP aliases unsolved, and needs refresh logic when an alias changes);
deriving aliases from senders already seen on `mailbox='sent'` rows (circular — empty
on a fresh database, and misses aliases only ever used to mail yourself); letting
`mailbox` hold 'sent' for self-sent mail (would drop those threads out of the inbox
view, which is where the user expects them).

## 2026-07-28 — Backfill progress lives in its own column, never in the user's sync-from preference

**Decision:** `accounts.sync_from_timestamp` is exclusively the user's chosen history
floor (`NULL` = "All mail") and is written only by the account-add and account-settings
paths. Sync's backfill progress moves to a dedicated `accounts.backfill_swept_from`
column (V017), where `F` means "swept from `F` up to the oldest stored inbox email and
found nothing new". The planner skips the backfill pass while the requested floor is at
or above `F`, and re-opens it if the user later asks for older history. Both sync
watermarks — newest and oldest — are inbox-scoped.
**Context:** A Gmail account added with "All mail" (`NULL`) stopped receiving anything
sent before the afternoon it was created. Two defects compounded: the backfill floor was
derived from `get_oldest_email_timestamp`, unscoped across mailboxes, so a reply the user
sent from the app became the account's oldest row; and on finding nothing older, sync
wrote that timestamp into `sync_from_timestamp` itself. The user's "All mail" was thereby
converted into a hard floor pinned to their own outgoing mail, permanently excluding
earlier inbound messages. The inbox-scoping bug was already fixed for the *newest*
watermark — the comment above `get_latest_email_timestamp_for_mailbox` describes exactly
this failure mode — but the fix was never applied to the oldest end of the range.
**Rejected:** Keeping the single overloaded column and only fixing the mailbox scoping
(the clobber would still destroy "All mail" whenever the oldest inbox row happened to sit
above the true floor, and leaves preference and progress indistinguishable on inspection);
dropping the watermark entirely and re-running the backfill every sync (re-queries an
exhausted range on every pass, which is what the watermark was introduced to avoid);
back-filling the new column from existing `sync_from_timestamp` values in the migration
(a clobbered value and a deliberate user choice are indistinguishable after the fact, so
this would launder corrupted floors into legitimate-looking preferences).

## 2026-07-28 — Gmail inbox listing filters categories negatively, over `in:inbox`

**Decision:** The Gmail list query is built as `((in:inbox -category:<deselected>…) OR
in:sent)` rather than `((category:<selected> OR …) OR in:sent)`. The account's category
selection is expressed by *excluding* the categories the user turned off, never by
requiring the ones they left on.
**Context:** Gmail's inbox tabs are optional and are commonly disabled on Google
Workspace accounts. On an account without them, `category:primary` matches nothing at
all — so the positive query returned an empty inbox while the `in:sent` branch kept
working. The result was an account that looked correctly connected, synced its own sent
mail and spam, and never showed a single received message. Negative terms degrade
correctly in both worlds: with tabs on the deselected categories are excluded exactly as
before, and with tabs off there is nothing to exclude so the whole inbox comes through.
An empty selection still means "sent only" — that branch is unchanged.
**Rejected:** Probing the account once and persisting a "has categories" flag to pick
between two query shapes (stateful, needs invalidation when the user toggles tabs in
Gmail, and doubles the query paths under test); listing `in:inbox` unfiltered and
dropping unwanted categories after fetching each message (correct, but pays a full
message fetch for mail that is immediately discarded — expensive on promotions-heavy
mailboxes); adding `in:inbox` as another positive OR term alongside the category clauses
(matches the entire inbox regardless of selection, silently discarding the user's
deliberate Promotions/Social exclusions).

## 2026-07-28 — Junk detection is local-flag-only, three-axis, and gated on false positives

**Decision:** Junk detection (spam / phishing-BEC / graymail) scores messages **locally
only** — it never moves, deletes or reports a message on the server, and the IMAP
`move_message` seam stays untouched. Messages are scored on **three independent axes**
that are never collapsed into one number, each with its own band and its own
false-positive budget. The measurement harness (`make eval-junk`,
`src-tauri/evals/junk/cases/`) is authoritative: a **false positive on legitimate mail
fails the build**, while a missed junk message is only a warning. The phishing axis has a
zero-tolerance budget on the curated synthetic corpus; spam is capped at 0.5% and
graymail at 2%. No statistical model is trained for the phishing axis.

**Context:** Gmail and Outlook already filter server-side, so the value is concentrated
where they don't help: IMAP accounts with weak server filtering, targeted BEC that
consumer filters pass through because it has no links and no bad grammar, and bulk mail
that is legitimate but unwanted. Those three fail in different ways and warrant different
treatment — badging a newsletter as a fraud attempt is as wrong as missing the fraud — so
one score cannot serve all three. The cost asymmetry is the governing constraint: a user
who misses one real invoice starts checking the junk group every time, which is exactly
the work the feature was supposed to remove. That makes precision, not recall, the thing
to optimize, and it has to be enforced mechanically rather than by intent. Phishing gets
no per-user statistical model because a mailbox yields a handful of positives at best;
the axis stays deterministic plus (later) an LLM band that may only move a score *within*
the uncertain range and can never clear a hard deterministic failure.

**Rejected:** A single junk score with one threshold (cannot express "bulk but
legitimate", and forces newsletters and wire fraud onto the same UI treatment);
server-side moves to the Junk folder in v1 (a false positive then hides mail in every
client, not just EmailOps — needs a demonstrated FP rate first, and Gmail/Outlook do not
implement the move seam anyway); an LLM classifier on every message (~600 tokens each,
weaker calibration than cheap statistical methods, and it cannot see the headers that
actually decide phishing); training the class prior from the provider's spam folder (that
folder is not a random sample of the inbox, so the empirical prior is wrong — it is fixed
by configuration instead); hiding junk from the inbox by default (deprioritize-and-
collapse keeps every message one click away and keeps the failure mode recoverable).

## 2026-07-29 — Linux and Windows are supported targets; portable crates over per-OS FFI

**Decision:** EmailOps builds and ships on Linux (`.deb`, `.AppImage`) and Windows
(`.msi`, NSIS `.exe`) alongside macOS. Platform-specific behaviour is obtained from
portable crates (`fd-lock`, `sysinfo`, `fs4`) rather than hand-written `#[cfg]` FFI
arms, and per-platform *decisions* are extracted into pure functions that take the OS
as an argument. Per-platform bundling lives in thin `tauri.<os>.conf.json` overlays
merged over the base config, exactly as `tauri.intel.conf.json` already did; the base
config keeps macOS-only bundle targets so the signed/notarized mac path is untouched.
**Context:** The codebase was macOS-first but only one thing actually blocked Windows
compilation (`std::os::unix::io::AsRawFd` + `libc::flock` in the single-instance lock).
The larger problem was silent degradation: RAM and disk probes returned "unknown" off
macOS, fatal startup errors showed no dialog at all outside macOS, and onboarding keyed
local-AI capability off `apple_silicon`, so every Linux and Windows machine was
defaulted to the no-AI client regardless of hardware. Development happens on macOS with
no cross-toolchain available, so any `#[cfg(windows)]` block a developer writes is code
they cannot compile — which is the argument for portable crates and for pure,
OS-as-parameter decision functions: both are compiled and table-tested on every host.
CI is consequently the only real verification gate, and a `windows-latest` job is now
the thing standing between a `std::os::unix` import and a broken release.
**Rejected:** Hand-rolled `windows-sys` FFI for the lock, RAM and disk probes (smaller
dependency footprint, but unverifiable on the development machine — precisely the
failure mode being fixed); enabling `cuda`/`vulkan` in the shipped binaries (a
GPU-linked build refuses to start without the matching driver, so the downloadable
artifact stays CPU-only and GPU backends remain opt-in build flags); shipping Linux and
Windows without embedded llama.cpp as the Intel-Mac build does (that exclusion exists
because Metal is Apple-Silicon-only, which says nothing about a CUDA workstation);
folding the Linux/Windows release jobs into the macOS matrix leg (the mac path carries
certificate import, notarization and keychain teardown with no analogue elsewhere, and
merging them would risk a working signed pipeline for no gain); code-signing the Windows
installers (needs an OV/EV certificate the project does not hold — unsigned artifacts
ship with a SmartScreen warning until one is acquired).

## 2026-07-30 — Windows and Linux releases build with Vulkan via dynamic backends

**Decision:** `make build-linux` / `make build-windows` in CI now pass
`DYNAMIC_BACKENDS=1 CARGO_FEATURES=vulkan`, so the released `.deb`/`.AppImage` and
`.msi`/NSIS artifacts ship ggml's Vulkan backend as a loadable module alongside the CPU
one, picked at runtime by VRAM/driver detection. The Vulkan SDK (headers, loader,
`glslc`) is installed in the CI job as a build-only dependency — end users only need
their normal GPU driver, which already ships the Vulkan runtime loader.
**Context:** This directly supersedes the "GPU-linked build refuses to start without
the matching driver" reasoning in the 2026-07-29 entry above — the `dynamic-backends`
Cargo feature (Linux/Windows only; macOS links Metal statically since every
Apple-Silicon Mac has it, so there is no missing-driver case to guard against there)
removed that failure mode by making the GPU backend a module the binary probes for and
loads conditionally, rather than something linked into it. Once that existed, shipping
CPU-only on Windows/Linux was leaving local-AI performance on the table for every user
with a discrete GPU, for no remaining safety reason.
**Rejected:** CUDA instead of Vulkan (faster on NVIDIA, but needs the NVIDIA toolkit at
build time and CUDA hardware at run time — Vulkan covers AMD/Intel/NVIDIA from one
build and only needs the driver every desktop already has); shipping separate
CPU-only and GPU-enabled artifacts (doubles the release matrix and asks users to know
their own hardware before downloading — dynamic backends exist specifically so one
artifact suffices); building both `vulkan` and `cuda` into the same binary (dynamic
backends pick one at build time; shipping both would double the bundled module size for
a codepath most users on a given machine never take).

## 2026-07-31 — Linux/Windows releases stay a separate, auto-published CI job; macOS stays fully manual

**Decision:** `.github/workflows/release.yml`'s `release-macos` job is removed entirely,
not merely left unused — macOS releases are built, signed, and notarized locally
(`make build-mac`) and uploaded by hand, permanently, not as a stopgap. The remaining
job builds and auto-publishes Linux (`.deb`/`.AppImage`) and Windows (`.msi`/NSIS `.exe`)
via `softprops/action-gh-release`'s upsert-by-tag behavior, gated behind a mandatory,
automated smoke test (install the built package on the same runner, launch it, confirm
the process survives a few seconds) that must pass before the release is published.
`workflow_dispatch` takes an explicit `tag_name` input so the workflow reliably attaches
its artifacts to a specific tag's release regardless of which ref triggered the run —
including a release the developer already created by hand for the macOS DMGs. A
`dry_run` input skips the release-publish step entirely (installers land as a plain
workflow artifact instead), so a change to this workflow or the build scripts can be
validated against real GitHub-hosted runners without ever touching the public releases
page or requiring a real tag.
**Context:** No Linux or Windows asset has ever shipped on a GitHub release, and the
workflow that would build them had never actually been run — `gh run list` came back
empty. The developer explicitly prefers keeping macOS signing entirely out of CI (no
certificate/notarization secrets need to live in a job whose only purpose is unsigned
Linux/Windows installers), and explicitly wants Linux/Windows release-testing to be
automatic rather than requiring a manually-managed VM (the GPU test VM used to debug the
Windows DLL-staging and Linux dropdown/OAuth fixes this session is private, per-developer
infrastructure — not something CI or a future contributor can rely on). The smoke test
specifically targets the failure mode a `DYNAMIC_BACKENDS` packaging regression takes
(binary fails to start because a shared library/DLL doesn't resolve) — GH-hosted runners
have no GPU, so it cannot and does not attempt to verify GPU offload; that still requires
occasional testing on real GPU hardware.
**Rejected:** keeping `release-macos` in the same workflow gated behind a
`workflow_dispatch` platform-select input (adds complexity for a job that should simply
never run again, versus deleting it outright); relying on `push: tags` to infer the
release tag (still disabled — releases are cut via the `release` skill, which triggers
this workflow explicitly once the tag exists on `origin`); a local Docker/VM-based smoke
test script instead of an in-CI step (doesn't scale to "every release, automatically" and
reintroduces the manual-VM friction this decision is meant to remove).

## 2026-08-03 — Windows CUDA ships as an additional, opt-in release asset alongside Vulkan

**Decision:** `.github/workflows/release.yml` gains a `release-windows-cuda` job,
independent of the existing `release` matrix, that builds Windows with
`DYNAMIC_BACKENDS=1 CARGO_FEATURES=cuda` and publishes `EmailOps-windows-cuda.msi` /
`-setup.exe` to the same release tag. This does **not** replace the Vulkan Windows
build from the 2026-07-30 entry — Vulkan stays the recommended default (broader
hardware coverage, no NVIDIA toolkit needed at build time); CUDA is offered for users
who specifically want it, not promoted over Vulkan. Two build-script fixes landed
alongside this: `scripts/build_platform.sh` now only forces the `--jobs 1` MSVC
PDB-race workaround when `CARGO_FEATURES` contains `vulkan` — that race is specific to
`vulkan-shaders-gen`, a CMake sub-project a CUDA-only build never configures — and
`scripts/dist_platform.sh` takes an optional variant suffix (`windows cuda` →
`EmailOps-windows-cuda.msi`) so two Windows installers can coexist in one release
without overwriting each other.
**Context:** Directly asked for after validating the Windows CUDA path end-to-end on a
real Tesla T4 test VM: real GPU offload confirmed (VRAM resident, a utilization spike,
and an explicit `llamacpp: ... offloading all layers` log line), and a from-scratch
release compile timed at 270m40s with the (misapplied) `--jobs 1` workaround vs 31m54s
once scoped to Vulkan only — an 8.5x difference that changes the calculus on whether a
CUDA CI leg is affordable at all. Every other llama.cpp-embedding project surveyed for
this decision (llama.cpp itself, Ollama, koboldcpp) builds Windows CUDA in CI the same
way: GitHub-hosted CPU-only runners (compile-only, no GPU to test offload on — this
project's own `release-windows-cuda` job accordingly only smoke-tests that the binary
starts and resolves `ggml-cuda.dll`, the same limitation the Vulkan legs already carry),
gated to manual dispatch or tag/release pushes, never per-PR. `CMAKE_CUDA_ARCHITECTURES`
is deliberately left unset rather than pinned to the T4's `sm_75` (which is what the
timing test above actually used, to isolate the `--jobs 1` variable) — ggml-cuda's own
upstream `CMakeLists.txt` already curates a virtual-PTX-plus-real-SASS architecture list
for cross-generation compatibility (llama.cpp's own CI takes the same approach: it
never overrides this at the workflow level either), and shipping a `sm_75`-only binary
would silently break or force slow PTX-JIT recompilation on every non-Turing GPU.
**Rejected:** pinning `CMAKE_CUDA_ARCHITECTURES` to a fixed list in CI for a faster
build (the `--jobs 1` fix alone recovers the vast majority of the win; trading real
multi-GPU-generation compatibility for a further speedup wasn't judged worth it without
a concrete need); running the CUDA leg on every PR (every comparable project gates this
to manual/release triggers given the build cost, and this project's own CI has no GPU
to validate offload on regardless of trigger frequency); a self-hosted GPU runner for
the build step (no project surveyed does this even for GPU-relevant testing, let alone
routine compilation — occasional real-hardware validation, as already established for
Vulkan, stays the pattern rather than adding standing GPU-runner infrastructure).

## 2026-08-03 — Stored credentials never cross the IPC boundary to the webview

**Decision:** Backend responses that describe a stored credential carry only its
*presence*, never its value. `get_imap_settings` returns `hasPassword` and the
non-secret server fields; the password itself stays in the keychain. The
re-auth/edit dialog therefore opens with an empty password box, and saving with an
empty box means "keep the stored password" (`resolve_update_password`). The
`get_imap_credentials` Tauri command — which returned the plaintext password and
had no frontend caller — was removed rather than kept as a trap. Credential structs
(`ImapCredentials`, `OAuthTokens`) also implement `Debug` by hand so a stray
`{:?}` cannot print a secret; `Serialize` still emits real values, which is how
they reach the keychain.
**Context:** The renderer that would have held the password is the same webview
that displays untrusted email HTML. Sanitization is good but is one bug away from
being the only thing between a malicious message and a live IMAP credential, so the
secret should simply not be reachable from that process. The "keep the stored
password" rule is what makes an empty box a valid save rather than an accidental
credential wipe.
**Rejected:** sending the password and relying on DOMPurify + CSP to protect it
(defence in depth argues for not having the secret there at all); masking it as
`••••••` in the payload (a placeholder that round-trips is indistinguishable from a
real password on save, and the real one still crossed the boundary); requiring the
user to retype the password on every settings change (punishes the common case of
editing only a port or server name).

## 2026-08-03 — `data:` in CSP `object-src`/`frame-src` is load-bearing

**Decision:** `data:` stays in the `object-src` and `frame-src` directives of the
production CSP. `blob:` was removed from both. A test in
`EmailHtmlFrame.csp.test.ts` pins *both* halves so neither is changed by accident.
**Context:** `object-src data: blob:` looks like gratuitous CSP weakening and was
flagged as such in a security review. It is not: `AttachmentViewer` builds a
`data:<mime>;base64,…` URI and renders it through `<object>` for PDFs and
`<iframe>` for HTML attachments, and `AttachmentTabView` does the same — dropping
`data:` silently breaks attachment preview. `blob:` genuinely was unused (there is
no `URL.createObjectURL` call anywhere in `src/`), so it was dropped. Email bodies
are unaffected either way: they render in a `srcdoc` iframe, which is governed by
the parent's CSP rather than `frame-src`.
**Rejected:** removing `data:` as well (breaks PDF/HTML attachment preview —
verified, not theorised); switching the attachment viewer to `asset:` or `blob:`
URLs so `data:` could be dropped (a real option, but a behaviour change to a
working feature for a marginal CSP win; revisit only if attachment sizes make the
base64 round-trip a performance problem).

## 2026-08-04 — The calendar shows every calendar of an account, coloured per calendar

**Decision:** Calendar sync enumerates every calendar an account can see (Google
`calendarList.list`, Graph `/me/calendars`) — its own, calendars shared with it,
subscribed ones — instead of only the primary calendar, and tints each event
with that calendar's own provider colour. Every calendar syncs; a per-calendar
`is_visible` toggle (calendar-view legend + Settings → Calendar) filters at
render time, so showing one again is instant rather than a poll away. Calendars
without a provider colour (Graph's named presets have no documented hex) get a
deterministic slot from an app palette, keyed on the calendar id so the colour
never shuffles between launches. Creating events remains primary-calendar-only.
**Context:** Only the primary calendar was ever fetched, so a shared team or
family calendar was invisible in the app while being visible in Google's own UI.
Two constraints shaped the answer: Google's `calendar.events` scope cannot list
calendars, so `calendar.calendarlist.readonly` was added (the narrowest scope
that grants it — every existing Gmail account must re-consent, and the Google
verification submission has to list it); and providers reuse a single event id
across every calendar an event appears in, so `calendar_events` was re-keyed to
`(account_id, calendar_id, provider_event_id)` in V022 — under the old key one
copy silently overwrote the other.
**Rejected:** Syncing only the calendars the provider marks as shown
(`selected`) with no in-app override — the provider flag is a fine *default* for
a newly seen calendar but a poor permanent policy. Not syncing hidden calendars
— saves API calls but makes every un-hide wait for the next 5-minute poll.
Assigning app-palette colours to everything — would not match the colours the
user already recognises from Google Calendar. Broadening to `calendar.readonly`
— grants reading every calendar's full contents when only the list is needed.
Mapping Graph's named colour presets to invented hexes — the guesses would not
match Outlook anyway, so those calendars use the app palette instead.

## 2026-08-04 — Hidden calendars are hidden from what acts, not just from the grid

**Decision:** Two event listings exist. `list_calendar_events` returns every
event and backs the calendar view, which filters client-side so the visibility
toggle is instant. `list_visible_calendar_events` excludes hidden calendars and
backs everything that *acts* on events: meeting notifications, the chat
`list_calendar_events` tool (and the weekly-report preseed through it), and the
CLI `/calendar` command. Events whose calendar has no registry row yet count as
visible.
**Context:** Multi-calendar sync means a hidden holiday or team calendar would
otherwise raise desktop meeting reminders and turn up in chat answers, which
reads as a bug — the user switched that calendar off.
**Rejected:** One filtered listing everywhere — the calendar view needs the
unfiltered set to re-filter locally on toggle without a refetch. Leaving
notifications and chat unfiltered — simpler, but surfaces exactly what the user
asked to hide.
## 2026-08-04 — Chat docks on the right; ambient context is the open thread only

**Decision:** Chat gains a persistent, resizable panel docked against the **right**
edge of the window (Copilot/Cursor-style), alongside — not replacing — the existing
full-page chat view. The nav sidebar's "Chat" entry toggles the panel; the panel's
expand button opens the full view; and an always-visible icon in the email-list
toolbar (top right, after "Load more") starts a fresh conversation and docks the
panel in one click, so beginning a chat never depends on the collapsible AI
FEATURES section being expanded. The panel grounds a turn in **the email thread the
main view currently shows**, offered as a removable chip above the input, and in
nothing else: list scope (mailbox, category, active filter, search query) and
selected body text are deliberately not part of the context.
**Context:** The chat was reachable only by navigating away from the mail you wanted
to ask about, which is exactly backwards for "summarise this thread" / "draft a
reply". Ambient thread context reuses the already-proven thread-bound turn path
(`run_thread_bound_turn`), so the feature is a new *entry point* to grounded chat
rather than a new retrieval mode. Per-turn plumbing (`context_thread_id` on
`send_chat_message`) keeps the binding ephemeral: a conversation is never silently
converted into a thread-bound one, and the user can move between threads inside a
single conversation. `plan_turn_mode` gives a conversation-level seeded thread
precedence over ambient context, so "Chat about this thread" keeps its meaning.
**Rejected:** docking on the left (the nav sidebar already owns that edge; chat would
either displace navigation or push it into the middle of the window); replacing the
full-page view with the panel (long sessions and conversation management still want
the room); making list scope part of the context (narrowing RAG by the visible
filter/search is a genuinely different retrieval mode, not just a new entry point —
larger change, deferred); including selected body text as a quotable chip (needs new
postMessage plumbing through the sandboxed `EmailHtmlFrame` bridge; deferred).

## 2026-08-06 — One universal macOS build; embedded AI refused on Intel at runtime

**Decision:** macOS ships exactly ONE artifact: a universal
(`universal-apple-darwin`) DMG that launches on every Mac. The separate,
`--no-default-features` Intel bundle (`build-mac-intel`, `verify-mac-intel`,
`dist-mac-intel`, `tauri.intel.conf.json`) is retired, and the Homebrew cask
becomes single-artifact. The embedded llama.cpp provider is refused on macOS
x86_64 at **runtime** — `ai::gpu_plan::embedded_runtime_supported` gates the
capability probe, the provider loader, model auto-select, and both provider
pickers. Intel Macs get the whole app minus embedded AI, with OpenRouter as the
realistic alternative; no CPU fallback is offered.
**Context:** A user on an Intel Mac reported every AI turn failing with
`Prefill decode failed: Decode Error -3: unknown`. `-3` is `GGML_STATUS_FAILED`
from `process_ubatch` — ggml's Metal kernels need an Apple7-family GPU, which an
Intel Mac does not have. Cargo features apply per build, not per slice, so the
universal bundle's x86_64 slice unavoidably carries a Metal-backed runtime it
cannot execute (`llama-cpp-sys-2` disables Metal only for watchOS). The two-DMG
split was supposed to prevent exactly this, and silently failed to: the Intel
exclusion only ever applied to the bundle Intel users mostly did not download.
The app then actively led them in — `ai_capability_from` treated x86_64 as
capable and the Settings provider tabs were ungated. Enforcing the rule in code
rather than in the build makes it hold no matter which artifact a user installs,
which in turn makes the second artifact pointless. Its remaining benefit was
~100 MB of download; its cost was a doubled release pipeline plus a download
page that has to guess the visitor's chip — and `navigator.platform` reports
`MacIntel` on Apple Silicon too, so that guess is unreliable in exactly the
browsers (Safari, Firefox) that lack UA-Client-Hints. **The runtime gate is
therefore load-bearing, not defence in depth** — it is the only thing between an
Intel Mac and a guaranteed inference failure.
**Rejected:** A single-arch arm64 `build-mac` (closes the hole at build time,
but a wrong download hands an Intel user a DMG that will not open at all, and it
still needs arch detection on the site). Keeping the Intel DMG as a smaller
optional download (the ~100 MB saving does not pay for a second signed,
notarized, verified pipeline once correctness no longer depends on it). Falling
back to `n_gpu_layers = 0` on Intel so embedded AI "works" — CPU-only inference
is too slow to be a real product experience; it would trade one bad experience
for another. Retrying on CPU after a fatal decode error — same objection, plus it
would mask genuine GPU faults on Apple Silicon. Leaving release builds to discard
ggml's WARN/ERROR lines: they are now retained in a ring buffer and quoted in the
decode error, because the line naming the real cause was being thrown away
precisely when it mattered.

## 2026-08-14 — Chat is scoped to one account, coupled to the mail list except in unified view

**Decision:** A chat conversation always answers from exactly one concrete account,
shown in a picker on both chat surfaces. Selecting a single account in the sidebar
re-points chat at it. Retargeting chat moves the mail list to match — **except** when
the list is showing "All accounts", where it stays unified. A thread is offered as
chat context only when it belongs to the account chat is scoped to. In unified view an
email from another account is not grounded on, but is not ignored either: the panel
names the account it belongs to and offers a one-click switch.
**Context:** Chat is structurally single-account — retrieval and all 12 account-scoped
tools take one account id — while the mail list can show every account. The two
selections could disagree silently: a question about mail living in another account
answered "no matching emails found", indistinguishable from having none, and an email
from account A could be handed as context to a chat answering from B. Making the scope
visible and changeable turns a wrong answer into a correctable one. The unified
exception exists because "All accounts" is a view the user deliberately chose;
collapsing it to one account as a side effect of retargeting a chat would throw that
away.
**Rejected:** *Give chat `AccountScope::AllEnabled`* so unified chat searches every
account — the DB layer already supports it (it backs the unified inbox), and this
dissolves the mismatch entirely rather than managing it. Deferred, not dismissed: it
means threading `AccountScope` through retrieval and 12 tools and deciding how
citations and drafts behave across accounts. The coupling above is correct behaviour
until that lands, and stays correct after. *Auto-switching chat to the open email's
account* — keeps unified browsing but changes the answering account as the user clicks
around, which is surprising and costs a conversation switch per click. *Silently declining the cross-account thread* — the first
attempt. It removed the incoherent grounding but recreated the original confusion in a
quieter form: asking about the email plainly on screen still produced an answer from a
different mailbox, now with nothing at all to explain why.

## 2026-08-15 — Read state and delete write back to Gmail; `gmail.modify` is used, not just held

**Decision:** Marking a message read and deleting it now push to the account, not just
the local DB: Gmail `users.messages.modify` (remove/add the `UNREAD` label) and
`users.messages.trash`. Read state is local-first with a best-effort push, so mail can
be read offline; delete is provider-first, so a message that could not be removed at
the provider stays visible locally instead of diverging with nothing to retry it.
Trash — never `messages.delete` — keeps the action reversible from Gmail's own UI.
Gated on `provider_supports_mailbox_writes`, which is Gmail-only today; IMAP flags and
Graph `isRead`/move are follow-up work and stay local until then.
**Context:** Google rejected the restricted-scope verification on 2026-08-15 asking why
`gmail.modify` was necessary "or why narrower permissions cannot be used". It was a
fair question: the app read mail and wrote drafts and nothing else, so
`gmail.readonly` + `gmail.compose` would have covered every call it actually made. The
same gap was a live user-facing bug — archiving or reading a message in EmailOps left
it unread and in the inbox everywhere else, which is not what an email client means by
those actions. Implementing the writes fixes the product gap and makes the scope
honestly demonstrable in the verification video, which must show the change landing in
the user's Gmail account.
**Rejected:** *Narrow the scopes to `gmail.readonly` + `gmail.compose`* — approvable
and strictly least-privilege for the code as it stood, but it locks the app out of
mailbox state permanently (a re-consent for every existing user to get it back later)
and ships an email client that cannot archive. *Re-shoot the video and argue that
`gmail.modify` is the standard email-client scope* — the reviewer had already asked
the narrower question, and no video can demonstrate functionality that does not exist.

## 2026-08-26 — Adoption is tracked by a daily metrics job, not by telemetry

**Decision:** A scheduled GitHub Actions workflow (`.github/workflows/metrics.yml`)
records release download counts and star count once a day, appends them to
`downloads.csv` on an orphan `metrics` branch, and posts the numbers plus the daily and
7-day deltas as a comment on one long-lived issue. Delivery is GitHub's own
notification mail, so the setup needs no configured secret at all. The app itself gains
no telemetry of any kind.
**Context:** There was no way to answer "how is adoption going?" without opening the
releases page and adding up assets by hand, and the GitHub API reports only today's
`download_count` — it keeps no history, so the history has to be kept on our side. The
privacy-first architecture rules out measuring anything from inside the app, which
leaves the distribution side as the only honest signal. `GITHUB_TOKEN` is minted per
run and expires with it, so nothing has to be stored, rotated, or kept out of a tracked
file — and no mailbox address appears anywhere in the repo.
**Rejected:** *In-app telemetry* — contradicts the core promise, and the numbers would
not be worth it. *SMTP from the workflow* (built first, then dropped) — it worked, but
it cost five repository secrets including an app password to store and rotate, to
deliver mail that GitHub already delivers for free. *Committing the CSV to `main`* — a
bot commit a day would bury real work in the log, in blame, and in every release diff;
an orphan branch shares no history with `main` and holds exactly one file. *A
Claude Routine on a schedule* — the fired sessions get no MCP tools, and GitHub is
reachable only through MCP in that environment, so the job could never read a single
`download_count`.

## 2026-09-04 — Repository traffic is archived daily, because GitHub forgets it

**Decision:** The metrics workflow also snapshots `/traffic/views`, `/traffic/clones` and
`/traffic/popular/referrers` every day, into `traffic.csv` and `referrers.csv` on the
same orphan `metrics` branch, and puts today's visits and top referrers in the daily
report. The traffic endpoints need push access that the run's `GITHUB_TOKEN` may not
have; when they answer 403 the run logs why and reports downloads and stars as before.
**Context:** Downloads went from 124 to 200 in ten days with no release in between, so
the growth was new users rather than the update toast — and there was no way to tell
which channel had sent them, because GitHub keeps traffic for only 14 days and then
discards it. The referrer list is the one thing that names the channel, and it expires
before a weekly review would ever catch it. Archiving it costs one API call a day.
**Rejected:** *A personal access token* to guarantee the traffic endpoints answer — it
reintroduces exactly the stored, rotatable secret that the issue-comment delivery was
chosen to avoid; if `GITHUB_TOKEN` turns out not to be enough, the right answer is to
drop the traffic half, not to add a secret. *Storing the 14-day totals* each payload
carries — they are a rolling sum, meaningless once stitched into a history, so only the
per-day breakdown is kept. *A separate workflow* — same schedule, same branch, same
report; a second job would just double the moving parts.

## 2026-09-04 — Windows splits the secrets vault across chunked keychain entries

**Decision:** On Windows the OS keychain backend is wrapped in a `ChunkedKeychain`
that transparently splits any value over ~2 KB across extra credential entries and
reassembles it on read. macOS and Linux keep storing the secrets vault as a single
item. Secrets stay entirely inside the OS credential store on every platform.
**Context:** `services::secrets_vault` deliberately consolidates every secret (OAuth
tokens, IMAP credentials, API keys) into ONE keychain item, because macOS authorizes
keychain access per item and N accounts meant N prompts at startup. Windows'
Credential Manager caps a credential blob at `CRED_MAX_CREDENTIAL_BLOB_SIZE`
(2560 bytes), measured after UTF-16 encoding — so barely 1280 ASCII characters. A
single Microsoft OAuth refresh token can exceed that on its own, and the shared vault
blob exceeds it as soon as a second account is added, which is why adding an
Outlook account (and IMAP accounts added after one) failed outright on Windows with
"Value of 'password encoded as UTF-16' is longer than the platform limit of 2560
chars" (issue #54). Windows has no per-item prompt, so splitting costs nothing there.
**Rejected:** *Splitting on every platform* — reintroduces the macOS prompt storm the
single-item vault exists to remove. *Moving the vault blob to an encrypted file with
only its key in the keychain* — breaks the "OAuth tokens live in the OS keychain, not
in files" guarantee in `CLAUDE.md`, and puts the ciphertext somewhere a backup or sync
tool can copy. *Storing only refresh tokens to shrink the blob* — Microsoft refresh
tokens are themselves multi-kilobyte, so the limit is still breached by one account.

## 2026-09-09 — Narrowing an account's sync range is not retroactive

**Decision:** Changing an account to a narrower sync window stops EmailOps fetching
anything older than the new floor, but leaves mail already downloaded in place. Only
the *future* of the sync is bounded; the local database is never pruned to match.
**Context:** The setting reads as a description of a window ("last 7 days"), so it is
natural to expect the inbox to end up containing exactly that window. Issue #50's fix
made the floor take effect immediately — the run in flight stops and a replacement
starts on the new range — which sharpened the question: after narrowing, an account
still shows older mail that the new range says it should not have. That is deliberate.
Mail already synced is the user's local copy of their own mailbox, and a setting about
how much to *download* is a weak mandate for deleting data the user can still see and
search. Widening is symmetric: it re-opens the backfill rather than re-fetching what
is already stored.
**Rejected:** *Pruning below the floor on narrowing* — destroys local data as a side
effect of a settings change, with no undo, and the mail may be the only copy if the
provider has since removed it. *Hiding rather than deleting below-floor mail* — the
data stays on disk, so the disk-space motive for narrowing is not served, and search
results silently disappearing is harder to understand than mail simply remaining.
*Prompting the user to choose at narrowing time* — a modal on a settings toggle, for a
question most users have no basis to answer.

## 2026-09-10 — Tag Board groups by one classified tag type at a time

**Decision:** The Tag Board renders a responsive grid of columns for a **single**
classified tag type at a time — Company, Priority, Intent or Topic, chosen from a
segmented control and persisted in `user_preferences` under `tagboard_tag_type`.
Columns are the tag values of that type, ordered by thread count, capped at 15.
Clicking a card opens the thread in the ordinary `EmailView` pane beside the board;
clicking a column header applies that tag as a smart filter and switches to the inbox.

**Context:** Emails already carry four independent classification dimensions, and the
sidebar only ever exposed them as a flat list of filters — one tag at a time, with no
way to see the shape of the mailbox along a dimension. A board answers "what is in my
mailbox, grouped how the classifier sees it". Keeping one dimension on screen makes the
per-column counts comparable and each thread appear exactly once, which is what makes
the board readable as a distribution rather than a pile.

Columns come from a new `get_tag_stats` command (live `COUNT(DISTINCT thread)` over
`email_tags`), not from the cached `smart_filter_suggestions` table the sidebar reads.
The cache is only written by an explicit "recalculate filters", so a board built on it
would be empty for any user who never pressed that button and stale for everyone else.

**Rejected:** *All four tag types mixed into one grid* — counts across dimensions are
not comparable, and a single thread appears in up to four columns, so the board reads
as a pile rather than a breakdown. *Rows per tag with horizontally-scrolling card
strips* — hides most of each tag behind a sideways scroll and wastes vertical space,
which is the axis a mail client has to spare. *Inline expansion of a card inside its
column* — a ~17rem column is a poor place to read HTML mail, and expanding one card
pushes every other column's content out of alignment. *Reusing the sidebar's saved
suggestions as the column source* — stale by construction, and empty until the user
finds the recalculate button.

## 2026-09-10 — Tag Board places a thread under the tag of its newest classified message

**Decision:** On the Tag Board a thread belongs to exactly one block per dimension: the
tag value carried by its most recent classified message (deleted messages and copies
outside inbox/sent do not count as "newest"). The rule is opt-in through
`EmailWindow.latest_tag_only`, which only the board sets; the sidebar smart filters and
their counts keep the inbox rule — a thread matches when **any** of its messages carries
the tag. Also: picking the Custom range seeds the two date boxes with the last 30 days
instead of leaving them empty.

**Context:** A six-message thread whose messages the classifier had labelled delivery,
notification, scheduling, request and conversation appeared in five intent blocks at
once, so the board read as a pile again. The newest message is what the thread is
currently about, and it is what the card already shows. The filter is a `NOT EXISTS`
probe on `idx_emails_thread_latest`, measured at 0.34 s vs 0.21 s for the unfiltered
company query on a 6 GB mailbox. Empty custom dates were rendered by macOS WebKit as
today's date while filtering nothing, so the board looked like "today → today" and
listed years of mail.

**Rejected:** *Most frequent tag in the thread* — ties are common in short threads and a
long-running thread would be stuck with its early history. *Applying the rule to the
sidebar filters too* — the user chose the any-message rule for the inbox on purpose:
replying to a company must not drop the thread from that company's filter. *Deduplicating
across blocks on the frontend* (as the priority ordinal dedupe does) — blocks page lazily,
so a thread's "right" block may not be loaded yet, and the counts would still disagree.

## 2026-09-10 — Chat dates are the user's local day; drafts need an explicit request; RAG sources carry ids

**Decision:** Every date the chat shows the model or parses from it — "today" and
"tomorrow" in the system prompt, the summary shortcuts' windows, `since`/`until` bounds,
message dates in tool results — is computed in the machine's local zone through a new
`Clock::utc_offset_secs()` seam (`SystemClock` reads the local offset; `FixedClock` pins
one for tests). `generate_email_draft` only runs on a turn whose question asks to
write/reply/draft, or on a short confirmation right after the assistant offered a draft;
otherwise the call is replaced by a note to the model and nothing is saved. Pre-retrieved
RAG sources carry `id=` on their header line and seed the turn's `email://` allowlist.
`search_emails` prepends the real total ("showing 25 of 156 matching threads") when a page
is full, and the daily/weekly summary shortcuts ask for received mail only.

**Context:** A judged pass of 18 real questions against the production mailbox found: a
lookup ("primer correo que envié a X") that ended in an unrequested reply draft saved to
the provider; RAG answers whose links were either the citation number (`email://2`) or a
prompt-example id, because the sources had no ids and the RAG allowlist started empty;
"¿cuántos correos de X?" answered "25" on a sender with 156; a Thursday labelled
"martes"; the user's own sent reply summarised as received mail; and every day boundary
computed in UTC for a user in Europe/Madrid.

**Rejected:** *Gating drafts only on inferred (nameless) tool calls* — the observed turn
also emitted explicit `generate_email_draft` calls after the first inferred one.
*A COUNT query inside the search SQL* — the sender filter is a three-arm UNION assembled
in two phases; re-running the same search with a 500-row probe only when the page is
full is one line and costs nothing on the common path. *Applying the draft gate to
thread-bound turns* — those already expose the tool only when the question asks for it.

## 2026-09-11 — The open email is context on an all-tools turn, not a tool-less mode

**Decision:** When the chat panel has an email open, the turn runs as an ordinary
turn — every tool available, no retrieval, no planner — with the thread injected as an
"OPEN EMAIL" block in the user message that states both halves of the contract: answer
from it when the question is about it, ignore it and use the tools otherwise. A keyword
hint (`question_leaves_thread`) still short-circuits obvious mailbox-wide questions to a
plain turn with no thread block, and a language-agnostic net catches the remaining
misses: an answer that claims to have no tools or no inbox access triggers one
corrective retry that salvages and runs the tool call the model then emits. Only a
conversation explicitly created with "chat about this thread" keeps the thread-bound,
tool-less path.

**Context:** Thread-bound turns exposed zero tools (a translate request had once saved a
reply draft), so "que correos tengo hoy" asked with an email open was answered "No tengo
herramientas disponibles para acceder a tu bandeja". Keyword detection of "is this about
the thread?" is not robust to paraphrase or to the FR/DE users; letting the model decide
with the whole question and the thread in front of it is. The draft gate
(`draft_call_allowed`) now prevents the original regression with every tool on the menu;
a refused draft call is labelled `(refused: no draft requested)` in the trace.

**Rejected:** *Improving the keyword classifier* — it would keep growing case by case
and never cover four languages. *A separate classifier round trip* — a per-turn cost
for a decision the main model can make in the same call. *Keeping thread mode tool-less
and re-running the whole turn on a refusal* — the corrective-retry ladder already
exists and costs one extra generation only on failure.

## 2026-09-11 — Chat search exposes the classifier's tags and never returns detector spam

**Decision:** `search_emails` (the chat tool) takes `intent` and `topic` filters backed by
the classification tags, plus `with_bodies` to inline cleaned bodies in one call. The
query planner maps concepts the mailbox never spells out onto `intent` — prospects /
potential clients / leads → `introduction` (then `question` / `request`), newsletters →
`newsletter`, marketing → `promotion` — instead of a literal keyword. Chat searches drop
mail the junk detector banded as spam or phishing (unless the user overrode it); the app's
own search box is unchanged. The system prompt defines a prospect (someone asking about
*your* services) and excludes vendors, recruiters and newsletters from that label.

**Context:** "últimos correos de prospects" on the consulting inbox ran
`search_emails(query="prospects")` → nothing, then broad retries, and the answer listed two
SEO vendors (one banded spam), a job seeker and one lead — while the five real prospects
(intent introduction/question/request, clean) never appeared. The classifier already
encoded the answer; the tool schema did not let the model reach it. Nine rounds and
26.6 s, half of them one `get_email_body` per row.

**Rejected:** *Teaching the keyword router about "prospects"* — a concept, not a word, and
one of many. *Excluding spam at the DB search for every caller* — the inbox search box must
still find a message the detector got wrong. *Relying on the model to filter vendors out
of a broad result* — it did not, twice; the definition in the prompt plus the intent filter
make the right set the default rather than a judgement call.

## 2026-09-11 — Concepts reach the search through the tag glossary, not per-concept prompt rules

**Decision:** The chat maps a *kind* of mail in the question ("prospects", "quote requests I
sent", "complaints in 2025", "newsletters this week") onto the classifier's tags through
data, not prose. Each built-in intent and topic carries a one-line definition next to its
name in `services::classification` (`TagGlossary`); the user's configured tag list — plus
any tag value actually present in `email_tags` that the list no longer names, since rules and
older defaults keep tagging — with those definitions, is rendered into the `search_emails`
parameter menu (a new
`Tool::parameters_schema_for(db)` hook, since the vocabulary follows Settings), into the
query planner prompt (`{{intent_definitions}}` / `{{topic_definitions}}`), and the planner
prompt keeps a handful of diverse examples. `search_emails` also takes `mode="semantic"`,
which ranks the query by meaning through the chat's hybrid retrieval and then applies the
sender / recipient / date / tag filters in memory — for descriptions no tag captures. The
system prompt keeps one generic sentence ("a kind of mail is a tag filter or a semantic
search, never a keyword") in place of the PROSPECTS paragraph.

**Context:** The prospects fix taught the prompt one concept. The next question ("emails
where I ask a provider for a quote") would have needed its own paragraph, and so would every
concept after it — a system prompt that grows per concept and still never covers what the
user says next. The classifier already has a vocabulary; what was missing was its meaning
in front of the model at the two points where it chooses filters. The first run also showed
why the menu cannot be the Settings list alone: the production mailbox has ~1,000 emails
tagged `newsletter` while its Settings list had dropped that intent, so a data-driven planner
could not name the tag the old hard-coded prompt used to spell out.

**Rejected:** *A concept → intent lookup table in code* — the same growth problem in a
different file, and blind to paraphrase and language. *Putting the definitions in the
static tool description* — the tag list is user-configurable, so the menu must be rendered
from the DB. *Semantic mode as a separate tool* — one tool with one `mode` switch keeps the
filters and output shape identical, so the model needs no second contract. *Feeding the
definitions to the classifier prompt in the same change* — it would move classification
results and needs its own eval run; the glossary is there for it when that lands.

## 2026-09-11 — UI verification drives the real app through an embedded, dev-only WebDriver

**Decision:** Agent-driven UI verification (`.claude/skills/verify-emailops`) drives the real
desktop app through `tauri-plugin-wdio-webdriver`, an embedded W3C WebDriver server. It is
behind the `webdriver` cargo feature (never in `default`, a `compile_error!` refuses release
profiles) and only starts when `TAURI_WEBDRIVER_PORT` is set at launch; the skill launches its
own instance on a separate Vite/Tauri port against the synthetic demo DB. cua-driver stays as
the native layer (window screenshots, menus, focus-free checks).

**Context:** Frontend fixes kept shipping on jsdom-only evidence and coming back as "still
broken" screenshots; the driver that can see the running app (cua-driver, Accessibility)
exposes nothing of a WKWebView whose window sits on another Space, which is the normal state
when the terminal is full-screen. DOM-level driving inside the app does not depend on Spaces,
focus or AX, and reaches the real IPC and data.

**Rejected:** *Opening the frontend in a browser through a dev HTTP bridge* — a second
dispatch path over 213 Tauri commands and 27 events that would drift from the real one, and a
localhost surface on the mailbox reachable from any web page. *`tauri-driver`* — no macOS
support. *CrabNebula's driver* — paid, external process. *Always-on plugin in debug builds* —
`make dev` often holds the production mailbox; an unauthenticated automation port must be
opt-in per launch.

## 2026-09-14 — Chat routing is a retrieval hint, never a capability gate

**Decision:** Every chat turn runs the tool loop with the full, feature-gated tool menu.
The route (`RagFirst` / `ToolsFirst`) only decides whether RAG sources are pre-retrieved
into the turn. Reaching a tool must never depend on the question matching a keyword; the
routing keyword list is not grown to chase paraphrases.
**Context:** `RagFirst` used to be sources-only, so any question the keyword heuristic
missed ("¿qué tengo pasado mañana?", "which conversations are still open?") could never
reach `list_calendar_events`, `list_open_threads` or `memory_search`. A fix that added
more keywords was rejected by the developer as fragile and reverted. Measured on the
embedded runtime: the system prefix is identical on both routes (~6.9k tokens, the tools
section already lives in the system prompt), so exposing tools on `RagFirst` turns costs
nothing in the KV-prefix cache.
**Rejected:** more routing keywords (fragile, every paraphrase and language needs an
entry, a miss silently removes capabilities); an LLM route classifier (a model round-trip
on every turn, and still a gate that can be wrong); dropping pre-retrieval altogether
(kept open: `chat.routing_mode=always_rag|always_tools` stay available to A/B it on the
eval).

## 2026-09-15 — The account name is the sender name on outgoing mail

**Decision:** `accounts.name` is the display name EmailOps puts in the `From` header of
mail it sends through Gmail and IMAP/SMTP (`"Name" <address>`), editable per account in
Account settings. An account whose name is blank or equal to its own address (as older
rows store it) has no sender name and sends the bare address. "No name" is stored as an
empty name and every reader falls back to the address; the address is never written in
its place. Gmail accounts take the name from Gmail's "Send mail as" setting
(`users.settings.sendAs`, readable under the `gmail.modify` scope already granted) when
connected, and once on the next sync for accounts connected earlier; a name the user set
is never replaced. Outlook is unaffected: Graph takes the sender name from the mailbox.
Drafts pushed to Gmail keep a bare address.
**Context:** Mail sent from EmailOps went out as `From: address`, so recipients saw no
name and the synced IMAP Sent copy had an empty sender. Gmail's profile endpoint carries
no name, so Gmail accounts were stored with their address as the name. `accounts.name`
already meant "your name on this account" (Outlook profile name, the IMAP setup display
name, the sender of the optimistic Sent row); its only label use is the Dashboard
account panel.
**Rejected:** a separate sender-name field beside an account label, as Thunderbird and
Apple Mail do — a second source of truth for the same identity, for a label shown in one
panel. It pays off only with per-account aliases / send-as identities, which would bring a
proper identity model (address + name + signature) anyway. Deriving the name from past
Sent headers — implicit, and wrong for accounts that never sent with a name. Writing the
address as the name when there is none — it invents data and hides the "no name" state;
the fallback belongs to whoever reads the name.


## 2026-09-15 — Chat search rows state their thread's size; reading the thread stays the model's call

**Decision:** `search_emails` keeps returning one row per thread. A row whose thread
holds more than one message carries `messages=N`, and the result opens with a
conditional hint: call `get_thread` when the answer needs the whole conversation. The
tool never inlines a thread on its own. The count uses `get_thread`'s own row filter,
so the number the model sees is the number the follow-up call returns.
**Context:** Asked to summarise an exchange with one person, the model answered from
the single representative row of a six-message thread in 4 of 5 eval-harness runs;
nothing in the row said it stood for more mail.
**Rejected:** Auto-expanding threads inside `search_emails` when few threads match —
deterministic, but it spends context on every small result, including listing and
counting questions that never need the thread. A dedicated "exchange with a person"
route — the narrowest fix, and it would lean on keyword detection. An unconditional
"call `get_thread`" instruction — it would fetch threads for questions that don't need
them.

## 2026-09-15 — Nightly local verification; Markdown summaries in git, HTML reports local

**Decision:** A launchd agent on the developer's Mac runs `make verify` every night at 03:00
and commits one Markdown summary per run under `docs/verification/` on the checked-out
branch (never `main`, never pushed). The full HTML report stays in the gitignored
`src-tauri/reports/verify/`, pruned to the last 10 runs. Private (real-mailbox) verification
is never summarised or committed.
**Context:** The chat evals need the local 35B model on the Metal GPU, the demo DB and a
WebDriver-driven dev app, so neither GitHub CI nor a cloud routine can run them. A full run
is about 41 MB (results.json 4.6 MB, informe.html 4.1 MB, raw layers and app evidence),
roughly 15 GB a year if committed nightly. A run while another EmailOps instance holds the
demo DB fails every eval with Metal out-of-memory, so the job skips that night instead of
recording false failures.
**Rejected:** Committing the HTML report without screenshots — about 4 MB a run, 1.5 GB a
year. Keeping full runs on a dedicated branch or in Git LFS — a second branch to maintain or
a new dependency. Running in GitHub CI or a cloud routine — no GPU or local model there.

## 2026-09-15 — Full verification runs at the start of each release, not on a schedule

**Decision:** Phase 1b of the release skill runs `make verify-release`: a full `make verify`
on the commit being released, whose Markdown summary under `docs/verification/` ships in the
`chore: release vX.Y.Z` commit. Newly failing tests stop the release until the developer has
triaged them. There is no scheduled run. This supersedes the nightly launchd entry above; the
rest of that entry (summaries in git, HTML reports local and pruned to the last 10, private
runs never summarised) still holds.
**Context:** The developer does not want verification automated on a schedule. Tying the run
to the release checks exactly the code that ships, and happens when the developer is present
to triage the failures.
**Rejected:** The nightly launchd agent at 03:00 (installed and removed the same day) — an
unattended job committing onto whatever branch was checked out, and competing for the GPU with
any EmailOps instance left open.

## 2026-09-17 — The chat answers questions about EmailOps itself from the bundled guides, via RAG

**Decision:** The published user guides (`docs/site/<lang>/*.md`, all four languages) are
compiled into the binary and indexed — FTS5 plus sqlite-vec, in the same shape as the
mailbox and memory corpora — so an ordinary chat turn can answer "how do I…" questions about
the app from them. The guides are a second, separate retrieval source: they never mix with
mailbox ranking, enter the prompt only past a vector-similarity gate, are served in the
answer's language (a hit on any language swaps for its sibling section), and are cited with
a `help://<lang>/<page>#<anchor>` link that opens the public docs page. When the answer cites
a section whose front matter carries a `nav:` target, the app opens that Settings tab or view
(`ToolEffect::NavigateTo`, from the model's citation, never from the lookup alone).
**Context:** The chat knew nothing about the app: "how do I connect Ollama?" went through
mailbox retrieval and ended in "not found" or an invented menu. The guides already existed
in four languages with stable heading anchors, so they are the single source of truth — a
stale guide is now a wrong chat answer, and the docs README says so. Per-turn content stays
out of the system prompt (KV-prefix cache); the block rides in the final user message like
the memory header. The `nav:` map is keyed on the language-invariant anchor ids and checked
for parity across languages by `scripts/check-docs-parity.sh` and a unit test.
**Rejected:** A keyword router plus a lexical `app_help` tool — a keyword list in four
languages is brittle, and a tool adds prompt cost on every turn while RAG reuses the query
embedding the mailbox retrieval already computes. Putting the guides in the system prompt —
about 9k tokens per turn on an 8k-token local context. Indexing only the UI language —
the developer chose all four so a question in one language finds the section whatever
language it is asked in. Navigating whenever the lookup matched — a false positive of the
gate would move the user's screen; the answer's own citation is the safer signal.
## 2026-09-17 — Chat states its single-account scope instead of searching every account

**Decision:** The chat system prompt names the one mailbox the turn can search, tells the
model that other accounts exist and are unreachable, and requires it to report absence as
scoped ("not in <address>") and suggest switching the chat's account rather than declaring
the mail was never sent. Every empty `search_emails` result names that mailbox too. Chat
stays structurally single-account; `AccountScope::AllEnabled` remains deferred.
**Context:** Asked for a message that lived in another enabled account, the model reported
it absent as fact and then presented unrelated years-old mail from the account it could see
as if it answered — the exact ambiguity the 2026-08-14 entry predicted ("indistinguishable
from having none"). Naming the scope costs one static block: it varies per account, not per
turn, so it rides inside the existing `user_identity` text in the KV-cached system prefix
without busting the anchor, and `prewarm_chat` inherits it by construction because both
paths call the same `build_prompt`. Keeping it out of the user-editable `chat.system`
template avoids a new placeholder the thread-bound path would have to bind.
**Rejected:** Threading `AccountScope::AllEnabled` through retrieval and the 12
account-scoped tools — still the real fix and still deferred, since it means deciding how
citations and drafts behave across accounts. Probing sibling accounts on an empty result to
offer a one-click switch — more machinery than the wording needs, worth revisiting only if
the prompt fix proves insufficient in practice.

## 2026-09-17 — AI processing limit: whole small accounts, day cutoff for large ones

**Decision:** Embeddings and classification cover every email of an account with at most
`ai_max_email_count` live emails (default 1000). Accounts above that only process emails
newer than `ai_max_email_age_days` (default 365). The count is evaluated per account; a
count limit of 0 always applies the day cutoff, a day limit of 0 removes the cutoff.
**Context:** The day cutoff alone was global, so a small account with a long history kept
most of its mail unclassified. Intent/topic search filters then silently missed it (a
chat search for contact-form requests found 4 of 31).
**Rejected:** Union semantics (always the newest N emails plus anything within D days) —
the developer preferred the simpler switch. A per-account settings UI — the global pair of
limits already makes small accounts whole without extra configuration.

## 2026-09-18 — The routing keyword list accelerates, the query planner decides

**Decision:** In `chat.routing_mode = auto` a keyword or date hit still settles the route for
free (and skips nothing else). A miss no longer falls to `RagFirst`: the query planner runs,
and its verdict sets the route — a plan carrying a real filter (from/to/subject/date/tag/unread)
becomes `ToolsFirst` with that call pre-seeded, a `defer` or a keyword-only plan stays
`RagFirst`. Forced modes and follow-up inheritance are unchanged.
**Context:** `TOOLS_FIRST_KEYWORDS` is an EN/ES substring list, so "que emails tengo de X" and
every German or French question fell to RAG: retrieval it did not need, and no pre-seeded
search. Growing the list was already rejected (14/09/2026). The planner reads any language and
already turns a question into a filter, so its Search/Defer verdict IS the routing signal.
Measured: the planner prompt is ~1.5k tokens, capped at 128 generated, and runs on the scratch
sequence with `cache_prompt=false`, so it never touches the chat KV prefix; warm latency
997-1466 ms on an M5 Pro with qwen3.5-4b-q4.
**Rejected:** the planner deciding every turn including keyword hits (pays a model call where a
substring already answers, and on a 16 GB M1 that lands on every open question); adding de/fr
keywords (the 14/09 rejection, one entry per paraphrase per language); a trained router model
(worth revisiting only if the planner call proves to be the bottleneck on older machines —
the route classifier could ride on the embedding already computed for RAG).

## 2026-09-18 — A classifier tag ranks a chat search, it never gates it

**Decision:** `search_emails`' `intent` / `topic` filters put the tagged rows first and keep the
other matches behind them, with a note giving both counts ("N emails carry the intent/topic asked
for; the M rows after them…"). A "how many of this kind" answer uses N. The tag clause now also
binds `tag_type`, so `intent=billing` cannot be answered by a company called billing. The sidebar,
Tag Board and lens filters are untouched — they stay exact.
**Context:** an email stores exactly ONE intent (`PRIMARY KEY (email_id, tag_type)`, prompt says
"pick exactly ONE"), while a real email is often several things at once, and mail outside the AI
window carries no tag at all. As a hard filter that lost whole answers: "¿qué correos de BorgBase
tengo sin leer?" planned `intent=notification` over invoices tagged `billing` and replied that
there were none. Measured on the production mailbox: 34.713 classified emails, mean confidence
0.94 (only 444 below 0.8), so confidence cannot be used to soften the filter either.
**Rejected:** real multi-label storage (new PK, primary + secondary intents, reclassifying 34.713
emails — hours of local inference, and a thread would start appearing in several Tag Board blocks);
dropping intent/topic from the chat tool (loses counting by kind, which is the only thing tags buy
there); leaving it as a gate and only warning (the note only fires when rows come back, so the
zero-result case — the one that hurt — stayed silent).

## 2026-09-19 — One-shot prompts get a prefix sequence; letter-scoring does not ship

**Decision:** the llama.cpp actor reserves one KV sequence (`AUX_SEQ`, seq 3) for the
invariant head of a one-shot prompt, and the query planner marks its split with
`complete_with_prefix`. The slot is the first thing evicted from either direction —
`plan_oneshot_cells` gives it up before the chat prefix, `chat_must_evict_aux` drops it
before a chat turn would truncate — and it is bypassed below a 16k window. The classifier
does **not** get one, and does **not** answer by picking a lettered menu: it keeps writing
JSON.
**Context:** both surfaces are one-shot completions with `cache_prompt=false`, so both
re-process their whole prompt every call. Measured on qwen3.5-4b-q4_k_m against the demo
DB before changing anything: the planner spends 682 ms of its 1161 ms on prefill of a
1693-token prompt, of which ~1.5k is the same instructions, examples and glossary every
time — so caching it is nearly all of the win. The classifier's prompt is 317 tokens and
its prefill is 2 ms of 587 ms; its cost is the ~28 tokens it generates, which a cache
cannot touch. After the change the planner runs at 495 ms mean / 379 ms p50 (prefill
46 ms) with identical output — 21/21 eval cases, same 17/4 search/defer split, prompt
byte-identical — and the chat turn still reuses 7459 of 7466 prompt tokens after a
classification burst, at +54 MiB of RSS. `EMAILOPS_AUX_PREFIX=0` restores the old path
from the same build (1185 ms), which is how the delta was isolated.
**Rejected:** a second slot for the classifier (its prefill is 2 ms — the slot would cost
cells and recurrent state to save nothing measurable). Answering the classifier with a
lettered menu scored from the logits: it is 2.4× faster (587 → 246 ms per email, 102 →
244 emails/min) and never produces an unparseable reply, but on 145 labelled synthetic
cases it costs 17-22 points of accuracy on every axis — intent 78.6% → 61.4% strict
(macro-F1 0.794 → 0.572), topic 71.0% → 54.5%, urgency 71.7% → 49.7%, all three axes
right on 90/145 emails → 44/145 — so the tags would get materially worse to make a
background job faster. Scoring the tag NAME instead of a letter is worse still on this
mechanism, because only the first token of a label is scored and the built-in tags share
initials (`notification`/`newsletter`, `complaint`/`conversation`). A grammar (GBNF) to
constrain the JSON: both harnesses measured zero unparseable replies (145 classifier
cases × 3 repeats, 21 planner cases), so there is nothing for it to fix.

## 2026-09-19 — Model-family behaviour is decided by GGUF metadata, not by file name

**Decision:** Any behaviour that depends on which model family is loaded reads the GGUF's
own header (`general.architecture`, via `ai::gguf`) and treats the file name only as a
fallback for headers that cannot be read. The first case is the Qwen 3 no-think primer
(`ai::think_priming`): architecture `qwen3*` gets the closed `<think></think>` block,
everything else does not, whatever the file is called.
**Context:** the primer was keyed off a `qwen3` file-name prefix, so every Qwen 3 build not
named that way — a re-quant, a community fine-tune, a file the user renamed, a GGUF adopted
from disk via "link local model" — got no primer. On a near-full context window that model
spends the whole generation reserve inside `<think>…`, `strip_reasoning` removes it, and the
user sees an empty answer: a silent, total failure of chat, drafts and classification on a
model the app otherwise supports. The header is written by the converter from the source
config, travels with the file, and is a few hundred bytes in, so it is both authoritative
and cheap to read before any weights are loaded.
**Rejected:** widening the file-name match (`contains("qwen3")` and friends — same class of
bug, now with false positives on look-alike names, and it still cannot see a renamed file);
asking llama.cpp for the metadata after the model is loaded (the decision is needed on paths
that must not pay for a multi-GB mmap, and it would put the rule behind the `llamacpp`
feature gate where the CI fast jobs cannot test it); a user-facing "disable thinking"
setting (makes the user responsible for a detail the file already states).

## 2026-09-21 — A tool turn's sources are the emails the tools returned; bare `[n]` only cites pre-retrieved Sources

**Decision:** A bare `[n]` opens the n-th numbered Source, so it is rendered only on turns
where no tool handed the model an email. On a turn where a tool did (`search_emails`,
`get_email_body`, `get_thread`, …), the message's sources become the emails the tools
returned — the ones the answer links first, then the rest in tool order — replacing the
pre-retrieved rows on disk and in the open bubble, every bare `[n]` is stripped, and the
answer's citations are its `email://ID` links (`plan_answer_grounding`). The prompt still
tells the model to link tool results with `email://` and to keep `[n]` for numbered
Sources, and `relink_self_numbered_citations` turns a self-numbered marker into a link when
the answer defines it; but neither is relied on for correctness.
**Context:** the UI resolved every bare `[n]` to the n-th pre-retrieved Source. When the
fact came from a tool result — which carries an `id=` but no number — Qwen 3.6 35B
numbered the bullets of its own answer `[1]`, `[2]`…, so a correct support address opened an
unrelated shipping notice. A prompt-only fix (the CITATION CONTRACT rewrite plus a reminder
under the Sources header) was measured on the demo DB, 54 chat cases, greedy decoding: it
raised tool turns with `email://` links from 22/34 to 26/36 and fixed `kelvo_support_addresses`
(`[1][2][2]` → two links), but `pc_priya_address` still answered `… [1]` for a fact in Source
`[6]` on both prompts, and real-world mail still got `[1][2][3]` in bullet
order with no links. Self-numbering survives the instruction, so the fix had to stop
depending on the model: on those two sweeps bare `[n]` appeared on 2–3 of ~35 tool turns
and was wrong in the self-numbered ones, while 22–26 of them carried `email://` links —
the links are the citation mechanism that works on tool turns, and the tool-returned
emails are the honest "sources used" list (the pre-retrieved rows were rarely what the
answer drew on). Correct source-number citations on a tool turn (`mem_borgbase_customer_number`
cited `[1] [2] [8] [9]` rightly once) are lost with the rule; that answer keeps its
`attachment://` links and its sources panel lists the four opened invoices.
**Rejected:** numbering tool results into the same citation space (`[9]`, `[10]`… in every
tool's output and the Sources panel) — the model ignored the numbers already in front of it
(`pc_priya_address`: `[1]` for Source `[6]`), so this adds format and cache churn without
making `[n]` trustworthy; resolving a tool turn's `[n]` against the tool-returned list — the
model's numbering follows its own bullets, not the tool order, so this only moves the wrong
pill; keeping `[n]` on a tool turn unless the markers read `1..k` in order — a pattern gate
that still lets a skipped number (`[1] [3]`) open the wrong email; a post-processor that
guesses the right Source from the cited sentence (matching an address against senders) — a
heuristic that only covers the shapes it was written for, and a wrong guess is worse than
no citation.

## 2026-09-22 — Chat answers cite by `email://` link only; linked emails are the sources

**Decision:** The RAG Sources block is no longer numbered (each line keeps its `id=`) and
the CITATION CONTRACT asks for a `[short label](email://ID)` link on every fact, whether
the email came from the Sources or from a tool; bare `[n]` markers are never requested.
`plan_answer_grounding` makes the emails an answer links its sources (link order, each
once); an answer that links nothing falls back to the emails the tools returned, and with
none of those the pre-retrieved Sources stay. The "show emails in list" button falls back
to the message sources when the answer cites nothing. This supersedes the 2026-09-21 rule
that kept `[n]` for RAG-only turns.
**Context:** numbers were the root of the wrong-source citations — Qwen 3.6 35B numbered
the bullets of its own answer and the UI opened unrelated emails. An opaque id cannot be
mistaken for a bullet position, and linking gives the sources panel the emails the answer
actually rests on instead of everything a tool returned. Full chat sweep (53 cases,
qwen3.6-35b-a3b-ud-q4_k_xl, demo DB), before → after: passes 49 → 49; answers with no
citation 20 → 17; answers with a bare `[n]` 3 → 0; answers with an `email://` link
32 → 36. The two cases that flipped to fail (`demo_draft_link_emitted`,
`at_fastmail_receipts`) failed on the old code too when re-run (1/3 vs 2/3, 1/3 vs 0/3).
**Rejected:** keeping `[n]` for RAG-only turns (the model self-numbers there as well,
and two citation schemes in one contract are what confused it); numbering tool results
into the Sources (the model ignored numbers it was given); an empty sources panel when
nothing is linked (the button would disappear and the user loses the only route to the
emails behind the answer).

## 2026-09-21 — The query planner decides when a chat question is about EmailOps itself

**Decision:** A question about the app (how to use, set up or fix EmailOps, its settings,
models or data) is recognised by the chat query planner, which answers `{"app_help": true}`.
That turn skips mailbox retrieval and is answered from the bundled guides; the help lookup
still runs on every route. A question about mail that mentions the app stays a mail search.
**Context:** app questions went through RAG-first retrieval, so 8-9 mailbox emails rode in
the prompt next to the guide sections — in the demo mailbox, users asking the very same
question — and the answer mixed both. The `help://` link in the answer hid it, because the
turn appends one whenever any guide section was included. The planner already reads every
question in any language, costs no extra model call on those turns, and its verdict is
measurable in `query_plan_eval`.
**Rejected:** skipping the mailbox whenever the help lookup outscores the best email
(cheapest, but a similarity threshold cannot tell "how do I connect Ollama?" from "what did
users say about Ollama?"); keeping both corpora and only asserting the guide is cited first
(accepts the mixing the change set out to remove); a keyword list of app terms (fails on
paraphrase and on every language the list does not cover).

## 2026-09-22 — The query planner names the guide page; the page is a preference

**Decision:** The planner's app-help verdict can name a guide page
(`{"app_help": "<page>"}`), picked from a table of contents generated from the English
guides (page title plus section titles, ~370 tokens, static, in the planner's cached head).
The help lookup then serves that page's intro (which lists its sections) and best section,
plus the two best sections from the other pages — never the picked page alone.
**Context:** sections were ranked only by bm25 over words shared with the question.
"que funcionalidades de ia tiene emailops" matched nothing specific ("funcionalidades"
appears in no guide; "de"/"ia" are too short for FTS) and got two "local AI" sections;
"cómo cambio el modelo" landed on troubleshooting. The planner reads the question in any
language. It still picks the wrong page now and then ("añado una nueva cuenta" →
installation), which is why the global ranking keeps two slots: the right section there was
its second hit. Cost: up to four guide sections (~1.8k tokens) in the user message of an
app-help turn, and a planner prompt ~370 tokens longer. Known open cost: with the table of
contents in the prompt the planner drops `query` on "when did Marisol first write to me
about the logistics dashboard?" (`no_date_window_on_a_dateless_question` in the planner
eval).
**Rejected:** ordering sections by vector similarity when bm25 is weak (measured: it served
"Privacy › Local AI by default" and "Turning it all off" — the embeddings misrank sections
of these guides); restricting the lookup to the picked page (lost "add an account" when the
planner picked the wrong page); page titles and descriptions only (did not tell the pages
apart); the table of contents at the end of the prompt (26/30 on the planner eval, worse).

## 2026-09-22 — The planner's verdict, not a similarity threshold, keeps the guides out of mail turns

**Decision:** When the query planner ran on a turn, the EmailOps guides are consulted only if
its verdict was `app_help`; a search or defer verdict means no help lookup at all. Turns about
the open email never consult the guides. The vector-similarity gate inside the lookup stays
only for turns the planner never saw (keyword-routed RAG turns, planner disabled). This
narrows the 2026-09-21 entry's "the help lookup still runs on every route".
**Context:** the 0.60 gate let two guide sections into almost every turn — "mándame la
factura de Fly.io de marzo" (0.62), "los últimos 5 correos de metrics@…" (0.74), and a
summary of an open email whose text happened to ask how to add an account (0.66). The
planner already classifies every such turn, and the chat panel promises that answers with an
open email are grounded in that thread only. The planner prompt also gained a general rule
that a specific named thing the mail is about (a project, product, document) is `query`,
not a tag, which closes the `no_date_window_on_a_dateless_question` cost recorded in the
previous entry (planner eval 29/30).
**Rejected:** raising the similarity threshold (a threshold cannot tell "how do I connect
Ollama?" from "what did users say about Ollama?", as the 2026-09-21 entry found); keeping the guides
on open-email turns behind the gate (it served them on the summary above). Accepted cost: an
app question asked with an email open as context gets no guides; removing the thread from
the context restores them.

## 2026-09-22 — Chat may answer general knowledge and pick one of two same-named senders

**Decision:** An out-of-scope general-knowledge question ("what is the capital of Peru?") may
be answered directly or declined; both pass. "resume el correo de Juan" with two senders
called Juan may ask which one or summarise one of them faithfully; both pass. What still
fails is searching the mailbox for a general question, inventing an email, or creating a
draft. The chat system prompt is unchanged.
**Context:** with the judge now able to fail a case, `oos_capital_peru` and
`oos_juan_ambiguous` failed on both the 4B and the 35B reference model: the goldens asked
for a refusal and a clarifying question, and both models answered instead. The answers were
correct and faithful.
**Rejected:** changing the chat system prompt to decline general knowledge and ask for
clarification on ambiguous names (moves replies on every route for behaviour the developer
does not need).

## 2026-09-22 — Published docs are verified against the app, sentence by sentence

**Decision:** Every block of `docs/site/` is a catalogued claim (`<!-- claim:id -->` +
`docs/site/claims.toml`), and `make docs-check ARGS=--with-app` verifies it against the
running app — a fresh install, a locked relaunch, the demo mailbox and the CLI — with
tests, release assets and source files only where the app cannot show it. The report is
the docs themselves, each sentence green (a deterministic check that *quotes* it passed,
reading its expected value from the docs and testing behaviour), yellow (not validatable,
only a label seen, partial, or not yet covered) or red, tagged with how it was checked.
Reference tables are generated from code (`make docs-gen`), not hand-maintained. What no
check reaches is judged by the agent running the `maintain-docs` skill against the
evidence the app run collected, shown as `[AGT]`. The `release` skill calls
`maintain-docs` before tagging.
**Context:** Case-level "OK" hid three failures: expectations typed into the check
instead of read from the docs, a label on screen taken as proof of behaviour, and one
passing check marking a whole paragraph verified. Each hid a real drift (the model
recommendation on a 16 GB Mac, Junk settings unreachable without AI, a disk figure that
matched no model, models the docs said were greyed out and were not).
**Rejected:** Checking text against source code (a locale string can exist and never be
rendered); one pass/fail per block; incremental runs of only the changed sections (too
complex for the gain — the whole check always runs); a local vision model as judge (the
embedded runtime has no image support and the useful models do not fit in 16 GB).

## 2026-09-23 — The chat fills app forms; the planner routes to them, the user saves them

**Decision:** A request to create something the app has a form for ("crea una lens
para seguir las facturas de mis proveedores") is recognised by the chat query planner,
which answers `{"form": "<id>"}` — a fourth verdict alongside `search` / `defer` /
`app_help`. That turn short-circuits: no retrieval, no tool loop, one focused
completion fills the form's fields, and the frontend opens the real form with the
values in it for the user to review and save. Forms are declared once in
`services::forms::registry` (`FormDef` + typed `FieldDef`s whose descriptions are
written for the model); Create Lens is the pilot. The filler never writes anything —
a fill is a proposal, and only the form's own Save button creates a lens.
**Context:** the same request previously landed on `app_help` and was answered with a
tutorial telling the user to do by hand what they had just asked for. Running the fill
as its own one-shot completion (the `plan_search` shape, scratch sequence,
`cache_prompt=false`) keeps the ~700 tokens of field definitions out of the chat system
prompt entirely: the only per-turn cost is the forms catalog, one `id: summary` line per
form (<600 chars, asserted) in the planner's cached head. Measured on
`qwen3.5-4b-q4_k_m`: 7/7 `form_fill_eval` cases pass, ~9.4s per fill, including typed
columns (`currency`/`date`/`enum`), restraint (no invented scope filters) and editing a
form already on screen.
**Rejected:** *a `fill_form` chat tool* — `is_available` gates per install, not per turn,
so its schema would tax every unrelated turn's prompt budget; *a dedicated system prompt
selected by the route* — `ai/llama_cpp/actor.rs` names a route flip as an explicit
`ColdPrefill` cause, so two system texts would wipe the KV anchor on every switch within
a conversation; *applying the values directly* — settings and lenses are the user's to
create, and a model that mis-scoped a lens would have created it before they saw it.

## 2026-09-23 — The chat is told what is on screen; a form on screen is a hint, never a gate

**Decision:** Every turn from the chat panel carries a validated view token —
`view/<mode>`, `settings/<tab>`, or `form/<form id>` plus that form's current values —
rendered as one line in the FINAL USER MESSAGE. An open form additionally reaches the
query planner as a per-call hint, so "añade una columna para el IVA" is recognised as a
form request at all. But the planner still decides *whether* a turn is a form turn; the
open form only decides *which* form (`view_context::resolve_target_form`).
The Create Lens dialog opens non-blocking (`Modal`'s `nonBlocking`, clearing the dock via
a `--chat-dock-width` CSS variable) so the chat stays visible and usable while the user
reviews what the model wrote.
**Context:** the panel is docked beside the app, so "esto", "aquí" and "añade una
columna" refer to whatever is on screen; without it those turns were unanswerable. The
gate distinction is the 2026-09-14 routing lesson applied to a new signal: with a form
up, "qué correos tengo hoy" must still be an ordinary mailbox turn. A blocking modal
would have covered the panel and swallowed every click, which defeats the feature.
**Rejected:** *putting the view line in the system prompt* — it changes on every
navigation, so the cached anchor would cold-prefill every turn; *letting an open form
force a fill turn* — pinned by a unit test
(`an_open_form_never_turns_an_ordinary_question_into_a_fill`); *threading the dock width
as a prop* — the dialogs that need it mount in unrelated subtrees, and it would couple
every form to the chat panel.

## 2026-09-23 — A wrong answer is corrected by a new turn, not by replacing it

**Decision:** Every finished assistant answer carries a "this answer isn't right"
control. It asks what was wrong, then runs an **ordinary new turn** — same route, same
tools, same retrieval — with one `CORRECTION:` block prepended to the user message,
naming the objection and quoting the rejected answer. The rejected answer stays in the
conversation, marked. Nothing is persisted beyond the conversation.
**Context:** a one-click thumbs-down tells the model nothing it can act on; the whole
value is in the specific objection ("esos correos son de septiembre, no de agosto"). The
wrong answer is deliberately kept: the model reads it in history, which is what makes the
objection actionable, and deleting it would destroy the evidence of what went wrong. A
correction is not a new mode because what was wrong is usually the answer, not the path
to it.
**Rejected:** *regenerating in place* — loses the evidence and the history the correction
refers to; *persisting rejections to a table for later eval graduation* — the developer
declined it as scope for now, so the reason steers the retry and is then discarded;
*a correction-specific route or prompt* — the answer was wrong, not the route.

## 2026-09-23 — Chat research mode is a per-message map-reduce on the auxiliary slot

**Decision:** The chat gets a **Research** toggle next to the category filter. It arms
research mode for the **next message only** (the send disarms it). A research turn skips
the heuristic shortcuts and the tool loop. It gathers candidates by paging the query
planner's `search_emails` filter, adding wide hybrid retrieval when the plan names no
structural filter. It then reads them in batches with one `complete_with_prefix` call per
batch (`chat.research_map`), keeping only the findings that cite an email of the batch,
and writes the report in one more call (`chat.research_reduce`). Batch size, the number
of batches and the email cap (≤100) come from the context window, so every batch fits
the map window and all of the notes fit one reduce window. The report is shipped like a
direct answer: citation cleanup, sources and trace. Bare `(email://ID)` and
`[email://ID]` references are relinked to `[subject](email://ID)`.
**Context:** a normal turn answers from about 8 sources or one 25-row page. That suits
"what did X say?" but is too thin for "what themes came up this quarter?". Measured
before and after on real-world mail (qwen3.5-4b-q8_0, n_ctx 15360), the same
question went from 25 rows read in 53 s to 100 emails read in 10 batches in 213 s, with
35 emails cited. Every research call runs on the one-shot prefix slot (2026-09-19
entry), so the chat's KV anchor survives for the next ordinary turn, and the map
instructions stay decoded across batches.
**Rejected:**
- *Only raising the existing limits* (more sources, larger pages, more tool rounds):
  with a 7k-token system prefix in an 8–32k window the prompt front-truncates, and a 4B
  model loses facts in a long context.
- *An agentic loop that plans its own sub-queries*: the local 4B model plans long
  investigations unreliably.
- *A persistent or per-conversation toggle*: it leaves the chat slow by accident.
- *Streaming the report through `chat_stream`*: its system prompt would replace the chat
  anchor, and the 7k chat prefix leaves no room for the notes at 8k. The report is
  therefore not streamed; progress events cover the wait.

## 2026-09-24 — Research mode has no email cap: estimate, confirm, stop

**Decision:** A research turn reads **every** email its question covers. The planner's
filter is run in full, straight against the DB (every matching thread, expanded to its
messages). A topic question with no filter gathers every email within a similarity band of
the best vector hit, plus every keyword hit. Before anything is sent, the user sees an
estimate: how many emails, how many batches and roughly how long, timed from this machine's
last run. They confirm it or cancel it. The confirmed run reads exactly the set the
estimate counted. While it reads, **Stop** ends the reading and the report is written from
what has been read. Notes that outgrow one report prompt are merged in rounds by a condense
step (`chat.research_condense`) before the report. The 50,000-email gather limit is a
safety net, not a product limit. This supersedes the ≤100-email cap in the 2026-09-23
entry.
**Context:** the developer asked for no limit. A cap of 100 also hid a real miss: paging
through the `search_emails` tool read 25 of 33 contact requests, because of the tool's page
and tag rules and its 500-row offset clamp. Unbounded reading costs about 1.5–2 s per email
on the embedded 4B model — about 30 min for 1,000 emails — so the count and time are shown
before the run starts, and the run can be cut short. Every call still runs on the
one-shot prefix slot.
**Rejected:**
- *A fixed cap (100, or one derived from the window)*: it silently drops part of what the
  user asked to have read.
- *Running without an estimate*: a vague question could start hours of work unannounced.
- *Cancel as abort*: after twenty minutes of reading, the partial report is worth more than
  nothing.
- *A fixed top-k for topic questions*: it cuts a large topic short and pads a small one
  with noise.

## 2026-09-24 — Research lists and counts are built in code; the status bar shows long AI work

**Decision:** The matches a research run finds — every email the reading step cites, each
with its first finding — are collected in code from the map notes, before any condense
round. The report prompt receives the exact counts (emails and conversations) as facts it
must state, not something it counts itself. When the question asks for a list or a count
("list", "todas", "how many", "cuántos", "combien", "wie viele"…), the report ends with the
complete numbered list of matches, rendered in code, with no size cap.
Research sizes its batches to the window the loaded model actually runs with — the
`AIProvider::context_window()` the llama.cpp actor publishes after clamping the setting to
RAM, KV fit and the trained window — and falls back to the setting before the model loads.
Closing the window or quitting (Cmd+Q) while research reads is held by the backend, and
the frontend asks the user to confirm.
The log bar shows the long-running AI work in progress: research first, with its step,
then lens, memory and task backfills, classification, Embeddings, junk scoring and model
downloads. What is running is polled from the task-queue snapshot; the numbers come from
each process's own status call or progress event.
**Context:** a model-written report stops at its output budget (a few dozen lines), and a
small model's count of its own notes is a guess. The status bar had no view of background
AI work at all; the queue snapshot was only visible on the Dashboard.
**Rejected:**
- *Raising the report's `max_tokens`*: the report would still be bounded, and the count
  would still be guessed.
- *Having the model write the list in chunks*: slower, and just as lossy.
- *A new unified progress event for every job*: it would touch every job module. Their
  existing status APIs and events already carry the numbers.

## 2026-09-24 — A research run is cancelled, not stopped early; a rejected research is re-researched

**Decision:** The running research's only control is **Cancel**. The run ends at the next
batch boundary with no condense and no report. Its answer states how far the reading got
("cancelled after reading 30 of 100 emails"). This replaces "Stop and write report" from
the entry above. Marking a research answer wrong retries it **as a research**: the new run
asks the original question plus the user's correction, in every step, and is estimated and
confirmed first like any research. A research answer's sources — and the chat's "show in
list" button — are every email the reading found relevant, not only the ones its prose
links. Coming back to an account (chat account picker or the app's account switch) reopens
the conversation where a turn is still running.
**Context:** the developer found "stop and write report" unclear and asked for a plain
cancel.
**Rejected:**
- *Keeping both buttons*: the developer chose one.
- *Retrying a rejected research as an ordinary turn*: it would answer from one page of
  results, which is the thing the user rejected.

## 2026-09-24 — One shared thread reader; each email is read for what it adds

**Decision:** Every feature that reads a conversation goes through
`services/thread_reader.rs`. It covers the chat's thread context, the `get_thread` tool,
draft generation, research (which now reads, counts and lists **one match per
conversation**), `search_emails` bodies, and the memory, task and lens extractors. A
message is read for its *new content*: the cleaned body minus quoted history (attribution
lines, `>` lines, HTML `<blockquote>`) and minus any paragraph an earlier message in the
thread already said. So a reply never re-sends the emails it answers. One water-fill
budget, with an optional focus message and oldest-first drops, replaces the per-feature
allocators.
**Context:** each feature cleaned threads its own way. A reply that pasted its
predecessor without a quote marker was read, and extracted from, twice. Research
listed one thread's replies as separate matches. The developer asked for one reusable
reader and for no conversation to appear twice in an answer.
**Rejected:**
- *Moving the chat's RAG sources to new content only*: a retrieved reply is often the
  only hit for its thread, and its quoted history is the context the answer needs. RAG
  keeps the whole cleaned body.
- *Deduplicating research matches after the map step*: the model would still read every
  reply with its history, and the count would stay per email.

## 2026-09-24 — Research knows who the user is; a finding says whether it answers

**Decision:** Research decides what counts from facts code already has. It does not
leave the model to guess them.
- **Roles:** every message the reading step sees is rendered with its role decided in
  code: `From: YOU (the user)` or `To: YOU`.
- **The user's addresses:** "the user" is every address they send from: the account's
  own plus each sender of the account's Sent mail. A provider's Sent folder holds only
  the owner's messages, so send-as aliases show up there with no configuration. A
  filter the planner puts on the user's address is run once per address, so mail sent
  from an alias is gathered too.
- **Direction:** the planner's filter sets the question's direction: a sender filter on
  the user means *sent*, a recipient filter on the user means *received*. The direction
  is passed to the map and report steps.
- **Direction check:** a conversation counts for a *sent* question only if a cited
  email is the user's own, and the mirror for *received*.
- **Finding tags:** each finding is tagged `MATCH` (the email answers the question) or
  `CONTEXT` (related background). Only `MATCH` findings make the list, the count and
  the sources. An untagged line counts as a match, so a user-edited prompt keeps
  working.
- **Report limit:** it is sized from the window: a quarter of it is reserved (1,536 to
  4,096 tokens), and the actual limit is what the real prompt leaves free, capped at
  4,096.

**Context:** "which quotes have I sent to clients?" counted quotes the user had received
(insurance, sworn translations) and listed the user as a client. The prompt never said
which participant was the user. Code also counted any cited conversation as a match,
so a correctly written "Requested a quote from X" still became a quote sent. The report
limit was a fixed 1,536 tokens on a 15k window, and long reports were cut mid-line.
**Rejected:**
- *Only rewording the prompt ("be careful about direction")*: the model still could not
  tell who the user was.
- *A code-only direction filter*: the user writes in a quote-request thread too, so
  "sent a quote" versus "asked for a quote" needs the model's judgement. The tag makes
  that judgement explicit, and code enforces it.
- *An uncapped report*: a 4B model repeats itself past a few thousand tokens, and
  every token costs time.

## 2026-09-24 — Research steps talk JSON under an enforced shape; the report cites by number

**Decision:**
- **Reading step:** it returns one JSON verdict per conversation,
  `{conversation, text, tag, emails}`. It names conversations and emails by short batch
  labels (`C1`, `E3`), never by id. The shape is *enforced*:
  - a GBNF grammar on llama.cpp (`ai::json_shape` renders it), which also confines the
    labels to the batch's own;
  - JSON Schema `format` on Ollama;
  - strict `response_format` on OpenRouter.

  Entries and their text are bounded, so the reply always fits the step's output budget.
- **Condense step:** it merges notes by label (`N3`), and code keeps which conversations
  each note covers.
- **Report:** it cites conversations as `[n]` and code writes every link.
- **Providers report stop reasons:** each one says when a reply stopped at its token
  limit (`CompletionResult.truncated`), and a cut reading reply fails its batch loudly
  instead of losing its tail.

**Context:** a recall eval showed research finding 8 of 13 weekly digests. The reading
step's 400-token free-text output was cut mid-batch without anyone noticing, and that
had been happening before the MATCH/CONTEXT tags. Free text also needed five Markdown
patches (bare `email://` refs, id-labelled links, repeated links and bullets) and a tag
parser, each covering one way a small model had failed. Ollama never reported token
counts, so cut detection by counting tokens never worked there.

Enforcing the grammar exposed a latent bug. The actor accepted each sampled token twice
(`llama_sampler_sample` already accepts), which is harmless for temperature and
distribution samplers but corrupts a grammar and makes llama.cpp throw. The duplicate
accept is gone.

**Rejected:**
- *Raising the free-text output budget*: still unbounded, and still cut without notice
  on a large batch.
- *Enabling llama-cpp-2's `common` feature for `json_schema_to_grammar`*: it changes the
  native build and the Linux/Windows packaging, which has no CI. A small in-house
  shape→GBNF renderer covers what research needs.
- *Asking the model to copy email ids*: 16-character ids are easy to mangle, and a
  grammar over labels makes an invented citation impossible.

## 2026-09-24 — Research answers lists and counts in code; only analysis gets a report

**Decision:** Before reading, a small classifier (`chat.research_mode`, with its reply
forced to `list` | `count` | `analysis`) decides how the answer is delivered.
- **List and count:** code writes the answer from the reading step's per-conversation
  verdicts: the exact counts, every match linked with its verdict, and how much was read.
  There are no condense or report calls.
- **Analysis:** only analysis (a trend, a summary, a comparison, a total) runs condense
  and the report.
- **When unsure:** the classifier answers `analysis`, which still serves a list, just
  more slowly.
- **The user sees the form first:** the confirmation card shows it before the run
  starts, so a wrong guess can be cancelled.

This replaces the `LIST_CUES` keyword list.
**Context:** a list question paid for the whole report step — about 100 s on a
450-email production run — only to have the code-built list appended to it. The keyword
list also missed plain list questions ("qué presupuestos he enviado"). The per-conversation
verdicts already hold everything a list needs, and they have already been held to who
wrote what.
**Rejected:**
- *A field on the shared `chat.query_plan` planner*: no extra call, but it would move
  every normal chat turn and that prompt's eval.
- *Keeping keyword cues*: fragile across phrasing and languages.
- *Letting the report decide*: the report is the cost being avoided.

## 2026-09-25 — "Emails with X" is its own filter, in both directions

**Decision:** Search has a `with` filter (the planner field, the `search_emails` argument
and the research gather). It means "exchanged with X": X is the sender, or X is among
the recipients or cc. A name is resolved in code to the addresses X writes from (the
account's most frequent senders matching the name), so mail the user sent to those
addresses is found even when it doesn't carry X's name. The planner is told: "with X" /
"con X" with no direction → `with = X`, not `from`/`to`.
**Context:** "resume todos los correos con Genoveva" was planned as
`from: genoveva, to: me`. On real-world mail that missed the 40 threads the user started
(about 40% of the conversations). `from` and `to` are AND-ed, so no plan could express
"either way", and a `to` on a name misses mail addressed to a bare address.
**Rejected:**
- *Running `from: X` and `to: X` separately and merging*: two limits and two orders,
  and `to` still misses bare addresses.
- *Keyword cues for "con"/"with"*: the planner reads the question anyway.

Planner eval: 36/39 → 37/39. Both new "with X" cases pass. One case flipped: "¿cómo creo
una lens?" now opens the create-lens form instead of the guide answer.

## 2026-09-25 — Any chat turn can be cancelled; it keeps what was shown

**Decision:** Every running chat turn shows Cancel, not only research. A turn registers a
flag under its assistant message id (`chat::cancel`).
- **Tool loop:** Cancel raises the flag. The tool loop checks it before each tool and
  each round, and the token callback returns `false`, which stops generation mid-reply
  on llama.cpp and Ollama.
- **The saved answer:** a cancelled turn makes no further model call (no synthesis, no
  guard retries). It keeps the text the user already saw, followed by a
  "Cancelled by the user" note in the reply language.
- **Research:** a cancelled research still stops at its next batch and writes its own
  note.

**Rejected:**
- *Discarding the partial answer*: the user saw it, and dropping it looks like data
  loss.
- *A separate research-only control*: one Cancel for every turn is simpler to find.

## 2026-09-25 — Email text reaches the AI minus only what its thread already contains

**Decision:** Quote, forward and signature markers ("On … wrote:", Outlook
`From:/Sent:` headers, "Original/Forwarded message", `>` lines, `<blockquote>`, the
`-- ` delimiter) only split a body into blocks (`thread_clean::segment`). A quoted or
signature block is dropped only when ~80% of its 5-word runs appear in an earlier
message of the same thread (`thread_clean::History`), and an own paragraph only when it
repeats one. "Sent from my iPhone" stubs always go. Everything else reaches the chat,
drafts, Lenses, Tasks and Memory. Draft style samples keep only the user's own text
(`own_text`).
**Context:** Cutting at the first marker lost the substance of emails whose quote was
the only copy: forwards with a note, Apple Mail forwards in a `<blockquote>`, replies
to mail that was never synced, contact-form notifications that open with `From:` /
`Subject:` or end in a `--` footer. Each new marker rule added another way to lose
text. Comparing with the thread fails safe: when it errs, the model reads more text,
never less. On a 50-email sample of real-world mail the AI now reads 66% of the text
instead of 31%.
**Rejected:**
- *More marker rules, forward detection by subject prefix* (Mailgun `talon` does this
  for a single message): still loses text whenever a rule misfires, and a thread-less
  view is not what the features read.
- *Cleaning only one-message threads differently*: a special case of comparing with
  the thread, which covers it.

## 2026-09-25 — Windows CUDA build targets desktop GPUs

**Decision:** The Windows CUDA release asset compiles native code for RTX 30/40/50
(`86-real;89-real;120a-real`), plus Turing PTX (`75-virtual`) that the driver
JIT-compiles on every other GPU. The list is set with `CMAKE_CUDA_ARCHITECTURES` in
`release.yml`.
**Context:** That job set the release's length. Measured on the 25/09/2026 run: 111 min
of build, ~95 of them in nvcc, because ggml-cuda's default list under CUDA 13 has 7
targets. The other 3 are datacenter parts (A100, H100, GB10) that a desktop mail client
is unlikely to run on.
**Rejected:**
- *Keeping all 7 targets*: it makes every release slower for GPUs this app is unlikely
  to run on.
- *PTX only (`75-virtual;89-real`)*: RTX 30/50 owners would pay the JIT on first
  launch and could lose throughput, and those are the users the asset is for.
- *Publishing without the CUDA asset and attaching it later*: it changes when a release
  goes out, not how long the build takes.

## 2026-09-25 — Suggested attachment rules are mined heuristically and always confirmed

**Decision:** EmailOps proposes candidate attachment rules from recurring document
attachments (PDF, office, XML/ZIP e-invoices — never images, `.ics` or signatures):
at least 2 emails in 2 different calendar months from one sender (many providers
send one invoice a month; senders of one corporate domain pooled, personal providers
never — those are named after the sender's display name), with a filename glob generalised
from numbers, dates and month names. Mining is a deterministic heuristic, re-run after
every sync that brought new mail and when the rules modal opens; a badge on the
sidebar's Attachments entry and the "Manage Rules" button shows the pending count.
A candidate is never turned into a rule automatically: "Review" opens the regular rule
form prefilled (apply-to-existing on), and only saving it accepts the suggestion.
Accepted and dismissed candidates are remembered by key and never proposed again.
**Context:** Users had to hand-write a rule per invoice sender; the recurring-document
pattern is visible in `email_attachment_meta` without reading bodies.
**Rejected:** LLM-based detection (slower, non-deterministic, unnecessary for a
sender × filename × cadence pattern); one-click creation without review (a wrong
glob silently downloads the wrong files); computing only when the modal opens (the
user would never discover the feature without a proactive signal).

## 2026-09-25 — Windows Vulkan builds without --jobs 1

**Decision:** `scripts/build_platform.sh` no longer forces `--jobs 1` on Windows Vulkan
builds. The C1041 PDB race it worked around is covered by three later fixes: the Ninja
generator, `CL=/FS`, and the short `CARGO_TARGET_DIR` (C:/ct). Confirmed by a
`windows-vulkan` dry run (run 36119358082): no C1041, the smoke test passed, pass 1 took
22m57s (was 43m53s) and the job 35 min (was 54).
**Context:** `--jobs 1` serialized every Rust crate, not just the CMake build. On the
25/09/2026 release run, pass 1 took 43m53s on Windows against 17m05s on Linux. After the
CUDA job was trimmed, the 54-minute Windows Vulkan leg became the next-longest part of
the release. The commit that added `--jobs 1` was made before any of the three later
fixes, and the short-target-dir fix showed C1041 also fired on a single, uncontended
compile, from path length alone.
**Rejected:**
- *Serializing only llama-cpp-sys-2* (a `cargo build -p llama-cpp-sys-2 --jobs 1`
  first pass): cargo refuses `--features` for a package outside the workspace. Without
  them the crate would build with different features and be rebuilt in pass 2.
- *Avoiding pass 2's recompile of `emailops`* (~3-5 min per leg): the merged Tauri
  config, which includes backends staged after pass 1, changes the app's build script
  input. Fixing it means reworking packaging around `tauri build --no-bundle` +
  `tauri bundle`, on a path with no per-PR CI coverage.

## 2026-09-27 — Task-queue submits box the task first; Windows reserves an 8 MiB main stack

**Decision:** `TaskQueue::submit*` are plain `fn`s that box the task before anything is
awaited, and never `async fn`s taking the task by value. The Windows app links with
`/STACK:8388608` (`build.rs`), the main-thread stack size macOS and Linux already give it.
Guarded by `#![deny(clippy::large_futures)]` (16 KiB, `clippy.toml`), a CI Clippy step
with `--features desktop` (the only one that compiles `commands/`), and a test that holds
every registered Tauri command's future to the same 16 KiB. The command list lives once, in
`app_commands!`, so the test sees every command `generate_handler!` does.
**Context:** v0.6.10 crashed on Windows with STATUS_STACK_OVERFLOW (0xc00000fd) once a
sync started. Tauri builds and spawns each async command's future on the main thread,
and copies it through `respond_async_serialized` → `async_runtime::spawn` →
`tokio::spawn`. Making `submit_named` an `async fn` that awaited `submit_with_priority`
put every queued task inline in the caller twice. That doubled the futures of the sync
and send commands: the spawn frames for `start_sync_account` went from ~486 KB (v0.6.9)
to ~939 KB, over Windows' 1 MB default and harmless under macOS' 8 MiB. With the task
boxed first, the largest spawn frame is ~75 KB.
**Rejected:**
- *Only raising the stack*: it hides the doubling, and every queued task still costs its
  full size on the stack several times over.
- *Only boxing the task*: the next command whose future grows would again crash on
  Windows alone. No CI leg runs the app on Windows.
- *Running a custom Tauri async runtime with larger worker stacks*: the frames that
  overflowed were on the main thread, which that setting does not reach.
- *An IPC test through `tauri::test`'s mock runtime on a 1 MB thread*: commands take
  `AppHandle`, which is `AppHandle<Wry>`, so they cannot be registered on the mock
  runtime, and a real Wry app needs a display and the process main thread. The future-size
  budget measures the same thing from the types alone, on every OS.

## 2026-09-28 — A search window's `until` includes its own day

**Decision:** Every date window the AI reads or writes — `search_emails`,
`list_calendar_events`, the query planner, research, the today/week shortcuts — treats
`until` as the last day included: one day is `since == until`, a week ends on its
Sunday, "last 6 months" ends today. The conversion to a timestamp happens in one place,
`parse_until_date_secs` (start of the next local day). "Today", "yesterday" and the
calendar weeks are injected into the planner prompt as computed dates, so the model
never does that arithmetic.
**Context:** `until` was end-exclusive, and the prompts said so, but the model kept
writing inclusive ends ("últimos 6 meses" → `until = today`, "en 2025" →
`until = 2025-12-31`), so the last day's mail — today's report, in the case that
surfaced it — silently dropped out. An inclusive end is what a model and a user both
assume, and the worst case of a model still writing a half-open end is one extra day
of mail rather than a missing one.
**Rejected:**
- *Ignoring `until` when it equals today*: `until = today` is also the correct
  half-open end for "yesterday" and for "last week" asked on a Monday, so the rule
  needed exceptions, and it overrode what the planner asked for.
- *A prompt rule "a period up to now has no until"*: it fixed those questions but moved
  unrelated plans on the 4B model (a recipient flipped to sender; a document keyword
  replaced by a tag), measured on the planner and chat evals.

## 2026-09-29 — Attachment rule suggestions: dismissals match by sender identity, not key

**Decision:** A resolved (accepted or dismissed) suggestion hides every later candidate
from the same sender identity (`*@domain` for a company, the address for a person on a
personal provider) whose documents its filename pattern mostly matches — a key equality
check is no longer the test. Candidate keys are `identity|filename pattern`. Mining also
covers filed folders (everything but sent, spam and trash), skips the user's own address
and — on a corporate account — their own domain, needs the first and last email ≥20 days
apart, and falls back to the most recurring extension (`*.pdf`) or to nothing, never to a
pattern-less rule. IMAP accounts join the attachment backfill: with no "has attachment"
search key, each stored folder is searched for `Content-Type` `multipart/mixed` or
`application/*`, and a backfill with failed fetches is not marked done.
**Context:** The key embedded the proposed patterns, which drift as mail arrives
(`billing@acme.com` → `*@acme.com` when a second address sends; `*.pdf` → `Invoice_*.pdf`
once a family recurs), so dismissed suggestions came back. A pattern-less fallback rule
collected logos and invites; colleagues' shared PDFs became suggestions named after the
user's own company.
**Rejected:** A migration re-keying resolved rows (the stored patterns already carry the
identity, so coverage is computed from them); matching the resolved row's exact sender
pattern (a dismissal of `billing@` must also hide `noreply@` of the same company).

## 2026-09-29 — Suggested attachment rules tag the document kind in the UI language

**Decision:** The kind tag a suggestion proposes (and the name built from it,
"Acme · factura") is written in the UI language — the `ui_language` preference, else the
OS locale, else English — while the keywords that detect the kind stay multilingual.
Pending suggestions are re-mined on every sync and modal open, so they follow a language
change.
**Context:** English-only tags ("invoice") next to the user's own Spanish tags
("factura") split one kind of document across two tags.
**Rejected:** Canonical English tags (they do not match the vocabulary the user types);
translating at display time (tags are user data stored on rules and attachments, not UI
strings).

## 2026-09-29 — Attachment rules reach the inbox, Sent and filed folders, not Spam or Trash

**Decision:** Attachment rules collect mail in the inbox, Sent and the user's own folders
(IMAP custom folders), both when new mail syncs and when a rule is applied to existing
mail; Spam, Trash and locally deleted mail are never collected.
**Context:** Only the inbox pass applied rules at sync time, so an invoice an IMAP server
filter moved into a folder never reached the attachments view unless the rule was
re-applied by hand — while that manual apply collected from everywhere, Spam and Trash
included.
**Rejected:** Inbox only (misses server-side filing); every mailbox (a sender rule would
pick up the junked or deleted copy of a message); leaving Sent out (the user wants the
invoices they send collected too).

## 2026-09-29 — OpenRouter never routes to providers that store or train on prompts

**Decision:** Every OpenRouter request that can carry mail content — chat completions and
embeddings — sends `provider.data_collection = "deny"`, fixed and not user-configurable.
Zero data retention (`provider.zdr = true`) is a user toggle in the OpenRouter panel, off
by default. A model with no provider meeting the policy fails with `AppError::AiDataPolicy`
naming the model; the request is never retried under a looser policy.
**Context:** Google's Workspace API user-data policy forbids transferring Gmail data "to
create, train, or improve a machine learning or artificial intelligence model beyond that
specific user's personalized model", and user consent does not lift that rule. OpenRouter
defaults to `data_collection: "allow"`, and a probe on 29/09/2026 showed free models
answering through training endpoints. `deny` cost 6 of 341 working models, all free tiers;
adding `zdr` cost 83 in total (including the Qwen API family, OpenAI `o3`/`o4-mini`,
Cohere), too many to impose.
**Rejected:**
- *`data_collection` as a user setting*: a user could switch Gmail data into training,
  which is what the policy prohibits; the restricted-scope verification needs a flat "no".
- *ZDR on by default*: cuts roughly a quarter of the usable models for a guarantee the
  policy does not require.
- *Silently switching or retrying a blocked model*: sends mail somewhere the user did not
  choose; a clear error pointing to Settings is better.

## 2026-09-27 — Chat skills are Agent Skills folders on disk, loaded on demand

**Decision:** The chat supports user *skills* in the Agent Skills shape, modelled on
Hermes Agent: a folder per skill under `<data dir>/skills/` holding a `SKILL.md` (YAML
frontmatter with `name` and `description`, then Markdown instructions) and optional
`.md`/`.txt` reference files. Progressive disclosure in three levels: a skills index (the
one-line catalog plus "load a matching skill FIRST") in the system prompt, the body when
the model calls `load_skill(name)`, a reference file when it calls
`load_skill(name, file)`. `/name` at the start of a message applies a skill directly
(stackable: `/a /b request`), riding in the final user message. Skills are instructions
only — nothing in a skill folder is executed.
Settings → AI Skills lists them, shows why one failed to load, opens the folder and holds
the `skills_enabled` toggle (default on). `emailops-cli skills` lists them too.
**Context:** The developer asked for skills "like Anthropic's" and chose a folder on disk
over an in-app editor, so skills can be written in any editor and shared as files. The
chat runs on an 8192-token window on the smallest supported machine and relies on a
byte-stable system prefix for the llama.cpp KV cache, so bodies cannot sit in the system
prompt, and per-turn content must stay out of it.
**Rejected:**
- *Skills stored in SQLite and edited in Settings*: the developer preferred files.
- *Putting every skill body in the system prompt*: eats the context window and grows
  with every skill.
- *Slash commands only (the model never picks a skill)*: loses the main value — the chat
  applying the right procedure without being told.
- *Running scripts bundled in a skill*: a local email client executing arbitrary code
  from a folder is a security surface this feature does not need.

## 2026-09-29 — Skills are experimental and off by default; the planner can pick one; they are edited in a Skills view

**Decision:** Chat skills ship as an **experimental** feature, **off by default**
(`skills_enabled` defaults to false; the Settings tab carries the Experimental badge).
When on, a **Skills view** in the sidebar lists every skill with its own on/off switch
(`skills_disabled` preference, a JSON array of names — a disabled skill leaves the prompt
but stays listed) and edits the selected `SKILL.md` in an editor on the right; **New**
creates one from a template. Saving validates the text with the same parser the catalog
uses and writes nothing if it would not load. The files on disk remain the source of truth.
The **query planner** may also name a skill (`"skill": "<name>"`) next to any verdict,
before retrieval; the turn then applies it exactly like `/name`, and `load_skill` stays as
the fallback. The trace records how each skill arrived (`applied_skills`, `via: slash |
planner`), and eval cases assert on the skill (`expected_skill` / `expected_no_skill`)
rather than on the `load_skill` call.
**Context:** The developer asked for skills to be opt-in and experimental, and for a
Hermes-style view with per-skill switches and an in-app editor, replacing the 27/09
choice of "files only, Settings just lists them". A 15-case skills eval showed the common
miss was selection: when the RAG sources already held the answer, the model answered
without loading the skill, on every model. Letting the planner choose before retrieval,
measured on the demo DB with an LLM judge (qwen3.6-35b): cases passed 4B 9→10,
9B 8→10, 35B 12→12 (one case lost to a GPU out-of-memory during the run); correct skill
on the 6 selection cases 4→5, 2→4, 4→6; zero skills applied where none fits on every
model. With no skills the planner prompt is byte-identical: `query_plan_eval` 35/39
before and after (same four failures) and the chat smoke tier 38/41 in both.
**Rejected:**
- *A fifth planner verdict (`{"skill": …}` instead of search/defer)*: a skill usually
  still needs its search, so the field rides alongside the verdict.
- *Wording the planner rule as "add the skill to whatever you output (a filter, …)"*:
  it pushed the 9B to swap `defer` for invented search filters (4 cases); the rule now
  says a skill never changes the verdict (2 cases, one of them noise).
- *Thinking on round 0 of the tool loop*: not needed once the planner selects; not tried.
- *Keeping skills on by default*: the developer wants them opt-in while experimental.

## 2026-09-29 — Skill turns replay as a one-line note; a saved SKILL.md's name wins

**Decision:** Later turns of a conversation replay a skill turn with a one-line
`[skill X was applied to this request]` note, not the skill's body. Saving a `SKILL.md`
whose `name:` differs from its folder renames the skill to that name (folder and on/off
switch), refusing when the name is taken; a save is also refused when the file changed on
disk since the editor opened it. A bare `/name` asks the skill's description instead of a
made-up "Apply the skill" sentence.
**Context:** A review of the skills feature found that replaying the body cost up to ~2k
tokens per past skill turn (a large share of the 8192-token floor), kept applying an old
procedure to unrelated follow-ups and froze a copy the user may have edited since. Pasting
a skill written elsewhere into a skill created under another name failed to save, and the
synthetic "Apply the skill" question steered routing, retrieval and titles (a bare
`/weekly-digest` was answered from the app guides).
**Rejected:**
- *Replaying the body byte-identically (the 27/09 design)*: better KV-prefix reuse on the
  next turn, but the costs above hit every later turn.
- *A "use the folder's name" fix-up button*: keeps the folder authoritative, but the user
  just pasted the name they want.
- *Caching the catalog per turn*: measured ~1 ms per read with 20 skills (~8 ms per turn
  against 9–14 s turns) — no measured problem, so no cache.

## 2026-09-30 — A draft with unpushed local edits always wins: pushed on sync, re-created if gone upstream

**Decision:** Every composer save marks the draft dirty (`drafts.dirty`, the number of
saves since the last successful push). The draft sync never prunes or overwrites a
dirty draft: it pushes it, and if the provider copy was sent or deleted from another
device it creates the draft again upstream and replaces the stale provider id. When
both sides changed, the local draft wins and the upstream edit is overwritten. Only
clean drafts are pruned or replaced by a pull. The per-draft decision is the pure
planner `sync::draft_plan::plan_draft_sync`.
**Context:** Editing one draft on two devices lost text three ways: a save against a
provider draft that no longer existed failed forever and kept the stale id; the next
sync then deleted the local draft, unpushed edits included; and a draft saved offline
was never pushed and was overwritten by any upstream change. Unsent text the user
typed here exists nowhere else, so losing it is worse than any other outcome.
**Rejected:** *Last-writer-wins by timestamp* — provider and local clocks are not
comparable, and the pull already rewrites `updated_at` with the provider's time.
*Keeping both as two drafts on a conflict* — no data loss at all, but it leaves the
user to work out which copy is current after every offline edit; the upstream edit
that loses is still recoverable on the other device until the push lands.
*Honouring the upstream delete for a dirty draft* — a draft sent from another device
comes back as a draft here, which is visible and one click to discard, whereas a
discarded edit is gone. *Marking every local write dirty* — the chat draft tool's
drafts would start appearing in the provider's Drafts folder unasked; they stay
local until the user saves them in the composer.

## 2026-09-30 — V026 (email FK child indexes) and V028 (draft dirty marker) are release-coupled

**Decision:** V026 indexes the five child columns that reference `emails(id)`; V028
adds `drafts.dirty`. Both ship in the next release; existing drafts start clean (a
draft saved offline before V028 stays local until its next save).
**Context:** A dev build applies pending migrations to whatever database it opens.
Released binaries do not contain V026/V028 and refuse a database that has them, so
running this build against the production data dir blocks the installed release until
a release that ships both is out. V027 and V029 belong to parallel work, so a
database this build opens before that work is merged ends at V028 without V027, and
a later build that contains V027 refuses it (refinery aborts on a migration file
older than the highest applied version). Keep such builds on throwaway data dirs.
**Rejected:** *Backfilling `dirty = 1` on existing local-only drafts* — it cannot tell
an offline save from an AI-generated draft never meant for the provider, and would
push all of them on the first sync after upgrade.

## 2026-09-30 — Read state and delete write back to IMAP and Outlook too

**Decision:** `provider_supports_mailbox_writes` now covers Gmail, IMAP and Outlook. Marking
read pushes `UID STORE ±FLAGS.SILENT (\Seen)` (IMAP) or `PATCH /me/messages/{id}` `isRead`
(Graph); delete moves the message to the account's Trash — the IMAP Trash folder through
the existing `UID MOVE` / `COPY` path, Graph `move` to `deleteditems` — and never expunges
or hard-deletes. The ordering of the 2026-08-15 entry is unchanged: read state local-first,
delete provider-first. Two refinements apply to every provider: a provider answering "no
such message" (`AppError::NotFound`) counts as deleted, so the local delete goes through;
and `trash_message` takes the message's Message-ID, which IMAP checks against the UID before
moving anything.
**Context:** A review at `c1f152f` found these changes stayed local on IMAP and Outlook and
diverged silently from the account. Outlook already requests `Mail.ReadWrite`, so no scope
changes and no account has to re-authenticate. An IMAP id is a UID, which a server-side
mailbox rebuild can hand to another message; without the Message-ID check a delete made
between the rebuild and the next sync would trash the wrong message.
**Rejected:**
- *Local-only delete when the IMAP server has no recognisable Trash folder*: that is the
  silent divergence this removes; the delete is refused with an error instead.
- *`\Deleted` + `EXPUNGE` in place*: permanent, and the app's delete is the reversible one.
- *Treating an IMAP Message-ID mismatch as "already gone"*: it would hide a message that
  still exists upstream; the delete is refused and the next sync repairs the id.

## 2026-09-30 — A failed read-state push is retried by the sync, for up to a week

**Decision:** Marking a message read sets `emails.read_push_pending_since` (V029) in the
same statement as `is_read`, and clears it once the provider has the change. Every sync
starts by pushing what is still pending (`retry_pending_read_pushes`): at most 100 rows,
stopping after 3 failures, and giving up on a change older than 7 days. "No such message"
settles a pending push. This applies to Gmail as well — it shares the service path.
**Context:** The push was best-effort and its failure only logged, and a re-opened message
returns early because it is already read locally — so one offline moment left a message
unread in every other client for good. The marker is written before the push, not after a
failure, so a crash or a concurrent sync never sees a locally read row the provider does
not know about.
**Rejected:**
- *A preference holding the pending ids* (no migration): the marker has to travel with the
  row when it is re-keyed, vanish with it, and be read and written atomically with
  `is_read` by two concurrent tasks; a JSON list in `user_preferences` does none of that.
- *Retrying forever*: a read-only mailbox or a revoked permission would cost a failing
  request per row on every sync.
- *A retry queue for delete*: delete is provider-first, so a failed delete is an error the
  user sees and nothing is left half-done.

## 2026-09-30 — Sync refreshes the state of recent stored IMAP/Outlook mail; pending local changes win

**Decision:** Once per 2 minutes per account, the sync asks the provider for the current
state of the account's stored mail from the last 30 days, newest first, at most 200 rows
(`EmailProvider::fetch_message_states`): IMAP one `UID FETCH (UID FLAGS)` per folder, Graph
one `$batch` of `$select=id,isRead` per 20 ids. Read state follows the server. A message
the provider no longer has under its id is located by Message-ID (`locate_message`) and
re-keyed in place into its new mailbox, or soft-deleted when the provider does not have it
any more — at most 25 such lookups per pass. **Conflict rule: a row with a pending local
push is never touched; for every other row the server wins.** Spam is left to
`reconcile_spam_moves`, Sent mail keeps its read flag, and a row the provider could not
check (its folder would not open, its sub-request was throttled) is left alone.
**Context:** The fetch passes drop every id the database already holds, so a message read,
deleted or filed in another client never changed here; only Spam was reconciled. Gmail is
out of scope for this pass (the developer asked for IMAP and Outlook) and answers `None`.
**Rejected:**
- *Graph delta queries*: exact and cheap in steady state, but they need a delta token per
  folder, its expiry handling and a first full enumeration; asking about the ids already
  stored needs no state and no "was the listing complete?" reasoning.
- *Listing each folder and diffing*: a truncated listing reads as mass deletion — the rule
  the spam reconciliation already has to work around.
- *Refreshing the whole mailbox*: unbounded on the 47k-message accounts this runs against.
- *Last-writer-wins by timestamp*: neither IMAP flags nor `isRead` carry a change time.

## 2026-09-30 — IMAP UIDVALIDITY is recorded per mailbox; a change re-keys stored mail by Message-ID

**Decision:** Every IMAP sync first reads each stored mailbox's `UIDVALIDITY` (`EXAMINE`)
and compares it with the value recorded in `folder_uid_validity` (V027). The first sight
of a mailbox records a baseline. On a change, before anything is listed, the mailbox is
listed as `(UID, Message-ID, INTERNALDATE)` and the stored rows are matched to it — by
Message-ID, copies of one Message-ID in order, and by arrival time for the few messages
without one when that is unambiguous — then re-keyed in one transaction, so read state,
tags, bodies and embeddings survive. Rows nothing matches are hard-deleted together with
the mailbox's failed-download records, and the mailbox's sync windows are reopened so
whatever is still on the server is downloaded again. A failure aborts the sync and leaves
the recorded value, so the next sync retries.
**Context:** IMAP ids are `{account}::{uid}`, and nothing read or stored UIDVALIDITY. After
a server migration, restore or index repair, new mail whose UID matched a stored id was
dropped as "already synced", and stored ids addressed other messages for re-fetch, move and
locate. A rebuild that happened before V027 was applied cannot be detected: the first sync
only records a baseline.
**Rejected:**
- *Drop the mailbox's rows and re-download*: loses classification, embeddings, memory and
  read state for every message, and the per-sync caps make a large inbox take days.
- *Keep unmatched rows under a detached id*: they could never be deleted or moved upstream
  and would duplicate any message the re-sweep brings back.
- *Include UIDVALIDITY in the message id*: the principled fix, but it re-keys every stored
  IMAP row of every install in a migration and changes an id format frozen for backward
  compatibility.
- *Non-fatal on failure*: a sync that goes on with stale ids in place is exactly the bug.
**Known limit:** the inbox reopens through its incremental window, which one IMAP sync lists
only as far back as the newest ~1 000 messages. An unmatched inbox message older than that
and without a usable Message-ID is not re-downloaded automatically.

## 2026-09-30 — V027 and V029 are release-coupled

**Decision:** `V027__folder_uid_validity.sql` (new table) and `V029__read_push_pending.sql`
(new `emails` column + partial index) ship together with the code above. Like every
migration, a development build applies them to whatever database it opens, and the released
binaries up to the current version then refuse that database until a release containing
both is installed. V026 and V028 belong to parallel work and are intentionally absent here.
**Context:** Same coupling as V008–V024 before them; recorded because two version numbers
are skipped on this branch and the merge order matters — `migration_versions_are_unique`
guards against a collision, not against a missing neighbour.
**Rejected:** *Storing both in `user_preferences` to avoid a migration* — see the two entries
above for why each needs real schema.

## 2026-09-30 — OpenRouter supports chat (streaming and tool calls), not only one-shot completions

**Decision:** The OpenRouter provider implements `chat_stream`, `chat_stream_with_tools`
and `chat_with_tools` over `/chat/completions` with `stream: true`, and reports
`tools`/`streaming` as supported, so the chat works on it like on the local backends.
- **Same data policy:** chat requests carry the `provider` preferences of every other
  request (`data_collection: "deny"`, `zdr` when the user asked for it).
- **Budget:** the chat loop's model calls go through `AiService::chat_stream` /
  `chat_stream_with_tools`, and a Lens extraction's tool call through
  `AiService::chat_with_tools`: refused before the call once the period's spend has
  reached the budget, recorded after it with the `usage.cost` the stream reports. A Lens
  refused for budget does not fall back to its text prompt.
- **Failures are shown, not retried:** a 429, a 5xx, a mid-stream `error` event or a
  stream silent for 60 s ends the turn with an error. Part of the reply may already be on
  screen, and a silent retry would bill the prompt twice.
- **Reasoning stays out of the answer:** `reasoning` deltas are never shown or stored, and
  no `reasoning` parameter is sent (the model's default applies).
- **Cancel:** the token callback returning `false` drops the connection, which is how
  OpenRouter stops generating.
**Context:** `chat_stream*` returned "not supported for OpenRouter backend", so every chat
turn failed once OpenRouter was selected — and OpenRouter is what an Intel Mac is pointed
to, since the embedded runtime cannot run there. The developer decided chat must work on
it rather than hide the chat for that provider.
**Rejected:**
- *Gating the chat off for OpenRouter*: leaves Intel Macs without a working chat unless
  they install Ollama, which has no GPU acceleration there.
- *A non-streaming fallback (`chat_with_tools` plus one final chunk)*: no live answer, no
  mid-reply Cancel, and a long answer waits out the whole generation timeout.
- *Sending `reasoning: {effort: "none"}` when thinking is off in Settings*: models whose
  reasoning is mandatory reject it, which would turn a preference into a failed turn.
- *Retrying a failed stream automatically*: see above.

## 2026-09-30 — Outgoing HTML keeps tables and safe inline styles

**Decision:** `sanitize_outgoing_html` allows table markup with its layout attributes and
an inline `style` reduced to a fixed property list (colour, font, alignment, spacing,
borders, size). `<style>` blocks, the `background` attribute and any declaration
containing `url(`, `expression(`, an escape or an at-rule are still removed.
**Context:** Drafts are now sanitized on save and on send, and the compose-editor
allowlist flattened a draft written in the provider's own client (tables, colours) when
it was sent from EmailOps. The developer chose to keep that formatting.
**Rejected:**
- *Passing provider drafts through unsanitized*: the backend is the security boundary;
  a draft pulled from the provider is as untrusted as one built over IPC.
- *Allowing `<style>` blocks or `background`*: both fetch remote resources when the
  recipient opens the message, and most mail clients drop `<style>` anyway.
**Limit:** this covers a draft sent without editing its body. The compose editor (Tiptap
StarterKit) has no table or style nodes, so editing such a draft in the app still
flattens it before the sanitizer sees it.

## 2026-09-30 — Remote models are sized to their own window, capped by a context budget

**Decision:** For OpenRouter, research batches are sized to `min(model window, budget)`.
The window is the selected model's `context_length` from the model catalogue (the smaller
of the model's and its top provider's), read on demand; the budget is
`chat.remote_n_ctx_budget`, 32 768 tokens by default. An unreadable catalogue falls back
to 8 192.
**Context:** OpenRouter reported no window, so research sized every batch to the 8 192
default inherited from local runtimes: many small paid calls, each resending the
instructions. The local cap exists because of RAM, which does not apply to a remote
model; what does apply is cost and how much mail leaves the machine per call.
**Rejected:**
- *Using the model's full window (128k–1M)*: one research call could ship, and bill, a
  large share of the mailbox.
- *Reusing `chat.n_ctx` for remote providers*: that setting is clamped to what the local
  KV cache fits and means something else.
- *Fetching the catalogue on every chat turn*: it is a large response; only research
  sizes prompts to the window today, so it asks when it runs.

## 2026-09-30 — The compose editor carries tables and verbatim inline styles

**Decision:** The Tiptap editor keeps table markup (with its layout attributes) and a raw
`style` attribute on text spans and block nodes, so a formatted draft survives being
edited in the app. This replaces the limit noted in "Outgoing HTML keeps tables and safe
inline styles". The backend sanitizer remains the only filter.
**Context:** With only the backend allowlist widened, editing the body of a draft written
in the provider's web client still flattened it, because the editor schema had no table
or style nodes.
**Rejected:**
- *Per-property style extensions (Color, FontFamily, FontSize…)*: each carries one
  property; a verbatim `style` keeps everything the backend allows with less code.
- *Toolbar controls for creating tables or picking colours*: the goal is to preserve
  existing formatting, not to author it.
**Limit:** `thead`/`tfoot` fold into one `tbody`, `caption` becomes a row, `colgroup`
widths, `center`/`sub`/`sup`/`small` and a `div` wrapping other blocks are lost, and
style strings are rewritten in normalised form (`#ff0000` → `rgb(255, 0, 0)`).

## 2026-09-30 — OpenRouter embeddings are an explicit, validated choice

**Decision:** With OpenRouter as the provider, embeddings run on OpenRouter too, but only
for an embedding model the user picked in the OpenRouter panel and that passed a probe.
- **Selector + notice:** the panel lists `GET /embeddings/models`, starts at "none", and
  says that with a model selected the text of every indexed email and of every search and
  chat question is sent to OpenRouter and counts against the budget; with none, semantic
  search is off and search is keyword-only.
- **Probe on save:** the email index is `float[768]`. Before a new model is saved the
  backend embeds one fixed neutral string asking for `dimensions: 768`; 768 floats back
  means the model is usable with `dimensions` on every request. If that is refused (4xx)
  or another length comes back, it asks once more without `dimensions`; 768 floats means
  usable without it. Anything else refuses the model with the length it returned, and
  nothing is saved. An outage is reported as an error, not as a verdict.
- **No request without a validated model:** the validated model id and its mode are stored
  (`openrouter_embedding_validated_model`, `openrouter_embedding_dimensions`), and the
  client embeds only while `ai_embedding_model` equals that id. Indexing, chat retrieval,
  research, memory and help lookups skip the vector path instead of sending a request.
- **One preference, many providers:** `ai_embedding_model` stays shared. Switching the
  provider tab in Settings replaces it with a model the new provider can run (none for
  OpenRouter), and the existing "embedding model changed" re-index clears the old vectors.
**Context:** OpenRouter was used for every embedding as soon as it was the provider, with
the local GGUF id left in the shared preference as the model: each email was posted to
OpenRouter, rejected, and retried on every sync, while the panel said embeddings ran
locally and offered no model field. The developer decided to keep embeddings on OpenRouter
and make that explicit.
**Rejected:**
- *Always embedding locally while chat is remote*: OpenRouter is what an Intel Mac is
  pointed to precisely because the embedded runtime cannot run there, so there would be no
  embedder at all; elsewhere it would need a second provider loaded behind a "remote" one.
- *Disabling embeddings under OpenRouter*: leaves those users with keyword search only and
  chat without retrieval by meaning, with no way to opt in.
- *Reading the vector size from the model catalogue*: `/embeddings/models` publishes no
  output dimension and an empty `supported_parameters`, so compatibility can only be found
  by asking the model.
**Limit:** the onboarding wizard still saves its OpenRouter embedding model without running
the probe, so semantic search stays off after onboarding until that model is saved once in
Settings. The probe itself is a paid call (one short string); its cost is recorded but it
is not refused for budget.

## 2026-09-30 — Onboarding validates the OpenRouter embedding model; model ids never cross providers

**Decision:** Follow-up to "OpenRouter embeddings are an explicit, validated choice"; its
Limit about onboarding no longer holds.
- **Onboarding validates:** the wizard's OpenRouter embedding field is optional and starts
  empty (no built-in default model). Empty means keyword-only search and no probe. A typed
  model is probed on Continue with the API key just typed; a failure is shown pinned at the
  top of the step and nothing is saved. The wizard shows the same privacy/cost notice as
  Settings.
- **Chat model follows the provider too:** `ai_model` stays one shared preference, and a
  provider switch in Settings replaces it like the embedding model — the saved model when
  returning to the saved provider, the first available model for Ollama and in-app, empty
  for OpenRouter. Save refuses an empty OpenRouter chat model, and the OpenRouter client
  refuses a chat model that is not `vendor/model` before sending anything.
- **Quick switcher:** the log panel's backend selector does not offer OpenRouter unless it
  is already the saved backend, because it has no field to type a model in.
- **Re-index is confirmed:** a Save that changes the embedding model (including through a
  provider switch) asks first, naming what is deleted, that search is reduced meanwhile,
  and — for OpenRouter — that every indexed email's text is sent there and billed.
- **Zero data retention:** no routing change. When the probe is refused for data policy
  with the ZDR toggle on, the message names that setting and the two ways out (turn it
  off, or use no embedding model).
**Context:** After onboarding with OpenRouter, semantic search stayed off until a Save in
Settings; switching to OpenRouter showed and saved the in-app GGUF id as its chat model;
and an embedding-model change wiped the index without warning.
**Rejected:**
- *Listing OpenRouter embedding models in the wizard*: the listing needs a saved key, and
  the key is only saved on Continue; a free-text field checked by the probe is smaller.
- *Remembering a last-used model per provider*: needs new stored state for a case the
  Settings panel already covers.
- *Warning about re-indexing in the wizard*: the wizard never triggers a re-index.

## 2026-09-30 — Settings recommends six measured OpenRouter embedding models

**Decision:** The OpenRouter embedding selector lists six recommended models first
(`openRouterEmbeddingModels.ts`), each labelled multilingual or English-only; the rest of
the catalogue follows and every model is still checked on save.
**Context:** The catalogue publishes neither the vector dimension nor the data policy of a
model's endpoints, so a user could only find a usable model by trial. On 30/09/2026
`scripts/probe_openrouter_embeddings.sh` probed all 33 catalogue models with
`data_collection: "deny"`: 15 returned 768 dimensions (through `dimensions` or natively),
and the six chosen also answered with zero data retention on.
**Rejected:**
- *Restricting the selector to the recommended models*: the probe already protects the
  vector tables, and the catalogue changes.
- *Recommending from the catalogue descriptions*: they omit the dimension for most models
  and say nothing of data policy; only a live probe shows both.

## 2026-09-30 — OpenRouter's default chat model is `google/gemini-3.5-flash-lite`

**Decision:** When no OpenRouter chat model was chosen yet, Settings and onboarding offer
`google/gemini-3.5-flash-lite` (one constant, `DEFAULT_OPENROUTER_CHAT_MODEL`). The field
stays free text.
**Context:** Switching to OpenRouter left the chat field empty (after the fix that stopped
it showing the in-app model), and onboarding defaulted to `openai/gpt-4o-mini`, a 2024
model with no zero-data-retention endpoint, so chat failed with that setting on. The
developer picked the default from the public catalogue: tool calls supported, a
zero-data-retention endpoint, 1M context.
**Rejected:** `openai/gpt-6-luna` (cheaper, offered first) and
`anthropic/claude-haiku-4.5` — the developer's choice; `openai/gpt-4o-mini` — no
zero-data-retention endpoint.
**Limit:** not measured on the app's chat eval; chosen on catalogue data only.

## 2026-09-30 — A chat prompt is cut to the window in a fixed order, never the system prompt

**Decision:** Before every model call of a chat turn the prompt is sized against the
model's window (`services/chat/budget.rs`). When it would not fit, it is cut in this
order: the emails earlier questions were asked with, then earlier exchanges, then this
turn's open thread and retrieved emails, then this turn's tool results. The system
prompt, the question and its per-turn blocks are never cut. Every cut is in the trace;
the user is told under the answer only when something of the current turn was cut or the
prompt did not fit. Sizes are estimated from chars and corrected with the provider's
token counts.
**Context:** Nothing checked the prompt against the window. The embedded runtime drops
tokens from the front, so an overflow cost the rules, the date and the tool catalogue
first, silently; Ollama truncates on its own and OpenRouter was sent everything. One
retrieval turn carries up to 11 emails of 4 000 chars, earlier turns replay theirs, and
tool results had no cap. What gives way first is a product choice, made by the developer.
**Rejected:**
- *Dropping whole earlier turns only*: simplest, but the model forgets what the
  conversation was about by the third or fourth retrieval turn at 32k although the
  questions and answers themselves are small.
- *Never replaying earlier emails*: uniform, but it changes every follow-up even where
  the prompt fits, and the prompt stops extending the previous one.
- *Exact token counts through the provider trait*: precise only on the embedded runtime,
  one actor round-trip per call, and a wider trait. The measured ratio gets close enough
  with the runtime's truncation kept as the safety net.
- *Telling the user about every cut*: on a small window the note would sit under most
  answers of a long conversation.

## 2026-09-30 — Windows under 16k tokens get a compact chat system prompt

**Decision:** When the model's window is under 16 384 tokens the chat renders
`chat.system_compact` and a compact tool catalogue (each tool's one-line summary, the
first sentence of each parameter description, stated once) instead of the full ones. The
choice depends on the window alone, never on the turn, and prewarm makes it through the
same function. A `chat.system` the user customised is kept at any window.
**Context:** The full system message is about 29 000 chars (~7 400 tokens): the template
11 500, the tool catalogue 16 600. An 8 192 window leaves 7 168 for the prompt, so on a
machine under 16 GB, and on Ollama, the system prompt did not fit even in an empty
conversation and the budget above had nothing left to cut. The compact message is pinned
under 16 000 chars by a test. On the demo mailbox a retrieval turn at 8 192 went from
11 961 prompt tokens with 4 793 dropped to 6 112 with none.
**Rejected:**
- *Raising the smallest tier to 16k*: the KV cache for 16k costs memory on exactly the
  machines that get 8k because they have none to spare, and it does nothing for Ollama.
- *Trimming only the tool catalogue*: the template alone is 11 500 chars; with a compact
  catalogue the message was still ~5 800 tokens, leaving about 1 000 for everything else.
- *A per-turn choice of prompt* (compact only when the turn is large): the system prefix
  would change between turns and cold-prefill the KV cache each time it did.
**Limit:** the compact prompt keeps the email-link contract and three examples whole (a
first version with one-line link rules lost the links on tool-result answers) and states
the other rules in one line each. Checked on the embedded runtime only, with
`qwen3.5-4b-q4_k_m`: the smoke tier at 8 192 passes 36 of 41 with no prompt truncated,
against 36 of 41 with 39 prompts truncated before; three of the five failures also fail at
the default window, and which other cases fail moved between runs of near-identical
prompts. Ollama and OpenRouter were not run.

## 2026-09-30 — AI models are remembered per provider; a save never keeps an unusable embedding model

**Decision:** Supersedes the "Remembering a last-used model per provider" rejection in the
entry above on onboarding and provider switches.
- **Remembered per provider:** `ai_model` / `ai_embedding_model` stay the models in use,
  and every save also records them under `ai_model:<provider>` /
  `ai_embedding_model:<provider>` (preferences only, no migration). Leaving a provider
  records the models it was using at that moment. `get_ai_config` returns the remembered
  models for every provider, and Settings and onboarding offer them on a provider switch
  before falling back to the defaults. Existing installs are seeded from what they already
  store: the saved provider's models, and `openrouter_embedding_validated_model` as
  OpenRouter's embedding model. A remembered OpenRouter model that is still the validated
  one is not probed again.
- **Save never keeps an embedding model the provider cannot use:** `save_config` decides
  with a pure planner (`plan_embedding_model`). OpenRouter takes a `vendor/model` id or
  none; the in-app runtime takes a catalogue embedding model; Ollama refuses a catalogue
  GGUF id and the model remembered for OpenRouter. Anything else is replaced by the model
  remembered for that provider, else by the provider default, and the correction is logged.
- **Quick switcher does not cross an Embeddings boundary:** the log panel's backend
  selector only performs a switch when the target provider is remembered with the very
  embedding model in use. Every other switch is disabled there with a hint to do it in AI
  Settings, the only place that asks before the email index is replaced.
**Context:** After saving OpenRouter with an embedding model, switching to the in-app
provider and coming back showed no embedding model; and the quick switcher, which named no
embedding model, left the in-app provider saved with OpenRouter's, so local Embeddings
could not run. Onboarding with Ollama likewise saved the in-app GGUF id for Ollama.
**Rejected:**
- *Refusing every `vendor/model` id under Ollama*: Ollama has namespaced models of its
  own, so a slash alone does not make an id OpenRouter's.
- *Letting the quick switcher switch and re-index*: it has no room for the warning, and a
  re-index sends every indexed email to OpenRouter when that is the target.
- *A migration to per-provider columns*: preferences hold it, and the seeding covers
  existing installs.
**Limit:** the in-app runtime and Ollama name the same nomic model differently, so the
quick switcher no longer switches between them either unless the ids happen to match; in
practice every backend change now goes through AI Settings.

## 2026-09-30 — Changing the AI provider or a model asks about background AI work first

**Decision:** Before a change to the AI provider, the chat model or the embedding model is
saved — in Settings → AI and in the log panel's quick selector — the app reads the AI
background queue. If work the change cuts across is running or queued, one dialog lists it
and offers **Stop and apply**, **Wait and apply** or **Cancel**; with nothing affected there
is no dialog. The re-index confirmation stays a separate, later step.
- **What counts:** tasks on the AI background queue that call the provider, by the kind
  their name maps to (`services::ai_activity::work_kind`): Embeddings rebuild and
  generation, classification, memory extraction, task extraction, Lens extraction. A chat
  model change cuts across the kinds that write with it, an embedding model change across
  the ones that embed, a provider change across all. Junk scoring (no model) and unknown
  tasks are never listed or stopped. Chat turns, drafts, translations and searches run on
  the interactive queue or inline, finish with the provider they started with and have their
  own controls: they do not block the change.
- **Stopping is cooperative:** `TaskQueue::cancel_matching` raises a per-task flag that the
  loops read between emails (`task_queue::cancel_requested`). The task leaves through its
  normal exit, so its terminal events and clean-up run; Embeddings emit
  `embedding-progress` with status `cancelled`. A queued task is not dropped either: it
  starts already cancelled and exits at its first check — a rebuild before deleting the
  index.
- **Waiting is polled:** while stopping or waiting the dialog re-reads the queue every
  second and applies the change when nothing affected is left, so work a sync queues
  meanwhile is waited for (or stopped) too.
**Context:** An embedding run loaded its provider once per batch of up to 500 emails. After
switching away from OpenRouter, the batch in hand kept sending email text there, billed;
the rest of the task continued with the new provider and the rebuild the save queued redid
everything. Nothing told the user.
**Rejected:**
- *Aborting the task's future at the queue*: simpler, but a dropped future skips what the
  task does on its way out — the Lens run registry, the memory and task backfill "running"
  flags, `lens_runs` rows left `running`, progress indicators waiting for a terminal event.
  The same holds for dropping queued futures unpolled.
- *Reloading the provider for every email*: fixes which provider is used, not that the
  user is never asked, and the rebuild queued by the save would still redo the work.
- *One dialog for the work in progress and the re-index*: they are two decisions; the
  second only exists when the embedding model changes.
- *A drain event from the queue*: a poll of the same snapshot is robust to tasks queued
  between the event and the save, and needs no new event.
**Limit:** a task stops at its next email, so the request in flight when the user stops
completes (one email, at most six chunk requests for Embeddings). A single-row Lens
re-extract has no loop and finishes its one call. A task queued in the instant between
the last poll and the save starts with the old settings for one batch.

## 2026-09-30 — The AI backend is changed only in Settings

**Decision:** The status bar of the Logs panel no longer has a backend selector. It names
the backend in use and keeps the chat-model selector of that backend; the backend is
changed in Settings → AI only.
**Context:** A backend change can replace the Embeddings, needs a provider-valid chat and
embedding model and may cut across running AI work. Settings asks about all three; the
quick selector had to be disabled for nearly every switch to stay safe, and it was the
path that left an OpenRouter embedding model under the in-app provider.
**Rejected:** *Keeping the selector with most options disabled* — a control that almost
never works is worse than none.

## 2026-09-30 — Attachments are quarantined one by one; dangerous types need confirmation

**Decision:** Every attachment file the app writes (rule collection, auto-download, save to
Downloads, bulk download) is marked as received from outside — `com.apple.quarantine` on
macOS (`0081;<hex time>;EmailOps;<uuid>`), the `Zone.Identifier` stream on Windows, nothing on
Linux — and "open in the default app" marks the file again before the hand-off, which also
covers files stored before this. Types whose default action runs code or opens another
location (one extension table in `services/attachment_safety.rs`, plus the declared MIME type)
are opened only after a dialog that names the file, its kind and that it came by email; the
backend enforces it with a `confirmed` argument and refuses with
`attachment_confirmation_required` otherwise.
- **When the mark cannot be written:** a save still succeeds and the failure is logged; an
  open is refused with the error, for every type — the OS would launch the file unchecked.
- **Reveal:** "Show in Finder" selects, never opens. A directory with a launchable name (an
  app bundle) is selected in its folder instead of opened.
**Context:** A security review found attachments stored under the sender's extension and
handed to `open::that` with no quarantine attribute and no type check, so a `.terminal`,
`.fileloc`, `.jar`, `.command` or local `.html` launched without the first-open prompt
Mail.app would show.
**Rejected:**
- *`LSFileQuarantineEnabled` for the whole app*: it quarantines every file the app creates —
  the database, models, exports, skills — not only what a sender controls.
- *Refusing dangerous types outright*: people do receive installers and scripts they asked
  for; the OS check plus an explicit confirmation is the Mail.app behaviour.
- *Classifying in the frontend*: a second copy of the list, and a direct IPC call would
  bypass it.
- *Treating documents with a risky reader (PDF, Office macros, archives) as dangerous*: they
  do not act on open by themselves; the quarantine mark lets their own apps apply Protected
  View and similar.

## 2026-09-30 — Gmail stored-mail state follows the History API

**Decision:** Gmail's stored mail is refreshed from `users.history.list` instead of being
polled: a per-account cursor in preferences (`mailbox_history_cursor:<account>`), read
under the same 2-minute throttle as the IMAP/Outlook refresh, at most 5 pages of 100 records
per pass, asking only for `labelAdded`, `labelRemoved` and `messageDeleted`. No new OAuth
scope: `gmail.modify` covers it.
- **Cursor:** seeded from `users.getProfile` on the first run (nothing is replayed); moved
  only past pages that were fully applied and written once per pass; cleared with the
  account.
- **What a change means locally:** the label deltas are folded onto the stored row and the
  result goes through the mapping the sync already stores mail with
  (`sync::gmail::mailbox_from_labels`). `UNREAD` is the read state; `TRASH` files the row
  under `trash` and removing it puts it back; a permanent delete soft-deletes the row.
  **Archiving is not a move**: the app has no archive mailbox and the sync already stores
  mail without `INBOX` under `inbox`, so archiving and user labels change nothing here.
  Spam is left to the Spam pass.
- **Conflict rule:** unchanged — a row with a pending local push is never touched, checked
  in the planner and again in each `UPDATE`. Because the log reports a change once, a page
  that had to skip such a row is not counted as applied and is replayed after the push.
- **Expired cursor (404):** the recent stored rows (the 30 days / 200 rows of the poll) are
  checked against their current labels in `format=minimal` batches, and the cursor is
  reseeded — from a position read before the check — only once every row was checked.
**Context:** Gmail answered `None` to the state poll, so read/unread, trash and permanent
deletes done in Gmail's web or mobile clients never reached stored mail; only Spam was
reconciled. The developer chose the History API over polling.
**Rejected:**
- *Polling `format=minimal` for the recent ids every pass* (what IMAP/Outlook do): 200 gets
  every two minutes against a quota-metered API, to learn that nothing changed.
- *Soft-deleting a message trashed in Gmail*, like the app's own delete: it could never
  come back when the user restores it in Gmail, and mail trashed there before it was ever
  synced already shows under Trash.
- *Looking each changed message up (`messages.get`) instead of folding the deltas*: exact,
  but one request per change turns a bulk clean-up into hundreds of requests, and a lookup
  cap would leave pages half applied.
- *An archive mailbox*: a product change (a new view and its sync pass), not part of
  following state.

## 2026-09-30 — The compose editor also keeps table sections, captions, column widths and sub/sup/small/center

**Decision:** Follow-up to "The compose editor carries tables and verbatim inline styles",
which listed these as a limit. A formatted draft now keeps them through load → edit →
save, with no new dependency:
- **`thead` / `tfoot`:** each row remembers the section it was written in (a row attribute,
  not rendered on the `<tr>`) and the table's serializer regroups the rows into `thead`,
  `tbody`, `tfoot`, in that order. A table without body rows gets no empty `tbody`.
- **`caption`:** an attribute of the table holding its text and its `style` / `align`,
  written back as `<caption>` and shown read-only above the rows in the editor.
- **`colgroup` / `col`:** an attribute of the table holding each column's `span`, `width`
  and `style`, written back as one `<colgroup>` — and left out once it no longer adds up to
  the table's columns.
- **`sub`, `sup`, `small`:** three marks written with `Mark.create`.
- **`center`:** a block node written back as `<center>`; text directly inside it becomes a
  paragraph inside it.
**Context:** ProseMirror's table model (`prosemirror-tables`, under Tiptap 3's table
extension) requires a table's children to be rows, so sections, captions and column groups
cannot be nodes of their own without replacing the table plugin.
**Rejected:**
- *`@tiptap/extension-subscript` / `-superscript`*: not installed; two five-line marks do
  the same without a dependency.
- *Turning `<center>` into a paragraph with `align="center"`*: it cannot hold a table, which
  is what newsletters centre with it.
- *Keeping the caption as HTML*: the editor would have to show markup it did not parse
  through the schema.
**Limit:** markup inside a caption is reduced to its text and the caption cannot be edited
in the app; attributes on `thead` / `tbody` / `tfoot` themselves are dropped (those on rows
and cells are kept); several `colgroup`s are merged into one. A `div` wrapping other blocks
is still unwrapped, and style strings are still rewritten in normalised form.

## 2026-09-30 — Outlook attachments past one request go through a draft and upload sessions

**Decision:** Outlook keeps sending everything in one Graph request while every attachment
is under 3 MB and they stay under 3 MB together. Past that — for new mail, replies
(`createReply`), and draft create/update — the message is created as a draft without
attachments, each attachment is added on its own (one `POST …/attachments` under 3 MB, an
upload session from 3 MB to Graph's 150 MB maximum, in 2,949,120-byte ranges), and the
draft is sent with `POST …/send`. A file over 150 MB is refused with `InvalidInput`,
naming it, before any request. No new scope: `Mail.ReadWrite` + `Mail.Send` cover it.
- **Ranges** are idempotent and are re-sent up to five times (retryable status, transport
  error, or an answer that does not move the upload forward); the upload continues from the
  `nextExpectedRanges` Graph reports. The final send keeps the no-retry-after-send policy.
- **Clean-up:** when an attachment cannot be added, the upload session is cancelled and the
  draft this client created is deleted. When the final send fails, the draft is **kept**:
  the send may have gone through, and if it did not the draft still holds the uploaded
  attachments; the error says so.
- **Draft updates** replace the provider draft's attachments (list, delete, add), because
  an upload session can only add.
- **Memory:** attachments reach the provider as base64 text; ranges are decoded from that
  text one at a time instead of decoding the whole file next to it.
**Context:** Every attachment was inlined as base64 in one JSON request, with no size check,
so an attachment over about 3 MB failed with Graph's request-too-large error. Graph documents
"under 3 MB" for an inline attachment, 3–150 MB for an upload session (which it refuses for a
smaller file), ranges under 4 MB, and a request limit of about 4 MB.
**Rejected:**
- *Always using the draft route*: three requests instead of one for the common small
  attachment, and a changed payload for a path that works.
- *Sending the file's MIME through `sendMail`*: still one request under the same limit.
- *Deleting the draft after a failed send*: a 5xx does not say whether the message left.
- *Skipping attachments that look unchanged on a draft update* (same name): a replaced file
  of the same name would stay stale in the provider's copy, and Graph does not report the
  content size to compare with.
**Limit:** a draft with large attachments is uploaded again on every push of that draft; the
base64 text itself is still built in memory by the compose layer.

## 2026-09-30 — Coverage with cargo-llvm-cov and vitest v8; mutation testing for Rust only

**Decision:** Test-suite quality is measured with three local tools, run by hand and not in CI
or the gates: `cargo-llvm-cov` for Rust coverage (`make coverage-rust`, measured with
`--no-default-features`, the feature set CI tests), `@vitest/coverage-v8` for TypeScript
coverage (`make coverage-ts`), and `cargo-mutants` for Rust mutation testing
(`make mutants`, run `--in-place` in dedicated detached worktrees, each with its own target
dir). Mutation testing of TypeScript is deferred. Equivalent mutants are recorded in
`docs/testing/MUTANTS-LEDGER.md`; the workflow is in `docs/testing/COVERAGE-AND-MUTATION.md`.
**Context:** Line coverage alone overstates how well the planners are guarded: a covered line
can have no assertion on it. Mutation testing measures that directly, and the pure
planner/executor split makes most planners cheap to mutate. A full-crate run is about 14,000
mutants, so it is run per module with a test-name filter, then the misses are re-checked
against the whole suite.
**Rejected:**
- *Stryker for TypeScript now*: its Vitest runner reports false survivors on Vitest 5, which
  would bury real gaps in noise. Revisit when the runner supports Vitest 5.
- *cargo-mutants' default scratch copies*: each copy builds every dependency cold.
- *Running mutants in the main checkout's target dir*: cargo names the crate's artifacts
  without the checkout path, so two checkouts overwrite each other's incremental cache
  (43 s per mutant build instead of 4–6 s).
- *cargo-nextest*: not needed at this scale, and no new tool beyond the three above.

## 2026-10-01 — Commands check that a record belongs to the account; Outlook grants are removed by the user

**Decision:** Every command that acts on a per-account record named by id (email, draft,
attachment rule, chat conversation, memory fact, task) takes the account the UI is working
in and refuses a record of another account with `NotFound`, through
`services::ownership`. Removing an Outlook account deletes its local tokens and tells the
user, in the delete confirmation and the removal log, where to remove EmailOps' access at
Microsoft (account.live.com/consent/Manage, or My Apps for work and school accounts).
**Context:** The CASA review (control 3.1.4) found commands that looked records up by id
alone. EmailOps is single-user, so this is not a barrier between people, but several were
real cross-account bugs: saving or deleting a draft could reach another account's draft and
its provider copy, deleting an attachment rule removed another account's files, and junk
feedback or a new task could point at another account's email. Google revokes a grant
through an RFC 7009 endpoint; Microsoft has no per-application revocation.
**Rejected:**
- *Microsoft Graph `revokeSignInSessions`*: it revokes every refresh token of the user, signing
  them out of all apps and devices — far more than removing one account here should do.
- *Scoping the batch reads (`get_email_tags_batch`, `get_junk_verdicts`)*: the unified inbox
  asks for many accounts at once and the `email-junk-scored` event carries no account; they
  return only tags and verdicts, so they stay keyed by email id.
- *Lens commands*: lenses can span every account (`account_id` NULL), so there is no single
  account to check against.

## 2026-10-01 — The macOS app ships with no Hardened Runtime entitlements

**Decision:** `src-tauri/entitlements.plist` is empty: the Developer ID build carries no
`com.apple.security.cs.*` exceptions, and `make verify-mac` (`scripts/verify_mac.sh`) fails
when one comes back. `make build-mac` also notarizes and staples the DMG, not only the app.
**Context:** The CASA desktop checklist (DASA 3.3.2) asks for a justification of every
entitlement that weakens the Hardened Runtime. `cs.allow-jit` and
`cs.allow-unsigned-executable-memory` had been added defensively with llama.cpp. A release
build signed with `--options runtime` and no entitlements completed an embedded-model chat
turn: Metal compiles shaders in the GPU driver's process, and llama.cpp maps no JIT or
writable-executable pages in ours. The DMG was signed but not notarized, so Gatekeeper
rejected the download itself (DASA 3.2.1).
**Rejected:**
- *Keep `cs.allow-jit` "just in case"*: it is an exception assessors ask to justify, with
  nothing to justify it today. If a llama.cpp upgrade ever needs it, the verify guard
  surfaces that as a deliberate decision.

## 2026-10-01 — Raw library and OS error text stays out of the webview

**Decision:** `AppError`'s `Serialize` (the Tauri boundary) sends `database`, `http`,
`json`, `io` and `keyring` errors with a generic message and no `detail` param; the full
error goes to the output panel through the logger. The CLI `--json` envelope uses
`AppError::diagnostic_json()` and keeps the detail. Codes whose detail is written for the
user (`invalid_input`, `auth`, `sync`, `ai`, …) are unchanged.
**Context:** The CASA desktop checklist (DASA 1.8.1) forbids user-visible errors that show
file paths, SQL, stack traces or other internals. Those five variants carry rusqlite,
reqwest, serde and keyring messages, or `format!`ed text with absolute paths.
**Rejected:**
- *Stripping detail in the frontend only*: the raw text would still cross IPC and be shown
  by any component that renders `message` directly.
- *Redacting the CLI too*: the CLI is a developer surface; agents debugging a failure need
  the raw cause.

## 2026-10-01 — Meeting-reminder OS notifications hide the title by default

**Decision:** The OS notification for an upcoming meeting reads "Upcoming meeting · Starts in
N min" unless the user turns on **Settings → Calendar → Show the meeting title in
notifications** (`calendar_notification_show_title`, default off). The in-app reminder
banner always shows the full event.
**Context:** The CASA desktop checklist (DASA 1.10.2) asks that sensitive data not be
exposed through notifications. Meeting titles often name people, deals or medical
appointments, and OS notifications reach the lock screen and Notification Center even
while EmailOps' main password lock is up.
**Rejected:**
- *Hide the title only when a main password is set*: the lock screen is outside EmailOps'
  lock either way, so the main password is no signal; one plain switch is easier to explain.

## 2026-10-01 — Archive is a mailbox of its own; IMAP archives into its Archive folder, or refuses

**Decision:** Archiving takes a conversation's inbox messages out of the inbox at the
provider and files them locally where the provider put them: Gmail removes the `INBOX`
label and Graph moves to the well-known `archive` folder, both stored under a new
`emails.mailbox = 'archive'` (an "Archive" view in the sidebar); IMAP moves to the folder
flagged `\Archive` (RFC 6154), else one named Archive/Archives/Archiv/Archivo, stored as
that `folder:` view like any other synced folder. An IMAP account with no such folder is
refused with `AppError::NoArchiveFolder` ("create a folder named Archive"); the app does
not create one. Archive and its inverse, *Move to Inbox* (Gmail adds `INBOX`, Graph and
IMAP move to the inbox), are **provider-first** like delete and folder moves — a failure
leaves the conversation where it was and is reported — while read state and the star stay
local-first. Gmail mail without `INBOX` (and not in Sent/Spam/Trash) now maps to `archive`
everywhere (`mailbox_from_labels`), so archiving in Gmail's own clients is followed by the
History API refresh, and Graph's `archive` folder maps to `archive` when a moved message is
located.
**Context:** Archive was the most-reached-for missing action (docs/COMPETITOR-PARITY.md).
Until now archived Gmail mail was deliberately filed under `inbox` ("the app has no archive
mailbox"), so an archive could not leave the inbox view. IMAP and Graph re-key a moved
message, so a local-first archive with a retry marker would leave a row the provider no
longer knows under its id between the change and the retry, racing the state refresh and
the UIDVALIDITY re-key; provider-first has nothing half-done.
**Rejected:**
- *A local `archived` flag on rows that stay in `inbox`*: every inbox query would need a
  second predicate, and Gmail's own archive (no `INBOX`) has no place to land.
- *Storing IMAP archives under `archive` too*: the Archive folder is already synced as a
  `folder:` mailbox with its own id prefix and UIDVALIDITY; two names for one folder
  would make the sync re-file the rows on every pass.
- *Creating an Archive folder on IMAP servers that lack one*: a folder appearing on the
  user's server unasked; the user can create it in one click with the existing folder
  management, and the error says so.
- *Archiving locally when the IMAP server has no archive folder*: the silent divergence the
  2026-09-30 entries removed for delete.

## 2026-10-01 — The star is per message, local-first with a retried push; a thread is starred when any message is

**Decision:** `emails.is_starred` (V030) mirrors Gmail `STARRED`, Graph `flag.flagStatus =
flagged` and IMAP `\Flagged`. Starring a conversation stars its latest message (spam and
trash copies aside); unstarring clears every starred message; lists that show one row per
conversation (inbox, tag/sender filters) widen the row's star to the thread's. The write
follows read state exactly: `star_push_pending_since` is set in the same statement as the
star, cleared once the provider has it, retried by every sync (same caps and one-week
give-up as V029), and a pending star is never overwritten by the server-to-local refresh.
Stars set in other clients are ingested where it is cheap: on download (Gmail labels,
Graph `flag` in `$select`, IMAP `FLAGS`), from Gmail's History API, and in the IMAP/Graph
state refresh (`FLAGS` / `$select=id,isRead,flag`). Outlook's `complete` flag is not a star.
**Context:** "Flagged" already meant junk in the UI; the parity audit asked for Gmail-style
stars with write-back. Gmail itself stars a message, not a conversation, and shows the
conversation starred when any message is — per-message storage keeps the sync a plain
mapping, and the thread view is derived.
**Rejected:** *A thread-level `starred_threads` table*: it has no provider counterpart, so
every sync would have to reconcile it against per-message state anyway.

## 2026-10-01 — Thread actions go through one command that reports failures per thread

**Decision:** Mark read/unread, star/unstar, archive and move-to-inbox are one command,
`apply_thread_action(threads, action)`, taking any number of `(account_id, thread_id)`
pairs. Each thread is planned on its own (`plan_thread_action`, pure) and the command never
fails as a whole: it returns the threads it could not change with the `AppError` code and
message, and the frontend (`emailStore` `setThreadsRead` / `setThreadsStarred` /
`archiveThreads` / `moveThreadsToInbox`) updates optimistically and rolls back exactly those
threads with one toast. Marking unread marks the latest received message, as Gmail does,
and leaves the open conversation, so it is not read again the moment it is looked at.
**Context:** Bulk selection, keyboard shortcuts and rules all need the same actions over
many threads; one entry point with per-thread outcomes lets them share the optimistic
update and rollback instead of each looping over single-message commands.
**Rejected:** *One command per action*: four copies of the same grouping, provider
resolution and reporting. *Failing the whole call on the first error*: a bulk archive with
one refused thread would roll back the ninety-nine that went through.

## 2026-10-01 — Archive and delete wait out a six-second undo window before reaching the provider

**Decision:** Archive and delete (one conversation or a bulk selection, from the list menu,
the reading pane or the bulk toolbar) take the rows out of the list at once and show a
"Archived 3 conversations · Undo" toast for six seconds (`UNDO_WINDOW_MS`), but the
provider call (`apply_thread_action`, which now has a `delete` action taking any number of
threads) is **deferred until the window closes**. Undo restores the rows locally; the
provider never hears of the action, so no provider needs an un-trash or un-archive path.
The pending action commits early when another archive/delete starts (one undo at a time,
as Gmail does), when a non-background list fetch opens another view (the Archive view must
show what was just archived), and on `beforeunload`. Until it commits, a background refetch
filters the pending conversations out (`pendingRemovals`) so a sync does not bring them
back. **If the app quits inside the window the action simply never happened**: the mail
is still where it was at the provider and reappears on the next launch. The queue is the
pure `createPendingActionQueue` (`src/lib/pendingActions.ts`). A commit that fails rolls
back exactly the refused conversations with one toast, as every thread action does.
**Context:** The parity audit made Undo for archive/delete a High item, and every
destructive action needs a reachable inverse. Delete and archive were provider-first and
immediate; undoing them afterwards would have needed per-provider restore paths (Gmail
untrash, Graph move back from Deleted Items/Archive, IMAP move back with a re-keyed UID),
each racing the state refresh.
**Rejected:**
- *Commit immediately and undo with a second provider call*: three new restore paths,
  each re-keying rows on IMAP/Graph, for a feature whose whole point is that nothing
  should have happened.
- *Persisting the pending action so it survives a quit*: a durable outbox for a six-second
  window; losing an archive the user just made (it stays in the inbox) is harmless and
  visible, unlike losing a send.
- *Keeping the old per-message delete loop in the frontend*: N invokes with no error
  aggregation; bulk delete is one `apply_thread_action(threads, delete)` call with a
  per-thread report.

## 2026-10-01 — Snooze is local state; a woken conversation sorts by its wake time

**Decision:** Snooze is stored locally (V031 `thread_snoozes`, keyed by
`(account_id, thread_id)`) and works the same on Gmail, Outlook and IMAP. A snoozed
conversation is left out of the Inbox list and count (a primary-key `NOT EXISTS` seek per
candidate row) but stays findable by search, filters and the other views; the new
**Snoozed** view lists it, soonest wake first. A ticker in `sync_scheduler` wakes due
snoozes every 30 s and once at start-up (a snooze that fell due while the app was closed
wakes on the next launch). Waking does three things: the record is kept as *woken*
(`woke_at`), the latest message is marked unread **and that is pushed to the provider**
like any mark-unread (otherwise the next state refresh would read it back as read, and
other clients would not see it as new), and a `snoozes-woken` event refreshes the list
(the hook a future new-mail notifier can use). The Inbox then sorts a woken conversation by
its **wake time** instead of its latest message's date, so a thread snoozed weeks ago comes
back at the top, as in Gmail, while the message keeps its real timestamp. The list query
merges two arms on a sort key: the woken conversations (a handful, driven from
`thread_snoozes`) and every other conversation read through `idx_emails_account_mailbox`
in date order and cut at `offset + limit`; only that small merge is sorted (pinned by an
`EXPLAIN QUERY PLAN` test). A new inbound inbox message in a snoozed or woken conversation
deletes the record in the ingest transaction (Gmail returns the thread on a reply; the
thread then sorts by the new message); sent mail, spam, backfill dated before the snooze
and re-downloads of stored messages do not. Archiving or deleting a conversation ends its
snooze. Woken records whose conversation left the inbox are pruned on each tick. Snooze and
unsnooze are separate commands (`snooze_threads(threads, until)`, `unsnooze_threads`,
`list_thread_snoozes`), not `ThreadAction` variants: they carry a time and never touch the
provider. Undo of a snooze is an unsnooze (no deferral needed — nothing reaches the
provider).
**Context:** Parity audit item (High). No provider exposes a portable snooze (Gmail's is not
in its API; Graph and IMAP have none), and the audit's constraint is that snooze, mute and
pin are local state. The inbox query must stay index-driven on 47k+ emails.
**Rejected:**
- *Rewriting the message timestamp to the wake time*: corrupts dates shown in the thread,
  search ordering and every date-based feature.
- *Ordering the inbox by `COALESCE(woke_at, timestamp)`*: defeats the index order, so every
  page sorts the whole inbox.
- *Normal ordering plus unread only*: a thread snoozed for a week reappears pages down,
  which defeats the point of snoozing.
- *Not pushing the wake's unread to the provider*: the state refresh would mark it read
  again within minutes.
- *Hiding snoozed mail at the provider (archive on snooze, move back on wake)*: provider
  writes, IMAP re-keying and failure modes for a reminder, and the conversation would be
  lost from the inbox if the app never ran again.

## 2026-10-01 — Undo send and scheduled send share one local outbox; overdue mail goes out at launch, an interrupted send never resends

**Decision:** A message sent with an undo window or scheduled for later waits in a local
`outbox` table (V032) holding the composed message as JSON (recipients, subject, sanitized
HTML, inline images and attachment bytes as base64 — files a draft referenced by path are
read in when the message is queued) until its `send_at`. Undo send is a scheduled send
`delay` seconds ahead (setting *Undo send*: off / 5 / 10 / 20 / 30 s, default 10, in
`user_preferences` as `compose.undo_send_delay_secs`; off keeps the direct send). A
dispatcher (`sync_scheduler::outbox_dispatch_loop`, every 15 s and woken when an undo
window closes or the user picks *Send now*) sends due rows through `emails::send_outgoing`,
i.e. the same `deliver_reply` / `deliver_new_email` path, optimistic Sent copy included.
- **No double send:** a row is flipped `scheduled → sending` in one guarded UPDATE before
  the provider is called; undo, edit and delete are guarded UPDATEs from `scheduled` /
  `failed`, so the backend decides the race and a late undo answers `outbox_not_pending`.
- **Crash mid-send:** a row still `sending` at start-up becomes `failed`
  (`interrupted`) with "may or may not have been sent — check Sent"; it is never resent
  automatically. A provider failure is `failed` with its error and is not retried on its
  own either (a 5xx does not say whether the message left); *Retry* is the user's call.
- **Overdue at launch:** a message whose time passed while the app was closed is sent
  automatically on the next launch, as Gmail and Outlook send it — the user asked for it
  to go out, and the Scheduled view and schedule menu say the app must be open.
- **Drafts:** queueing a message the composer had saved as a draft deletes that draft
  (locally and at the provider), as an immediate send does; the queued copy owns the
  content. Undo / *Edit* reopen it in a compose tab (replies keep `replyToEmailId`, so they
  are still sent as replies), whose auto-save creates a fresh draft.
- The payload is emptied when a row is sent or cancelled; finished rows are deleted after
  7 days. The payload is never logged.
**Context:** Parity audit items (High). IMAP has no server-side scheduling and Gmail's API
exposes none, so a local queue is the only way to offer both on every account type, and one
queue gives both features the same send path and the same safety rules.
**Rejected:**
- *A frontend timer for undo send*: the message would be lost if the window closed or the
  app quit during the delay, and a scheduled send needs persistence anyway.
- *Asking at launch before sending overdue mail*: a prompt the user may not see for hours
  turns "send Monday 8:00" into "send whenever I next click"; the delay is visible in Sent.
- *Keeping the draft while a message is scheduled*: two copies of the same text, and the
  draft sync could push or prune it while the queued copy is the one that will go out.
- *Referencing attachment files by path*: temp files and moved files would break a send
  hours later.
- *Retrying a failed or interrupted send automatically*: a duplicate email is worse than a
  visible failure with a Retry button.

## 2026-10-01 — One signature per account, kept in a marked block of the composer body

**Decision:** Each account has one rich-HTML signature (V033 `account_signatures`, a row
per account, cascading with it) with two options, "insert in new messages" and "insert in
replies and forwards". It is edited in Settings → Signatures with the compose editor and
sanitized on save with the send allowlist (`sanitize_outgoing_html`), so pasted images are
kept as data URLs (the send path already turns them into inline `cid:` parts) under a
512 KB cap. Composers insert it inside `<div data-emailops-signature>`, a node the compose
editor schema keeps:
- **Placement:** below the text in a new message or a reply, above the forwarded message
  in a forward (Gmail's default). No option to put it "below the quote": replies carry no
  quoted original in the editor or at send time, and a forward's quote is part of the
  body, where only "above" makes sense. No `-- ` separator is forced; the user can type
  one into the signature.
- **Swap / no double insert:** changing the From account swaps the block's content (or
  removes it when the new account has none). A body that already exists — a reopened
  draft, a message taken back from the outbox, a maximized composer — is never given a
  second signature; it is only swapped if it still has the block. A fresh compose tab
  (mailto link) inserts it.
- **AI drafts:** a generated draft replaces the text above the block, keeping the
  signature as the user left it; the block is left out of the brief sent to the model, of
  the send-time "did you mean to attach?" check and of the autosave trigger (a composer
  holding only its signature is not saved as a draft).
- **Gmail import:** Settings offers "Import from Gmail" for Gmail accounts, reading the
  `signature` of the account's "Send mail as" entry (the endpoint already used for the
  sender name). It fills the editor; the user saves. Graph exposes no Outlook signature
  and IMAP has none, so there is no import for them.
**Context:** Parity audit item (High). The composers own their body as one HTML string,
so the signature has to be findable inside it to be swapped or kept.
**Rejected:** Appending the signature at send time — the user could not see or edit it
before sending, and a draft opened in Gmail would lack it. Storing it in the
`account_settings:` preference blob — that blob is saved whole by the account dialog, and
a signature needs its own sanitizing save and should go with its account. Several named
signatures per account — beyond the parity gap; one per account covers Gmail's default.

## 2026-10-01 — Keyboard shortcuts come from one registry, Gmail's bindings, one root handler

**Decision:** Every keyboard shortcut is a row of `SHORTCUTS` (`src/lib/shortcuts.ts`:
id, bindings, scope, help group, i18n label). A pure matcher turns a key press into an id
(platform modifier: Cmd on macOS, Ctrl elsewhere, strictly; two-key sequences such as
`g i` with a 1 s timeout on an injected clock), a pure planner (`src/lib/shortcutPlan.ts`)
turns the id into an effect for the current screen, and one hook mounted in `App`
(`useGlobalShortcuts`) executes it. The `?` overlay is rendered from the same table, so
the help cannot drift from the keys.
- **Bindings follow Gmail** where Gmail has one: `j/k`, `Enter`/`o`, `u`, `x`, `* a`,
  `* n`, `e`, `#` (plus `Delete`), `s`, `Shift+U`, `Shift+I`, `b`, `c`, `r`, `a`, `f`,
  `/`, `?`, and `g i/s/t/d/b/a/c` (inbox, starred, sent, drafts, snoozed, archive,
  contacts). Scheduled, which Gmail lacks a key for, is `g l` ("later"). Cmd/Ctrl+K keeps
  opening the search overlay.
- **Targets:** a conversation action applies to the multi-selection when there is one,
  else to the open conversation (sent to `EmailView` as a pane command, so the toolbar's
  own handlers — including "go back to the list" after archive/delete/mark unread — run),
  else to the keyboard-cursor row of the full-width list. `b` opens the existing snooze
  picker (`SnoozeMenuButton.openSignal`) rather than a second picker.
- **When keys are not shortcuts:** in a text field, select or contenteditable (the TipTap
  composer, the chat input) only modifier combinations fire; nothing global fires while a
  modal is open (modals keep their own Escape/Enter), while an IME is composing, or when a
  focused button/link would take Enter. Composers bind Cmd/Ctrl+Enter themselves, in the
  capture phase, to their single send function (so undo send applies).
- **Setting:** Settings → Appearance → Keyboard shortcuts, on by default, SQLite pref
  `ui.keyboard_shortcuts_enabled`. Off means off for every binding, Cmd/Ctrl+K and the
  composer send key included.
**Context:** Parity audit item (High): only Cmd/Ctrl+K existed, as an ad-hoc effect.
**Rejected:** Per-component `keydown` listeners for each shortcut — the conflicts between
list, reading pane and composers would be resolved by mount order, and the help list would
be a second, hand-maintained copy. A keyboard library (react-hotkeys-hook, tinykeys) —
sequences, platform modifiers and the editable/modal rules are a few dozen lines that are
easier to test as pure functions. User-rebindable keys and a command palette — out of the
parity gap.

## 2026-10-01 — One-click unsubscribe contacts the sender only when the user asks

**Decision:** The reading pane offers "Unsubscribe" on any message whose stored
`List-Unsubscribe` header yields a usable method, preferred in this order: RFC 8058
one-click (an https URI plus `List-Unsubscribe-Post: List-Unsubscribe=One-Click`), then
`mailto:`, then an https page. Only `<…>`-bracketed `https:` and `mailto:` URIs count;
`http:`, `javascript:`, `file:`, custom schemes, credentials in the URL, multi-address
mailtos and overlong values are refused. The raw headers stay in the backend: the webview
receives a derived `UnsubscribeOption` (method, host or address, and the URL only for a
page it must open), and the unsubscribe command re-parses the stored headers rather than
trusting anything the frontend sends. One-click is a backend HTTPS POST with the body
`List-Unsubscribe=One-Click` (form-encoded), a plain `EmailOps` user agent, no cookie
store, 10 s connect / 20 s total timeouts and redirects followed only to https (at most
three); only a 2xx counts. A mailto is sent from the account that received the message,
with the list's subject and body and no "Sent with EmailOps" footer. A page is never fetched
by the backend: it opens in the system browser through the same https check as links in
mail. Every request is recorded per account and sender (`sender_unsubscribes`) so the pane
says "Unsubscribed"; the confirmation offers to block the sender afterwards.
**Context:** The privacy rule is "no external calls except to email providers or AI
providers the user chose". An unsubscribe request goes to the *sender's* server, a third
party, so it is allowed only as an explicit user action: the confirmation dialog names the
host it will contact (or the address it will email) and says it is the sender, not the
mail provider, before anything is sent — the same reasoning that lets a user open a link
from a message. Nothing is contacted automatically (no prefetch, no background
"unsubscribe from everything"), and the request carries nothing beyond what the list put
in its own URI.
**Rejected:** *Fetching the https page from the backend* — a page is meant for a person, may
need a click or a CAPTCHA, and fetching it silently confirms the address to trackers;
*sending the raw header to the webview to parse there* — breaks the rule that raw headers
never reach the webview, and moves URL validation to the less trusted side; *following
any redirect* — a one-click endpoint that bounces to plain http would leak the token in
the clear; *recording "unsubscribed" per List-Id* — not every list sends one, and the
sender address is what the user recognises in the banner.

## 2026-10-01 — Block sender files arrivals in the provider's spam folder; blocks are per account

**Decision:** "Block sender" stores the address (lowercased) per account in
`blocked_senders`. Every message that a sync stores in that account's inbox from a blocked
address is marked junk locally (the user's override, recorded first) and then filed in the
provider's Spam/Junk folder before the batch is announced — Gmail adds `SPAM` and drops
`INBOX`, Graph moves it to `junkemail`, IMAP moves it to the `\Junk` folder (or one named
like it) and re-keys it to the id the Spam pass uses. An IMAP server with no Junk folder
keeps the message in place, marked junk. Blocking offers to file the sender's existing
inbox and archived mail too (checkbox, on by default, up to 500 messages); unblocking —
from the message banner or Settings → Junk → Blocked senders — offers the inverse, bringing
their Spam back to the inbox and forgetting the block's junk mark (checkbox, on by
default). "Report junk" uses the same provider move (`EmailProvider::move_to_spam`), so it
now files on Gmail and Outlook as well, not only IMAP. The old ⋮ item that only hid the
sender's smart-filter chip is kept under its real name, "Hide from smart filters".
**Context:** Gmail's own block creates a server-side filter, which needs the
`gmail.settings.basic` scope; the app asks only for `gmail.modify` and should not widen
OAuth consent for this. Outlook's blocked-senders list is not in Graph's mail API, and IMAP
has no filters at all. Applying the block in the app on ingest works the same on all three,
and pushing the move to the provider keeps other clients (phone, webmail) in agreement —
a local-only hide would leave the mail sitting in every other inbox. Blocks are per
account like Gmail's: the same address can be wanted in one mailbox and not another.
This does not contradict the local-flag-only junk decision (2026-07-28): that one forbids
the *detector* from moving mail on its own; a block is an explicit, attributable user rule
with a reachable inverse.
**Rejected:** *A local-only hide (the junk detector's "keep out of the inbox" mode)* — other
clients disagree and the provider's own filter never learns; *creating a Gmail filter* —
needs a broader OAuth scope for one feature; *a global (all-accounts) block list* — differs
from both providers and makes a per-account unblock impossible to express; *deleting
blocked mail* — irreversible, and a block entered by mistake would destroy mail.

## 2026-10-01 — New-mail desktop notifications: sync-side planner, no click-to-open

**Decision:** New-mail notifications are decided in the backend at the end of each
sync (`services::mail_notifications::plan_new_mail_notifications`), after junk scoring
and the blocked-sender hook, so junk and blocked mail never notify. Only mail from the
sync's incremental pass qualifies — never an account's first sync, a backfill slice,
mail older than the inbox watermark, mail already read elsewhere, mail sent by the
user, promotions, or anything outside the inbox. 1–3 messages notify one by one; more
become one "N new messages in <account>" summary, at most one per account per minute.
Content defaults to sender + subject (Gmail/Outlook default), never the body; "Hide
content" and a locked app (main password not yet entered this session) show only the
account. Settings → Notifications holds the master switch, per-account switches, the
content option and "only when EmailOps is not focused" (all default on, SQLite prefs).
A snoozed conversation coming back notifies behind the same switches. Notifications go
through a `Notifier` trait seam (`services::notifier`) so tests never hit the OS.
**Context:** Gmail/Outlook parity (docs/COMPETITOR-PARITY.md). The notification plugin
was already wired for meeting reminders.
**Rejected:** Click-to-open the conversation — `tauri-plugin-notification` delivers no
click events on desktop (same limit recorded for meeting reminders); a click only
focuses the app, and the notification carries its thread so a future plugin can open
it without a planner change. No workaround (custom native notification code) was built.
A dock/taskbar unread badge was skipped: there is no unread-inbox count query to feed
it yet, and it is optional for parity. Deciding in the frontend on `sync-progress`
events was rejected: the webview may be hidden or locked, and it cannot see junk
verdicts or the blocked-sender filing reliably.

## 2026-10-01 — Dialogs keep the dark chrome; the app does not follow the OS theme

**Decision:** The app keeps one fixed two-tone look: dark chrome — sidebar, Output bar,
Settings and every dialog built on `Modal` (sender dialogs, shortcut help, account and
lens dialogs, onboarding) — around light content — the list, the reading pane and the
composers. Editors that show what a recipient will see (the composer, the signature
editor in Settings) stay white inside dark chrome, as a page preview. No surface follows
the OS appearance: Tailwind's `dark:` variant, which v4 drives from
`prefers-color-scheme`, is not used anywhere (`src/lib/theme/noOsDarkMode.test.ts`).
**Context:** The competitor-parity demo recording read the dark Settings, Block sender and
shortcut-help dialogs next to the light list and composer as a theme mismatch. It is not
an OS-media-query leak: there are no `dark:` classes in `src/`, and the dark dialogs come
from `Modal`'s deliberate dark scaffold (on `main` long before this branch), which every
dialog shares. Turning every dialog light means restyling `Modal` plus every form inside
it (Settings alone spans some twenty tab components with dark-surface classes), which is
the "cross-cutting restyle" that `docs/COMPETITOR-PARITY.md` scopes as its own branch
together with a real dark mode.
**Rejected:** Restyling only `Modal` light — the dark-surface form controls inside it
(`Select` variant `dark`, gray-100 text, `#333` inputs) would sit unreadable on white.
`darkMode: 'class'` / an `@custom-variant dark` — a no-op today, since nothing uses the
variant; the guard test states the invariant more directly.

## 2026-10-01 — The demo DB's schema comes from the checkout's own migrations

**Decision:** `scripts/generate_demo_db.py` builds the demo DB schema by running the app's
migration runner on a scratch data dir (the `init_db` cargo example, `Database::new`)
and copying the result in with SQLite's backup API. The developer's production DB is read
for schema only when `--prod-db PATH` asks for it. `scripts/ensure_demo_db.sh` rebuilds
the demo DB when a migration file is newer than it or its schema version is behind the
newest `V*.sql`.
**Context:** The generator used to copy `sqlite_master` and `refinery_schema_history`
from the production DB, which lags behind any branch that adds a migration; every
verification run on such a branch needed the schema patched by hand.
**Rejected:** Applying `src-tauri/migrations/*.sql` from Python — V001 creates vec0 tables
(needs sqlite-vec loaded in Python), and refinery verifies a checksum of every applied
migration on open, so Python would have to re-implement refinery's hashing. The CLI's
`doctor` — read-only by design, and its bootstrap touches the keychain.

## 2026-10-02 — Archive is live mail: in every search, filter and AI scope, out of only the Inbox view

**Decision:** `emails.mailbox = 'archive'` is live mail. Only the Inbox view (and what is
defined as "in the inbox": snooze, new-mail notifications, the archive action itself)
leaves it out. Search, the chat tools, embeddings, classification, lenses, contacts,
attachment rules and the junk detector's "not spam" training reach it, and so do the
sidebar's sender/domain/tag filters and their counts, which used to read
`mailbox IN ('inbox', 'sent')` and now share one list, `db::live_mailboxes_sql!()`
(`('inbox', 'sent', 'archive')`).
**Context:** Before the Archive mailbox, Gmail-archived mail stayed `inbox` locally, so
"inbox + sent" meant all live mail. Once Gmail mail without `INBOX` mapped to `archive`,
those queries silently dropped it: a sender filter missed the archived half of a
conversation, and an attachment rule ignored an archived invoice. Gmail and Outlook search
and labels include archived mail; only the Inbox excludes it.
**Rejected:** *`mailbox NOT IN ('spam', 'trash')` for the smart filters*: it would also
pull custom IMAP folders into them, a separate product change; the filters' covering
indexes (V008) serve an `IN` list either way. *Leaving archive out of the filters*: the
archived message of a thread would vanish from a sender filter the moment it was archived.

## 2026-10-02 — Overlays register themselves; no conversation shortcut runs behind one

**Decision:** Every dialog, drawer, lightbox, menu and popover calls `useOverlay(open)`
(`src/stores/overlayStore.ts`) while it is on screen. The app-wide key handler
(`useGlobalShortcuts`) treats a non-zero count, or any `[aria-modal="true"]` element, as
"an overlay owns the keyboard" and runs no shortcut at all; each overlay handles its own
Escape (the row ⋮ menu and the snooze picker gained one). The old `.fixed.inset-0` class
sniffing is gone. `overlayStore.test.ts` fails when a component draws a `fixed inset-0`
layer without registering, and `useGlobalShortcuts.overlays.test.tsx` presses `#`,
`Delete`, `e`, `s`, `b` and `j` with each real overlay open. Backspace is not bound on any
platform and stays that way.
**Context:** A contributor PR's Delete key trashed the conversation behind dialogs because
its "is a dialog open?" check only matched elements with a role. Probing ours with the real
overlays showed the same class of hole: the portalled row ⋮ menu and the snooze picker
popover (neither is `fixed inset-0`) let `#`, `Delete`, `e`, `s`, `b` and `j` act on the
open conversation or the cursor row.
**Rejected:** Adding more selectors to the DOM query — it only knows the overlays someone
remembered, and a styling change silently disables it. Stopping propagation inside each
overlay — every overlay would need it on every key, and a portalled menu is outside its
owner's DOM subtree.

## 2026-10-02 — Auto-advance: open the next conversation after it leaves the list

**Decision:** When the open conversation leaves the list because of archive, delete,
snooze (toolbar or `e`/`#`/`Delete`/`b`), a block that files it in Spam, or "Confirm
junk", the reading pane opens the next conversation in list order (the previous one at the
end of the list, back to the list when none is left). Settings → Appearance → "After
archiving or deleting" offers next (default) / previous / back to the list, SQLite pref
`ui.after_thread_leave`. Mark as unread always goes back to the list (staying would read
it again); bulk actions never advance; a conversation shown in a tab just closes. The
decision is the pure `planAdvance` (`src/lib/autoAdvance.ts`); `beginLeave`/`finishLeave`
(`src/stores/autoAdvanceStore.ts`) capture the screen when the action starts and do
nothing if the user navigated since (`selectionGeneration()`), so a slow action never
closes or replaces a conversation opened meanwhile. Undo brings the conversation back into
the list but reopens it only when nothing else is open.
**Context:** Parity with Gmail's Auto-advance and Outlook's "after moving or deleting an
item". Default "next" follows Outlook rather than Gmail (whose default is back to the
list): EmailOps' split layout already shows a reading pane, and triaging a queue with `e`
or `#` without a click between items is the point of the shortcuts. "Confirm junk" used to
`await` the provider and then clear the selection, which closed whatever the user had
opened in the meantime.
**Rejected:** Gmail's default (back to the list) — an extra click or `Enter` per message
during triage. Reopening the conversation on undo while another is open — it would yank
the user away from what they are reading. Advancing after bulk actions — there is no
single "current" conversation to step from.

## 2026-10-02 — Signature images: PNG, JPEG, GIF or WebP, ≤ 200 KB and ≤ 1200 px, checked twice

**Decision:** Settings → Signatures has an "Add image" button (logo or handwritten
signature). The frontend (`src/lib/signatureImage.ts`, a pure planner plus an executor with
the FileReader/canvas work injected) accepts only PNG, JPEG, GIF and WebP, refuses source
files over 10 MB, and redraws an image wider than 600 px or heavier than 200 KB at most
600 px wide (JPEG stays JPEG; anything else becomes PNG to keep transparency); a result
still over 200 KB is refused. Small images are inserted untouched, so a GIF keeps its
animation. The backend re-checks every `data:` URL of a signature on save
(`services/signatures.rs::check_signature_image`): allowed MIME type, `;base64` with the
base64 alphabet only, decoded ≤ 200 KB, magic bytes matching the declared type, and width
≤ 1200 px read from the image header (room for a pasted high-DPI logo, which does not go
through the upload path). A failing image refuses the save with the reason; remote
`https:` and `cid:` images are not embedded bytes and are not checked. The 512 KB total
cap stays.
**Context:** A contributor PR added signature images without validation; an SVG data URL
can carry script and external references, and an unbounded image rides along with every
message the account sends.
**Rejected:** Silently stripping bad images on save — the user would not learn why their
logo vanished. Allowing SVG and sanitizing it — another sanitizer to maintain for a
format many mail clients refuse anyway. A Rust image crate to decode and re-encode —
header parsing is enough to check width, and resizing belongs in the webview where the
user picks the file.

## 2026-10-02 — The text/plain part marks a closing signature with "-- "

**Decision:** When an outgoing message's signature block (`data-emailops-signature`)
closes the message, the text/plain alternative carries the RFC 3676 separator line `-- `
(dash, dash, space) before the signature text, so other clients can fold or strip it. The
HTML part is unchanged (no visible separator). The plain text is derived in the frontend
(`prepareOutgoingHtml` → `htmlToPlainText(html, { signatureDelimiter: true })`), where the
marker still exists; the send sanitizer drops it later. No separator when content follows
the block — a forward's quoted message sits below the signature, and clients would fold the
forwarded message as signature — nor when the user typed a `--` line into the signature.
This amends the 2026-10-01 signature entry's "no `-- ` separator is forced", which still
holds for the HTML.
**Context:** Contributor PR review; Thunderbird, mutt and many list archives rely on the
separator in plain text.
**Rejected:** Deriving the plain text in the backend — the marker is gone after
sanitizing, and the text is already produced by the composer. A separator in the HTML too
— Gmail and Outlook show none, and it would look like stray dashes.

## 2026-10-02 — AI drafts leave the sign-off to the account signature when one applies

**Decision:** The backend decides, per draft, whether the model may sign: a pure planner
(`plan_sign_off` in `services/emails/drafts.rs`) returns "app signature" when the sending
account has a non-empty signature that applies to the draft's kind (`use_for_new` for a new
message, `use_for_replies` for a reply or a forward), and "unchanged" otherwise. In the
first case one rule is added to the per-draft part of the prompt: a short closing line is
fine, the sender's name (the account's display name, written out: with only "the sender's
name" the 9B model still signed one English reply in three), a title, contact details or a
"[Your name]" placeholder are not, because the app adds the signature. In the second case the prompt is byte-for-byte what it was.
Both draft paths (the composer's "Generate draft" and the chat's `generate_email_draft`)
go through the same service, so they get the same decision. The rule lives where the
`{instructions}` placeholder is (the reply prompt's cached prefix is identical with and
without it — a test pins this); a custom reply template without `{instructions}` gets the
rule appended at its end so it is not dropped. A chat draft (plain text, saved without a
composer) opens in a compose tab on its own draft row with the signature inserted once;
a draft a composer saved opens as it is.
**Context:** Signatures landed (2026-10-01 entry) and AI drafts still ended with
"Best regards,\nName" above the inserted signature, signing twice: the template's
"no signature" wording did not stop it (synthetic eval: 3/3 drafts signed on the demo
model). Idea from contributor PR #127; its review asked for the rule to stay out of the
cached prefix, for no change to accounts without a signature (a bare "Best regards," with
no name would be a regression) and for custom templates not to drop it silently.
Measured with `make eval-draft-cases` (synthetic, 3 drafts per case): drafts that sign
before → after — qwen3.5-9b 9/9 → 0/9, qwen3.5-4b 9/9 → 4/9 (new messages 0/3; replies
still sign most of the time on the 4B model, which follows the inbound message's sign-off
over the instruction). A deterministic strip of a trailing name line is the candidate
follow-up if the 4B replies matter; not done here.
**Rejected:** A user setting for the sign-off ("name / none / signature") — the account's
signature options already say whether the app signs, so a second switch could only
contradict them. Stripping the name from the generated text afterwards — names and
closings vary by language and the draft can legitimately end with a name (a P.S., a
mention). Putting the rule in the system/prefix part — it varies per account and kind and
would bust the KV-prefix cache.

## 2026-10-04 — Shared documents sync over email, sent automatically once the user consents

**Decision:** Users can edit documents and sheets together without any server: each
install keeps a Yjs CRDT (`yrs` in the backend, `yjs` in the webview) and changes travel
as an `.eodoc` JSON attachment on ordinary messages between the participants' own
accounts (`services/shared_docs`). Sharing a document (a dialog that names the recipients
and says changes will be mailed to them automatically) or accepting an invitation is the
consent; after it, pending changes are mailed in the background after a 2-minute pause in
editing, with no click per message. This is the first mail the app sends without a click
per message, and it is limited to exactly that: the document's participants, about that
document, only while it is active and consented. A failed send stays pending and is
retried on the next pass (a duplicate update is harmless to the CRDT), unlike the outbox,
which never retries. Only EmailOps users can edit; anyone else gets an invitation with a
readable copy. Update messages are marked read and archived; the invitation stays in the
inbox. No new OAuth scope: it uses `gmail.send` / `gmail.modify` (archive) as sending and
archiving already do. Messages are not end-to-end encrypted: they are as private as the
user's other mail, and acceptance of a change rests on the sender being a stored
participant, which a forged `From` can fake when the provider does not reject it.
**Context:** The developer wants Google Docs/Sheets-style collaboration with no cloud,
consistent with the privacy-first rule of no external calls beyond mail and AI providers.
A message is recognised by its attachment rather than an `X-` header because none of the
three send paths can set headers and the sync keeps only an allowlist. The update sent is
the diff against the least up-to-date recipient's known state vector, so a lost message is
made good by the next one.
**Rejected:** A relay or peer-to-peer server — a cloud dependency by another name. A
"Send changes" button per edit — safe but too clumsy for collaboration. Letting people
without EmailOps edit by replying — free-text replies cannot be merged reliably. A custom
`X-EmailOps-*` header — not settable on any provider today. End-to-end encryption in the
first version — needs key exchange between participants; left for later, the envelope is
versioned (`v`) so it can be added without breaking older messages.

## 2026-10-05 — EO Docs: personal folders, view-only history, title-and-text search

**Decision:** Shared documents are presented as "EO Docs" (the feature's name in every
language; code identifiers keep `shared_docs`). Folders are personal to each install
(V037 `shared_doc_folders`): they are never mailed, sharing stays per document, and
deleting a folder moves what it holds up one level rather than deleting documents. Each
change is kept as a version (`shared_doc_versions`: local edits by the same person within
5 minutes are one version, each change that arrives by email is its own, 200 kept per
document) and shown in a right-hand history panel; a version can be viewed read-only,
not restored. Search covers titles and text (a sheet's cell values) through an FTS5
index refreshed on every content change.
**Context:** The developer asked for Google Docs-style folders, a change history and
search. Shared folders would need per-folder membership travelling by email; a restore
would have to be mailed as a new change to everyone.
**Rejected:** Shared folders — membership and moves between shared folders over email,
for organisation each person can do alone. Restoring a version — left out on request;
viewing covers looking back. Diff highlighting between versions — more work than the
view-only need justifies today. Title-only search — the text is what people remember.

## 2026-10-05 — "My organization" is the account's own domain, unless it is a free provider

**Decision:** An account's organization is the domain of its address
(`services::contacts::organization_domain`), except when that domain is a free personal
provider (`util::email_addr::PERSONAL_EMAIL_DOMAINS`: gmail.com, outlook.com…), in which
case the account has none. Contacts gets a "My organization" tab listing the people on
that domain (only shown when there is one), and the EO Docs share dialog suggests them
first: before anything is typed, and ranked first among the matches while typing (the
existing `autocomplete_recipients` domain boost). People already sharing the document are
not suggested.
**Context:** The developer asked for colleagues to come first when sharing, inferring the
company from the domain.
**Rejected:** A configurable organization domain or list — not asked for, and the address
already says it. Treating free-provider domains as an organization — everyone on
gmail.com would become a "colleague".

## 2026-10-05 — Attaching an EO Doc to an email shares it with the email's recipients

**Decision:** The composer's attach button offers "From this computer" and "From EO Docs"
(only while EO Docs is on). Attaching an EO Doc shares it with the email's To and Cc, as
attaching from Drive does: the picker warns that EO Docs only works between EmailOps
users and asks for the same consent to automatic mail as sharing. The composer sends a
placeholder attachment (`application/vnd.emailops.doc-ref`, data = document id); the
send path (`deliver_new_email` / `deliver_reply`, shared by immediate sends, undo send and
scheduled send) swaps it for the document's envelope only when the email actually goes
out, then records the recipients as participants — so an undone or cancelled email shares
nothing. The envelope now says why it was sent (`purpose`: invitation, update, message);
only background `update` messages are archived on arrival, so a person's own email with a
document attached stays in the inbox.
**Context:** The developer asked to attach from local files or EO Docs, with a warning that
EO Docs is for EmailOps users only, and chose sharing over attaching a frozen copy.
**Rejected:** Attaching a frozen copy — not what was chosen. Sharing as a separate
invitation email next to the user's email — two messages for one action, and it would go
out even when the email is undone. Archiving any message whose document is already known
— it would file away emails people wrote.

## 2026-10-05 — EO Docs imports Word with mammoth (webview) and spreadsheets with calamine (backend)

**Decision:** "Import" in EO Docs, and "Open in EO Docs" on a .docx / .xlsx / .xlsm / .xls /
.ods email attachment, turn the file into EO Docs. Word goes through mammoth (BSD-2-Clause,
attributed in THIRD_PARTY_LICENSES.md) in the webview: its HTML is parsed with the doc
editor's own TipTap schema (`src/lib/docSchema.ts`: headings, marks, lists, links, tables,
images) and written into a new document as one Yjs update; images over ~1 MB of base64
are left out (the document travels by email) and the user is told how many. Spreadsheets
are read in the backend with calamine (MIT): one EO Docs sheet per non-empty tab, values
only, formulas as their stored result (or their formula text when the file stores none),
capped at 5,000 rows × 100 columns. Imported documents start unshared.
**Context:** The developer asked to import Word and Excel and approved these two libraries
after weighing BSD-2 against MIT.
**Rejected:** SheetJS — Apache-2.0 but no longer published on npm (CDN only), harder to
audit and update. Parsing spreadsheets in the webview — the backend keeps an untrusted
binary format out of the page. Live formulas — they need a calculation engine; the best
known one (HyperFormula) is GPLv3 or paid. A Rust .docx reader (`docx-rs`) — it gives the
structure but no HTML, so the conversion would have to be written by hand.

## 2026-10-05 — EO sheets: shared column widths, local filters, formulas evaluated on display

**Decision:** Column widths live in the shared `Y.Doc` (`colWidths` map), so everyone
sees the same layout. Column filters (first row as header, Excel-style value checklist)
are view state of the person filtering and are never written to the document. A cell
whose value starts with `=` is a formula stored as text and evaluated in the webview on
display (`src/lib/sheetFormula.ts`): `SUM`/`SUMA`, `AVERAGE`/`PROMEDIO`, `MIN`, `MAX`,
`COUNT`/`CONTAR` over ranges, with `#REF!`, `#NAME?`, `#DIV/0!` and `#CYCLE!` errors.
**Context:** The developer asked for column resizing, basic aggregation formulas
(starting with sum) and column filters. Widths are part of how a shared sheet reads;
a filter is a question one person is asking of it. Storing the formula text keeps the
CRDT the only source of truth: every peer computes the same result from the same cells.
Numbers are parsed leniently (European "1.234,56 €" and "$1,234.50") because pasted
Excel blocks arrive as display text.
**Rejected:** Shared filters (one person's filter would hide rows from everyone);
storing computed results next to the formula (two values that can disagree after a
merge); a formula library such as HyperFormula (a new dependency for five functions);
keeping Excel formulas on import (imports still bring values, see the entry above).

## 2026-10-05 — EO sheets: formulas follow inserted and deleted rows; undo is per person

**Decision:** Inserting or deleting a row or column rewrites every formula's A1
references in the same Yjs transaction, as a spreadsheet does: references past the
change move, a range spanning it grows or shrinks, a reference to a deleted cell becomes
`#REF!`. Undo and redo in sheets and documents use Yjs's `UndoManager`, which only
tracks this person's own transactions; changes merged from other people are never
undone. Both editors answer the toolbar, Cmd/Ctrl+Z, Cmd/Ctrl+Shift+Z (and Ctrl+Y in
sheets) and the native Edit menu, which reaches the webview as a
`historyUndo`/`historyRedo` input event. A row added while a filter is on stays in
view until the filters change.
**Context:** The developer reported that sums went stale after creating or deleting
rows, that undo/redo was missing, and that "Add row" seemed to do nothing with a filter
on (the new, empty row was filtered out).
**Rejected:** Storing references by row/column id (immune to concurrent inserts, but
every formula would need translating between ids and A1 text on each edit); the known
cost of the A1 rewrite is that two people inserting rows at the same time both rewrite
the same formula cell and the last write wins, which can leave a range off by one.

## 2026-10-05 — Deleting an EO Doc is local, with a tombstone; moving is drag-and-drop

**Decision:** Deleting a document (after a confirmation that says what happens to a
shared one) removes it from this install with its history and search entry, and records
its id in `shared_doc_tombstones` so later mail from the other participants is ignored
instead of reappearing as an invitation. Nothing is mailed about the deletion: the others
keep their copies and go on editing among themselves. Edits still pending in an open
editor are discarded, not mailed. Documents move between folders by dragging them onto a
folder or a breadcrumb entry, besides the "Move to" menu of an open document.
**Context:** The developer asked to move documents to other folders and to delete them
with confirmation.
**Rejected:** Deleting for everyone (no owner exists in a peer-to-peer document, and
mailing a delete would let any participant destroy the others' work); a soft-delete flag
on `shared_docs` (every list, search and flush query would need to filter it); a hard
delete without a tombstone (the next change from a peer would bring the document back as
an invitation).

## 2026-10-05 — EO Docs refuses changes whose sender fails DMARC; on by default, still experimental; PDF through the print dialog

**Decision:** An arriving `.eodoc` message is refused, before it touches any document,
when the receiving server's `Authentication-Results` (read with the junk detector's
`junk::auth::assess`, so only a verdict attributable to the account's own MTA counts)
says the sender's domain failed DMARC, or publishes no DMARC policy and the message
failed SPF without a valid DKIM signature. EO Docs is now on by default and keeps its
Experimental label; turning it off still stops all ingest and mail. "Export PDF" prints
the document alone through the system print dialog (`window.print()`, which Tauri routes
to the native webview print on macOS; capability `core:webview:allow-print`), where the
user picks "Save as PDF".
**Context:** The developer asked for sender verification, PDF export and the feature on
by default while it stays experimental. The analysis in `docs/EO-DOCS.md` listed a forged
`From` as the main security gap.
**Rejected:** Refusing on `softfail`, `neutral` or a missing header (much legitimate mail
lands there, and an IMAP account cannot attribute any verdict, which would make EO Docs
unusable on IMAP). A PDF library (jsPDF, printpdf) writing the file directly — a new
dependency for what the system dialog already does, and a second renderer to keep in step
with the editor. Known gap: on IMAP accounts no verdict can be attributed, so a forged
`From` there is still accepted.

## 2026-10-05 — EO Docs "Export PDF" writes the file with pdfmake instead of opening the print dialog

**Decision:** "Export PDF" builds the PDF in the webview with pdfmake (MIT, 0.3.11) from
the document's own structure — `editor.getJSON()` for a text document, the cells' shown
values for a sheet (`src/lib/docPdf.ts`) — and saves it to Downloads through the existing
`save_attachment_to_downloads` path, with the usual "Show in Finder" toast. pdfmake and its
fonts (~2 MB) load on first export only. Only images carried in the document (`data:`
URLs) are drawn; a remote image would be a request to someone else's server. This
replaces the print-dialog export of the entry above, and the `core:webview:allow-print`
capability is gone with it.
**Context:** The developer found that "Export PDF" opened the print dialog and wanted the
file directly. The app had no PDF generator (it only displays PDF attachments), so a
dependency was needed; the developer chose pdfmake.
**Rejected:** The webview's own PDF engine (WebKit `createPDF`, WebView2 `PrintToPdf`,
WebKitGTK) — no new library, but three native implementations, two only testable in CI.
jsPDF — lighter, but its layout (line wrapping, lists, tables) would have to be written by
hand.

## 2026-10-05 — EO sheets surface concurrent cell edits instead of dropping a value silently

**Decision:** When two people change the same cell before either saw the other's value,
the CRDT still keeps one value on every copy, but the dropped value is now shown: the cell
is marked, a banner names the dropped and the kept value with "bring back" / "keep"
buttons, a toast and log line fire when such a change arrives in an open sheet, and the
History panel lists every such cell, settled or not. Detection reads the document itself
(`src/lib/sheetConflicts.ts`): a map entry whose `origin` is not the entry it replaced was
written without seeing it. The choice is stored in the shared document
(`resolvedConflicts`), so it is settled for everyone. This needs the replaced values kept:
the editor's `Y.Doc` runs with `gc: false`, and the backend keeps storing merged updates
(`merge_updates_v1` / `diff_updates_v1`), never re-encoding the state through a collected
`yrs::Doc` — a Rust test guards that.
**Context:** The developer asked to mark the overwritten cells in the history and to warn
when changes arrive in a cell one just edited, after the explanation that a concurrent cell
edit loses a value without notice.
**Rejected:** Detecting in the backend at merge time — `yrs` keeps an item's `origin`
crate-private, and the app would also have to know which client ids are "this person's";
reading the document in the webview covers sheets that were closed when the change arrived,
since the stored state still holds both values. Last-writer-wins by wall clock — clocks
differ between machines and the CRDT's own choice is already the same everywhere. Locking
cells — impossible without a server.

## 2026-10-05 — Bundled, translated release notes after an update; update toast returns every 24h

**Decision:** On the first launch after an update, a dialog shows the release notes of every
version since the last one seen, in the UI language. The notes ship inside the app as
`src/releaseNotes/<version>/{en,es,fr,de}.md`, written by the release skill next to the
CHANGELOG section; a vitest guard fails while the `package.json` version lacks a file in any
language. The `release_notes_seen_version` pref records the version once the dialog is
closed; a fresh install (onboarding wizard showing) records it silently, and an upgrade from
a build that never recorded one shows only the running version's notes. The update toast is
no longer once per version: it shows at startup and on the backend event until the user
updates, and closing it (X or Download) snoozes it for 24 hours
(`app_update_dismissed_version` / `app_update_dismissed_at` prefs, re-checked hourly). A
release newer than the dismissed one shows at once. This supersedes the "once per version"
part of the 2026-07-24 update-notification entry; the backend check is unchanged.
**Context:** The developer asked for the notes in the user's language on first open after
updating, and for a closed update notification to come back every 24h until they update.
**Rejected:** Fetching the GitHub release body — English only, needs the network at startup,
and the dialog would trust remote markdown. Translating the CHANGELOG at runtime with the
local model — slow on first launch and not reviewable. Moving the snooze into the backend
check — it fetches at most once per 24h, so a snooze measured there would drift up to 48h.

## 2026-10-06 — The macOS DMG layout comes from a tracked .DS_Store, not from Finder at build time

**Decision:** `make build-mac` runs the Tauri build with `CI=true`, so the bundler passes
`--skip-jenkins` and never sends AppleScript to Finder, then `scripts/dmg_apply_layout.sh`
copies `src-tauri/dmg/layout.DS_Store` (the window size and icon positions Finder wrote for
the 0.6.12 DMG) into the DMG before `notarize_mac_dmg.sh` signs and notarizes it.
`make verify-mac` fails a DMG without the layout.
**Context:** The developer wants the release skill to build the signed macOS release from a
background agent session with no one at the keyboard. macOS only lets an app the user granted
Automation send Apple events to Finder; a background job (parented by launchd) cannot be
granted it, so `bundle_dmg.sh` failed with `-1743` after notarizing the app.
**Rejected:** *Skipping the layout* (`CI=true` alone): the DMG opens with the icons unarranged
and no visual hint to drag the app to Applications. *A DMG tool such as `dmgbuild`*: a new
dependency to write the same 6 KB file. *Asking the developer to run the build from a
terminal*: keeps a person in the loop the change exists to remove.

## 2026-10-07 — A reply may answer another account's email; only RFC headers cross accounts

**Decision:** A reply's parent may belong to any account, not only the one it is sent from:
the outbox and the reply-draft save check only that the parent exists. What reaches the
sending account's provider is the parent's `Message-ID` and `References`; its provider ids
(Gmail `threadId`, Graph item id) go along only when the parent is in the sending mailbox
(`ReplyTarget::for_parent`). Without them Gmail threads by headers, and Outlook sends the
reply through `/sendMail`, unthreaded on the recipient's side.
**Context:** The composer offers a From selector on replies, but replying from another
account failed with "not found": the outbox and draft save refused the parent under the
2026-09 ownership rule, and the direct send handed the receiving mailbox's ids to the other
provider, which answered 404. The ownership rule still holds for commands that act on a
record (read, delete, tag, draft upsert); a reply only reads the parent's headers and subject.
**Rejected:** *Hiding the From selector on replies* — answering from another identity is a
normal need. *Looking up the parent's copy in the sending mailbox by `Message-ID`* — a copy
exists only when both accounts received it, and the headers already thread the reply.
*Raw MIME through Graph `/sendMail` to keep `In-Reply-To` on Outlook* — a second send path
for a single case.

## 2026-10-07 — Windows artifacts are signed in CI by jobs that never build

**Decision:** Windows release artifacts (installers and the binaries they install) are
code-signed in CI with a cloud-held certificate. Only dedicated signing jobs can use the
signing credential; they run behind owner approval and never build or run the project's
dependencies. Signing happens around a split build (compile, sign binaries, bundle, sign
installers) and the result is checked on Windows before publishing.
**Context:** Unsigned installers showed "Unknown publisher" in SmartScreen, and the CASA
assessment asks for signed Windows executables. The signing credential must not be
reachable from a job that runs the build's third-party dependency tree.
**Rejected:** Signing inside the build job (simpler, but exposes the credential to every
build dependency); certificate options whose validation requirements the project cannot
meet or that would show a third party as publisher; a hardware token (cannot be used from
hosted CI).

## 2026-10-07 — Linux downloads ship GPG-signed checksums

**Decision:** Each release publishes SHA256 checksums for the Linux packages, signed with
a dedicated project release key whose public half is committed in `docs/`. The private key
is available only to a signing job that never builds, under the same rule as Windows.
**Context:** The CASA assessment asks for an integrity-verified Linux download and accepts
GPG-signed checksums.
**Rejected:** An embedded AppImage signature, a signed package repository or a store
listing (more to build and maintain than the requirement needs); the maintainer's personal
key (a project key can be rotated without touching a personal identity).
