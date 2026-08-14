# Changelog

All notable changes to EmailOps are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

No unreleased changes yet.

## [0.6.6] — 2026-08-14

### Added

- **Chat panel docked on the right** — a resizable chat alongside the inbox, in
  addition to the full-page view. With an email open the panel offers that
  thread as context via a removable chip, and answers from it instead of
  searching. The context applies to one turn and is never saved onto the
  conversation, so you can move between emails inside a single chat.
- **Chat shows which account it is answering from, and lets you change it.**
  Chat searches one account at a time. A picker in both the docked panel and the
  full-page view names that account and switches it. Each account keeps its own
  conversation for as long as the app is open, so moving between them returns
  you to where you were instead of a blank chat.
- **Every calendar of an account, each in its own colour** — previously only the
  primary calendar was fetched, so a calendar shared with you was invisible in
  EmailOps while visible in Google/Outlook. All calendars now sync, tinted with
  the colour the provider gives them, and each can be hidden or shown from the
  legend above the grid or from Settings → Calendar.

### Changed

- **The chat panel is docked open by default.** Closing it still sticks.
- **"Chat" in the sidebar opens the full-page chat view** instead of toggling
  the docked panel. The panel has its own close button, and the new-chat icon
  beside the inbox reopens it.
- **Choosing an account keeps mail and chat in step.** Selecting a single
  account in the sidebar points chat at it, and changing chat's account moves
  the mail list to match — except in "All accounts", which stays as you left it.
- **One macOS download for every Mac.** The separate Intel build is retired; the
  universal download launches on both Apple Silicon and Intel.
- **Embedded local AI is no longer offered on Intel Macs.** Its GPU kernels
  require Apple Silicon, so on Intel it now says so up front and points at
  Ollama or OpenRouter, instead of failing mid-answer with a decode error.

### Fixed

- **The app crashed on quit** for anyone using the embedded local AI. Quitting
  produced a macOS crash report every time because the AI runtime was still
  loaded as the process exited. It now shuts down cleanly. The standalone
  `emailops-cli` was affected the same way.
- **Asking about the open email searched the whole inbox instead.** With an
  email open and its context chip showing, a question like "summarise this
  email" could answer "you haven't said which email you mean" and then
  summarise unrelated threads — whenever the open email belonged to a different
  account than the one the chat was running on, which is the normal case in the
  "All accounts" view. When context genuinely cannot be used, the app now says
  so rather than silently answering from search.
- **An answer still being written is no longer lost** if you switch account or
  conversation while it generates. Coming back showed an empty reply with no
  sign anything was still running, and only a second visit revealed the answer.
  The reply and its progress now come back with you.
- **Asking about an email from another account now says so.** In "All accounts"
  you can be reading an email from one account while chat answers from another.
  Chat now names the account that email belongs to and offers to switch, rather
  than quietly answering from the wrong mailbox.
- **Calendar questions in chat could not reach your calendar.** "What's my next
  meeting?" (or the Spanish equivalent) was answered from email instead, since
  nothing routed calendar wording to the calendar. Phrasings that happened to
  include a date word worked by accident; the rest did not.
- **Internal tool names leaked into answers** — a reply could end with a stray
  `list_calendar_events` line.
- **"Generate with AI" disappeared when the compose window was maximised.**
- **A blank gap above the inbox rows after closing an email.** Going back from
  an email left the list looking empty, with a few rows stranded at the bottom
  edge, until you scrolled. An earlier fix restored the scroll position but not
  the list's own idea of where it was, and hiding the list also made every
  visible row measure as zero-height. Both are now handled, so the list comes
  back exactly as you left it.
- **A background sync no longer throws away the pages you had loaded.** Scrolled
  a long way down, a sync could snap the list back to the first 50 messages
  underneath you.

### Security

- Updated `dompurify` (3.4.13), which sanitises email HTML — the advisory covers
  an XSS via a subtree left executable after a hook is removed, the exact
  pattern EmailOps uses.
- Updated `js-yaml` (4.3.1) and `nanoid` (3.3.18), both denial-of-service
  advisories in build-time tooling.
- Updated `postcss`, `undici` and `brace-expansion` for known advisories.

## [0.6.5] — 2026-08-03

### Added

- **Linux and Windows releases** — `.deb`/`.AppImage` for Linux, `.msi`/NSIS
  installer for Windows, both with optional GPU acceleration via Vulkan
  (auto-detects a compatible driver, falls back to CPU otherwise).
- **Windows CUDA build** — an additional NVIDIA-only download for users who
  specifically want it; Vulkan remains the recommended default since it
  covers more hardware.

### Fixed

- **Security:** stopped sending the stored IMAP password to the frontend
  process; malformed stored password hashes are now rejected instead of
  silently accepted.
- **Security:** block remote media and unsafe URL schemes in untrusted email
  content.
- OAuth account connection no longer hangs indefinitely after the browser
  opens.
- Calendar sync no longer treats a rate-limited retry response as success.
- Fixed dropdown popup styling and positioning issues on Linux and Windows.
- Fixed a blank band appearing above inbox rows after returning from an
  email.

## [0.6.4] — 2026-07-24

### Fixed

- **Gmail sync respects Google's rate-limit window**: when Gmail says
  "retry after \<time\>", sync stops retrying, pauses that account's requests
  until the window reopens, and resumes on the next scheduled sync —
  previously each operation burned six rapid retries against the exhausted
  quota and flooded the log with warnings.

## [0.6.3] — 2026-07-24

### Added

- **Calendar view** with provider sync, invites, and reminders.
- **Unified "All accounts" inbox** across enabled accounts.
- **IMAP custom folder sync and in-app folder management** — create, rename,
  delete, and move folders (including drag-and-drop), with localized folder
  detection.
- **New-release notifications** via a sticky toast and sidebar link.
- **Attachment downloads to the Downloads folder** with a "Show in Finder"
  toast.
- **In-thread search and pinch-zoom** in email bodies.
- **Sent emails appear instantly** via optimistic local insert.
- **App version and commit SHA** shown under the sidebar logo.
- **Homebrew distribution** — install via the `emailops/homebrew-tap` cask.

### Fixed

- **Send and account-save errors are surfaced** instead of being silently
  hidden.
- **Chat answers that contradict tool results are retried.**
- **Keychain secrets consolidated into one vault item**, reducing keychain
  prompts.
- **Sync error banner shows which account is failing.**
- **Email row menu flips above the button** when near the viewport bottom.

### Changed

- Dependency bumps to latest compatible versions.

## [0.6.2] — 2026-07-09

### Fixed

- **Chat no longer returns empty replies or "can't access your mailbox"
  refusals on long analytical prompts**: degenerate tool calls are repaired
  (filterless searches get the question's verbatim address, id-less body reads
  walk the search results), mangled sender addresses are auto-corrected on
  empty results, and malformed tool-call syntax (flattened args, trailing
  braces, missing closing tags) is tolerated.
- **Empty final answers now retry with tool-call salvage** and, as a last
  resort, show a localized rephrase hint instead of a blank bubble.
- **Chat input auto-grows with the message** instead of a fixed 2-row height.

## [0.6.1] — 2026-07-08

### Added

- **Give feedback button** in the sidebar — opens a pre-filled email
  (general feedback / bug report / idea) to hello@getemailops.com in the user's
  chosen UI language, with the app version, OS, and active AI provider/model
  auto-filled into a "technical info" line.
- **Draft-with-AI button in compose** — generates a draft from the recipients,
  subject, and notes, honoring the configured output language.
- **Compose + provider drafts with Gmail / Outlook sync** — drafts saved locally
  push to the provider on save and pull back on sync.
- **Qwen 3.6 35B A3B** and **Qwen 3.5 4B Q8** chat models added to the local
  catalog.
- **Configurable embedded context window (`n_ctx`)** for the llama.cpp runtime.
- **Model-backed query planner** with oldest-first email search.
- **Styled CLI terminal output** plus run-scoped prompt overrides.

### Fixed

- **Chat category scope no longer widens beyond the current selection.**
- **Raised the `get_email_body` cap to 16,000 characters** so long emails are no
  longer truncated for the model.
- **Chat model selection from the CLI is honored** (`/model` and `--model`).
- **Sync re-extracts attachments** that were missed at their original sync time.

### Changed

- **Hardened chat**: no-think fast path, tool-call salvage, and planner dates +
  trace surfaced in the reasoning trace.

### Security

- Patched 5 RustSec advisories via dependency bumps:
  - **ammonia** 4.1.2 → 4.1.3 — mXSS via MathML `annotation-xml` encoding strip
    (RUSTSEC-2026-0193). ammonia is the email-HTML sanitizer, so this is
    directly relevant.
  - **plist** 1.9.0 → 1.10.0, pulling **quick-xml** 0.39.4 → 0.41.0 — quadratic
    parse time on duplicate attributes + unbounded namespace-declaration
    allocation (RUSTSEC-2026-0194 / RUSTSEC-2026-0195, both high).
  - **quinn-proto** 0.11.14 → 0.11.16 — remote memory exhaustion from unbounded
    out-of-order stream reassembly (RUSTSEC-2026-0185, high).
  - **crossbeam-epoch** 0.9.18 → 0.9.20 — invalid pointer dereference in the
    `fmt::Pointer` impl (RUSTSEC-2026-0204).

## [0.6.0] — 2026-06-16

### Added

- **Headless CLI (`emailops-cli`)** — power-user / agent command line over the
  same `services::*` entry points the desktop app uses. One-shot mode for
  scripting plus an interactive REPL. A signed universal binary now ships
  alongside the app DMG, with a dedicated user guide under `docs/`.
- **Reusable llama.cpp KV cache across chat turns.** Chat prefill drops ~50%
  on follow-up questions; cache reuse and prefill latency are surfaced in the
  reasoning trace.
- **File attachments on email replies.**
- **Compose opens directly from `mailto:` links** in email bodies.
- **Classification uses the main AI model.** Drops the per-feature override
  that forced a llama.cpp runtime swap on every background classification.
- **`classify --id <ID>`** CLI flag for targeted single-email reclassification,
  with the full result (priority / intent / topic / confidence / method)
  returned in the response.

### Fixed

- **"AI returned empty response for classification" on Qwen 3.5.** One-shot
  completions now prime Qwen 3 family models with the canonical
  `<think>\n\n</think>` block, so they skip the unbounded reasoning span and
  emit JSON directly. Confirmed on 10 real-inbox emails: 100% success at
  ~1.8 s avg (was 30% at ~8.3 s avg before the fix).
- **Release builds compile warning-free again.** Dev-only trace formatters
  are now `#[cfg(debug_assertions)]`-gated alongside their call sites.
- **Frontend CI.** `@biomejs/cli-darwin-x64` moved to `optionalDependencies`
  so Linux `npm ci` no longer fails with `EBADPLATFORM`.

### Security

- Patched 5 npm advisories (2 high, 2 moderate, 1 low):
  - **vite** 8.0.14 → 8.0.16 (Windows NTLMv2 hash disclosure via launch-editor
    + `server.fs.deny` bypass on Windows alternate paths).
  - **dompurify** 3.4.7 → 3.4.10 (Trusted Types policy survives `clearConfig`
    + `SAFE_FOR_TEMPLATES` bypass inside `<template>`).
  - **js-yaml** → 4.2.0 via npm `overrides` (merge-key quadratic-time DoS).
  - **esbuild** → 0.28.1 via npm `overrides` (Deno-only NPM_CONFIG_REGISTRY
    binary integrity bypass; not exercised by our Node-only build but pinned
    for cleanliness).

## [0.5.2] — 2026-06-08

### Added

- **Gemma 4 12B Instruct** is available as a local chat model (replaces Gemma 3
  12B; runs in 16 GB+ RAM).
- **Per-tool chat status labels** show which tool the assistant is running.
- **Live assistant prose** now streams during the tool-calling round instead of
  appearing only after the tools finish.

### Fixed

- **Latest email by sender** now returns the newest message regardless of Gmail
  category — an `updates`-category newsletter no longer hides behind an older
  `primary` email for an explicit "last email from X" query.
- **Reasoning and tool-call markup no longer leak into chat answers.** Gemma
  `<|channel>` / `<|tool_call>` and Qwen `<think>` spans are stripped from
  user-visible output, including mid-stream.
- **Startup no longer aborts opaquely**, and local backups no longer grow
  unbounded.
- **Reasoning "Flow" panel** is unified, tool ordering is corrected, and prompt
  status is shown.
- **Thread-bound chat** drafts real emails, dedupes chip warnings, decodes HTML
  entities, and no longer clips conversation context.
- **Full-width layout** closes the open email when switching views.
- **Email footer** no longer clips, and list/sidebar render bugs are fixed.
- **Sent view** shows sent copies, and the EmailOps footer is localized.

## [0.5.1] — 2026-06-01

### Fixed

- **Case-insensitive `from:` search.** Sender-address searches now match
  regardless of the casing used in the query or the stored address.
- **PDF attachment previews render** instead of opening a blank tab.
- **Send with a typed-but-untokenized recipient.** Composing now sends even
  when the recipient address was typed but not yet converted into a chip/token.
- **Account setup ordering.** The account row is now inserted before its
  credentials are stored, avoiding a setup failure during account creation.

### Changed

- **Stable release DMG names.** Release builds publish a versionless
  `EmailOps-macos.dmg` so the GitHub `latest` download link is permanent.

## [0.5.0] — 2026-05-29

First public release, re-released with the multi-language UI and AI language
preference updates. Source history was squashed prior to publication; commits
with hashes before this release exist only in the private archive.

### Added

- **Multi-language UI (i18n).** EmailOps now ships in **English**, **Spanish**,
  **French**, and **German**. The UI language is auto-detected from the OS
  locale on first launch and falls back to English when the system locale is
  not one of the four supported languages. Override it any time in
  **Settings → Appearance**.
- **Decoupled AI output language.** A new dropdown in **Settings → AI**
  controls which language the assistant replies in. The default is
  **"Same as UI"** — pick a specific language only if you want the AI to
  always respond in (for example) English while keeping the UI in Spanish.
  The legacy Portuguese / Italian / Catalan options have been removed (they
  were never fully supported elsewhere in the product).
- `get_system_locale` Tauri command used by the frontend bootstrap to
  pick a sensible default when `ui_language` is unset.
- Multi-account **Gmail** sync (OAuth via PKCE; tokens stored in the OS keychain).
- **Outlook / Microsoft 365** sync via Microsoft Graph OAuth.
- **IMAP / SMTP** support with presets for iCloud, Yahoo, Fastmail, Outlook,
  and ProtonMail Bridge (plus arbitrary custom servers).
- Local-first storage: emails, threads, attachments, and embeddings in SQLite.
- **Sandboxed email HTML rendering** in a null-origin iframe, with a
  DOMPurify-based sanitizer that preserves `<style>` blocks and inline CSS
  while blocking `javascript:` / `expression(...)` / data-URL HTML/SVG payloads.
- **Remote-content gating** per sender (load-images banner with persistent
  trusted-sender allowlist).
- **AI provider abstraction** with embedded **llama.cpp** (default), local
  **Ollama**, and remote **OpenRouter** backends.
- **Semantic search** powered by `sqlite-vec` and configurable embedding models.
- **AI classification** (priority, intent, topic, company) with rule-based
  short-circuits.
- **Lenses**: saved scopes that combine search + filter + classification.
- **AI draft generation** using persona + thread history.
- **Rich-text compose** with inline images.
- Background **task queue** (separate `ai_queue` and `db_queue` so a long
  AI job cannot starve lightweight DB work).
- Tauri 2.x desktop bundle (macOS `.app` + `.dmg`).

### Changed

- **AI output language default flipped from Spanish to English.** Existing
  installs that previously had no `ai_output_language` preference set were
  implicitly defaulting to Spanish; that default is now English. Users who
  prefer Spanish output can re-select it under Settings → AI. Saved
  preferences are honoured — only the previously-implicit default changes.
- Backend AI services (classification, chat, memory extraction, task
  extraction) now resolve the AI output language through a single
  `services::i18n::resolve_ai_language` helper instead of duplicating the
  preference-read logic. The chain is:
  `ai_output_language_v2` → legacy `ai_output_language` → `ui_language` →
  English default.
- Eval rubrics and judges no longer assume Spanish-only mailboxes. The
  shortcuts language heuristic now scores stop-word ratios for English,
  Spanish, French, and German; the agent-search LLM judge is no longer
  primed with "most emails are in Spanish".

### Security

- Production CSP enabled in `src-tauri/tauri.conf.json`.
- Dev-only plaintext token storage gated behind `cfg!(debug_assertions)`.
- OAuth state validated on every callback; the local listener tolerates
  malformed requests without crashing.
- Sync errors fail loudly rather than masquerading as "already processed".

### Known limitations

- Bundle is currently macOS-only; Windows and Linux targets are planned.

[Unreleased]: https://github.com/emailops/emailops/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/emailops/emailops/releases/tag/v0.5.0
