---
name: maintain-docs
description: Keep EmailOps' published documentation true as the app changes — every paragraph, list item, table and code block of the docs is a catalogued claim (docs/site/claims.toml) verified by running the app itself — a fresh install, a locked relaunch, the demo mailbox and the CLI — or, where the app cannot show it, against the published release, the tests, or explicitly marked manual; run the checks, fix what fails (in all four languages), catalogue any new prose, and report it with an HTML run that renders the docs themselves, every sentence coloured green (validated), yellow (not validatable or not validated yet) or red (wrong) and tagged with how it was checked; where no deterministic check reaches, the agent judges the sentence against the evidence the app run collected. Covers docs/site/{en,es,fr,de}, README.md, ROADMAP.md, a DECISIONS.md prompt and a website-copy notice. Use before cutting a release (the release skill's Phase 5d calls this), after landing a feature or fix that changes a view, setting, CLI flag, model or user-visible string, when `make docs-check` reports failures, or whenever the developer asks whether the docs are still accurate.
allowed-tools: Bash, Read, Edit, Write, Grep, Glob
---

# Keep the published docs true

`docs/site/{en,es,fr,de}/` is the **source of truth** for
<https://getemailops.com/docs/> — 8 pages per language. The site repo does not
track them: its `content/*/docs/` is gitignored and its sync script `rm -rf`s
each destination and re-copies from here before every Hugo build. Never edit
them there.

Prose is not compiled, so it rots silently and always in the same direction:
the code moves and the page stays still. Every drift found when this skill was
written had an untouched `.md` and a changed source file. This skill is the
upkeep loop. The unit of work is the **claim** — something the docs assert
about the app — and every claim leaves either proven, corrected, or explicitly
recorded as unverifiable.

**The docs are fully catalogued.** Every paragraph, list item, table and code
block in `docs/site/en/*.md` carries a `<!-- claim:id -->` marker (the same
markers, on the same blocks, in es/fr/de), and every id has an entry in
`docs/site/claims.toml` saying how it is verified. `check-docs-claims.py` fails
on any unmarked block, so new prose cannot slip in unverified — it runs in
pre-commit.

## Rules that apply throughout

- **Four languages, one change.** Every edit to `docs/site/` ships in en, es,
  fr and de together. A section added only to English leaves three quarters of
  readers reading about an older product.
- **Quote the app, never translate it.** UI labels come from
  `src/locales/<lang>/`, copied verbatim. Paraphrasing is exactly how the
  Spanish page came to send readers hunting for a switch the app never called
  "correo basura".
- **Anchor ids are not translated.** `{#the-model-catalog}` stays identical in
  all four languages; only the heading text changes.
- **No personal mailbox data** in anything tracked — demo persona only.
- **Everything lands uncommitted.** The developer reviews the diff before it is
  committed, and always before a tag.
- **Never commit or push the website repo.** Report what its copy needs; stop
  there.

## 1. Run the checks

```bash
make docs-check                      # guards, release assets, code tests — minutes, no app
make docs-check ARGS="--with-app"    # the real verification: runs the app itself
```

**The app is the ground truth.** A sentence being true of the code is not the
same as it being true of the app a reader installs: a locale string can exist
and never be rendered (the docs quoted "In-app (local)"; the UI says "In-app"),
a default can be pinned in one backend and not the other. So `--with-app` is
the verification, and the fast pass is a pre-flight — it marks every app check
as pending (yellow), never as validated. Run `--with-app` before a release and after
any change a user could see.

`--with-app` drives the app in four phases (`scripts/check_docs_app.sh`):

| Phase | What runs | What it proves |
|---|---|---|
| `fresh` | a brand-new data dir | the first-run wizard both ways (AI and plain), factory defaults in every settings tab, what lands on disk; ends by setting a main password |
| `locked` | the same data dir, relaunched | the lock screen, and that the SQLite file stays readable — the app locks, it does not encrypt |
| `demo` | the synthetic demo mailbox | reading pane, unified inbox, calendar, attachments, tasks, memory, tag board, chat, AI Search |
| `cli` | `emailops-cli` on a copy of the demo DB | every command, flag, exit code and JSON shape the CLI page shows; the REPL through a pseudo-terminal |

**The report is the docs, coloured.** One section per page: a summary table
(fragments per kind of validation × colour), the recommended actions, then the
page as a reader sees it. Every **fragment** — a sentence, a table row, a code
block — is coloured and ends in one tag per validation that covers it; hover a
tag for how it was checked, the evidence, and what to do:

| Tag | How the fragment is checked |
|---|---|
| `[APP]` | a `claim('<id>', …)` case drives the app (fresh install, locked relaunch, demo mailbox) |
| `[CLI]` | a case in `scripts/docs_cli_claims.py` runs `emailops-cli` |
| `[TST]` | named tests prove behaviour the demo cannot drive (a real Gmail server, the keychain, an Intel Mac) |
| `[COD]` | only where the file *is* the thing: the cask, `tauri.conf.json`, `release.yml`, `LICENSE` |
| `[GEN]` | the table is generated from code (`make docs-gen`; `crate::docs_sourcegen`) |
| `[REL]` | the asset is on the latest GitHub release |
| `[AGT]` | the agent's judgment on the evidence (step 2) |
| `[MAN]` | manual, with its reason; or the fragment asserts nothing |
| `[SIN]` | nothing covers it |

**Green is earned per sentence.** A fragment is green only when a check that
*quotes it* passes — `covers` in the case or catalog entry — and, for app
cases, the case read its expected value from the docs and tested behaviour.
Yellow says why not: the check does not declare what it covers, its
expectation is typed into the case, it only saw a label, manual, pending, or
nothing covers it. Red: a check that covers it fails, a check quotes text the
page no longer has, or the agent contradicts it.

A claim can combine an app check with a `manual` entry for the part the app
check does not reach ("renaming folders is not exercised: the demo has none"),
with `covers` naming the sentences each one is about.

Plus the structural guards, which vouch for the claims as a set:

| Guard | Proves | On failure |
|---|---|---|
| `check-docs-parity.sh` | same pages, sidebar weights and `{#anchors}` in all four languages | a language is missing a page or an anchor a cross-page link targets |
| `check-docs-labels.sh` | every UI label the docs tell you to click exists verbatim in that language's locale | the app renamed a control, or a translation paraphrased it |
| `check-docs-paths.py` | every repo path quoted in any `.md` resolves | a file moved or was deleted and the prose did not follow |
| `check-docs-claims.py` | every block is marked in all four languages, every marker is catalogued, every `app` check has its case | new prose was added uncatalogued, or a marker was lost in a translation |

**A failing app check is sometimes an app bug.** The first run of this design
found the docs right and the app wrong twice: the model recommendation on a
16 GB Mac, and the Junk settings being unreachable without AI. Report those to
the developer; do not bend the page to match a bug.

## 2. Judge what the checks do not reach

After a `--with-app` run, pack the evidence and judge the yellow fragments:

```bash
uv run --no-project python scripts/docs_judge_pack.py <run dir>   # → <run>/judge/packets.json
```

Each packet is one sentence, its block, the screens the app showed while that
block's cases ran (and the files a `judge` catalog entry names). Read every
packet and write `<run>/judge/judgments.json` in the format documented at the
top of `docs_judge_pack.py`, then fold it in:

```bash
bash scripts/check_docs.sh --render <run dir>
```

Rules for judging:

- **Evidence first.** `supported` needs a quote from the evidence that shows
  the sentence is true; put it in `evidence`. No quote, no `supported`.
- **`insufficient` before guessing.** A screen that does not show the thing is
  not proof either way.
- **`contradicted` needs the contradicting quote and the edit** (`fix`). It
  turns the fragment red and goes to step 4, where a human decides.
- Name the judging model in `model`. The report shows `[AGT]` verdicts as the
  agent's, apart from deterministic checks.
- A sentence the agent keeps having to judge is a case waiting to be written:
  prefer adding a deterministic check (step 5) over re-judging it every release.

## 3. Coverage — the part no script can do

The checks prove that what the docs *say* is true. They cannot notice what the
docs **fail to say**. A page can be perfectly accurate about the old feature
set and never mention what shipped since.

Walk the `### Added` and `### Changed` entries of the release's CHANGELOG
section one at a time and grep `docs/site/` for a distinctive phrase from each.
If nothing comes back, that feature is undocumented — say so explicitly rather
than reporting "docs look fine". **Absence of a stale sentence is not
coverage.** v0.6.6 shipped a docked chat panel and multi-calendar sync; both
were absent from the docs while every existing sentence was still correct, and
the release went out before anyone noticed.

Read the diff for signals, each with the page it touches:

| Change | Docs to revisit |
|---|---|
| Model added, resized or retired (`ai/model_catalog.rs`) | `make docs-gen` regenerates the `ai-features.md` table ×4; review the prose around it |
| New settings tab (`SettingsTab` in `SettingsDialog.tsx`) | the `**Settings → …**` path ×4 |
| New sidebar view (`ViewMode` in `Sidebar.tsx`) | `features.md` / `ai-features.md` ×4 |
| New CLI subcommand or flag (`cli/mod.rs`) | `cli.md` ×4 and `docs/cli.md` |
| Changed UI string (`src/locales/en/*.json`) | every page quoting that label ×4 |
| New platform or installer | `README.md` download section, `installation.md` ×4, website copy |
| macOS floor changed (`homebrew/Casks/emailops.rb`) | `installation.md` ×4 |

Also check, each release:

- **`README.md`** — does the download section still match the platforms that
  actually ship?
- **`ROADMAP.md`** — does anything there read as pending that this release
  completed? (It no longer restates a version number; do not reintroduce one.)
- **`docs/DECISIONS.md`** — **ask** the developer whether anything here is a
  durable decision worth logging. That file is append-only and
  durable-decisions-only; not every release earns an entry, so ask rather
  than assume.
- **Public website** (`getemailops.com`, a separate Hugo repo) — report what
  its download/feature copy needs. Do not commit or push there.

## 4. Fix

Apply the corrections, prose included, in all four languages. When writing
es/fr/de, open `src/locales/<lang>/` and copy the label — do not translate it
by eye.

A failed `doc` claim has two possible culprits. Decide which before editing:
the app regressed (fix the app, with a regression test — that is a bug, not a
docs task), or the page over-claimed (fix the page). The report's proposed fix
assumes the page is wrong because that is the common case; it is not a verdict.

Re-run step 1 until nothing is red.

## 5. Cataloguing new or changed prose

`check-docs-claims.py` fails the commit the moment a block has no marker, so
this is not optional:

1. Mark the block in **all four** languages: `<!-- claim:some-id -->` on its own
   line above a paragraph, table or code block; at the end of the **last** line
   of a list item (the first line can sit inside a `**bold span**` that wraps).
2. Add `[some-id]` to `docs/site/claims.toml`. **Default to `{ app = true }`**:
   if a reader could see it in the app, the app is where it gets checked. A
   reference table belongs in a generated region (`{ generated = "<test>" }`).
   Fall back, in order, to `release`, `tests`, a `file` that *is* the thing, and
   only then `manual` with an honest reason. Give every non-app check `covers`
   for the sentences it proves. See the file header.
3. For an app check, add a literal case in the phase that reaches that screen:
   `doc_claims.mjs` (fresh install / locked), `doc_claims_demo.mjs` (demo mailbox)
   or `scripts/docs_cli_claims.py` (CLI):

   ```js
   await claim('some-id', 'what', {
     covers: ['the exact words it verifies'],   // as the reader sees them: no ** or `
     how: 'Una frase: qué pantalla abre, qué lee y con qué lo compara.',
     proof: 'behaviour',                        // 'label' if it only sees a text
   }, async ({ doc }) => {
     const n = doc.number(/up to (\w+) steps/);  // expected value FROM the docs
     …
   });
   ```

   The expected value comes from `doc` (`doc.text`, `doc.match`, `doc.number`,
   `doc.bold`) — a case that never reads its claim is reported as a fixed
   expectation. Test the behaviour, not the help text: change the setting and
   observe the effect where that is cheap. Sentences a case does not quote stay
   yellow; cover them with another case, a `manual` entry with `covers`, or
   leave them to the agent. Never loop over ids: the completeness guard only
   sees literal ids.
4. Ground selectors in the live DOM first (`VERIFY_DATA_DIR=<empty dir>` gives a
   fresh instance; see `verify-emailops`). Pick toggles by the label of their
   row, never "the first toggle": the sidebar has its own.
5. `uv run --no-project scripts/check-docs-claims.py`, then
   `make docs-check ARGS="--with-app"`.

When a `manual` claim becomes observable — a screen gains data, a phase learns
to reach it — upgrade it. The yellow count in the report is the honest size of
what is still taken on trust.

## 6. Report

Chat-style, as `fix-ai-bug` does. Cover: what was checked and at which layer;
what was broken and why (code moved, or prose was always wrong); what was
fixed, listing the languages; what still needs the developer — a DECISIONS
entry, website copy, or a claim that could not be verified; and the `file://`
link to the HTML run plus the uncommitted diff to review.

Do not paste the raw report unless asked.
