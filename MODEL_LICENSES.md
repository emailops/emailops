# Model Licenses

This document tracks license terms and attribution for models referenced by EmailOps.

## Bundled in macOS release artifacts

### `nomic-embed-text-v1.5-q4_k_m.gguf`

- **Purpose:** default local embedding model used for semantic search.
- **Source artifact:** [nomic-ai/nomic-embed-text-v1.5-GGUF on Hugging Face](https://huggingface.co/nomic-ai/nomic-embed-text-v1.5-GGUF)
- **Configured URL in repo:** `scripts/fetch_bundled_models.sh`
- **Catalog entry:** `src-tauri/src/ai/model_catalog.rs`
- **License metadata in catalog:** `apache-2.0`

Upstream model/license pages should be reviewed for the latest terms before distributing modified binaries.

## Downloadable model catalog entries (not bundled by default)

Model entries exposed in-app include a `license` field in `src-tauri/src/ai/model_catalog.rs`.
Some entries (for example, `gemma`) may use licenses with additional use/re-distribution terms.
Contributors and distributors should review upstream model terms before shipping those models.
