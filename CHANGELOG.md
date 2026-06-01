# Changelog

All notable changes to EmailOps are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

No unreleased changes yet.

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
