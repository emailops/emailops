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
`[6]` on both prompts, and the developer's real mailbox still got `[1][2][3]` in bullet
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
before and after on the developer's mailbox (qwen3.5-4b-q8_0, n_ctx 15360), the same
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
