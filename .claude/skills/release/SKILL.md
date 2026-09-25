---
name: release
description: Cut a new EmailOps release across all three platforms — full verification run first (`make verify-release`, its summary committed with the release), version bump across all source-of-truth files, CHANGELOG, quality gates, signed + notarized macOS universal app + standalone CLI builds (local) with a local install + launch smoke test, a doc-staleness check (app docs + public website) before tagging, commit, tag, push, a DRAFT GitHub release with the macOS DMGs attached, then the Linux/Windows CI build (triggered and watched, attaches its installers to the same draft). Only once every platform's binaries are on the draft does the skill ask the developer whether to publish it — publishing is never done without asking. After publishing, it regenerates the Homebrew cask from the release assets and pushes it to emailops/homebrew-tap.
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
  machine (Phases 5-5b), uploaded by the skill to the **draft** GitHub release
  (Phase 7). This is a permanent choice, not a stopgap — signing secrets never
  need to touch CI for this platform.
- **Linux + Windows**: built in CI (`.github/workflows/release.yml`), unsigned
  by convention, smoke-tested on the same runner, and attached to the same
  **draft** release (Phase 7b). The skill triggers this and waits for it.
- **The release is always created as a draft** and stays one until every
  platform's binaries are attached. Publishing it is the one step the skill
  never takes without asking (Phase 8).
- Both share one version bump and one git tag — there is no separate release
  cycle per platform.

## Golden rule: when in doubt, ask

If you are ever unsure about *anything* — the target version, whether the
working tree is in a safe state, an ambiguous CHANGELOG entry, an unexpected
build failure, missing tooling, conflicting state, etc. — **stop
and ask the user.** Do not guess, do not invent a version, do not improvise a
workaround. A published release is irreversible; a clarifying question is
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

## Phase 1b — Full verification

Run this right after the pre-flight checks and before touching any file, so it
verifies exactly the commit being released. Warn the user it takes about 15
minutes (every layer, judged chat evals on the local model), then:

```bash
make verify-release
```

It runs `make verify`, writes `docs/verification/<stamp>-<sha>.md` (totals,
per-feature counts, what changed since the previous full run, what fails) and
keeps the full HTML report local at
`src-tauri/reports/verify/current-full/informe.html`. It refuses to start while
another EmailOps process has the demo DB open (that instance's model holds the
GPU and every eval would fail out of memory): ask the user to close it, then
rerun.

Show the user the summary's "Since …" and "Failing" sections. A test that is
**newly failing** since the previous full run stops the release: triage it
(product bug, test drift, or model/judge flake — see the `maintain-verification`
skill) and ask the user whether to fix it first or release anyway. Tests that
were already failing in the previous run are reported, not blocking, unless the
user says otherwise. The summary file stays uncommitted until Phase 6.

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

Run this **before committing/tagging** — not as tidiness, as a hard dependency.
The site's sync script resolves its ref as `DOCS_REF` → newest `v*` tag →
`main`, and Amplify sets no `DOCS_REF`, so a doc fix committed *after* the tag
is cut cannot reach getemailops.com without either another release or someone
editing the Amplify environment. **Treat "the docs are behind" as a
tag-blocking finding, not a follow-up.**

Invoke the `maintain-docs` skill, passing it the new CHANGELOG section from
Phase 3. It runs the doc guards and the doc↔code contract tests, drives the
`docClaim()` cases against the real UI, walks the CHANGELOG entries for
features that shipped undocumented, fixes what it finds in all four languages,
and reports an HTML run case by case:

```bash
make docs-check ARGS="--with-app"
```

Two things that skill deliberately leaves to you:

- It **asks** before adding to `docs/DECISIONS.md` rather than assuming — that
  file is append-only and durable-decisions-only.
- It **never commits or pushes the website repo**
  (`/Users/gerodp/CTO/AI/Email/landingpage_cursor/emailops_web`, deployed via
  Amplify on push to `main`). It reports what that copy needs; you decide. Even
  when the developer asks for website changes here, hold them until Phase 7b
  confirms the CI build for those platforms actually succeeded — there is no
  point publishing download links for a build that just failed — then push,
  still confirmation-gated.

Review the uncommitted diff before moving on; it ships in the Phase 6 commit.

## Phase 6 — Commit + tag

Once gates and build pass:

```bash
git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock CHANGELOG.md \
  docs/verification/<stamp>-<sha>.md   # the summary written in Phase 1b
git commit -m "chore: release vX.Y.Z"
git tag vX.Y.Z
```

Do **not** add Claude/agent as author or co-author (repo convention). Use the
real new version in both the message and tag.

## Phase 7 — Push + draft GitHub release

Every gate before this point passed, so push without asking — the release is
not visible to users until it is published in Phase 8:

```bash
git push origin main
git push origin vX.Y.Z
```

Write the release notes from the new CHANGELOG section to a temp file (e.g.
`/tmp/emailops-vX.Y.Z-notes.md`), then create the release **as a draft** with
both macOS DMGs attached:

```bash
gh release create vX.Y.Z --draft --verify-tag \
  --title "vX.Y.Z" \
  --notes-file /tmp/emailops-vX.Y.Z-notes.md \
  release/EmailOps-macos.dmg \
  release/EmailOps-CLI-macos.dmg
```

- **Never omit `--draft`**, and never run `gh release create`/`edit` in a way
  that publishes before Phase 8's confirmation.
- If a release for the tag already exists (`gh release view vX.Y.Z` exits 0),
  check it is still a draft (`gh release view vX.Y.Z --json isDraft`) and use
  `gh release upload vX.Y.Z <dmgs>` instead. If it is already published, stop
  and ask — never replace assets on a published release (see Phase 9's
  invariant).
- The filenames must stay exactly `EmailOps-macos.dmg` and
  `EmailOps-CLI-macos.dmg` — that is what makes the permanent
  `releases/latest/download/<name>` links resolve to this build. Never attach
  versioned copies.
- If `gh` is missing or unauthenticated, stop and tell the developer
  (`brew install gh && gh auth login`).

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
  attached to the `vX.Y.Z` draft (`softprops/action-gh-release` finds the draft
  by tag and only adds files). The workflow sets `draft: true` on that step —
  without it the action *publishes* an existing draft once its uploads finish,
  so never remove it. Note in your status line that the smoke tests passed
  — this confirms the installed binary starts and resolves its shared
  libraries/DLLs on a clean machine, **not** that GPU offload works (CI
  runners have no GPU; that still needs an occasional real-hardware check).

Once Phase 7b's CI build succeeds, this is also the point to follow through on
any website updates queued back in Phase 5d — now that the platforms in
question are confirmed actually working, not just built.

## Phase 8 — Check the draft, then ask to publish

Run this only after Phase 7b succeeded (or the developer explicitly accepted a
missing platform). Verify the draft before asking — don't assume:

```bash
gh release view vX.Y.Z --json isDraft,assets --jq '{isDraft, assets: [.assets[] | {name, size, digest}]}'
shasum -a 256 release/EmailOps-macos.dmg release/EmailOps-CLI-macos.dmg
```

The draft must be `isDraft: true` and carry every expected asset:
`EmailOps-macos.dmg`, `EmailOps-CLI-macos.dmg`, `EmailOps-linux.AppImage`,
`EmailOps-linux.deb`, `EmailOps-windows.msi`, `EmailOps-windows-setup.exe`,
`EmailOps-windows-cuda.msi`, `EmailOps-windows-cuda-setup.exe` (plus the
optional `EmailOps-linux.rpm`). The macOS digests must match the local
`shasum` output. Anything missing or mismatched: stop and report.

Then show the developer the tag (+ commit), the asset list with sizes, the
draft URL, and the release notes, and **ask whether to publish**. Only on an
explicit yes:

```bash
gh release edit vX.Y.Z --draft=false --latest
```

Confirm it is live (`gh release view vX.Y.Z --json isDraft,url`) and that
`https://github.com/emailops/emailops/releases/latest/download/EmailOps-macos.dmg`
resolves (`curl -sIL -o /dev/null -w '%{http_code}' <url>` → 200). If the
developer says no, stop here and leave the draft as is.

## Phase 9 — Homebrew cask update (post-publish)

EmailOps is also distributed via the `emailops/homebrew-tap` cask. The cask is
generated **from the published release assets** (it pins the sha256 digests
GitHub computes for each asset), so this phase can only run **after** the
release was published in Phase 8. Full background and
invariants: `homebrew/README.md`.

1. **Verify the release is live and digested:**

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

   This rewrites `homebrew/Casks/emailops.rb`. macOS ships one universal DMG,
   so the cask is single-artifact — no `arch` stanza, no `depends_on arch:`.
   It fails outright if `EmailOps-macos.dmg` is missing from the release.

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

5. **Push the tap.** The developer already approved publishing, so no
   further confirmation:

   ```bash
   git -C ../homebrew-tap push origin main
   ```

   Users pick the new version up via `brew upgrade --cask emailops`.

6. **Commit the regenerated cask in the main repo too** (`homebrew/Casks/
   emailops.rb` is tracked here as the source of what was shipped):
   `git add homebrew/Casks/emailops.rb && git commit -m "chore: update Homebrew cask to vX.Y.Z"`,
   and push `main`.

**Invariant:** never replace a DMG asset on an already-published tag — the
cask pins its sha256, so swapping the file breaks every install of that
version. If an asset is bad, cut a new patch release instead.

## Done

Report: the new version, the Phase 1b verification result (summary path, newly failing tests and what the developer decided about them), that gates/build/verify passed, the local install
smoke-test result (with screenshot), any doc-staleness findings from Phase 5d
(app docs and website) and whether the developer acted on them, the commit +
tag created and pushed, the Linux/Windows CI result
(run URL, conclusion, smoke-test outcome per platform), the website push state
(updated + pushed, or held pending developer confirmation), the release state
(draft with its asset list, or published with its URL), and the Homebrew cask
state (updated + pushed to the tap, or pending the release publish).
