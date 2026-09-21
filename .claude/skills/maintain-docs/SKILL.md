---
name: maintain-docs
description: Keep EmailOps' published documentation true as the app changes — run the doc guards and doc↔code contract tests, drive the docClaim() cases against the real UI, then fix what they find (in all four languages) and report it with an HTML run. Covers docs/site/{en,es,fr,de}, README.md, ROADMAP.md, a DECISIONS.md prompt and a website-copy notice. Use before cutting a release (the release skill's Phase 5d calls this), after landing a feature or fix that changes a view, setting, CLI flag, model or user-visible string, when `make docs-check` reports failures, or whenever the developer asks whether the docs are still accurate.
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

What each layer proves, and what a failure means:

| Layer | Proves | On failure |
|---|---|---|
| `check-docs-parity.sh` | same pages, sidebar weights and `{#anchors}` in all four languages | a language is missing a page or an anchor a cross-page link targets |
| `check-docs-labels.sh` | every UI label the docs tell you to click exists verbatim in that language's locale | the app renamed a control, or a translation paraphrased it |
| `check-docs-paths.py` | every repo path quoted in any `.md` resolves | a file moved or was deleted and the prose did not follow |
| `check-docs-claims.py` | every `<!-- claim:id -->` has a `docClaim()` and vice versa, in all four languages | a paragraph was rewritten past its marker, or a case was renamed |
| contract tests | `ai-features.md` / `getting-started.md` match `model_catalog.rs` | a model was added, resized or retired without touching the published figures |
| `doc` claims | the app still does what a marked paragraph promises | the app changed, **or the page was always wrong** — decide which before editing |

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

## 4. Adding a claim

Worth doing when a paragraph tells the reader a control exists or is named
something specific, on a screen the sweep already visits — the marginal cost is
then one assertion, not an app launch.

1. Put `<!-- claim:some-id -->` on the line before the paragraph, in **all four**
   languages.
2. Add a `docClaim('some-id', feature, page, expect, fix, fn)` in
   `.claude/skills/verify-emailops/scripts/sweep.mjs`, next to the existing
   steps for that screen so it reuses the navigation already done.
3. `uv run --no-project scripts/check-docs-claims.py` to confirm the pairing.

Ground every selector in the live DOM first rather than guessing
(`$V wd find 'button=…'`, see the `verify-emailops` skill). `fix` is shown in
the report when the claim fails: say which paragraph to edit and in which
languages.

## 5. Report

Chat-style, as `fix-ai-bug` does. Cover: what was checked and at which layer;
what was broken and why (code moved, or prose was always wrong); what was
fixed, listing the languages; what still needs the developer — a DECISIONS
entry, website copy, or a claim that could not be verified; and the `file://`
link to the HTML run plus the uncommitted diff to review.

Do not paste the raw report unless asked.
