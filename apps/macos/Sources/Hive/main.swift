import AppKit
import HiveKit
import os

MainActor.assumeIsolated {
    // An Objective-C exception crashes the app (with a report) instead of
    // leaving it running half set up.
    UserDefaults.standard.register(defaults: ["NSApplicationCrashOnExceptions": true])
    // A write to a pipe whose reader is gone (the watchdog's, say) returns
    // EPIPE instead of ending the app; children get the default back.
    signal(SIGPIPE, SIG_IGN)
    // The settings from before the rename (io.github.manishtiwari25.pulse)
    // move over before anything reads them: the supervisor reads HiveHome,
    // HubPort and HubURL as the delegate is made. A test run
    // (`-EphemeralRun YES`, scripts/check.sh) writes nothing.
    if !UserDefaults.standard.bool(forKey: "EphemeralRun") {
        let copied = DefaultsMigration.runForApp()
        if !copied.isEmpty {
            Logger(subsystem: AppIdentity.bundleID, category: "app")
                .info("settings carried over from \(DefaultsMigration.legacyBundleID, privacy: .public): \(copied.joined(separator: ", "), privacy: .public)")
        }
    }
    let delegate = AppDelegate()
    NSApplication.shared.delegate = delegate
    withExtendedLifetime(delegate) {
        NSApplication.shared.run()
    }
}
