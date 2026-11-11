import Foundation
import Testing
@testable import HiveKit

/// The first launch under dev.hivefleet.hive carries the settings over from
/// the app's earlier identifiers (io.github.manishtiwari25.hive, and before
/// the rename io.github.manishtiwari25.pulse), once. The tests work on the
/// domains' contents, so nothing lands in ~/Library/Preferences.
@Suite struct DefaultsMigrationTests {
    /// The domain from before the rename to Hive, the oldest.
    static var legacy: [String: Any] { [
        "HiveHome": "/srv/other-home",
        "HubPort": 8791,
        "DeveloperExtras": true,
        "PulseHome": "/srv/old-home",
        "LastPage": "/tasks/",
    ] }

    /// The domain from before the move to hivefleet.dev, the more recent.
    static var recentLegacy: [String: Any] { [
        "HiveHome": "/srv/newer-home",
        "LastPage": "/chat/",
        "HiveHandHome": "/srv/hand",
    ] }

    @Test func theBundleIDIsHives() {
        #expect(AppIdentity.defaultBundleID == "dev.hivefleet.hive")
        #expect(DefaultsMigration.legacyBundleIDs == ["io.github.manishtiwari25.hive", "io.github.manishtiwari25.pulse"])
        #expect(DefaultsMigration.legacyBundleID == "io.github.manishtiwari25.hive")
    }

    @Test func copiesTheOldDomainOnTheFirstLaunch() throws {
        let writes = try #require(DefaultsMigration.carryOver(current: [:], legacy: Self.legacy))
        #expect(Set(writes.keys) == ["DeveloperExtras", "HiveHome", "HubPort", "LastPage", "PulseHome", DefaultsMigration.doneKey])
        #expect(writes[DefaultsMigration.doneKey] as? Bool == true)
        // What the app reads from them is what it read under the old id.
        let defaults = try #require(UserDefaults(suiteName: "dev.hivefleet.hive.test-\(UUID().uuidString)"))
        defaults.register(defaults: writes)
        let overrides = SetupOverrides.from(defaults)
        #expect(overrides.hiveHome == "/srv/other-home")
        #expect(overrides.hubPort == 8791)
        #expect(defaults.bool(forKey: "DeveloperExtras"))
    }

    @Test func copiesBothOldDomainsAndTheNewerOneWins() throws {
        let writes = try #require(DefaultsMigration.carryOver(current: [:], legacy: [Self.recentLegacy, Self.legacy]))
        // Every key of either domain comes over...
        #expect(Set(writes.keys) == [
            "DeveloperExtras", "HiveHome", "HubPort", "LastPage", "PulseHome", "HiveHandHome", DefaultsMigration.doneKey,
        ])
        // ...the more recent domain's value where both have the key...
        #expect(writes["HiveHome"] as? String == "/srv/newer-home")
        #expect(writes["LastPage"] as? String == "/chat/")
        // ...and the older one's where only it has it.
        #expect(writes["HubPort"] as? Int == 8791)
        #expect(writes["PulseHome"] as? String == "/srv/old-home")
        #expect(writes["HiveHandHome"] as? String == "/srv/hand")
    }

    @Test func aMissingNewerDomainFallsBackToTheOlder() throws {
        let writes = try #require(DefaultsMigration.carryOver(current: [:], legacy: [nil, Self.legacy]))
        #expect(writes["HiveHome"] as? String == "/srv/other-home")
        #expect(writes["HubPort"] as? Int == 8791)
    }

    @Test func keepsWhatTheNewDomainAlreadyHolds() throws {
        let writes = try #require(DefaultsMigration.carryOver(current: ["HubPort": 9001], legacy: Self.legacy))
        #expect(writes["HubPort"] == nil)
        #expect(writes["HiveHome"] as? String == "/srv/other-home")
        // Also against a key both old domains hold.
        let both = try #require(DefaultsMigration.carryOver(current: ["HiveHome": "/srv/mine"], legacy: [Self.recentLegacy, Self.legacy]))
        #expect(both["HiveHome"] == nil)
        #expect(both["LastPage"] as? String == "/chat/")
    }

    @Test func runsOnlyOnce() {
        // Done before: a setting the owner deleted since must not come back.
        let current: [String: Any] = [DefaultsMigration.doneKey: true, "HubPort": 9001]
        #expect(DefaultsMigration.carryOver(current: current, legacy: Self.legacy) == nil)
        #expect(DefaultsMigration.carryOver(current: current, legacy: [Self.recentLegacy, Self.legacy]) == nil)
    }

    @Test func noOldDomainStillMarksItDone() throws {
        let writes = try #require(DefaultsMigration.carryOver(current: [:], legacy: nil))
        #expect(Array(writes.keys) == [DefaultsMigration.doneKey])
        let none = try #require(DefaultsMigration.carryOver(current: [:], legacy: [nil, nil]))
        #expect(Array(none.keys) == [DefaultsMigration.doneKey])
    }

    @Test func aBuildStillUnderAnOldIDOrOutsideABundleCopiesNothing() {
        #expect(DefaultsMigration.runForApp(bundleID: nil).isEmpty)
        for legacy in DefaultsMigration.legacyBundleIDs {
            #expect(DefaultsMigration.runForApp(bundleID: legacy).isEmpty)
        }
    }
}
