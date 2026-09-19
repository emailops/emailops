# Implementation-gap audit and remediation plan

**Audited commit:** `323d335` (branch `feature/oneshot-prefix-and-model-bench`)
**Date:** 19/09/2026
**Scope:** `src-tauri/src` (267 files, ~137k LOC) and `src` (294 files, ~55k LOC)

## Status

Phase 0 and the contained half of Phase 1 are **done**, on branch
`fix/critical-implementation-gaps` (off `main`). Every fix landed with a
regression test confirmed failing against the previous behaviour first — where
the fix changed a function's signature, by re-running the new tests against the
old algorithm.

| Finding | Commit | Tests |
|---|---|---|
| P0-1 IMAP marks every message `\Seen` on the server | `b1ed47c` | 3 new, 2 red first |
| P0-5 Lenses bypass the master AI switch | `0769ebb` | 3 new, 2 red first |
| P0-6 Previews ignore the remote-content policy | `7b9df3f` | 5 new, 3 red first |
| P0-2 + P1-13 `INSERT OR REPLACE` cascade + unindexed retry path | `be2b3b6` | 4 new, 2 re-verified red |
| P0-3 Gmail `$batch` correlated by position | `7442018` | 6 new, 2 re-verified red |
| Third `INSERT OR REPLACE` (optimistic Sent copy) | `1ee9e9f` | 1 new, red first |
| P0-7 + P0-9 + AI budget reset — all of class H | `d6aa120` | 11 new |

Gates on every commit: full Rust suite (2041 + 149), full frontend suite
(1126), clippy `-D warnings`, rustfmt, tsc, biome, gitleaks, jsx-literals,
i18n drift, `no-invoke-outside-api`.

Two further defects surfaced while fixing class H, both fixed in `d6aa120`:

- `get_ai_usage` / `reset_ai_usage` built an `AiService` — and therefore a
  provider — to reach a DB counter. Once the master AI switch grew its guard
  (`0769ebb`) that made reading your own spend fail exactly when a user who had
  hit their budget would look, and on llama.cpp it loaded a multi-GB model to
  read an integer.
- A latent deadlock in `get_usage_since`, found by the first test ever to cover
  it: it held the write connection while calling `get_config`, and
  `Database::reader()` falls back to that same mutex when no reader pool exists
  — which is the case for the in-memory test database.

Fixing P0-9 also revealed the finding understated it. The undo was missing two
things, not one: nothing listed excluded rows (so `include_lens_row` had no row
to name), *and* `remove_lens_exclusion` deliberately left
`lens_rows.status = 'excluded'`, which `get_lens_rows` filters out — so the
command would have restored nothing visible even once called.

**Still open, in plan order:** Phases 2–7 below.

---

## 1. Why this document exists

Several recent bug fixes were not really bugs. They were **features that landed
half-wired**: the capability existed, the tests passed, the UI looked right, and
one path — a second provider, a second entry point, a second field — was never
connected.

Three from the last two weeks:

| Fix | What was actually wrong |
|---|---|
| `500fd5e` / `6d83d79` | The Add-IMAP form had **one input for two values**. The SASL login was also stored as `accounts.email`, so the account synced fine and could never send — and every self-address comparison in the app silently stopped matching. |
| `6fa31e6` | `SyncScheduler` enumerated accounts **once at startup**. An account added mid-session got no watcher, no poll loop, no `sync_state` row, and stayed at zero emails forever. The same commit found `forget_account` had **no production call site at all**. |
| `aa9a5d2` | `send_draft` routed every draft through the *new message* path, so a reply saved as a draft shipped with no `In-Reply-To`, no `References`, no thread id. Gmail normalised the reply subject inside its own send path; IMAP did not. Outlook replied to the wrong resource id. |

None of these had a `TODO`. That is the point, and it is confirmed below: a
tree-wide sweep found **zero** `FIXME`, `HACK`, `XXX`, `todo!()` markers, one
`TODO` (inside a test string literal) and four `unimplemented!()` (all in a test
fake). **This codebase does not carry its unfinished work in comments.** Grepping
for markers finds nothing; counting call sites and comparing sibling paths finds
everything.

So this audit deliberately hunted *shapes*, not keywords.

### Method

Seven parallel sweeps, one per gap class, each briefed with the shipped fix that
exemplifies it. Findings marked ✅ below were **re-verified directly against the
source by the coordinating session** (and, for the `INSERT OR REPLACE` finding,
reproduced against a live SQLite). Unmarked findings carry the reporting sweep's
`path:line` anchors and have not been independently re-opened — treat those as
strong leads, not settled facts, and confirm before acting.

### Supporting numbers

- **156 `fix:` vs 88 `feat:`** commits since 01/06/2026 (443 total). Nearly two
  fixes per feature.
- Per-provider test coverage is lopsided: **IMAP 62** test fns, **Gmail 36**,
  **Outlook 13**. Zero `fix:` commits mention Outlook in three months — that is a
  usage and coverage signal, not a quality one.
- The last full verification run (`7aa542c`, 15/09) reported **3471 ok / 8 fail**,
  with 549 checks green under "Cuentas y sincronización" — and both the IMAP
  login bug and the sync-starvation bug shipped anyway. `make verify` runs against
  the demo DB (2 IMAP + 1 credential-less Gmail account), so real server variation
  and mid-session account creation are **structurally invisible** to it.
- `lefthook.yml` gates lint, types, format, i18n literals, the
  `no-invoke-outside-api` rule and secrets. **None of them can catch any gap class
  in this document.** §6 proposes the ones that can.

---

## 2. The taxonomy

Seven recurring shapes. Every finding in §3 is one of these.

| # | Shape | Tell | Exemplar fix |
|---|---|---|---|
| **A** | **Provider asymmetry** — logic inside one provider's impl instead of the shared service | `match provider`, a trait method with a default `unsupported` body | `aa9a5d2` (`Re:` in Gmail only) |
| **B** | **Field conflation** — one column serving two meanings | A sentinel that is also a legal value; a "default from" that runs both directions | `500fd5e` (login = address) |
| **C** | **Startup enumeration** — a per-entity collection built once at init | `for account in db.list_accounts()` inside `setup()` | `6fa31e6` (scheduler) |
| **D** | **Twin entry points** — two functions doing the same job, one guarded | `foo` and `foo_with_provider`, CLI vs Tauri command | `6d83d79` (CLI vs modal) |
| **E** | **Parse-then-discard** — data read on ingest, used for a derived value, dropped | A header parsed into a hash and never stored | `9a2e704` (`References`) |
| **F** | **Optimistic success** — the operation reports done without proving it | `let _ =` on the write, then `count += 1` | `8d903b1` (attachment rules) |
| **G** | **Built, never wired** — complete implementation, zero production callers | A `pub fn` with tests and no call site | `6fa31e6` (`forget_account`) |
| **H** | **One-way door** — a destructive or privacy-relevant action is wired; its inverse is built, registered, and unreachable | An `exclude`/`add`/`consume` with a call site whose `include`/`remove`/`reset` twin has none | — (found here, three instances) |

**Class H is the highest-value pattern in this audit.** Three of the top findings
are the same defect, not three separate ones: the destructive half of a pair
shipped and the undo did not. It should be one checklist item — *for every
irreversible user action, is its inverse reachable from the UI?* — rather than
three fixes.

---

## 3. Findings

### P0 — user data loss, corruption, or privacy leak

These change or expose the user's real mail. Nothing else ships before these.

---

#### P0-1 ✅ IMAP sync marks every downloaded message `\Seen` on the server — class A

`src-tauri/src/sync/imap_search.rs:100` and `:151` fetch with
`session.uid_fetch(set, "RFC822")`. RFC 3501 §6.4.5: `RFC822` is equivalent to
`BODY[]`, and **any non-`PEEK` body fetch implicitly sets `\Seen`**. `uid_store`
appears once in the whole file, only for `+FLAGS (\Deleted)` (`imap.rs:1466`).
Nothing restores the flag.

**Scenario.** A user adds an IMAP account with a 2020 sync floor. EmailOps
downloads 40,000 messages. Every one of them is now read in their phone, their
webmail and every other client. Unread counts everywhere go to zero. Gmail and
Graph reads never touch read state, so this is IMAP-only — and irreversible,
because the app has no record of what was unread before.

**Fix.** `"BODY.PEEK[]"` in both call sites, and adjust the response-key parsing
in `parse_search_response` (its tests at `imap_search.rs:433+` hard-code the
`RFC822` key). **Size: S. Blocking.**

---

#### P0-2 ✅ `INSERT OR REPLACE` cascade-deletes every dependent row — class F

`src-tauri/src/db/emails/crud.rs:90` and `:142`, `src-tauri/src/db/emails/batch.rs:23`
all use `INSERT OR REPLACE INTO emails`, and `PRAGMA foreign_keys = ON` is set on
every connection (`db/mod.rs:199,288,664`).

Reproduced against SQLite in this session:

```
PRAGMA foreign_keys=ON;  -- emails, email_tags (ON DELETE CASCADE), AFTER DELETE trigger
INSERT OR REPLACE INTO emails VALUES('e1','redownloaded');
  → tags_left=0        -- the FK CASCADE fires
  → trigger_fired=0    -- the AFTER DELETE trigger does not
```

`batch.rs:21` hand-deletes the stale FTS row precisely because the author knew
the delete trigger would not fire. **The cascade side was never handled.**

Dependents of `emails(id)` with `ON DELETE CASCADE` (V001, V018, V019):
`email_extraction_status`, `email_bodies`, `embedding_chunks`, `email_tags`,
`attachments`, `email_attachment_meta`, `chat_message_sources`, `lens_rows`,
`lens_exclusions`, `email_headers`, `email_junk`. Plus `ON DELETE SET NULL` on
`memory_facts.source_email_id` and `pending_tasks.source_email_id`.

Ordinary sync filters existing ids first (`sync.rs:502,1624`) so it is not hit
there. **The user-facing re-download hits it by construction**
(`redownload.rs:39`, `:114` call `insert_email` on a row that exists).

**Scenario.** The user re-downloads a message. They silently lose its AI
classification tags, its junk verdict *including a permanent `not_junk` user
override*, its lens rows *including hand-edited `overrides_json`*, the chat
citations linking it to past answers, and its embeddings — while `vec_emails`
rows survive as orphans (vec0 honours no FK). `is_deleted` and `pending_sync` are
absent from the column list, so a **soft-deleted email un-deletes itself**.

**Fix.** `INSERT … ON CONFLICT(id) DO UPDATE SET …` so the row updates in place
and no cascade fires. **Size: M. Blocking.**

---

#### P0-3 ✅ Gmail `$batch` results are correlated by position — class A

The request writes `Content-ID: <item{i}>` for each sub-request
(`sync/gmail.rs:1367`). `parse_batch_parts` (`gmail.rs:1778`) **never reads it
back**: it pushes results positionally and `continue`s past empty parts. The
consumer then does `for (idx, part) in initial_parts.into_iter().enumerate()` →
`final_results[idx]` (`gmail.rs:1681`). There is no `parts.len() == message_ids.len()`
check.

Outlook does it correctly — `parse_batch_response` (`outlook.rs:1358`) parses the
returned `id` into an explicit `index`.

**Scenario.** One sub-part is skipped or malformed. Every subsequent result
shifts by one: **the body of message B is written under the id of message A.**
Silent cross-contamination of mail content, which then feeds FTS, embeddings and
chat citations. Separately, `final_results[idx]` and `rate_limited[local_i]`
(`gmail.rs:1728`) can index out of bounds — a **panic in production Rust**, which
`src-tauri/CLAUDE.md` forbids.

**Fix.** Parse the response `Content-ID` and slot by it, mirroring Outlook. Add
the length assertion as a defensive second line. **Size: M. Blocking.**

---

#### P0-4 ✅ A failed download closes the mailbox backfill as "complete", forever — class F

`src-tauri/src/services/emails/sync.rs:2372-2379`:

```rust
_ => {
    // Cannot advance with the same cursor — ... every ref is either unknown
    // (download failed) or at/above the cursor. Mark done so we don't
    // infinite-loop. This is defensive; normal operation should always ...
    let _ = db.set_preference(&done_key, "1");
    break;
}
```

The comment names the failure itself: *"download failed"*. When
`batch_get_messages` fails for every chunk of a page, `outcome.min_timestamp` is
`None`, `get_min_timestamp_for_ids` is `None` (the rows do not exist), and this
arm marks the mailbox permanently done — **with no log**, and the `set_preference`
error discarded too. The `[UNAVAILABLE]` login throttle this repo already tracks
on WorkMail produces exactly this state.

**Scenario.** First backfill of Sent on a new IMAP account; the server throttles
one page. Sent is latched as "history complete". The user sees their Sent folder
truncated at an arbitrary date, everything looks healthy, and the only recovery
is `resync_mailbox_full` — which is itself unreachable (see P2-1).

**Fix.** Do not latch `done` when the page inserted nothing *and* downloads
failed; log `warn` and leave the cursor for the next sync. **Size: S. Blocking.**

---

#### P0-5 ✅ Lenses bypass the master AI switch — class D (privacy)

`commands/lenses.rs:165` (`run_lens`), `:279` (`reextract_lens_row`), `:319`
(`preview_lens_extraction`) call `AiService::load_provider(&db)` directly.

Fourteen other AI commands gate first — `chat.rs:142`, `search.rs:136,156,217,231`,
`classification.rs:31,94`, `memory.rs:239,345,459`, `translation.rs:58`,
`emails.rs:334,414`. A grep for `is_ai_enabled` across `services/lenses/`,
`commands/lenses.rs` and `services/ai.rs` returns **zero hits**;
`load_provider` (`ai.rs:340`) never consults it.

**Scenario.** The user turns AI off in Settings — the privacy switch. They open
Lenses and click Run. Extraction runs. With OpenRouter configured, **mail content
leaves the machine with the master privacy switch off.** That is precisely what
the switch exists to prevent.

**Fix.** Guard inside `AiService::load_provider`, which covers every current and
future caller, rather than at the three commands. **Size: S. Blocking.**

---

#### P0-6 ✅ `EmailPreviewById` ignores the remote-content policy — class D (privacy)

`src/components/shared/EmailPreviewById.tsx:48` calls `sanitizeEmailHtml`
(`src/lib/emailFormatting.ts:139`), which only sanitises inline `style` and
**never installs the `afterSanitizeAttributes` hook** that strips `src`/`poster`/
`srcset`. `EmailBody.tsx:97` correctly uses `sanitizeEmailHtmlFull`, resolving
`privacy.allow_remote_content` (default **off**) and the trusted-sender
allowlist first.

Three consumers render mail through the unprotected path:
`Tasks/TasksPanel.tsx:335`, `Memory/MemoryView.tsx:227`,
`Lenses/LensRowDrawer.tsx:54`.

**Scenario.** The user leaves "load remote content" off. They open the Tasks
panel and select a task extracted from a marketing email. The tracking pixel
fires; the sender gets a read receipt and an IP. The "load images?" banner never
appears on these surfaces, so there is no signal at all.

**Fix.** Extract the `EmailBody` policy block into a `useRemoteContentPolicy()`
hook and consume it in both. **Size: S. Blocking.**

---

#### P0-7 Trusted-sender grants are a one-way door — class H (privacy)

`addTrustedSender` is wired (`EmailBody.tsx:103`). `listTrustedSenders` and
`removeTrustedSender` — both backend commands registered and working
(`commands/trusted_senders.rs`), both wrapped in `api.ts:1049,1053` — have **zero
callers anywhere in `src/`**. Two independent sweeps reported this.

**Scenario.** One click permanently allows remote content from a sender. There is
no screen listing the grants and no way to revoke one. In a privacy-first client,
the irreversible direction is the one that got built.

**Fix.** A trusted-senders list with a remove action in Privacy settings; both
backend commands already exist. **Size: S. Blocking.**

---

#### P0-8 IMAP `UIDVALIDITY` is read off the wire and discarded — class E

`imap_search::select` (`imap_search.rs:77-88`) issues `SELECT "<mailbox>"` and
does `.map(|_| ())`, throwing away the untagged `OK [UIDVALIDITY n]`. The
`folders` table (V013) has no column for it; a repo-wide grep for
`uid_validity|UIDVALIDITY` returns nothing.

IMAP ids are `{account}::{UID}` (`imap.rs:1050,1096`).

**Scenario.** The server resets UIDVALIDITY — mailbox recreated, provider-side
migration. Every stored id now names a **different message**. `email_exists`
returns true so sync never re-downloads; opening a message shows another one;
a delete or a move acts on the wrong message. No detection, no self-heal.
Low probability, silent and destructive when it fires.

**Fix.** Parse it, persist it on `folders`, and on mismatch purge and re-ingest
that folder. **Size: M.** (Migration → release coupling.)

---

#### P0-9 ✅ Excluding a Lens row is irreversible — class H

`lib.rs:672,673` registers **both** `exclude_lens_row` and `include_lens_row`;
both are implemented (`commands/lenses.rs:153,158`) and both are wrapped
(`api.ts:1620,1624`). Exclude runs end to end — `api.ts:1620` →
`stores/lensStore.ts:329` → `Lenses/LensesView.tsx:325`. Include **stops dead at
the wrapper**: a tree-wide grep for `includeLensRow` / `include_lens_row` returns
only the `api.ts` definition, the command and its registration. No store action,
no component, no button.

**Scenario.** The user clicks "Exclude this row". The email is permanently removed
from that Lens — no undo, no "show excluded" toggle, no way back — while the
command that would restore it sits registered one line away from the one that
removed it.

**Fix.** An `includeRow` store action mirroring `excludeRow` (`lensStore.ts:326`)
plus a "Show excluded rows" toggle. **Size: S. Blocking.**

The third class-H instance is the **AI budget**: `BudgetExceeded` is genuinely
produced (`services/ai.rs:549`) and both `getAiUsage` and `resetAiUsage` are
registered and wrapped with zero callers — a user who sets a budget and hits it
cannot see their spend or reset the period. AI simply stops, with no recourse
short of editing SQLite. Fix all three together.

---

### P1 — the app silently stops doing its job

---

#### P1-1 The scheduler starts with zero accounts if one DB read fails — class F

`services/sync_scheduler.rs:112` — `db.list_accounts().unwrap_or_default()`. This
is the entire background-sync bootstrap. A failed read yields an empty list: no
IDLE watchers, no poll loops, no calendar or memory tickers, for the whole
process, with no log and no user-visible signal. Same at `:714` for calendar.
`:137` has the matching `if let Ok(status)` with no `else`, skipping the reset of
a `"syncing"` status left over from a previous crash.

This is `6fa31e6`'s failure mode applied to **every account at once**.
**Size: S.**

---

#### P1-2 ✅ A deduplicated sync failure emits no terminal event — classes C+F

`sync_scheduler.rs:423` gates *everything* behind `if !already_reported`,
including `emit_sync_error` at `:443`. `LAST_SYNC_ERROR` is retained across ticks
**only for `NeedsReauth`** (`:449`) — the one failure that persists. Meanwhile
`services/emails/sync.rs:186` emits `starting` on every tick regardless, and
`src/stores/accountStore.ts:123` only removes an account from `syncingAccountIds`
on `complete`/`error`, while `:348` early-returns if the id is already in the set.

**Scenario.** A token is revoked. Tick 1 reports correctly. From tick 2 on, every
tick emits `starting` and swallows the error: permanent spinner over that inbox,
**refresh button disabled, and the user's click is a no-op**. Only an app restart
escapes. It disables the exact control needed to fix the problem.

**Fix.** Move `emit_sync_error` outside the dedup; keep the dedup on the log line
and the `upsert_sync_status` write only. **Size: S.**

---

#### P1-3 ✅ `update_imap_credentials` never re-registers the IDLE watcher — class C

`sync_scheduler.rs:353` captures `get_imap_credentials` **once**, inside
`spawn_watcher`. `commands/accounts.rs:117-154` stores the new credentials and
returns — no `unwatch_account`/`watch_account`. Every other mutating account
command notifies the scheduler (`:36`, `:64`, `:77`, `:188`); the one command
whose entire purpose is replacing what the watcher captured does not.

**Scenario.** The mailbox password changes. The user opens Account Settings,
updates it, the dialog live-tests it and reports success. Push delivery stays
broken for the rest of the session while manual refresh works — because
`sync_account` rebuilds the provider from storage each run and the watcher does
not.

Second variant: if the keychain is unavailable at startup the watcher returns at
`:768` but `watch_account` already inserted its map entry at `:213`, so
`plan_watch_change` answers `AlreadyWatched` forever. **Size: S.**

---

#### P1-4 An aborted sync leaves `sync_state.status = 'syncing'` on disk — class F

`services/emails/sync.rs:547` and `:833` — `if take_sync_abort(..) { return Ok(()); }`
skips the epilogue at `:917` (terminal `complete` + `upsert_sync_status("idle")`).
`SyncStatusGuard` only repairs when `completed == false`, and `:214` sets
`completed = true` because the abort returns `Ok`.

**Scenario.** The user changes the sync window (`commands/accounts.rs:210` raises
the abort). The replacement run is enqueued with `SyncContention::Wait`; if *it*
times out, its own path (`sync.rs:114`) emits `complete` and returns `Ok` without
touching the DB status. The poll loop then does `if already_syncing { continue; }`
on every subsequent tick and **the account stops receiving mail until the next app
start**. **Size: S.**

---

#### P1-5 ✅ IMAP never reads `\Seen` — every message is stored unread — class E

`sync/imap.rs:647` hard-codes `is_read: false`; the only override is
`is_read = true` for the Sent folder (`imap.rs:1054`, `:1104`). `FLAGS` is never
fetched. Gmail derives it from the `UNREAD` label, Outlook from `isRead`.
`\Answered` and `\Flagged` are likewise unavailable, so "already replied" and
starred state have no local representation. Re-sync cannot repair it:
`emails_exist_batch` makes sync skip every stored id.

Paired with P0-1, the state is **exactly inverted**: everything unread locally,
everything read remotely. **Size: M** (fetch `(FLAGS RFC822)` + a flags-only
re-listing pass so stored rows track the server).

---

#### P1-6 ✅ Delete and mark-read are silently local-only on Outlook and IMAP — class A

`sync/provider.rs:22` — `provider_supports_mailbox_writes` is
`matches!(provider, "gmail")`. `mailbox_state.rs:66` returns `Ok(None)` for the
other two and the local write proceeds with **no log, no toast, no UI
affordance**; `commands/emails.rs:177-185` emits no log line at all on the delete
path. `delete_email` is a soft delete (`crud.rs:445`) and `emails_exist_batch`
prevents re-ingest, so the divergence is permanent in both directions.

Graph supports both (`PATCH /me/messages/{id}`, `POST .../move` to
`deleteditems`); IMAP supports both (`STORE +FLAGS \Seen`, move to Trash).

The module doc (`mailbox_state.rs:11`) says delete is provider-first *"so a
refused push leaves the message visible instead of diverging silently"* — for two
of three providers the divergence happens anyway, just without the refusal. The
frontend comment at `EmailActionsMenu.tsx:434` states *"Deleting now also removes
the message at the provider"*, which is false for two of three.

**Fix, staged.** (a) Immediately: one warning per non-Gmail account plus a line in
the delete confirmation. (b) Then: implement the write-back. **Size: S then L.**

---

#### P1-7 ✅ Trash gets none of the correction Spam gets — class A

`services/emails/sync.rs:1582` re-flags already-known **Sent** rows; `:1601`
re-files already-known **Spam** rows; `:1717` hides older copies of a re-keyed
Spam message. There is **no `ExtraMailbox::Trash` branch anywhere**, although the
enum defines it (`provider.rs:169`). `reconcile_spam_moves` (`sync.rs:1906`) is
spam-only.

**Scenario.** Gmail (stable ids): the user deletes a message in Gmail Web; the
Trash pass lists the same id, `existing_ids` contains it, it is dropped, and the
local row stays in the inbox forever. IMAP/Outlook (re-keyed ids): the message
arrives as a new row with `mailbox='trash'` while the old inbox row survives —
**the message appears twice**.

**Fix.** Extend the two `mailbox_name == Spam` branches to `Spam | Trash`; both
helpers already take the mailbox name. **Size: S.**

---

#### P1-8 ✅ `redownload_email` overwrites the row's location — class A

`services/emails/redownload.rs:38` does `db.insert_email(&updated_email)` with no
fix-up. `outlook.rs:1204` returns `mailbox: "inbox"`, `is_sent: false` with the
comment *"Caller (sync_folder) overrides per mailbox pass"* — and the redownload
caller is not `sync_folder`. Gmail derives mailbox from labels and IMAP from the
folder in the id, so **only Outlook** is affected here.

`redownload_empty_emails` (`:78`) loops over `get_emails_with_empty_body(account_id)`
— account-wide, not inbox-scoped.

**Scenario.** An Outlook user runs "re-download empty emails". Every Sent /
Junk / Deleted row with an empty body is rewritten as `mailbox='inbox'`,
`is_sent=false`. Their Sent view loses mail; their Inbox fills with deleted and
junk mail. Compounded by P0-2, which also wipes that message's tags and verdicts.
**Size: S.**

---

#### P1-9 Memory and task extraction mark failures as done and count them as successes — class F

`services/memory/extractor.rs:143-158` — `mark_memory_facts_extracted` runs in
**both** match arms and returns `Ok(true)` unconditionally; `:98` counts
`ok += 1` and logs `"Extracted memories for {ok}/{total}"`. Per-item failures go
to `debug`. `services/tasks/extractor.rs` mirrors it exactly.
`email_extraction_status` has **no status column** — it is a binary "seen"
marker, so a failed email is excluded from every future backfill
(`db/memory.rs:624`).

**Scenario.** The local model times out during a 200-email backfill; 40 fail. The
log says "200/200". Those 40 will never yield a fact or a task. The only recovery
is `reset_memory_extraction`, which reprocesses the entire account.

The correct pattern already exists in `services/lenses/runner.rs:64-73,111-124`.
**Size: M** (needs a status column → migration → release coupling).

---

#### P1-10 Sync always reports `success`, and drops mail after 3 retries — class F

`sync.rs:917-931` emits `complete` with `new_count/new_count` and logs
`"Synced {synced_count} new emails"` at `success`, with status `idle` and no
error — regardless of how many failed (`:597-616` records them). After
`MAX_RETRY_COUNT = 3`, `:808-822` logs `"Permanently skipping email {id}"` at
`warn` and does `let _ = db.remove_failed_email(...)`.

**Scenario.** 200 new emails, 40 fail transiently. The UI shows "Synced 160 new
emails" in green and a healthy account. After three syncs those 40 are gone from
`failed_emails` and from the mailbox, permanently. The user finds out by searching
for a message they know they received.

`services/emails/redownload.rs:171-185` already does this correctly
("N succeeded, M failed"). **Size: S.**

---

#### P1-11 ✅ The Send button enables on an empty body; clicking does nothing — class D

`src/components/ComposeModal.tsx:724` gates the button on `!bodyHtml.trim()`; the
handler at `:320` gates on `!plain`, where `plain` is the *plain text*. Tiptap's
`getHTML()` returns `"<p></p>"` for an empty document
(`shared/RichTextEditor.tsx:73`), which passes `.trim()` but yields empty plain
text.

**Scenario.** Fill To and Subject, type a word in the body, delete it. "Send"
lights up. Clicking does **nothing** — no send, no error, no spinner. It only
appears once the user has touched the editor, which is the common case.

Same bug in `EmailView/ComposeTabView.tsx:585` and `EmailView/ReplyCompose.tsx:534`.
**Size: S.** This is the most visible defect in the audit.

---

#### P1-12 V018 `email_headers` has no backfill, and the migration claims one exists — class E

`migrations/V018__email_headers.sql:11-13` states: *"Backfill of already-synced
mail runs separately and is resumable."* No such backfill exists — `junk`,
`tag_priority`, `send_as_name` and `imap_settings` are the only `backfill_*`
functions, and none re-fetches headers.

`signals::materialize` reads `get_email_headers_batch`, gets `None`, and
`verdict.rs:941` forces `Band::Unknown`.

**Scenario.** The user enables junk detection and runs the backfill. **The entire
historical mailbox comes back "unknown"** — no band, no `X-Spam-*` recall (which
V018 itself calls most of the achievable IMAP recall) — forever. Only deleting
and re-adding the account fixes it. **Size: M.**

---

#### P1-13 ✅ The retry path writes no FTS row, no headers, no attachment metadata — class D

`sync.rs:847` ingests retried emails through `Database::insert_email`
(`crud.rs:81`), which writes **only** `emails` + `email_bodies`.
`insert_emails_batch` (`batch.rs:21,55-66`) additionally deletes the stale FTS
row, calls `insert_email_headers_tx`, and inserts into `emails_fts`. The retry
path is the *first* insert for these rows, and `populate_fts_if_empty`
(`db/mod.rs:388`) only runs when `emails_fts` is entirely empty.

**Scenario.** An email that failed once and succeeded on retry is **invisible to
every keyword search and to the `search_emails` chat tool, forever**; its
attachments never appear; junk scores it `Unknown` permanently.

**Fix.** Have `insert_email` delegate to `insert_emails_batch(&[email])` — same
column list, plus FTS and headers. This also fixes P0-2 for free if the
`ON CONFLICT` change lands there. **Size: S.**

---

### P2 — incomplete functionality, wasted work, missing validation

Grouped; full anchors in the sweep reports. Detail on request.

#### Validation that lives in one entry point only — class D

The unifying defect. The repo already has the right pattern three times —
`validate_pref` (`commands/preferences.rs:17`), `validate_new_event`
(`services/calendar/create.rs:77`), `validate_folder_name`
(`sync/folder_plan.rs:359`): a pure function in the service, unit-tested, called
by every caller. These capabilities do not have one:

| Capability | Gap |
|---|---|
| ✅ `send_new_email` | `send.rs:349-361` (the CLI/test path) rejects empty recipients and CRLF in the subject. `send.rs:421` — the path `commands/emails.rs:224` actually calls — has **neither**. A CLI repro shows the bug as fixed. |
| Classification rules | `commands/classification.rs:129` → `services/classification.rs:1246`: no name check, no pattern check, **no taxonomy check on `priority`/`intent`/`topic`**. Off-taxonomy tags become invisible to every sidebar filter. `update_rule` takes the whole struct from the frontend. |
| Lens create/update/duplicate | `commands/lenses.rs:61-93` → `db/lenses.rs:96`: bare INSERT. Empty name, empty schema, whitespace-only prompt all accepted. Blanking a prompt bumps `prompt_version` and invalidates every extracted row. |
| `set_ai_config` | `commands/ai_config.rs:33`: no provider allowlist (contrast `commands/accounts.rs:15`, which validates), no budget range (a negative budget silently disables the spend guard), and `save_config` resets `ai_period_start` on **every** save — changing the chat model zeroes the monthly accounting window. |
| Memory / Tasks numeric settings | `services/memory/config.rs:207` writes raw and `:150` reads unclamped. A promote threshold of 5 (max is 1) stops memory promotion permanently, with no error. |

**Structural cause:** only **6 of ~40 dialogs in `src/components/` use a `<form>`
at all**. Every `required` / `min` / `max` / `type="email"` outside those six is
**decorative** — the browser never runs constraint validation without a submit.
Combined with backends that trust the frontend, several capabilities are validated
by nobody.

Secondary-button divergence (the original bug's exact shape, relocated):

- **AI Settings**: Test sends `provider + model + a hard-coded embedding model`
  (`ai_config.rs:212`); Save persists the user's chosen one. Test also falls back
  to the *stored* key when the box is blank, so clearing the key and testing
  passes. And Test is disabled on an empty model while **Save is not** — the app
  itself sets `config.model = ''` when the selected local model is deleted
  (`AiSettings.tsx:351`), so Save writes an empty `ai_model` and every later AI
  call fails far from the settings screen.
- **AddImapAccountModal**: `canTest` has no port term and no address-format term.
  Entering `alex` in the `type="email"` box gives a green "Connection succeeded",
  then "Add Account" rejects it.
- **"Run backfill"** in Memory/Tasks settings ignores the tuning fields the user
  just edited — it calls `start*Backfill(accountId)` and the backend re-reads
  persisted config.

#### Settings honoured by some pipelines only — class B

| Setting | Honoured by | Ignored by |
|---|---|---|
| ✅ AI recency cap (`ai_max_email_age_days` / `_count`) | classification, embeddings | **junk** reads `get_preference("ai_processing_min_timestamp")` — a key **nothing writes**; only the *method* `db::ai_processing_min_timestamp` exists. Always `0`. **memory** passes `None` outright. |
| `accounts.enabled` | sync, scheduler, search, filters, unified inbox, calendar | `services/embeddings.rs:354` iterates `list_accounts()` unfiltered; `regenerate_embeddings(None)` wipes and rebuilds disabled accounts too |
| `ai_output_language` | `resolve_ai_language` chain | `MemoryConfig` claims the same key, defaults it to `"English"`, and **rewrites it on every Memory settings save** — then the startup migration promotes the accident to an explicit `ai_output_language_v2` choice |
| junk `flagged_action = hide` | TagBoard (SQL) | Inbox (client-side, per page, so counts never match); filtered views exclude spam **always**, even with detection off |

#### Identity fragmentation — class B

✅ **Eight different predicates for "I sent this"** — from
`is_sent=1 OR mailbox='sent' OR LOWER(a.email)=LOWER(sender_email)`
(`crud.rs:46-50`) down to a bare address comparison
(`memory/extractor.rs:129`, `tasks/extractor.rs:165`, `agent_search/mod.rs:634`,
`cli/output.rs:501`). The app never persists the set of the user's own addresses:
`gmail.rs:55-81` **does** fetch `users/me/settings/sendAs` but keeps only the
display name and discards the alias list. No `+tag` normalisation anywhere.

With `memory_extract_from_self_only` / `task_extract_from_self_only` defaulting to
`true`, a user who replies from a send-as alias gets **zero memory facts and zero
tasks** from those messages, while the dashboard reports "100% analysed".

✅ **Duplicate accounts by case.** `db/accounts.rs:129` guards with `WHERE email = ?1`
and `V001__init.sql:12` declares `email TEXT NOT NULL UNIQUE` with **binary
collation**. `alex@example.de` and `Alex@Example.de` are two accounts on one
mailbox: double sync, double storage, and every message twice in the unified
inbox (dedup is `(account_id, thread_id)`).

#### Data captured and never consumed — class E

| Field | Status |
|---|---|
| `In-Reply-To` | IMAP parses it into the thread hash and drops it. **Identical shape to the `References` bug `9a2e704` fixed.** A mobile-client reply with `In-Reply-To` but no `References` splits the thread, unrepairably. |
| `Reply-To` | Captured, stored (V018), read back — consumed **only** by the junk divergence check. The reply path always addresses `sender_email` (`send.rs:180`, `:271`; `ReplyCompose.tsx:141`). Replying to a mailing list or a ticketing system goes to the wrong address, and the user believes they answered. |
| `Received-SPF`, `DKIM-Signature` | Captured, stored, **zero consumers**. `junk::auth::assess` reads only `Authentication-Results`, and `expected_authserv` returns `None` for anything not Gmail/Outlook — so self-hosted IMAP, exactly the population whose MTAs write SPF/DKIM without an `Authentication-Results` line, gets **no authentication signal at all**. |
| `first_received`, `to_raw`, `content_type`, `extra_json` (`x-priority`, `auto-submitted`, `x-originating-ip`) | Captured, stored, zero consumers. `x-originating-ip` is the sender's IP retained indefinitely with no purpose — a privacy cost against the module's own stated principle. |
| `List-Unsubscribe` / `-Post` | Captured, used as junk evidence only. No unsubscribe action anywhere in the app, though the graymail band already identifies exactly the mail one belongs on. |
| Gmail user labels, `STARRED`, `IMPORTANT`; Graph `categories`, `flag`, `importance`, `replyTo` | Not persisted (Graph: not even in `$select`). The `folders` sidebar is IMAP-only. |
| IMAP `INTERNALDATE` | Not fetched; the sender-claimed `Date:` is trusted. Gmail uses `internalDate`, Graph `receivedDateTime` — three providers, three meanings of "timestamp", and a forged Date can drag the backfill floor. |

#### Built and never wired — class G

- ✅ **Three registered commands with no `api.ts` wrapper and no other caller**:
  `start_redownload_empty_emails`, `start_resync_mailbox`,
  `reextract_email_attachments` (`lib.rs:499,502,583`). The third's own doc says
  it repairs a gap that is *"otherwise permanent"*. `start_resync_mailbox` is the
  only full-mailbox resync path — the recovery for P0-4.
- **26 `api.ts` wrappers with no caller.** Three are class H and were promoted to
  P0 (`includeLensRow`, `getAiUsage`, `resetAiUsage`). The rest that are missing
  features rather than leftovers: `rebuildFtsIndex` (the only FTS-corruption
  repair), `regenerateEmbeddingsBlocking`, `getPendingEmbeddingsCount`.
- **`isTagBoardRange`** (`src/lib/tagBoard.ts:49`) has zero references, while its
  two identically-shaped siblings `isTagBoardType` and `isTagBoardDensity` are both
  imported at `App.tsx:57` and used as `parse:` validators for persisted prefs at
  `:153` and `:159`. `TagBoardView.tsx:86` keeps the range in plain `useState`. So
  the Tag Board remembers your tag-type tab and your density across sessions and
  silently resets the date range to "All" every time. The validator was written
  for exactly this wiring; the wiring never happened.
- **Memory fact embeddings are computed on every sync and never read.**
  `embed_pending_facts` is live (`commands/memory.rs:291`, `sync.rs:1125`); the
  read half `hybrid_search_facts` (`memory/embeddings.rs:80`) has zero callers.
  The production reader (`memory/header.rs:59`) is FTS-only, and its comment
  claiming the tool dispatcher pays for the hybrid variant is false. Wasted
  embedding calls *and* keyword-only fact recall.
- **`agent_search`** — 669 lines, ungated at `services/mod.rs:2` so it ships in
  every binary, reachable only from an eval harness. No command, no frontend.
- **`classification-progress`** is emitted with a full payload; 18 of 19 backend
  events have a listener, this one has none. Classifying a large mailbox shows no
  progress while sync and embeddings both do.
- **`retag_personal_domains` / `rebuild_account_tag_type`** — a data migration
  reachable only from an `examples/` binary. DBs predating the vocabulary change
  keep stale tags forever.
- **`validate_ai_base_url`** — 8 passing security tests rejecting `file:`,
  `javascript:`, `data:`; zero call sites. `OLLAMA_HOST` is read unvalidated.
- **`lenses.max_body_chars`** — documented as "settable from the Settings UI";
  no writer exists. Pinned to 4000 forever.
- **`EMAILOPS_DEV_TOKENS`** — in both `.env.example` files, read by nothing.
- **Dead frontend**: `Lenses/LensScopeEditor.tsx` (255 lines) and
  `LensPromptEditor.tsx` (99) — `LensConfigModal.tsx:2` says outright it replaces
  them. `common/InlineError.tsx` — a shared error component with zero consumers,
  while **22 component files** under `Settings/`, `Dashboard/`, `Lenses/`,
  `Onboarding/` and `AccountSettingsDialog` hand-code the same dark-red
  `role=alert` bubble it exists to unify. That is a drift risk, not dead weight:
  adopt it rather than delete it. It also looks like the component built for the
  "error banners visible without scrolling" rule and never wired in.
- ✅ `accountStore.ts:183` — the global `isSyncing` field survived `6fa31e6` as
  **write-only** (six writers, zero readers). Live bait: one
  `useAccountStore(s => s.isSyncing)` reintroduces the starvation bug.
- `remove_imap_credentials_on_delete` (`services/accounts.rs:851`) — zero callers,
  a doc claiming `remove_account` calls it, and a body containing the exact
  `let _ =` discard that the regression test at `:1568` exists to forbid.

#### Missing indexes — class F

- ✅ `emails.message_id` is unindexed. `hide_other_copies_of_message`
  (`crud.rs:816`) filters on `(account_id, message_id)` and runs **once per newly
  ingested Spam message** — a full account scan each time.
- Six FK child columns with no index on the referencing column:
  `chat_message_sources.email_id`, `lens_rows.email_id`,
  `lens_exclusions.email_id`, `memory_facts.source_email_id`, `drafts.email_id`,
  `interaction_events.email_id`. SQLite scans each once per deleted parent row, so
  deleting an account or a folder stalls inside a write transaction holding the DB
  lock — and pays the same cost per `INSERT OR REPLACE` (P0-2).
- `(account_id, provider_draft_id)` on `drafts` is assumed unique by
  `upsert_provider_draft` (`db/drafts.rs:194`, select-then-insert, no transaction)
  but the index is **not** UNIQUE (V007). Two overlapping syncs create two local
  rows for one provider draft.

#### Retrieval and deletion disagree — class F

✅ `delete_email` is a soft delete on `is_deleted` (`crud.rs:445`) that does not
touch `mailbox`. `db/emails/search.rs:363` (FTS) filters `is_deleted = 0`;
`db/embeddings.rs:156` (`vec_search`) filters `mailbox NOT IN ('spam','trash')`
and **never `is_deleted`**; `chat/retrieval.rs:429` hydrates via
`get_emails_by_ids`, which does not filter either. Keyword search hides a deleted
email; chat still ranks it, quotes its body and cites it as a source. Its vectors
are never purged.

#### Other lifecycle gaps — class C

- `delete_lens` (`commands/lenses.rs:73`) does not stop an in-flight run: the
  runner keeps burning model time on a deleted lens and re-inserts orphan rows
  after the delete transaction.
- Memory/task backfill "running" state is a **process-global** `OnceLock<AtomicBool>`
  (`commands/memory.rs:24`) while the UI is per-account. Start on A, then on B:
  B returns `Ok`, does nothing, shows "running", and cancelling from B cancels A.
- `delete_local_model` removes the file and nothing else — the deleted model stays
  selected as `ai_model`, the runtime cache is not evicted, and no
  `ai-config-updated` fires.
- Five frontend listeners leak on unmount-during-`listen()`
  (`LensesView.tsx:78`, `EmailView.tsx:154`, `ComposeTabView.tsx:223`,
  `ComposeModal.tsx:126`, `TranslateComposeControl.tsx:59`). `LensesView` is worst:
  each leaked handler re-runs three DB round-trips per `app-log` line during a lens
  run. The correct pattern is in `useAccounts.ts:67`.
- Account deletion orphans ~10 families of account-scoped `user_preferences` keys
  (no FK on a flat KV table), and `forget_account` prunes three `AppCore` maps but
  not `LAST_SYNC_ERROR` (which also holds a `calendar:<id>` sibling key that
  `clear_sync_error_dedup` never removes) or `LAST_ON_DEMAND_PULL`.

### P3 — surface coverage and consistency

- **CLI blind spots** (agent cannot reproduce or verify): account
  remove/enable/rename/re-auth/test-connection, **send reply** (no surface at
  all), folders, smart filters, lenses, calendar writes, memory/tasks, AI provider
  config, app preferences, trusted senders.
- **CLI-only paths the app never runs**: `classify_email_by_id`,
  `score_email_by_id`, `install_overrides`. A bug reproduced through the CLI
  exercises a single-email path the shipped app never executes, and vice versa.
- **CLI `search` diverges on both arguments** from the UI: it always passes
  `categories = None` (so it returns hits the app would never show), and the
  command logs `AI: true` while `services/search.rs:171` has AI **disabled**
  (`let _requested_ai = use_ai; let use_ai = false;`). The log line is false, in
  the panel the user is told to check.
- **CLI `search` paging silently caps at 100** (`services/search.rs:528`) while
  `docs/cli.md:256` claims `totalHits` is the full match count. An agent measuring
  recall gets a wrong number with no error.
- **`--name` clap help contradicts the code and `docs/cli.md`**
  (`cli/accounts.rs:79` says it defaults to the username; it defaults to empty and
  readers fall back to the address).
- **i18n is clean** — 1,788 keys, 18 files, **0 missing** in de/es/fr. The
  `check-i18n-drift.mjs` gate works. Eight genuine untranslated leftovers, all
  cosmetic (`sidebar.smartFilters.title` = "Smart Filters" in all three, plus
  seven single keys). This is the one area that is *not* the half-wired shape.
- **Import / export** is exposed nowhere and implemented nowhere.
- **Forward** exists only on the unmerged PR #67.

---

## 4. Impact analysis

### By affected population

| Population | Findings | Net effect |
|---|---|---|
| **IMAP users** | P0-1, P0-8, P1-5, P1-6, P1-7, plus SPF/DKIM unused and `INTERNALDATE` untrusted | The worst-served provider despite having the most tests. Their real mailbox is **modified** by sync (`\Seen`), their read state is inverted, deletes never reach the server, and junk has no authentication signal. |
| **Outlook users** | P1-6, P1-7, P1-8, no folder discovery, no junk write-back, `Retry-After` clamped to 30 s with no rate-limit gate | Thinnest test coverage (13 fns) and zero `fix:` commits in three months. Filing mail into an Outlook folder leaves it in no EmailOps mailbox at all. |
| **Gmail users** | P0-3, P1-7 | Best served overall — but the only positional-batch corruption risk is theirs. |
| **Everyone** | P0-2, P0-4, P0-5, P0-6, P0-7, P1-1..4, P1-9..13 | |

### By kind of harm

1. **Modifies the user's real mailbox without being asked** — P0-1. The only
   finding that reaches outside the app and cannot be undone.
2. **Destroys local user work** — P0-2 (junk overrides, lens edits, tags,
   citations), P0-4 and P1-10 (mail history), P1-9 (extraction coverage).
3. **Corrupts content silently** — P0-3.
4. **Leaks data the privacy switch is supposed to hold** — P0-5, P0-6, P0-7.
5. **Traps the user in an irreversible action whose undo exists** — P0-7, P0-9,
   AI budget reset (class H). Three instances, one checklist item, one gate.
6. **Stops the app doing its job with no signal** — P1-1..4.
7. **Burns local model time and battery on work nobody reads** — memory fact
   embeddings, junk ignoring the recency cap, embeddings ignoring `enabled`.

### Release coupling

P0-8, P1-9 and the index work each need a migration (V024+), which per this
repo's history means released binaries at an older schema refuse to open the DB
until the next release ships. Group them into **one** migration, land it early in
the cycle, and do not split them across releases.

### Why the existing gates missed all of it

| Gate | Blind to |
|---|---|
| clippy / tsc / biome | Everything here — all of it type-checks and lints clean |
| `cargo test` | Both halves of a twin-entry-point pair are tested; the tests assert the *guarded* one |
| `make verify` (3471 checks) | Runs on the demo DB: no real IMAP server, no mid-session account creation, no Outlook account, no provider-side moves |
| `check-i18n-drift` | Works — and i18n is the one clean area, which is the proof that a gate closes its class |
| `no-invoke-outside-api` | Enforces the *direction* api.ts → invoke, not **coverage** in either direction |

---

## 5. Remediation plan

Ordered by risk retired per unit of work. Each phase is one branch, TDD per the
repo rules (failing test first), full `npx lefthook run pre-commit` before
hand-back, commit at every green checkpoint, no push without an explicit ask.

### Phase 0 — stop the bleeding (`fix/imap-seen-and-batch-integrity`)

The three findings that damage data. Nothing else until these land.

1. **P0-1** `BODY.PEEK[]` in both fetch sites + parser key update. Regression test
   asserting the fetch spec contains `PEEK`.
2. **P0-3** Slot Gmail `$batch` results by response `Content-ID`; add the length
   assertion. Regression test with a deliberately skipped part, asserting the
   remaining results keep their ids.
3. **P0-2** `INSERT … ON CONFLICT(id) DO UPDATE` in `insert_email` and
   `insert_emails_batch`. Regression test: insert an email, tag it, re-insert,
   assert the tag survives. Fold in **P1-13** by routing `insert_email` through
   the batch path (FTS + headers).

**Retires:** silent mailbox mutation, content cross-contamination, loss of tags /
junk overrides / lens edits / citations on every re-download.

### Phase 1 — privacy switch integrity (`fix/ai-and-remote-content-gating`)

4. **P0-5** Move the `is_ai_enabled` guard into `AiService::load_provider` so it
   covers every caller, present and future. Test: master switch off → `run_lens`
   returns `AiDisabled`.
5. **P0-6** `useRemoteContentPolicy()` hook; consume it in `EmailBody` and
   `EmailPreviewById`. Test: policy off → no `src` survives in the preview's HTML.
6. **Class H, all three at once** — for each, the command, the registration and
   the `api.ts` wrapper already exist; only the store action and the control are
   missing:
   - **P0-7** Trusted-senders list + remove action in Privacy settings.
   - **P0-9** `includeRow` store action + "Show excluded rows" toggle in Lenses.
   - AI budget: usage readout + reset button in AI settings.

**Retires:** mail content reaching a remote provider with privacy off; tracking
pixels firing on three surfaces; and three irreversible actions whose undo was
built and left one line short of reachable.

### Phase 2 — mail must not vanish (`fix/sync-loss-and-honest-reporting`)

7. **P0-4** Do not latch `done` on a page that inserted nothing after download
   failures; `warn` and keep the cursor.
8. **P1-10** Honest terminal line: `"Synced X of Y (Z failed)"` at `warn` when
   `Z > 0`; persist exhausted ids instead of dropping them. Copy
   `redownload.rs:171-185`.
9. **P1-1** Stop swallowing `list_accounts()` errors in the scheduler bootstrap.
10. **P1-2** Move `emit_sync_error` outside the dedup guard.
11. **P1-4** Give the abort path the same epilogue, or return a distinct `Aborted`
    outcome the guard treats as incomplete.
12. **P1-3** Re-register the IDLE watcher after `store_imap_credentials`; have the
    watcher reap its own map entry on early return.

**Retires:** permanent history truncation, silent mail loss, permanent spinner
with a dead refresh button, background sync stopping for a whole process.

### Phase 3 — the schema migration, batched (`feature/v024-integrity`)

One migration. Do not split.

13. **P1-9** `status` column on `email_extraction_status`; mark only on success;
    exclude failures from the tally (mirror `lenses/runner.rs`).
14. **P0-8** `uid_validity` on `folders`; parse it from `SELECT`; purge and
    re-ingest on mismatch.
15. **P1-12** Resumable header backfill (shape of `junk::backfill_account`, keyed
    on `emails LEFT JOIN email_headers WHERE h.email_id IS NULL`). If it is not
    built, **correct the V018 comment** and surface "headers unavailable for N
    older messages" in junk settings.
16. Indexes: `emails(account_id, message_id)` partial, plus the six FK child
    columns; make `drafts(account_id, provider_draft_id)` UNIQUE partial and wrap
    `upsert_provider_draft` in a transaction.

### Phase 4 — provider parity (`feature/provider-parity`)

17. **P1-6** (a) Warn once per non-Gmail account that delete and read-state stay
    local; fix the false frontend comment. (b) Implement `set_read_state` and
    `trash_message` for Graph and IMAP; widen
    `provider_supports_mailbox_writes`.
18. **P1-5** Fetch `(FLAGS RFC822)`; map `\Seen`; add a flags-only re-listing pass.
19. **P1-7** Extend the already-stored correction to `Spam | Trash`.
20. **P1-8** Preserve `mailbox` / `is_sent` in `redownload_email`.
21. Graph `list_folders` / `list_folder_messages`; Gmail labels as folder rows.
22. Unify retry policy through `http_retry::classify_attempt`: drop Outlook's
    30 s `Retry-After` clamp, give it Gmail's rate-limit gate, give IMAP
    `connect_sync` a retry with backoff.

### Phase 5 — validation planners (`refactor/shared-validators`)

23. One pure, unit-tested planner per capability, called by **every** entry point:
    `validate_outgoing(to, subject)`, `validate_rule(..)`,
    `validate_lens_definition(..)`, `validate_ai_config(..)`, clamps for the
    memory/task numeric settings.
24. **P1-11** Gate every Send button on the same `plainText.trim()` its handler
    uses (3 files).
25. Align every Test/Preview button with the Save it precedes (AI Settings,
    AddImapAccountModal, AccountSettingsDialog); make "Run backfill" use the edited
    values. `Onboarding/StepAiBackend.tsx` is the correct model.
26. Wrap the dialogs that own required inputs in a real `<form>`, or drop the
    decorative attributes so nobody mistakes them for enforcement.

### Phase 6 — wire or delete (`chore/wire-or-delete`)

Every item gets an explicit decision; nothing stays in the "exists but
unreachable" state.

27. **Wire:** `start_resync_mailbox` (the recovery for P0-4),
    `reextract_email_attachments`, `rebuildFtsIndex`, `isTagBoardRange` as the
    `parse:` validator for a persisted range pref, the `classification-progress`
    listener, `validate_ai_base_url` in `ollama_base_url()`, and
    `hybrid_search_facts` **or** delete the whole fact-embedding pass and correct
    the stale comment. (The three class-H items ship in Phase 1.)
28. **Delete:** `LensScopeEditor.tsx`, `LensPromptEditor.tsx`,
    `remove_imap_credentials_on_delete`, the write-only `accountStore.isSyncing`,
    `EMAILOPS_DEV_TOKENS` from both `.env.example`s, the dead `api.ts` wrappers and
    their commands, `sync_state.history_id` / `next_page_token` / `emails.raw_json`,
    `junkStore.isDeprioritized`, `connectivityStore.navigatorOnline`/`backendOnline`.
29. **Decide:** gate `agent_search` behind `feature = "eval"` or expose it; run
    `retag_personal_domains` from a versioned migration or delete it.
30. **Adopt `InlineError`** across the 22 files that hand-code its bubble — this
    is a refactor, not a deletion, and it is where the "errors visible without
    scrolling" rule gets enforced in one place instead of 22.
31. **Test-only selectors that production reimplements inline** — e.g.
    `LensesView.tsx:88` hand-rolls `selectActiveRunStatus`; same shape at
    `filterStore.ts:141,145` and `lensStore.ts:132,137,143`. The tested copy and
    the shipped copy can drift silently. Have production call the selector.

### Phase 7 — remaining P2

Identity unification (`is_self_authored` + an `account_addresses` table fed by
Gmail `sendAs`), case-insensitive account uniqueness, `Reply-To` in the reply
path, `In-Reply-To` persisted, SPF/DKIM fallback in `junk::auth::assess`,
`is_deleted` in `vec_search`, the settings-scope fixes (junk phantom key, memory
recency cap, `enabled` in embeddings, `MemoryConfig` releasing
`ai_output_language`), dashboard numerator/denominator, the lifecycle gaps, the
five leaking listeners, and the CLI surface gaps from P3.

---

## 6. Prevention — gates that close these classes

The i18n gate is the existence proof: **the one class with a gate is the one
class that is clean.** Each of these is a script under `scripts/`, called by a
thin `Makefile` target and wired into `lefthook.yml`.

| Gate | Closes | Shape |
|---|---|---|
| **`check-provider-parity`** | A | For each `EmailProvider` trait method, assert every provider either overrides it or appears in an explicit, reviewed `UNSUPPORTED` table. A new provider method fails the build until all three are decided. |
| **`check-command-coverage`** | D, G | Diff `lib.rs`'s `invoke_handler` list against `api.ts` exports **and** against `src/` call sites. A registered command with no wrapper, or a wrapper with no caller, fails — with an allowlist requiring a comment. |
| **`check-twin-entry-points`** | D | Flag any `pub fn foo` whose sibling `foo_with_provider` / `foo_blocking` performs validation it does not. Start as a report, promote to a gate. |
| **`check-inverse-reachable`** | H | For every registered command pair with inverse verbs (`add`/`remove`, `include`/`exclude`, `enable`/`disable`, `set`/`reset`), assert both halves have a `src/` call site or an explicit reviewed exemption. Catches all three class-H findings mechanically. Cheapest gate here and it closes the class with the worst consequence-to-effort ratio. |
| **`check-dead-pub-fns`** | G | Count non-test call sites for `pub fn` in `services/` and `sync/`; report items reachable only from tests, only from the CLI, or not at all. `forget_account` and the three orphan commands would all have been caught the day they landed. |
| **`check-migration-backfill`** | E | Every `ALTER TABLE … ADD COLUMN` must be paired with a backfill statement or an explicit `-- NO BACKFILL: <reason>` line. V018's false comment and V023's silent gap both fail. |
| **`check-fk-indexes`** | F | Every FK child column must have an index with it leftmost. |
| **Extend `make verify`** | C | Two accounts on the demo DB, one added *mid-run*, and a sweep step that toggles enable/disable and updates credentials. The sync-starvation bug was invisible to 549 green checks precisely because none of them created an account while another was working. |

Two smaller ones worth adding to `src-tauri/CLAUDE.md` as review rules:

- **A `let _ =` on a DB write is a review-blocking finding.** The census found 163
  `let _ =`, of which ~29 are DB writes; four of them are real findings here
  (`sync.rs:653`, `sync.rs:2378`, `junk/mod.rs:205,217,286`,
  `attachments.rs:617`). Event emissions and IMAP `logout()` teardown stay fine.
- **Every operation with a `starting` event must emit a terminal event on every
  path**, including early returns, aborts and dedup branches. Three findings here
  (P1-2, P1-4, the post-sync embedding failure) are the same missing epilogue.

---

## 7. Verified sound — do not re-audit

Checked specifically because they were likely sites for these shapes, and found
correct. Recorded so the next audit skips them.

- `resolve_imap_identity` — the `500fd5e`/`6d83d79` fix holds: `email: &str` is
  required, defaulting runs address → login only, both surfaces call it.
- ✅ V023 end to end: `references_header` → `row_to_email` → `ReplyTarget.references`
  → `reply_references`, with the RFC 5322 §3.6.4 ordering pinned by test.
- `Re:` normalisation now sits in the service ahead of every provider, and is
  idempotent. `ReplyTarget` names each provider's threading field explicitly, with
  wiremock tests on the Graph reply URL.
- MIME tree, footer handling, sent-copy creation (IMAP `APPEND` with `\Seen`),
  sent reconciliation, `locate_message`'s `Ok(None)` semantics, Outlook `$batch`
  slotting, pagination and `before`/`after` bounds for all three providers.
- `header_capture` order semantics (topmost `Authentication-Results`, bottom-most
  `Received`) with security-rationale tests; RFC 5322 folding unfolded before
  capture.
- Folder name and move-target validation; attachment path sandboxing;
  `validate_pref`; calendar event creation; task status validation; outgoing HTML
  sanitised server-side regardless of surface.
- `delete_account` / `delete_folder` / `hard_delete_email` — transactional and
  vec0-aware. `PRAGMA foreign_keys` enforced and asserted.
- Downgrade behaviour fails closed with an accurate, tested message.
- All 16 `AppError` variants have producers, `code()` mappings and translations.
  Every declared Cargo feature has real gate sites.
- `SyncStatusGuard`, `task_queue`'s checked `send`, OAuth token recovery, chat
  retrieval's logged vector fallback, `classification`'s honest terminal line,
  `lenses/runner`'s separated success/failure counts, `junkStore`'s exemplary
  optimistic rollback, `api.ts`'s clean error propagation (0 of 211 wrappers point
  at a non-existent command).
- ✅ `accounts.enabled` in the scheduler lifecycle; `chat.default_categories`
  unified across CLI, REPL, evals and command; calendar notification settings;
  the IMAP settings keychain mirror; `sender_domain` and `trusted_senders`
  lowercase normalisation; unified-inbox `(account_id, thread_id)` dedup.
