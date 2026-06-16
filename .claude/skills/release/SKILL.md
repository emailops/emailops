---
name: release
description: Cut a new signed + notarized EmailOps macOS release — version bump across all source-of-truth files, CHANGELOG, quality gates, universal app + standalone CLI builds, commit, tag, and (confirmation-gated) push. The GitHub release is never created automatically; the skill prints the exact info to publish it manually.
argument-hint: <patch|minor|major|X.Y.Z>
disable-model-invocation: true
allowed-tools: Bash, Read, Edit, Write, Grep
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

**Do not create the GitHub release yourself.** Even if `gh` is installed and
authenticated, never run `gh release create` (or otherwise publish the release)
as part of this skill. The developer creates the GitHub release manually — your
job ends at printing the exact info they need (see Phase 8).

## Phase 8 — Print the manual GitHub-release info

The GitHub release is always created by the developer by hand. Gather and print
everything they need to do it, so it is copy-paste ready. Verify each fact
before printing it (don't assume):

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
- **Both ways to publish**, and let the developer pick:
  - *gh CLI* (run from repo root):

    ```bash
    gh release create vX.Y.Z \
      --title "vX.Y.Z" \
      --notes-file /tmp/emailops-vX.Y.Z-notes.md \
      release/EmailOps-macos.dmg \
      release/EmailOps-CLI-macos.dmg
    ```

    If `gh` is not installed, mention it (`brew install gh && gh auth login`).
  - *Web UI*: open `https://github.com/emailops/emailops/releases/new`, choose
    the existing `vX.Y.Z` tag, set the title, paste the notes, drag in **both**
    `release/EmailOps-macos.dmg` and `release/EmailOps-CLI-macos.dmg` (keep the
    filenames as-is), keep "Set as the latest release" checked, and publish.

## Done

Report: the new version, that gates/build/verify passed, the commit + tag
created, the push state (pushed or pending), and that the GitHub release is left
for the developer to create manually (with the info from Phase 8 printed above).
