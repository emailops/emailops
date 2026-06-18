# emailops-cli

`emailops-cli` is a headless, power-user / agent-driven front-end over the same
`services::*` entry points the Tauri commands call — no `AppHandle`, no webview.
It exists so that:

1. **Power users** can drive EmailOps from the terminal (list, search, show,
   sync, chat, classify, embed).
2. **Tests and agents** can exercise real features headlessly and assert on
   **structured output** (`--json`) instead of guessing whether a change works.

The binary is gated behind the `cli` cargo feature, so it never compiles into
default or release desktop builds. Logic lives in `src-tauri/src/cli/`; the bin
(`src-tauri/src/bin/emailops_cli.rs`) is a thin wrapper.

It operates on the **real** data directory. SQLite runs in WAL mode, so read
commands (`accounts`, `emails`, `show`, `search`, `stats`, `doctor`, `config get/list`)
are safe to run while the desktop app is open. Run heavy **write** commands
(`sync`, `classify`, `embed`) with the app closed to avoid contention.

---

## Building & running

Prefer the Makefile targets over ad-hoc `cargo run`:

```bash
make cli                                    # build the bin (with embedded llama.cpp)
make cli-run  ARGS="accounts --json"        # build + run (with llama.cpp)
make cli-fast ARGS="search invoice --json"  # build + run, --no-default-features (no llama.cpp)
make cli-fast                               # no ARGS → interactive REPL
make cli-demo ARGS="search 'invoice'"       # run against the synthetic demo DB (safe for GIFs)
make cli-eval ARGS="--tier smoke --json"    # run chat eval cases through the CLI (needs cli,eval)
```

- `cli-fast` drops the `llamacpp` default feature for fast iteration on
  read/search commands, or when using Ollama / OpenRouter for AI paths.
- `cli` / `cli-run` keep the embedded local model, needed for local
  `chat` / `classify` / `embed`.

### Demo data for screen recordings

`make cli-demo` drives the CLI against the **synthetic demo DB** (an isolated
`.emailops-demo-data/` dir, never your real mail) — ideal for recording a GIF or
sharing examples without exposing personal email. It keeps `llama.cpp` on so
`chat` works fully offline, and auto-builds the demo DB + embeddings on first run.

```bash
make demo-db                                          # (re)generate the synthetic DB (optional; auto-built)
make cli-demo ARGS="config set default-account demo-acct-work"  # one-time, so commands need no --account
make cli-demo ARGS="search 'invoice' --json"
make cli-demo ARGS="chat 'what did Acme say about the contract?' --trace"
make cli-demo                                          # no ARGS → interactive REPL on demo data
```

The demo mailbox has two enabled accounts, so set a default once (above) or pass
`--account demo-acct-work` per command. Everything mutates only the disposable
demo dir.

---

## Global flags

These are accepted before or after the subcommand:

| Flag | Effect |
|---|---|
| `--json` | Emit one machine-readable JSON envelope to stdout (see below). |
| `--quiet` | Suppress the app-log stream on stderr (errors still print). |
| `--data-dir <DIR>` | Override the data dir (else `$EMAILOPS_DATA_DIR` → platform default). |
| `--account <ID\|EMAIL>` | Account to operate on (else the saved default / single enabled account). |
| `--model <MODEL>` | Model override for AI commands (else the `ai_model` preference). |

---

## Output: the JSON envelope

With `--json`, every command prints exactly **one stable envelope** to stdout.
Logs always go to stderr, so stdout stays a clean data channel.

```jsonc
{ "ok": true,  "data": { /* result */ }, "error": null }                                   // success
{ "ok": false, "data": null, "error": { "code": "not_found", "params": {…}, "message": "…" } } // failure
```

`error` is the same `{code, params, message}` shape `AppError` serializes to at
the Tauri boundary, so a shell/agent can branch on the failure class without
parsing prose.

### Exit codes (grouped by remediation)

| Code | Meaning |
|---|---|
| `0` | Success |
| `2` | Invalid input |
| `3` | Not found |
| `4` | Auth (OAuth / keyring / needs re-auth) |
| `5` | Network / sync (HTTP, sync) |
| `6` | AI (inference error, AI disabled, budget exceeded) |
| `130` | Cancelled |
| `1` | Anything else (DB / JSON / IO) |

---

## Account resolution

Commands that need an account resolve it in this order:

1. `--account <id|email>` if given (a non-matching value is fatal).
2. The saved `config set default-account` preference, **while it still names an
   enabled account** (a stale default for a disabled account is ignored).
3. The single enabled account, if there is exactly one.

If none of these resolve and a command needs an account, it errors with exit
code `2` and a message pointing you at `--account` or `config set
default-account`.

---

## Commands

### `accounts`
List configured accounts (bare `accounts` or `accounts list`), or add a new one
with `accounts add <provider>`. Adding reuses the same `services::accounts`
entry points the desktop app's "Add account" dialog calls and stores credentials
in the OS keychain — so an account added here shows up in the desktop app too,
and vice-versa.

```bash
make cli-fast ARGS="accounts --json"          # list
```

#### `accounts add gmail` / `accounts add outlook`
OAuth providers. Opens your default browser to authorize, runs a loopback
listener to capture the callback, exchanges the code, and persists the tokens.
Requires the provider's OAuth client env vars to be set (e.g.
`EMAILOPS_GMAIL_CLIENT_ID` / `EMAILOPS_GMAIL_CLIENT_SECRET`,
`EMAILOPS_OUTLOOK_CLIENT_ID`) — the same ones the desktop dev build uses.

| Flag | Default | Meaning |
|---|---|---|
| `--sync-from <YYYY-MM-DD>` | all history | Only sync mail on/after this date. |

```bash
# needs llama.cpp-free build is fine; OAuth opens a browser, so run it directly:
make cli-run ARGS="accounts add gmail"
make cli-run ARGS="accounts add outlook --sync-from 2024-01-01"
```

#### `accounts add imap`
IMAP/SMTP with a username + (app) password. The credentials are verified by
logging in before anything is saved.

| Flag | Default | Meaning |
|---|---|---|
| `--host <H>` | *(required)* | IMAP server host, e.g. `imap.fastmail.com`. |
| `--port <N>` | `993` | IMAP TLS port. |
| `--username <U>` | *(required)* | Login username (usually the full email). |
| `--password <P>` | *(prompt)* | App password. Omit to be prompted **without echo**; required with `--json`. |
| `--smtp-host <H>` | = `--host` | SMTP server host. |
| `--smtp-port <N>` | `587` | SMTP STARTTLS port. |
| `--name <N>` | = username | Display name. |
| `--sync-from <YYYY-MM-DD>` | all history | Only sync mail on/after this date. |

```bash
# Prompts for the password (no echo):
make cli-run ARGS="accounts add imap --host imap.fastmail.com --username me@fastmail.com"
```

> Prefer the interactive prompt over `--password` so the secret doesn't land in
> your shell history. In `--json` (agent) mode there's no prompt, so `--password`
> is required there.

### `emails`
List recent emails for the resolved account.

| Flag | Default | Meaning |
|---|---|---|
| `--limit <N>` | `25` | Max emails to return. |
| `--offset <N>` | `0` | Skip this many emails before returning — for paging (page 2 of a 25-row list = `--offset 25`). |
| `--mailbox <M>` | — | `inbox` \| `sent` \| `spam` \| `trash`. |
| `--category <C>` | — | Gmail category: `primary` \| `social` \| `promotions` \| `updates` \| `forums`. |

The pretty (non-JSON) table leads with each thread's most-recent message date
(`YYYY-MM-DD HH:mm`, local time), followed by the read marker, sender, subject,
and id.

```bash
make cli-fast ARGS="emails --mailbox inbox --category promotions --limit 20 --json"
make cli-fast ARGS="emails --limit 25 --offset 25 --json"   # page 2
```

### `show <id>`
Show the email's **thread**. In pretty mode the whole conversation is rendered
chronologically — a heading, then one block per message (position, date, sender,
`(you)` for your own messages, `▶` marking the id you asked for) with each body
**indented** under its header and HTML stripped to readable text. In `--json`
the command keeps its single-email contract: it returns just the requested
message with the raw stored body as the source of truth.

```bash
make cli-fast ARGS="show <email-id>"          # pretty: full thread, indented bodies
make cli-fast ARGS="show <email-id> --json"   # JSON: the single requested email
```

### `search <query>`
Full-text search across the account's mail.

| Flag | Default | Meaning |
|---|---|---|
| `--limit <N>` | `25` | Max hits to return. |
| `--offset <N>` | `0` | Skip this many hits before returning — for paging (page 2 of a 25-hit list = `--offset 25`). |
| `--trace` | off | Include search diagnostics: the search method, AI availability, the parsed filters (`from:` / `subject:` / keywords / date bounds), and hit counts. |

```bash
make cli-fast ARGS="search 'invoice' --limit 10 --json"
make cli-fast ARGS="search 'invoice' --limit 10 --offset 10 --json"   # page 2
make cli-fast ARGS="search 'from:ana invoice' --trace --json"
```

With `--trace`, `--json` switches from a bare emails array to
`{ "emails": [...], "trace": { searchMethod, aiAvailable, parsedQuery, shown,
offset, totalHits, query } }`; pretty mode prints the emails, then a trace block
to stderr. Without `--trace` the JSON shape is unchanged (a plain emails array).
`totalHits` is the full match count (before paging), `offset` is the skip
applied, and `shown` is how many rows this page returned.

### `chat <question>`
Ask one question against your mail and stream the answer.

| Flag | Meaning |
|---|---|
| `--trace` | Include the chat trace (route, retrieval, tool calls, timings). `--json` puts it under `data.trace`; pretty mode prints a dim trace block after the answer. |
| `--conversation <ID>` | Continue an existing conversation instead of starting a new one. Pass the `conversationId` returned by a previous `chat --json` to carry context across one-shot invocations (multi-turn). |

```bash
make cli-run ARGS="chat 'what did Acme say about the contract?' --json --trace"
```

`--trace` is invaluable for diagnosing *why* a reply changed: it surfaces the
routing decision, retrieval stats, and tool calls — the same `ChatTrace`
persisted on the assistant message.

When the turn writes a draft (the model returns a `draft://DRAFT_ID` chip), the
CLI also surfaces the draft itself so you see the body, not just the link:
`--json` adds a `data.drafts` array (each `{id, accountId, toAddresses, subject,
body, …}`); pretty mode and the REPL print a `── draft ──` block (To / Subject /
Id + body) after the answer.

Every `chat` reply (`--json`) returns a `conversationId`. Feed it back via
`--conversation` to verify or script **multi-turn** exchanges headlessly — this
is the same context-carrying behaviour the REPL's `/chat` (and bare text) use:

```bash
# turn 1 — capture the id
CID=$(make -s cli-run ARGS="chat 'my favorite color is teal; just ack' --json" 2>/dev/null \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['conversationId'])")
# turn 2 — same conversation recalls the context
make -s cli-run ARGS="chat 'what is my favorite color?' --conversation $CID --json"
```

### `sync [account]`
Download new mail. The positional `account` overrides the global `--account` /
default. v1 sync is download-only (no AI follow-ups / attachment fetches).

```bash
make cli-run ARGS="sync gerodp@gmail.com"
```

### `classify [--all]`
Classify emails into categories. By default only new emails; `--all`
re-classifies every email.

```bash
make cli-run ARGS="classify --json"
```

### `embed [--batch N]`
Generate search embeddings for pending emails (`--batch`, default `50`, rows per
batch).

```bash
make cli-run ARGS="embed --batch 200 --json"
```

### `doctor`
Report environment readiness (DB, accounts, AI config). Read-only and fast —
loads no AI model. **Start here** to confirm the CLI is pointed at a usable
install.

```bash
make cli-fast ARGS="doctor --json"
```

### `stats`
The same per-account aggregates as the app's dashboard cards: local email count,
sent count, cached server total (when fetched), per-category counts, and
pipeline coverage — classified, embeddings, memory, and task extraction (each as
`done / eligible`). Read-only; loads no AI model and ignores `--account` (it
reports every account). `--json` emits the full `AccountDashboard` array.

```bash
make cli-fast ARGS="stats"
make cli-fast ARGS="stats --json"
```

### `config <get|set|unset|list> ...`
Get/set CLI-local preferences. Values live in the shared SQLite
`user_preferences` table under a `cli_` namespace, so they never collide with
the desktop app's own settings.

| Action | Example |
|---|---|
| `get <key>` | `config get default-account` |
| `set <key> <value>` | `config set default-account gerodp@gmail.com` |
| `unset <key>` | `config unset default-account` |
| `list` | `config list` |

Keys:

| Key | Meaning |
|---|---|
| `default-account` | Account (id or email) used when `--account` is omitted and more than one account is enabled. Stored canonicalized to the account **id**, so resolution is stable regardless of how you typed it. |

```bash
make cli-fast ARGS="config set default-account gerodp@gmail.com --json"
make cli-fast ARGS="config list --json"
```

### `eval [--case <id>] [--tier <tier>] [--cases-dir <dir>]`
Run chat eval cases through the shared harness and report pass/fail. Requires
the `eval` cargo feature (`make cli-eval`). Heuristics only — no judge, no HTML
report, and it does **not** mutate provider preferences. Each case runs in a
throwaway conversation that is deleted afterwards. This does **not** replace the
`examples/*_eval.rs` harnesses mandated for AI-reply changes — it's a faster
inner-loop check.

```bash
make cli-eval ARGS="--tier smoke --json"
make cli-eval ARGS="--case kickoff_date_es"
```

---

## Interactive REPL

Running `emailops-cli` with **no subcommand** drops into an interactive shell
(à la Claude Code):

```bash
make cli-fast        # no ARGS
```

**Every action is an explicit slash-command** — bare text does *not* silently
start a chat (it's rejected with a hint pointing at `/chat`). The one exception:
a bare line that is a single token matching a real email id runs `/show <id>`,
so you can paste an id straight from an `/emails` or `/search` listing to open
it. This keeps the REPL and the one-shot CLI on the same `Command` enum so
behaviour never diverges.

- **`/chat <question> [--trace]`** → a multi-turn chat in the current
  conversation; tokens stream live to stdout. `--trace` prints the route /
  retrieval / tool-call block after the answer. Successive turns share the
  conversation context just like the desktop app; `/new` starts a fresh one. The
  conversation is persisted as a normal chat conversation, so REPL history shows
  up in the desktop app's chat sidebar too. (This is distinct from the one-shot
  `emailops-cli chat`, which always starts a new conversation unless you pass
  `--conversation <ID>`.)
- **other `/`-prefixed** → slash-commands. `/search <query> [--trace]`,
  `/accounts`, `/emails`, `/show`, `/sync`, `/classify`, `/embed`, `/stats`, and
  `/config` map onto the **same `Command` enum** as the one-shot CLI. Session commands:
  `/account [<id|email>]` (show/switch), `/model [<name>]`, `/new` (fresh
  conversation), `/help`, `/quit`. Switching with `/account <id|email>` is
  **persisted as the CLI default** (same `cli_default_account` preference
  `config set default-account` writes), so the next launch resolves that account
  automatically — no need to re-select each session.
- **Quoting multi-word arguments** → slash-command arguments are tokenized with
  single/double-quote awareness, so a multi-word value is passed as one token:
  `/search "IB Trading Assistant"` or `/search 'from:ana invoice' --limit 5`.
  Without quotes the words become separate arguments and clap rejects the extras.
- `Ctrl-C` / `Ctrl-D` exit cleanly.

Anything you can script one-shot you can also drive interactively.

---

## Agent self-validation loop

When validating a change, prefer driving the real feature over guessing:

```bash
make cli-fast ARGS="doctor --json"                          # env wired? (DB, accounts, AI) — no model load
make cli-fast ARGS="search 'invoice' --json"                # read/search paths without booting llama.cpp
make cli-run  ARGS="chat 'what did X say?' --json --trace"  # full AI path; trace surfaces route/retrieval/tools
make cli-eval ARGS="--tier smoke --json"                    # re-run chat eval cases (needs cli,eval)
```

---

## Smoke test plan

A cheapest-first pass over the whole command surface. Read-only/no-model checks
run under `cli-fast`; the AI path uses `cli-run` (embedded llama.cpp). Run write
commands (`sync`/`classify`/`embed`) and account adds with the desktop app
**closed**. After each `--json` call, `echo $?` to confirm the exit code matches
the remediation class (`0` ok, `2` invalid, `3` not found, `4` auth, `5`
net/sync, `6` AI).

### Phase 0 — build & environment (no model load)
```bash
make cli-fast ARGS="doctor --json"            # exit 0; data shows DB path, accounts, AI config
make cli-fast ARGS="--help"                   # all subcommands listed
make cli-fast ARGS="config list --json"       # settings dump, ok:true
```

### Phase 1 — read paths (fast, no llama.cpp)
```bash
make cli-fast ARGS="accounts --json"                      # bare = list; ok:true, data is array
make cli-fast ARGS="accounts list --json"                 # same shape as bare
make cli-fast ARGS="emails --limit 5 --json"              # <=5 emails
make cli-fast ARGS="emails --limit 5 --mailbox inbox --category primary --json"
make cli-fast ARGS="search 'invoice' --limit 5 --json"    # hits or empty array, ok:true
make cli-fast ARGS="search 'invoice' --trace"             # pretty + retrieval trace
make cli-fast ARGS="show <ID_FROM_EMAILS> --json"         # ok:true, full email
```

### Phase 2 — config round-trip
```bash
make cli-fast ARGS="config set default-account you@example.com --json"   # ok:true
make cli-fast ARGS="config get default-account --json"                   # echoes value
make cli-fast ARGS="config unset default-account --json"                 # ok:true
```

### Phase 3 — error / exit-code contract
```bash
make cli-fast ARGS="show nonexistent-id --json"; echo "exit=$?"   # ok:false code:not_found, exit=3
make cli-fast ARGS="accounts add imap --host x --username u --json"; echo "exit=$?"  # missing --password → invalid_input, exit=2
make cli-fast ARGS="bogus-command"; echo "exit=$?"                # clap usage error, exit=2
```

### Phase 4 — account add
```bash
# IMAP, secure prompt (no echo) — pretty mode
make cli-fast ARGS="accounts add imap --host imap.fastmail.com --username you@example.com"
# IMAP, JSON requires --password (cannot prompt)
make cli-fast ARGS="accounts add imap --host imap.fastmail.com --username you@example.com --password APPPW --json"
# OAuth — opens a browser; needs EMAILOPS_GMAIL_CLIENT_ID/SECRET (or OUTLOOK) in .env
make cli-run  ARGS="accounts add gmail --sync-from 2025-01-01"
make cli-run  ARGS="accounts add outlook"
```
Expect `Added <provider> account <email> (<id>).`, then confirm with `accounts list`.

### Phase 5 — AI path (with llama.cpp; app closed)
```bash
make cli-run ARGS="chat 'what did the last sender ask?' --json --trace"  # data.answer + data.trace + data.sources
make cli-run ARGS="chat 'draft a reply confirming the meeting' --trace"  # renders ── draft ── block (To/Subject/Body)
make cli-eval ARGS="--tier smoke --json"                                 # curated chat cases pass
```

### Phase 6 — REPL parity (interactive)
```bash
make cli-fast    # no ARGS → REPL; then type: /doctor, /accounts, /search invoice, /chat what's new?, /quit
```

**Pass criteria:** every `--json` call emits a well-formed envelope; exit codes
match the remediation classes (Phase 3); `accounts add` persists and reappears in
`list`; `chat` surfaces created drafts inline; the smoke eval is green.
