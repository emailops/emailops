---
name: maintain-verification
description: Keep EmailOps' full verification honest as the app changes — map what changed since the last verified commit to features and test layers, add the missing tests (unit, integration, contract, e2e sweep steps, oracle checks, eval cases), update selectors, recipes and the feature manifest when UI or behaviour moved, re-run `make verify`, and record the verified commit. Use after landing a feature or fix, before a release, when `make verify` reports new failures or coverage gaps, or when a view, command, setting, migration or AI tool is added.
---

# Maintain the verification

`make verify` (see `.claude/skills/verify-emailops/`) runs every test layer once and
attributes each result to a feature from `features.json`. That map, the sweep steps, the
oracle checks, the eval cases and the recipes rot the moment the app changes. This skill is
the upkeep loop. The unit of work is the **feature touched**, and every touched feature
leaves with every layer it needs, or an explicit written reason it does not.

Rules that apply throughout: TDD (failing test first, then code); no personal mailbox data
in anything tracked (demo persona only; personal cases go to `private-evals/` after asking);
never weaken an assertion or mark a test n/a to make a run green; hooks green before commit.

## 1. What changed

```bash
python3 .claude/skills/maintain-verification/scripts/verify_changes.py          # since verified.json
python3 .claude/skills/maintain-verification/scripts/verify_changes.py <commit> # or explicit
```

It lists touched files per feature and, from the last `*-full` run, which layers that
feature has and which are missing. Files under "sin mapear" mean `features.json` needs a
new prefix or a new feature: fix the map first, it drives everything else.

Read the signals a diff carries, each with the layer it demands:

| Change | Layer to add or update |
|---|---|
| New sidebar view / route (`ViewMode`, `Sidebar.tsx`) | e2e: a `step('Vistas', …)` in `sweep.mjs`, a file in `features/`, the feature in `features.json` |
| New or changed Tauri command (`#[tauri::command]`, `src/lib/api.ts`) | unit on the service planner; contract: args/return in `api.ts` match Rust (ts-rs export), CLI envelope if exposed |
| Changed UI strings (`src/locales/en/*.json`) | e2e/oracle selectors that used the old text (`button=…`, `aria/…`, placeholders); i18n parity is automatic |
| New setting tab | e2e: the settings-tab loop in `sweep.mjs`; unit for its store round-trip |
| DB migration (`src-tauri/migrations/V*.sql`) | contract: schema parity runs automatically; **oracle SQL** in `tagboard_check.mjs` (and any other `*_check.mjs`) if it reads a touched table |
| New chat tool, prompt edit, retrieval tweak | eval: a case in `src-tauri/evals/<kind>/cases/` (synthetic); drive the change itself through the `build-ai-feature` skill |
| Layout/CSS change in a pane the sweep measures | ui: a measurement step (see the Tag Board toolbar check) |
| New list/grid of user data | oracle: `<feature>_check.mjs` with the three layers (backend ↔ DB, UI ↔ backend, UI ↔ DB) and `data-*` hooks on rows |

## 2. Coverage check before writing anything

```bash
make verify ARGS="--tier quick"        # static + rust + vitest + contract, ~5 min
```

Open `src-tauri/reports/verify/current-full/informe.html` → "Huecos de cobertura". For every
touched feature decide per missing layer: **add** (default), or **n/a with a written reason**
in the feature file (e.g. "no integration seam: pure UI"). A feature touched in this change
never keeps a missing layer silently.

## 3. Add or update tests, one layer at a time

- **Unit**: Rust `#[cfg(test)]` next to the planner; vitest next to the component or lib
  (`*.test.ts[x]`, `createRoot` + `act`, `vi.mock` as in the existing files).
- **Integration**: `src-tauri/tests/integration.rs` against `FakeEmailProvider` and the
  in-memory DB; name it with the feature's keyword (`integration_names` in `features.json`).
- **Contract**: schema parity (`db::schema_parity_tests`), CLI `--json` envelope, i18n
  parity, ts-rs types. Name the test so the `contract` patterns in `features.json` catch it.
- **e2e**: a `step(feature, name, expect, fn)` in `sweep.mjs`. Ground every selector in the
  live DOM first (`$V launch`, `$V wd find 'button=…'`); prefer text/aria selectors; add
  `data-testid` hooks when text is ambiguous or translated. Update `features/<feature>.md`.
- **Oracle**: extend `tagboard_check.mjs` or clone its shape for another list. The oracle
  SQL reproduces the backend rule with no cap; the UI is compared to the backend's ranked
  output; rows are compared to the DB set.
- **Eval**: a YAML case in `src-tauri/evals/chat/cases/` keyed to the demo persona
  (`ulises@emailopslabs.dev`), deterministic anchors, no judge. Run it alone first:
  `make cli-eval ARGS="--case <id> --json"`.

Red first: run the new test, watch it fail for the right reason, then make it pass.

## 4. Update the map and the recipes

- `features.json`: new prefixes, new feature, new `integration_names`, budgets.
- `features/*.md`: the four H2s (`Sub-features`, `How to get to it (user POV)`,
  `Driving it with verify.sh`, `Gotchas`) and the "test kind per case" table.
- Selectors in `sweep.mjs` / `*_check.mjs` when a string or placeholder changed.

## 5. Full run, then triage

```bash
make verify                            # all layers, ~25 min with evals
```

Every failure is one of: **product bug** (fix in a separate commit, with its regression
test), **test drift** (fix the test or selector, in this commit), **infrastructure** (port
busy, model not loaded: fix the environment, re-run). Say which for each in the summary.
Flaky evals (small local model) are reported as model-quality signals, not silenced.

## 6. Record and commit

Update `.claude/skills/verify-emailops/verified.json` with the commit, date and counts of
the green run:

```bash
python3 .claude/skills/maintain-verification/scripts/record_verified.py
```

Commit tests and map changes as `test: …`, product fixes as `fix: …`, each on its own.
Never commit `src-tauri/gen/schemas/*.json` churn from the webdriver dev build
(`git checkout -- src-tauri/gen/schemas`).
