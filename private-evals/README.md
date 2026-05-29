# Private Evals

This directory is the local-only workspace for benchmark suites that use real
mailbox data. Keep actual cases, configs, and reports out of Git.

## Safety Model

- Evals copy the production SQLite DB to a temp directory by default.
- Use `--in-place-dangerous` only for manual maintainer debugging.
- Keep OpenRouter judging disabled unless you are intentionally sending private
  snippets to a cloud model with `--use-judge`.
- Store reports under `reports/evaluations/` or another ignored directory.

## Typical Commands

```bash
# Chat eval against a temp copy of the production DB.
cargo run --manifest-path src-tauri/Cargo.toml --features eval --example chat_eval -- \
  --prod-db "$HOME/Library/Application Support/com.emailops.app/emailops.db" \
  --private

# Shortcut prompt variants against a temp copy of the production DB.
cargo run --manifest-path src-tauri/Cargo.toml --features eval --example chat_shortcut_eval -- \
  --prod-db "$HOME/Library/Application Support/com.emailops.app/emailops.db" \
  --private

# Agent-search eval against private cases.
cargo run --manifest-path src-tauri/Cargo.toml --features eval --example agent_search_eval -- \
  --prod-db "$HOME/Library/Application Support/com.emailops.app/emailops.db" \
  --private

# Chat eval with OpenRouter judge enabled.
cargo run --manifest-path src-tauri/Cargo.toml --features eval --example chat_eval -- \
  --prod-db "$HOME/Library/Application Support/com.emailops.app/emailops.db" \
  --private \
  --use-judge

# Lens extraction eval against a temp copy of the production DB.
cargo run --manifest-path src-tauri/Cargo.toml --features eval --example lens_extract_eval -- \
  --prod-db "$HOME/Library/Application Support/com.emailops.app/emailops.db" \
  --lens-id "<private-lens-id>" \
  --out reports/evaluations/private/lenses
```

## Local Files

Suggested ignored layout:

```text
private-evals/
  benchmark.toml
  chat/cases/*.yaml
  chat/shortcuts/*.yaml
  agent_search/*.yaml
  notes/*.md
```

Commit only examples that contain synthetic data.

Generated reports should not be committed. Private and public reports can include
subjects, senders, snippets, model traces, and judge output; keep them under the
already-ignored `reports/evaluations/` tree.
