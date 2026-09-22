---
name: maintain-docs
description: Keep EmailOps' published documentation true as the app changes — every paragraph, list item, table and code block of the docs is a catalogued claim (docs/site/claims.toml) verified against the code, existing tests or the running app, or explicitly marked manual; run the checks, fix what fails (in all four languages), catalogue any new prose, and report it with an HTML run showing each claim and its result. Covers docs/site/{en,es,fr,de}, README.md, ROADMAP.md, a DECISIONS.md prompt and a website-copy notice. Use before cutting a release (the release skill's Phase 5d calls this), after landing a feature or fix that changes a view, setting, CLI flag, model or user-visible string, when `make docs-check` reports failures, or whenever the developer asks whether the docs are still accurate.
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
make docs-check                      # guards + contract tests, seconds
make docs-check ARGS="--with-app"    # also drives the docClaim() cases through the real UI
```

The report prints as a `file://` link — open it. Every case is listed with its
result, and a failing one carries the edit it expects. Use `--with-app` before
a release and whenever a change touched the UI; the fast pass marks the
app-driven claims as skipped rather than omitting them, so it never reads as
"all clear" while testing less.

The report groups claims by page. Each row quotes the claim, says how it was
proven, and — when it fails — why and what edit it expects. A claim is proven
one of four ways, and the report files it under the strongest that ran:

| Type in the report | How the claim is proven | On failure |
|---|---|---|
| **Comprobado en la app** | a `docClaim()` in sweep.mjs drives the real UI (`--with-app`) | the app changed, **or the page was always wrong** — decide which before editing |
| **Tests existentes** | named Rust tests that prove the behaviour exist and pass | the behaviour changed, or the test was renamed |
| **Código fuente** | a `quoted` literal is in both the claim and the code/config/release script, or the code has/lacks something | the code moved, or the page was edited past what the catalogue quotes |
| **Sin prueba automática** | nothing automatic can prove it (performance, third-party behaviour, whole-program promises like "no phone-home") | shown as **MANUAL** with its reason — never counted as OK |

Plus the structural guards, which vouch for the claims as a set:

| Guard | Proves | On failure |
|---|---|---|
| `check-docs-parity.sh` | same pages, sidebar weights and `{#anchors}` in all four languages | a language is missing a page or an anchor a cross-page link targets |
| `check-docs-labels.sh` | every UI label the docs tell you to click exists verbatim in that language's locale | the app renamed a control, or a translation paraphrased it |
| `check-docs-paths.py` | every repo path quoted in any `.md` resolves | a file moved or was deleted and the prose did not follow |
| `check-docs-claims.py` | every block is marked in all four languages, every marker is catalogued, every `app` check has its `docClaim()` | new prose was added uncatalogued, or a marker was lost in a translation |

**A locale string proves a label, never a feature.** The app's own help text can
be as stale as the docs — it still promised a "find similar" feature that does
not exist. Evidence for a feature is code: `src/lib/api.ts` is the one path from
the UI to the backend.

## 2. Coverage — the part no script can do

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
| Model added, resized or retired (`ai/model_catalog.rs`) | `ai-features.md` table ×4 — the contract test fails first |
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

## 3. Fix

Apply the corrections, prose included, in all four languages. When writing
es/fr/de, open `src/locales/<lang>/` and copy the label — do not translate it
by eye.

A failed `doc` claim has two possible culprits. Decide which before editing:
the app regressed (fix the app, with a regression test — that is a bug, not a
docs task), or the page over-claimed (fix the page). The report's proposed fix
assumes the page is wrong because that is the common case; it is not a verdict.

Re-run step 1 until green.

## 4. Cataloguing new or changed prose

`check-docs-claims.py` fails the commit the moment a block has no marker, so
this is not optional:

1. Mark the block in **all four** languages: `<!-- claim:some-id -->` on its own
   line above a paragraph, table or code block; at the end of the **last** line
   of a list item (the first line can sit inside a `**bold span**` that wraps).
2. Add `[some-id]` to `docs/site/claims.toml` with the strongest check that
   applies — see the file header for the forms. Prefer, in order: an existing
   test that proves the behaviour; a `quoted` literal shared by the claim and the
   code; an `app` check; and only then `manual` with an honest reason. `none` is
   for lead-ins that make no claim of their own.
3. For an `app` check, add `docClaim('some-id', feature, page, expect, fix, fn)`
   in `.claude/skills/verify-emailops/scripts/sweep.mjs`, next to the steps for
   that screen so it reuses the navigation already done. Ground selectors in the
   live DOM first (`$V wd find 'button=…'`, see `verify-emailops`) — the first
   run of a claim is as likely to expose a bad selector as a bad page.
4. `uv run --no-project scripts/check-docs-claims.py`, then `make docs-check`.

When a `manual` claim becomes provable — a test lands, a sweep step is added —
upgrade its entry. The count of MANUAL rows in the report is the honest size of
what is still taken on trust.

## 5. Report

Chat-style, as `fix-ai-bug` does. Cover: what was checked and at which layer;
what was broken and why (code moved, or prose was always wrong); what was
fixed, listing the languages; what still needs the developer — a DECISIONS
entry, website copy, or a claim that could not be verified; and the `file://`
link to the HTML run plus the uncommitted diff to review.

Do not paste the raw report unless asked.
