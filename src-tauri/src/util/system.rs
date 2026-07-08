//! System-capability probes used to size AI runtime resources.

/// Total physical RAM in bytes, or `None` when the platform probe fails.
/// Callers must treat `None` as "assume a small machine" — never as an error.
#[cfg(target_os = "macos")]
pub fn total_ram_bytes() -> Option<u64> {
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    let name = c"hw.memsize";
    // SAFETY: sysctlbyname writes at most `size` bytes into `value`, and we
    // pass the exact buffer size of the u64 it documents for hw.memsize.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut value as *mut u64 as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc == 0 && value > 0 {
        Some(value)
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
pub fn total_ram_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb_line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = kb_line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn total_ram_bytes() -> Option<u64> {
    None
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

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn total_ram_probe_returns_plausible_value() {
        let ram = total_ram_bytes().expect("RAM probe should work on macOS/Linux");
        assert!(ram >= 1024 * 1024 * 1024, "expected at least 1GiB, got {ram}");
    }
}
