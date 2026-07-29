//! How many model layers to offload to the GPU.
//!
//! The runtime used to pass `n_gpu_layers = u32::MAX` unconditionally — "put
//! everything on the GPU". That is right on Apple Silicon, where the GPU shares
//! the machine's RAM, but wrong on a discrete card: a 6 GB GPU asked to hold a
//! 20 GB model does not fall back gracefully, it fails the load. System RAM
//! says nothing about whether that will happen; VRAM is the binding constraint.
//!
//! This module is deliberately free of any llama.cpp types so it compiles and
//! is tested even in `--no-default-features` builds, where the `llamacpp`
//! feature (and therefore any real GPU) is absent. The runtime converts ggml's
//! device list into [`GpuDevice`] and asks [`plan_offload`] what to do.

/// What kind of memory a backend device has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    /// Discrete card with its own VRAM — the case that needs a real budget.
    Discrete,
    /// Integrated / unified memory (Apple Silicon Metal, iGPUs). The GPU shares
    /// system RAM, so there is no separate pool to run out of.
    Unified,
    /// The CPU backend. Always present; never a candidate for offload.
    Cpu,
    /// Anything ggml reports that does not map to the above.
    Other,
}

/// ggml's own device classification, mirrored here so the mapping below can be
/// tested without the `llamacpp` feature (and therefore without a GPU).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawDeviceType {
    Cpu,
    Gpu,
    IntegratedGpu,
    Accelerator,
    Unknown,
}

/// Decide whether a device's memory is its own or shared with the host.
///
/// ggml reports Apple Silicon's Metal device as a plain `Gpu`, but its memory
/// is the machine's RAM. Treating it as discrete would apply a VRAM budget to a
/// pool that has none and needlessly cap offload on exactly the platform where
/// full offload is both correct and fastest — so the backend name is consulted,
/// not just the type.
pub fn classify_device(backend: &str, raw: RawDeviceType) -> DeviceKind {
    if backend.eq_ignore_ascii_case("metal") {
        return DeviceKind::Unified;
    }
    match raw {
        RawDeviceType::Cpu => DeviceKind::Cpu,
        // An iGPU carves its memory out of system RAM, so it shares the host's
        // budget in the same way Metal does.
        RawDeviceType::IntegratedGpu => DeviceKind::Unified,
        RawDeviceType::Gpu => DeviceKind::Discrete,
        RawDeviceType::Accelerator | RawDeviceType::Unknown => DeviceKind::Other,
    }
}

/// A backend device, reduced to the fields the decision needs.
#[derive(Debug, Clone)]
pub struct GpuDevice {
    pub name: String,
    pub backend: String,
    pub kind: DeviceKind,
    /// Free device memory in bytes, as reported by ggml.
    pub memory_free: u64,
    /// Total device memory in bytes.
    pub memory_total: u64,
}

/// The chosen offload, plus a human-readable reason for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffloadPlan {
    /// Value to pass to `LlamaModelParams::with_n_gpu_layers`.
    pub n_gpu_layers: u32,
    /// One line explaining the choice, emitted to the output panel so a user
    /// who expected GPU acceleration can see why they did not get it.
    pub reason: String,
}

/// "Offload everything" sentinel. llama.cpp clamps this to the model's real
/// layer count.
pub const ALL_LAYERS: u32 = u32::MAX;

/// Fraction of *free* VRAM we are willing to fill with model weights.
///
/// The remainder absorbs the KV cache, compute buffers, and whatever the
/// desktop compositor and other applications take while EmailOps runs. Sizing
/// weights to 100% of free VRAM reliably fails once inference actually starts.
const VRAM_USABLE_FRACTION: f64 = 0.80;

/// Absolute floor held back on top of [`VRAM_USABLE_FRACTION`], for small cards
/// where 20% is not enough to cover the fixed compute-buffer cost.
const VRAM_RESERVE_BYTES: u64 = 512 * 1024 * 1024;

/// Bytes of VRAM available for model weights on `device`.
///
/// Saturating throughout: a tiny or busy card must yield 0, never wrap.
fn usable_vram(device: &GpuDevice) -> u64 {
    let budget = (device.memory_free as f64 * VRAM_USABLE_FRACTION) as u64;
    budget.saturating_sub(VRAM_RESERVE_BYTES)
}

/// Pick the device to offload to: the one with the most free memory.
///
/// Multi-GPU splitting is not attempted — llama.cpp can do it, but choosing a
/// split needs per-device topology this app does not model. One card is the
/// honest option.
fn best_device(devices: &[GpuDevice]) -> Option<&GpuDevice> {
    devices
        .iter()
        .filter(|d| matches!(d.kind, DeviceKind::Discrete | DeviceKind::Unified))
        .max_by_key(|d| d.memory_free)
}

/// Decide how many layers to offload.
///
/// `model_bytes` is the on-disk size of the GGUF, which approximates the weight
/// memory closely enough for a budget decision. `n_layers` is the model's block
/// count when known; when it is `None` the choice degrades to all-or-nothing,
/// because splitting layers requires knowing how many there are.
pub fn plan_offload(devices: &[GpuDevice], model_bytes: u64, n_layers: Option<u32>) -> OffloadPlan {
    let Some(device) = best_device(devices) else {
        return OffloadPlan {
            n_gpu_layers: 0,
            reason: "no GPU backend available — running on CPU".to_string(),
        };
    };

    // Unified memory: the GPU draws on the same pool the process already had to
    // fit in, so there is no second budget to blow. This is the Apple Silicon
    // path and it must keep offloading everything.
    if device.kind == DeviceKind::Unified {
        return OffloadPlan {
            n_gpu_layers: ALL_LAYERS,
            reason: format!(
                "{} ({}) has unified memory — offloading all layers",
                device.name, device.backend
            ),
        };
    }

    let usable = usable_vram(device);
    let free_gb = device.memory_free as f64 / 1e9;

    if usable >= model_bytes {
        return OffloadPlan {
            n_gpu_layers: ALL_LAYERS,
            reason: format!(
                "{} ({}) has {:.1} GB free — offloading all layers",
                device.name, device.backend, free_gb
            ),
        };
    }

    let Some(total_layers) = n_layers.filter(|n| *n > 0) else {
        // Without a layer count a partial split cannot be expressed, and
        // guessing high is the failure mode this module exists to prevent.
        return OffloadPlan {
            n_gpu_layers: 0,
            reason: format!(
                "{} ({}) has {:.1} GB free but the model needs {:.1} GB, and its layer \
                 count is unknown — running on CPU",
                device.name,
                device.backend,
                free_gb,
                model_bytes as f64 / 1e9
            ),
        };
    };

    let bytes_per_layer = (model_bytes / u64::from(total_layers)).max(1);
    let fits = u32::try_from(usable / bytes_per_layer).unwrap_or(u32::MAX);
    let layers = fits.min(total_layers);

    OffloadPlan {
        n_gpu_layers: layers,
        reason: format!(
            "{} ({}) has {:.1} GB free — offloading {}/{} layers, rest on CPU",
            device.name, device.backend, free_gb, layers, total_layers
        ),
    }
}

/// Choose which directory to load ggml's loadable backend modules from.
///
/// The bundled directory (inside the app's resources) wins when it exists;
/// otherwise the path baked in at build time, which lives under `target/` and
/// so only resolves for `make dev`; otherwise nothing — in which case ggml
/// keeps its statically linked CPU backend and inference still works, just
/// without a GPU.
///
/// Lives here rather than in the runtime so it compiles and is tested without
/// the `dynamic-backends` feature, which cannot be built on macOS at all (see
/// the feature comment in Cargo.toml).
pub fn resolve_backends_dir(
    bundled: Option<&std::path::Path>,
    compiled_in: Option<&str>,
    exists: impl Fn(&std::path::Path) -> bool,
) -> Option<std::path::PathBuf> {
    if let Some(dir) = bundled {
        if exists(dir) {
            return Some(dir.to_path_buf());
        }
    }
    let compiled = compiled_in.map(std::path::PathBuf::from)?;
    exists(&compiled).then_some(compiled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const GB: u64 = 1_000_000_000;

    // ── resolve_backends_dir ─────────────────────────────────────────────────

    /// Treat every listed path as present on disk.
    fn present(paths: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p| paths.iter().any(|q| Path::new(q) == p)
    }

    #[test]
    fn the_bundled_directory_wins_when_it_exists() {
        // A shipped app must load the modules that travel with it, not a
        // build-time path that happens to still exist on a dev machine.
        let got = resolve_backends_dir(
            Some(Path::new("/app/resources/backends")),
            Some("/build/out/backends"),
            present(&["/app/resources/backends", "/build/out/backends"]),
        );
        assert_eq!(got, Some(PathBuf::from("/app/resources/backends")));
    }

    #[test]
    fn falls_back_to_the_build_time_path_for_dev_runs() {
        // `make dev` runs the binary straight out of target/ with no bundle.
        let got = resolve_backends_dir(
            Some(Path::new("/app/resources/backends")),
            Some("/build/out/backends"),
            present(&["/build/out/backends"]),
        );
        assert_eq!(got, Some(PathBuf::from("/build/out/backends")));
    }

    #[test]
    fn yields_nothing_when_no_candidate_exists() {
        // Must be `None`, not a bogus path: ggml then keeps its built-in CPU
        // backend and inference degrades rather than breaking.
        assert_eq!(
            resolve_backends_dir(Some(Path::new("/app/backends")), Some("/build/backends"), |_| false),
            None
        );
    }

    #[test]
    fn handles_a_static_build_with_no_compiled_in_path() {
        // `BACKENDS_DIR` is `None` on static builds.
        assert_eq!(resolve_backends_dir(None, None, |_| true), None);
        assert_eq!(
            resolve_backends_dir(Some(Path::new("/app/backends")), None, present(&["/app/backends"])),
            Some(PathBuf::from("/app/backends"))
        );
    }

    // ── plan_offload / classify_device ───────────────────────────────────────

    fn discrete(name: &str, free_gb: u64) -> GpuDevice {
        GpuDevice {
            name: name.to_string(),
            backend: "CUDA".to_string(),
            kind: DeviceKind::Discrete,
            memory_free: free_gb * GB,
            memory_total: free_gb * GB,
        }
    }

    fn unified(name: &str, free_gb: u64) -> GpuDevice {
        GpuDevice {
            name: name.to_string(),
            backend: "Metal".to_string(),
            kind: DeviceKind::Unified,
            memory_free: free_gb * GB,
            memory_total: free_gb * GB,
        }
    }

    fn cpu() -> GpuDevice {
        GpuDevice {
            name: "CPU".to_string(),
            backend: "CPU".to_string(),
            kind: DeviceKind::Cpu,
            memory_free: 64 * GB,
            memory_total: 64 * GB,
        }
    }

    // ── classify_device ──────────────────────────────────────────────────────

    #[test]
    fn metal_is_unified_despite_reporting_as_a_gpu() {
        // The regression guard for Apple Silicon: ggml types Metal as `Gpu`,
        // and classifying it as discrete would cap offload on the one platform
        // where full offload is correct.
        assert_eq!(classify_device("Metal", RawDeviceType::Gpu), DeviceKind::Unified);
        assert_eq!(classify_device("metal", RawDeviceType::Gpu), DeviceKind::Unified);
    }

    #[test]
    fn discrete_backends_are_budgeted() {
        for backend in ["CUDA", "Vulkan", "ROCm"] {
            assert_eq!(
                classify_device(backend, RawDeviceType::Gpu),
                DeviceKind::Discrete,
                "{backend} has its own VRAM"
            );
        }
    }

    #[test]
    fn integrated_gpus_share_system_ram() {
        assert_eq!(
            classify_device("Vulkan", RawDeviceType::IntegratedGpu),
            DeviceKind::Unified
        );
    }

    #[test]
    fn cpu_and_unknown_devices_are_never_offload_targets() {
        assert_eq!(classify_device("CPU", RawDeviceType::Cpu), DeviceKind::Cpu);
        assert_eq!(classify_device("BLAS", RawDeviceType::Accelerator), DeviceKind::Other);
        assert_eq!(classify_device("???", RawDeviceType::Unknown), DeviceKind::Other);

        // `best_device` must ignore both.
        let devices = vec![
            GpuDevice {
                kind: DeviceKind::Other,
                ..discrete("accel", 48)
            },
            cpu(),
        ];
        assert_eq!(plan_offload(&devices, GB, Some(8)).n_gpu_layers, 0);
    }

    // ── plan_offload ─────────────────────────────────────────────────────────

    #[test]
    fn no_gpu_means_cpu_only() {
        let plan = plan_offload(&[cpu()], 5 * GB, Some(32));
        assert_eq!(plan.n_gpu_layers, 0);
        assert!(plan.reason.contains("no GPU"), "{}", plan.reason);
    }

    #[test]
    fn empty_device_list_does_not_panic() {
        assert_eq!(plan_offload(&[], 5 * GB, Some(32)).n_gpu_layers, 0);
    }

    #[test]
    fn unified_memory_always_offloads_everything() {
        // The Apple Silicon path. Must not regress: even a model larger than
        // the reported "free" figure is fine, because the GPU and CPU draw on
        // the same pool and the OS pages it.
        for model_gb in [1, 8, 40] {
            let plan = plan_offload(&[unified("Metal", 16)], model_gb * GB, Some(48));
            assert_eq!(
                plan.n_gpu_layers, ALL_LAYERS,
                "unified memory must offload all layers for a {model_gb} GB model"
            );
        }
    }

    #[test]
    fn a_model_that_fits_comfortably_is_fully_offloaded() {
        let plan = plan_offload(&[discrete("RTX 4090", 24)], 5 * GB, Some(32));
        assert_eq!(plan.n_gpu_layers, ALL_LAYERS);
    }

    #[test]
    fn a_model_far_larger_than_vram_is_split_not_forced() {
        // The exact bug this module fixes: a 20 GB model on a 6 GB card used to
        // be sent entirely to the GPU and fail the load.
        let plan = plan_offload(&[discrete("RTX 2060", 6)], 20 * GB, Some(40));
        assert!(
            plan.n_gpu_layers > 0 && plan.n_gpu_layers < 40,
            "expected a partial split, got {}",
            plan.n_gpu_layers
        );
        // 6 GB free → 80% − 512 MB ≈ 4.3 GB usable; 20 GB / 40 layers = 500 MB
        // per layer → 8 layers.
        assert_eq!(plan.n_gpu_layers, 8);
    }

    #[test]
    fn the_split_never_exceeds_the_layer_count() {
        let plan = plan_offload(&[discrete("RTX 4090", 24)], GB, Some(12));
        assert!(plan.n_gpu_layers <= 12 || plan.n_gpu_layers == ALL_LAYERS);
    }

    #[test]
    fn headroom_is_left_for_the_kv_cache_and_compute_buffers() {
        // A model exactly the size of free VRAM must NOT be fully offloaded —
        // inference needs room beyond the weights.
        let plan = plan_offload(&[discrete("RTX 3080", 10)], 10 * GB, Some(40));
        assert_ne!(
            plan.n_gpu_layers, ALL_LAYERS,
            "weights sized to 100% of VRAM leave nothing for inference"
        );
    }

    #[test]
    fn a_tiny_card_falls_back_to_cpu_rather_than_offloading_nothing_usefully() {
        // 512 MB free: the reserve alone exceeds the budget, so usable is 0.
        let device = GpuDevice {
            memory_free: 512 * 1024 * 1024,
            ..discrete("old iGPU", 1)
        };
        let plan = plan_offload(&[device], 5 * GB, Some(32));
        assert_eq!(plan.n_gpu_layers, 0);
    }

    #[test]
    fn unknown_layer_count_degrades_to_all_or_nothing() {
        // Fits → all.
        let plan = plan_offload(&[discrete("RTX 4090", 24)], 5 * GB, None);
        assert_eq!(plan.n_gpu_layers, ALL_LAYERS);

        // Does not fit → CPU, never a guessed split.
        let plan = plan_offload(&[discrete("RTX 2060", 6)], 20 * GB, None);
        assert_eq!(plan.n_gpu_layers, 0);
        assert!(plan.reason.contains("layer count is unknown"), "{}", plan.reason);
    }

    #[test]
    fn zero_layers_is_treated_as_unknown() {
        // A malformed catalog entry must not divide by zero.
        let plan = plan_offload(&[discrete("RTX 2060", 6)], 20 * GB, Some(0));
        assert_eq!(plan.n_gpu_layers, 0);
    }

    #[test]
    fn the_roomiest_gpu_wins() {
        let devices = vec![cpu(), discrete("small", 4), discrete("big", 24)];
        let plan = plan_offload(&devices, 5 * GB, Some(32));
        assert_eq!(plan.n_gpu_layers, ALL_LAYERS);
        assert!(plan.reason.contains("big"), "{}", plan.reason);
    }

    #[test]
    fn unified_is_preferred_over_a_smaller_discrete_card() {
        // A MacBook with an eGPU, or an iGPU + weak discrete pairing: pick on
        // free memory, and unified wins when it has more.
        let devices = vec![unified("Metal", 32), discrete("weak", 2)];
        let plan = plan_offload(&devices, 20 * GB, Some(40));
        assert_eq!(plan.n_gpu_layers, ALL_LAYERS);
        assert!(plan.reason.contains("unified"), "{}", plan.reason);
    }

    #[test]
    fn the_reason_always_explains_the_outcome() {
        // The string is user-facing in the output panel; empty or generic text
        // would leave "why is this slow?" unanswerable.
        let cases = vec![
            plan_offload(&[cpu()], 5 * GB, Some(32)),
            plan_offload(&[discrete("RTX 4090", 24)], 5 * GB, Some(32)),
            plan_offload(&[discrete("RTX 2060", 6)], 20 * GB, Some(40)),
            plan_offload(&[unified("Metal", 16)], 5 * GB, Some(32)),
        ];
        for plan in cases {
            assert!(!plan.reason.is_empty());
            assert!(
                plan.reason.len() > 20,
                "reason should be explanatory, got {:?}",
                plan.reason
            );
        }
    }
}
