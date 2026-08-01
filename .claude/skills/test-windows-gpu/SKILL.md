---
name: test-windows-gpu
description: Provision (or reuse) the Azure Windows GPU test VM, install the latest Windows release artifact, get the chat feature working without real email credentials, send a test chat message, verify whether GPU/Vulkan offload actually engaged (vs CPU fallback), capture a screenshot of the running app, send it to the user, and offer to clean up the VM afterward.
argument-hint: "[ci-run-id] (optional — defaults to the latest successful release.yml run)"
disable-model-invocation: true
allowed-tools: Bash, Read
---

# Test Windows GPU offload end-to-end

Validates that a real Windows release build actually runs and chats on real
GPU hardware — the one thing CI can never check (GH-hosted runners have no
GPU). Full background, gotchas, and the Linux equivalent of this flow live in
`docs/TEST-VMS.md`; this skill is the Windows runbook made runnable. Read
that file first if anything here is unclear or a step fails in a new way —
update it with what you learn, the same way this skill was itself written
from the first real run.

**Ask before any destructive/costly step you're not sure about** (provisioning
a VM, deleting a snapshot, wiping the app's DB). This flow is expensive
(~15-20 min VM provisioning, GPU-hour billing) and semi-manual (RDP-driven UI
automation) — don't silently retry a failed step more than twice; stop and
report what happened instead.

## Phase 1 — Provision or reuse the VM

```bash
bash scripts/testvm.sh status
```

Only one GPU VM (Linux or Windows) fits in the subscription's T4 quota. If a
**Linux** VM exists, stop and ask the user whether to destroy it first
(`make testvm-destroy` snapshots before deleting, so nothing is lost) — never
destroy it unprompted. If a **Windows** VM already exists, skip to Phase 2.
Otherwise:

```bash
make testvm-windows
```

This prints a generated admin password once — capture it from the command
output for Phase 3, it is not stored anywhere.

## Phase 2 — Enable SSH

Fresh Windows VMs don't have OpenSSH running. Use the control-plane channel
(`az vm run-command`, works with zero inbound ports open) to bootstrap it —
reuses the keypair already checked into `private-scripts/`:

```bash
PUB_KEY="$(cat private-scripts/emailops_gpu_test_key.pub)"
az vm run-command invoke -g EMAILOPS-LINUX-GPU-TEST-RG -n emailops-win-gpu \
  --command-id RunPowerShellScript --scripts @private-scripts/enable-ssh.ps1 \
  --parameters "pubKey=${PUB_KEY}"
```

If this errors with `Conflict: Run command extension execution is in
progress`, the NVIDIA driver extension (or a previous attempt) is still
installing — `az vm extension list` may show stale "Creating" status; trust
`az vm get-instance-view --query instanceView.extensions` instead, and retry
enable-ssh once it shows `succeeded`. Can take 15-20 min on a fresh VM.

Test the connection, and if it times out, fix the firewall profile (a fresh
`enable-ssh.ps1` run defaults its rule to the Private profile, but Azure VMs
report NetworkCategory Public):

```bash
IP=$(az vm show -d -g EMAILOPS-LINUX-GPU-TEST-RG -n emailops-win-gpu --query publicIps -o tsv)
ssh-keygen -R "$IP"  # if the IP was reused by a previous, different VM
ssh -i private-scripts/emailops_gpu_test_key -o ConnectTimeout=15 -o StrictHostKeyChecking=accept-new azureuser@"$IP" "nvidia-smi"
# If that times out:
az vm run-command invoke -g EMAILOPS-LINUX-GPU-TEST-RG -n emailops-win-gpu \
  --command-id RunPowerShellScript \
  --scripts "Set-NetFirewallRule -Name OpenSSH-Server-In-TCP -Profile Any"
```

Confirm `nvidia-smi` shows a Tesla T4 before continuing.

## Phase 3 — Establish an interactive RDP session

**This is required, not optional.** GUI apps launched via SSH's
`Start-Process` run in a non-interactive window station on a VM nobody has
ever logged into — the process reports "Responding" but no window ever
renders, and `System.Drawing`'s `CopyFromScreen` throws `The handle is
invalid`. A real interactive session (RDP) is the only way to get a
renderable desktop and drive/screenshot the actual app.

```bash
docker run --rm -d --name rdp-capture debian:bookworm-slim sleep 3600
docker exec rdp-capture bash -c "apt-get update -qq && apt-get install -y -qq xvfb freerdp2-x11 imagemagick xdotool"
docker exec -d rdp-capture bash -c "Xvfb :1 -screen 0 1600x1000x24 > /tmp/xvfb.log 2>&1"
sleep 2
docker exec rdp-capture bash -c "DISPLAY=:1 xfreerdp /v:$IP /u:azureuser /p:'<password from Phase 1>' /cert:ignore /w:1600 /h:1000 &"
sleep 8
```

Screenshot helper (use this pattern throughout the rest of the skill):

```bash
docker exec rdp-capture bash -c "DISPLAY=:1 import -window root /tmp/shot.png"
docker cp rdp-capture:/tmp/shot.png <scratchpad>/shot_NN.png
```

Then `Read` the PNG to see the current screen state before deciding the next
click. Click/type via:

```bash
docker exec rdp-capture bash -c "DISPLAY=:1 xdotool mousemove <x> <y> click 1"
docker exec rdp-capture bash -c "DISPLAY=:1 xdotool type --delay 50 '<text>'"
```

If a fresh VM shows Windows OOBE (privacy settings, "Choose privacy
settings" → Accept) instead of a desktop, click through it — this only
happens once per VM lifetime, not on every RDP reconnect.

## Phase 4 — Get the release artifact onto the VM

Prefer downloading the already-CI-built `.msi` over rebuilding from source on
the VM (a full `DYNAMIC_BACKENDS=1 CARGO_FEATURES=vulkan` Windows build takes
~45 min even when it succeeds). Find the latest successful `release.yml` run
(or use `$0` if the user passed a specific run id) and generate a short-lived
signed URL — same reasoning as the Linux flow in `docs/TEST-VMS.md`: avoids
putting a GitHub token on the VM:

```bash
RUN_ID="${0:-$(gh run list --workflow=release.yml --status success --limit 1 --json databaseId --jq '.[0].databaseId')}"
ARTIFACT_ID=$(gh api "repos/emailops/emailops/actions/runs/$RUN_ID/artifacts" --jq '.artifacts[] | select(.name=="emailops-windows-dry-run") | .id')
URL=$(curl -s -o /dev/null -D - -H "Authorization: token $(gh auth token)" \
  "https://api.github.com/repos/emailops/emailops/actions/artifacts/$ARTIFACT_ID/zip" \
  | grep -i '^location:' | sed 's/^[Ll]ocation: //' | tr -d '\r\n')
ssh -i private-scripts/emailops_gpu_test_key azureuser@"$IP" \
  "Invoke-WebRequest -Uri '$URL' -OutFile C:\art.zip; Expand-Archive -Path C:\art.zip -DestinationPath C:\art -Force"
```

**Install the VC++ Redistributable before the app** — the raw `.msi` (unlike
the NSIS `.exe` installer) does not bootstrap it, and every launch dies
silently with exit code `-1073741515` (`STATUS_DLL_NOT_FOUND`) otherwise:

```bash
ssh -i private-scripts/emailops_gpu_test_key azureuser@"$IP" "Invoke-WebRequest -Uri 'https://aka.ms/vs/17/release/vc_redist.x64.exe' -OutFile C:\vc_redist.x64.exe; Start-Process -FilePath C:\vc_redist.x64.exe -ArgumentList '/install','/quiet','/norestart' -Wait"
ssh -i private-scripts/emailops_gpu_test_key azureuser@"$IP" "Start-Process msiexec.exe -ArgumentList '/i C:\art\EmailOps-windows.msi /quiet /qn /norestart' -Wait"
```

## Phase 5 — First launch, onboarding, unlock chat

Launch **from the RDP session** (double-click the desktop icon via
`xdotool`), not via SSH — see Phase 3. Click through onboarding: "Use AI" is
pre-selected on step 1 (Continue); step 2 downloads/selects the recommended
chat model (`Qwen 3.5 4B`, ~3 GB — wait for "✓ Ready" before continuing, this
takes a few minutes); step 3 is layout (Continue); step 4 is account setup —
click **"Skip for now"**, since OAuth/IMAP need real credentials that don't
belong on a throwaway VM.

Chat needs at least one account row to unlock (even a non-functional one).
Close the app, then insert a synthetic account directly into its DB:

```bash
ssh -i private-scripts/emailops_gpu_test_key azureuser@"$IP" "Get-Process emailops -ErrorAction SilentlyContinue | Stop-Process -Force"
# sqlite3.exe from https://sqlite.org/download.html if not already present on the VM
ssh -i private-scripts/emailops_gpu_test_key azureuser@"$IP" '$now = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds(); & C:\sqlite\sqlite3.exe "$env:APPDATA\com.emailops.app\emailops.db" "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) VALUES (''test-account-1'', ''imap'', ''test@example.test'', ''GPU Test Account'', $now, 0, 1);"'
```

Relaunch (from the RDP session again) — the account now appears in the
sidebar (with an expected "Authentication required" banner, since it has no
real credentials) and **Chat** unlocks in AI FEATURES.

If instead you see "EmailOps could not start because it failed to open its
local database" / "DB migration failed", the DB's schema doesn't match this
exact build (e.g. if you tried copying in `make demo-db`'s output, which is
schema-versioned against whatever your local dev DB happens to be, not this
specific release commit) — delete just `emailops.db*` (keep `models/`, it's
3 GB and already downloaded) and let the app create a fresh one on next
launch instead.

## Phase 6 — Send a chat message, verify GPU offload

Click into the chat input, type a short prompt (`xdotool type`), send it.
While it's generating, poll GPU utilization in parallel:

```bash
for i in 1 2 3 4 5; do
  ssh -i private-scripts/emailops_gpu_test_key azureuser@"$IP" \
    "nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader"
  sleep 1
done
```

Screenshot the finished response — the assistant message footer shows
`<model> · <N> tokens · <T>s · <X> tok/s`. Compare against
`docs/TEST-VMS.md`'s verified baselines (~1.5-3 tok/s CPU fallback vs 33-36
tok/s real GPU/Vulkan on a T4). If `nvidia-smi` stayed at `0 %, 9 MiB`
throughout and tok/s matches the CPU range, GPU offload did not engage —
check `nvidia-smi -q | Select-String -Context 0,2 'Driver Model'`. If it
reads **TCC**, this is not a bug: `Standard_NC4as_T4_v3` cannot run Vulkan
(TCC has no WDDM/Vulkan ICD, and `nvidia-smi -dm 0` returns `Not Supported`
on this SKU) — report this as a known VM-series limitation, not a failure of
the app or the release build. See `docs/TEST-VMS.md`'s "Windows GPU
limitation" section before spending time debugging it further.

## Phase 7 — Send results to the user

`SendUserFile` the clearest single screenshot — ideally one showing both the
chat response (with the tok/s line) and, if you expanded it, the app's
Output panel (bottom bar) showing the `llamacpp` log lines. Caption it with:
what was tested, the tok/s achieved, and — explicitly — whether GPU offload
was confirmed or whether this run only proved CPU-fallback + app
functionality (state the TCC limitation plainly if that's what happened,
don't bury it).

## Phase 8 — Offer cleanup

**Always ask, never assume.** Use `AskUserQuestion` (or just ask directly) —
options along the lines of:

- **Stop (deallocate)** — `make testvm-stop`. Keeps the VM/disk, stops
  compute billing. Best if more testing is likely soon.
- **Destroy (snapshot then delete)** — `make testvm-destroy`. Snapshots the
  OS disk first (so a future `create-windows` — once snapshot-restore exists
  for Windows — or manual disk-attach can resume from it), then deletes
  VM + disk + NIC. Best if no more Windows GPU testing is planned soon.
- **Leave it running** — only if the user says so explicitly; it bills by
  the hour (`Standard_NC4as_T4_v3`).

Also stop the local Docker RDP helper regardless of which option is chosen:

```bash
docker rm -f rdp-capture
```
