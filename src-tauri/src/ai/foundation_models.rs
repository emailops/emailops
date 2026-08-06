//! Apple Foundation Models availability, as seen from Rust.
//!
//! The probe itself is Swift (`src-tauri/ios/EmailOpsFoundationModels.swift`) —
//! `SystemLanguageModel` has no Objective-C interface, so there is no other way
//! in. This module owns the FFI boundary and, more importantly, the *meaning*
//! of what comes back: the status enum is duplicated across a C ABI, and a
//! silent mismatch between the two halves would report "available" for a device
//! that is merely still downloading the model.

/// Why Apple's on-device model can or cannot serve a request.
///
/// Discriminants are load-bearing: they cross the C boundary as `i32` and must
/// match `Status` in `EmailOpsFoundationModels.swift`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum AppleIntelligenceStatus {
    Available = 0,
    /// The hardware cannot run it. The only reason that never changes.
    DeviceNotEligible = 1,
    /// The user has Apple Intelligence switched off in Settings.
    NotEnabled = 2,
    /// Eligible and enabled, but the model assets are still downloading.
    ModelNotReady = 3,
    /// Unavailable for a reason this build does not know about — a newer OS
    /// added a case. Deliberately not fatal.
    UnavailableOther = 4,
    /// The framework is absent: not iOS, or an SDK older than 26.
    FrameworkMissing = 5,
}

impl AppleIntelligenceStatus {
    /// Whether a request may be sent right now.
    pub fn is_available(self) -> bool {
        matches!(self, AppleIntelligenceStatus::Available)
    }

    /// Whether waiting could change the answer. Drives the settings copy: a
    /// downloading model is worth a "check back shortly", an ineligible device
    /// is not.
    pub fn is_transient(self) -> bool {
        matches!(
            self,
            AppleIntelligenceStatus::ModelNotReady | AppleIntelligenceStatus::NotEnabled
        )
    }

    /// Stable wire value for the frontend.
    pub fn as_str(self) -> &'static str {
        match self {
            AppleIntelligenceStatus::Available => "available",
            AppleIntelligenceStatus::DeviceNotEligible => "deviceNotEligible",
            AppleIntelligenceStatus::NotEnabled => "notEnabled",
            AppleIntelligenceStatus::ModelNotReady => "modelNotReady",
            AppleIntelligenceStatus::UnavailableOther => "unavailable",
            AppleIntelligenceStatus::FrameworkMissing => "frameworkMissing",
        }
    }
}

/// Map the raw C return value onto the enum.
///
/// Pure, and separate from the FFI call, so the contract with the Swift side is
/// pinned by tests that run on any host — including the CI machines that never
/// compile the Swift half.
pub fn status_from_raw(raw: i32) -> AppleIntelligenceStatus {
    match raw {
        0 => AppleIntelligenceStatus::Available,
        1 => AppleIntelligenceStatus::DeviceNotEligible,
        2 => AppleIntelligenceStatus::NotEnabled,
        3 => AppleIntelligenceStatus::ModelNotReady,
        4 => AppleIntelligenceStatus::UnavailableOther,
        5 => AppleIntelligenceStatus::FrameworkMissing,
        // An unknown value means the two halves have drifted. "Missing" is the
        // safe reading: it routes work to a backend that definitely exists.
        _ => AppleIntelligenceStatus::FrameworkMissing,
    }
}

/// The Swift probe, registered at launch.
///
/// **Why a registered pointer instead of an `extern "C"` declaration.** Cargo
/// builds this crate as a `cdylib` as well as a staticlib (see Cargo.toml), and
/// a dylib must resolve every symbol at link time. The Swift function lives in
/// the *app* target, not in the crate, so declaring it `extern` made the cdylib
/// link fail with "Undefined symbols: _emailops_ios_apple_intelligence_status"
/// — while the staticlib Xcode actually ships would have been perfectly happy.
///
/// So the dependency points the other way, which is the direction that already
/// works everywhere else here: the app registers its probe with Rust at launch
/// (`EmailOpsAiBridge.m`), and Rust calls back through the pointer. Calling it
/// stays a *live* probe, which matters — three of the four unavailable reasons
/// change while the app is running.
type StatusProbe = extern "C" fn() -> i32;

static PROBE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Called once from the app at launch. Safe to call more than once; the last
/// registration wins.
///
/// # Safety
///
/// `probe` must be a valid `extern "C" fn() -> i32` that stays alive for the
/// life of the process — in practice a `@_cdecl` function in the app binary.
#[no_mangle]
pub extern "C" fn emailops_ios_register_ai_status_probe(probe: StatusProbe) {
    PROBE.store(probe as usize, std::sync::atomic::Ordering::Release);
}

fn registered_probe() -> Option<StatusProbe> {
    match PROBE.load(std::sync::atomic::Ordering::Acquire) {
        0 => None,
        // SAFETY: only ever written by `emailops_ios_register_ai_status_probe`
        // from a `StatusProbe`, and function pointers have no lifetime.
        addr => Some(unsafe { std::mem::transmute::<usize, StatusProbe>(addr) }),
    }
}

/// Probe Apple's on-device model.
///
/// Cheap and side-effect free (the framework caches availability), so callers
/// may ask per turn rather than trusting a launch-time answer. Reports
/// `FrameworkMissing` when no probe has been registered — every non-iOS build,
/// and iOS before the app finishes launching.
pub fn apple_intelligence_status() -> AppleIntelligenceStatus {
    match registered_probe() {
        Some(probe) => status_from_raw(probe()),
        None => AppleIntelligenceStatus::FrameworkMissing,
    }
}

// ── Generation ───────────────────────────────────────────────────────────────

/// Why a generation attempt failed. Mirrors `GenerateResult` in
/// `EmailOpsFoundationModels.swift`; discriminants cross the C ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfmError {
    /// The model is not available right now (see [`AppleIntelligenceStatus`]).
    Unavailable,
    /// Apple's safety guardrails refused the request.
    GuardrailViolation,
    /// The prompt did not fit the model's context window.
    ContextTooLong,
    /// Anything else, including a reason a newer OS introduced.
    Failed,
}

impl AfmError {
    /// Whether the same prompt could succeed later. A guardrail refusal and an
    /// oversized prompt will not; an unavailable model may.
    pub fn is_retryable(self) -> bool {
        matches!(self, AfmError::Unavailable)
    }
}

/// Map the Swift result code onto success or a typed failure.
///
/// Pure and separately tested, because a wrong mapping here is silent: a
/// guardrail refusal read as success would surface Apple's refusal text as if
/// the model had answered the question.
pub fn generate_result_from_raw(raw: i32) -> std::result::Result<(), AfmError> {
    match raw {
        0 => Ok(()),
        1 => Err(AfmError::Unavailable),
        2 => Err(AfmError::GuardrailViolation),
        3 => Err(AfmError::ContextTooLong),
        _ => Err(AfmError::Failed),
    }
}

/// `emailops_ios_afm_generate` — see the Swift side for the contract.
type GenerateFn = extern "C" fn(
    prompt: *const std::os::raw::c_char,
    instructions: *const std::os::raw::c_char,
    temperature: f64,
    max_tokens: i32,
    out_text: *mut *mut std::os::raw::c_char,
) -> i32;

/// `emailops_ios_afm_free` — frees a buffer handed out by the generator.
type FreeFn = extern "C" fn(*mut std::os::raw::c_char);

static GENERATE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static FREE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Registered by the app at launch, alongside the status probe.
///
/// # Safety
///
/// Both pointers must be valid `@_cdecl` functions living in the app binary,
/// and `free` must be the deallocator matching the allocator `generate` used —
/// Swift's `strdup`/`free`, never Rust's.
#[no_mangle]
pub extern "C" fn emailops_ios_register_afm(generate: GenerateFn, free: FreeFn) {
    GENERATE.store(generate as usize, std::sync::atomic::Ordering::Release);
    FREE.store(free as usize, std::sync::atomic::Ordering::Release);
}

/// Whether the app has handed over a generator. False on every desktop build.
pub fn generation_registered() -> bool {
    GENERATE.load(std::sync::atomic::Ordering::Acquire) != 0 && FREE.load(std::sync::atomic::Ordering::Acquire) != 0
}

/// Ask Apple's model for one completion. **Blocks** until it answers — callers
/// must be on a blocking-pool thread, never a runtime worker.
///
/// `temperature` below zero means "the model's default".
pub fn generate_blocking(
    prompt: &str,
    instructions: Option<&str>,
    temperature: f64,
    max_tokens: i32,
) -> std::result::Result<String, (AfmError, String)> {
    let (Some(generate), Some(free)) = (load_fn::<GenerateFn>(&GENERATE), load_fn::<FreeFn>(&FREE)) else {
        return Err((
            AfmError::Unavailable,
            "Apple's on-device model is not wired up in this build".to_string(),
        ));
    };

    let c_prompt = to_c_string(prompt, "prompt")?;
    let c_instructions = match instructions {
        Some(text) => Some(to_c_string(text, "instructions")?),
        None => None,
    };

    let mut out: *mut std::os::raw::c_char = std::ptr::null_mut();
    let code = generate(
        c_prompt.as_ptr(),
        c_instructions.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
        temperature,
        max_tokens,
        &mut out,
    );

    // Taken unconditionally: the Swift side writes a message on failure too, and
    // leaking it on every refusal would be a slow leak in the classifier path.
    let payload = if out.is_null() {
        String::new()
    } else {
        // SAFETY: `out` was written by the registered generator, which
        // guarantees a NUL-terminated buffer it allocated with `strdup`.
        let text = unsafe { std::ffi::CStr::from_ptr(out) }.to_string_lossy().into_owned();
        free(out);
        text
    };

    match generate_result_from_raw(code) {
        Ok(()) => Ok(payload),
        Err(e) => Err((e, payload)),
    }
}

/// Convert to a C string, refusing interior NULs.
///
/// Email bodies are arbitrary bytes and a NUL is not impossible. `CString`
/// rejects them, and rejecting is the honest answer — silently truncating would
/// classify half a message and report success.
pub fn to_c_string(text: &str, label: &str) -> std::result::Result<std::ffi::CString, (AfmError, String)> {
    std::ffi::CString::new(text).map_err(|_| (AfmError::Failed, format!("{label} contains a NUL byte")))
}

fn load_fn<T: Copy>(slot: &std::sync::atomic::AtomicUsize) -> Option<T> {
    match slot.load(std::sync::atomic::Ordering::Acquire) {
        0 => None,
        // SAFETY: only ever written by the registration functions above, from a
        // value of exactly this function-pointer type.
        addr => Some(unsafe { std::mem::transmute_copy::<usize, T>(&addr) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_swift_discriminant_round_trips() {
        // If this table and `Status` in EmailOpsFoundationModels.swift ever
        // disagree, a downloading model reads as "available" and every request
        // fails at the point of use instead of being routed elsewhere.
        let cases = [
            (0, AppleIntelligenceStatus::Available),
            (1, AppleIntelligenceStatus::DeviceNotEligible),
            (2, AppleIntelligenceStatus::NotEnabled),
            (3, AppleIntelligenceStatus::ModelNotReady),
            (4, AppleIntelligenceStatus::UnavailableOther),
            (5, AppleIntelligenceStatus::FrameworkMissing),
        ];
        for (raw, want) in cases {
            assert_eq!(status_from_raw(raw), want, "raw {raw}");
            assert_eq!(want as i32, raw, "discriminant of {want:?}");
        }
    }

    #[test]
    fn an_unknown_value_degrades_to_missing() {
        // A newer Swift half, an older Rust half. Claiming availability here
        // would send work to a framework this build cannot talk to.
        assert_eq!(status_from_raw(99), AppleIntelligenceStatus::FrameworkMissing);
        assert_eq!(status_from_raw(-1), AppleIntelligenceStatus::FrameworkMissing);
    }

    #[test]
    fn only_available_is_available() {
        assert!(AppleIntelligenceStatus::Available.is_available());
        for s in [
            AppleIntelligenceStatus::DeviceNotEligible,
            AppleIntelligenceStatus::NotEnabled,
            AppleIntelligenceStatus::ModelNotReady,
            AppleIntelligenceStatus::UnavailableOther,
            AppleIntelligenceStatus::FrameworkMissing,
        ] {
            assert!(!s.is_available(), "{s:?}");
        }
    }

    #[test]
    fn every_generate_result_code_maps_to_its_swift_case() {
        // A guardrail refusal read as success would surface Apple's refusal
        // text to the user as if the model had answered the question.
        assert_eq!(generate_result_from_raw(0), Ok(()));
        assert_eq!(generate_result_from_raw(1), Err(AfmError::Unavailable));
        assert_eq!(generate_result_from_raw(2), Err(AfmError::GuardrailViolation));
        assert_eq!(generate_result_from_raw(3), Err(AfmError::ContextTooLong));
        assert_eq!(generate_result_from_raw(4), Err(AfmError::Failed));
    }

    #[test]
    fn an_unknown_generate_code_is_a_plain_failure() {
        // A newer Swift half must not be read as success.
        assert_eq!(generate_result_from_raw(42), Err(AfmError::Failed));
        assert_eq!(generate_result_from_raw(-7), Err(AfmError::Failed));
    }

    #[test]
    fn only_unavailability_is_worth_retrying() {
        assert!(AfmError::Unavailable.is_retryable());
        assert!(!AfmError::GuardrailViolation.is_retryable());
        assert!(!AfmError::ContextTooLong.is_retryable());
        assert!(!AfmError::Failed.is_retryable());
    }

    #[test]
    fn generation_is_unregistered_off_device() {
        assert!(!generation_registered());
        let err = generate_blocking("hello", None, -1.0, 0).unwrap_err();
        assert_eq!(err.0, AfmError::Unavailable);
    }

    #[test]
    fn a_string_with_an_interior_nul_is_refused_not_truncated() {
        // Tested on the conversion directly: going through `generate_blocking`
        // would hit the unregistered-generator check first and prove nothing.
        let err = to_c_string("before\0after", "prompt").unwrap_err();
        assert_eq!(err.0, AfmError::Failed);
        assert!(err.1.contains("prompt"), "message names the field: {}", err.1);

        assert!(to_c_string("perfectly ordinary text", "prompt").is_ok());
    }

    #[test]
    fn transient_reasons_are_the_ones_worth_retrying() {
        // Drives the settings copy: "still downloading" and "you turned it off"
        // are fixable now; ineligible hardware is not.
        assert!(AppleIntelligenceStatus::ModelNotReady.is_transient());
        assert!(AppleIntelligenceStatus::NotEnabled.is_transient());
        assert!(!AppleIntelligenceStatus::DeviceNotEligible.is_transient());
        assert!(!AppleIntelligenceStatus::FrameworkMissing.is_transient());
    }

    extern "C" fn fake_not_enabled() -> i32 {
        AppleIntelligenceStatus::NotEnabled as i32
    }

    #[test]
    fn the_probe_is_absent_until_the_app_registers_one() {
        // Both halves in one test on purpose: `PROBE` is process-global and
        // cargo runs tests in parallel, so splitting this in two would make
        // whichever ran second read the other's registration.
        assert_eq!(apple_intelligence_status(), AppleIntelligenceStatus::FrameworkMissing);

        emailops_ios_register_ai_status_probe(fake_not_enabled);
        assert_eq!(apple_intelligence_status(), AppleIntelligenceStatus::NotEnabled);

        PROBE.store(0, std::sync::atomic::Ordering::Release);
        assert_eq!(apple_intelligence_status(), AppleIntelligenceStatus::FrameworkMissing);
    }
}
