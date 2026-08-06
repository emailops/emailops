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
pub mod model_fit;
pub mod model_manager;
