---
name: fix-ai-bug
description: Reproduce a bug the user reported in chat, drafts, classification, lenses, retrieval, or any other AI feature — using a CLI-driven repro (eval case, integration test, or shell script) against the user's personal DB by default. All read-only DB inspections (doctor, accounts, emails, show, search, chat --trace, raw sqlite3) run via `make cli-*` without asking the user — those commands are pre-authorised in `.claude/settings.local.json`. Run the repro to surface the failure, explain the root cause, propose a fix (gated on confirmation only when there is real ambiguity), then re-run until the repro turns green. If the repro is a good fit for a permanent regression case in private-evals/ (deterministic, depends on personal mailbox content, exercises a regressable class of bug), ASK the user before adding it — never copy in silently. The final report uses a chat-style format (Session / User: / A:) and never dumps the raw trace unless the user asks. Use whenever the user reports unexpected AI behavior — even if they only paste a screenshot or trace fragment.
argument-hint: <optional bug summary or pasted trace>
allowed-tools: Bash, Read, Edit, Write, Grep, Glob, TaskCreate, TaskUpdate, TaskList, TaskGet
---

# Reproduce an AI-feature bug

You are debugging a bug the user just reported in an AI feature of EmailOps.
Run the phases below in order. Each phase ends with a short status line; do
**not** dump raw JSON traces, full prompts, or full search results into the
chat unless the user explicitly asks for them.

## Golden rules

1. **Reproduce before theorising.** No fix proposal until you have seen the
   failure in your own terminal. If you cannot reproduce, say so plainly and
   ask for more context — never patch on guess.
2. **CLI first.** `emailops-cli` exercises the same `services::*` entrypoints
   the Tauri commands call; prefer it over new test scaffolding. Only fall
   back to an eval case, a Rust/TS test, or a `scripts/*.sh` when the CLI
   cannot reach the failing path.
3. **DB consultation always goes through the CLI — no confirmation step.**
   Whenever you need to look at the user's mailbox state (which accounts
   exist, which emails are present, what `search` returns, what `doctor`
   reports, what the chat trace looks like end-to-end), run the `make cli-*`
   command directly. These are pre-authorised in
   `.claude/settings.local.json` (`Bash(make *)`, `Bash(sqlite3:*)`,
   `Bash(cargo run:*)`), so they will not trigger a permission prompt — do
   not ask the user "should I run this?" before executing a read-only CLI
   inspection. Just run it and report the result.
4. **Personal DB by default.** `make cli-run` and `make cli-fast` target the
   personal app data dir already. Switch to `make cli-demo` (synthetic) or
   `make cli-ask` (`.env.local` account override) only when the user asks or
   the bug is account-specific.
5. **Read paths are safe with the app open** (SQLite WAL). **Write paths
   (`sync`, `classify`, `embed`) need the app closed** — warn the user before
   running them.
6. **Don't ask for confirmation when there is nothing to choose.** A
   mechanical fix that the repro will verify does not need a prompt; design
   forks do.
7. **Cap iteration at 3 unsuccessful re-runs.** After three failed attempts,
   stop and ask the user how to proceed — don't grind silently.

## Read-only DB inspection cheat sheet (run without asking)

These are the common questions you might need answered to frame or debug a
bug. All are read-only, all are pre-authorised — execute them directly the
moment you need the data.

| Question | Command |
|----------|---------|
| Which model / accounts / data dir is active? | `make cli-fast ARGS="doctor --json"` |
| Which accounts exist on this install? | `make cli-fast ARGS="accounts --json"` |
| What's in the inbox right now? | `make cli-fast ARGS="emails --limit 10 --json"` |
| Show one email (headers + body) | `make cli-fast ARGS="show <id> --json"` |
| What does search return for X? | `make cli-fast ARGS="search '<q>' --json --trace"` |
| What does the chat answer end-to-end? | `make cli-run ARGS="chat '<prompt>' --json --trace --fresh"` |
| Raw SQL when the CLI doesn't cover it | `sqlite3 "$HOME/Library/Application Support/com.emailops.app/emailops.db" "<query>"` |

`cli-fast` skips the `llamacpp` feature build — faster iteration when you do
not need the chat model loaded. Use `cli-run` whenever the path under
investigation needs the local model.

Direct `sqlite3` is also pre-authorised (`Bash(sqlite3:*)`); reach for it
only when the CLI cannot answer the question (custom joins, schema
inspection, `PRAGMA` probes) — see `src-tauri/CLAUDE.md` "Inspecting the
database while debugging" for the canonical paths.

## Phase 1 — Frame the bug

From the user's message (and any pasted trace), capture in 3-5 bullets:

- **Feature** (chat / draft / classification / lens / retrieval / route / memory)
- **Model** in use (read from the trace, or `make cli-fast ARGS="doctor --json"`)
- **Account** the bug affects (or "any" if not account-specific)
- **User prompt** (verbatim — preserve language and accents)
- **Expected vs actual** — what should have happened, what did

If anything is unclear after reading the report, ask **one** question before
moving on. Don't drown the user in clarifications.

If a trace was pasted, additionally note:
- Which round failed (`llm round N: returned ... (K tool_calls)`)
- The raw `output` of the failing round
- Which parser / router / tool was responsible

## Phase 2 — Pick the repro vehicle

Choose the smallest vehicle that reaches the failing code path:

| Vehicle | When to use | Command |
|---------|-------------|---------|
| `emailops-cli chat` | Chat, drafts, heuristic routing, tool-call parsing | `make cli-run ARGS="chat '<prompt>' --json --trace --fresh"` |
| `emailops-cli search` | Search, FTS, hybrid retrieval | `make cli-run ARGS="search '<query>' --json --trace"` |
| `emailops-cli doctor` | Env / model / accounts / AI config | `make cli-fast ARGS="doctor --json"` |
| `emailops-cli eval` | Deterministic regression — case worth keeping | `make cli-eval ARGS="--case <id> --json"` |
| `examples/*_eval.rs` | Classification / lens / draft / memory / task / invoice / agent-search / chat-shortcut | `cargo run --manifest-path src-tauri/Cargo.toml --features eval --example <name>` |
| Rust unit test | Parser, planner, pure logic | new `#[test]` in the relevant module |
| TS Vitest | Store / hook / component | new `*.test.ts` next to the source |
| `scripts/*.sh` | Only when none of the above fit | new script + thin Makefile target |

**Multi-turn chat bugs**: capture the `conversationId` from the first
`--json` response, then pass `--conversation <id>` to subsequent calls — the
process stays up so the model and KV cache persist.

**Account-specific bugs**: if `.env.local` has `EMAILOPS_PERSONAL_ACCOUNT`,
use `make cli-ask Q='<prompt>'` (it sets `--account` for you).

**Don't invent a new eval harness** when an existing one already covers the
failing surface — see `src-tauri/CLAUDE.md` "AI replies must be verified by
an eval" for the list.

If the bug touches AI-reply content, **add a focused case** to the matching
`examples/*_eval.rs` so the fix lands with a regression guard.

## Phase 3 — Run the repro

Execute the chosen command **without asking the user first** — the `make
cli-*`, `cargo run`, and `sqlite3` commands the skill uses are all
pre-authorised. While it runs, the only on-screen update should be a
one-line status (`Reproducing on cli-run chat …`). Do not paste the raw
output.

When it returns, confirm **the same symptom the user reported**:

- Chat bug: same empty/incorrect/missing reply, same failing round.
- Draft bug: same missing/wrong draft body, missing `draft://` chip.
- Retrieval bug: same missing/wrong source, same hit count.

If the symptom does **not** reproduce on the first try, try:

1. `--fresh` (rules out conversation-state pollution)
2. Different account or DB (if account-specific is plausible)
3. Identical prompt verbatim (small wording changes can flip stochastic
   models)

If after those three attempts it still doesn't reproduce, **stop and ask the
user** — share what you ran and what you got. Do not invent a fix.

When the symptom reproduces, walk the user through the root cause in 2-4
sentences with `file:line` references. Quote at most one short line of the
failing output (e.g. the malformed tool-call JSON) — never the full trace.

## Phase 4 — Propose the fix

Two sentences: the change + the main tradeoff. **Skip the confirmation
prompt** when ALL of these hold:

- The fix is mechanical (typo, missing branch, schema mismatch, obvious
  regression, lenient parser for a known LLM error shape).
- There is no design choice between equally valid approaches.
- The repro from Phase 2 will verify the fix.

Otherwise **ask**, present 1-3 alternatives with their tradeoff, and let the
user pick. Don't propose more than 3 — that's analysis paralysis, not help.

When in doubt about scope ("just this one shape" vs. "the whole class of
shapes"), ASK.

## Phase 5 — Implement and verify

Follow TDD when writing logic — failing test first, watch it fail for the
right reason, minimum code to pass, refactor. Invoke the
`test-driven-development` skill for any non-trivial logic change.

Re-run the **same** repro command from Phase 2. If it still fails:

- Inspect the new trace (specifically the failing round's `output`).
- Iterate on the fix, not on the repro vehicle.
- Cap at 3 re-runs. After three failed attempts, stop and ask.

Before declaring done, run the matching local gate:

- Rust logic changed → `make test-fast` (or `make test` if you touched the
  `llamacpp` paths) + `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` + `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` on the files you touched.
- TS changed → `npx biome check src/` + `npx tsc --noEmit`.
- Full hook suite (only if explicitly requested or the change is large) →
  `npx lefthook run pre-commit`.

## Phase 5b — Offer to graduate the repro into private-evals

`private-evals/` is the gitignored home for regression cases that ride on the
user's real mailbox (personal names, subjects, IDs that the public
`src-tauri/evals/chat/cases/` would violate). When the repro you just turned
green is a good fit, **ask the user** before adding it — never copy into
`private-evals/` silently.

A repro is worth graduating when **all** of these hold:

- It's **deterministic enough to replay** — same prompt + same account + same
  fix path → same outcome. Stochastic-only failures (random sampling, race
  conditions) are not a fit.
- It **depends on personal-mailbox content** (a specific sender, subject, or
  email id) that cannot land in the public `src-tauri/evals/chat/cases/`.
  Repros driven purely by synthetic prompts belong in the public cases
  directory instead — see `src-tauri/CLAUDE.md` "AI replies verified by an
  eval".
- It **exercises a class of bug that can regress** — model upgrade, prompt
  rewrite, parser change, retrieval/ranking tweak, tool-schema change. Pure
  one-off typos rarely need a permanent case.

If those hold, ask the user with a concrete one-liner:

> This repro looks like a good fit for `private-evals/chat/cases/<id>.yaml`
> (account=<email>, asserts <metric/expectation>). Add it as a regression
> case? (y/n)

Only proceed on a yes. If yes:

- Pick the right subdirectory based on the feature
  (`private-evals/chat/cases/`, `private-evals/chat/shortcuts/`,
  `private-evals/agent_search/`, etc.). Read `private-evals/CLAUDE.md` for
  the case schema and conventions before writing the YAML.
- Re-run via `cargo run --manifest-path src-tauri/Cargo.toml --features cli,eval --bin emailops-cli -- eval --case <id> --json` (no `--cases-dir` override — the CLI auto-resolves
  `private-evals/chat/cases/` when present). Confirm the case passes.
- Note in the final report: `Regression case: private-evals/<path> (passing)`.

If the user says no — or the criteria don't hold — skip the graduation step
silently. Don't lobby.

## Phase 6 — Report

Replace the raw trace with the chat-style block below. **Do not** include
the JSON `--trace` payload, the full system prompt, or full search results
unless the user asks. End the report with the offer to drill in.

### Chat-style format (chat / draft bugs)

```
Session: <model> | <account> | route=<route> | <N> LLM rounds | <K> tool calls

User: <prompt verbatim>

A: <reply, or "(empty reply)" pre-fix>
   tools: <tool_a(args)>, <tool_b(args)>           ← only if any tool ran
   draft: <subject> [draft://<id>]                 ← only if a draft was saved
   (Xs total — round0 Ys, round1 Zs)
```

Multi-turn: repeat the `User:` / `A:` blocks for each turn, in order.

### Non-chat format (eval / test / script repros)

```
Repro: <command or test name>
Input: <case id / prompt / argv>
Result: <PASS / FAIL — metric or assertion that flipped>
(Xs elapsed)
```

### Final line

End with one grep-friendly line so the user can find it later:

```
Fix: <file:line>. Repro: <command or test name>. Status: GREEN.
```

If the user accepted graduation in Phase 5b, add a second line:

```
Regression case: private-evals/<path>.yaml (passing).
```

Then **explicitly offer the trace**: "Want the raw `--trace` payload, the
failing-round output, or the system prompt? Ask and I'll paste."

## Common pitfalls (avoid)

- **Pasting the whole `--json` blob into chat.** The user reads only the
  chat-style summary; everything else lives in the persisted tool result.
- **Running the full eval suite** for a parser bug. Use `--case <id>` or the
  smallest scoped flag.
- **Mutating the personal DB** silently. `sync`/`classify`/`embed` are
  write paths — ask first, and warn that the app must be closed.
- **Switching to the demo DB** to make the repro "work". The demo DB is for
  screen recordings, not for reproducing production bugs unless the user
  asks.
- **Claiming GREEN** when only the unit test passes but the original CLI
  repro is still red. Both must turn green for the fix to count.
- **Embedding real personal data** (names, emails, subjects) in any
  git-tracked artifact you create — see CLAUDE.md "NEVER include real
  names…" and paraphrase to synthetic equivalents that preserve the shape.
