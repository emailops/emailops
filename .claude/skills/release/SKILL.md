---
name: release
description: Cut a new EmailOps release across all three platforms — version bump across all source-of-truth files, CHANGELOG, quality gates, signed + notarized macOS universal app + standalone CLI builds (local, manual publish) with a local install + launch smoke test, a doc-staleness check (app docs + public website) before tagging, commit, tag, (confirmation-gated) push, then the Linux/Windows CI build (triggered and watched, auto-published on success). The macOS GitHub release asset is never uploaded automatically; the skill prints the exact info to publish it manually — Linux/Windows assets attach to the same release automatically via CI. After the developer publishes the macOS DMGs, the skill regenerates the Homebrew cask from the release assets and pushes it to emailops/homebrew-tap (confirmation-gated).
argument-hint: <patch|minor|major|X.Y.Z>
disable-model-invocation: true
allowed-tools: Bash, Read, Edit, Write, Grep
---

# Release EmailOps

You are cutting a new release of EmailOps (a Tauri app shipping on macOS, Linux,
and Windows). Follow the phases below in order. Run each phase, report a short
status line, and **stop on the first failure** — never paper over a failing gate.

## Platforms at a glance

- **macOS**: built, signed, and notarized **locally** on the developer's
  machine (Phases 5-5b), uploaded to the GitHub release **by hand** (Phase 8).
  This is a permanent choice, not a stopgap — signing secrets never need to
  touch CI for this platform.
- **Linux + Windows**: built in CI (`.github/workflows/release.yml`), unsigned
  by convention, smoke-tested on the same runner, and **auto-published** to the
  same tagged release (Phase 7b). The skill triggers this and waits for it.
- Both share one version bump and one git tag — there is no separate release
  cycle per platform.

## Golden rule: when in doubt, ask

If you are ever unsure about *anything* — the target version, whether the
working tree is in a safe state, an ambiguous CHANGELOG entry, an unexpected
build failure, whether to push, missing tooling, conflicting state, etc. — **stop
and ask the user.** Do not guess, do not invent a version, do not improvise a
workaround. A release is irreversible once pushed; a clarifying question is
always cheaper than a bad tag.

## Argument

`$0` is the desired bump: `patch`, `minor`, `major`, or an explicit `X.Y.Z`.
If it is missing or you cannot confidently resolve it to a concrete version,
**ask the user** for the exact version before doing anything else.

## Phase 1 — Pre-flight checks

Verify all of the following. If any fails, stop and report (ask the user how to
proceed rather than fixing silently):

1. Current branch is `main` (`git rev-parse --abbrev-ref HEAD`).
2. Working tree is clean (`git status --porcelain` is empty).
3. Local `main` is in sync with `origin/main` (`git fetch` then compare
   `git rev-parse HEAD origin/main`).
4. `.env.signing` exists at the repo root — `make build-mac` requires it. If
   absent, stop and tell the user to create it from `.env.signing.example`.
5. Read the current version from `package.json` and compute the new version
   from `$0`. Confirm the new version is strictly greater than the current one.

State the resolved version (e.g. `0.5.0 → 0.6.0`) before continuing.

## Phase 2 — Version bump

Update the version in **all three** sources of truth (they must stay in
lockstep) plus the lockfile:

- `package.json` → `"version"`
- `src-tauri/tauri.conf.json` → `"version"`
- `src-tauri/Cargo.toml` → `version` (the `[package]` entry near the top)
- `src-tauri/Cargo.lock` → regenerate so the `emailops` package entry matches.
  Run `cargo update -p emailops --manifest-path src-tauri/Cargo.toml --precise <new-version>`
  if that works, otherwise a plain `cargo build --manifest-path src-tauri/Cargo.toml`
  to refresh the lock. Keeping the lockfile synced matters — CI runs `npm ci` /
  `cargo` and will fail on drift.

`src-tauri/tauri.intel.conf.json` only overrides `bundle.resources`; it inherits
the version, so do not edit it.

## Phase 3 — CHANGELOG.md

EmailOps follows Keep a Changelog + SemVer. In `CHANGELOG.md`:

1. Take the entries currently under `## [Unreleased]` and move them into a new
   section `## [X.Y.Z] — YYYY-MM-DD` (today's date), placed directly below
   `[Unreleased]`, preserving the existing subsection headings
   (`### Added`, `### Fixed`, etc.).
2. Reset `## [Unreleased]` to a single line: `No unreleased changes yet.`
3. **If `[Unreleased]` has no real entries** (only the placeholder line), do not
   invent release notes — **ask the user** what the headline changes are before
   tagging.

## Phase 4 — Quality gates

Run `make check` (lint + typecheck + Rust tests + frontend tests + clippy).
If anything fails, stop and report the failure. Do not auto-fix beyond obvious
formatting unless the user asks.

## Phase 5 — Signed + notarized universal build

Warn the user this step is slow (full release build + Apple notarization), then:

```bash
make build-mac && make verify-mac
```

This produces the universal (Apple-Silicon + Intel) signed/notarized bundle with
the embedded AI provider. Surface the `verify-mac` output (codesign,
architectures, spctl, stapler) so the user can confirm it passed. If verify
reports any problem, stop and ask the user.

The signed DMG lands under:
`src-tauri/target/universal-apple-darwin/release/bundle/dmg/`

Then stage a stable, versionless copy so the published release exposes a
permanent `releases/latest/download/EmailOps-macos.dmg` link:

```bash
make dist-mac
```

This copies the versioned bundle DMG to `release/EmailOps-macos.dmg`.

## Phase 5b — Standalone CLI companion build

EmailOps also ships a standalone `emailops-cli` for terminal / power users as a
**separate** download on the same GitHub release — it is **not** bundled inside
the `.app`. It shares the crate version bumped in Phase 2, so there is no extra
version edit. This build is also slow (universal cross-compile + Apple
notarization); warn the user, then:

```bash
make build-cli-mac && make verify-cli-mac
```

`build-cli-mac` produces a universal (Apple-Silicon + Intel) Developer-ID-signed
binary, wraps it in a `.dmg`, notarizes it, and staples the ticket (a bare
Mach-O cannot be stapled, so the container is what verifies offline).
`verify-cli-mac` asserts the binary is universal + signed and the `.dmg` is
stapled (`stapler validate` + `spctl`). If verify reports any problem, stop and
ask the user.

Then stage the stable, versionless copy:

```bash
make dist-cli-mac
```

This copies the notarized `.dmg` to `release/EmailOps-CLI-macos.dmg`, reachable
at `releases/latest/download/EmailOps-CLI-macos.dmg`. Requires the aarch64 +
x86_64 Rust targets that `make bootstrap-mac` installs.

## Phase 5c — Local install smoke test

Static verification (`verify-mac`) proves the bundle is signed/notarized
correctly, not that it actually launches and renders. Before committing/tagging,
install the freshly built app locally and confirm it runs:

1. If `/Applications/EmailOps.app` already exists, back it up rather than
   deleting it — `mv /Applications/EmailOps.app /Applications/EmailOps-<old-version>-backup.app` —
   so the previous version is recoverable if something goes wrong.
2. Mount the versionless DMG staged in Phase 5 and install the new build:

   ```bash
   hdiutil attach release/EmailOps-macos.dmg -nobrowse -mountpoint /tmp/emailops-dmg-mount
   ditto /tmp/emailops-dmg-mount/EmailOps.app /Applications/EmailOps.app
   hdiutil detach /tmp/emailops-dmg-mount
   ```

3. Confirm the installed build is actually the new version:
   `defaults read /Applications/EmailOps.app/Contents/Info.plist CFBundleShortVersionString`.
4. Launch it (`open /Applications/EmailOps.app`), wait a few seconds, then
   confirm the process really started — `pgrep -fl "/Applications/EmailOps.app/Contents/MacOS/emailops"`.
   A launch that silently fails to spawn is a real failure; do not treat
   `open` returning immediately as success.
5. **A keychain-access system dialog may appear** ("EmailOps wants to access
   key ... in your keychain"). This is expected: a freshly re-signed build
   gets a new code signature, which invalidates the previous keychain ACL
   grant for the stored credentials item, so macOS re-prompts. **Never enter
   the keychain password yourself** — ask the developer to click
   Allow/Always Allow and enter it, then continue once they confirm.
6. Take a screenshot **scoped to just the app's window, not the full
   screen** — a full-screen capture leaks whatever else is on the developer's
   desktop, and the app itself will be showing their real mailbox (personal
   data). Get the window bounds and capture just that region:

   ```bash
   osascript -e 'tell application "System Events" to tell process "emailops" to set frontmost to true'
   osascript -e 'tell application "System Events" to tell process "emailops" to {position of window 1, size of window 1}'
   # then, using the returned x, y, w, h:
   screencapture -x -R<x>,<y>,<w>,<h> <path>.png
   ```

   Read the image back before sending it, to confirm it actually shows the
   running app (not a blank/loading state) and nothing unexpected is in
   frame.
7. Share the screenshot with the developer as visual proof the signed bundle
   actually launches and renders — this is a stronger signal than the static
   `verify-mac` checks alone.
8. If anything looks wrong (crash, blank window, error banner), stop and
   report it — same rule as every other phase.

## Phase 5d — Doc staleness check (run every release, not just once)

Do this **before committing/tagging** — once a tag is pushed, you want docs and
the public website already caught up, not a follow-up commit chasing it. Check
whether anything user-facing or platform-specific has fallen behind what this
release actually ships, based on the new CHANGELOG section from Phase 3 (the
Phase 7b CI build hasn't run yet at this point, so judge "did this release add
a platform for the first time" from the CHANGELOG entries, not from actual
produced GitHub-release assets). Report findings as a checklist — **do not
silently auto-edit prose docs or website copy**; ask the developer to confirm
each change first, since this is user-facing/marketing content:

1. **`README.md` download section.** If this release's CHANGELOG entries
   introduce Linux/Windows installers for the first time, or the README still
   reads "on the roadmap" for a platform that has now shipped (this or a prior
   release), flag it and offer to update the download links.
2. **`ROADMAP.md`.** Check for entries describing work this release just
   completed (compare against the new CHANGELOG section) — flag any that now
   read as still-pending when they're done.
3. **`docs/*.md`.** Scan for platform-specific claims that may be stale given
   what changed (e.g. a doc that only describes a macOS-only install flow,
   when this release adds a Linux/Windows equivalent). Flag, don't rewrite.
4. **`docs/DECISIONS.md`.** Ask the developer whether anything in this release
   constitutes a durable decision worth logging there (that file is
   append-only and durable-decisions-only by convention — not every release
   needs an entry, but the skill should ask rather than assume).
5. **Public website** (`getemailops.com`, source at
   `/Users/gerodp/CTO/AI/Email/landingpage_cursor/emailops_web`, a separate
   Hugo repo/remote from this one, deployed via AWS Amplify on push to
   `main`). Check whether its download/pricing/feature copy needs updating for
   this release — new platform support, new download links, a changed feature
   set. That repo's own `AGENTS.md` tells agents to never commit/push there by
   default; that guardrail is overridden only when the developer explicitly
   asks in this session, same as any other confirmation-gated push elsewhere
   in this skill. **Do not push website changes yet even if the developer asks
   for them here** — hold them until after Phase 7b confirms the CI build for
   the platforms in question actually succeeded (no point publishing new
   download links for a build that just failed); push then, still
   confirmation-gated.

## Phase 6 — Commit + tag

Once gates and build pass:

```bash
git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock CHANGELOG.md
git commit -m "chore: release vX.Y.Z"
git tag vX.Y.Z
```

Do **not** add Claude/agent as author or co-author (repo convention). Use the
real new version in both the message and tag.

## Phase 7 — Push (confirmation-gated)

Pushing is a shared, irreversible action. **Always ask for explicit
confirmation before it**, even if the user kicked off the release.

Push branch + tag (only after the user confirms):

```bash
git push origin main
git push origin vX.Y.Z
```

**Do not upload the macOS DMGs yourself.** Even if `gh` is installed and
authenticated, never run `gh release create`/`gh release upload` for the macOS
assets as part of this skill. The developer uploads those manually — your job
for macOS ends at printing the exact info they need (see Phase 8). This does
**not** apply to Linux/Windows — those *are* auto-published by CI, deliberately
(Phase 7b), since they carry no signing secrets to protect.

## Phase 7b — Trigger and watch the Linux/Windows CI release build

Run this **after** the tag is pushed (Phase 7) — the workflow builds a specific
tag, so it must exist on `origin` first.

```bash
gh workflow run release.yml --ref main -f tag_name=vX.Y.Z
```

To validate a change to this workflow or the build scripts *without* publishing
anything (e.g. after editing `scripts/build_platform.sh` or this workflow
itself), add `-f dry_run=true` — the build/verify/smoke-test steps still run in
full, but the installers land as a downloadable workflow artifact instead of a
GitHub Release, and `tag_name` can be any existing branch/tag, not just a real
release tag.

Then poll until it finishes — do not block silently for a long time without
telling the user; there is currently no empirical timing for this build (it has
never been run to completion as of this skill's last update), so warn that it
may take a while:

```bash
gh run list --workflow=release.yml --limit 1 --json databaseId,status,conclusion,url
# then, once you have the run id:
gh run watch <run-id>
```

- **On failure**: stop and ask. Report which platform leg failed (Linux,
  Windows, or both) and the run URL. Do not proceed to Phase 8 implying the
  release is complete when one platform is missing — a release with only
  macOS assets (or only some platforms) is a legitimate outcome only if the
  developer explicitly accepts it after seeing the failure.
- **On success**: both `.deb`/`.AppImage` and `.msi`/setup `.exe` are already
  attached to the `vX.Y.Z` GitHub release (the workflow creates it if it
  doesn't exist yet, via `softprops/action-gh-release`'s upsert-by-tag
  behavior — safe to run before *or* after the developer manually uploads the
  macOS DMGs in Phase 8, since it only adds files, never removes or renames
  what's already there). Note in your status line that the smoke tests passed
  — this confirms the installed binary starts and resolves its shared
  libraries/DLLs on a clean machine, **not** that GPU offload works (CI
  runners have no GPU; that still needs an occasional real-hardware check).

Once Phase 7b's CI build succeeds, this is also the point to follow through on
any website updates queued back in Phase 5d — now that the platforms in
question are confirmed actually working, not just built.

## Phase 8 — Print the manual GitHub-release info (macOS only)

The macOS assets are always uploaded by the developer by hand — Linux/Windows
are not (Phase 7b already attached them, or created the release if it didn't
exist). Gather and print everything the developer needs for the macOS side, so
it is copy-paste ready. Verify each fact before printing it (don't assume). If
Phase 7b already created the release, `gh release create` below will fail
(release exists) — use `gh release upload` instead; check which applies with
`gh release view vX.Y.Z` first and print the correct command.

1. **Confirm the assets exist.** Stat each stable-named DMG and compute its
   SHA-256 so the developer can sanity-check the uploads. The release attaches
   **two** assets — the desktop app and the standalone CLI — each reachable by
   filename through a permanent latest-download link:
   - `EmailOps-macos.dmg` →
     `https://github.com/emailops/emailops/releases/latest/download/EmailOps-macos.dmg`
   - `EmailOps-CLI-macos.dmg` →
     `https://github.com/emailops/emailops/releases/latest/download/EmailOps-CLI-macos.dmg`

   ```bash
   ls -la release/EmailOps-macos.dmg release/EmailOps-CLI-macos.dmg
   shasum -a 256 release/EmailOps-macos.dmg release/EmailOps-CLI-macos.dmg
   ```

2. **Confirm the tag is on origin** (`git ls-remote --tags origin vX.Y.Z`) and
   note the repo (`git remote get-url origin`).

3. **Write the release notes to a temp file** from the new CHANGELOG section
   (e.g. `/tmp/emailops-vX.Y.Z-notes.md`) so the `--notes-file` path is real.

Then print, in your final message:

- **Repo, tag (+ commit it points at), and release title** (`vX.Y.Z`).
- **The two DMG asset paths**, `release/EmailOps-macos.dmg` (desktop app) and
  `release/EmailOps-CLI-macos.dmg` (standalone CLI). The filenames must stay
  exactly as-is — that is what makes the permanent latest-download links resolve
  to this build. Do not rename them per-version and do not attach versioned
  copies; GitHub serves the latest-download link by filename, so a versioned
  name would not be reachable through the permanent URL.
- **The raw release notes** (inline) plus the temp-file path.
- **Both ways to publish**, and let the developer pick. Which `gh` command
  applies depends on whether Phase 7b already created the release (check with
  `gh release view vX.Y.Z >/dev/null 2>&1` — exit 0 means it exists):
  - *gh CLI, release does not exist yet* (Phase 7b hasn't run or hasn't
    finished — this creates it):

    ```bash
    gh release create vX.Y.Z \
      --title "vX.Y.Z" \
      --notes-file /tmp/emailops-vX.Y.Z-notes.md \
      release/EmailOps-macos.dmg \
      release/EmailOps-CLI-macos.dmg
    ```

  - *gh CLI, release already exists* (Phase 7b already created it with the
    Linux/Windows assets — this just adds the macOS DMGs to it):

    ```bash
    gh release upload vX.Y.Z \
      release/EmailOps-macos.dmg \
      release/EmailOps-CLI-macos.dmg
    ```

    If `gh` is not installed, mention it (`brew install gh && gh auth login`).
  - *Web UI*: open `https://github.com/emailops/emailops/releases/new` (or
    `.../releases/edit/vX.Y.Z` if it already exists), choose the `vX.Y.Z` tag,
    set the title if new, paste the notes if new, drag in **both**
    `release/EmailOps-macos.dmg` and `release/EmailOps-CLI-macos.dmg` (keep the
    filenames as-is), keep "Set as the latest release" checked, and publish.

Tell the user that once they have published the release, the Homebrew cask
still needs updating (Phase 9) — offer to do it as soon as they confirm the
release is live.

## Phase 9 — Homebrew cask update (post-publish)

EmailOps is also distributed via the `emailops/homebrew-tap` cask. The cask is
generated **from the published release assets** (it pins the sha256 digests
GitHub computes for each asset), so this phase can only run **after** the
developer has published the GitHub release from Phase 8. Full background and
invariants: `homebrew/README.md`.

1. **Verify the release is live and digested.** Do not rely on the user's
   word alone — check:

   ```bash
   gh api repos/emailops/emailops/releases/tags/vX.Y.Z --jq '.assets[] | {name, digest}'
   ```

   Proceed only when `EmailOps-macos.dmg` is listed **with** a `sha256:` digest
   (GitHub computes it within ~a minute of upload; if `digest` is null, wait
   and retry).

2. **Regenerate the cask** in the main repo:

   ```bash
   make cask TAG=vX.Y.Z
   ```

   This rewrites `homebrew/Casks/emailops.rb`. If the script warns that
   `EmailOps-macos-intel.dmg` is missing it emits an arm64-only cask — that is
   the expected outcome of the standard flow above (which only uploads the
   universal + CLI DMGs). Only chase the Intel warning if the user wants
   Intel Homebrew support for this release (`make build-mac-intel` etc.).

3. **Lint it:** `brew style homebrew/Casks/emailops.rb` must report **exactly
   one** offense — `Homebrew/OSDependsOn` on the `depends_on macos:
   ">= :monterey"` line. That string form is deliberate (older Homebrew treats
   the symbol form as an exact-version match and refuses to install); see the
   header comment in `scripts/generate_cask.sh`. Any *other* offense is a real
   problem — stop and investigate.

4. **Copy it into the tap.** The tap clone lives at `../homebrew-tap`
   (sibling of this repo); if it is missing, clone it:
   `gh repo clone emailops/homebrew-tap ../homebrew-tap`.

   ```bash
   git -C ../homebrew-tap pull
   cp homebrew/Casks/emailops.rb ../homebrew-tap/Casks/emailops.rb
   git -C ../homebrew-tap add Casks/emailops.rb
   git -C ../homebrew-tap commit -m "emailops X.Y.Z"
   ```

5. **Push the tap (confirmation-gated).** Same rule as Phase 7 — ask before
   pushing:

   ```bash
   git -C ../homebrew-tap push origin main
   ```

   Users pick the new version up via `brew upgrade --cask emailops`.

6. **Commit the regenerated cask in the main repo too** (`homebrew/Casks/
   emailops.rb` is tracked here as the source of what was shipped):
   `git add homebrew/Casks/emailops.rb && git commit -m "chore: update Homebrew cask to vX.Y.Z"`,
   and include it when pushing `main` (ask first, as always).

**Invariant:** never replace a DMG asset on an already-published tag — the
cask pins its sha256, so swapping the file breaks every install of that
version. If an asset is bad, cut a new patch release instead.

## Done

Report: the new version, that gates/build/verify passed, the local install
smoke-test result (with screenshot), any doc-staleness findings from Phase 5d
(app docs and website) and whether the developer acted on them, the commit +
tag created, the push state (pushed or pending), the Linux/Windows CI result
(run URL, conclusion, smoke-test outcome per platform), the website push state
(updated + pushed, or held pending developer confirmation), that the macOS
GitHub release upload is left for the developer to do manually (with the info
from Phase 8 printed above), and the Homebrew cask state (updated + pushed to
the tap, or pending the release publish).
