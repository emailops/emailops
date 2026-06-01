---
name: release
description: Cut a new signed + notarized EmailOps macOS release — version bump across all source-of-truth files, CHANGELOG, quality gates, universal build, commit, tag, and (confirmation-gated) push + GitHub release.
argument-hint: <patch|minor|major|X.Y.Z>
disable-model-invocation: true
allowed-tools: Bash, Read, Edit, Grep
---

# Release EmailOps

You are cutting a new release of EmailOps (a Tauri macOS app). Follow the phases
below in order. Run each phase, report a short status line, and **stop on the
first failure** — never paper over a failing gate.

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

## Phase 6 — Commit + tag

Once gates and build pass:

```bash
git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock CHANGELOG.md
git commit -m "chore: release vX.Y.Z"
git tag vX.Y.Z
```

Do **not** add Claude/agent as author or co-author (repo convention). Use the
real new version in both the message and tag.

## Phase 7 — Publish (confirmation-gated)

Pushing and releasing are shared, irreversible actions. **Always ask for
explicit confirmation before each**, even if the user kicked off the release.

1. Push branch + tag (only after the user confirms):

   ```bash
   git push origin main
   git push origin vX.Y.Z
   ```

2. GitHub release: check whether `gh` is installed (`command -v gh`).
   - If **available** and the user confirms, create the release and attach both
     the stable-named DMG (permanent latest link) and the versioned bundle DMG
     (per-release archival copy):

     ```bash
     gh release create vX.Y.Z \
       --title "vX.Y.Z" \
       --notes-file <changelog-section-or-temp-file> \
       release/EmailOps-macos.dmg \
       src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg
     ```

     The stable name is what makes
     `https://github.com/emailops/emailops/releases/latest/download/EmailOps-macos.dmg`
     resolve to this release's build.

   - If **not available**, do not fail. Print the manual steps: the DMG paths
     above and the exact `gh release create` command to run once `gh` is
     installed, then let the user take it from there.

## Done

Report: the new version, that gates/build/verify passed, the commit + tag
created, and the current push/release state (pushed or pending the user's
manual step).
