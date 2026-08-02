---
name: validate-release
description: Run the deep, real-hardware release validation pass for a given platform (linux, windows, windows-cuda, or macos) — install the actual release artifact on a clean/throwaway machine, drive onboarding and chat through a real UI interaction, capture evidence (screenshots, GPU utilization, tok/s), assemble a pass/fail report, and offer to clean up afterward. This is the deeper layer behind CI's shallow install-and-check-alive smoke test; see docs/RELEASE-TEST-PLAN.md.
argument-hint: "<linux|windows|windows-cuda|macos|all> [ci-run-id]"
disable-model-invocation: true
allowed-tools: Bash, Read, Write
---

# Validate a release on real hardware

Full rationale, evidence-bundle format, pass/fail criteria, and per-platform
detail live in `docs/RELEASE-TEST-PLAN.md` — read it first, this skill is
that plan made runnable. Platform-specific runbooks it builds on:
`docs/TEST-VMS.md` (Linux/Windows Azure GPU VMs, including the RDP-via-Docker
screenshot technique and the Windows TCC/Vulkan limitation) and
`.claude/skills/test-windows-gpu/SKILL.md` (the Windows Vulkan flow this
skill generalizes). Update those docs with anything new you learn — this
skill and its docs are a two-way contract, not a one-off.

**Ask before any destructive/costly step you're not sure about** (provisioning
a VM, destroying a snapshot, running a from-source build that takes 1-2 hr).
Don't silently retry a failed step more than twice — stop and report instead.

## Phase 0 — Resolve platform(s) and artifact

Parse the argument: `linux`, `windows` (Vulkan path — the shipped default),
`windows-cuda` (from-source CUDA build — see Phase W-CUDA below), `macos`, or
`all` (runs linux + windows in sequence; `windows-cuda` and `macos` are opt-in
only, since they're respectively a 1-2 hr from-source build and a
not-yet-automated path — never run them under `all` without being asked).

```bash
RUN_ID="${2:-$(gh run list --workflow=release.yml --status success --limit 1 --json databaseId --jq '.[0].databaseId')}"
gh run view "$RUN_ID" --json headSha,displayTitle
```

Create the evidence directory for this run:
`<scratchpad>/release-validation/<platform>-<date>/` (see
`docs/RELEASE-TEST-PLAN.md`'s "Evidence bundle" section for the exact file
layout this skill must produce).

## Phase L — Linux

```bash
bash scripts/testvm.sh status   # check for a conflicting Windows VM first — only one GPU VM fits the quota
make testvm-linux               # restores the provisioned snapshot; ask before `--fresh` or destroying a Windows VM
```

Follow `docs/TEST-VMS.md`'s "Running a release artifact" (Linux) section
verbatim: signed-URL artifact download, `apt-get install --reinstall`,
`dbus-run-session` + unlocked keyring, VNC screenshot via `vncdotool`. Send a
chat message, poll `nvidia-smi --query-gpu=utilization.gpu,memory.used
--format=csv,noheader` during inference, screenshot the response. Compare
tok/s against the recorded baseline (33-36 tok/s GPU, 1.5 tok/s CPU,
`docs/TEST-VMS.md:272-282`).

## Phase W — Windows (Vulkan, the shipped default)

This is `.claude/skills/test-windows-gpu/SKILL.md` end to end — run its
Phases 1-7 (provision/reuse VM, enable SSH, RDP session, install the release
`.msi` + VC++ Redistributable, onboarding + synthetic account, send a
message + poll GPU, screenshot). Its Phase 6 already documents the expected
outcome: **CPU fallback, not GPU** — TCC mode has no Vulkan ICD on this VM
SKU. Confirming that (again) is a pass for this phase, not a failure — see
`docs/RELEASE-TEST-PLAN.md`'s pass/fail criteria. Only run Phase W-CUDA below
if you specifically need GPU offload confirmed.

## Phase W-CUDA — Windows (CUDA, from source — expensive, ask first)

**This is a 1-2 hr from-source build on the GPU VM, not an artifact
install.** Confirm with the user before starting (VM billing + build time).
The Vulkan-path CI artifact does not exist for CUDA — there's no
`release.yml` CUDA leg — so this phase builds it locally on the VM, which
already has the right driver/TCC mode for CUDA to actually work where Vulkan
can't.

```bash
bash scripts/testvm.sh status
make testvm-windows   # or testvm-start if it already exists, stopped
```

1. **Bootstrap prerequisites** (Rust msvc target, VS2022 Build Tools
   VCTools workload, CMake, LLVM/libclang, Node.js, Git for Windows, CUDA
   Toolkit matching the driver's max-supported version per `nvidia-smi`).
   Write a PowerShell script covering all of these (see
   `docs/RELEASE-TEST-PLAN.md`'s Windows/CUDA section for the exact package
   list and URLs used the first time this was run) and launch it as a
   **Scheduled Task**, not `Start-Process` over SSH:
   ```bash
   scp -i private-scripts/emailops_gpu_test_key private-scripts/bootstrap-cuda-build.ps1 azureuser@"$IP":C:/bootstrap-cuda-build.ps1
   ssh -i private-scripts/emailops_gpu_test_key azureuser@"$IP" \
     "schtasks /Create /TN BootstrapCuda /TR 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\bootstrap-cuda-build.ps1' /SC ONCE /ST 00:00 /RU SYSTEM /RL HIGHEST /F; schtasks /Run /TN BootstrapCuda"
   ```
   **Why a Scheduled Task, not `Start-Process`**: Windows OpenSSH puts every
   command's process tree in a Job Object tied to the SSH session; even a
   detached `Start-Process` child gets killed the instant the SSH connection
   closes. A one-shot Scheduled Task fully escapes that tree. Corollary:
   `/RU SYSTEM` installs into the SYSTEM profile, not the interactive user's
   — the script must force `CARGO_HOME`/`RUSTUP_HOME` (and anything else
   normally per-user) to a machine-wide path and add it to the **machine**
   `Path`, or a later SSH session (as the normal admin user) won't find
   `cargo`/`rustc`.
   Poll `Get-Content C:\bootstrap-cuda.log -Tail 2` every 30s (or use
   `schtasks /Query /TN BootstrapCuda /V /FO LIST` for task status) until it
   logs `DONE`. This alone can take 20-40 min (VS Build Tools + CUDA Toolkit
   are multi-GB downloads).
2. **Transfer source**: `git archive HEAD | gzip > src.tar.gz`, `scp` it up,
   `tar.exe -xzf src.tar.gz -C C:\emailops` (Windows 11 ships `tar.exe` as
   bsdtar — no install needed). Cleaner than `git clone`: no credentials on
   the VM, no `.git` history, no local build artifacts along for the ride.
3. **Build**: `DYNAMIC_BACKENDS=1 CARGO_FEATURES=cuda scripts/build_platform.sh
   windows`, run via Git for Windows' `bash.exe` (the Makefile needs a POSIX
   shell). Expect this to surface *new* failures — nobody has built this
   feature combination before. Don't assume the existing Vulkan-specific
   `/FS`/Ninja workarounds in `build_platform.sh` apply; let it fail on its
   own terms first.
4. **Verify + install**: `scripts/verify_platform.sh windows`, then the VC++
   Redistributable + `.msi` install as in Phase W.
5. **Test**: RDP session (Phase W's Docker/FreeRDP technique), send a
   message, poll `nvidia-smi`. Non-zero GPU utilization + tok/s well above
   the 3.2 tok/s CPU baseline confirms real CUDA offload — the first time
   this repo will have proven GPU offload on Windows at all.
6. Append the result to `docs/TEST-VMS.md`'s verified-baseline table
   (pass or fail — a failure here is real information, not something to
   hide) — this is a factual runbook update, do it regardless of Phase 8's
   commit/push decision.

## Phase M — macOS (proposed path, not yet automated)

There is no clean-VM tooling for macOS in this repo yet — see
`docs/RELEASE-TEST-PLAN.md`'s macOS section. Do not fabricate a pass. Options,
weakest-to-strongest, ask the user which (if any) to attempt:

1. **Report the gap and stop** — valid default. Say plainly: "no macOS clean-
   install tooling exists; the only validation available is the developer's
   own dev Mac via `make build-mac && make verify-mac`, which is not a clean-
   room test."
2. **Scratch user account** (lowest new-infra cost): `sysadminctl -addUser
   <name> -fullName "Release Test" -password <random>`, install the `.dmg`
   under that user, drive it (this still needs an interactive login session —
   `osascript`/`System Events` UI scripting, or manual), then `sysadminctl
   -deleteUser <name> -deleteHome` when done.
3. **Tart VM** (highest fidelity, real infra investment — only if explicitly
   requested): requires an Apple Silicon host and a base image; see
   `docs/RELEASE-TEST-PLAN.md` for why this isn't built yet.

## Phase R — Assemble the report

Regardless of platform, write `summary.json` in the evidence directory (see
`docs/RELEASE-TEST-PLAN.md`'s "Evidence bundle" section for the schema) even
on failure — `pass: false` with `notes` explaining what happened is a valid,
required outcome, not something to skip because the run didn't go cleanly.

## Phase S — Send results to the user

`SendUserFile` the clearest screenshot(s) plus `summary.json`. State plainly
in the caption: what was tested, the tok/s achieved, and whether GPU offload
was confirmed — or, for macOS, that this remains a documented gap rather than
something silently skipped.

## Phase C — Offer cleanup

**Always ask, never assume** — same pattern as
`.claude/skills/test-windows-gpu/SKILL.md` Phase 8:

- **Stop (deallocate)** — `make testvm-stop`. Keeps VM/disk (including any
  from-source CUDA build — rebuilding it is 20-40 min of prerequisites alone).
- **Destroy (snapshot then delete)** — `make testvm-destroy`.
- **Leave it running** — only if the user explicitly says so; GPU VMs bill by
  the hour.

Also stop any local Docker RDP helper regardless of platform:
```bash
docker rm -f rdp-capture 2>/dev/null || true
```
