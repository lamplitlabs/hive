import Foundation

/// A launchd unit `hive service install` wrote: the plist, the /bin/sh
/// launcher it runs, and the environment it hands hive (PATH, HIVE_HOME,
/// LANG). crates/hive-runner/src/service.rs writes both files.
public struct ServiceUnit: Equatable, Sendable {
    public var label: String
    public var plist: URL
    public var launcher: URL?
    public var environment: [String: String]

    public init(label: String, plist: URL, launcher: URL?, environment: [String: String]) {
        self.label = label
        self.plist = plist
        self.launcher = launcher
        self.environment = environment
    }
}

/// Where this Mac's Hive lives and where its console is served, read the way
/// the services read it.
public struct HiveSetup: Equatable, Sendable {
    /// The console's port under the service, and the one the console's own
    /// "Keep Hive running" handover needs (crates/hive-hub/src/service.rs).
    public static let servicePort = 8765
    public static let hubLabel = "hive.hub"
    public static let daemonLabel = "hive.daemon"
    /// The product's name before the rename to Hive: an install from then has
    /// `pulse.hub`/`pulse.daemon` units, a PULSE_HOME and ~/.pulse, which the
    /// app still reads until `hive service install` moves them over.
    public static let legacyName = "pulse"
    public static let legacyHubLabel = "\(legacyName).hub"
    public static let legacyDaemonLabel = "\(legacyName).daemon"

    /// HIVE_HOME: the store, the logs, usually the binary.
    public var home: URL
    /// The hive the app runs. Never a copy inside the app bundle: self-update
    /// rewrites the binary in place, which would break the bundle's signature.
    public var binary: URL
    /// The console's launchd unit, once `hive service install` wrote one.
    public var hubService: ServiceUnit?
    /// The runner's unit; its PATH stands in when the console's is missing.
    public var daemonService: ServiceUnit?
    /// Where the console is served: the service's `--port`, or 8765.
    public var port: Int
    /// A console the owner pointed the app at (`HubURL`); the app never starts
    /// or stops it.
    public var hubOverride: URL?

    public init(
        home: URL, binary: URL, hubService: ServiceUnit? = nil, daemonService: ServiceUnit? = nil,
        port: Int = HiveSetup.servicePort, hubOverride: URL? = nil
    ) {
        self.home = home
        self.binary = binary
        self.hubService = hubService
        self.daemonService = daemonService
        self.port = port
        self.hubOverride = hubOverride
    }

    public var store: URL { home.appendingPathComponent("hive.sqlite") }
    /// The store's name before the rename; `hive` renames it to `store` the
    /// first time it opens the home.
    public var legacyStore: URL { home.appendingPathComponent("\(Self.legacyName).sqlite") }
    public var logs: URL { home.appendingPathComponent("logs", isDirectory: true) }
    /// The service writes the console's output here (launchd StandardOutPath).
    public var serviceLog: URL { logs.appendingPathComponent("hub.log") }
    /// A console the app started itself writes here.
    public var appHubLog: URL { logs.appendingPathComponent("hub-app.log") }
    public var consoleURL: URL { hubOverride ?? Self.consoleURL(port: port) }

    /// Always 127.0.0.1, never localhost: the hub listens on IPv4 only and the
    /// console's storage belongs to this exact origin.
    public static func consoleURL(port: Int) -> URL {
        URL(string: "http://127.0.0.1:\(port)/")!
    }

    /// The environment for a `hive` the app runs: the service's PATH (a GUI
    /// app's own PATH is /usr/bin:/bin:/usr/sbin:/sbin, too little for the
    /// tools a conversation runs), this HIVE_HOME, and a UTF-8 locale.
    public func childEnvironment(base: [String: String], loginPath: String?) -> [String: String] {
        var env = base
        let unit = hubService?.environment ?? daemonService?.environment ?? [:]
        env["PATH"] = unit["PATH"] ?? loginPath ?? Self.fallbackPath(userHome: base["HOME"])
        env["HIVE_HOME"] = home.path
        env["LANG"] = unit["LANG"] ?? base["LANG"] ?? "en_US.UTF-8"
        // Belongs to the app's own launch; a child that keeps it would be
        // taken for part of the app bundle by CoreFoundation.
        env.removeValue(forKey: "__CFBundleIdentifier")
        return env
    }

    public static func fallbackPath(userHome: String?) -> String {
        var dirs: [String] = []
        if let home = userHome, !home.isEmpty {
            dirs += ["\(home)/.hive/bin", "\(home)/.cargo/bin", "\(home)/.local/bin"]
        }
        dirs += ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"]
        return dirs.joined(separator: ":")
    }
}

/// The owner's overrides, from the app's defaults (`defaults write
/// dev.hivefleet.hive HiveHome ~/other`, or `-HiveHome` on the
/// command line).
public struct SetupOverrides: Equatable, Sendable {
    public var hiveHome: String?
    public var hiveBinary: String?
    public var hubURL: String?
    public var hubPort: Int?

    public init(hiveHome: String? = nil, hiveBinary: String? = nil, hubURL: String? = nil, hubPort: Int? = nil) {
        self.hiveHome = hiveHome
        self.hiveBinary = hiveBinary
        self.hubURL = hubURL
        self.hubPort = hubPort
    }

    public static func from(_ defaults: UserDefaults) -> SetupOverrides {
        func text(_ key: String) -> String? {
            guard let value = defaults.string(forKey: key)?.trimmingCharacters(in: .whitespaces), !value.isEmpty else {
                return nil
            }
            return value
        }
        let port = defaults.integer(forKey: "HubPort")
        // PulseHome/PulseBinary: the keys from before the rename, still honoured.
        return SetupOverrides(
            hiveHome: text("HiveHome") ?? text("PulseHome"),
            hiveBinary: text("HiveBinary") ?? text("PulseBinary"),
            hubURL: text("HubURL"),
            hubPort: (1...65535).contains(port) ? port : nil
        )
    }
}

public enum SetupReader {
    /// Reads the setup from the launchd units, the environment and the
    /// overrides, in that order of weight: an override beats a unit, a unit
    /// beats the environment, and ~/.hive is the default.
    public static func read(
        overrides: SetupOverrides,
        environment: [String: String],
        userHome: URL,
        launchAgents: URL? = nil,
        fileManager: FileManager = .default
    ) -> HiveSetup {
        let agents = launchAgents ?? userHome.appendingPathComponent("Library/LaunchAgents", isDirectory: true)
        // A unit from before the rename stands in while there is no Hive one.
        var hub = readUnit(label: HiveSetup.hubLabel, in: agents, fileManager: fileManager)
            ?? readUnit(label: HiveSetup.legacyHubLabel, in: agents, fileManager: fileManager)
        var daemon = readUnit(label: HiveSetup.daemonLabel, in: agents, fileManager: fileManager)
            ?? readUnit(label: HiveSetup.legacyDaemonLabel, in: agents, fileManager: fileManager)

        func homeVariable(_ vars: [String: String]?) -> String? {
            guard let vars else { return nil }
            return (vars["HIVE_HOME"] ?? vars["PULSE_HOME"]).flatMap { $0.isEmpty ? nil : $0 }
        }
        let homePath = overrides.hiveHome
            ?? homeVariable(hub?.environment)
            ?? homeVariable(daemon?.environment)
            ?? homeVariable(environment)
            ?? defaultHome(userHome: userHome, fileManager: fileManager).path
        let home = URL(fileURLWithPath: expandTilde(homePath, userHome: userHome), isDirectory: true)

        // Units that serve another HIVE_HOME's store are not this home's service.
        func servesHome(_ unit: ServiceUnit?) -> Bool {
            guard let unitHome = homeVariable(unit?.environment) else { return true }
            let resolved = URL(fileURLWithPath: expandTilde(unitHome, userHome: userHome), isDirectory: true)
            return resolved.standardizedFileURL.path == home.standardizedFileURL.path
        }
        if !servesHome(hub) { hub = nil }
        if !servesHome(daemon) { daemon = nil }

        let launcherText = hub?.launcher.flatMap { try? String(contentsOf: $0, encoding: .utf8) }
        let binary: URL
        if let chosen = overrides.hiveBinary {
            binary = URL(fileURLWithPath: expandTilde(chosen, userHome: userHome))
        } else if let text = launcherText, let path = binaryPath(fromLauncher: text) {
            binary = URL(fileURLWithPath: path)
        } else {
            binary = defaultBinary(home: home, userHome: userHome, fileManager: fileManager)
        }

        // The service's console is where launchd serves it; HubPort names the
        // port for a console the app starts itself, when there is no service.
        let port: Int
        if hub != nil {
            port = launcherText.flatMap(port(fromLauncher:)) ?? HiveSetup.servicePort
        } else {
            port = overrides.hubPort ?? HiveSetup.servicePort
        }

        var hubOverride: URL?
        if let text = overrides.hubURL, let url = URL(string: text.hasSuffix("/") ? text : text + "/"),
           let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https", url.host != nil {
            hubOverride = url
        }

        return HiveSetup(
            home: home, binary: binary, hubService: hub, daemonService: daemon, port: port, hubOverride: hubOverride
        )
    }

    /// The unit's plist, the launcher its ProgramArguments run and its
    /// EnvironmentVariables; nil when the plist is missing or unreadable.
    public static func readUnit(label: String, in agents: URL, fileManager: FileManager = .default) -> ServiceUnit? {
        let plist = agents.appendingPathComponent("\(label).plist")
        guard fileManager.fileExists(atPath: plist.path),
              let data = try? Data(contentsOf: plist),
              let object = try? PropertyListSerialization.propertyList(from: data, format: nil),
              let dict = object as? [String: Any] else {
            return nil
        }
        let arguments = dict["ProgramArguments"] as? [String] ?? []
        // ["/bin/sh", "<home>/bin/hive-hub.sh"]; a unit that runs hive
        // directly has no launcher to read.
        let launcher = arguments.first(where: { $0.hasSuffix(".sh") }).map { URL(fileURLWithPath: $0) }
        let environment = (dict["EnvironmentVariables"] as? [String: Any] ?? [:])
            .compactMapValues { $0 as? String }
        return ServiceUnit(label: label, plist: plist, launcher: launcher, environment: environment)
    }

    /// The `--port N` of the `hive hub` line in the service's launcher:
    /// `/Users/me/.hive/bin/hive hub --port 8765 --no-browser --restart-on-change &`.
    public static func port(fromLauncher text: String) -> Int? {
        for words in hubLines(text) {
            for (i, word) in words.enumerated() {
                var value: String?
                if word == "--port" || word == "-port", i + 1 < words.count {
                    value = words[i + 1]
                } else if word.hasPrefix("--port=") {
                    value = String(word.dropFirst("--port=".count))
                }
                if let value, let port = Int(value), (1...65535).contains(port) {
                    return port
                }
            }
        }
        return nil
    }

    /// The program on the launcher's `hive hub` line: the binary the service
    /// runs. `hive service install` quotes a path with spaces the shell's
    /// way (`'/Volumes/My Disk/.hive/bin/hive' hub …`).
    public static func binaryPath(fromLauncher text: String) -> String? {
        for words in hubLines(text) {
            if let path = words.first, path.hasPrefix("/") { return path }
        }
        return nil
    }

    /// The words of the lines that run the console: a word `hub` after the
    /// program, outside comments.
    static func hubLines(_ text: String) -> [[String]] {
        text.split(whereSeparator: \.isNewline).map { shellWords(String($0)) }.filter { words in
            words.dropFirst().contains("hub")
        }
    }

    /// A line's words the way /bin/sh reads them: single quotes keep
    /// everything, double quotes keep spaces, a backslash keeps the next
    /// character, and `#` at the start of a word begins a comment.
    public static func shellWords(_ line: String) -> [String] {
        var words: [String] = []
        var current = ""
        var inWord = false
        var quote: Character?
        var escaped = false
        for character in line {
            if escaped {
                current.append(character)
                escaped = false
                continue
            }
            if let open = quote {
                if character == open {
                    quote = nil
                } else if character == "\\", open == "\"" {
                    escaped = true
                } else {
                    current.append(character)
                }
                continue
            }
            switch character {
            case "'", "\"":
                quote = character
                inWord = true
            case "\\":
                escaped = true
                inWord = true
            case " ", "\t":
                if inWord {
                    words.append(current)
                    current = ""
                    inWord = false
                }
            case "#" where !inWord:
                return words
            default:
                current.append(character)
                inWord = true
            }
        }
        if inWord { words.append(current) }
        return words
    }

    /// ~/.hive, or ~/.pulse while only the home from before the rename
    /// exists (the same rule as `hive_store::default_home_in`).
    static func defaultHome(userHome: URL, fileManager: FileManager) -> URL {
        let hive = userHome.appendingPathComponent(".hive", isDirectory: true)
        let legacy = userHome.appendingPathComponent(".\(HiveSetup.legacyName)", isDirectory: true)
        var isDir: ObjCBool = false
        if fileManager.fileExists(atPath: hive.path) { return hive }
        if fileManager.fileExists(atPath: legacy.path, isDirectory: &isDir), isDir.boolValue { return legacy }
        return hive
    }

    /// <home>/bin/hive (`cargo install --root ~/.hive`), else the first
    /// other install that exists, else <home>/bin/hive so the app can say
    /// where it looked.
    static func defaultBinary(home: URL, userHome: URL, fileManager: FileManager) -> URL {
        let primary = home.appendingPathComponent("bin/hive")
        let candidates = [
            primary,
            userHome.appendingPathComponent(".cargo/bin/hive"),
            URL(fileURLWithPath: "/opt/homebrew/bin/hive"),
            URL(fileURLWithPath: "/usr/local/bin/hive"),
        ]
        return candidates.first(where: { fileManager.isExecutableFile(atPath: $0.path) }) ?? primary
    }

    static func expandTilde(_ path: String, userHome: URL) -> String {
        if path == "~" { return userHome.path }
        if path.hasPrefix("~/") { return userHome.appendingPathComponent(String(path.dropFirst(2))).path }
        return path
    }
}
