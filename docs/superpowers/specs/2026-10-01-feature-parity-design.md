# Feature parity matrix and completeness reviewer — design

Date: 2026-10-01. Branch: `chore/feature-parity-matrix` (off `main`).

## Problem

Features land complete on the entry point the change touched and missing on its
siblings. Trigger case: draft autosave (`15211c7`, 2026-07-03) went into `ComposeModal`
and `ComposeTabView` but not `ReplyCompose`, which serves Reply, Reply All and Forward.
Forward (`f975766`, 2026-08-17) was added to `ReplyCompose` later and did not pick it up
either. The same pattern elsewhere in the composers:

- save-on-close (`flushPending`) exists only in the modal; the tab loses the last 0.8 s;
- pre-send warnings (`findSendWarnings`) exist only in `ReplyCompose`;
- Escape closes only the modal.

Root causes:

1. A change is scoped to the component it touches, not to the capability across every
   entry point that offers it.
2. `features/compose.md` names Reply as an entry point, but its `compose.draft`
   sub-feature describes the modal only. Nothing crosses capabilities with entry points.
3. The e2e sweep checks that the Reply buttons exist (`sweep.mjs:64`) and never opens them.
4. No skill or CLAUDE.md defines "done" beyond gates and evals.

## Goal

A parity gap is visible and fails `make verify`. A new entry point or a new user-facing
capability cannot land without being declared.

Out of scope: fixing the gaps the audit finds (separate branches), and merging the three
composers into one (a candidate follow-up the audit data will argue for or against).

## Decisions (developer, 2026-10-01)

- Approach: a machine-checked matrix **and** an LLM reviewer (not either alone).
- Every `gap` cell fails verification, always — not only at release.
- The reviewer runs before every commit and before push/PR, alongside `privacy-reviewer`.

## Design

### 1. `## Parity` matrix in each `features/<feature>.md`

A fifth H2 in every feature file; the contract in
`.claude/skills/verify-emailops/features/README.md` changes from four H2s to five.

- Rows: the ids from `Sub-features` (`compose.draft`, …).
- Columns: the user's entry points (what "How to get to it" lists in prose today).
- Each cell is exactly one of:
  - a test reference: `e2e:<Feature>/<step>`, `vitest:<file>::<test name>`,
    `rust:<file>::<fn>`, `integration:<test fn>`;
  - `n/a: <reason>` — the capability does not apply to that entry point;
  - `gap: missing — <what>` — not implemented there;
  - `gap: untested — <what>` — implemented, but no test proves it.
- A feature with one entry point still has the table, with one column. That column is
  what forces the next entry point to be declared.

Example (first pass on compose, 2026-10-01; references abbreviated):

| Capability | Compose modal | Compose tab | Reply | Reply all | Forward |
|---|---|---|---|---|---|
| `compose.draft` | `e2e:Compose/cerrar y borrador` | `e2e:Compose/borrador con tabla` | `gap: missing` | `gap: missing` | `gap: missing` |
| `compose.flush-on-close` | `gap: untested` | `gap: missing` | `gap: missing` | `gap: missing` | `gap: missing` |
| `compose.send-warnings` | `gap: missing` | `gap: missing` | `gap: untested` | `gap: untested` | `gap: untested` |

### 2. `check_parity.py`

A new script in the verify-emailops `scripts/` directory, run by `verify_all.py` in the
`static` layer (so `--tier quick` runs it too).

- Parses the `## Parity` table of every feature file.
- Resolves references statically: the step name in `sweep.mjs`; the test name in the
  vitest file; the `fn` in the Rust file; the integration fn under `src-tauri/tests/`.
- Emits one result per cell, attributed to the feature, layer `static`. Each of these
  is a `fail`:
  - a missing feature file section or an empty cell;
  - `n/a` without a reason;
  - a reference that does not resolve;
  - any `gap`.
- `summary_md.py` and `report.py` get a "Parity gaps" section (count per feature,
  list of cells) and the delta against the previous run, like "Since <previous run>".
- Tests: `test_check_parity.py`, same style as `test_summary_md.py`, on synthetic
  tables: empty cell, reasonless `n/a`, broken reference, each `gap` kind, green table.

### 3. `completeness-reviewer` subagent

A new agent definition next to `privacy-reviewer.md` in `.claude/agents/`, read-only, shaped like `privacy-reviewer.md`,
scope `staged` | `branch`.

- Input: the diff, plus the `## Parity` section of every feature the diff touches
  (mapped with the logic of `maintain-verification/scripts/verify_changes.py`).
- Verdicts, one per finding:
  - `MISSING-ROW` — the diff adds a user-visible capability with no row;
  - `MISSING-COLUMN` — the diff adds an entry point (component, button, route, CLI
    command) with no column;
  - `SIBLING-SKIPPED` — the diff changes a capability in one entry point and not in a
    sibling whose cell is not `n/a`;
  - `OK`.
- Runs when `privacy-reviewer` runs: before a commit (`staged`), before push/PR (`branch`).

### 4. Workflow integration

- Root `CLAUDE.md`, Git Conventions: run `completeness-reviewer` with `privacy-reviewer`;
  a three-line definition of done — gates green, the touched features' `## Parity` has
  no empty cell and no unresolved reference, reviewer reports no finding.
- `maintain-verification/SKILL.md`: step 1 signal row "new entry point or user-visible
  capability → row/column in `## Parity`"; step 2 "a `gap` is closed by a test or a
  fix, never by `n/a`"; step 4 "update `## Parity`".
- `build-ai-feature/SKILL.md`, phase 1: declare the parity rows and columns affected.
- `fix-ai-bug/SKILL.md`, root cause: check the sibling columns for the same defect.
- `sweep.mjs`: a capability with several columns is written as one loop over its entry
  points (the shape of the settings-tab loop), so closing a gap adds an entry to a list.
- `docs/DECISIONS.md`: entry "Feature parity matrix gates verification"
  (Rejected: LLM reviewer alone; gaps blocking only at release; merging the composers now).

### 5. Initial audit

Fill `## Parity` in all 15 feature files, compose first, by reading each entry point's
component and the existing tests. The first pass on compose (10 capabilities × 5 entry
points) found about 13 `gap: missing` and about 30 `gap: untested`. `make verify` will
be red on them until each one is fixed or proven; that is expected.

Not a parity gap, tracked in `NEXT-SESSION.md` for its own fix: editing a reply draft
sets `drafts.email_id` to NULL (`src-tauri/src/db/drafts.rs` upsert;
`src/lib/composeDraft.ts` never sends `emailId`), so `send_draft` sends it as a new
message outside the thread.

## Verification

1. `test_check_parity.py` red, then green.
2. `make verify ARGS="--tier quick"`: compose gaps appear as `fail` in `static`;
   "Parity gaps" appears in the summary.
3. Reviewer: on a synthetic diff that adds a capability to `ReplyCompose` only →
   `SIBLING-SKIPPED`; on `f975766` → flags `compose.draft` for Forward.
4. `make gates SET=commit` green; `privacy-reviewer` on `staged`.
