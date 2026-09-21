# services/help_docs

## What this module owns

Answering questions about **the EmailOps app itself** from the bundled user guides
(`docs/site/<lang>/<page>.md`), inside the ordinary chat turn — "how do I connect Ollama?",
"where is my data stored?", "why is chat slow?" — plus the navigation the app performs when
an answer cites a guide section.

- **corpus.rs** — pure. `include_str!`s every guide in the four site languages, splits each
  page into heading sections (long ones into parts), reads the page's `nav:` front matter and
  fingerprints the whole corpus. Section N means the same thing in every language (the docs
  parity check pins heading counts), which is what lets a hit be served in the answer's
  language.
- **index.rs** — planner + executor. `ensure_text_index` rebuilds `help_doc_chunks` +
  `help_docs_fts` when the corpus hash changed (synchronous, cheap); `ensure_embeddings`
  fills `vec_help_docs` for the active embedding model in batches (async, best-effort).
- **retrieval.rs** — `lookup_help` fetches FTS + KNN candidates, fuses them with the shared
  RRF, and the pure `plan_help_sources` collapses to one source per section, gates on
  vector similarity (`HELP_MIN_SIMILARITY`, overridable with `chat.help_min_similarity`)
  and swaps each kept section for its sibling in the UI language.
- **prompt.rs** — pure. Renders the `EMAILOPS HELP` block that rides in the final user
  message (never the system prompt) with the `help://<lang>/<page>#<anchor>` links the
  answer must cite.
- **nav.rs** — pure. The allowed `nav:` targets (`settings/<tab>`, `view/<mode>` — kept in
  sync with the frontend unions) and `plan_help_navigation`, which navigates only when the
  finished answer actually cites a section that carries a target.

## Dependencies

- `db/help_docs.rs` — all SQL (rebuild, embedding upsert, FTS/KNN fetch, sibling lookup)
- `services/retrieval` — `fuse_rrf`
- `ai/provider.rs` — `AIProvider::embed` / `embed_batch`
- `services/chat/turn.rs` is the only caller: lookup after mailbox retrieval, block
  prepended to the final user message, `ToolEffect::NavigateTo` after the answer

## Public surface

- `ensure_index(db, provider)` / `ensure_text_index(db)` / `ensure_embeddings(db, provider)`
- `lookup_help(db, provider, query, query_embedding, ui_lang, k) -> (Vec<HelpSource>, HelpTrace)`
- `render_help_block(&[HelpSource], app_help: bool) -> Option<String>` — `app_help` is the
  query planner's verdict that the question is about EmailOps; the block then instructs
  unconditionally instead of asking the model to judge whether it applies
- `plan_help_navigation(answer, &[HelpSource]) -> Option<(NavTarget, &HelpSource)>`

## What should NOT live here

- Editing the guides: they are the docs site's source of truth (`docs/site/README.md`).
- Mailbox retrieval, prompt assembly, the tool loop — `services/chat`.
- The frontend side of `help://` links and the `navigateTo` effect — `src/lib/chatToolEffects.ts`,
  `src/components/Chat/MarkdownContent.tsx`.
