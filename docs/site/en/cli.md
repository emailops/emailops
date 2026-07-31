---
title: 'Command line (emailops-cli)'
description: 'Script and automate your inbox from the terminal, with stable JSON output for scripts and agents.'
weight: 50
---

`emailops-cli` drives the same local engine as the desktop app — your mail, your accounts,
your local AI — from a terminal. It reads the database the app already synced, so there is no
separate setup and no second copy of your mail.

Currently macOS only.

## Install

Download `EmailOps-CLI-macos.dmg` from the
[latest release](https://github.com/emailops/emailops/releases/latest), mount it and put the
binary on your `PATH`:

```bash
hdiutil attach ~/Downloads/EmailOps-CLI-macos.dmg
cp /Volumes/EmailOps\ CLI/emailops-cli /usr/local/bin/emailops-cli
hdiutil detach /Volumes/EmailOps\ CLI

emailops-cli doctor    # confirms it sees your data and accounts
```

The binary is universal (Apple Silicon + Intel), signed and notarized, so Gatekeeper lets it
through without a prompt.

## Quick start

```bash
emailops-cli accounts                     # which accounts are connected
emailops-cli emails --limit 10            # 10 most recent emails
emailops-cli search "invoice"             # full-text search
emailops-cli chat "what did Acme say about the contract?"
emailops-cli                              # no subcommand → interactive REPL
```

In the REPL, plain text is a chat turn (tokens stream live) and `/`-prefixed lines map onto
the subcommands: `/search`, `/account`, `/sync`, `/help`, `/quit`.

## Commands

| Command | Purpose |
|---|---|
| `accounts` | List configured accounts |
| `emails [--limit N] [--mailbox inbox\|sent\|spam\|trash]` | List recent emails |
| `show <id>` | Show one email (headers and body) |
| `search <query> [--limit N]` | Full-text search |
| `chat <question> [--trace]` | Ask a question; `--trace` adds routing and retrieval timings |
| `sync [account]` | Download new mail |
| `calendar [--days N] [--next] [--sync]` | Upcoming events (`--next` = next meeting only) |
| `classify [--all]` | Classify new — or all — emails |
| `embed [--batch N]` | Generate search embeddings |
| `doctor` | Read-only readiness report (database, accounts, AI config) |

Global flags work before or after the subcommand: `--json`, `--quiet`,
`--account <id|email>`, `--model <model>`, `--data-dir <dir>`.

Read commands are safe to run while the app is open. Heavy writes (`sync`, `classify`,
`embed`) are best run with the app closed.

## Scripting with `--json`

With `--json` every command prints exactly one envelope on stdout — same shape on success or
failure — while logs go to stderr:

```jsonc
{ "ok": true,  "data": { /* result */ }, "error": null }
{ "ok": false, "data": null, "error": { "code": "not_found", "message": "…", "params": {} } }
```

```bash
# Subjects of the 20 most recent emails
emailops-cli emails --limit 20 --json | jq -r '.data[].subject'

# Just the answer text from a chat question
emailops-cli chat "summarize my unread mail" --json | jq -r '.data.answer'

# Sender + subject of every search hit, as TSV
emailops-cli search "from:ana invoice" --json | jq -r '.data[] | [.sender, .subject] | @tsv'
```

Exit codes are grouped by what you would do about them: `0` success, `2` invalid input,
`3` not found, `4` auth, `5` network/sync, `6` AI, `130` cancelled, `1` anything else — so
scripts can branch on the code rather than parsing text.

If you have more than one account, save a default instead of repeating `--account`:

```bash
emailops-cli config set default-account you@example.com
```
