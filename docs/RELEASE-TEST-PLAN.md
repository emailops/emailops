# Release validation plan (Linux / Windows / macOS)

This is the deep, occasional, real-hardware validation layer — not a replacement
for CI. It exists to answer questions CI's own smoke test structurally cannot:
does the app actually work on a bare, never-touched machine, with a real GPU,
driven through a real UI interaction, not just "did the process stay alive for
8 seconds behind Xvfb."

Read this together with `docs/TEST-VMS.md` (the Azure GPU VM inventory,
gotchas, and verified-baseline numbers this plan produces) and
`docs/DECISIONS.md` (why CI's smoke test stays lightweight and this stays a
separate, manually-triggered layer instead of replacing it).

## What CI already proves, and where it stops

`.github/workflows/release.yml`'s `release` job installs the built package and
launches it on every Linux/Windows release build:

- **Linux** (`release.yml:244-289`): installs the `.deb`, launches under
  `xvfb-run` + `dbus-run-session` with an unlocked throwaway keyring, waits,
  confirms the process is still alive via `kill -0`, screenshots blind with
  `import -window root`.
- **Windows** (`release.yml:291-331`): installs the `.msi` silently, launches
  `emailops.exe`, confirms via `Get-Process`, best-effort screenshots via
  `System.Drawing.CopyFromScreen` (allowed to fail silently — GH's Windows
  runners may have no attached desktop session).

Both explicitly prove one thing only: **the binary starts and resolves its
shared libraries/DLLs on a clean machine.** Neither runner has a GPU, neither
drives the UI, and the Windows screenshot is best-effort. This is deliberate
(`docs/DECISIONS.md:417-419` rejected folding a full VM-based smoke test into
CI — it doesn't scale to every push and reintroduces manual-VM friction) —
but it means GPU offload, onboarding, and the AI feature actually answering a
question have **never** been verified by CI, on any platform, ever.

That gap is this plan's job. Run it:

- Before a release that changes the GPU backend path (Vulkan/CUDA wiring,
  `dynamic-backends`, driver-detection logic, DLL/so staging).
- Before a release that changes onboarding, model download, or first-run flow.
- Periodically (e.g. once a quarter) even with no relevant change, as a
  canary against upstream llama.cpp/driver/OS drift.
- **Not** on every release — it's manual-triggered, VM-billed, and takes
  15 min – 2 hr depending on platform and whether a from-source build is
  needed.

## Evidence bundle

Every run produces one directory (local scratch, never committed — this is
throwaway-VM output, not a source artifact):

```
<scratchpad>/release-validation/<platform>-<date>/
  summary.json          # {platform, artifact, backend, tok_per_sec, gpu_offload_confirmed, pass, notes}
  screenshot-chat.png   # chat response with the "<model> · N tokens · Ts · X tok/s" footer visible
  screenshot-output.png # Output panel (llamacpp log lines), if the case is about backend loading
  gpu-poll.log          # nvidia-smi (or equivalent) samples taken during inference
  app.log               # whatever the app itself logged (Output panel export, or file log if present)
```

`summary.json` is the one machine-checkable artifact — the skill's Phase "assemble
report" step must always produce it, even on failure (with `pass: false` and
`notes` explaining why), so a run is never just "it hung, no evidence."

## Pass/fail criteria

A platform run **passes** when all of:

1. The installed artifact (not a `cargo run` build) launches and completes
   onboarding without a crash or fatal dialog.
2. Chat unlocks and a sent message returns a real assistant response (not an
   error banner).
3. The Output panel / logs show the expected backend loaded (`llamacpp` model
   load line) with no fatal error.
4. If the run's purpose is GPU validation: `tok/s` and the GPU-utilization
   poll are captured either way. **A confirmed CPU fallback is a documented
   result, not a failure** — mark `gpu_offload_confirmed: false` with the
   reason (e.g. TCC-mode SKU) rather than treating it as a broken test. Only
   treat it as a failure if the platform/VM SKU was specifically chosen to
   validate GPU and turns out incapable for a *new, previously undocumented*
   reason.

## Linux — Azure GPU VM (implemented)

Fully scriptable today. `make testvm-linux` restores the pre-provisioned
snapshot (NVIDIA driver + Vulkan SDK + Xvfb/fluxbox/x11vnc already baked in,
`docs/TEST-VMS.md:60-68`); the existing runbook
(`docs/TEST-VMS.md:87-131`) covers signed-URL artifact download, `apt-get
install --reinstall`, launching under `dbus-run-session` with an unlocked
keyring, and VNC screenshotting via `vncdotool`. Verified baseline already on
record: CPU 1.5 tok/s vs Vulkan-GPU 33-36 tok/s (`docs/TEST-VMS.md:272-282`) —
this is the reference range future runs compare against.

## Windows — Azure GPU VM (implemented, Vulkan path; CUDA path new)

Also fully scriptable, with one hard platform caveat already discovered and
documented: `Standard_NC4as_T4_v3`'s Tesla T4 runs in **TCC** driver mode,
which has no Vulkan ICD (`docs/TEST-VMS.md:219-245`) — Vulkan offload cannot
be validated on this VM SKU, full stop, `nvidia-smi -dm 0` returns "Not
Supported." That is exactly why a **CUDA** backend variant matters here: TCC
mode is what CUDA compute workloads are *for*, so the same VM that cannot
prove Vulkan offload can prove CUDA offload.

The `cuda` Cargo feature already exists (`src-tauri/Cargo.toml:190`,
`cuda = ["llama-cpp-2/cuda"]`) but has never been built, has no CI wiring
(`release.yml:227` hardcodes `vulkan`), and needs the CUDA Toolkit, which
GitHub's runners don't have — so today the only way to produce a CUDA build is
**from source, directly on a GPU VM that already has the toolkit**, not
cross-compiled. Runbook (see `.claude/skills/validate-release/SKILL.md`
Windows/CUDA phase for the runnable form):

1. Bootstrap the VM: Rust (msvc target), VS2022 Build Tools (VCTools
   workload), CMake, LLVM (libclang, for bindgen), Node.js, Git for Windows
   (bash.exe — the Makefile requires a POSIX shell), and the CUDA Toolkit
   matching the installed driver's max-supported CUDA version (check via
   `nvidia-smi`'s "CUDA Version" field — newer toolkits are fine on older
   drivers within the same major series, but never install a toolkit newer
   than the driver supports).
2. **Gotcha discovered building this plan**: launching a long-running
   installer via SSH's `Start-Process` (even detached, even with
   `-RedirectStandardOutput`) gets killed when the SSH session closes —
   Windows OpenSSH assigns spawned processes to a Job Object tied to the
   session, and Job Object membership survives `Start-Process` unless the
   child explicitly breaks away. **Use a one-shot Scheduled Task
   (`schtasks /Create ... /SC ONCE ... ; schtasks /Run ...`) instead** — it
   fully escapes the SSH session's process tree. Corollary: a task created
   with `/RU SYSTEM` installs into the SYSTEM profile, not the interactive
   user's — force `CARGO_HOME`/`RUSTUP_HOME` (and anything else normally
   per-user) to a machine-wide path (e.g. `C:\rust\cargo`) and add it to the
   **machine** `Path`, not the user `Path`, so a later SSH session (running
   as the normal admin user) can still find `cargo`/`rustc`.
3. Transfer source via `git archive HEAD | gzip` + `scp` + `tar.exe -xzf` (the
   Windows 11 image ships bsdtar as `tar.exe` — no separate install needed).
   Cleaner than `git clone` since it needs no credentials on the VM and never
   ships `.git` history or local build artifacts.
4. Build: `DYNAMIC_BACKENDS=1 CARGO_FEATURES=cuda scripts/build_platform.sh
   windows` (run via `bash.exe`, per the Makefile's POSIX-shell requirement).
   The existing Vulkan-specific `/FS`/Ninja/`--jobs 1` workarounds in that
   script target the `vulkan-shaders-gen` sub-project specifically
   (`build_platform.sh:44-51`) and may simply not apply to a CUDA build —
   don't assume they're needed; let it fail first if it's going to.
5. `scripts/verify_platform.sh windows` (backend-staging checks are already
   backend-name-agnostic — see `docs/TEST-VMS.md` and
   `scripts/verify_platform.sh:115-151`).
6. Install the produced `.msi` (still needs the VC++ Redistributable
   prerequisite first, `docs/TEST-VMS.md:184-197`), drive it via the
   RDP-via-Docker technique (`docs/TEST-VMS.md:164-176` — required, not
   optional; SSH-launched GUI processes render in a non-interactive window
   station and never produce a screenshottable window), send a chat message,
   and poll `nvidia-smi --query-gpu=utilization.gpu,memory.used
   --format=csv,noheader` during inference. Non-zero utilization + a
   tok/s figure well above the CPU baseline (3.2 tok/s,
   `docs/TEST-VMS.md:284-299`) confirms real CUDA offload.
7. Record the result — pass or fail either way — as a new
   `docs/TEST-VMS.md` verified-baseline entry, dated and commit-tagged,
   the same way the CPU-fallback baseline was recorded.

Once proven working this way, wiring `CARGO_FEATURES=cuda` into
`release.yml` as a genuine second Windows build leg (with a CUDA Toolkit
install step, e.g. `Jimver/cuda-toolkit`) is future work, tracked separately —
this plan's job is the one-off "does it even work" proof, not permanent CI
integration.

## macOS — no VM story exists yet (proposed, not implemented)

Unlike Linux/Windows, this repo has **zero** existing tooling for a clean,
throwaway macOS install test. `verify-mac` checks the *build artifact*
(codesign, notarization stapling, Gatekeeper, both universal slices present)
but never installs or launches it — release
validation for macOS today is "the developer's own already-provisioned dev
Mac, run manually." There's no Azure-Mac-VM equivalent to lean on (Azure
doesn't offer flexible ephemeral Apple-silicon VMs the way it offers T4 GPU
boxes), and Apple's EULA restricts running macOS in a VM to genuine Apple
hardware — ruling out cloud providers outright for anything cheap.

Two options, not yet built:

1. **[Tart](https://github.com/cirruslabs/tart)** (Cirrus Labs, MIT-licensed,
   what GitHub's own macOS CI images are increasingly built on) — creates
   real, ephemeral macOS VMs on an Apple Silicon host via Apple's
   Virtualization framework. This is the high-fidelity option: a genuinely
   clean OS, snapshot/restore like the Linux Azure VM, scriptable via `tart
   create`/`tart run`/`tart exec`. Requires an Apple Silicon Mac as the host
   (the developer's own machine, run locally — this is not cloud-hosted) and
   a base macOS IPSW/image, which is a one-time multi-GB fetch analogous to
   the Linux snapshot's initial provisioning cost.
2. **Ephemeral scratch user account** (lower fidelity, zero new tooling) — a
   throwaway admin user created via `sysadminctl -addUser`, install the
   `.dmg` there, test, then `sysadminctl -deleteUser`. Does not prove a truly
   clean OS (shared system frameworks, shared driver/GPU state, shared
   Homebrew/dev-tool pollution on `$PATH` for other users can still leak in
   via system-level state) but catches per-user state bugs (fresh keychain,
   fresh `~/Library/Application Support`, no dev-only env vars) with no
   infrastructure investment.

Recommendation: start with the scratch-user-account approach (cheap, catches
real bugs like "assumes a keychain item that only exists on the dev machine"),
and only invest in Tart if/when a macOS-specific release regression actually
needs a genuinely clean OS to reproduce or if GPU/Metal backend validation
becomes relevant here the way Vulkan/CUDA did for Linux/Windows. Track this as
a documented gap, not a silent one — `validate-release`'s macOS phase should
say exactly this and fall back to the scratch-user path, or clearly report
"skipped, no VM tooling" rather than pretending to validate something it
didn't.

## Cleanup

Every run must end by asking the user (never assuming) how to handle the VM —
stop (deallocate, keep disk), destroy (snapshot then delete), or leave running
— exactly as `.claude/skills/test-windows-gpu/SKILL.md` Phase 8 already does.
GPU VMs bill by the hour; a validation run that's "done" but left running
silently is a cost bug, not a convenience.
