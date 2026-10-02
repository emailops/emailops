# Coverage and mutation testing

Two local tools measure how well the test suite guards the code, on top of the
gates (`make gates`) and the full verification (`make verify`):

| Target | Tool | Measures |
|---|---|---|
| `make coverage-rust` | cargo-llvm-cov | Rust lines, regions and functions the tests execute |
| `make coverage-ts` | vitest `--coverage` (`@vitest/coverage-v8`) | TypeScript statements, branches, functions and lines |
| `make coverage` | both | both, plus a per-feature ranking |
| `make mutants ARGS=…` | cargo-mutants | Rust only: whether the tests *fail* when the code is changed |

Coverage says what code the tests run; mutation testing says whether they would
notice if that code were wrong. A line can be 100% covered by a test that
asserts nothing about it.

Neither runs in CI or in the gates. Both are developer tools, run by hand at the
cadence below.

## Coverage

```bash
make coverage          # Rust + TS, ~3 min + ~20 s on an M-series Mac
make coverage-rust     # Rust only
make coverage-ts       # TS only
```

Reports, all under the gitignored `src-tauri/reports/coverage/`:

- rust/html/index.html — browsable per-file report
- rust/summary.json — `cargo llvm-cov report --json --summary-only`
- rust/lcov.info and rust/coverage.json — line and region data (input of the ranking)
- ts/index.html, ts/coverage-summary.json, ts/coverage-final.json
- by-feature.md — per-feature and per-file ranking, largest uncovered spans,
  files under 40% (`scripts/coverage_by_feature.py`)

### What is measured

- **Rust feature set:** `--no-default-features`, exactly what CI's Rust test job
  runs (`ci.yml`). That build leaves out `commands/` (the `desktop` feature) and
  `ai/llama_cpp/` (the `llamacpp` feature), so neither appears in the report.
  The local `make gates` `rust-test` gate runs the default features instead; the
  difference is those two trees and the llama.cpp-only tests.
- **Branch coverage:** stable rustc has none (`--branch` is nightly-only).
  Region coverage is the closest stable proxy and is what the ranking reports.
- **Test code:** `coverage_by_feature.py` removes inline `#[cfg(test)] mod …`
  blocks and test-only files (`tests.rs`, `test_helpers.rs`, …) before counting,
  so a file's percentage is about its production code. The totals printed by
  `cargo llvm-cov` itself include test code and read higher.
- **TypeScript:** everything under `src/` except tests, `*.d.ts`, `src/types/`
  and `src/main.tsx` (config in `vite.config.ts`, `test.coverage`). The plain
  `vitest run` gate does not collect coverage.
- **Features:** files are attributed with the prefixes of
  `.claude/skills/verify-emailops/features.json` (`rust` module prefixes,
  `vitest` path prefixes; the longest match wins, unmatched files are
  "Transversal") — the same map `make verify` uses.

## Mutation testing (Rust)

cargo-mutants rewrites one piece of code at a time (`<` into `<=`, a function
body into `Default::default()`, a match arm deleted …), rebuilds, and runs the
tests. A mutant the tests fail on is **caught**; one they pass is **missed** — a
behaviour no test pins.

```bash
make mutants ARGS="--list --file src/sync/http_retry.rs"          # what would be tried
make mutants ARGS="--file src/sync/http_retry.rs -- -- sync::"     # fast pass
SHARDS=2 make mutants ARGS="--file 'src/services/junk/*.rs' -- -- services::junk"
git diff --relative=src-tauri main -- src-tauri/src > /tmp/pr.diff
make mutants ARGS="--in-diff /tmp/pr.diff"                         # only what a branch changed
RECHECK=<label> make mutants                                       # re-test a run's misses, whole suite
```

Paths in `--file` are relative to `src-tauri/`. `--re` / `-F` filter by
mutant name, but in cargo-mutants 27 they do not filter struct-field deletions
("delete field … from struct … expression"), so combine them with `--file`. Results land in
`src-tauri/reports/mutants/<LABEL>/[shard-N/]mutants.out/` (`caught.txt`,
`missed.txt`, `timeout.txt`, `unviable.txt`, `outcomes.json`, one log per
mutant); `LABEL` defaults to a timestamp.

### How `scripts/mutants.sh` runs it

- **Never in this checkout.** It creates (or moves to `HEAD`) the detached
  worktree `.claude/worktrees/mutants` and runs `cargo mutants --in-place`
  there, so a crash can never leave mutated code in the working tree. Mutants
  are generated from `HEAD`: **commit a new test before re-running** to see it
  catch something. The config is read from this checkout, so an edited
  `src-tauri/.cargo/mutants.toml` applies at once.
- **Own target dir per worktree** (`/Volumes/Build/emailops/mutants/target`,
  linked by `scripts/build_target.sh`). cargo names the crate's artifacts
  without the checkout path, so a worktree building into the main checkout's
  target overwrites its library and incremental cache: measured 43 s per mutant
  build instead of 4–6 s while the main checkout (or a second shard) was also
  building. The first run in a new worktree builds the dependencies once
  (~2 min, ~3.5 GB). At the start of each run the crate's own incremental
  cache and artifacts are dropped (dependencies stay), which bounds the disk the
  run starts from and costs one full crate build.
- **Disk.** Each shard's target grows by about 1 GB per 15 minutes of mutants
  (incremental codegen objects in `deps/`), so a single long run (the
  ~550-mutant junk scope took ~50 min on two shards) can grow two targets to
  8–9 GB each. Do **not** delete those objects while a run is going: the next
  incremental build links against them and fails, and cargo-mutants then
  reports the mutant as unviable. The script refuses to start under 12 GB free;
  on a tight disk use `SHARDS=1` and split long scopes into several runs. The
  worktrees' target dirs are disposable between runs:
  `rm -rf /Volumes/Build/emailops/mutants*/target` costs one dependency build.
- **Incremental builds** stay on (the crate's `.cargo/config.toml` default): a
  mutant rebuild takes 4–6 s against ~19 s non-incremental.
- **`--no-default-features`**, plus `FEATURES` when set. Without `llamacpp`,
  `src/ai/llama_cpp/` and the one llamacpp-gated helper outside it are excluded
  — a mutant in code that is not compiled would be reported as missed. For the
  llama.cpp planners, run `FEATURES=llamacpp make mutants ARGS="--file
  src/ai/llama_cpp/tool_parser.rs …"` (the native library is built once per
  worktree target).
- **Lib unit tests only** (`additional_cargo_args = ["--lib"]` in
  `mutants.toml`): each mutant pays one link, not a second one for
  `tests/integration.rs` plus a doctest pass.
- **Excluded files** (`exclude_globs`): entry points and Tauri wiring,
  `commands/`, `cli/`, `evals/`, the WebDriver server, test helpers and fakes,
  and log/event/spawn glue. See the comments in `mutants.toml`.
- **Timeouts:** tests ×3 of the baseline (never under 60 s); builds ×10 of the
  baseline incremental build.

### Runtime

The whole lib suite takes ~35 s, so a mutant tested against it costs ~45 s. The
fast pass passes a test-name filter (`-- -- <module path>`): only that module's
tests run, and a mutant costs ~10 s per shard (~5–6 s effective with
`SHARDS=2`). Two-stage workflow:

1. **Fast pass** per module with its own filter (e.g. `-- -- services::junk`).
2. **Recheck** the misses against the whole library and integration suite:
   `RECHECK=<label> make mutants`. What still survives is a real gap or an
   equivalent mutant.

To resume an interrupted run, or re-test only what an earlier run did not catch
after adding tests, pass `--iterate` with the same `LABEL`: cargo-mutants skips
every mutant listed in that output dir's `caught.txt` and `unviable.txt`.

`--list` shows the size of a scope before running it; 1,000 mutants is roughly
1.5 h with two shards.

### What to do with a missed mutant

1. **A test gap** — write the test that fails on the mutant and passes on the
   original (apply the mutation by hand, or re-run the mutant after committing).
2. **An equivalent or log-only mutant** (no observable behaviour changes) —
   record it in [MUTANTS-LEDGER.md](MUTANTS-LEDGER.md) with a one-line reason.
   Gaps that need a seam first (a clock, a provider fake) are listed there too.
3. **A product bug** — fix it in its own `fix:` commit with the regression test.

Timeouts and unviable mutants (the rewrite does not compile) are reported by
cargo-mutants and need no action unless a timeout repeats.

## The planner scope

The modules mutated so far, with the test filter used for the fast pass. Pure
planners and decision functions first; executors that only call a provider or
the database are left to the integration suite.

| Area | `--file` (from `src-tauri/`) | Fast-pass filter |
|---|---|---|
| Chat planner and routing | `src/services/chat/{planner,routing}.rs` | `services::chat` |
| Research mode | `src/services/chat/research/{control,mode,plan,notes,reading}.rs` | `services::chat::research` |
| Sync planners | `src/sync/{draft_plan,folder_plan,http_retry}.rs` | `sync::` |
| Outlook uploads | `src/sync/outlook_upload.rs` | `sync::outlook` |
| Mailbox state | `src/services/emails/{state_refresh,history_refresh,uid_validity,mailbox_state,optimistic,reconcile,html_sanitizer}.rs`, `src/db/emails/mailbox_state.rs` | the file's own module |
| Attachments and AI activity | `src/services/{attachment_safety,ai_activity}.rs` | the file's own module |
| Junk detector | `src/services/junk/{verdict,lookalike,content,auth,model,mod,tokens,signals,config}.rs` | `services::junk` |
| Classification rules | `src/services/classification.rs` with `-F 'normalise\|glob_to\|compile_rules\|rule_based_classify\|extract_json\|build_classify_prompt\|_definition'` | `services::classification` |
| Search query builders | `src/db/emails/search.rs` | `db::emails::search` (recheck against the whole library: most of its callers' tests live elsewhere) |
| AI stream parsing | `src/ai/{openrouter_stream,utf8_stream,prompt_guard}.rs` | `ai::` |

About 1,600 mutants; on 01/10/2026 the first fast pass caught 78.8% of the
viable ones. Not yet run: `src/services/chat/budget.rs` and the other files the
review-hardening branch changed (`--in-diff`), and `src/ai/llama_cpp/{planner,tool_parser}.rs`
(`FEATURES=llamacpp`, about 180 mutants, needs the llama.cpp native build in the
worktree's target).

## Recommended cadence

- **Before merging a PR that touches a pure planner** (anything named
  `plan_*`, a `*_plan.rs`, a parser or a normaliser): `make mutants` with
  `--in-diff` against `main`, fast pass then recheck. Minutes, not hours.
- **Before a release:** the planner scope above, module by module (~3 h on one
  shard, ~2 h on two when the disk allows), and `make coverage` to spot new
  uncovered areas. Compare `by-feature.md` with the previous release's.
- **Not in CI for now:** a full-crate run is ~14,000 mutants. A nightly job over
  the planner scope, or `--in-diff` on PRs that touch `services/`, `sync/` or
  `ai/`, would be the next step if the developer wants it.
