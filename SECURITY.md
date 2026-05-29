# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in EmailOps, please report it
privately. **Do not open a public GitHub issue.**

Open a [private security advisory](https://github.com/emailops/emailops/security/advisories/new)
on GitHub — that's the only supported channel. It routes directly to the
maintainers and stays confidential until a fix is shipped.

Please include:

- A description of the issue and its impact.
- Steps to reproduce (ideally a minimal repro).
- Any logs, screenshots, or proof-of-concept code.
- The version of EmailOps you tested against (`emailops --version` or the
  release tag from `package.json` / `src-tauri/Cargo.toml`).

We aim to acknowledge reports within **3 business days** and to provide a
remediation plan or status update within **10 business days**. Critical
issues that affect users in the wild are prioritized over feature work.

## Scope

In scope:

- The EmailOps desktop application (Tauri + Rust + React/TypeScript).
- Local SQLite database handling, OAuth token storage, and the OS keychain
  integration.
- Email HTML rendering / sanitization pipeline (the sandboxed iframe and
  the DOMPurify policy in `src/lib/sanitizeEmailHtml.ts`).
- Code that ships in the released desktop application.

Out of scope:

- Vulnerabilities in third-party services we integrate with (Gmail,
  Microsoft Graph, OpenRouter, Ollama). Please report those directly to
  the upstream vendor.
- Issues that require an attacker to already have local access to the
  user's machine and unlocked keychain.
- Social engineering of EmailOps contributors.
- Development-only eval harnesses (binaries under `src-tauri/src/bin/`
  enabled via the `eval` feature flag) — these are not part of release
  builds and not part of the trust boundary.

## Security-Relevant Architecture

A few invariants are worth knowing before reporting:

- **OAuth tokens** live in the OS keychain (`keyring` crate) in release
  builds. Plaintext SQLite token storage is gated behind
  `cfg!(debug_assertions)` and only used during local development.
- **Email HTML** is rendered inside a `sandbox="allow-scripts allow-popups"`
  iframe with **no `allow-same-origin`**. The null origin means scripts
  inside the iframe cannot reach the parent DOM, cookies, or storage.
- **Remote content** in email HTML is gated per-sender. Images and CSS
  `url(...)` references are stripped until the user explicitly trusts the
  sender via the "Load images" banner.
- **CSP** is enabled in `src-tauri/tauri.conf.json` and restricts script
  / connect / frame sources to a known allowlist.

## Disclosure Policy

We follow coordinated disclosure. Once a fix is available and shipped, we
publish an advisory crediting the reporter (unless they prefer to remain
anonymous). Please give us a reasonable window to ship the fix before
publishing details.
