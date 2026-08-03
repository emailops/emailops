# Roadmap

This is a living document describing where EmailOps is headed. Dates are
intentionally absent — milestones land when they're ready. If something
here matters to you, open an issue or a PR; community input shapes
priority.

> The current shipping version is **0.6.5** (see [CHANGELOG.md](CHANGELOG.md)).

## Next up (targeting 0.6.0)

- **Qwen 3.6 27B in the embedded catalog.** Add the unsloth `UD-Q4_K_XL`
  GGUF (~17.6 GB) as a chat option for Apple Silicon Macs with ≥24 GB RAM,
  once `llama-cpp-2 > 0.1.146` ships bundling llama.cpp b9180+ (the version
  that introduced Qwen 3.6's hybrid-attention support).
- **Add more tools to the chat to control application**
- **Allow to anonymize and copy a thread of emails**: this can be handy if users wants to share that with a remote LLM without exposing sensitive data.

## On the radar (0.7.0+)

- **Calendar integration** (Google Calendar + Microsoft Graph) so
  EmailOps can surface "this email implies an event."
- **Auto-send with approval workflows** for AI drafts (currently the
  user always reviews and clicks Send).
- **Per-account model preferences** — pick a different embedded model
  per account (e.g. heavier model for a work account, fast small model
  for personal).
- **Lens marketplace / sharing** — export a Lens definition (scope,
  rules, filters) and import it into another EmailOps install.
- **Better attachment classification** beyond filename heuristics.
- **Outlook / Microsoft 365 polish**. Sync parity with Gmail, calendar
  integration deferred to a later release.
- **Tool-calling reliability on small local models.** Models in the
  ~4–9B class (Qwen 3.5 9B, Qwen 3.5 4B and similar) often emit tool
  calls as plain text or stop at a "Drafting the reply…" narration
  instead of chaining `search_emails → generate_email_draft`. A
  registry-aware text salvager and a one-shot nudge land in 0.5.x as
  partial mitigations; longer term we want intent-based preseeded
  multi-call chains for draft / forward / reply requests, progressive
  nudging, and stricter JSON-schema requireds (e.g. `email_id` for
  reply drafts) so the model literally cannot omit the arg.
- **Email-sync integration testing.** The provider mock-server harness
  (Gmail / Outlook / IMAP cassettes) is in place; expand coverage to
  the failure modes that bite in practice: large first-sync (>10k
  messages), partial-batch failures + resume, history-id resets,
  attachment-only deltas, Outlook delta-token expiry, label / folder
  rename across runs, and rate-limit backoff. Goal: every regression
  we currently catch by running `make demo` against a real account
  should be reproducible in CI from a cassette.
- **Dependency hygiene.** `.github/dependabot.yml` is wired.
- **Audit existing `#[allow(clippy::unwrap_used \| expect_used)]` opt-outs**
  annually. The crate-wide deny lint is in place (see `src-tauri/src/lib.rs`),
  but justified panics drift over time and the comments next to each
  `#[allow]` should still hold.

## Explicitly out of scope (for now)

- **Cloud-hosted variant.** Always-local-first is a deliberate
  positioning choice.

## How to influence the roadmap

- **Bug reports** with clear repros get fixed quickly.
- **Feature requests** are most effective when they explain the
  underlying workflow, not just the asked-for feature.
- **Pull requests** for items in "Next up" or "On the radar" are very
  welcome — please open a tracking issue first so we can coordinate.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the dev workflow.
