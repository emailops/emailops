//
//  Apple Foundation Models availability probe.
//
//  Source of truth. `scripts/ios_patch_project.sh` copies this into
//  `gen/apple/Sources/emailops/`, which `tauri ios init` regenerates.
//
//  ── Why Swift, when everything else here is Objective-C ────────────────────
//
//  The FoundationModels framework is Swift-only: `SystemLanguageModel` has no
//  Objective-C interface, so there is no way to reach it from `main.mm` or from
//  Rust directly. `@_cdecl` exports a plain C symbol the Rust side can link
//  against without a bridging header in either direction.
//
//  ── Why availability is a runtime question ─────────────────────────────────
//
//  It is tempting to infer it from the device model, and wrong. Apple reports
//  four distinct unavailable cases, and three of them can change *while the app
//  is running*: the user turned Apple Intelligence off in Settings, the model
//  assets are still downloading, or the device is in a mode where it will not
//  serve requests. Only "device not eligible" is fixed. So this is probed per
//  call, and `ai::device_tier` treats the answer as a live fact rather than a
//  capability baked in at launch.
//
//  Returns a small integer rather than a bool so the reason survives the C
//  boundary — the settings UI has to explain *why* on-device AI is off, and
//  "not eligible" and "you switched it off" deserve different words.
//

import Foundation

#if canImport(FoundationModels)
    import FoundationModels
#endif

/// Keep in sync with `AppleIntelligenceStatus` in
/// `src-tauri/src/ai/foundation_models.rs`.
private enum Status: Int32 {
    case available = 0
    case deviceNotEligible = 1
    case notEnabled = 2
    case modelNotReady = 3
    case unavailableOther = 4
    /// The framework is not in this SDK at all — the build is older than iOS 26.
    case frameworkMissing = 5
}

/// Whether Apple's on-device model can serve a request right now.
///
/// Safe to call from any thread and from a background-refresh window: it reads
/// a cached availability value and performs no I/O.
@_cdecl("emailops_ios_apple_intelligence_status")
public func emailopsAppleIntelligenceStatus() -> Int32 {
    #if canImport(FoundationModels)
        if #available(iOS 26.0, *) {
            switch SystemLanguageModel.default.availability {
            case .available:
                return Status.available.rawValue
            case .unavailable(.deviceNotEligible):
                return Status.deviceNotEligible.rawValue
            case .unavailable(.appleIntelligenceNotEnabled):
                return Status.notEnabled.rawValue
            case .unavailable(.modelNotReady):
                return Status.modelNotReady.rawValue
            case .unavailable:
                // A reason added by a later OS. Reported as "unavailable" with
                // no detail rather than crashing on an unknown case — the app
                // must keep working on an OS newer than it was built against.
                return Status.unavailableOther.rawValue
            }
        }
        return Status.frameworkMissing.rawValue
    #else
        return Status.frameworkMissing.rawValue
    #endif
}

// ── Generation ───────────────────────────────────────────────────────────────
//
// `LanguageModelSession.respond(to:)` is async, and the Rust side of this
// boundary is a plain C function pointer. Rather than build a callback protocol
// across the FFI (context pointers, lifetimes, thread-safety, cancellation),
// this blocks on a semaphore and the Rust caller invokes it from
// `tokio::task::spawn_blocking` — a blocking-pool thread, never a runtime
// worker. The trade is real but small: Apple's model is only ever asked short,
// structured questions (classification, tags, junk, translation, per-email
// summaries), never chat over retrieved threads, whose context this model
// cannot hold.

/// Result codes, mirrored by `AfmError` in `ai::foundation_models`.
private enum GenerateResult: Int32 {
    case ok = 0
    case unavailable = 1
    case guardrailViolation = 2
    case contextTooLong = 3
    case failed = 4
}

/// Copy a Swift string into a C buffer the caller owns.
/// Freed by `emailops_ios_afm_free`, never by Rust's allocator — the two heaps
/// are different and mixing them corrupts both.
private func copyToC(_ text: String) -> UnsafeMutablePointer<CChar>? {
    return strdup(text)
}

/// Free a buffer handed out by `emailops_ios_afm_generate`.
@_cdecl("emailops_ios_afm_free")
public func emailopsAfmFree(_ ptr: UnsafeMutablePointer<CChar>?) {
    guard let ptr else { return }
    free(ptr)
}

/// Ask Apple's on-device model for a single completion.
///
/// Returns a `GenerateResult` code. On success `outText` receives a malloc'd
/// UTF-8 string; on failure it receives a malloc'd error message (or nil).
/// Either way the caller must free it with `emailops_ios_afm_free`.
///
/// Blocks the calling thread until the model answers.
@_cdecl("emailops_ios_afm_generate")
public func emailopsAfmGenerate(
    _ prompt: UnsafePointer<CChar>?,
    _ instructions: UnsafePointer<CChar>?,
    _ temperature: Double,
    _ maxTokens: Int32,
    _ outText: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> Int32 {
    guard let outText else { return GenerateResult.failed.rawValue }
    outText.pointee = nil

    guard let prompt, let promptText = String(validatingUTF8: prompt) else {
        outText.pointee = copyToC("prompt was not valid UTF-8")
        return GenerateResult.failed.rawValue
    }
    let instructionText = instructions.flatMap { String(validatingUTF8: $0) }

    #if canImport(FoundationModels)
        if #available(iOS 26.0, *) {
            guard case .available = SystemLanguageModel.default.availability else {
                outText.pointee = copyToC("Apple Intelligence is not available on this device right now")
                return GenerateResult.unavailable.rawValue
            }

            var code = GenerateResult.failed
            var payload: String?
            let done = DispatchSemaphore(value: 0)

            Task {
                defer { done.signal() }
                do {
                    let session = instructionText.map { LanguageModelSession(instructions: $0) }
                        ?? LanguageModelSession()
                    var options = GenerationOptions()
                    if temperature >= 0 {
                        options = GenerationOptions(temperature: temperature)
                    }
                    if maxTokens > 0 {
                        options = GenerationOptions(
                            temperature: temperature >= 0 ? temperature : nil,
                            maximumResponseTokens: Int(maxTokens)
                        )
                    }
                    let response = try await session.respond(to: promptText, options: options)
                    payload = response.content
                    code = .ok
                } catch let error as LanguageModelSession.GenerationError {
                    // Guardrails and context overflow are the two the caller can
                    // act on: one means "ask differently", the other "send less".
                    switch error {
                    case .guardrailViolation:
                        payload = "the request was refused by Apple's safety guardrails"
                        code = .guardrailViolation
                    case .exceededContextWindowSize:
                        payload = "the prompt exceeds this model's context window"
                        code = .contextTooLong
                    default:
                        payload = String(describing: error)
                        code = .failed
                    }
                } catch {
                    payload = error.localizedDescription
                    code = .failed
                }
            }

            done.wait()
            outText.pointee = payload.flatMap(copyToC)
            return code.rawValue
        }
        outText.pointee = copyToC("this OS predates the Foundation Models framework")
        return GenerateResult.unavailable.rawValue
    #else
        outText.pointee = copyToC("this build has no Foundation Models framework")
        return GenerateResult.unavailable.rawValue
    #endif
}
