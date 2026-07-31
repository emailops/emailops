# EmailOps server port — feasibility spike

**Branch:** `worktree-server-port` · **Worktree:** `.claude/worktrees/server-port`
**Status:** in progress
**Goal:** get a multi-user EmailOps server running locally, testable in a browser, with AI features working — and prove the open-core boundary (public desktop repo + private server repo) is viable.

This is a **spike**, not the production implementation. It is deliberately structured so the
early commits are independently valuable to the desktop app even if the server idea is dropped.

## Read next

| Doc | What's in it |
|---|---|
| `01-DECISIONS.md` | Every decision taken during the spike, with the reasoning and what was rejected |
| `02-PROGRESS.md` | Running log: what landed in each commit, what broke, what was learned |
| `03-RUNNING-LOCALLY.md` | How to actually start the thing and poke it in a browser |
| `04-SALVAGE.md` | Which commits are worth merging back to `main` regardless of the server outcome |

## Scope of the spike

**In:**
- Tauri-free core (feature-gated, not yet a workspace split)
- Ambient per-user context so services resolve the right user's DB/sink/secrets
- A separate local server repo consuming core as a path dependency
- Postgres control plane: users, sessions, per-user SQLite provisioning
- HTTP RPC over a subset of the 203 commands — enough to drive the real UI
- SSE event fan-out per user
- Frontend transport switch (Tauri invoke ↔ HTTP) with the existing UI unchanged
- Docker Compose + a `dev-seed` that provisions two users from the demo-DB generator
- Chat / AI working end to end in the browser

**Out (deliberately):**
- Full cargo workspace split into `crates/*` — feature-gating Tauri proves the same
  boundary at a fraction of the churn. Documented as the production follow-up.
- All 203 commands. The registry covers the boot path plus mail + chat.
- OAuth (Gmail/Outlook consent) — `import-user` + demo data gets us to a testable
  two-user system without it.
- OIDC/SAML, admin UI, backups, HA, Postgres for mail data.

## The one-line summary of the approach

> The desktop app already has every seam this needs — `EventSink`, `Logger`, `Keychain`,
> `Clock`, `AIProvider`, `MailProvider`, and a fully headless CLI bootstrap. They are just
> wired to **process-global** singletons. Make those globals resolve through an ambient
> per-user context first, and the server becomes a thin HTTP shell over code that already works.
