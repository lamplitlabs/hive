import Foundation

/// What joining a brain as a hand needs on this Mac (ADR-070 step 2): the
/// hand's own directory and the `hive-hand` that enrols it and dials out.
/// crates/hive-hand-core/src/dial.rs is the protocol; the app shells out to
/// `hive-hand join` and `hive-hand dial` rather than speaking it itself, so
/// the device key, device.json and the dial loop live in one place.
public struct HandSetup: Equatable, Sendable {
    /// HIVE_HAND_HOME: the device key, device.json, the dial log.
    public var home: URL
    /// The hive-hand the app joins with; nil when there is none.
    public var binary: URL?

    public init(home: URL, binary: URL?) {
        self.home = home
        self.binary = binary
    }

    public var deviceFile: URL { home.appendingPathComponent("device.json") }
    /// `hive-hand dial`'s output under the launchd agent.
    public var dialLog: URL { home.appendingPathComponent("dial.log") }
    /// Where the agent's copy of hive-hand lives when the app's own is
    /// inside Hive.app: moving or replacing the app must not stop the hand.
    public var installedBinary: URL { home.appendingPathComponent("bin/hive-hand") }

    /// A path with the user's home as `~`, for the screen.
    public static func tilde(_ path: String, userHome: String = FileManager.default.homeDirectoryForCurrentUser.path) -> String {
        path.hasPrefix(userHome + "/") ? "~" + path.dropFirst(userHome.count) : path
    }

    /// The device this Mac joined as, when it did.
    public func device() -> HandDevice? {
        guard let data = try? Data(contentsOf: deviceFile) else { return nil }
        return try? JSONDecoder().decode(HandDevice.self, from: data)
    }

    /// Reads the setup: `HiveHandHome` / `HiveHandBinary` overrides first,
    /// then HIVE_HAND_HOME, then `~/.hive-hand`; the binary is the override,
    /// the one inside the app (Contents/Helpers/hive-hand), the hand's own
    /// copy, then the usual install places.
    public static func read(
        homeOverride: String?,
        binaryOverride: String?,
        environment: [String: String],
        userHome: URL,
        bundle: URL?,
        isExecutable: (URL) -> Bool = { FileManager.default.isExecutableFile(atPath: $0.path) }
    ) -> HandSetup {
        let homePath = homeOverride.flatMap { $0.isEmpty ? nil : $0 }
            ?? environment["HIVE_HAND_HOME"].flatMap { $0.isEmpty ? nil : $0 }
            ?? userHome.appendingPathComponent(".hive-hand").path
        let home = URL(fileURLWithPath: SetupReader.expandTilde(homePath, userHome: userHome), isDirectory: true)
        if let chosen = binaryOverride.flatMap({ $0.isEmpty ? nil : $0 }) {
            return HandSetup(home: home, binary: URL(fileURLWithPath: SetupReader.expandTilde(chosen, userHome: userHome)))
        }
        var candidates: [URL] = []
        if let bundle {
            candidates.append(bundle.appendingPathComponent("Contents/Helpers/hive-hand"))
        }
        candidates += [
            home.appendingPathComponent("bin/hive-hand"),
            userHome.appendingPathComponent(".hive/bin/hive-hand"),
            userHome.appendingPathComponent(".cargo/bin/hive-hand"),
            URL(fileURLWithPath: "/opt/homebrew/bin/hive-hand"),
            URL(fileURLWithPath: "/usr/local/bin/hive-hand"),
        ]
        return HandSetup(home: home, binary: candidates.first(where: isExecutable))
    }
}

/// device.json as `hive-hand join` writes it (hive-hand-core dial.rs `Device`).
public struct HandDevice: Codable, Equatable, Sendable {
    public var brain: String
    public var device: String
    public var executor: String
    public var workdir: String

    public init(brain: String, device: String, executor: String, workdir: String) {
        self.brain = brain
        self.device = device
        self.executor = executor
        self.workdir = workdir
    }
}

/// What the owner typed on the join screen, made into a brain and a code.
public struct JoinInput: Equatable, Sendable {
    public var brain: URL
    /// The six words joined by dashes, as the brain normalises them.
    public var code: String

    public enum Problem: Error, Equatable, Sendable {
        case noBrain
        case badBrain
        case badCode(words: Int)

        /// The line the join screen shows under the fields.
        public var words: String {
            switch self {
            case .noBrain:
                return "Type the brain's address: the start of the Machines page's address, like https://hive.example.com."
            case .badBrain:
                return "That is not a web address. Type the brain's, like https://hive.example.com."
            case .badCode(0):
                return "Type the six-word join code the Machines page shows under Add a machine → Run one command on it."
            case .badCode(let words):
                return "A join code is six words; that is \(words). Copy it again from the Machines page."
            }
        }
    }

    /// Reads the two fields. The code field also takes the whole line the
    /// Machines page shows (`hive-hand join --brain URL CODE`); its brain
    /// fills an empty brain field.
    public static func parse(brain brainText: String, code codeText: String) -> Result<JoinInput, Problem> {
        var brainTyped = brainText.trimmingCharacters(in: .whitespacesAndNewlines)
        var tokens = codeText.split(whereSeparator: \.isWhitespace).map(String.init)
        if let at = tokens.firstIndex(of: "--brain") {
            if at + 1 < tokens.count {
                if brainTyped.isEmpty { brainTyped = tokens[at + 1] }
                tokens.remove(at: at + 1)
            }
            tokens.remove(at: at)
        } else if let at = tokens.firstIndex(where: { $0.hasPrefix("--brain=") }) {
            if brainTyped.isEmpty { brainTyped = String(tokens[at].dropFirst("--brain=".count)) }
            tokens.remove(at: at)
        }
        if tokens.first == "hive-hand" { tokens.removeFirst() }
        if tokens.first == "join" { tokens.removeFirst() }
        let words = tokens.joined(separator: " ").lowercased()
            .split(whereSeparator: { !("a"..."z").contains($0) })
            .map(String.init)
        guard !brainTyped.isEmpty else { return .failure(.noBrain) }
        guard let brain = brainURL(brainTyped) else { return .failure(.badBrain) }
        guard words.count == 6 else { return .failure(.badCode(words: words.count)) }
        return .success(JoinInput(brain: brain, code: words.joined(separator: "-")))
    }

    /// A brain address as typed: a full http(s) address, or a host the way
    /// the browser panel reads one (https, http for this Mac's own and local
    /// names); no trailing slash, as `hive-hand join --brain` takes it.
    public static func brainURL(_ text: String) -> URL? {
        guard let url = TypedAddress.url(text), url.host != nil else { return nil }
        var spelled = url.absoluteString
        while spelled.hasSuffix("/") { spelled.removeLast() }
        return URL(string: spelled)
    }
}

/// How a join ended, in the words the join screen shows.
public enum JoinOutcome: Equatable, Sendable {
    /// device.json is written: this Mac is a hand of that brain.
    case joined(HandDevice)
    /// The brain said no (403: a wrong, used or lapsed code): its own words.
    case refused(status: Int, why: String, fix: String)
    /// The brain did not answer.
    case unreachable(String)
    /// Anything else: hive-hand's own words.
    case failed(String)

    /// The line under the fields.
    public var words: String {
        switch self {
        case .joined(let device): return "Joined \(device.brain) as \(device.executor)."
        case .refused(_, let why, let fix): return fix.isEmpty ? "\(why)." : "\(why). \(fix)"
        case .unreachable(let why): return "The brain did not answer: \(why)"
        case .failed(let why): return why
        }
    }
}

/// `hive-hand join`, run the way the app runs it.
public enum HandJoin {
    /// `join --brain URL CODE`: the join only; the launchd agent dials after it.
    public static func arguments(_ input: JoinInput) -> [String] {
        ["join", "--brain", input.brain.absoluteString, input.code]
    }

    /// The child's environment: the hand's home, and HIVE_HAND_JOIN_ONLY so
    /// `hive-hand join` returns once enrolled instead of dialling in itself.
    public static func environment(base: [String: String], home: URL) -> [String: String] {
        var env = base
        env["HIVE_HAND_HOME"] = home.path
        env["HIVE_HAND_JOIN_ONLY"] = "1"
        env.removeValue(forKey: "__CFBundleIdentifier")
        if (env["PATH"] ?? "").isEmpty {
            // curl and ssh-keygen ship with macOS; hive-hand finds them here.
            env["PATH"] = "/usr/bin:/bin:/usr/sbin:/sbin"
        }
        return env
    }

    /// Joins: runs hive-hand and reads what it left (device.json) and said.
    public static func run(_ input: JoinInput, setup: HandSetup, base: [String: String], timeout: TimeInterval = 90) async -> JoinOutcome {
        guard let binary = setup.binary else {
            return .failed("There is no hive-hand on this Mac. Build Hive.app with apps/macos/scripts/build.sh (it bundles one), or install hive-hand into ~/.hive/bin.")
        }
        try? FileManager.default.createDirectory(at: setup.home, withIntermediateDirectories: true)
        let result = await Command.run(
            binary.path, arguments(input),
            environment: environment(base: base, home: setup.home),
            currentDirectory: setup.home,
            timeout: timeout
        )
        return outcome(result, device: setup.device())
    }

    /// What the run means. hive-hand says `join refused (403): WHY - FIX`
    /// when the brain refuses (hive-hand-core dial.rs `Answer::ok`), and
    /// `curl exit N: ...` when the brain does not answer.
    public static func outcome(_ result: CommandResult, device: HandDevice?) -> JoinOutcome {
        if result.succeeded, let device, !device.device.isEmpty {
            return .joined(device)
        }
        if result.timedOut {
            return .unreachable("no answer in time; check the address and that this Mac can reach it.")
        }
        let said = result.output
        if let refusal = refusal(said) { return refusal }
        if said.contains("verb not allowed: join") || said.contains("verb not allowed:join") {
            return .failed("This hive-hand is older than joining with a code. Rebuild Hive.app (or hive-hand) from a Hive that has `hive-hand join`.")
        }
        if let range = said.range(of: "curl exit ") {
            let rest = said[range.upperBound...]
            let detail = rest.split(separator: ":", maxSplits: 1).dropFirst().first.map {
                $0.trimmingCharacters(in: .whitespaces)
            } ?? String(rest)
            return .unreachable(detail.replacingOccurrences(of: "curl: ", with: ""))
        }
        if result.succeeded {
            return .failed("hive-hand joined but left no device.json; run `hive-hand join` in Terminal to see why.")
        }
        let line = said.isEmpty ? "hive-hand join exited with code \(result.status)." : said
        return .failed(line.hasPrefix("hive-hand join: ") ? String(line.dropFirst("hive-hand join: ".count)) : line)
    }

    static func refusal(_ text: String) -> JoinOutcome? {
        guard let range = text.range(of: #"refused \(([0-9]{3})\): "#, options: .regularExpression) else { return nil }
        let marker = text[range]
        let status = Int(marker.filter(\.isNumber)) ?? 0
        let message = text[range.upperBound...].split(separator: "\n").first.map(String.init) ?? ""
        let parts = message.components(separatedBy: " - ")
        let why = parts[0].trimmingCharacters(in: .whitespaces)
        let fix = parts.dropFirst().joined(separator: " - ").trimmingCharacters(in: .whitespaces)
        return .refused(status: status, why: why, fix: fix)
    }
}

/// `hive-hand dial` kept running by launchd once this Mac has joined: a
/// LaunchAgent that starts at login and comes back when it stops - except
/// when the brain refuses the device (revoked: hive-hand dial exits 3),
/// which a new join code fixes, not a restart.
public enum DialAgent {
    public static let label = "hive.hand"

    /// The agent's PATH: a login shell's (what a new terminal has, so the
    /// jobs find git, cargo, node), else the usual install places.
    public static func path(loginPath: String?, userHome: String?) -> String {
        loginPath.flatMap { $0.isEmpty ? nil : $0 } ?? HiveSetup.fallbackPath(userHome: userHome)
    }

    public static func plistURL(agents: URL) -> URL {
        agents.appendingPathComponent("\(label).plist")
    }

    /// The agent's command: dial, and turn "refused" (3) into a clean exit so
    /// KeepAlive leaves it stopped.
    public static func programArguments(binary: URL) -> [String] {
        ["/bin/sh", "-c", "\"$0\" dial; s=$?; [ \"$s\" -eq 3 ] && exit 0; exit \"$s\"", binary.path]
    }

    public static func plist(binary: URL, setup: HandSetup, path: String) -> [String: Any] {
        [
            "Label": label,
            "ProgramArguments": programArguments(binary: binary),
            "EnvironmentVariables": [
                "HIVE_HAND_HOME": setup.home.path,
                "PATH": path,
                "LANG": "en_US.UTF-8",
            ],
            "WorkingDirectory": setup.home.path,
            "RunAtLoad": true,
            "KeepAlive": ["SuccessfulExit": false],
            "ThrottleInterval": 10,
            "ProcessType": "Background",
            "StandardOutPath": setup.dialLog.path,
            "StandardErrorPath": setup.dialLog.path,
        ]
    }

    /// The binary the agent runs: the app's own is copied into the hand's
    /// home when it lives inside an app bundle.
    public static func agentBinary(for setup: HandSetup, fileManager: FileManager = .default) throws -> URL {
        guard let binary = setup.binary else {
            throw NSError(domain: "HiveHand", code: 1, userInfo: [NSLocalizedDescriptionKey: "there is no hive-hand on this Mac"])
        }
        guard binary.path.contains(".app/Contents/") else { return binary }
        let target = setup.installedBinary
        if !sameFile(binary, target, fileManager: fileManager) {
            try fileManager.createDirectory(at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
            let staged = target.deletingLastPathComponent().appendingPathComponent(".hive-hand.new")
            try? fileManager.removeItem(at: staged)
            try fileManager.copyItem(at: binary, to: staged)
            _ = try fileManager.replaceItemAt(target, withItemAt: staged)
        }
        return target
    }

    static func sameFile(_ a: URL, _ b: URL, fileManager: FileManager) -> Bool {
        guard let left = try? fileManager.attributesOfItem(atPath: a.path),
              let right = try? fileManager.attributesOfItem(atPath: b.path) else { return false }
        return (left[.size] as? NSNumber) == (right[.size] as? NSNumber)
            && fileManager.contentsEqual(atPath: a.path, andPath: b.path)
    }

    /// Writes the agent and (re)loads it with `launchctl`; `launchctl` is the
    /// seam tests replace. Returns nil when it runs, else why it does not.
    public static func install(
        setup: HandSetup,
        agents: URL,
        path: String,
        launchctl: @Sendable ([String]) async -> CommandResult = { await Command.run("/bin/launchctl", $0, timeout: 30) }
    ) async -> String? {
        let binary: URL
        do {
            binary = try agentBinary(for: setup)
        } catch {
            return "could not place hive-hand: \(error.localizedDescription)"
        }
        let url = plistURL(agents: agents)
        let wanted = plist(binary: binary, setup: setup, path: path)
        do {
            try FileManager.default.createDirectory(at: agents, withIntermediateDirectories: true)
            try FileManager.default.createDirectory(at: setup.home, withIntermediateDirectories: true)
            let data = try PropertyListSerialization.data(fromPropertyList: wanted, format: .xml, options: 0)
            try data.write(to: url, options: .atomic)
        } catch {
            return "could not write \(url.path): \(error.localizedDescription)"
        }
        // A changed unit only takes effect after a bootout; a missing one fails it harmlessly.
        _ = await launchctl(["bootout", "\(Launchd.domain)/\(label)"])
        let loaded = await launchctl(["bootstrap", Launchd.domain, url.path])
        if !loaded.succeeded {
            // 5 / 37: still going away from the bootout; once more after a moment.
            try? await Task.sleep(nanoseconds: 1_000_000_000)
            let again = await launchctl(["bootstrap", Launchd.domain, url.path])
            if !again.succeeded {
                return "launchctl bootstrap failed: \(again.output.isEmpty ? "exit \(again.status)" : again.output)"
            }
        }
        return nil
    }

    /// The agent as written, if it is: its ProgramArguments.
    public static func installedArguments(agents: URL) -> [String]? {
        guard let data = try? Data(contentsOf: plistURL(agents: agents)),
              let plist = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any] else { return nil }
        return plist["ProgramArguments"] as? [String]
    }
}
