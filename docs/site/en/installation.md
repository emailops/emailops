---
title: 'Installation'
description: 'Download and install EmailOps on macOS, Windows or Linux.'
weight: 10
---

## System requirements

Running the AI **locally** is the part that needs more powerful hardware, and it is optional.
You can decline it in the first-run wizard and run EmailOps as a plain email client, or keep
every AI feature and route it to a remote provider instead. The two modes have very different
requirements.

### With local AI {#with-local-ai}

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

**Sizing rule:** the model has to fit, whole, in whatever memory it runs in. The default
**Qwen 3.5 4B** needs about 8 GB; the largest model in the catalog wants 32 GB. Every model's
figure is in the [model catalog](../ai-features/#the-model-catalog).

- **Apple Silicon** has unified memory — the GPU addresses the same pool as the CPU, so the
  number to compare against is your total system memory. A 16 GB Mac runs models up to the
  16 GB row comfortably, minus what macOS and your other apps are already using.
- **A GPU on Windows or Linux** has its own separate VRAM, and that is the number that
  counts — 32 GB of system RAM does not help if the card only has 8 GB. A model that does not
  fit spills to the CPU, which works but is several times slower.
- **No GPU at all** is supported and needs no different download. The app falls back to the
  CPU and system RAM; budget the model's figure in RAM and expect answers to take noticeably
  longer.

Intel Macs are the exception: that build ships without the embedded AI runtime altogether —
see the [note below](#direct-download).

### Without local AI

| | Minimum | Recommended |
|---|---|---|
| RAM | 2 GB | 4 GB |
| Free disk | ~500 MB, plus room for the mail you sync | Depends on your mailbox size |
| Processor | 64-bit, 2 cores | — |
| Graphics | None | None |

These are the requirements in two cases: AI switched off entirely, and AI switched **on but
routed to OpenRouter**. Remote inference happens on someone else's hardware, so an old laptop
is enough — at the cost of an API key, a per-use fee, and your email content leaving the
device. See [choosing a backend](../ai-features/#choosing-a-backend).

### Operating system

Both modes need one of:

- **macOS** Monterey (12) or later — Apple Silicon or Intel.
- **Windows** 10 or 11, 64-bit.
- **Linux** 64-bit, with WebKitGTK and a Secret Service keyring — see
  [Linux](#linux) below.

## macOS

### Homebrew

```bash
brew install --cask emailops/tap/emailops
```

Upgrade later with `brew upgrade --cask emailops`.

### Direct download {#direct-download}

1. Download **EmailOps-macos.dmg** from the
   [latest release](https://github.com/emailops/emailops/releases/latest).
2. Open the DMG and drag **EmailOps.app** into your Applications folder.
3. Launch it from Applications.

> **Intel Macs:** use the `EmailOps-macos-intel.dmg` build. It does not include the embedded
> AI runtime, so if you want AI features on an Intel Mac, point EmailOps at
> [Ollama or OpenRouter](../ai-features/#choosing-a-backend) instead.

## Windows

1. Download **EmailOps-windows.exe** from the
   [latest release](https://github.com/emailops/emailops/releases/latest).
2. Run the installer and follow the prompts.
3. Launch EmailOps from the Start menu.

### GPU acceleration

There is nothing extra to install. The Windows version carries a **Vulkan** backend that loads
at runtime whenever a working graphics driver is present, and falls back to the CPU when it
is not — one download either way.

Vulkan was chosen over CUDA precisely so this stays simple: it covers AMD, Intel and NVIDIA
through the graphics driver you already have, with no vendor toolkit to install. Keep your
GPU driver reasonably current and it works.

## Linux {#linux}

1. Download **EmailOps-linux.AppImage** from the
   [latest release](https://github.com/emailops/emailops/releases/latest).
2. Make it executable and run it:

```bash
chmod +x EmailOps-linux.AppImage
./EmailOps-linux.AppImage
```

### GPU acceleration

Same as on Windows: the AppImage carries a **Vulkan** backend that is used automatically when
a graphics driver is present and falls back to the CPU when it is not. No CUDA toolkit, no
vendor SDK, no separate build to pick.

What you need is the ordinary Vulkan driver stack for your card — `mesa-vulkan-drivers` on
AMD and Intel, the proprietary NVIDIA driver on NVIDIA — which most desktop distributions
already install. If `vulkaninfo` reports a device, EmailOps will use it.

### A keyring is required

EmailOps never writes account credentials to a file — OAuth tokens and IMAP passwords go to
the system credential store. macOS and Windows ship one (Keychain and Credential Manager);
on Linux you have to provide one yourself.

You need a **Secret Service** provider installed and unlocked. Any of these work:

- **GNOME Keyring** (`gnome-keyring`) — the default on GNOME, Ubuntu, Fedora Workstation.
- **KWallet** (`kwalletmanager` with the Secret Service interface) — the KDE equivalent.
- **KeePassXC** with *Settings → Secret Service Integration* enabled.

On a minimal window manager or a headless session there is often no keyring running. Install
one of the above and make sure it is unlocked when EmailOps starts — otherwise adding an
account fails, because there is nowhere safe to put the credentials.

```bash
# Debian / Ubuntu
sudo apt install gnome-keyring

# Fedora
sudo dnf install gnome-keyring

# Arch
sudo pacman -S gnome-keyring
```

## Where your data lives

Everything EmailOps stores is on your machine, in your OS application data directory:

- **Mail, contacts, calendar events, embeddings** — a local SQLite database.
- **Downloaded AI models** — a `models/` folder next to the database.
- **OAuth tokens and passwords** — your OS keychain, never a plain file.

To move or share a data directory (for testing, or a second profile), set the
`EMAILOPS_DATA_DIR` environment variable before launching. The exact paths per platform, and
what is written where, are in [Privacy & security](../privacy-security/#where-your-data-is-stored).

## Uninstalling

Removing the app leaves your mail database and downloaded models behind on purpose, so a
reinstall picks up where you left off. Delete the data directory too for a clean slate.

Nothing is removed from your mail provider either way — uninstalling EmailOps never touches
the mail on Gmail, Outlook or your IMAP server.

### macOS

With Homebrew, one command removes both the app and its data:

```bash
brew uninstall --zap --cask emailops
```

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

Uninstall from **Settings → Apps → Installed apps → EmailOps**, or run the uninstaller from
the Start menu entry. Then delete the data directory:

```
%APPDATA%\com.emailops.app
```

### Linux

Delete the AppImage file. Then delete the data and config directories:

```bash
rm -rf ~/.local/share/com.emailops.app
rm -rf ~/.config/com.emailops.app
```

### Stored credentials

On every platform, OAuth tokens and IMAP passwords live in the system keyring rather than in
the data directory, so they survive all of the above. Remove the `com.emailops.app` entries
from Keychain Access (macOS), Credential Manager (Windows) or your keyring manager (Linux) if
you want them gone as well.

## Building from source

If you would rather build it yourself, the repository README covers the Rust + Node
toolchain, the Tauri prerequisites and the `make dev` workflow. Note that source builds need
your **own** Gmail / Microsoft OAuth credentials in `.env.local`; the released binaries ship
with credentials already configured.

Two build notes on the AI runtime:

- Windows and Linux releases are built with `DYNAMIC_BACKENDS=1` and `CARGO_FEATURES=vulkan`,
  which is what produces a single artifact that picks up a GPU at runtime. Building the
  Vulkan backend needs the Vulkan SDK — a build-time dependency only; users never install it.
- A `cuda` Cargo feature also exists for anyone who wants to compile an NVIDIA-specific build
  by hand. It is not used by the release pipeline, so the binaries you download are the
  Vulkan ones.
