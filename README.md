# EmailOps

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
![Status: Alpha](https://img.shields.io/badge/status-alpha-orange.svg)

> **Alpha software** — EmailOps is in early development. Expect rough edges, breaking changes, and missing features. It is not recommended as your primary email client yet.

Privacy-first, Local AI-native desktop email client (currently Mac only).

## Screenshots

![EmailOps chat screenshot](.github/assets/chat_screenshot.jpg)

## Download

Download **macOS** releases from: [Releases page](https://github.com/emailops/emailops/releases/latest).

Windows and Linux releases are on the [roadmap](ROADMAP.md).

## Features

- **Multi-account email**: Gmail, Outlook / Microsoft 365 (Graph API), and IMAP/SMTP (iCloud, Yahoo, Fastmail, ProtonMail Bridge, custom servers)
- **Chat with your emails**: use AI to answer questions about your emails, generate drafts, ...
- **AI email classification**: auto-tag emails by priority, intent, and topic using configurable rules + local AI
- **Smart filters**: filter inbox by domain, sender, or classification tags
- **AI draft generation**: context-aware reply drafts using persona + thread history
- **Attachments view**: organize and access your attachments directly without searching in emails
- **AI provider abstraction**: embedded llama.cpp (default), local Ollama, or remote OpenRouter — switchable per feature

## Data + privacy model

- **Email content**: stored locally in SQLite in your OS app data directory.
- **OAuth tokens**: stored in the OS keychain via the Rust `keyring` crate (not in files).
- **AI**: by default runs in-process via the embedded **llama.cpp** runtime — no separate daemon, no network calls. Switching the provider to Ollama or OpenRouter is opt-in per feature.
- **Network**: external calls are limited to email provider APIs (Gmail, Microsoft Graph, IMAP/SMTP servers you configure) and — only if you've explicitly enabled it — the OpenRouter API.

## Troubleshooting

- **AI not available**: by default models run via embedded llama.cpp; make sure the recommended model has finished downloading from the in-app model manager. If you've switched the provider to Ollama, ensure the daemon is running and reachable at `http://localhost:11434`.
- **Semantic search returns keyword results**: generate embeddings from the in-app settings
- **Chat is slow:** depending on your machine and the model used this feature can take dozens of seconds. Try with a smaller model. We're working on making it faster

---

**The sections below are for developers building EmailOps from source.**

## Tech stack

- **Desktop**: Tauri 2.x
- **Backend**: Rust (Tokio, rusqlite, reqwest, oauth2, keyring)
- **Frontend**: React + TypeScript (Vite)
- **State**: Zustand
- **Styling**: Tailwind CSS
- **AI**: embedded **llama.cpp** runtime (default); optional Ollama (`http://localhost:11434`) or OpenRouter backends

## Prerequisites

- **Node.js** (recommended: latest LTS) + **npm**
- **Rust toolchain** (stable) + **Cargo**
- **Tauri system dependencies**
  - Follow the official Tauri prerequisites for your OS: `https://tauri.app/start/prerequisites/`
- **CMake** + a working C++ toolchain — required to build the embedded `llama-cpp-2` crate. On macOS, Xcode Command Line Tools cover this.
- **Ollama** is **optional**. Only install it if you want to route AI through Ollama instead of the embedded llama.cpp runtime:
  - Install Ollama: `https://ollama.com/`
  - Start the daemon and pull whichever models you plan to use, e.g.:

```bash
ollama pull llama3.2
ollama pull nomic-embed-text
```

## OAuth setup (required for Gmail / Outlook accounts)

1. **Gmail**: create OAuth credentials in [Google Cloud Console](https://console.cloud.google.com/apis/credentials) (type: **Desktop app**, enable Gmail API)
2. **Outlook / Microsoft 365**: register an application in the [Microsoft Entra admin center](https://entra.microsoft.com/) with the Graph API mail scopes (a public-client / desktop redirect)
3. **IMAP / SMTP** accounts don't need OAuth — add them directly from the in-app onboarding wizard.
4. Copy `.env.example` to `.env.local` in both the project root and `src-tauri/`:

    ```bash
    cp .env.example .env.local
    cp src-tauri/.env.example src-tauri/.env.local
    ```

5. Fill in your Gmail and/or Outlook credentials in both `.env.local` files.

## Running the app (dev)

Common workflows are wrapped in the top-level [`Makefile`](Makefile). The most useful targets:

```bash
make install     # npm install + lefthook install
make dev         # run Tauri against the repo-local dev data directory
make dev-fresh   # run against a throwaway data dir (safe for experiments)
make check       # lint + typecheck + clippy + Rust + frontend tests
make test        # cargo tests
make test-fast   # cargo tests with the embedded llama.cpp feature disabled (faster iteration)
make build       # production frontend build
make demo        # launch against a synthetic demo DB (auto-generated on first run)
```

`make dev` stores app data under an ignored repo-local directory by default.
Set `EMAILOPS_DATA_DIR=...` in `.env.local` when you need to point local
workflows at a different app data directory.

If you'd rather not use `make`:

```bash
npm install
npm run tauri dev        # full desktop app
npm run dev              # frontend-only in a browser
```

See the `Makefile` for the full list (release/notarization targets, evaluation manifests, etc.).

## Bundled Model License + Attribution

EmailOps bundles one embedding model in macOS releases:

- `nomic-embed-text-v1.5-q4_k_m.gguf` from Nomic AI (GGUF conversion)

Attribution and license terms for bundled/discoverable models are tracked in [`MODEL_LICENSES.md`](MODEL_LICENSES.md). Please review those terms before redistributing binaries.

## Git hooks

The repo uses [lefthook](https://github.com/evilmartians/lefthook) to run lint, typecheck, clippy, fmt, and tests on `pre-commit` / `pre-push`. `make install` wires them up automatically; if you ran `npm install` directly, install them with:

```bash
npx lefthook install
# or:
make hooks
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup instructions, coding standards, and PR guidelines.

## Security

See [SECURITY.md](SECURITY.md) for the privacy/security model and how to report vulnerabilities.

## License

[Apache 2.0](LICENSE)
