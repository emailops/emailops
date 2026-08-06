//
//  Background refresh registration for EmailOps.
//
//  Source of truth. `scripts/ios_patch_project.sh` copies this into
//  `gen/apple/Sources/emailops/`, which `tauri ios init` regenerates.
//
//  ── Why a `+load` observer instead of an app delegate ──────────────────────
//
//  BGTaskScheduler requires every launch handler to be registered *before*
//  `application:didFinishLaunchingWithOptions:` returns, or the system throws.
//  This app has no app delegate to put that in: `main.mm` calls Rust's
//  `start_app()`, and Tauri creates the UIApplication and its delegate inside
//  its own runtime. Subclassing or swizzling that delegate would couple us to
//  Tauri's internals and break on upgrade.
//
//  `+load` runs when the image is loaded — before `main()`, so before anything
//  has launched — and the notification it subscribes to is posted *during*
//  launch, from inside UIKit's own didFinishLaunching handling. That is early
//  enough for the scheduler and needs nothing from Tauri.
//
//  ── What this does and does not buy ────────────────────────────────────────
//
//  BGAppRefreshTask is opportunistic: iOS decides when, learning from how the
//  user opens the app, typically minutes to hours apart, never on demand, and
//  never at all while Low Power Mode is on or after the user force-quits. It
//  makes the inbox current *before* the app is opened. It is not new-mail
//  delivery — that needs a server pushing through APNs. See docs/DECISIONS.md.
//
//  ── Testing it ─────────────────────────────────────────────────────────────
//
//  The scheduler will not run this on demand, and never on the simulator. On a
//  device, from Xcode: pause the debugger after backgrounding the app and run
//
//    e -l objc -- (void)[[BGTaskScheduler sharedScheduler] \
//        _simulateLaunchForTaskWithIdentifier:@"com.emailops.app.refresh"]
//
//  then resume. `_simulateExpirationForTaskWithIdentifier:` exercises the
//  expiration path the same way.
//

#import <BackgroundTasks/BackgroundTasks.h>
#import <Foundation/Foundation.h>
#import <UIKit/UIKit.h>

/// Implemented in Rust — see `services::background_refresh`.
extern bool emailops_ios_background_refresh(void);
extern void emailops_ios_expire_background_refresh(void);

/// Must match `BGTaskSchedulerPermittedIdentifiers` in Info.plist. The system
/// rejects a request whose identifier is not declared there.
static NSString *const kEmailOpsRefreshTaskIdentifier = @"com.emailops.app.refresh";

/// Earliest the system may run the next refresh. A floor, not a schedule: iOS
/// routinely waits much longer. Asking for a very small value does not make it
/// run sooner, it just wastes the request.
static const NSTimeInterval kEmailOpsRefreshEarliestInterval = 15 * 60;

@interface EmailOpsBackgroundRefresh : NSObject
@end

@implementation EmailOpsBackgroundRefresh

+ (void)load {
    NSNotificationCenter *center = [NSNotificationCenter defaultCenter];

    // Registration is attempted twice, on purpose. `+load` runs before
    // `main()`, which is the widest possible margin ahead of the "must be
    // registered before the app finishes launching" rule — but it is also
    // before UIApplication exists, and BGTaskScheduler is not documented to
    // work that early. If it declines, fall back to the launch notification,
    // which is what an app with its own delegate would use. One of the two
    // holds on any OS version; the log line says which.
    if ([self registerLaunchHandler]) {
        NSLog(@"[emailops] background refresh registered at load");
    } else {
        [center addObserver:self
                   selector:@selector(applicationDidFinishLaunching:)
                       name:UIApplicationDidFinishLaunchingNotification
                     object:nil];
    }

    // Re-arm every time the app leaves the foreground. A submitted request is
    // consumed when it runs, so without this the app refreshes exactly once.
    [center addObserver:self
               selector:@selector(applicationDidEnterBackground:)
                   name:UIApplicationDidEnterBackgroundNotification
                 object:nil];
}

+ (BOOL)registerLaunchHandler {
    return [[BGTaskScheduler sharedScheduler] registerForTaskWithIdentifier:kEmailOpsRefreshTaskIdentifier
                                                                 usingQueue:nil
                                                              launchHandler:^(BGTask *task) {
                                                                  [self handleRefreshTask:(BGAppRefreshTask *)task];
                                                              }];
}

+ (void)applicationDidFinishLaunching:(NSNotification *)notification {
    if ([self registerLaunchHandler]) {
        NSLog(@"[emailops] background refresh registered at launch");
    } else {
        // Almost always a mismatch between this identifier and
        // BGTaskSchedulerPermittedIdentifiers in Info.plist — both are written
        // by scripts/ios_patch_project.sh, which asserts they exist.
        NSLog(@"[emailops] background refresh handler was refused: %@", kEmailOpsRefreshTaskIdentifier);
    }
}

+ (void)applicationDidEnterBackground:(NSNotification *)notification {
    [self scheduleNextRefresh];
}

+ (void)scheduleNextRefresh {
    BGAppRefreshTaskRequest *request =
        [[BGAppRefreshTaskRequest alloc] initWithIdentifier:kEmailOpsRefreshTaskIdentifier];
    request.earliestBeginDate = [NSDate dateWithTimeIntervalSinceNow:kEmailOpsRefreshEarliestInterval];

    NSError *error = nil;
    if ([[BGTaskScheduler sharedScheduler] submitTaskRequest:request error:&error]) {
        // Logged on success too, not just failure. "No error in the console" is
        // not evidence that scheduling worked — this line is, and it is the
        // only way to confirm the mechanism from outside the app, since pending
        // requests can only be enumerated from inside the process.
        NSLog(@"[emailops] background refresh scheduled, earliest %@", request.earliestBeginDate);
    } else {
        // Expected and harmless in the common cases: background refresh is off
        // in Settings, or a request is already queued. Logged rather than
        // ignored so a genuine misconfiguration (undeclared identifier) is
        // visible in the device console instead of silently never running.
        NSLog(@"[emailops] background refresh not scheduled: %@", error);
    }
}

+ (void)handleRefreshTask:(BGAppRefreshTask *)task {
    // Queue the successor first. Doing it after the work would skip the next
    // refresh entirely whenever this one is expired or crashes.
    [self scheduleNextRefresh];

    task.expirationHandler = ^{
        // Runs on an arbitrary thread with almost no time budget: flip a flag
        // and return. The Rust pass checks it between accounts.
        emailops_ios_expire_background_refresh();
    };

    NSLog(@"[emailops] background refresh window opened");
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
        // Blocking is intentional and safe here — a utility queue thread, not
        // the main thread, and not inside Rust's async runtime.
        bool success = emailops_ios_background_refresh();
        // The pair of window/closed lines is what tells you, from the device
        // console alone, whether iOS ever granted a window and whether the pass
        // fit inside it.
        NSLog(@"[emailops] background refresh window closed, success=%d", success);
        [task setTaskCompletedWithSuccess:success];
    });
}

@end
