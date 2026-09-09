// Not gated behind `llamacpp`: the offload decision is pure and is unit-tested
// in `--no-default-features` builds, which is the only configuration the CI
// fast jobs compile.
pub mod gpu_plan;
pub mod ollama;
pub mod openrouter;
pub mod provider;
pub mod stream_gate;
pub mod thinking_filter;
pub mod tracing;

#[cfg(feature = "llamacpp")]
pub mod llama_cpp;

pub mod model_catalog;
pub mod model_manager;

/// How much VRAM a discrete GPU can lend to model weights on this machine, if
/// any. Feeds `model_catalog::recommended_chat_model`.
///
/// `None` in three cases, all meaning "the GPU changes nothing here": this
/// build has no embedded runtime, the runtime cannot execute on this host (an
/// Intel Mac — see `gpu_plan::embedded_runtime_supported`, which also keeps us
/// from initialising a backend that would fail), or the machine has no discrete
/// card with room to spare.
pub fn discrete_vram_budget_bytes() -> Option<u64> {
    #[cfg(feature = "llamacpp")]
    {
        if gpu_plan::embedded_runtime_supported(std::env::consts::OS, std::env::consts::ARCH) {
            gpu_plan::discrete_vram_budget(&llama_cpp::runtime::devices_for_planning())
        } else {
            None
        }
    }
    #[cfg(not(feature = "llamacpp"))]
    None
}
