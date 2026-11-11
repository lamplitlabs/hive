import Foundation

/// The real world: launchd, the hub's health route, a hive child process.
@MainActor
public final class LiveWorld: HubWorld {
    private let defaults: UserDefaults
    private var loginPath: String??

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    public func readSetup() -> HiveSetup {
        SetupReader.read(
            overrides: .from(defaults),
            environment: ProcessInfo.processInfo.environment,
            userHome: FileManager.default.homeDirectoryForCurrentUser
        )
    }

    public func probe(_ url: URL) async -> ProbeResult {
        await HubProbe().check(url)
    }

    public func job(_ label: String) async -> LaunchdJob {
        await Launchd.job(label)
    }

    public func kickstart(_ label: String, restart: Bool) async {
        await Launchd.kickstart(label, restart: restart)
    }

    public func bootstrap(_ plist: URL) async {
        await Launchd.bootstrap(plist)
    }

    public func fileExists(_ url: URL) -> Bool {
        FileManager.default.fileExists(atPath: url.path)
    }

    public func childEnvironment(for setup: HiveSetup) async -> [String: String] {
        let base = ProcessInfo.processInfo.environment
        var path: String?
        if setup.hubService == nil, setup.daemonService == nil {
            // No unit recorded a PATH: ask a login shell once, like a new terminal.
            if loginPath == nil {
                loginPath = .some(await LoginShell.path(shell: base["SHELL"]))
            }
            path = loginPath ?? nil
        }
        return setup.childEnvironment(base: base, loginPath: path)
    }

    public func launchHub(_ setup: HiveSetup, port: Int, environment: [String: String]) throws -> any OwnedHub {
        let bin = setup.home.appendingPathComponent("bin", isDirectory: true)
        let workingDirectory = fileExists(bin) ? bin : FileManager.default.homeDirectoryForCurrentUser
        return try HubProcess.launch(
            binary: setup.binary, port: port, environment: environment, log: setup.appHubLog,
            workingDirectory: workingDirectory
        )
    }

    public func runInit(_ setup: HiveSetup, folder: URL, environment: [String: String]) async -> CommandResult {
        await Command.run(
            setup.binary.path, ["init", folder.path], environment: environment, currentDirectory: folder, timeout: 120
        )
    }

    public func binaryStamp(_ url: URL) -> String? {
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.path) else { return nil }
        let modified = (attributes[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0
        let size = (attributes[.size] as? NSNumber)?.int64Value ?? 0
        return "\(modified):\(size)"
    }

    public func logTail(_ url: URL) -> String? {
        LogTail.read(url)
    }

    public func sleep(seconds: Double) async throws {
        try await Task.sleep(nanoseconds: UInt64(max(seconds, 0) * 1_000_000_000))
    }

    public func now() -> Date {
        Date()
    }
}
