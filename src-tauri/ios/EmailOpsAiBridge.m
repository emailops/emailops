//
//  Hands the Swift Apple-Intelligence probe to Rust at launch.
//
//  Source of truth. `scripts/ios_patch_project.sh` copies this into
//  `gen/apple/Sources/emailops/`, which `tauri ios init` regenerates.
//
//  ── Why registration, rather than Rust calling Swift directly ──────────────
//
//  Cargo builds this crate as a `cdylib` as well as the staticlib Xcode links,
//  and a dylib must resolve every symbol at link time. A Rust `extern "C"`
//  declaration of the Swift function therefore broke the *cargo* link long
//  before Xcode got involved:
//
//    Undefined symbols for architecture arm64:
//      "_emailops_ios_apple_intelligence_status", referenced from: ...
//
//  The symbol lives in the app target, which cargo knows nothing about. So the
//  dependency is inverted — the app pushes a function pointer into Rust, which
//  is the same direction as the background-refresh bridge and links cleanly in
//  both build systems. Rust still *calls* it on demand, so availability stays a
//  live answer rather than a launch-time snapshot; three of the four
//  unavailable reasons change while the app is running.
//
//  `+load` runs before `main()`, so the probe is in place before any Rust code
//  can ask.
//

#import <Foundation/Foundation.h>

/// Implemented in EmailOpsFoundationModels.swift (`@_cdecl`).
extern int32_t emailops_ios_apple_intelligence_status(void);

/// Generation, also from EmailOpsFoundationModels.swift.
extern int32_t emailops_ios_afm_generate(const char *prompt,
                                         const char *instructions,
                                         double temperature,
                                         int32_t maxTokens,
                                         char **outText);
extern void emailops_ios_afm_free(char *ptr);

/// Implemented in Rust — see `ai::foundation_models`.
extern void emailops_ios_register_ai_status_probe(int32_t (*probe)(void));
extern void emailops_ios_register_afm(int32_t (*generate)(const char *, const char *, double, int32_t, char **),
                                      void (*freeFn)(char *));

@interface EmailOpsAiBridge : NSObject
@end

@implementation EmailOpsAiBridge

+ (void)load {
    emailops_ios_register_ai_status_probe(&emailops_ios_apple_intelligence_status);
    // The generator and its deallocator travel together on purpose: the buffer
    // comes from Swift's `strdup`, so it must go back to Swift's `free`. Rust's
    // allocator freeing it would corrupt both heaps.
    emailops_ios_register_afm(&emailops_ios_afm_generate, &emailops_ios_afm_free);
    // Logged once at launch because this is otherwise invisible from outside
    // the app: the value decides whether on-device AI is offered at all, and
    // "why is Apple Intelligence off on my phone" is answerable from the device
    // console rather than by guessing. Values match `AppleIntelligenceStatus`
    // in `ai::foundation_models`: 0 available, 1 device not eligible,
    // 2 not enabled, 3 model not ready, 4 unavailable, 5 framework missing.
    NSLog(@"[emailops] apple intelligence status=%d", emailops_ios_apple_intelligence_status());
}

@end
