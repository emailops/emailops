# Homebrew Cask Distribution

EmailOps is distributed to Homebrew users through a dedicated tap repository
(`emailops/homebrew-tap`). This directory holds the **generated** cask file
(`Casks/emailops.rb`) plus this runbook. The cask is rendered by
`scripts/generate_cask.sh` (invoked as `make cask`) — never edit it by hand;
regenerate it instead.

## How it works

- The cask downloads the signed + notarized DMGs from the GitHub release
  assets at tag-scoped URLs
  (`https://github.com/emailops/emailops/releases/download/v<version>/…`).
- macOS ships ONE universal DMG (`EmailOps-macos.dmg`) that launches on both
  Apple Silicon and Intel, so the cask carries no `arch` stanza and no
  `depends_on arch:`. Embedded AI is refused on Intel at runtime
  (`ai::gpu_plan::embedded_runtime_supported`) rather than by shipping a
  second, trimmed build — see the `bootstrap-mac` comment in the Makefile.
- `sha256` values come from the digests GitHub computes for each release
  asset, read via the public API — the generator downloads nothing.
- `livecheck` follows the latest GitHub release, so `brew livecheck emailops`
  flags when the tap is behind.

## The tap

The tap lives at [`emailops/homebrew-tap`](https://github.com/emailops/homebrew-tap)
(`brew tap emailops/tap` resolves to it). The local clone is expected at
`../homebrew-tap` (sibling of this repo); recreate it with
`gh repo clone emailops/homebrew-tap ../homebrew-tap` if missing.

Users install with:

```bash
brew install --cask emailops/tap/emailops
```

## Per-release flow

1. Build and stage the artifact as usual:
   `make build-mac && make verify-mac && make dist-mac`.
2. Upload `release/EmailOps-macos.dmg` to the GitHub release for the tag.
3. Regenerate the cask: `make cask` (or `make cask TAG=vX.Y.Z`).
4. Copy `homebrew/Casks/emailops.rb` to `../homebrew-tap/Casks/emailops.rb`,
   commit (`emailops X.Y.Z`), push.

The `release` skill (`.claude/skills/release/SKILL.md`) runs this as its
Phase 9, after the developer publishes the GitHub release.
5. Optional local verification before pushing the tap (Homebrew ≥ 4 only
   loads casks from taps, so the download check needs a throwaway local tap):

   ```bash
   # lint — expect exactly ONE offense (Homebrew/OSDependsOn on the
   # depends_on line); it's deliberate, see scripts/generate_cask.sh header
   brew style homebrew/Casks/emailops.rb

   brew tap-new emailops/casktest --no-git
   cp homebrew/Casks/emailops.rb "$(brew --repository)/Library/Taps/emailops/homebrew-casktest/Casks/"
   brew fetch --cask emailops/casktest/emailops   # download + sha256 check
   brew audit --cask emailops/casktest/emailops
   brew untap emailops/casktest
   ```

## Invariants

- **Never replace a DMG asset on an already-tagged release.** The cask pins
  the asset's sha256; replacing the file breaks every user's install of that
  version. Ship a new tag instead.
- The versionless `latest/download/EmailOps-macos.dmg` links used elsewhere
  (website, README) are unaffected — the cask deliberately uses the
  tag-scoped URLs so its checksum stays stable.
- `zap` removes the app's local data (`~/Library/Application Support/
  com.emailops.app` — SQLite DB, downloaded models — plus caches and webview
  state). It cannot remove OAuth tokens from the macOS keychain; those stay
  until the user deletes them in Keychain Access.
