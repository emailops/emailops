---
title: 'Installation'
description: 'Download and install EmailOps on macOS, Windows or Linux.'
weight: 10
---

## System requirements

<!-- claim:inst-system-requirements-1 -->
Running the AI **locally** is the part that needs more powerful hardware, and it is optional.
You can decline it in the first-run wizard and run EmailOps as a plain email client, or keep
every AI feature and route it to a remote provider instead. The two modes have very different
requirements.

### With local AI {#with-local-ai}

<!-- claim:inst-system-requirements-local-ai-1 -->
One of the most important requirements for running local AI is the memory available to load
the model and its context. Depending on your machine, that means one type of memory or
another:

| | Apple Silicon Mac | Windows / Linux |
|---|---|---|
| Runs the model on | The built-in GPU, via Metal | Your GPU, via Vulkan — or the CPU if there is no GPU |
| Memory it has to fit in | Unified memory, shared with the system | The GPU's **VRAM**, or system RAM when running on CPU |
| Minimum | 8 GB unified | 8 GB VRAM, or 16 GB system RAM with no GPU |
| Recommended | 16 GB unified or more | 12–16 GB VRAM |
| Free disk | ~3 GB — app plus the default chat model | ~3 GB — app plus the default chat model |

<!-- claim:inst-system-requirements-local-ai-2 -->
**Sizing rule:** the model has to fit, whole, in whatever memory it runs in. The default
**Qwen 3.5 4B** needs about 8 GB before the app offers it (it uses under 4 GB while
answering); the largest model in the catalog wants 32 GB. Every model's
figure is in the [model catalog](../ai-features/#the-model-catalog).

- **Apple Silicon** has unified memory — the GPU addresses the same pool as the CPU, so the
  number to compare against is your total system memory. A 16 GB Mac runs models up to the
  16 GB row comfortably, minus what macOS and your other apps are already using. <!-- claim:inst-system-requirements-local-ai-3 -->
- **A GPU on Windows or Linux** has its own separate VRAM, and that is the number that
  counts — 32 GB of system RAM does not help if the card only has 8 GB. A model that does not
  fit spills to the CPU, which works but is several times slower. <!-- claim:inst-system-requirements-local-ai-4 -->
- **No GPU at all** is supported and needs no different download. The app falls back to the
  CPU and system RAM; budget the model's figure in RAM and expect answers to take noticeably
  longer. <!-- claim:inst-system-requirements-local-ai-5 -->

<!-- claim:inst-system-requirements-local-ai-6 -->
Intel Macs are the exception: the embedded AI runtime needs an Apple Silicon chip (M1 or
newer) and cannot run on them at all — see the [note below](#direct-download).

### Without local AI

<!-- claim:inst-system-requirements-without-local-1 -->
| | Minimum | Recommended |
|---|---|---|
| RAM | 2 GB | 4 GB |
| Free disk | ~500 MB, plus room for the mail you sync | Depends on your mailbox size |
| Processor | 64-bit, 2 cores | — |
| Graphics | None | None |

<!-- claim:inst-system-requirements-without-local-2 -->
These are the requirements in two cases: AI switched off entirely, and AI switched **on but
routed to OpenRouter**. Remote inference happens on someone else's hardware, so an old laptop
is enough — at the cost of an API key, a per-use fee, and your email content leaving the
device. See [choosing a backend](../ai-features/#choosing-a-backend).

### Operating system

<!-- claim:inst-system-requirements-operating-system-1 -->
Both modes need one of:

- **macOS** Monterey (12) or later — Apple Silicon or Intel. <!-- claim:inst-system-requirements-operating-system-2 -->
- **Windows** 10 or 11, 64-bit. <!-- claim:inst-system-requirements-operating-system-3 -->
- **Linux** 64-bit, with WebKitGTK and a Secret Service keyring — see
  [Linux](#linux) below. <!-- claim:inst-system-requirements-operating-system-4 -->

## macOS

### Homebrew

<!-- claim:inst-macos-homebrew-1 -->
```bash
brew install --cask emailops/tap/emailops
```

<!-- claim:inst-macos-homebrew-2 -->
Upgrade later with `brew upgrade --cask emailops`.

### Direct download {#direct-download}

1. Download **EmailOps-macos.dmg** from the
   [latest release](https://github.com/emailops/emailops/releases/latest). <!-- claim:inst-macos-direct-download-1 -->
2. Open the DMG and drag **EmailOps.app** into your Applications folder. <!-- claim:inst-macos-direct-download-2 -->
3. Launch it from Applications. <!-- claim:inst-macos-direct-download-3 -->

<!-- claim:inst-macos-direct-download-4 -->
> **Intel Macs:** this one download works on every Mac — there is no separate Intel build.
> The AI features are the exception: the in-app AI needs an Apple Silicon chip (M1 or newer),
> so on an Intel Mac it stays switched off and EmailOps tells you why. Everything else works
> normally. For AI, point EmailOps at
> [OpenRouter](../ai-features/#choosing-a-backend) instead.

## Windows

1. Download **EmailOps-windows-setup.exe** from the
   [latest release](https://github.com/emailops/emailops/releases/latest). <!-- claim:inst-windows-1 -->
2. Run the installer and follow the prompts. <!-- claim:inst-windows-2 -->
3. Launch EmailOps from the Start menu. <!-- claim:inst-windows-3 -->

### "Windows protected your PC" {#smartscreen}

<!-- claim:inst-windows-smartscreen-1 -->
When you run the installer, Windows may show a blue **Microsoft Defender SmartScreen** screen
saying *"Windows protected your PC"*. This is expected: the installer is not code-signed yet,
because signing on Windows needs a paid certificate the project does not have. The warning
says nothing about the file itself. SmartScreen shows it for every unsigned download it has
not seen often.

<!-- claim:inst-windows-smartscreen-2 -->
To continue:

1. Click **More info**. <!-- claim:inst-windows-smartscreen-3 -->
2. Check that the app name is **EmailOps**, then click **Run anyway**. <!-- claim:inst-windows-smartscreen-4 -->

<!-- claim:inst-windows-smartscreen-5 -->
If you want to confirm the download is the real one first, compare its SHA-256 hash
(`Get-FileHash .\EmailOps-windows-setup.exe` in PowerShell) against the checksums on the
[release page](https://github.com/emailops/emailops/releases/latest).

### GPU acceleration

<!-- claim:inst-windows-gpu-acceleration-1 -->
There is nothing extra to install. The Windows version carries a **Vulkan** backend that loads
at runtime whenever a working graphics driver is present, and falls back to the CPU when it
is not — one download either way.

<!-- claim:inst-windows-gpu-acceleration-2 -->
Vulkan was chosen over CUDA precisely so this stays simple: it covers AMD, Intel and NVIDIA
through the graphics driver you already have, with no vendor toolkit to install. Keep your
GPU driver reasonably current and it works.

## Linux {#linux}

1. Download **EmailOps-linux.AppImage** from the
   [latest release](https://github.com/emailops/emailops/releases/latest). <!-- claim:inst-linux-1 -->
2. Make it executable and run it: <!-- claim:inst-linux-2 -->

```bash
chmod +x EmailOps-linux.AppImage
./EmailOps-linux.AppImage
```

### GPU acceleration

<!-- claim:inst-linux-gpu-acceleration-1 -->
Same as on Windows: the AppImage carries a **Vulkan** backend that is used automatically when
a graphics driver is present and falls back to the CPU when it is not. No CUDA toolkit, no
vendor SDK, no separate build to pick.

<!-- claim:inst-linux-gpu-acceleration-2 -->
What you need is the ordinary Vulkan driver stack for your card — `mesa-vulkan-drivers` on
AMD and Intel, the proprietary NVIDIA driver on NVIDIA — which most desktop distributions
already install. If `vulkaninfo` reports a device, EmailOps will use it.

### A keyring is required

<!-- claim:inst-linux-keyring-required-1 -->
EmailOps never writes account credentials to a file — OAuth tokens and IMAP passwords go to
the system credential store. macOS and Windows ship one (Keychain and Credential Manager);
on Linux you have to provide one yourself.

<!-- claim:inst-linux-keyring-required-2 -->
You need a **Secret Service** provider installed and unlocked. Any of these work:

- **GNOME Keyring** (`gnome-keyring`) — the default on GNOME, Ubuntu, Fedora Workstation. <!-- claim:inst-linux-keyring-required-3 -->
- **KWallet** (`kwalletmanager` with the Secret Service interface) — the KDE equivalent. <!-- claim:inst-linux-keyring-required-4 -->
- **KeePassXC** with *Settings → Secret Service Integration* enabled. <!-- claim:inst-linux-keyring-required-5 -->

<!-- claim:inst-linux-keyring-required-6 -->
On a minimal window manager or a headless session there is often no keyring running. Install
one of the above and make sure it is unlocked when EmailOps starts — otherwise adding an
account fails, because there is nowhere safe to put the credentials.

<!-- claim:inst-linux-keyring-required-7 -->
```bash
# Debian / Ubuntu
sudo apt install gnome-keyring

# Fedora
sudo dnf install gnome-keyring

# Arch
sudo pacman -S gnome-keyring
```

## Where your data lives

<!-- claim:inst-where-data-1 -->
Everything EmailOps stores is on your machine, in your OS application data directory:

- **Mail, contacts, calendar events, embeddings** — a local SQLite database. <!-- claim:inst-where-data-2 -->
- **Downloaded AI models** — a `models/` folder next to the database. <!-- claim:inst-where-data-3 -->
- **OAuth tokens and passwords** — your OS keychain, never a plain file. <!-- claim:inst-where-data-4 -->

<!-- claim:inst-where-data-5 -->
To move or share a data directory (for testing, or a second profile), set the
`EMAILOPS_DATA_DIR` environment variable before launching. The exact paths per platform, and
what is written where, are in [Privacy & security](../privacy-security/#where-your-data-is-stored).

## Uninstalling

<!-- claim:inst-uninstalling-1 -->
Removing the app leaves your mail database and downloaded models behind on purpose, so a
reinstall picks up where you left off. Delete the data directory too for a clean slate.

<!-- claim:inst-uninstalling-2 -->
Nothing is removed from your mail provider either way — uninstalling EmailOps never touches
the mail on Gmail, Outlook or your IMAP server.

### macOS

<!-- claim:inst-uninstalling-macos-1 -->
With Homebrew, one command removes both the app and its data:

```bash
brew uninstall --zap --cask emailops
```

<!-- claim:inst-uninstalling-macos-2 -->
Without `--zap`, only the app goes. To do it by hand: drag **EmailOps.app** from Applications
to the Trash, then delete:

```
~/Library/Application Support/com.emailops.app
~/Library/Caches/com.emailops.app
~/Library/HTTPStorages/com.emailops.app
~/Library/Preferences/com.emailops.app.plist
~/Library/Saved Application State/com.emailops.app.savedState
~/Library/WebKit/com.emailops.app
```

### Windows

<!-- claim:inst-uninstalling-windows-1 -->
Uninstall from **Settings → Apps → Installed apps → EmailOps**, or run the uninstaller from
the Start menu entry. Then delete the data directory:

```
%APPDATA%\com.emailops.app
```

### Linux

<!-- claim:inst-uninstalling-linux-1 -->
Delete the AppImage file. Then delete the data and config directories:

```bash
rm -rf ~/.local/share/com.emailops.app
rm -rf ~/.config/com.emailops.app
```

### Stored credentials

<!-- claim:inst-uninstalling-stored-credentials-1 -->
On every platform, OAuth tokens and IMAP passwords live in the system keyring rather than in
the data directory, so they survive all of the above. Remove the `com.emailops.app` entries
from Keychain Access (macOS), Credential Manager (Windows) or your keyring manager (Linux) if
you want them gone as well.

## Building from source

<!-- claim:inst-building-from-1 -->
If you would rather build it yourself, the repository README covers the Rust + Node
toolchain, the Tauri prerequisites and the `make dev` workflow. Note that source builds need
your **own** Gmail / Microsoft OAuth credentials in `.env.local`; the released binaries ship
with credentials already configured.

<!-- claim:inst-building-from-2 -->
Two build notes on the AI runtime:

- Windows and Linux releases are built with `DYNAMIC_BACKENDS=1` and `CARGO_FEATURES=vulkan`,
  which is what produces a single artifact that picks up a GPU at runtime. Building the
  Vulkan backend needs the Vulkan SDK — a build-time dependency only; users never install it. <!-- claim:inst-building-from-3 -->
- A `cuda` Cargo feature builds an NVIDIA-only variant. The release pipeline publishes it
  for Windows as a separate installer, `EmailOps-windows-cuda.msi`, alongside the default
  Vulkan build, which covers every GPU vendor. <!-- claim:inst-building-from-4 -->
