# Published user documentation

Everything in this directory is **user-facing documentation published to
<https://getemailops.com/docs/>**. It is the source of truth for those pages — the
marketing site holds only the templates and CSS that render them.

Contributor-facing docs (`../DECISIONS.md`, `../cli.md`) stay outside this
directory and are never published.

## Layout

```
docs/site/
  en/   es/   fr/   de/      ← one directory per site language
```

Every language holds the **same filenames** — `installation.md`, `getting-started.md`,
… — because the filename becomes the URL (`/es/docs/installation/`). Renaming a file
means renaming it in all four.

## The four-language rule

**Every change ships in all four languages, in the same PR.** A page that exists only
in English is worse than no page: the nav shows it, the reader clicks it, and gets a
language they did not ask for. If you cannot translate a change, say so in the PR
rather than merging a partial one.

Run `scripts/check-docs-parity.sh` from the repo root before opening the PR. It fails
if the page sets, sidebar weights or heading anchor ids diverge between languages.

## Stable anchors

Headings that are the target of a cross-page link carry an explicit id:

```markdown
### With local AI {#with-local-ai}
### Con IA local {#with-local-ai}
```

The heading text is translated; **the id is not**. That keeps
`../ai-features/#the-model-catalog` valid in every language instead of needing four
different fragment spellings. When you add a link to a heading, give that heading an
id first — in all four files.

Current ids: `with-local-ai`, `direct-download`, `linux` (installation);
`choosing-a-backend`, `the-model-catalog`, `classification` (ai-features);
`where-your-data-is-stored` (privacy-security).

## How it reaches the site

The [`getemailops.com`](https://github.com/gerodp/getemailops.com) repository runs
`scripts/sync-docs.sh` during its build. That script clones this repository at the
**latest `v*` release tag** and copies `docs/site/<lang>/*.md` into
`content/<lang>/docs/` before Hugo runs. So:

- Merging a docs change here does **not** publish it. It goes live with the next
  release tag.
- The site does not rebuild on its own when this repo changes. Trigger a deploy in
  Amplify (or push to the site repo) after tagging.

## Editing rules

- Each file needs Hugo front matter — `title`, `description`, and a `weight` that
  sets its position in the sidebar (10, 20, 30, …). `_index.md` is the section
  landing page and takes no weight. `weight` values must match across languages so
  the sidebar order does.
- `description` is used three times: the sidebar card, the subtitle under the H1,
  and the `og:description` meta tag. Keep it to one sentence.
- Links between pages are relative to the published URL (`../ai-features/`), not to
  paths in this repo.
- These pages quote concrete values — model sizes and RAM floors from
  `src-tauri/src/ai/model_catalog.rs`, setting labels from
  `src/locales/en/settings.json`, CLI flags and exit codes from
  `../cli-user-guide.md`, the macOS floor from `../../homebrew/Casks/emailops.rb`.
  When you change any of those, update the matching page in the same PR — in all four
  languages.
- Keep UI labels in each page matching the app's own translations in
  `src/locales/<lang>/`. If the German UI says „Einstellungen → KI-Suche“, the German
  docs must not say „Einstellungen → AI-Suche“.

## Preview locally

From a checkout of the site repo, next to this one:

```bash
DOCS_SRC=../../emailopsv2 npm run docs:sync
hugo serve
```
