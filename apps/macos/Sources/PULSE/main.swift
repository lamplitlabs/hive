import AppKit

MainActor.assumeIsolated {
    // An Objective-C exception crashes the app (with a report) instead of
    // leaving it running half set up.
    UserDefaults.standard.register(defaults: ["NSApplicationCrashOnExceptions": true])
    // A write to a pipe whose reader is gone (the watchdog's, say) returns
    // EPIPE instead of ending the app; children get the default back.
    signal(SIGPIPE, SIG_IGN)
    let delegate = AppDelegate()
    NSApplication.shared.delegate = delegate
    withExtendedLifetime(delegate) {
        NSApplication.shared.run()
    }
}
