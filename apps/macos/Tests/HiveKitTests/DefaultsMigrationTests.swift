import Foundation
import Testing
@testable import HiveKit

/// The first launch under io.github.manishtiwari25.hive carries the settings
/// over from io.github.manishtiwari25.pulse, once. The tests work on the
/// domains' contents, so nothing lands in ~/Library/Preferences.
@Suite struct DefaultsMigrationTests {
    static var legacy: [String: Any] { [
        "HiveHome": "/srv/other-home",
        "HubPort": 8791,
        "DeveloperExtras": true,
        "PulseHome": "/srv/old-home",
        "LastPage": "/tasks/",
    ] }

    @Test func theBundleIDIsHives() {
        #expect(AppIdentity.defaultBundleID == "io.github.manishtiwari25.hive")
        #expect(DefaultsMigration.legacyBundleID == "io.github.manishtiwari25.pulse")
    }

    @Test func copiesTheOldDomainOnTheFirstLaunch() throws {
        let writes = try #require(DefaultsMigration.carryOver(current: [:], legacy: Self.legacy))
        #expect(Set(writes.keys) == ["DeveloperExtras", "HiveHome", "HubPort", "LastPage", "PulseHome", DefaultsMigration.doneKey])
        #expect(writes[DefaultsMigration.doneKey] as? Bool == true)
        // What the app reads from them is what it read under the old id.
        let defaults = try #require(UserDefaults(suiteName: "io.github.manishtiwari25.hive.test-\(UUID().uuidString)"))
        defaults.register(defaults: writes)
        let overrides = SetupOverrides.from(defaults)
        #expect(overrides.hiveHome == "/srv/other-home")
        #expect(overrides.hubPort == 8791)
        #expect(defaults.bool(forKey: "DeveloperExtras"))
    }

    @Test func keepsWhatTheNewDomainAlreadyHolds() throws {
        let writes = try #require(DefaultsMigration.carryOver(current: ["HubPort": 9001], legacy: Self.legacy))
        #expect(writes["HubPort"] == nil)
        #expect(writes["HiveHome"] as? String == "/srv/other-home")
    }

    @Test func runsOnlyOnce() {
        // Done before: a setting the owner deleted since must not come back.
        let current: [String: Any] = [DefaultsMigration.doneKey: true, "HubPort": 9001]
        #expect(DefaultsMigration.carryOver(current: current, legacy: Self.legacy) == nil)
    }

    @Test func noOldDomainStillMarksItDone() throws {
        let writes = try #require(DefaultsMigration.carryOver(current: [:], legacy: nil))
        #expect(Array(writes.keys) == [DefaultsMigration.doneKey])
    }

    @Test func aBuildStillUnderTheOldIDOrOutsideABundleCopiesNothing() {
        #expect(DefaultsMigration.runForApp(bundleID: nil).isEmpty)
        #expect(DefaultsMigration.runForApp(bundleID: DefaultsMigration.legacyBundleID).isEmpty)
    }
}
