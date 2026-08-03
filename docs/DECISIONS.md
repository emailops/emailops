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
