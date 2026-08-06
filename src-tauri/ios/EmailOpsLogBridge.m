//
//  Gives Rust a way to reach the iOS system log.
//
//  Source of truth. `scripts/ios_patch_project.sh` copies this into
//  `gen/apple/Sources/emailops/`, which `tauri ios init` regenerates.
//
//  ── Why this exists ────────────────────────────────────────────────────────
//
//  `app-log` events are rendered by the desktop output panel, which the phone
//  layout does not have. Without this, everything the sync scheduler, AI
//  pipeline and background refresh report on a device goes to the webview and
//  is never seen — on the platform with the fewest other ways to look.
//
//  With it, `xcrun devicectl device process launch --console` and `log stream`
//  show those lines live.
//
//  ── Why registration, rather than Rust calling NSLog directly ──────────────
//
//  Same constraint as EmailOpsAiBridge.m: the crate is also built as a cdylib,
//  and a dylib resolves every symbol at link time, so a Rust `extern "C"`
//  declaration of a symbol that lives in the app target breaks the cargo link
//  before Xcode is involved. The app pushes the sink into Rust instead.
//
//  `+load` runs before `main()`, so the sink is in place before the Rust
//  runtime emits its first line.
//

#import <Foundation/Foundation.h>

/// Implemented in Rust — see `services::ios_log`.
extern void emailops_ios_register_log_sink(void (*sink)(const char *));

/// Also Rust — calls straight back through the pointer just registered.
extern void emailops_ios_log_self_test(void);

static void EmailOpsWriteLog(const char *line) {
    if (line == NULL) {
        return;
    }
    // %s, not the string itself: a log line carries user-influenced text
    // (subjects, error messages), and passing it as the format would let a
    // stray "%@" read a garbage pointer.
    NSLog(@"[emailops] %s", line);
}

@interface EmailOpsLogBridge : NSObject
@end

@implementation EmailOpsLogBridge

+ (void)load {
    emailops_ios_register_log_sink(&EmailOpsWriteLog);
    // Logged so the console can tell "the bridge never loaded" apart from
    // "the bridge loaded and Rust emitted nothing" — two very different bugs
    // that look identical from outside.
    NSLog(@"[emailops] log bridge registered at load");
    // Round-trip check: if the next line does not appear, Rust is reading a
    // different `SINK` than the one just written to, and every later silence
    // from the Rust side is a linkage artefact rather than a quiet app.
    emailops_ios_log_self_test();
}

@end
