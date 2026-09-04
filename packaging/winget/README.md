# winget

Manifests for the Windows Package Manager, so `winget install EmailOps.EmailOps`
works and EmailOps shows up in `winget search email`.

That search surface is half the reason to be here: winget is not only a way to
install, it is a place Windows users go looking. Windows is also the platform
where the download page loses the most people — the installer is unsigned, so
first launch shows SmartScreen's "Windows protected your PC". A winget install
verifies the sha256 in the manifest before running anything, which sidesteps
that first impression entirely.

## Generating

`scripts/generate_winget.sh` reads the release's asset list from the public API
— including the sha256 digests GitHub computes for every asset — so nothing is
downloaded and no auth is needed. Run it after the Windows installer is
uploaded to the release:

```bash
make winget            # latest release
make winget TAG=v0.6.6 # specific tag
```

It writes three files under `packaging/winget/<version>/`, which is what the
winget schema requires: a version manifest naming the other two, an installer
manifest, and a locale manifest carrying the description people read in
`winget show`.

## Submitting

Manifests live in this repo only as the source of truth; winget itself serves
them from [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs),
so each release needs a pull request there.

```bash
# On a Windows machine, once per release:
winget validate --manifest packaging/winget/<version>
winget install wingetcreate
wingetcreate submit --token <github-token> packaging/winget/<version>
```

`wingetcreate submit` forks winget-pkgs, commits the manifests under
`manifests/e/EmailOps/EmailOps/<version>/` and opens the pull request. A bot
then runs validation and an automated install test; a human moderator merges.
Expect a few days for a first submission — the package identifier
`EmailOps.EmailOps` is claimed by whoever lands first, so the initial PR gets
more scrutiny than later version bumps.

`winget validate` and `wingetcreate` are Windows-only. The generator is not:
it runs anywhere, so the manifests can be prepared on any machine and only the
validate-and-submit step needs Windows.

## Choices worth not relitigating

**The NSIS installer, not the MSI.** winget wants one installer per
architecture. `EmailOps-windows-setup.exe` is the build the README recommends
(Vulkan GPU acceleration, CPU fallback when no compatible driver is present),
and Tauri's NSIS output supports the silent `/S` switch winget requires. The
CUDA build is deliberately absent: it is an NVIDIA-only alternative, not a
second architecture, and offering it here would ask users to make a hardware
judgement at install time.

**`Scope: user`.** The Tauri NSIS installer defaults to a per-user install and
needs no elevation. Declaring it lets winget install without a UAC prompt.

**No code signing.** winget accepts unsigned installers; the manifest pins the
sha256 and winget refuses to run a binary that does not match. Signing would
still be worth doing for the direct-download path — see the SmartScreen note in
the README — but it is not a blocker for being in winget.
