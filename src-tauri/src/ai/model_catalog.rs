// Curated catalog of GGUF models available for download.
//
// Each entry has a pinned HuggingFace URL + SHA256 so downloads are
// verifiable and reproducible. The catalog is embedded at compile time;
// updates ship with app updates.
//
// min_ram_gb reflects peak RAM during inference (weights + KV cache +
// activations). Used by the UI to grey-out models that won't fit.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    /// Text generation / chat.
    Chat,
    /// Sentence embedding (encoder-only or encoder-decoder).
    Embedding,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    /// Unique stable identifier (used as filename prefix).
    pub id: &'static str,
    pub display_name: &'static str,
    pub kind: ModelKind,
    /// Compressed on-disk size in bytes.
    pub size_bytes: u64,
    /// Context window in tokens.
    pub context_window: u32,
    /// Lowercase hex SHA-256 of the GGUF file.
    pub sha256: &'static str,
    /// Direct HTTPS download URL (HuggingFace resolve/main).
    pub url: &'static str,
    pub license: &'static str,
    /// Minimum system RAM (GiB) required to run comfortably.
    pub min_ram_gb: u8,
    /// Shown first in the picker.
    pub recommended: bool,
    /// Model supports structured tool-calling via its chat template.
    pub supports_tools: bool,
    /// File ships inside the .app bundle and is copied into the user's
    /// models dir on first launch. The download UI hides "Download" for
    /// these entries — they appear pre-installed.
    pub bundled: bool,
}

/// How much headroom over a model's stated minimum a machine needs before that
/// model is *recommended* rather than merely offered.
///
/// `min_ram_gb` is peak inference RAM — weights, KV cache and activations — so
/// a machine that only just meets it has nothing left for the OS, the mail
/// database, the webview, or whatever else the user has open. Recommending at
/// twice the minimum keeps the suggested model comfortable on a machine that is
/// also being used as a computer. Models below the bar are still listed and
/// still downloadable; this only moves the badge.
const RECOMMENDATION_RAM_HEADROOM: u64 = 2;

/// Pick the chat model to badge as "Recommended" for this machine.
///
/// The largest model the hardware can carry wins, because within that limit
/// bigger is better at every job the app uses the chat model for. Ties are
/// broken by catalog order, which is curated — without that the pick would
/// depend on nothing more meaningful than where an entry happens to sit in the
/// array.
///
/// Two independent signals qualify a model, and either is enough:
///
/// - **System RAM** — the model's stated minimum with
///   [`RECOMMENDATION_RAM_HEADROOM`] to spare.
/// - **A discrete GPU** — `vram_budget_bytes` (from
///   `gpu_plan::discrete_vram_budget`) large enough to hold the weights. The
///   weights then live on the card rather than in system RAM, so a 16 GB
///   machine with a 24 GB card is not a 16 GB machine for this decision.
///   `None` means no usable discrete card; unified memory (Apple Silicon,
///   iGPUs) arrives as `None` because it *is* the system RAM already counted,
///   and counting it twice would spend one pool as if it were two.
///
/// Two deliberate fallbacks:
///
/// - Both signals absent — `total_ram_gb == 0` means the probe failed (see
///   `util::system`), not that the machine has no memory. Guessing "smallest"
///   from a failed probe would be a silent downgrade, so the catalog's own
///   default stands.
/// - A machine too small for anything still gets the smallest chat model.
///   Whether local AI is viable at all is `ai_capability_from`'s decision;
///   returning nothing here would just leave the picker with no default
///   selected and the onboarding Continue button dead.
pub fn recommended_chat_model(
    models: &[CatalogModel],
    total_ram_gb: u64,
    vram_budget_bytes: Option<u64>,
) -> Option<&CatalogModel> {
    let chat = || models.iter().filter(|m| matches!(m.kind, ModelKind::Chat));

    // A model qualifies on either signal. They are alternatives, not a
    // minimum: a discrete card holds the weights itself, so it lifts a machine
    // above what its RAM alone would allow — while a small card must never drag
    // a roomy machine's pick back down.
    let fits_in_ram = |m: &CatalogModel| {
        total_ram_gb > 0 && (m.min_ram_gb as u64).saturating_mul(RECOMMENDATION_RAM_HEADROOM) <= total_ram_gb
    };
    let fits_on_gpu = |m: &CatalogModel| vram_budget_bytes.is_some_and(|budget| m.size_bytes <= budget);

    if total_ram_gb == 0 && vram_budget_bytes.is_none() {
        return chat().find(|m| m.recommended).or_else(|| smallest_chat_model(models));
    }

    chat()
        .filter(|m| fits_in_ram(m) || fits_on_gpu(m))
        // `max_by_key` keeps the *last* maximum; the catalog is ordered, so the
        // first entry at a given size is the curated pick.
        .fold(None, |best: Option<&CatalogModel>, candidate| match best {
            Some(current) if current.min_ram_gb >= candidate.min_ram_gb => Some(current),
            _ => Some(candidate),
        })
        .or_else(|| smallest_chat_model(models))
}

fn smallest_chat_model(models: &[CatalogModel]) -> Option<&CatalogModel> {
    models
        .iter()
        .filter(|m| matches!(m.kind, ModelKind::Chat))
        .min_by_key(|m| m.min_ram_gb)
}

/// All models the app knows how to download. Extend this list with new app
/// releases. Prefer keeping entries — existing installs may reference them by
/// id — and only retire a model when it's genuinely unfit for the app, e.g. it
/// can't do tool-calling (which chat features rely on). A retired model's local
/// file still loads via the runtime; it's just no longer offered for download.
pub static CATALOG: &[CatalogModel] = &[
    // ── Chat models ─────────────────────────────────────────────────────────
    CatalogModel {
        id: "qwen3.5-4b-q4_k_m",
        display_name: "Qwen 3.5 4B",
        kind: ModelKind::Chat,
        // bartowski/Qwen_Qwen3.5-4B-GGUF/main · Qwen_Qwen3.5-4B-Q4_K_M.gguf
        // Upstream Content-Length and x-linked-etag (LFS sha256).
        size_bytes: 3_013_027_808,
        context_window: 262_144,
        sha256: "13c16f426047e2de38cd075bdade4a7bcbc8c774384876f677740cda65f8a983",
        url: "https://huggingface.co/bartowski/Qwen_Qwen3.5-4B-GGUF/resolve/main/Qwen_Qwen3.5-4B-Q4_K_M.gguf",
        license: "apache-2.0",
        min_ram_gb: 8,
        recommended: true,
        supports_tools: true,
        bundled: false,
    },
    CatalogModel {
        id: "qwen3.5-4b-q8_0",
        display_name: "Qwen 3.5 4B Q8",
        kind: ModelKind::Chat,
        // bartowski/Qwen_Qwen3.5-4B-GGUF/main · Qwen_Qwen3.5-4B-Q8_0.gguf
        // Upstream Content-Length (x-linked-size) and x-linked-etag (LFS sha256).
        // Higher-fidelity sibling of qwen3.5-4b-q4_k_m: ~4.6 GB of weights vs 3.0,
        // so peak RAM (weights + f16 KV + activations) lands ~12 GB.
        size_bytes: 4_622_131_168,
        context_window: 262_144,
        sha256: "5c74c0ede371924357dff0cb6ba145bd67208b9b2389ded681adfff3f7608db7",
        url: "https://huggingface.co/bartowski/Qwen_Qwen3.5-4B-GGUF/resolve/main/Qwen_Qwen3.5-4B-Q8_0.gguf",
        license: "apache-2.0",
        min_ram_gb: 12,
        recommended: false,
        supports_tools: true,
        bundled: false,
    },
    CatalogModel {
        id: "qwen3.5-9b-q4_k_m",
        display_name: "Qwen 3.5 9B",
        kind: ModelKind::Chat,
        // unsloth/Qwen3.5-9B-GGUF/main · Qwen3.5-9B-Q4_K_M.gguf
        // Upstream Content-Length (x-linked-size) and x-linked-etag (LFS sha256).
        size_bytes: 5_680_522_464,
        context_window: 262_144,
        sha256: "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8",
        url: "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_K_M.gguf",
        license: "apache-2.0",
        min_ram_gb: 16,
        recommended: false,
        supports_tools: true,
        bundled: false,
    },
    CatalogModel {
        id: "qwen3.5-27b-ud-q4_k_xl",
        display_name: "Qwen 3.5 27B",
        kind: ModelKind::Chat,
        // unsloth/Qwen3.5-27B-GGUF/main · Qwen3.5-27B-UD-Q4_K_XL.gguf
        // Upstream Content-Length (x-linked-size) and x-linked-etag (LFS sha256).
        size_bytes: 17_621_125_024,
        context_window: 262_144,
        sha256: "13cb6228344898afa50d963c02ae0d991ae25094eea8837db8d0e452e91c5888",
        url: "https://huggingface.co/unsloth/Qwen3.5-27B-GGUF/resolve/main/Qwen3.5-27B-UD-Q4_K_XL.gguf",
        license: "apache-2.0",
        min_ram_gb: 24,
        recommended: false,
        supports_tools: true,
        bundled: false,
    },
    CatalogModel {
        id: "gemma-4-12b-it-qat-ud-q4_k_xl",
        display_name: "Gemma 4 12B Instruct",
        kind: ModelKind::Chat,
        // unsloth/gemma-4-12B-it-qat-GGUF/main · gemma-4-12B-it-qat-UD-Q4_K_XL.gguf
        // Upstream Content-Length (x-linked-size) and x-linked-etag (LFS sha256).
        size_bytes: 6_716_355_328,
        context_window: 262_144,
        sha256: "cc9ff072e0a8203429ed854e6662c17a6c2bc1e5dca5b475dd4736caaacbc165",
        url: "https://huggingface.co/unsloth/gemma-4-12B-it-qat-GGUF/resolve/main/gemma-4-12B-it-qat-UD-Q4_K_XL.gguf",
        license: "gemma",
        min_ram_gb: 16,
        recommended: false,
        supports_tools: true,
        bundled: false,
    },
    CatalogModel {
        id: "qwen3.6-35b-a3b-ud-q4_k_xl",
        display_name: "Qwen 3.6 35B A3B",
        kind: ModelKind::Chat,
        // unsloth/Qwen3.6-35B-A3B-GGUF/main · Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf
        // Authoritative size + sha256 from the HuggingFace tree API (file `size`
        // and `lfs.oid`); single file, not split.
        // MoE: 35B total params, ~3B active per token — dense-35B memory footprint
        // (all experts resident) but ~3B-class decode speed.
        size_bytes: 22_360_456_160,
        context_window: 262_144,
        sha256: "707a55a8a4397ecde44de0c499d3e68c1ad1d240d1da65826b4949d1043f4450",
        url: "https://huggingface.co/unsloth/Qwen3.6-35B-A3B-GGUF/resolve/main/Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf",
        license: "apache-2.0",
        min_ram_gb: 32,
        recommended: false,
        supports_tools: true,
        bundled: false,
    },
    // ── Embedding models ─────────────────────────────────────────────────────
    CatalogModel {
        // Ships inside the .app via tauri.conf.json `bundle.resources` and is
        // copied into <app_data_dir>/models/embed/ on first launch by
        // model_manager::seed_bundled_model. scripts/fetch_bundled_models.sh
        // downloads the file into src-tauri/resources/ before bundling so the
        // .gguf isn't committed; its identity is pinned by the SHA-256 below
        // and enforced by the script.
        id: "nomic-embed-text-v1.5-q4_k_m",
        display_name: "Nomic Embed Text v1.5",
        kind: ModelKind::Embedding,
        size_bytes: 84_106_624,
        context_window: 8192,
        sha256: "d4e388894e09cf3816e8b0896d81d265b55e7a9fff9ab03fe8bf4ef5e11295ac",
        url:
            "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5-GGUF/resolve/main/nomic-embed-text-v1.5.Q4_K_M.gguf",
        license: "apache-2.0",
        min_ram_gb: 1,
        recommended: true,
        supports_tools: false,
        bundled: true,
    },
];

/// Look up a catalog entry by its ID.
pub fn find(id: &str) -> Option<&'static CatalogModel> {
    CATALOG.iter().find(|m| m.id == id)
}

/// All chat models.
pub fn chat_models() -> impl Iterator<Item = &'static CatalogModel> {
    CATALOG.iter().filter(|m| m.kind == ModelKind::Chat)
}

/// All embedding models.
pub fn embedding_models() -> impl Iterator<Item = &'static CatalogModel> {
    CATALOG.iter().filter(|m| m.kind == ModelKind::Embedding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen3_5_4b_q8_entry_is_present_and_correct() {
        let entry = find("qwen3.5-4b-q8_0").expect("Qwen 3.5 4B Q8_0 must be in the catalog");
        assert_eq!(entry.kind, ModelKind::Chat);
        assert_eq!(
            entry.url,
            "https://huggingface.co/bartowski/Qwen_Qwen3.5-4B-GGUF/resolve/main/Qwen_Qwen3.5-4B-Q8_0.gguf"
        );
        // Authoritative values from the HuggingFace `resolve` endpoint headers:
        // x-linked-size (Content-Length) and x-linked-etag (LFS sha256).
        assert_eq!(entry.size_bytes, 4_622_131_168);
        assert_eq!(
            entry.sha256,
            "5c74c0ede371924357dff0cb6ba145bd67208b9b2389ded681adfff3f7608db7"
        );
        assert_eq!(entry.context_window, 262_144);
        assert_eq!(entry.license, "apache-2.0");
        assert!(entry.supports_tools, "Qwen 3.5 4B Q8 must support tool-calling");
        // Q8 weights (~4.6 GB) + f16 KV + activations → ~12 GB peak, vs 8 for Q4.
        assert_eq!(entry.min_ram_gb, 12);
        assert!(
            !entry.recommended,
            "Q4 stays the recommended default; Q8 is the opt-in higher-fidelity sibling"
        );
    }

    #[test]
    fn qwen3_5_9b_q4_entry_is_present_and_correct() {
        let entry = find("qwen3.5-9b-q4_k_m").expect("Qwen 3.5 9B Q4_K_M must be in the catalog");
        assert_eq!(entry.kind, ModelKind::Chat);
        assert_eq!(
            entry.url,
            "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_K_M.gguf"
        );
        // Authoritative values from the HuggingFace `resolve` endpoint headers:
        // x-linked-size (Content-Length) and x-linked-etag (LFS sha256).
        assert_eq!(entry.size_bytes, 5_680_522_464);
        assert_eq!(
            entry.sha256,
            "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8"
        );
        assert_eq!(entry.context_window, 262_144);
        assert_eq!(entry.license, "apache-2.0");
    }

    #[test]
    fn qwen3_5_27b_ud_q4_entry_is_present_and_correct() {
        let entry = find("qwen3.5-27b-ud-q4_k_xl").expect("Qwen 3.5 27B UD-Q4_K_XL must be in the catalog");
        assert_eq!(entry.kind, ModelKind::Chat);
        assert_eq!(
            entry.url,
            "https://huggingface.co/unsloth/Qwen3.5-27B-GGUF/resolve/main/Qwen3.5-27B-UD-Q4_K_XL.gguf"
        );
        // Authoritative values from the HuggingFace `resolve` endpoint headers:
        // x-linked-size (Content-Length) and x-linked-etag (LFS sha256).
        assert_eq!(entry.size_bytes, 17_621_125_024);
        assert_eq!(
            entry.sha256,
            "13cb6228344898afa50d963c02ae0d991ae25094eea8837db8d0e452e91c5888"
        );
        assert_eq!(entry.context_window, 262_144);
        assert_eq!(entry.license, "apache-2.0");
    }

    #[test]
    fn gemma4_12b_entry_is_present_and_correct() {
        let entry = find("gemma-4-12b-it-qat-ud-q4_k_xl").expect("Gemma 4 12B QAT must be in the catalog");
        assert_eq!(entry.kind, ModelKind::Chat);
        assert_eq!(
            entry.url,
            "https://huggingface.co/unsloth/gemma-4-12B-it-qat-GGUF/resolve/main/gemma-4-12B-it-qat-UD-Q4_K_XL.gguf"
        );
        // Authoritative values from the HuggingFace `resolve` endpoint headers:
        // x-linked-size (Content-Length) and x-linked-etag (LFS sha256).
        assert_eq!(entry.size_bytes, 6_716_355_328);
        assert_eq!(
            entry.sha256,
            "cc9ff072e0a8203429ed854e6662c17a6c2bc1e5dca5b475dd4736caaacbc165"
        );
        assert!(entry.supports_tools, "Gemma 4 12B must support tool-calling");
        assert_eq!(entry.license, "gemma");
        assert_eq!(entry.min_ram_gb, 16, "Gemma 4 12B QAT runs in 16 GB+");
    }

    #[test]
    fn qwen3_6_35b_a3b_entry_is_present_and_correct() {
        let entry = find("qwen3.6-35b-a3b-ud-q4_k_xl").expect("Qwen 3.6 35B A3B UD-Q4_K_XL must be in the catalog");
        assert_eq!(entry.kind, ModelKind::Chat);
        assert_eq!(
            entry.url,
            "https://huggingface.co/unsloth/Qwen3.6-35B-A3B-GGUF/resolve/main/Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf"
        );
        // Authoritative values from the HuggingFace tree API: file `size` and
        // `lfs.oid` (sha256) for Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf (single file, ~22.4 GB).
        assert_eq!(entry.size_bytes, 22_360_456_160);
        assert_eq!(
            entry.sha256,
            "707a55a8a4397ecde44de0c499d3e68c1ad1d240d1da65826b4949d1043f4450"
        );
        assert_eq!(entry.context_window, 262_144);
        assert_eq!(entry.license, "apache-2.0");
        assert!(entry.supports_tools, "Qwen 3.6 35B A3B must support tool-calling");
        // 35B-A3B MoE: all experts stay resident (~22.4 GB of weights) even though
        // only ~3B activate per token, so peak RAM (weights + KV + activations)
        // needs 32 GB+ — matches upstream's "32 GB+" Mac guidance.
        assert_eq!(entry.min_ram_gb, 32);
        assert!(
            !entry.recommended,
            "the 4B Q4 stays the recommended default; the 35B A3B is an opt-in heavy model"
        );
    }

    #[test]
    fn small_gemma4_variants_remain_absent() {
        // The E2B/E4B variants stay out — they're the ones with unreliable
        // tool-calling. The 12B (above) benchmarks strongly and is offered.
        assert!(find("gemma-4-e2b-it-q4_k_m").is_none(), "Gemma 4 E2B must be absent");
        assert!(
            find("gemma-4-e4b-obliterated-q4_k_m").is_none(),
            "Gemma 4 E4B must be absent"
        );
    }

    #[test]
    fn gemma3_12b_is_replaced_by_gemma4() {
        // Gemma 3 12B was swapped out for Gemma 4 12B in the picker.
        assert!(find("gemma-3-12b-it-q4_k_m").is_none(), "Gemma 3 12B must be removed");
    }

    #[test]
    fn display_names_omit_quant_and_size_suffix() {
        // Keep onboarding display names free of the "(Q4_K_M · ~2.9 GB)" tail —
        // the model row already renders size/RAM/license in its meta line, so
        // duplicating them in the title is what made onboarding feel technical.
        for m in CATALOG {
            assert!(
                !m.display_name.contains('('),
                "{}: display_name should not contain quant/size suffix, got: {}",
                m.id,
                m.display_name
            );
        }
    }

    #[test]
    fn nomic_embedding_is_bundled_with_real_sha_and_size() {
        // Nomic ships inside the .app via `tauri.conf.json` and
        // `scripts/fetch_bundled_models.sh`. If these values drift, the
        // copy-on-first-run path (model_manager::seed_bundled_model) and the
        // installer-side SHA gate (script) get out of sync — the latter would
        // fail noisily but the former would silently ship a mismatched binary
        // alongside a catalog that claims a different identity.
        let entry = find("nomic-embed-text-v1.5-q4_k_m").expect("Nomic embedding must remain in the catalog");
        assert!(
            entry.bundled,
            "Nomic must stay flagged as bundled — it ships in the .app"
        );
        assert_eq!(entry.kind, ModelKind::Embedding);
        assert_eq!(
            entry.size_bytes, 84_106_624,
            "size_bytes must match the bundled .gguf exactly"
        );
        assert_eq!(
            entry.sha256, "d4e388894e09cf3816e8b0896d81d265b55e7a9fff9ab03fe8bf4ef5e11295ac",
            "sha256 must match scripts/fetch_bundled_models.sh"
        );
    }

    #[test]
    fn all_chat_models_support_tools() {
        for m in chat_models() {
            assert!(
                m.supports_tools,
                "chat model {} must support tool-calling to stay in the catalog",
                m.id
            );
        }
    }

    #[test]
    fn all_catalog_ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for m in CATALOG {
            assert!(seen.insert(m.id), "duplicate catalog id: {}", m.id);
        }
    }

    #[test]
    fn all_urls_are_https_gguf() {
        for m in CATALOG {
            assert!(m.url.starts_with("https://"), "{} url must be https: {}", m.id, m.url);
            assert!(
                m.url.ends_with(".gguf"),
                "{} url must point at a .gguf file: {}",
                m.id,
                m.url
            );
        }
    }

    #[test]
    fn sha256_is_64_lowercase_hex_when_present() {
        for m in CATALOG {
            if m.sha256.is_empty() {
                continue;
            }
            assert_eq!(m.sha256.len(), 64, "{} sha256 must be 64 hex chars", m.id);
            assert!(
                m.sha256.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "{} sha256 must be lowercase hex: {}",
                m.id,
                m.sha256
            );
        }
    }
}

#[cfg(test)]
mod recommendation_tests {
    use super::*;

    fn recommended_id(total_ram_gb: u64) -> Option<&'static str> {
        recommended_chat_model(CATALOG, total_ram_gb, None).map(|m| m.id)
    }

    fn recommended_id_with(total_ram_gb: u64, vram_budget: Option<u64>) -> Option<&'static str> {
        recommended_chat_model(CATALOG, total_ram_gb, vram_budget).map(|m| m.id)
    }

    #[test]
    fn a_roomier_machine_is_recommended_a_larger_model() {
        // The point of the change: the badge tracks the hardware instead of
        // being pinned to the smallest model for everyone.
        assert_eq!(recommended_id(16), Some("qwen3.5-4b-q4_k_m"));
        assert_eq!(recommended_id(32), Some("qwen3.5-9b-q4_k_m"));
        assert_eq!(recommended_id(64), Some("qwen3.6-35b-a3b-ud-q4_k_xl"));
    }

    #[test]
    fn a_recommendation_leaves_the_machine_room_to_be_a_computer() {
        // `min_ram_gb` is peak inference RAM. Recommending a model that only
        // just fits would have the mail DB, the webview and everything else
        // the user is running fighting it for pages, so a recommendation needs
        // twice its minimum. A 24 GB machine can *run* the 9B (16 GB) — it is
        // offered, just not recommended.
        assert_eq!(recommended_id(24), Some("qwen3.5-4b-q8_0"));
    }

    #[test]
    fn a_machine_too_small_for_any_model_still_gets_the_smallest_one() {
        // Whether local AI is viable at all is `ai_capability_from`'s call, not
        // this one's. Returning nothing here would leave the picker with no
        // default selected and the Continue button dead.
        assert_eq!(recommended_id(8), Some("qwen3.5-4b-q4_k_m"));
        assert_eq!(recommended_id(4), Some("qwen3.5-4b-q4_k_m"));
    }

    #[test]
    fn a_failed_ram_probe_falls_back_to_the_catalog_default() {
        // `total_ram_bytes` reports 0 when the probe fails. Treating that as a
        // 0 GB machine would be a silent downgrade dressed up as a decision.
        let expected = CATALOG
            .iter()
            .find(|m| matches!(m.kind, ModelKind::Chat) && m.recommended)
            .map(|m| m.id);
        assert_eq!(recommended_id(0), expected);
    }

    #[test]
    fn embedding_models_are_never_recommended_as_the_chat_model() {
        // The embedding model is bundled and picked separately; suggesting it
        // for chat would hand the user a model that cannot hold a conversation.
        for ram in [0, 4, 8, 16, 32, 64, 512] {
            let picked = recommended_chat_model(CATALOG, ram, None).expect("a chat model is always suggested");
            assert!(
                matches!(picked.kind, ModelKind::Chat),
                "{ram} GB machine was recommended a {:?} model",
                picked.kind
            );
        }
    }

    #[test]
    fn a_tie_on_ram_is_broken_by_catalog_order() {
        // Qwen 9B and Gemma 12B both declare 16 GB. The catalog is curated, so
        // its order is the tiebreak — otherwise the pick would silently depend
        // on struct field ordering.
        assert_eq!(recommended_id(32), Some("qwen3.5-9b-q4_k_m"));
    }

    #[test]
    fn the_recommendation_never_exceeds_what_the_machine_can_hold() {
        // Property: whatever is recommended must itself be runnable.
        for ram in [16, 24, 32, 48, 64, 128] {
            let picked = recommended_chat_model(CATALOG, ram, None).expect("a chat model");
            assert!(
                picked.min_ram_gb as u64 <= ram,
                "{ram} GB machine recommended {} which needs {} GB",
                picked.id,
                picked.min_ram_gb
            );
        }
    }

    const GB: u64 = 1_000_000_000;

    #[test]
    fn a_roomy_discrete_card_lifts_a_machine_above_its_ram() {
        // The case the RAM-only signal got wrong: 16 GB of system RAM with a
        // 24 GB card was recommended the 4B model, even though the card holds
        // the 35B's weights comfortably.
        assert_eq!(
            recommended_id_with(16, Some(24 * GB)),
            Some("qwen3.6-35b-a3b-ud-q4_k_xl")
        );
    }

    #[test]
    fn the_card_must_actually_hold_the_weights() {
        // A 6 GB budget fits the 9B's 5.7 GB of weights but not the 27B's 17.6,
        // so the recommendation stops where the card does.
        assert_eq!(recommended_id_with(8, Some(6 * GB)), Some("qwen3.5-9b-q4_k_m"));
    }

    #[test]
    fn the_gpu_signal_never_lowers_the_ram_based_pick() {
        // A big machine with a small card must keep the model its RAM affords;
        // the two signals are alternatives, not a minimum.
        assert_eq!(
            recommended_id_with(64, Some(4 * GB)),
            Some("qwen3.6-35b-a3b-ud-q4_k_xl")
        );
        assert_eq!(recommended_id_with(64, None), Some("qwen3.6-35b-a3b-ud-q4_k_xl"));
    }

    #[test]
    fn no_discrete_card_leaves_the_ram_signal_alone() {
        // Apple Silicon and iGPU machines arrive here as `None` — their memory
        // is the system RAM already counted.
        assert_eq!(recommended_id_with(32, None), recommended_id_with(32, Some(0)));
    }

    #[test]
    fn a_failed_ram_probe_still_uses_the_card() {
        // RAM unknown is not "no memory". With a card big enough to hold the
        // weights the GPU signal alone is a sound basis for the pick.
        assert_eq!(
            recommended_id_with(0, Some(24 * GB)),
            Some("qwen3.6-35b-a3b-ud-q4_k_xl")
        );
    }
}
