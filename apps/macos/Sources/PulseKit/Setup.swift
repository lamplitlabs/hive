import Foundation

/// A launchd unit `pulse service install` wrote: the plist, the /bin/sh
/// launcher it runs, and the environment it hands pulse (PATH, PULSE_HOME,
/// LANG). crates/pulse-runner/src/service.rs writes both files.
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

/// Where this Mac's PULSE lives and where its console is served, read the way
/// the services read it.
public struct PulseSetup: Equatable, Sendable {
    /// The console's port under the service, and the one the console's own
    /// "Keep PULSE running" handover needs (crates/pulse-hub/src/service.rs).
    public static let servicePort = 8765
    public static let hubLabel = "pulse.hub"
    public static let daemonLabel = "pulse.daemon"

    /// PULSE_HOME: the store, the logs, usually the binary.
    public var home: URL
    /// The pulse the app runs. Never a copy inside the app bundle: self-update
    /// rewrites the binary in place, which would break the bundle's signature.
    public var binary: URL
    /// The console's launchd unit, once `pulse service install` wrote one.
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
        port: Int = PulseSetup.servicePort, hubOverride: URL? = nil
    ) {
        self.home = home
        self.binary = binary
        self.hubService = hubService
        self.daemonService = daemonService
        self.port = port
        self.hubOverride = hubOverride
    }

    public var store: URL { home.appendingPathComponent("pulse.sqlite") }
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

    /// The environment for a `pulse` the app runs: the service's PATH (a GUI
    /// app's own PATH is /usr/bin:/bin:/usr/sbin:/sbin, too little for the
    /// tools a conversation runs), this PULSE_HOME, and a UTF-8 locale.
    public func childEnvironment(base: [String: String], loginPath: String?) -> [String: String] {
        var env = base
        let unit = hubService?.environment ?? daemonService?.environment ?? [:]
        env["PATH"] = unit["PATH"] ?? loginPath ?? Self.fallbackPath(userHome: base["HOME"])
        env["PULSE_HOME"] = home.path
        env["LANG"] = unit["LANG"] ?? base["LANG"] ?? "en_US.UTF-8"
        // Belongs to the app's own launch; a child that keeps it would be
        // taken for part of the app bundle by CoreFoundation.
        env.removeValue(forKey: "__CFBundleIdentifier")
        return env
    }

    static func fallbackPath(userHome: String?) -> String {
        var dirs: [String] = []
        if let home = userHome, !home.isEmpty {
            dirs += ["\(home)/.pulse/bin", "\(home)/.cargo/bin", "\(home)/.local/bin"]
        }
        dirs += ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"]
        return dirs.joined(separator: ":")
    }
}

/// The owner's overrides, from the app's defaults (`defaults write
/// io.github.manishtiwari25.pulse PulseHome ~/other`, or `-PulseHome` on the
/// command line).
public struct SetupOverrides: Equatable, Sendable {
    public var pulseHome: String?
    public var pulseBinary: String?
    public var hubURL: String?
    public var hubPort: Int?

    public init(pulseHome: String? = nil, pulseBinary: String? = nil, hubURL: String? = nil, hubPort: Int? = nil) {
        self.pulseHome = pulseHome
        self.pulseBinary = pulseBinary
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
        return SetupOverrides(
            pulseHome: text("PulseHome"),
            pulseBinary: text("PulseBinary"),
            hubURL: text("HubURL"),
            hubPort: (1...65535).contains(port) ? port : nil
        )
    }
}

public enum SetupReader {
    /// Reads the setup from the launchd units, the environment and the
    /// overrides, in that order of weight: an override beats a unit, a unit
    /// beats the environment, and ~/.pulse is the default.
    public static func read(
        overrides: SetupOverrides,
        environment: [String: String],
        userHome: URL,
        launchAgents: URL? = nil,
        fileManager: FileManager = .default
    ) -> PulseSetup {
        let agents = launchAgents ?? userHome.appendingPathComponent("Library/LaunchAgents", isDirectory: true)
        var hub = readUnit(label: PulseSetup.hubLabel, in: agents, fileManager: fileManager)
        var daemon = readUnit(label: PulseSetup.daemonLabel, in: agents, fileManager: fileManager)

        let homePath = overrides.pulseHome
            ?? hub?.environment["PULSE_HOME"]
            ?? daemon?.environment["PULSE_HOME"]
            ?? environment["PULSE_HOME"].flatMap { $0.isEmpty ? nil : $0 }
            ?? userHome.appendingPathComponent(".pulse").path
        let home = URL(fileURLWithPath: expandTilde(homePath, userHome: userHome), isDirectory: true)

        // Units that serve another PULSE_HOME's store are not this home's service.
        func servesHome(_ unit: ServiceUnit?) -> Bool {
            guard let unitHome = unit?.environment["PULSE_HOME"] else { return true }
            let resolved = URL(fileURLWithPath: expandTilde(unitHome, userHome: userHome), isDirectory: true)
            return resolved.standardizedFileURL.path == home.standardizedFileURL.path
        }
        if !servesHome(hub) { hub = nil }
        if !servesHome(daemon) { daemon = nil }

        let launcherText = hub?.launcher.flatMap { try? String(contentsOf: $0, encoding: .utf8) }
        let binary: URL
        if let chosen = overrides.pulseBinary {
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
            port = launcherText.flatMap(port(fromLauncher:)) ?? PulseSetup.servicePort
        } else {
            port = overrides.hubPort ?? PulseSetup.servicePort
        }

        var hubOverride: URL?
        if let text = overrides.hubURL, let url = URL(string: text.hasSuffix("/") ? text : text + "/"),
           let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https", url.host != nil {
            hubOverride = url
        }

        return PulseSetup(
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
        // ["/bin/sh", "<home>/bin/pulse-hub.sh"]; a unit that runs pulse
        // directly has no launcher to read.
        let launcher = arguments.first(where: { $0.hasSuffix(".sh") }).map { URL(fileURLWithPath: $0) }
        let environment = (dict["EnvironmentVariables"] as? [String: Any] ?? [:])
            .compactMapValues { $0 as? String }
        return ServiceUnit(label: label, plist: plist, launcher: launcher, environment: environment)
    }

    /// The `--port N` of the `pulse hub` line in the service's launcher:
    /// `/Users/me/.pulse/bin/pulse hub --port 8765 --no-browser --restart-on-change &`.
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

    /// The program on the launcher's `pulse hub` line: the binary the service
    /// runs. `pulse service install` quotes a path with spaces the shell's
    /// way (`'/Volumes/My Disk/.pulse/bin/pulse' hub …`).
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

    /// <home>/bin/pulse (`cargo install --root ~/.pulse`), else the first
    /// other install that exists, else <home>/bin/pulse so the app can say
    /// where it looked.
    static func defaultBinary(home: URL, userHome: URL, fileManager: FileManager) -> URL {
        let primary = home.appendingPathComponent("bin/pulse")
        let candidates = [
            primary,
            userHome.appendingPathComponent(".cargo/bin/pulse"),
            URL(fileURLWithPath: "/opt/homebrew/bin/pulse"),
            URL(fileURLWithPath: "/usr/local/bin/pulse"),
        ]
        return candidates.first(where: { fileManager.isExecutableFile(atPath: $0.path) }) ?? primary
    }

    static func expandTilde(_ path: String, userHome: URL) -> String {
        if path == "~" { return userHome.path }
        if path.hasPrefix("~/") { return userHome.appendingPathComponent(String(path.dropFirst(2))).path }
        return path
    }
}
