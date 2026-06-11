# Script your inbox — the EmailOps CLI

`emailops-cli` is a power-user command line for EmailOps. It drives the same
local engine the desktop app uses — your mail, your accounts, your local AI — so
anything you can do in the app you can also **script, pipe, and automate** from a
terminal.

Why you might want it:

- **Local-first & private.** It reads the same on-device SQLite database the app
  already synced. Nothing new leaves your machine; AI runs locally by default,
  exactly like the app.
- **Machine-readable.** Every command takes `--json` and prints one stable
  envelope, so you can pipe results into `jq`, `grep`, cron jobs, or your own
  scripts.
- **No separate account setup.** It attaches to the accounts you already
  connected in the app (OAuth tokens stay in the macOS Keychain). Run `doctor`
  and you're ready.

> **Status:** the CLI currently ships with the desktop source build and as a
> standalone release binary (see *Installation*). In-app "install to PATH" is
> being wired up; until then, follow the install steps below.

---

## Installation

### Option A — standalone release binary (recommended)

Download `emailops-cli` from the latest EmailOps release and drop it on your
`PATH`:

```bash
# once downloaded to ~/Downloads/emailops-cli
chmod +x ~/Downloads/emailops-cli
mv ~/Downloads/emailops-cli /usr/local/bin/emailops-cli

emailops-cli doctor          # confirm it sees your data + accounts
```

It's a universal binary (Apple Silicon + Intel), signed with a Developer ID. The
first time macOS Gatekeeper may ask you to allow it.

### Option B — build it yourself

From a checkout of the repo:

```bash
make build-cli-mac           # universal binary → src-tauri/target/cli-release/emailops-cli
make dist-cli-mac            # stage it → release/emailops-cli
cp release/emailops-cli /usr/local/bin/
```

Contributors iterating on the CLI itself should use the `make cli-*` dev targets
instead — see [`docs/cli.md`](./cli.md) for the full build/feature/eval reference.

---

## Quick start

```bash
emailops-cli doctor                       # is everything wired up? (read-only, fast)
emailops-cli accounts                     # which accounts are connected
emailops-cli emails --limit 10            # 10 most recent emails
emailops-cli search "invoice"             # full-text search
emailops-cli chat "what did Acme say about the contract?"
emailops-cli                              # no subcommand → interactive REPL
```

Run `emailops-cli --help` for the complete list, and `emailops-cli <command>
--help` for a command's flags.

---

## Global flags

Accepted before or after the subcommand:

| Flag | Effect |
|---|---|
| `--json` | Emit one machine-readable JSON envelope to stdout. |
| `--quiet` | Suppress the progress-log stream on stderr (errors still print). |
| `--account <ID\|EMAIL>` | Which account to act on (else your saved default / only account). |
| `--model <MODEL>` | Override the AI model for this command. |
| `--data-dir <DIR>` | Point at a different EmailOps data directory. |

If you have more than one account, set a default once so you don't repeat
`--account`:

```bash
emailops-cli config set default-account you@example.com
```

---

## Scripting with `--json`

With `--json`, **every** command prints exactly one envelope to stdout — the same
shape whether it succeeds or fails — while logs go to stderr. That makes stdout a
clean data channel you can pipe anywhere.

```jsonc
{ "ok": true,  "data": { /* result */ }, "error": null }
{ "ok": false, "data": null, "error": { "code": "not_found", "message": "…", "params": {…} } }
```

Parse `ok` first, then read `data` (or `error`). A few recipes:

```bash
# Subjects of your 20 most recent unread-ish emails
emailops-cli emails --limit 20 --json | jq -r '.data[].subject'

# Just the answer text from a chat question
emailops-cli chat "summarize my unread mail" --json | jq -r '.data.answer'

# Sender + subject of every hit for a search, as TSV
emailops-cli search "from:ana invoice" --json \
  | jq -r '.data[] | [.sender, .subject] | @tsv'

# Fail fast in a script: branch on the exit code, not the text
if ! emailops-cli sync you@example.com --json >/tmp/out.json 2>/tmp/err.log; then
  echo "sync failed (exit $?):" >&2
  jq -r '.error.message' /tmp/out.json >&2
  exit 1
fi
```

### Exit codes

Grouped by *how you'd fix it*, so a script can branch on failure class without
parsing prose:

| Code | Meaning |
|---|---|
| `0` | Success |
| `2` | Invalid input (bad flags / missing account) |
| `3` | Not found (no such email / conversation / account) |
| `4` | Auth — needs re-login / Keychain issue |
| `5` | Network or sync error |
| `6` | AI error (inference failed, AI disabled, budget exceeded) |
| `130` | Cancelled |
| `1` | Anything else (database / IO) |

---

## Multi-turn chat from scripts

Each `chat --json` reply returns a `conversationId`. Feed it back with
`--conversation` to continue the same thread across separate invocations — the
context carries over just like in the app:

```bash
CID=$(emailops-cli chat "my flight is on the 14th; just ack" --json \
        | jq -r '.data.conversationId')

emailops-cli chat "what day is my flight?" --conversation "$CID" --json \
        | jq -r '.data.answer'
```

Add `--trace` to any `chat` (or `search`) to see *why* you got the answer you
did — the routing decision, what was retrieved, which tools ran, and timings.

---

## Automating with cron

Because read commands are safe while the app is open (the database is shared in
WAL mode), you can schedule lightweight digests. For example, a 9am "what's new"
summary mailed to yourself via your system mailer:

```cron
0 9 * * *  emailops-cli chat "summarize anything important since yesterday" --json \
             | jq -r '.data.answer' | mail -s "EmailOps digest" you@example.com
```

> **Heavy write commands** — `sync`, `classify`, `embed` — are best run with the
> **desktop app closed** to avoid write contention. Keep those out of schedules
> that might fire while you're using the app, or guard them so they skip when the
> app is running.

---

## Command reference (cheat sheet)

| Command | What it does |
|---|---|
| `doctor` | Report environment readiness (DB, accounts, AI). Read-only, fast. **Start here.** |
| `accounts` | List connected accounts. `accounts add gmail\|outlook\|imap` connects a new one. |
| `emails [--limit N] [--offset N] [--mailbox …] [--category …]` | List recent emails. |
| `show <id>` | Show one email (headers + body). |
| `search <query> [--limit N] [--offset N] [--trace]` | Full-text search across your mail. |
| `chat <question> [--trace] [--conversation <id>]` | Ask a question against your mail; streams the answer. |
| `sync [account]` | Download new mail (run with the app closed). |
| `classify [--all]` | Sort emails into categories (run with the app closed). |
| `embed [--batch N]` | Build search embeddings for new mail (run with the app closed). |
| `config get\|set\|unset\|list` | Manage CLI preferences (e.g. your default account). |

For exhaustive flag tables, account-add details, and the contributor build/eval
workflow, see [`docs/cli.md`](./cli.md).

---

## Privacy notes

- The CLI never sends your mail anywhere the app wouldn't. Local AI is the
  default; remote providers (OpenRouter) are only used if **you** configured them
  in the app.
- OAuth tokens stay in the macOS Keychain — they're never printed, even with
  `--json`.
- In `--json` mode, prefer passing IMAP app passwords on the command line only in
  trusted scripts; interactive `accounts add imap` prompts without echo so the
  secret never lands in your shell history.
