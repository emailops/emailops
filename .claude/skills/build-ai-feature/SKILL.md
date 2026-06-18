---
name: build-ai-feature
description: Build a net-new AI feature or change an existing AI surface in EmailOps — a new chat tool, a model/heuristic planner or route, a prompt edit, a shortcut fast-path, a new classifier/extractor, or a tweak to retrieval/drafts/memory. Drives the work through the repo's seams: pure planner first (TDD, failing test before code), thin executor, frontend toggle + feature gating when user-facing, and a mandatory eval gate because any change that can move an AI reply must be verified by an eval. Before building, it checks the change fits the current setup and config — context-window budget (n_ctx) and the KV-prefix prompt cache especially: a prompt extension that puts per-turn-variable content in the system prefix busts the cache anchor every turn, so the skill warns and proposes fixes. Establishes a CLI baseline ("before") to show the delta, gates confirmation to genuine design forks, and offers to graduate eval cases (synthetic → public `src-tauri/evals/`, personal-mailbox → `private-evals/`). All read-only DB inspections run via `make cli-*` without asking. Reports in a chat-style before/after format. Use whenever the user wants to add to or modify an AI feature — e.g. "add a planner that catches search-email/contact prompts as a direct tool call." For diagnosing broken AI behavior, use fix-ai-bug instead.
argument-hint: <feature idea or the AI surface to change>
allowed-tools: Bash, Read, Edit, Write, Grep, Glob, TaskCreate, TaskUpdate, TaskList, TaskGet
---

# Build or change an AI feature

You are adding a new AI capability to EmailOps, or changing how an existing
one behaves. This is design-and-build work, not bug triage — but it shares
`fix-ai-bug`'s spine: **CLI-driven validation against the real `services::*`
entrypoints, no raw-trace dumps, eval-gated, chat-style reporting.**

Run the phases in order. Each ends with a short status line. Do **not** dump
raw JSON traces, full prompts, or full search results into the chat unless
the user asks.

## Golden rules

1. **Find the seam before writing code.** Every AI surface in this repo has a
   home (see "Where AI features live"). A new capability is almost always a
   new file in an existing registry — not a new subsystem. Locate the seam
   first; if you're inventing a new one, that's a design fork → ask.
2. **Check the change fits the current setup *before* building.** A feature
   that works in isolation can still break the running config — most often by
   blowing the context-window budget or busting the KV-prefix prompt cache
   (Phase 2b). Verify compatibility, and when there's a cost, **warn the user
   and propose solutions** rather than shipping a regression.
3. **Pure planner first, thin executor.** Split the work into a pure decision
   function (input: data → output: a plan/decision struct, no I/O) and a thin
   executor that does the I/O. Unit-test the planner exhaustively;
   integration-test the executor against the trait fakes. Mandatory here —
   see `src-tauri/CLAUDE.md` "Pure planner + thin executor".
4. **TDD, no exceptions.** Failing test first, watch it fail for the right
   reason, minimum code to pass, refactor. Invoke the
   `test-driven-development` skill for every non-trivial logic change. Don't
   write production branches without a test that drives them.
5. **Baseline before building.** Capture how the feature behaves *today* via
   the CLI ("before") so you can show the delta and prove the change did what
   you claimed. The analog of `fix-ai-bug`'s reproduce-before-theorise.
6. **Any change that can move an AI reply must be verified by an eval.** The
   central gate. Prompt edits, new/removed tools, tool-description or schema
   changes, route/planner changes, retrieval tweaks, post-processing,
   default-model bumps — all move replies. Run the matching `examples/*_eval.rs`
   (scoped to related cases) before declaring done; add a focused case if none
   covers the new behaviour.
7. **DB consultation goes through the CLI — no confirmation step.** Read-only
   `make cli-*` and `sqlite3` inspections are pre-authorised in
   `.claude/settings.local.json`. Run them directly; never ask "should I run
   this?" for a read. Write paths (`sync`, `classify`, `embed`) need the app
   closed — warn first.
8. **Confirm design forks, not mechanics.** Building is more design-heavy than
   bug-fixing, so you'll ask more often than `fix-ai-bug` does — but only when
   there's a real choice (seam, model vs heuristic, schema shape, user-facing
   default, a cache/context tradeoff). A mechanical wiring step the eval will
   verify needs no prompt. Present at most 3 alternatives.

## Where AI features live (the seams)

Pick the smallest seam that delivers the capability. Most features are one new
file in one of these registries plus an eval case.

| Capability | Seam | Where |
|------------|------|-------|
| New chat tool the LLM can call (search, draft, recall, create…) | `Tool` trait impl + registry entry | new file in `src-tauri/src/services/chat/tools/`, registered in `tools/mod.rs` |
| Route decision (RAG-first vs tools-first, intent → fast path) | pure heuristic / classifier planner | `src-tauri/src/services/chat/routing.rs` |
| Prompt the user can edit (chat system, classify, memory, tasks) | `PromptDef` in the registry | `src-tauri/src/services/prompts/registry.rs` + `defaults.rs` |
| Shortcut fast-path (fixed prompt → deterministic tool call, skipping the model) | shortcut definition + eval | `src-tauri/src/services/chat/` + `evals/shortcuts/`, cases in `private-evals/chat/shortcuts/` |
| New classifier / extractor (classification, lens, memory, task, invoice) | pure prompt-assembly planner + executor + eval harness | `src-tauri/src/services/<area>/` + matching `examples/*_eval.rs` |
| Retrieval / ranking change (FTS, hybrid, RRF) | planner in retrieval | `src-tauri/src/services/chat/retrieval.rs` |
| Full user-facing AI feature (backend + command + settings UI) | command + service + eval + settings | see the draft-review vertical below |

**Canonical full-vertical example — "Review with AI" (draft review).** A
recent net-new AI feature touched every layer; read these together to see the
shape an end-to-end feature takes here: `src-tauri/src/commands/review.rs`
(thin command), `src-tauri/src/services/emails/review.rs` (service + planner),
`src-tauri/src/evals/draft_review.rs` + `src-tauri/examples/draft_review_eval.rs`
(eval), `src/components/Settings/AiReviewSettings.tsx` +
`src/stores/featureToggleStore.ts` (UI toggle + gating),
`private-evals/draft_review/cases.yaml` (private regression cases).

**The user's example — "a planner that catches prompts that map to a simple
tool call (search emails / contacts)"** — is a **route/planner** change in
`routing.rs` (possibly plus a shortcut fast-path). `routing.rs` already holds
`heuristic_route()`: a pure function returning `Option<(RouteMode, Vec<String>)>`,
unit-tested with table-driven EN/ES cases. A new intent planner is the same
pattern: a pure classifier returning a plan (which tool + extracted args, or
`None` to fall through to the model), exhaustively unit-tested, then wired into
`classify_route` / the turn loop. Decide up front: **heuristic (cheap,
deterministic, testable) vs. a model call (handles paraphrase, costs a
round-trip per turn, needs an eval for quality)** — a design fork → ask.

## Read-only inspection cheat sheet (run without asking)

| Question | Command |
|----------|---------|
| Which model / accounts / data dir is active? | `make cli-fast ARGS="doctor --json"` |
| Which accounts exist? | `make cli-fast ARGS="accounts --json"` |
| What does search return for X? | `make cli-fast ARGS="search '<q>' --json --trace"` |
| How does chat behave today (the baseline)? | `make cli-run ARGS="chat '<prompt>' --json --trace --fresh"` |
| Re-run eval cases through the shared harness | `make cli-eval ARGS="--tier smoke --json"` |
| Raw SQL the CLI can't express | `sqlite3 "$HOME/Library/Application Support/com.emailops.app/emailops.db" "<query>"` |

`cli-fast` skips the `llamacpp` build (no model load); `cli-run` boots the
local model for full AI paths.

## Phase 1 — Frame the feature

From the user's request, capture in 4-6 bullets:

- **Capability** — what the feature does, in one sentence.
- **Surface** — which seam from the table above (tool / route / planner /
  prompt / shortcut / classifier / full vertical).
- **Inputs → outputs** — what data goes in, what decision/text/effect comes out.
- **Success criteria** — the assertion an eval or test will make. Concrete:
  "prompts like 'search emails from X' route to a direct `search_emails` call
  with no model round-trip", not "works well".
- **User-facing?** — does it need a Settings toggle + feature gating (most
  user-visible AI features do), or is it internal plumbing?
- **Model vs heuristic** — if a planner/classifier: deterministic heuristic or
  a model call? Note the leaning; confirm in Phase 2 if it's a real fork.

Ask **one** question if the capability or success criteria are genuinely
unclear. Don't over-clarify.

## Phase 2 — Design & confirm the seam

State the design in 2-4 sentences: which file(s) get the new code, the
planner's signature (input type → plan type), and where the executor wires in.
Reference `file:line` for the registry/seam it plugs into.

**Confirm with the user** (1-3 alternatives with tradeoffs) on a genuine fork:
heuristic vs model call; new tool vs extending an existing tool's schema; new
shortcut vs the model loop; the shape of a new plan/output struct or tool
parameter schema; the default for a user-facing toggle. **Skip the prompt**
when the seam is obvious and choice-free (e.g. "add one keyword family to the
existing `heuristic_route` list"). When unsure about scope ("just this intent"
vs "the whole class"), ASK.

## Phase 2b — Compatibility check: context budget + prompt cache

Before writing code, confirm the change fits the **running setup and config**,
not just the abstract design. Building an AI feature in EmailOps means living
inside three hard constraints. Walk each that the change touches; if any costs
something, **warn the user with the concrete impact and propose fixes** — do
not silently ship a regression.

### A. Context-window budget (`n_ctx`)

The embedded llama.cpp runtime clamps context to `MAX_N_CTX = 8192`
(`src-tauri/src/ai/llama_cpp/actor.rs`). `plan_prompt_budget`
(`src-tauri/src/ai/llama_cpp/planner.rs`) shrinks the generation budget and
then **front-truncates the prompt** when `prompt_len + max_tokens > n_ctx` —
silently dropping the head of the prompt (and emitting a warning).

- Anything that grows the prompt (a longer system prompt, a new tool's schema
  in the tool array, more retrieved sources, longer summaries) eats this
  budget. The system prompt, tool schemas, retrieval block, and conversation
  history all share 8192 tokens with generation.
- **Risk to flag:** a large addition can push real turns into front-truncation
  (losing the head of the system prompt) or starve generation. **Fix options:**
  trim/condense the addition; gate it behind the feature toggle so it's only
  present when used; fair-share long content with a per-item budget (see the
  summary-preseed body-budget pattern); or move bulky content out of every
  turn and fetch it via a tool only when needed.

### B. KV-prefix prompt cache (the big one for prompt edits)

The llama.cpp path reuses the **longest common token prefix** across turns
(`plan_cached_prefix` / `plan_prefix_reuse` in `planner.rs`). The system
message is the cached **anchor** (seq 2). Three outcomes:

- `Extend` — the new prompt purely extends the resident prefix → reuse
  everything, decode only the new suffix (within-conversation case).
- `RestartFromAnchor` — the prompt diverges but the **stable system prefix is
  still a prefix** of it → reuse the anchor, decode the suffix (new
  conversation, same system prompt).
- `ColdPrefill` — the system prefix itself diverged → **full re-prefill**, the
  expensive case. This throws away the ~50% prefill win the KV-cache work
  bought.

The rule that follows, and the single most common way a prompt change
regresses performance:

> **Static** additions to the system prompt cost a *one-time* re-prefill, then
> cache normally — acceptable. **Per-turn-variable** content (today's date, the
> user's name, counts, retrieved email snippets, anything that changes between
> turns) placed *inside* the system prefix changes the anchor tokens **every
> turn** → `ColdPrefill` on every turn → the cache never helps.

This is exactly why KV-cache Phase 2 moved the dynamic sources block **out of**
the system message. So when a change adds prompt content:

- **Static text** (instructions, a fixed tool description, a new invariant
  rule): fine — append it at a stable position in the system prefix.
- **Variable text**: keep it **out of the system prefix**. Inject it *after*
  the stable system block (as a separate user/context message or a later
  segment), so the invariant prefix still matches turn-to-turn and only the
  cheap suffix re-decodes. If the design needs variable data in the prompt,
  this placement is the fix to propose — not "make the system prompt dynamic".

**Warn-and-propose template** when a proposed prompt extension carries variable
content in the prefix:

> Heads-up: putting `<the variable bit>` in the system prompt would change the
> cached prefix every turn (`ColdPrefill` in `plan_cached_prefix`), losing the
> KV-prefix cache's ~50% prefill saving on every chat turn. Options: (1) keep
> the static instruction in the system prompt and inject `<the variable bit>`
> after it as a context message; (2) fetch it via a tool only when needed; (3)
> accept the cost (only sensible if it's rare / one-shot). I'd recommend (1).

### C. Provider & model config

- The prefix cache and `n_ctx` truncation above are **`llamacpp`-specific**.
  Ollama and OpenRouter have their own caching/limits — a change that's free on
  one provider may cost on another. Note which providers the feature targets;
  don't assume llamacpp.
- **Never hardcode a model name** — query available models / read the
  preference (see `src-tauri/CLAUDE.md` "AI / Ollama Integration"). A planner
  that adds a model round-trip pays latency on **every** matching turn — weigh
  it against a heuristic.
- A new tool is only worth its prompt cost if the LLM actually picks it; a new
  always-present tool grows the tool array (budget A) for every turn — gate it
  via `is_available(&db)` so disabled features don't tax unrelated turns.

End this phase with a one-line verdict: `Compatibility: OK` or
`Compatibility: <cost> — proposing <fix>`.

## Phase 3 — Baseline the current behavior

Before writing code, capture how the surface behaves today via the CLI, so the
final report can show before → after. Run the relevant baseline **without
asking** (pre-authorised):

- Route/planner/chat feature: `make cli-run ARGS="chat '<prompt>' --json --trace --fresh"`
  — note the current route, whether the model round-trips, the latency, the reply.
- Search/retrieval: `make cli-fast ARGS="search '<q>' --json --trace"`.
- Classifier/extractor: the matching `examples/*_eval.rs` on a related case to
  capture the current score.

Record one or two baseline data points (route taken, rounds, latency, eval
score). This is the "before" half of the report.

## Phase 4 — TDD the pure planner

Write the failing test first. For the user's planner example that means
table-driven cases in `routing.rs`'s `#[cfg(test)] mod tests` (mirror the
existing `heuristic_routes_*` tests): prompts that *should* trigger the new
intent, prompts that should *not* (open-ended questions that must still fall
through to the model), EN **and** ES coverage (the user works in both).

- Invoke the `test-driven-development` skill.
- Watch the test fail for the right reason (planner absent / returns the old
  decision).
- Write the **minimum** pure planner to pass. No I/O in the planner — it takes
  data, returns a plan/decision. Keep `.unwrap()`/`.expect()` out of production
  paths (the crate denies them; see `src-tauri/CLAUDE.md`).
- Add the false-positive guard tests: the cost of over-triggering (routing a
  normal question to a forced tool call) must be pinned by a boundary test.

For a new tool, the "planner" is the arg-extraction + the tool's pure logic;
TDD that, then the `Tool` trait impl is the thin executor.

## Phase 5 — Wire the executor and (if user-facing) the frontend

- Wire the planner into its registry/loop (call the new intent planner in
  `classify_route` or the turn loop; register the new tool in `tools/mod.rs`;
  add the `PromptDef` to the registry — placing any variable bits per Phase 2b).
- Keep the executor thin — dispatch the plan, do the I/O, return the result.
  Integration-test it against the trait fakes (`AiProvider`, `Clock`,
  `MailProvider`, `Keychain`, `Logger`), never real clients.
- **Feature gating** (user-facing): advertise `is_available(&db)` on a new tool
  so the registry hides it when the feature is off (the LLM never sees a tool
  it can't use, and it doesn't tax the prompt budget). Add the toggle the way
  the draft-review vertical does: a `featureToggleStore` entry + a Settings
  panel like `AiReviewSettings.tsx`, persisted to **SQLite** (never
  localStorage).
- **Logging:** emit `app-log` events for user-visible operations (levels
  info/success/error/debug, sources sync/embeddings/account/ai/system).
- **i18n:** new user-facing strings get keys in all locales (`en`, `es`, `fr`,
  `de`). Respect locked terms: "Embeddings" stays English everywhere; the
  reasoning-trace `tool:` label stays "tool:" in Spanish.
- **Docs:** update the module's `MODULE.md` if you changed its public surface.

## Phase 6 — Eval gate (mandatory for reply-moving changes)

If the change can move an AI reply (a new route, tool, or prompt edit all do),
run the matching eval **scoped to related cases**, not the full suite:

| Change | Eval |
|--------|------|
| Chat / route / planner / tool | `chat_eval` (`make cli-eval ARGS="--case <id> --json"` or `cargo run --features eval --example chat_eval -- --case <id>`) |
| Shortcut fast-path | `chat_shortcut_eval` |
| Draft generation | `draft_eval` |
| Draft review | `draft_review_eval` |
| Classification | `email_classification_eval` |
| Lens / memory / task / invoice / agent-search | the matching `examples/*_eval.rs` |

- Eval canonical model/provider: `qwen3.5-4b-q4_k_m` on `llamacpp` (matches
  what ships). The `make eval-*` / `make cli-eval` targets default to it.
- If **no** existing case covers the new behaviour, add a focused one in this
  same change (synthetic → `src-tauri/evals/<area>/cases/`; personal-mailbox →
  `private-evals/` — see Phase 7).
- A planner that adds a deterministic route usually wants a unit test (Phase 4)
  **and** a chat eval case proving the end-to-end reply is still correct on
  that route. Both must be green.
- Cap iteration at 3 unsuccessful eval re-runs. After three, stop and report
  what's failing — don't grind.

Then run the local quality gates on what you touched:

- Rust → `make test-fast` (or `make test` if you touched `llamacpp` paths) +
  `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` +
  `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`.
- TS → `npx biome check src/` + `npx tsc --noEmit`.
- Large change / explicitly requested → `npx lefthook run pre-commit`.

## Phase 7 — Offer to graduate eval cases

A new AI feature should leave behind a permanent regression case. Decide the
home by data sensitivity:

- **Synthetic prompts, no personal-mailbox content** → public
  `src-tauri/evals/<area>/cases/` (e.g. `chat/cases/`). Ship in-tree; never put
  real names/emails/subjects in them. You can add these without asking,
  following the existing case schema.
- **Depends on the user's real mailbox** (a specific sender, subject, id) →
  `private-evals/` (gitignored). **Ask the user before adding** — never copy in
  silently. Read `private-evals/CLAUDE.md` for the schema first.

Ask with a concrete one-liner when proposing a private case:

> This feature is worth a regression case at
> `private-evals/chat/cases/<id>.yaml` (account=<email>, asserts <expectation>).
> Add it? (y/n)

Only proceed on yes. Confirm the case passes via the matching eval, then note
it in the report.

## Phase 8 — Report

Chat-style, before → after. **No** raw `--trace` payload, system prompt, or
full search results unless asked.

```
Feature: <one-line capability> — <seam, file:line>
Compatibility: <OK | cost + fix applied>

Before:
User: <prompt verbatim>
A: <old behavior — e.g. "RAG-first, model round-trip, 4.2s">

After:
User: <prompt verbatim>
A: <new behavior — e.g. "direct search_emails(from:X), no model round-trip, 0.3s">
   tools: <tool(args)>            ← if a tool ran
   (route=<route>, <N> rounds, Xs)

Tests: <unit test names> — GREEN
Eval:  <harness --case id> — <metric / PASS>
```

### Final line (grep-friendly)

```
Feature: <file:line>. Tests: <names>. Eval: <case> PASS. Status: GREEN.
```

If a private regression case was accepted, add:

```
Regression case: private-evals/<path>.yaml (passing).
```

Then offer the detail: "Want the raw `--trace`, the full diff, or the system
prompt? Ask and I'll paste."

## Common pitfalls (avoid)

- **Building a new subsystem when a registry entry would do.** New tools,
  prompts, routes are one file each — don't scaffold infrastructure.
- **Putting variable content in the system prompt.** Dates, names, counts,
  retrieved snippets in the system prefix bust the KV-prefix anchor every turn
  (`ColdPrefill`). Keep them out of the prefix — inject after the stable block.
- **Ignoring the context budget.** A bigger prompt / tool array can front-
  truncate real turns at `n_ctx=8192`. Check before, gate or trim if it's big.
- **Writing the executor before the planner is tested.** The planner is the
  decision; test it exhaustively first. The executor is plumbing.
- **A model call where a heuristic suffices** (or vice versa). Intent matching
  is deterministic — a heuristic is cheaper, testable, no per-turn round-trip.
- **Declaring done without an eval** when the change moves replies. A passing
  unit test is necessary but not sufficient — the eval is the reply-moving gate.
- **Over-triggering a new route/shortcut.** Pin the boundary with negative
  tests (open-ended questions that must still reach the model / RAG).
- **localStorage for a feature toggle.** Preferences live in SQLite.
- **Real personal data in a public eval case.** Synthetic only in-tree;
  personal-mailbox cases go to gitignored `private-evals/` with the user's ok.
- **Forgetting i18n / feature gating** on a user-facing feature, or leaving
  `MODULE.md` stale after changing a module's surface.
```
