import Foundation

/// Carries the owner's settings over from the bundle identifier the app had
/// before the rename to Hive (`io.github.manishtiwari25.pulse`) to the one it
/// has now: HiveHome, HubPort, DeveloperExtras, the last page, the window's
/// frame - whatever `defaults write` or the app put in the old domain. It runs
/// once, on the first launch under the new identifier; a key the new domain
/// already holds is kept, and the old domain is left as it was (a build from
/// before the rename still finds its settings).
public enum DefaultsMigration {
    /// The bundle identifier before the rename.
    public static let legacyBundleID = "io.github.manishtiwari25.pulse"
    /// Set in the new domain once the copy ran, so it never runs twice: a key
    /// the owner deletes afterwards does not come back.
    public static let doneKey = "MigratedFromLegacyBundleID"

    /// What to write into the new domain, whose stored values are `current`,
    /// from `legacy` (the old domain's contents, nil when there is none): every
    /// old key the new domain does not hold, and the done mark. Nil once the
    /// copy ran. Only the new domain's stored values win over the old ones -
    /// not a registered default, not a `-Key value` argument of this launch.
    public static func carryOver(current: [String: Any], legacy: [String: Any]?) -> [String: Any]? {
        guard current[doneKey] == nil else { return nil }
        var writes = (legacy ?? [:]).filter { current[$0.key] == nil }
        writes[doneKey] = true
        return writes
    }

    /// Writes `carryOver` into `defaults`, whose own persistent domain is
    /// `domain`. Returns the old keys copied, sorted.
    @discardableResult
    public static func run(into defaults: UserDefaults, domain: String, legacy: [String: Any]?) -> [String] {
        guard let writes = carryOver(current: defaults.persistentDomain(forName: domain) ?? [:], legacy: legacy) else {
            return []
        }
        for (key, value) in writes { defaults.set(value, forKey: key) }
        return writes.keys.filter { $0 != doneKey }.sorted()
    }

    /// The app's own migration, into `UserDefaults.standard`: only inside a
    /// bundle whose identifier is no longer the legacy one (not under
    /// `swift run`, not in a build from before the rename).
    @discardableResult
    public static func runForApp(bundleID: String? = Bundle.main.bundleIdentifier) -> [String] {
        guard let bundleID, bundleID != legacyBundleID else { return [] }
        let defaults = UserDefaults.standard
        return run(into: defaults, domain: bundleID, legacy: defaults.persistentDomain(forName: legacyBundleID))
    }
}
