import Foundation

/// Carries the owner's settings over from the bundle identifiers the app had
/// before (`io.github.manishtiwari25.pulse` before the rename to Hive, then
/// `io.github.manishtiwari25.hive` before the move to hivefleet.dev) to the one
/// it has now: HiveHome, HubPort, DeveloperExtras, the last page, the window's
/// frame - whatever `defaults write` or the app put in an old domain. It runs
/// once, on the first launch under the new identifier; a key the new domain
/// already holds is kept, a key the newer old domain holds wins over the older
/// one's, and the old domains are left as they were (a build from before the
/// rename still finds its settings).
public enum DefaultsMigration {
    /// The bundle identifiers before this one, most recent first.
    public static let legacyBundleIDs = ["io.github.manishtiwari25.hive", "io.github.manishtiwari25.pulse"]
    /// The most recent bundle identifier before this one.
    public static var legacyBundleID: String { legacyBundleIDs[0] }
    /// Set in the new domain once the copy ran, so it never runs twice: a key
    /// the owner deletes afterwards does not come back.
    public static let doneKey = "MigratedFromLegacyBundleID"

    /// What to write into the new domain, whose stored values are `current`,
    /// from `legacy` (the old domains' contents, most recent first; nil for a
    /// domain there is none of): every old key the new domain does not hold,
    /// taken from the most recent old domain that has it, and the done mark.
    /// Nil once the copy ran. Only the new domain's stored values win over the
    /// old ones - not a registered default, not a `-Key value` argument of
    /// this launch.
    public static func carryOver(current: [String: Any], legacy: [[String: Any]?]) -> [String: Any]? {
        guard current[doneKey] == nil else { return nil }
        var writes: [String: Any] = [:]
        for domain in legacy {
            for (key, value) in domain ?? [:] where current[key] == nil && writes[key] == nil {
                writes[key] = value
            }
        }
        writes[doneKey] = true
        return writes
    }

    /// `carryOver` from one old domain.
    public static func carryOver(current: [String: Any], legacy: [String: Any]?) -> [String: Any]? {
        carryOver(current: current, legacy: [legacy])
    }

    /// Writes `carryOver` into `defaults`, whose own persistent domain is
    /// `domain`. Returns the old keys copied, sorted.
    @discardableResult
    public static func run(into defaults: UserDefaults, domain: String, legacy: [[String: Any]?]) -> [String] {
        guard let writes = carryOver(current: defaults.persistentDomain(forName: domain) ?? [:], legacy: legacy) else {
            return []
        }
        for (key, value) in writes { defaults.set(value, forKey: key) }
        return writes.keys.filter { $0 != doneKey }.sorted()
    }

    /// `run` from one old domain.
    @discardableResult
    public static func run(into defaults: UserDefaults, domain: String, legacy: [String: Any]?) -> [String] {
        run(into: defaults, domain: domain, legacy: [legacy])
    }

    /// The app's own migration, into `UserDefaults.standard`: only inside a
    /// bundle whose identifier is none of the legacy ones (not under
    /// `swift run`, not in a build from before the rename).
    @discardableResult
    public static func runForApp(bundleID: String? = Bundle.main.bundleIdentifier) -> [String] {
        guard let bundleID, !legacyBundleIDs.contains(bundleID) else { return [] }
        let defaults = UserDefaults.standard
        return run(into: defaults, domain: bundleID, legacy: legacyBundleIDs.map { defaults.persistentDomain(forName: $0) })
    }
}
