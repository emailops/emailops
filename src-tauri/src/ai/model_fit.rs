//! Whether a catalog model can actually be downloaded and run on this device.
//!
//! Pure decision, no I/O: the caller supplies the probes from `util::system`.
//! Split out of the catalog command so the thresholds are table-testable, and
//! because the answer differs in kind between platforms rather than in degree.
//!
//! **Desktop memory limits are soft, iOS's is hard.** On a desktop, "not enough
//! RAM" means paging and a slow reply; the user is entitled to try, which is
//! why the catalog has always shown every entry with its `min_ram_gb` printed
//! next to it. On iOS, `os_proc_available_memory()` is the jetsam ceiling — a
//! model above it is not slow, it is a process kill mid-answer. That is the
//! difference `memory_is_hard_limit` encodes, and it is why a phone gets
//! entries removed rather than annotated.

/// How a catalog entry stands on this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFit {
    /// Download and run it.
    Fits,
    /// Runnable, but above the model's stated comfortable RAM. Offered anyway —
    /// only reachable where the memory limit is soft.
    Tight,
    /// Cannot run here: the weights do not fit under a hard memory ceiling.
    TooLarge,
    /// Would not fit on disk, with room to spare for the app's own data.
    NoDiskSpace,
}

impl ModelFit {
    /// Whether the UI should offer a download at all.
    pub fn is_downloadable(self) -> bool {
        matches!(self, ModelFit::Fits | ModelFit::Tight)
    }

    /// Stable wire value for the frontend.
    pub fn as_str(self) -> &'static str {
        match self {
            ModelFit::Fits => "fits",
            ModelFit::Tight => "tight",
            ModelFit::TooLarge => "tooLarge",
            ModelFit::NoDiskSpace => "noDiskSpace",
        }
    }
}

/// Runtime overhead above the weights themselves: KV cache, activations, and
/// the app's own working set. Deliberately generous — being wrong downward
/// costs a jetsam kill, being wrong upward costs one catalog entry.
const RUNTIME_OVERHEAD_BYTES: u64 = 768 * 1024 * 1024;

/// Disk left over after the download. A device with no headroom cannot sync
/// mail, and a full disk during the final rename loses the whole download.
const DISK_HEADROOM_BYTES: u64 = 1024 * 1024 * 1024;

/// What a failed memory probe means where the ceiling is hard. `total_ram_bytes`
/// returns `None` from contexts where `os_proc_available_memory()` is
/// unsupported; assuming a small machine is the documented contract, and here
/// it means offering nothing rather than offering a kill.
const UNKNOWN_HARD_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

/// Decide whether `size_bytes` of weights (declaring `min_ram_gb` of
/// comfortable RAM) can be downloaded and run.
///
/// `available_memory` is total RAM where the limit is soft, and the
/// per-process ceiling where it is hard. `None` means the probe failed.
/// `available_disk` is free bytes on the models volume, `None` if unprobed —
/// an unknown disk never blocks, since a failed download is recoverable and a
/// false "no space" is not.
pub fn model_fit(
    size_bytes: u64,
    min_ram_gb: u8,
    available_memory: Option<u64>,
    available_disk: Option<u64>,
    memory_is_hard_limit: bool,
) -> ModelFit {
    if let Some(disk) = available_disk {
        if disk < size_bytes.saturating_add(DISK_HEADROOM_BYTES) {
            return ModelFit::NoDiskSpace;
        }
    }

    if memory_is_hard_limit {
        // Compare against the weights, not `min_ram_gb`: that field describes a
        // whole machine ("8+ GB RAM"), while the ceiling here is a few GB of
        // per-process allowance. Judging a 3 GB model by its 8 GB device
        // requirement would reject every entry on every phone.
        let ceiling = available_memory.unwrap_or(UNKNOWN_HARD_LIMIT_BYTES);
        return if ceiling >= size_bytes.saturating_add(RUNTIME_OVERHEAD_BYTES) {
            ModelFit::Fits
        } else {
            ModelFit::TooLarge
        };
    }

    match available_memory {
        // Soft limit: the OS pages, so "too small" is a warning, never a veto.
        Some(ram) if ram < u64::from(min_ram_gb).saturating_mul(BYTES_PER_GIB) => ModelFit::Tight,
        _ => ModelFit::Fits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn a_small_model_fits_under_a_hard_ceiling_with_room_for_the_kv_cache() {
        // 3 GB weights on a phone allowed ~4.5 GB by the increased-memory-limit
        // entitlement: fits, with the runtime overhead accounted for.
        assert_eq!(
            model_fit(3 * GIB, 8, Some(4 * GIB + GIB / 2), None, true),
            ModelFit::Fits
        );
    }

    #[test]
    fn a_large_model_is_removed_rather_than_offered_under_a_hard_ceiling() {
        // The 22 GB catalog entry against a phone's real allowance. Offering it
        // is not "slow", it is a jetsam kill part-way through the first answer.
        assert_eq!(model_fit(22 * GIB, 32, Some(4 * GIB), None, true), ModelFit::TooLarge);
    }

    #[test]
    fn a_hard_ceiling_leaves_room_for_the_kv_cache_and_activations() {
        // Weights alone would "fit" in 3.5 GB; with the runtime on top they do not.
        assert_eq!(
            model_fit(3 * GIB, 8, Some(3 * GIB + GIB / 2), None, true),
            ModelFit::TooLarge
        );
    }

    #[test]
    fn a_failed_probe_under_a_hard_ceiling_offers_nothing_it_cannot_prove() {
        // os_proc_available_memory returns 0 in unsupported contexts. Assume a
        // small machine, per util::system's contract.
        assert_eq!(model_fit(3 * GIB, 8, None, None, true), ModelFit::TooLarge);
        assert_eq!(model_fit(GIB / 2, 4, None, None, true), ModelFit::Fits);
    }

    #[test]
    fn a_soft_limit_never_vetoes_a_model_for_memory() {
        // Desktop: undersized RAM means paging, and the user may still want to
        // try. This is the pre-existing behaviour and must not regress.
        assert_eq!(model_fit(22 * GIB, 32, Some(8 * GIB), None, false), ModelFit::Tight);
        assert!(model_fit(22 * GIB, 32, Some(8 * GIB), None, false).is_downloadable());
    }

    #[test]
    fn a_soft_limit_with_enough_ram_simply_fits() {
        assert_eq!(model_fit(3 * GIB, 8, Some(16 * GIB), None, false), ModelFit::Fits);
    }

    #[test]
    fn an_unprobed_machine_with_a_soft_limit_is_not_second_guessed() {
        assert_eq!(model_fit(22 * GIB, 32, None, None, false), ModelFit::Fits);
    }

    #[test]
    fn disk_space_is_checked_with_headroom_left_over() {
        // Exactly enough room for the file and nothing else is not enough: the
        // final rename needs space, and a phone with 0 bytes free cannot sync.
        assert_eq!(
            model_fit(3 * GIB, 8, Some(16 * GIB), Some(3 * GIB), false),
            ModelFit::NoDiskSpace
        );
        assert_eq!(
            model_fit(3 * GIB, 8, Some(16 * GIB), Some(5 * GIB), false),
            ModelFit::Fits
        );
    }

    #[test]
    fn disk_space_is_judged_before_memory() {
        // Both are wrong; the actionable one (free some space) is reported.
        assert_eq!(
            model_fit(22 * GIB, 32, Some(4 * GIB), Some(GIB), true),
            ModelFit::NoDiskSpace
        );
    }

    #[test]
    fn an_unknown_disk_never_blocks_a_download() {
        assert_eq!(model_fit(3 * GIB, 8, Some(16 * GIB), None, false), ModelFit::Fits);
    }

    #[test]
    fn only_runnable_entries_are_downloadable() {
        assert!(ModelFit::Fits.is_downloadable());
        assert!(ModelFit::Tight.is_downloadable());
        assert!(!ModelFit::TooLarge.is_downloadable());
        assert!(!ModelFit::NoDiskSpace.is_downloadable());
    }
}
