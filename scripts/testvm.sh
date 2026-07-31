#!/usr/bin/env bash
#
# Manage the Azure GPU test VM used to validate real Linux/Windows release
# artifacts on real hardware (CI runners have no GPU, so Vulkan offload can
# only ever be verified here).
#
# THE CENTRAL CONSTRAINT: the subscription's NCASv3_T4 quota is 4 vCPUs, and
# one Standard_NC4as_T4_v3 uses exactly 4. So there is room for *one* GPU test
# VM at a time — creating the Windows one requires destroying the Linux one
# and vice versa. That is why `destroy` snapshots before deleting: the Linux
# box carries ~1h of provisioning (NVIDIA driver, Vulkan SDK, Xvfb/x11vnc
# desktop, demo DB, 3GB chat model) that would otherwise be rebuilt each swap.
#
# The networking (VNet, static public IP, NSG) is deliberately NOT torn down
# with the VM: it is cheap to idle, it keeps the IP stable across rebuilds,
# and it means the NSG allowlist survives. Both platforms share it.
#
# Full runbook, costs, and known gotchas: docs/TEST-VMS.md
set -euo pipefail

RG="${EMAILOPS_TESTVM_RG:-EMAILOPS-LINUX-GPU-TEST-RG}"
LOCATION="${EMAILOPS_TESTVM_LOCATION:-westeurope}"
SIZE="${EMAILOPS_TESTVM_SIZE:-Standard_NC4as_T4_v3}"

VNET="emailops-gpu-testVNET"
SUBNET="emailops-gpu-testSubnet"
PUBLIC_IP="emailops-gpu-testPublicIP"
NSG="emailops-gpu-testNSG"

LINUX_VM="emailops-gpu-test"
WIN_VM="emailops-win-gpu"
# Windows rejects computer names over 15 chars, and `az vm create` defaults the
# computer name to the VM name — "emailops-win-gpu" is 16 and fails deployment
# with a bare InvalidParameter after the NIC has already been created.
WIN_COMPUTER_NAME="emailops-win"

LINUX_IMAGE="Canonical:0001-com-ubuntu-server-jammy:22_04-lts-gen2:latest"
WIN_IMAGE="MicrosoftWindowsDesktop:windows-11:win11-24h2-pro:latest"

# A fully provisioned Linux box measures ~26GB (13G /usr for the Vulkan SDK +
# driver + WebKit deps, 9.5G /home incl. the 3GB GGUF, 2.5G /opt for NVIDIA).
# 64GB leaves comfortable headroom for release artifacts and logs. Bump to 128
# only if you intend to build from source on the VM — `src-tauri/target` alone
# reached 10GB, and a debug build can reach 45GB.
DISK_GB="${EMAILOPS_TESTVM_DISK_GB:-64}"

err() { echo "ERROR: $*" >&2; exit 1; }
log() { echo "==> $*"; }

require_az() {
  command -v az >/dev/null 2>&1 || err "az CLI not found"
  az account show >/dev/null 2>&1 || err "not logged in — run 'az login'"
}

# Both VMs share one 4-vCPU quota slot. Refuse to create a second rather than
# letting Azure fail deployment halfway and leave a dangling NIC behind.
assert_no_gpu_vm() {
  local existing
  existing=$(az vm list -g "$RG" --query "[].name" -o tsv 2>/dev/null || true)
  if [ -n "$existing" ]; then
    err "a GPU VM already exists ($existing). Only one fits in the NCASv3_T4 quota — run '$0 destroy' first."
  fi
}

# Home/office IPs change. Re-point every SSH/VNC/RDP rule at the current
# public IP on each create so a rebuilt VM is reachable without hand-editing
# the NSG in the portal.
refresh_nsg_allowlist() {
  local myip
  myip=$(curl -fsS https://ifconfig.me) || err "could not determine current public IP"
  log "allowlisting $myip on the NSG (ssh/vnc/rdp)"
  az network nsg rule create -g "$RG" --nsg-name "$NSG" -n default-allow-ssh \
    --priority 1000 --access Allow --protocol Tcp --direction Inbound \
    --destination-port-ranges 22 --source-address-prefixes "$myip/32" -o none
  az network nsg rule create -g "$RG" --nsg-name "$NSG" -n allow-vnc \
    --priority 1001 --access Allow --protocol Tcp --direction Inbound \
    --destination-port-ranges 5901 --source-address-prefixes "$myip/32" -o none
  az network nsg rule create -g "$RG" --nsg-name "$NSG" -n allow-rdp \
    --priority 1002 --access Allow --protocol Tcp --direction Inbound \
    --destination-port-ranges 3389 --source-address-prefixes "$myip/32" -o none
}

latest_linux_snapshot() {
  az snapshot list -g "$RG" \
    --query "sort_by([?osType=='Linux'], &timeCreated)[-1].name" -o tsv 2>/dev/null
}

create_linux() {
  require_az; assert_no_gpu_vm
  local snap
  snap=$(latest_linux_snapshot || true)

  if [ "${1:-}" = "--fresh" ] || [ -z "$snap" ] || [ "$snap" = "None" ]; then
    log "creating Linux GPU VM from a clean $LINUX_IMAGE image"
    log "NOTE: a fresh box still needs provisioning (driver, models, demo DB) — see docs/TEST-VMS.md"
    az vm create -g "$RG" -n "$LINUX_VM" --image "$LINUX_IMAGE" --size "$SIZE" \
      --admin-username azureuser --generate-ssh-keys \
      --vnet-name "$VNET" --subnet "$SUBNET" \
      --public-ip-address "$PUBLIC_IP" --nsg "$NSG" \
      --os-disk-size-gb "$DISK_GB" -o none
  else
    log "restoring Linux GPU VM from snapshot: $snap (already fully provisioned)"
    local snap_id disk_name
    snap_id=$(az snapshot show -g "$RG" -n "$snap" --query id -o tsv)
    disk_name="${LINUX_VM}-osdisk-$(date +%Y%m%d%H%M%S)"
    # A managed disk restored from a snapshot cannot be smaller than the
    # snapshot's source disk, so this ignores DISK_GB by design.
    az disk create -g "$RG" -n "$disk_name" --source "$snap_id" \
      --sku Premium_LRS -o none
    az vm create -g "$RG" -n "$LINUX_VM" --attach-os-disk "$disk_name" --os-type linux \
      --size "$SIZE" --vnet-name "$VNET" --subnet "$SUBNET" \
      --public-ip-address "$PUBLIC_IP" --nsg "$NSG" -o none
  fi

  log "installing the NVIDIA driver extension"
  az vm extension set -g "$RG" --vm-name "$LINUX_VM" -n NvidiaGpuDriverLinux \
    --publisher Microsoft.HpcCompute --version 1.11 --no-wait -o none
  refresh_nsg_allowlist
  status
}

create_windows() {
  require_az; assert_no_gpu_vm
  local pw
  # Never hand-write a password for an internet-reachable host. Printed once
  # here and not persisted by this script — put it in your password manager.
  pw="$(openssl rand -base64 18 | tr -d '=+/')Aa1!"

  log "creating Windows 11 GPU VM"
  az vm create -g "$RG" -n "$WIN_VM" --computer-name "$WIN_COMPUTER_NAME" \
    --image "$WIN_IMAGE" --size "$SIZE" \
    --admin-username azureuser --admin-password "$pw" \
    --vnet-name "$VNET" --subnet "$SUBNET" \
    --public-ip-address "$PUBLIC_IP" --nsg "$NSG" \
    --security-type Standard --license-type Windows_Client \
    --os-disk-size-gb 128 -o none

  log "installing the NVIDIA driver extension"
  az vm extension set -g "$RG" --vm-name "$WIN_VM" -n NvidiaGpuDriverWindows \
    --publisher Microsoft.HpcCompute --version 1.6 --no-wait -o none
  refresh_nsg_allowlist
  status
  echo ""
  echo "  RDP: $(az network public-ip show -g "$RG" -n "$PUBLIC_IP" --query ipAddress -o tsv)"
  echo "  user: azureuser"
  echo "  password: $pw"
  echo "  ^ shown once — store it in your password manager now."
}

# Snapshot-then-delete. The snapshot is what makes the next rebuild a
# one-command restore instead of an hour of provisioning, so it is not
# optional: pass --no-snapshot only when the box holds nothing worth keeping.
destroy() {
  require_az
  local vm
  vm=$(az vm list -g "$RG" --query "[0].name" -o tsv 2>/dev/null || true)
  [ -n "$vm" ] && [ "$vm" != "None" ] || err "no VM found in $RG"

  local os_type disk_id
  os_type=$(az vm show -g "$RG" -n "$vm" --query "storageProfile.osDisk.osType" -o tsv)
  disk_id=$(az vm show -g "$RG" -n "$vm" --query "storageProfile.osDisk.managedDisk.id" -o tsv)

  if [ "${1:-}" != "--no-snapshot" ]; then
    local snap_name="emailops-gpu-$(echo "$os_type" | tr '[:upper:]' '[:lower:]')-$(date +%Y%m%d)"
    log "deallocating first so the snapshot is a clean point-in-time"
    az vm deallocate -g "$RG" -n "$vm" -o none
    log "snapshotting OS disk -> $snap_name"
    # Incremental + Standard_LRS: billed on used capacity, not the 128GB
    # nominal size, and survives deletion of the source disk.
    az snapshot create -g "$RG" -n "$snap_name" --source "$disk_id" \
      --incremental true --sku Standard_LRS -o none
  else
    log "skipping snapshot (--no-snapshot)"
  fi

  log "deleting VM, OS disk, and NIC (VNet/IP/NSG are kept on purpose)"
  az vm delete -g "$RG" -n "$vm" --yes -o none
  az disk delete -g "$RG" --ids "$disk_id" --yes -o none
  for nic in $(az network nic list -g "$RG" --query "[].name" -o tsv); do
    az network nic delete -g "$RG" -n "$nic" -o none
  done
  status
}

start()  { require_az; az vm start      -g "$RG" -n "$(az vm list -g "$RG" --query '[0].name' -o tsv)" -o none; refresh_nsg_allowlist; status; }
stop()   { require_az; az vm deallocate -g "$RG" -n "$(az vm list -g "$RG" --query '[0].name' -o tsv)" -o none; status; }

status() {
  require_az
  echo ""
  echo "Resource group: $RG ($LOCATION)"
  az vm list -d -g "$RG" -o table 2>/dev/null || true
  echo "Snapshots:"
  az snapshot list -g "$RG" --query "[].{name:name,os:osType,created:timeCreated}" -o table 2>/dev/null || true
  echo "T4 quota:"
  az vm list-usage -l "$LOCATION" --query "[?contains(localName,'NCASv3_T4')].{name:localName,used:currentValue,limit:limit}" -o table 2>/dev/null || true
}

usage() {
  cat <<EOF
Usage: $0 <command>

  create-linux [--fresh]   Restore the Linux GPU VM from the newest snapshot
                           (fully provisioned). --fresh forces a clean image.
  create-windows           Create the Windows 11 GPU VM. Prints a generated
                           admin password once.
  destroy [--no-snapshot]  Snapshot the OS disk, then delete VM + disk + NIC.
                           Keeps VNet/IP/NSG for the next VM.
  start | stop             Start / deallocate (stop billing compute).
  status                   VMs, snapshots, and remaining T4 quota.

Only one GPU VM fits in the NCASv3_T4 quota — see docs/TEST-VMS.md
EOF
}

case "${1:-}" in
  create-linux)   shift; create_linux "$@" ;;
  create-windows) shift; create_windows "$@" ;;
  destroy)        shift; destroy "$@" ;;
  start)          start ;;
  stop)           stop ;;
  status)         status ;;
  *)              usage; exit 1 ;;
esac
