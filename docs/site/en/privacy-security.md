---
title: 'Privacy & security'
description: 'Where your mail is stored, what leaves your machine, and the controls that protect you from the mail itself.'
weight: 45
---

EmailOps is built around one rule: your mail stays on your machine. This page describes what
that means concretely — where data is written, what network calls exist, and which safety
features you can turn on.

## Where your data is stored {#where-your-data-is-stored}

Everything lives in your OS application data directory:

| Platform | Location |
|---|---|
| macOS | `~/Library/Application Support/com.emailops.app` |
| Windows | `%APPDATA%\com.emailops.app` |
| Linux | `~/.local/share/com.emailops.app` |

Inside it:

- **A SQLite database** — messages, threads, contacts, calendar events, classification tags,
  search embeddings and AI memory. This is the only copy EmailOps keeps.
- **A `models/` folder** — the AI models you downloaded.

Point `EMAILOPS_DATA_DIR` somewhere else before launching to use a different location — a
second profile, or an encrypted volume.

**Credentials are not in there.** OAuth tokens and IMAP passwords go to the system credential
store: macOS Keychain, Windows Credential Manager, or a Secret Service keyring on Linux. They
are never written to a config file, and they survive uninstalling the app.

## There is no EmailOps server

There is no account to create, no sign-up, and no backend operated by us — so there is
nowhere for your mail to be uploaded to, and nothing to breach. The app talks to exactly
these hosts, all of which you can name:

| Destination | When | Contains your mail? |
|---|---|---|
| Your mail provider (Gmail, Microsoft Graph, your IMAP/SMTP server) | Every sync and send | Yes — it is your mailbox |
| Your calendar provider (Google, Outlook) | Calendar sync, if enabled | Calendar data only |
| Hugging Face | Only while downloading an AI model you picked | No |
| OpenRouter | Only if you switch the AI provider to it | **Yes — prompts include email content** |

The last row is the only path by which your mail can reach a third party, it is off by
default, and it takes a deliberate change in **Settings → AI Backend & Models** plus your own
API key to enable.

## No telemetry

The app collects no usage analytics, sends no crash reports, and has no phone-home of any
kind in released builds. There is no opt-out because there is nothing to opt out of. (The
source tree contains an optional OpenTelemetry tracing feature for local development; it is
compiled out of every release build.)

## Local AI by default

The default backend runs models in-process via an embedded llama.cpp runtime. No daemon, no
localhost server, no network socket — the model reads your email from the same process that
already has it. Classification, drafts, embeddings, chat, task and memory extraction all run
there.

Switching to Ollama keeps inference local too, just in a separate process on your machine.
Only OpenRouter sends content off the device. See
[choosing a backend](../ai-features/#choosing-a-backend).

## Protection from the mail itself

Email is an attack surface. The client-side defences:

- **Remote content blocking** — external images, tracking pixels and other remote resources
  are blocked until you allow them. A per-email banner lets you load them once, or you can
  trust a specific sender permanently. This is what stops senders learning when and how often
  you opened a message.
- **Junk and bulk scoring** — every message is scored locally for spam and unwanted bulk
  mail. Your "junk" / "not junk" corrections train it. Flagged mail is faded or hidden, never
  deleted or moved on the server unless you explicitly confirm.
- **Impersonation warnings** — an optional check that flags messages appearing to come from
  someone they do not. Off by default, because it is the one check that accuses a sender of
  fraud and it has the least evidence to go on.
- **Sanitised rendering** — message HTML is stripped of scripts, event handlers and embedded
  objects before it is displayed, on both sides of the app. Attachments are never opened on
  your behalf.

## Locking the app

Set a **main password** in **Settings → Privacy & Security** and EmailOps stays locked on
startup until you enter it. There is no recovery path — if you forget it, you reinstall
against a fresh data directory and re-sync from your provider.

Be clear about what this does: it locks the application, it does **not** encrypt the
database. Anyone with access to your unlocked user account and the data directory can read
the SQLite file directly. If that is part of your threat model, use full-disk encryption —
FileVault on macOS, BitLocker on Windows, LUKS on Linux — which is the right tool for it.

## Auditing any of this

EmailOps is Apache-2.0 and developed in the open. The claims on this page are checkable
against the source at
[github.com/emailops/emailops](https://github.com/emailops/emailops), and so is the network
behaviour — run it behind a proxy or `tcpdump` and compare against the table above. If
something does not match, please [open an issue](https://github.com/emailops/emailops/issues).
