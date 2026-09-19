//! Deciding whether the loaded chat model needs the "thinking disabled" primer.
//!
//! Qwen 3 family models open every answer with a `<think>…</think>` span. On a
//! near-full context window the prompt-budget planner shrinks the generation
//! reserve to `GEN_RESERVE_TOKENS`; a reasoning model then spends that whole
//! reserve inside `<think>…`, `strip_reasoning` removes it, and the user gets
//! an empty reply. Priming an already-closed, empty think block makes Qwen read
//! the reasoning as done and emit the answer straight away.
//!
//! Which models need it used to be decided from the GGUF's *file name*
//! (`qwen3*`), which is wrong for every Qwen 3 build that isn't named that way —
//! a fine-tune, a re-quant, or a file the user renamed — and those answered "".
//! The family is recorded inside the file as `general.architecture`, so that is
//! what decides now; the file name survives only as a fallback for GGUFs whose
//! header we cannot read.
//!
//! Kept out of `llama_cpp` (and so out of the `llamacpp` feature gate) because
//! the decision is pure and must stay unit-tested in `--no-default-features`
//! builds, which is all the CI fast jobs compile.

use std::path::Path;

use crate::ai::gguf;

/// The empty closed `<think></think>` block that puts a Qwen 3 family model
/// into no-think mode.
pub const QWEN3_NO_THINK_PRIMER: &str = "<think>\n\n</think>\n\n";

/// llama.cpp's architecture ids for the Qwen 3 family all share this prefix
/// (`qwen3`, `qwen3moe`, `qwen3next`, `qwen3vl`, `qwen3vlmoe`, …). Qwen 2.x is
/// `qwen2*` and never had a thinking mode, so the prefix separates the two
/// cleanly — and a future `qwen3`-derived id is covered for free.
const QWEN3_ARCH_PREFIX: &str = "qwen3";

/// The priming block to append to the prompt tail for this model, or `""` when
/// the model needs none.
///
/// `architecture` is the GGUF's `general.architecture` when it could be read;
/// `file_name` is the GGUF's file name, consulted only when it could not.
/// Append the result AFTER the cached-prefix byte counts are computed: it lands
/// at the generation point, so it never shifts the prefix the KV cache anchors
/// on.
pub fn no_think_priming(architecture: Option<&str>, file_name: Option<&str>) -> &'static str {
    let needs_priming = match architecture {
        // The header is authoritative — a misleading name cannot override it.
        Some(arch) => is_qwen3_architecture(arch),
        None => looks_like_qwen3_file_name(file_name.unwrap_or_default()),
    };
    if needs_priming {
        QWEN3_NO_THINK_PRIMER
    } else {
        ""
    }
}

/// Resolve the priming block for a GGUF on disk, reading its architecture from
/// the header. Does one small header read, so callers cache the result rather
/// than calling this per inference.
pub fn no_think_priming_for_model(path: &Path) -> &'static str {
    let architecture = gguf::read_architecture(path);
    let file_name = path.file_name().and_then(|n| n.to_str());
    no_think_priming(architecture.as_deref(), file_name)
}

/// True for every `general.architecture` in the Qwen 3 family.
fn is_qwen3_architecture(architecture: &str) -> bool {
    architecture.trim().to_ascii_lowercase().starts_with(QWEN3_ARCH_PREFIX)
}

/// Last-resort guess for a GGUF whose header we could not read. Matches the
/// family anywhere in the name (`unsloth-qwen3-30b-a3b.gguf` is as much a
/// Qwen 3 as `qwen3-14b.gguf`), which is as far as a name can honestly take us.
fn looks_like_qwen3_file_name(file_name: &str) -> bool {
    file_name.to_ascii_lowercase().contains(QWEN3_ARCH_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen3_architecture_primes_regardless_of_file_name() {
        // The bug: a Qwen 3 GGUF whose file name doesn't start with "qwen3"
        // (a re-quant, a fine-tune, a user-renamed file) got no primer and
        // answered "" once the generation reserve was spent inside <think>.
        for arch in ["qwen3", "qwen3moe", "qwen3next", "qwen3vl"] {
            assert_eq!(
                no_think_priming(Some(arch), Some("my-assistant-4b-q4_k_m.gguf")),
                QWEN3_NO_THINK_PRIMER,
                "architecture {arch} should prime no-think"
            );
        }
    }

    #[test]
    fn architecture_wins_over_a_misleading_file_name() {
        // A non-Qwen3 model named qwen3-something must NOT get the primer:
        // the header is authoritative once we can read it.
        assert_eq!(no_think_priming(Some("gemma3"), Some("qwen3-lookalike.gguf")), "");
        assert_eq!(no_think_priming(Some("qwen2"), Some("qwen3-mislabelled.gguf")), "");
    }

    #[test]
    fn non_qwen3_architectures_prime_nothing() {
        // Older Qwen families never had a `<think>` mode; other thinking
        // families need a different priming shape (Gemma 4 uses `<|channel>`,
        // DeepSeek-R1 ships the closed-block hint in its own template).
        for arch in ["qwen2", "llama", "gemma3", "deepseek2", "phi3"] {
            assert_eq!(no_think_priming(Some(arch), Some("model.gguf")), "", "arch {arch}");
        }
    }

    #[test]
    fn architecture_matching_ignores_case_and_padding() {
        assert_eq!(
            no_think_priming(Some("Qwen3"), Some("model.gguf")),
            QWEN3_NO_THINK_PRIMER
        );
        assert_eq!(
            no_think_priming(Some(" qwen3 "), Some("model.gguf")),
            QWEN3_NO_THINK_PRIMER
        );
    }

    #[test]
    fn falls_back_to_the_file_name_when_the_header_is_unreadable() {
        // Metadata absent (unreadable/truncated GGUF) → the old heuristic is
        // all we have. Matched anywhere in the name, not just as a prefix.
        assert_eq!(
            no_think_priming(None, Some("qwen3.5-4b-q4_k_m.gguf")),
            QWEN3_NO_THINK_PRIMER
        );
        assert_eq!(
            no_think_priming(None, Some("Qwen3-14B-Instruct.gguf")),
            QWEN3_NO_THINK_PRIMER
        );
        assert_eq!(
            no_think_priming(None, Some("unsloth-qwen3-30b-a3b.gguf")),
            QWEN3_NO_THINK_PRIMER
        );
        assert_eq!(no_think_priming(None, Some("qwen2.5-7b-instruct.gguf")), "");
        assert_eq!(no_think_priming(None, Some("gemma-4-12b-it-q4_k_xl.gguf")), "");
    }

    #[test]
    fn nothing_known_primes_nothing() {
        assert_eq!(no_think_priming(None, None), "");
        assert_eq!(no_think_priming(Some(""), Some("")), "");
    }

    #[test]
    fn reads_the_architecture_from_a_gguf_on_disk() {
        // End-to-end over the real header reader: the name is not Qwen-ish at
        // all, the header says qwen3, the primer is emitted.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("assistant-4b-q4_k_m.gguf");
        std::fs::write(&path, gguf_fixture("qwen3")).unwrap();
        assert_eq!(no_think_priming_for_model(&path), QWEN3_NO_THINK_PRIMER);

        let gemma = tmp.path().join("qwen3-named-but-gemma.gguf");
        std::fs::write(&gemma, gguf_fixture("gemma3")).unwrap();
        assert_eq!(no_think_priming_for_model(&gemma), "");

        // Not a GGUF at all → file-name fallback.
        let garbage = tmp.path().join("qwen3-corrupt.gguf");
        std::fs::write(&garbage, b"not a gguf").unwrap();
        assert_eq!(no_think_priming_for_model(&garbage), QWEN3_NO_THINK_PRIMER);
    }

    /// Smallest valid GGUF v3 header carrying only `general.architecture`.
    fn gguf_fixture(architecture: &str) -> Vec<u8> {
        let mut out = b"GGUF".to_vec();
        out.extend_from_slice(&3u32.to_le_bytes()); // version
        out.extend_from_slice(&0u64.to_le_bytes()); // tensor count
        out.extend_from_slice(&1u64.to_le_bytes()); // kv count
        let key = gguf::KEY_ARCHITECTURE;
        out.extend_from_slice(&(key.len() as u64).to_le_bytes());
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(&8u32.to_le_bytes()); // value type: string
        out.extend_from_slice(&(architecture.len() as u64).to_le_bytes());
        out.extend_from_slice(architecture.as_bytes());
        out
    }
}
