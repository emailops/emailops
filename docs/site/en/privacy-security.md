---
title: 'Privacy & security'
description: 'Where your mail is stored, what leaves your machine, and the controls that protect you from the mail itself.'
weight: 45
---

<!-- claim:priv-intro-1 -->
EmailOps is built around one rule: your mail stays on your machine. This page describes what
that means concretely — where data is written, what network calls exist, and which safety
features you can turn on.

## Where your data is stored {#where-your-data-is-stored}

<!-- claim:priv-where-data-1 -->
Everything resides in your OS application data directory:

| Platform | Location |
|---|---|
| macOS | `~/Library/Application Support/com.emailops.app` |
| Windows | `%APPDATA%\com.emailops.app` |
| Linux | `~/.local/share/com.emailops.app` |

<!-- claim:priv-where-data-2 -->
Inside it:

- **A SQLite database** — messages, threads, contacts, calendar events, classification tags,
  search embeddings and AI memory. This is the only copy EmailOps keeps. <!-- claim:priv-where-data-3 -->
- **A `models/` folder** — the AI models you downloaded. <!-- claim:priv-where-data-4 -->

<!-- claim:priv-where-data-5 -->
Point `EMAILOPS_DATA_DIR` somewhere else before launching to use a different location — a
second profile, or an encrypted volume.

<!-- claim:priv-where-data-6 -->
**Credentials are not in there.** OAuth tokens and IMAP passwords go to the system credential
store: macOS Keychain, Windows Credential Manager, or a Secret Service keyring on Linux. They
are never written to a config file, and they survive uninstalling the app.

## There is no EmailOps server

<!-- claim:priv-there-no-1 -->
There is no account to create, no sign-up, and no backend operated by us — so there is
nowhere for your mail to be uploaded to, and nothing to breach. The app talks to exactly
these hosts, all of which you can name:

| Destination | When | Contains your mail? |
|---|---|---|
| Your mail provider (Gmail, Microsoft Graph, your IMAP/SMTP server) | Every sync and send | Yes — it is your mailbox |
| Your calendar provider (Google, Outlook) | Calendar sync, if enabled | Calendar data only |
| Hugging Face | Only while downloading an AI model you picked | No |
| OpenRouter | Only if you switch the AI provider to it | **Yes — prompts include email content** |

<!-- claim:priv-there-no-2 -->
The last row is the only path by which your mail can reach a third party, it is off by
default, and it takes a deliberate change in **Settings → AI Backend & Models** plus your own
API key to enable.

## What EmailOps changes in your mailbox

<!-- claim:priv-what-emailops-1 -->
Most of what EmailOps does is read-only: it downloads your mail and keeps a local copy. A
few actions deliberately reach back to the account, so that what you do here is what you see
everywhere else:

| Action | Effect on the account |
|---|---|
| Marking a message read or unread | The same message is marked read on the account (Gmail) |
| Deleting a message | The message is moved to the account's **Trash** (Gmail), where it stays recoverable for 30 days |
| Moving a message to another folder, or **Confirm junk** | The message moves on the account too — Confirm junk files it in the provider's junk folder |
| Creating, renaming or deleting a folder | The folder changes on the account too |
| Saving a draft | The draft is saved to the account's Drafts |

<!-- claim:priv-what-emailops-2 -->
EmailOps never permanently erases a message: deletes always go to Trash, never to a hard
delete. Reading is applied locally first so the app works offline, and the account catches up
in the background. Everything else — labels, filters, folders you have not touched — is left
exactly as it is.

<!-- claim:priv-what-emailops-3 -->
Every message you send from EmailOps ends with a short "Sent with EmailOps" line that links
to getemailops.com. The link carries `utm_source=email_footer`, which only tells the
website's analytics that a visit came from an email footer; nothing in it identifies you or
the recipient.

## No telemetry

<!-- claim:priv-no-telemetry-1 -->
The app collects no usage analytics, sends no crash reports, and has no phone-home of any
kind in released builds. There is no opt-out because there is nothing to opt out of. (The
source tree contains an optional OpenTelemetry tracing feature for local development; it is
compiled out of every release build.)

## Local AI by default

<!-- claim:priv-local-ai-1 -->
The default backend runs models in-process via an embedded llama.cpp runtime. No daemon, no
localhost server, no network socket — the model reads your email from the same process that
already has it. Classification, drafts, embeddings, chat, task and memory extraction all run
there.

<!-- claim:priv-local-ai-2 -->
Switching to Ollama keeps inference local too, just in a separate process on your machine.
Only OpenRouter sends content off the device. See
[choosing a backend](../ai-features/#choosing-a-backend).

## Protection from the mail itself

<!-- claim:priv-protection-from-1 -->
Email is an attack surface. The client-side defences:

- **Remote content blocking** — external images, tracking pixels and other remote resources
  are blocked until you allow them. A per-email banner lets you load them once, or you can
  trust a specific sender permanently. This is what stops senders learning when and how often
  you opened a message. <!-- claim:priv-protection-from-2 -->
- **Junk and bulk scoring** — every message is scored locally for spam and unwanted bulk
  mail. Your "junk" / "not junk" corrections train it. Flagged mail is faded or hidden, never
  deleted or moved on the server unless you explicitly confirm. <!-- claim:priv-protection-from-3 -->
- **Impersonation warnings** — an optional check that flags messages appearing to come from
  someone they do not. Off by default, because it is the one check that accuses a sender of
  fraud and it has the least evidence to go on. <!-- claim:priv-protection-from-4 -->
- **Sanitised rendering** — message HTML is stripped of scripts, event handlers and embedded
  objects before it is displayed, on both sides of the app. Attachments are never opened on
  your behalf. <!-- claim:priv-protection-from-5 -->

## Locking the app

<!-- claim:priv-locking-app-1 -->
Set a **main password** in **Settings → Privacy & Security** and EmailOps stays locked on
startup until you enter it. There is no recovery path — if you forget it, you reinstall
against a fresh data directory and re-sync from your provider.

<!-- claim:priv-locking-app-2 -->
Be clear about what this does: it locks the application, it does **not** encrypt the
database. Anyone with access to your unlocked user account and the data directory can read
the SQLite file directly. If that is part of your threat model, use full-disk encryption —
FileVault on macOS, BitLocker on Windows, LUKS on Linux — which is the right tool for it.

## Auditing any of this

<!-- claim:priv-auditing-any-1 -->
EmailOps is Apache-2.0 and developed in the open. The claims on this page are checkable
against the source at
[github.com/emailops/emailops](https://github.com/emailops/emailops), and so is the network
behaviour — run it behind a proxy or `tcpdump` and compare against the table above. If
something does not match, please [open an issue](https://github.com/emailops/emailops/issues).
