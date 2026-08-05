//! System-capability probes used to size AI runtime resources.
//!
//! Probes here delegate to `sysinfo` / `fs4` rather than to hand-written per-OS
//! code wherever one implementation can serve every platform. That is
//! deliberate: hand-written arms could only ever be compiled on the developer's
//! own platform, and an earlier `#[cfg(not(any(macos, linux)))]` fallback
//! silently returned "unknown" on Windows — which pinned the automatic context
//! window to its smallest tier on every Windows machine.
//!
//! [`total_ram_bytes`] is the one exception, and it earns the `#[cfg]` split: on
//! iOS the portable answer (device RAM) is not merely imprecise, it is the wrong
//! quantity. See that function's docs.

use std::path::Path;

/// Memory in bytes this process may actually use, or `None` when the probe
/// fails. Callers must treat `None` as "assume a small machine" — never as an
/// error.
///
/// On desktop this is total physical RAM: the process may use essentially all
/// of it, and the OS will page rather than kill.
///
/// **iOS is different and the difference is not cosmetic.** `sysinfo` reports
/// the device's physical RAM (8 GB on an iPhone 16 Pro), but jetsam kills any
/// app that exceeds a per-process limit far below that. Sizing a model against
/// the device total picks weights that cannot be loaded — on the simulator it
/// reported the *Mac's* RAM and selected a 3 GB model that was never even
/// downloaded. See `docs/DECISIONS.md`, "iOS targets iOS 26".
#[cfg(not(target_os = "ios"))]
pub fn total_ram_bytes() -> Option<u64> {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    // sysinfo reports 0 rather than an error when it cannot read the value.
    match sys.total_memory() {
        0 => None,
        bytes => Some(bytes),
    }
}

/// iOS: bytes this process can still allocate before jetsam terminates it.
///
/// `os_proc_available_memory()` (`<os/proc.h>`) is the only figure that
/// reflects the real ceiling — it already accounts for what the app has
/// allocated and for whether the `com.apple.developer.kernel.increased-memory-limit`
/// entitlement raised the limit. It returns 0 from contexts where it is
/// unsupported (app extensions), which maps to `None` like any other failure.
#[cfg(target_os = "ios")]
pub fn total_ram_bytes() -> Option<u64> {
    unsafe extern "C" {
        fn os_proc_available_memory() -> usize;
    }
    // SAFETY: no arguments, no pointers; returns a scalar byte count.
    match unsafe { os_proc_available_memory() } {
        0 => None,
        bytes => Some(bytes as u64),
    }
}

/// Free space in bytes available to an unprivileged user on the filesystem
/// holding `path`, or `None` when the probe fails.
///
/// `path` need not exist; the nearest existing ancestor is probed instead, so
/// callers can ask about a directory they are about to create (e.g. the model
/// download target).
pub fn available_disk_bytes(path: &Path) -> Option<u64> {
    let probe = nearest_existing_ancestor(path)?;
    match fs4::available_space(probe) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            eprintln!("[system] disk-space probe failed for {}: {e}", probe.display());
            None
        }
    }
}

/// Walk up from `path` until a component that exists on disk is found.
///
/// Pure apart from the `exists()` calls, and the piece most likely to be wrong,
/// so it is unit-tested directly.
fn nearest_existing_ancestor(path: &Path) -> Option<&Path> {
    let mut candidate = Some(path);
    while let Some(p) = candidate {
        if p.exists() {
            return Some(p);
        }
        candidate = p.parent();
    }
    None
}

/// RAM in whole gibibytes, rounded down. `0` when the probe failed.
///
/// Convenience wrapper for callers that gate on coarse tiers (model-size
/// recommendations, onboarding capability checks) rather than exact bytes.
pub fn total_ram_gb() -> u64 {
    total_ram_bytes().map_or(0, |b| b / (1024 * 1024 * 1024))
}

/// Is this process running under Rosetta translation?
///
/// `std::env::consts::ARCH` is a compile-time constant, so an x86_64 build
/// reports `x86_64` whether it is on a real Intel Mac or being translated on
/// Apple Silicon. Bug reports carrying that string are therefore ambiguous
/// exactly where it matters most — an Intel Mac cannot run the Metal AI
/// runtime, an Apple Silicon one can. `sysctl.proc_translated` disambiguates.
///
/// Shells out rather than calling `sysctlbyname` because `libc` was removed
/// from this crate on purpose (see Cargo.toml); this runs once, only when the
/// user opens the feedback form, so the process spawn is not a hot path.
/// Anything other than a clean `1` is reported as "not translated" — a probe
/// failure must not masquerade as a positive result.
pub fn is_rosetta_translated() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    std::process::Command::new("sysctl")
        .args(["-n", "sysctl.proc_translated"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim() == "1")
        .unwrap_or(false)
}

/// RAM tier for the AUTOMATIC context window (`chat.n_ctx` unset/0).
///
/// Hard-capped at 32768 regardless of RAM — anything larger is opt-in via
/// the explicit `chat.n_ctx` preference (bigger windows cost real memory and
/// most mailbox turns never need them). `None` RAM (probe failed) falls back
/// to the conservative 8192 baseline.
pub fn auto_n_ctx_tier(total_ram_bytes: Option<u64>) -> u32 {
    const GIB: u64 = 1024 * 1024 * 1024;
    match total_ram_bytes {
        Some(ram) if ram >= 24 * GIB => 32768,
        Some(ram) if ram >= 16 * GIB => 16384,
        _ => 8192,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_n_ctx_tier_table() {
        const GIB: u64 = 1024 * 1024 * 1024;
        // (total_ram, expected, label)
        let cases: &[(Option<u64>, u32, &str)] = &[
            (None, 8192, "unknown RAM falls back to the conservative baseline"),
            (Some(8 * GIB), 8192, "8GB stays at the baseline"),
            (Some(16 * GIB - 1), 8192, "just under 16GB stays at the baseline"),
            (Some(16 * GIB), 16384, "16GB unlocks 16k"),
            (Some(24 * GIB - 1), 16384, "just under 24GB stays at 16k"),
            (Some(24 * GIB), 32768, "24GB unlocks 32k"),
            (Some(64 * GIB), 32768, "more RAM never exceeds the 32k auto cap"),
            (
                Some(192 * GIB),
                32768,
                "even workstation RAM stays at 32k — larger is opt-in",
            ),
        ];
        for (ram, want, label) in cases {
            assert_eq!(auto_n_ctx_tier(*ram), *want, "{label}");
        }
    }

    /// Runs on every platform now — on a Windows CI runner this is what proves
    /// the probe no longer returns `None` there.
    ///
    /// iOS is excluded from the 1 GiB floor rather than from the test: the
    /// probe reports what jetsam will still let this process allocate, which
    /// legitimately drops below a gigabyte on a loaded device. Asserting only
    /// non-zero still catches a broken FFI declaration.
    #[test]
    fn total_ram_probe_returns_plausible_value() {
        let ram = total_ram_bytes().expect("RAM probe should work on every supported platform");
        if cfg!(target_os = "ios") {
            assert!(ram > 0, "iOS available-memory probe returned 0");
        } else {
            assert!(ram >= 1024 * 1024 * 1024, "expected at least 1GiB, got {ram}");
        }
    }

    /// The probe must answer for *this process*, not for the machine. On iOS a
    /// device-total answer is what selected an unloadable model; everywhere
    /// else the two coincide, so the assertion is only meaningful on iOS —
    /// but keeping it unconditional documents the contract at every call site.
    #[test]
    fn total_ram_probe_never_exceeds_a_plausible_process_ceiling() {
        let Some(ram) = total_ram_bytes() else {
            return; // probe failure is a documented outcome, not a test failure
        };
        if cfg!(target_os = "ios") {
            // No shipping iPhone lets one process address 16 GiB. A figure at
            // or above this means the device total leaked back in.
            assert!(
                ram < 16 * 1024 * 1024 * 1024,
                "iOS probe reported {ram} bytes — that is device RAM, not the per-process limit"
            );
        }
    }

    #[test]
    fn total_ram_gb_agrees_with_the_byte_probe() {
        let gb = total_ram_gb();
        assert!(gb >= 1, "expected at least 1GiB, got {gb}");
        if let Some(bytes) = total_ram_bytes() {
            assert_eq!(gb, bytes / (1024 * 1024 * 1024));
        }
    }

    #[test]
    fn disk_probe_reports_space_for_an_existing_dir() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let free = available_disk_bytes(tmp.path()).expect("disk probe should work");
        assert!(free > 0, "expected a non-zero free-space figure, got {free}");
    }

    #[test]
    fn disk_probe_falls_back_to_an_existing_ancestor() {
        // The model-download flow asks about a directory it has not created
        // yet; probing must not fail just because the leaf is missing.
        let tmp = tempfile::tempdir().expect("temp dir");
        let missing = tmp.path().join("models").join("chat").join("not-yet");
        assert!(!missing.exists());

        let free = available_disk_bytes(&missing).expect("probe should walk up to an existing dir");
        assert!(free > 0, "expected a non-zero free-space figure, got {free}");
    }

    #[test]
    fn nearest_existing_ancestor_finds_the_deepest_real_dir() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let real = tmp.path().join("real");
        std::fs::create_dir(&real).expect("create dir");

        let probe = real.join("a").join("b");
        assert_eq!(nearest_existing_ancestor(&probe), Some(real.as_path()));
    }

    #[test]
    fn nearest_existing_ancestor_gives_up_on_a_relative_phantom_path() {
        // A relative path whose components do not exist bottoms out at the
        // empty parent, which must yield `None` rather than looping forever.
        let probe = Path::new("definitely-not-a-real-dir-9f3a/child");
        assert_eq!(nearest_existing_ancestor(probe), None);
    }
}
