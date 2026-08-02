# Azure GPU test VMs

The one place where EmailOps release artifacts are validated on **real GPU
hardware**. CI runners have no GPU, so their smoke test only ever exercises the
CPU fallback — Vulkan offload, driver packaging, and the actual desktop UI can
only be confirmed here.

Everything is driven by `make testvm-*` (thin targets over `scripts/testvm.sh`).

```bash
make testvm-status     # VMs, snapshots, remaining T4 quota
make testvm-linux      # restore Linux GPU VM from its snapshot (~5 min)
make testvm-windows    # create Windows 11 GPU VM
make testvm-stop       # deallocate — stops compute billing, keeps the disk
make testvm-destroy    # snapshot the OS disk, then delete VM + disk + NIC
```

## The constraint that shapes everything

The subscription's **`Standard NCASv3_T4 Family` quota is 4 vCPUs**, and one
`Standard_NC4as_T4_v3` consumes exactly 4.

**There is room for one GPU test VM at a time.** Linux and Windows take turns:
`make testvm-destroy` before creating the other. `scripts/testvm.sh` refuses to
create a second rather than letting Azure fail deployment halfway (which leaves
a dangling NIC holding the public IP).

Raising the quota is a support request against the NCASv3_T4 family in
`westeurope`, if ever having both simultaneously is worth it.

## Fixed inventory

| Resource | Name | Notes |
|---|---|---|
| Resource group | `EMAILOPS-LINUX-GPU-TEST-RG` | Name predates the Windows VM — it holds both. |
| Region | `westeurope` | Quota is per-region; moving means requesting quota again. |
| Size | `Standard_NC4as_T4_v3` | 4 vCPU, 28 GB RAM, 1× Tesla T4 (16 GB). |
| VNet / subnet | `emailops-gpu-testVNET` / `emailops-gpu-testSubnet` | |
| Public IP | `emailops-gpu-testPublicIP` | Static — resolve it, don't hardcode it (below). |
| NSG | `emailops-gpu-testNSG` | SSH 22, VNC 5901, RDP 3389 — each locked to one source IP. |
| Linux snapshot | `emailops-gpu-linux-<date>` | Incremental, `Standard_LRS`. |

This file is in a **public repo**, so it deliberately records no address,
subscription ID, or credential. Resolve the IP when you need it:

```bash
VMIP=$(az network public-ip show -g EMAILOPS-LINUX-GPU-TEST-RG \
  -n emailops-gpu-testPublicIP --query ipAddress -o tsv)
```

**Networking is deliberately never torn down with the VM.** It costs little to
idle, keeps the address stable across rebuilds, and preserves the NSG rules.
`testvm.sh` re-points the allowlist at your *current* public IP on every
create/start, so a changed home IP doesn't lock you out.

## Why destroy always snapshots

A fully provisioned Linux box carries roughly an hour of setup:

- NVIDIA driver (via the `NvidiaGpuDriverLinux` extension) + LunarG Vulkan SDK
- headless desktop: `Xvfb` on `:1`, `fluxbox`, `x11vnc` on 5901
- `gnome-keyring` + `dbus` (the app aborts at startup without a Secret Service)
- the demo DB at `~/.emailops-demo-data` and the **3 GB** `qwen3.5-4b` GGUF

`make testvm-destroy` deallocates, snapshots the OS disk incrementally, then
deletes VM + disk + NIC. Restoring is one command and arrives fully provisioned.
Snapshots bill on *used* capacity at standard-storage rates, far below what an
idle 128 GB Premium SSD costs.

### Disk sizing

A provisioned Linux box measures **~26 GB** after cleanup:

| Path | Size | What |
|---|---|---|
| `/usr` | 13 GB | Vulkan SDK, NVIDIA driver, WebKitGTK deps |
| `/home` | 9.5 GB | 3 GB GGUF, demo DB, toolchains |
| `/opt` | 2.5 GB | NVIDIA |

So fresh VMs default to `--os-disk-size-gb 64` (`EMAILOPS_TESTVM_DISK_GB`).
Use 128 only to build from source on the VM: `src-tauri/target` alone reached
10 GB, and a debug build can reach 45 GB.

A disk restored from a snapshot **cannot be smaller than the snapshot's source
disk**, so restores land at the original 128 GB regardless of that setting.

## Running a release artifact

Full worked example — this is the loop that caught the glibc regression:

```bash
make testvm-linux                    # restore, ~5 min

# Download the artifact with a short-lived signed URL (avoids putting a GitHub
# token on the VM). Generate and use it in one step — it expires quickly.
URL=$(curl -s -o /dev/null -D - -H "Authorization: token $(gh auth token)" \
  "https://api.github.com/repos/emailops/emailops/actions/artifacts/<ID>/zip" \
  | grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r\n')
```

Then on the VM: `wget -O art.zip "$URL"`, `unzip`, and
`sudo apt-get install --reinstall -y ./EmailOps-linux.deb`.

`--reinstall` is required, not cosmetic: apt matches on version string, and
consecutive builds don't bump it, so a plain install silently no-ops and you
keep testing a stale binary.

Launch it against the demo mailbox on the virtual display:

```bash
export DISPLAY=:1
export EMAILOPS_DATA_DIR=/home/azureuser/.emailops-demo-data
export WEBKIT_DISABLE_COMPOSITING_MODE=1
dbus-run-session -- bash -c 'echo -n "" | gnome-keyring-daemon --unlock; exec /usr/bin/emailops'
```

Screenshot it from your Mac over VNC (5901 is already allowlisted):

```bash
uv run --with vncdotool python -c "
from vncdotool import api
import os
c = api.connect(os.environ['VMIP'] + '::5901', password=None); c.timeout=60
c.captureScreen('shot.png'); api.shutdown()"
```

> The x11vnc session runs with `-nopw`. Its only protection is the NSG
> single-source-IP rule, so treat that rule as the actual security boundary:
> never widen it to `0.0.0.0/0` "just to test", and prefer a VNC password if
> you ever run it from a shared/NAT'd network where the allowlisted address
> is not exclusively yours.

## Running a release artifact (Windows)

`make testvm-windows` always creates from the base `WIN_IMAGE` (no
snapshot-restore path exists yet for Windows, unlike Linux) — the VM needs
first-login OOBE completed before anything else works, and OpenSSH is not
enabled by default.

```bash
make testvm-windows                  # create, ~5-10 min (NVIDIA driver install)
```

**Enable SSH** (control-plane channel, no inbound port needed to get started):

```bash
PUB_KEY="$(cat private-scripts/emailops_gpu_test_key.pub)"
az vm run-command invoke -g EMAILOPS-LINUX-GPU-TEST-RG -n emailops-win-gpu \
  --command-id RunPowerShellScript --scripts @private-scripts/enable-ssh.ps1 \
  --parameters "pubKey=${PUB_KEY}"
# Then, same profile-scoping fix as any fresh Windows box (see Gotchas):
az vm run-command invoke -g EMAILOPS-LINUX-GPU-TEST-RG -n emailops-win-gpu \
  --command-id RunPowerShellScript \
  --scripts "Set-NetFirewallRule -Name OpenSSH-Server-In-TCP -Profile Any"
```

**Complete first-login OOBE.** A freshly created Windows 11 VM has never had
anyone log in, so there is no interactive desktop session yet — GUI apps
launched over SSH run in a different, non-interactive window station and
their windows never render (`Start-Process` reports the process "Responding"
but no window ever appears, and screenshot capture throws `The handle is
invalid`). RDP in once to drive OOBE and get a real interactive session:

```bash
# FreeRDP + Xvfb + ImageMagick in a throwaway container — avoids needing
# XQuartz/X11 on the Mac itself.
docker run --rm -d --name rdp-capture debian:bookworm-slim sleep 3600
docker exec rdp-capture bash -c "apt-get update -qq && apt-get install -y -qq xvfb freerdp2-x11 imagemagick xdotool"
docker exec -d rdp-capture bash -c "Xvfb :1 -screen 0 1600x1000x24 > /tmp/xvfb.log 2>&1"
docker exec rdp-capture bash -c "DISPLAY=:1 xfreerdp /v:<ip> /u:azureuser /p:'<password>' /cert:ignore /w:1600 /h:1000 &"
# Screenshot the virtual display at any point:
docker exec rdp-capture bash -c "DISPLAY=:1 import -window root /tmp/shot.png"
docker cp rdp-capture:/tmp/shot.png ./shot.png
# Click through OOBE (privacy settings -> Accept), then the desktop appears.
docker exec rdp-capture bash -c "DISPLAY=:1 xdotool mousemove <x> <y> click 1"
```

Once OOBE is done, this session persists — launch/interact with the app the
same way (`xdotool mousemove ... click 1`, screenshot after each step) rather
than going back to SSH's `Start-Process`, which will hit the same
non-interactive-window-station problem for *any* GUI app on this VM, not
just first boot.

**Install the release artifact + prerequisites.** Same signed-URL pattern as
Linux (`docs/TEST-VMS.md`'s Linux section above) to avoid a GitHub token on
the VM. Unlike the NSIS `.exe` installer, the raw `.msi` does **not**
bootstrap the VC++ Redistributable — install it explicitly first, or every
launch dies silently with exit code `-1073741515`
(`STATUS_DLL_NOT_FOUND`/`0xC0000135`) and empty stdout/stderr (GUI-subsystem
apps don't reliably surface output through `-RedirectStandardOutput` either,
so check `$proc.ExitCode` — see Gotchas):

```powershell
Invoke-WebRequest -Uri 'https://aka.ms/vs/17/release/vc_redist.x64.exe' -OutFile C:\vc_redist.x64.exe
Start-Process -FilePath C:\vc_redist.x64.exe -ArgumentList '/install','/quiet','/norestart' -Wait
Start-Process msiexec.exe -ArgumentList '/i C:\art\EmailOps-windows.msi /quiet /qn /norestart' -Wait
```

**Get a chat model loaded without real email credentials.** Onboarding
requires an account before the Chat view unlocks, and OAuth/IMAP needs real
credentials you should not put on a throwaway VM. Skip account setup in the
wizard, then insert a synthetic row directly (find `sqlite3.exe` via the
[official Windows build](https://sqlite.org/download.html) if not already on
the VM — a bare Windows 11 image doesn't ship one):

```powershell
$now = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
& sqlite3.exe "$env:APPDATA\com.emailops.app\emailops.db" `
  "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) VALUES ('test-account-1', 'imap', 'test@example.test', 'GPU Test Account', $now, 0, 1);"
```

The account will show an "Authentication required" banner (expected — no
real credentials exist) but that's enough to unlock Chat, which works
without a working sync. Restart the app after inserting the row (accounts
load once at startup); delete `emailops.db-wal`/`emailops.db-shm` first if
the app was already running when you touched the DB, or SQLite will try to
replay a stale WAL against rows it didn't see change.

## Windows GPU limitation: `Standard_NC4as_T4_v3` cannot validate Vulkan

**This VM series cannot be used to confirm Windows GPU offload.** Its Tesla
T4 runs in **TCC** (Tesla Compute Cluster) driver mode — confirmed via
`nvidia-smi -q | Select-String -Context 0,2 'Driver Model'` — which is
compute-only (CUDA) and does not expose a Vulkan-capable ICD. `nvidia-smi -dm
0` (the documented way to request WDDM) returns `Unable to set driver model
... Not Supported`: this is a hard limitation of the NC-series SKU, not a
driver/config problem fixable from inside the VM.

Symptom if you hit this: chat works end-to-end and produces a real answer,
but at CPU-fallback speed (~3 tok/s on `qwen3.5-4b-q4_k_m`, matching the 1.5
tok/s CPU baseline below, not the 33-36 tok/s GPU one) — and
`nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader`
polled during inference stays at `0 %, 9 MiB` throughout, i.e. the GPU is
never touched despite `ggml-vulkan.dll` being present and correctly staged
in the installed app's `backends\` directory (rule out a packaging bug
before assuming it's this TCC issue — check the DLL is actually there
first).

**To actually validate Windows Vulkan offload, use a WDDM-capable GPU VM
series instead** — e.g. the NV-series (`Standard_NV6ads_A10_v5` or similar),
which is built for GPU-accelerated remote visualization/RDP and ships a
WDDM driver by default. That requires its own quota request and is not
wired into `scripts/testvm.sh` yet (which only provisions
`Standard_NC4as_T4_v3`, shared with the Linux leg's quota) — follow-up work,
not something to improvise by reusing the existing NC-series VM.

## Gotchas

- **Use absolute paths in launcher scripts.** Started via `setsid` from an
  `az vm run-command` shell, `$HOME` is unset — `$HOME/.emailops-demo-data`
  silently resolved to `/.emailops-demo-data` and booted a blank DB against
  which every chat answer looked broken.
- **`az vm run-command` output is capped at ~4 KB.** Fine for logs, useless for
  shipping a screenshot back. Use VNC for images.
- **`az vm run-command --parameters` mangles signed URLs** (it does
  `export <value>`, and the URL contains `=`). Inline the URL into the script
  instead — and build that script with `printf`, not `sed`, because `&` in a
  `sed` replacement expands to the whole match.
- **Windows computer names max out at 15 chars.** `az vm create` defaults the
  computer name to the VM name; `emailops-win-gpu` is 16 and fails deployment
  *after* creating the NIC. `testvm.sh` passes `--computer-name` explicitly.
- **The keychain aborts startup.** No D-Bus session bus on a bare VM means
  `[startup][fatal] ... failed to initialise the OS keychain` (whose message
  blames disk space, which is misleading here). Always wrap in
  `dbus-run-session` with an unlocked throwaway keyring.
- **WebKitGTK needs compositing disabled** on these headless boxes, or the
  Tauri window paints blank white.
- **Don't leave it running.** `Standard_NC4as_T4_v3` bills by the hour;
  `make testvm-stop` deallocates. The Linux box also carries an
  `idle-shutdown.sh`.

## Verified baseline (2026-07-31, v0.6.4)

Linux `.deb` installed natively on Ubuntu 22.04, chat against the demo mailbox:

| | tok/s | Notes |
|---|---|---|
| CPU fallback | 1.5 | no Vulkan ICD visible |
| **Tesla T4 via Vulkan** | **33–36** | ~4.7 GB model resident on the GPU |

Confirms `libggml-vulkan.so` ships in the `.deb` and the dynamic-backend
packaging resolves correctly against the driver's own Vulkan ICD.

## Verified baseline (2026-08-01, v0.6.4 / commit 9501e95)

Windows `.msi` installed natively on the same VM series, chat against a
synthetic single-account DB (see above):

| | tok/s | Notes |
|---|---|---|
| CPU fallback | 3.2 | `nvidia-smi` confirms 0% GPU util throughout — this VM series cannot reach Vulkan, see above |

Confirms the app installs, starts, loads the embedded model, and answers a
chat message end to end on Windows — including every fix from the
`worktree-cross-platform` build-pipeline work (C1041 CMake race, MSVC
linker, path-length limit, `bin/` vs `lib/` DLL staging). Does **not**
confirm GPU offload — see the TCC limitation above. A real Windows
Vulkan-vs-CPU comparison, matching the Linux table, is still open work
pending a WDDM-capable VM series.

## Verified baseline (2026-08-02, v0.6.4, CUDA feature, commit ca85ff1)

TCC mode blocks Vulkan (see above) but is exactly what CUDA compute wants —
this closes the open item above without needing a WDDM-capable VM series.
Built from source on the same `Standard_NC4as_T4_v3` VM (`CARGO_FEATURES=cuda
DYNAMIC_BACKENDS=1`, no CI equivalent — CI has no CUDA Toolkit), installed
the resulting `.msi` natively, chat against a synthetic single-account DB:

| | tok/s | Notes |
|---|---|---|
| CPU fallback | 4.5 | measured earlier in the same session, stale-install artifact (see below) |
| **Tesla T4 via CUDA** | **7.8–8.2** | `nvidia-smi` confirmed ~4.7 GB resident + a 73% utilization spike during generation; app log: `llamacpp: chat model offload — CUDA0 (CUDA) has 16.9 GB free — offloading all layers` |

Real GPU offload, confirmed three independent ways (VRAM allocation, a
utilization spike, and an explicit backend-selection log line) — but
noticeably below the Linux Vulkan baseline (33–36 tok/s) for the same
model/quant on the same GPU. Not yet root-caused; plausible factors include
CUDA kernel/arch selection defaults (no explicit `CMAKE_CUDA_ARCHITECTURES`
was set — nvcc's own default was used), or genuine CUDA-vs-Vulkan backend
performance differences in ggml. Worth a follow-up if CUDA becomes a shipped
release feature rather than a one-off validation.

**Building this from source surfaced new gotchas**, all specific to a
first-ever local (non-CI) `DYNAMIC_BACKENDS=1 CARGO_FEATURES=cuda` Windows
build:

- **`npm ci` fails silently under a SYSTEM-run Scheduled Task** (the
  Job-Object-escape technique used for all long-running detached builds —
  see the pattern elsewhere in this doc) but succeeds instantly run directly
  over SSH as the interactive user. Root cause not identified; workaround is
  to run `npm ci` once directly, then have the build script skip it when
  `node_modules` already exists.
- **The CUDA Toolkit's narrow silent-install component list
  (`nvcc_13.0 cudart_13.0 cublas_13.0 ...`) can produce an install missing
  the entire `include\crt\` header directory** (`cuda_runtime.h`'s
  `#include <crt/host_config.h>` then fails with C1083 during CMake's CUDA
  compiler detection). Reinstalling with `cuda_installer.exe -s` (full
  default component set, no explicit list) fixed it.
- **A stale CMake build directory from a pre-fix failed attempt can wedge a
  later, correctly-configured retry**: CMake sees `CMakeCache.txt` and
  assumes "already configured," but `build.ninja` was never generated by the
  failed run, so the actual `cmake --build` step fails with `loading
  'build.ninja': The system cannot find the file specified`. Delete
  `<CARGO_TARGET_DIR>\<target-triple>\release\build\llama-cpp-sys-2-*`
  before retrying after any earlier CMake-stage failure.
- **npm's optional-dependency resolution can simply fail to install the
  correct native binding for the host platform** even when running natively
  on that platform (a documented upstream npm bug,
  [npm/cli#4828](https://github.com/npm/cli/issues/4828)) — surfaced here as
  `rolldown`'s `@rolldown/binding-win32-x64-msvc` missing, breaking Vite's
  production build (`tauri build`'s `beforeBuildCommand`) with `Cannot find
  native binding`. Fix: `npm install <package>@<version> --no-save` for the
  specific missing optional dependency.
- **WiX's `candle.exe`/`light.exe` (bundled by `tauri-bundler`, not
  separately installed) crash immediately with exit `-2146232576` and a
  content-free ".NET Runtime ... This application could not be started"
  Application-log entry when run from the SYSTEM account's profile**
  (`C:\Windows\system32\config\systemprofile\AppData\Local\tauri\...`) —
  every earlier CUDA build attempt hit this since they all ran via a
  SYSTEM-privileged Scheduled Task. Ruled out first: file corruption (fresh
  re-download, tauri-bundler's own hash check passes), missing .NET
  Framework (2.0/3.5 SP1/4.8 all present and independently confirmed
  working via `ngen.exe`), Defender/Mark-of-the-Web, general WOW64 breakage,
  UAC/elevation. Fix: run the build as a normal user instead (`net user
  <user> <randompw>` then `schtasks /Create ... /RU <user> /RP <pw>`) so
  tauri-bundler's WiX cache lands under a normal `C:\Users\<user>\AppData\Local\tauri\`
  path — legacy .NET Framework 2.0/3.5 side-by-side assembly activation is
  known to behave unpredictably from the SYSTEM profile path specifically.
  This also incidentally confirmed this VM's actual OS is Windows 10 Pro
  (client), not Windows Server as `config-windows.sh`'s `IMAGE_URN` implied
  (`Get-WindowsFeature` isn't a client-Windows cmdlet, which is what first
  raised the question).
- **WiX also needs .NET Framework 3.5** (`Disabled with Payload Removed` by
  default on this VM), separately from the profile-path issue above — enable
  via `dism /online /Enable-Feature /FeatureName:NetFx3 /All /NoRestart`
  (needs a SYSTEM-run Scheduled Task too, direct SSH invocation gets `Access
  is denied`) and reboot.
- **A fresh MSI install can silently no-op** if Windows Installer sees the
  same `ProductCode`+`ProductVersion` already registered — this project's
  version string doesn't change between a Vulkan build and a CUDA build of
  the "same" `0.6.4`, so `msiexec /i ... /quiet` on top of an existing
  install just did nothing (confirmed by the installed `emailops.exe`'s
  `LastWriteTime` being from the *previous day's* Vulkan-test session, and
  its `backends\` folder containing `ggml-vulkan.dll` instead of the
  freshly-built `ggml-cuda.dll`). Always uninstall
  (`msiexec /X{productcode} /quiet`) before installing a same-version
  rebuild when validating a specific build's artifact, not just its
  installer's exit code.
