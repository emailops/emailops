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
