import Foundation

/// A launchd job as `launchctl print gui/<uid>/<label>` describes it.
public struct LaunchdJob: Equatable, Sendable {
    public var loaded: Bool
    public var running: Bool
    public var pid: Int32?
    public var lastExitCode: Int?

    public init(loaded: Bool, running: Bool = false, pid: Int32? = nil, lastExitCode: Int? = nil) {
        self.loaded = loaded
        self.running = running
        self.pid = pid
        self.lastExitCode = lastExitCode
    }

    public static let notLoaded = LaunchdJob(loaded: false)

    /// Reads the top-level `state = running`, `pid = 63413` and `last exit
    /// code = 3` lines. A job launchd does not know exits 113 ("Could not find
    /// service").
    public static func parse(printOutput: String, status: Int32) -> LaunchdJob {
        guard status == 0 else { return .notLoaded }
        var job = LaunchdJob(loaded: true)
        for raw in printOutput.split(whereSeparator: \.isNewline) {
            // Nested blocks (endpoints, the environment) are indented deeper
            // than the job's own one-tab lines.
            guard raw.hasPrefix("\t"), !raw.hasPrefix("\t\t") else { continue }
            let line = raw.trimmingCharacters(in: .whitespaces)
            guard let eq = line.range(of: " = ") else { continue }
            let key = line[..<eq.lowerBound]
            let value = line[eq.upperBound...].trimmingCharacters(in: .whitespaces)
            switch key {
            case "state": job.running = value == "running"
            case "pid": job.pid = Int32(value)
            case "last exit code":
                job.lastExitCode = Int(value.split(separator: ":").first.map(String.init) ?? value)
            default: break
            }
        }
        if job.pid != nil { job.running = true }
        return job
    }
}

/// The few launchd calls the app makes about the console's service. It never
/// installs or removes units: that is `pulse service install` (or the
/// console's "Keep PULSE running"), which also signs pulse for macOS.
public enum Launchd {
    static let launchctl = "/bin/launchctl"

    public static var domain: String { "gui/\(getuid())" }

    public static func job(_ label: String) async -> LaunchdJob {
        let result = await Command.run(launchctl, ["print", "\(domain)/\(label)"], timeout: 10)
        return LaunchdJob.parse(printOutput: result.stdout, status: result.status)
    }

    /// Starts a loaded job that is not running; with `restart`, stops it first.
    @discardableResult
    public static func kickstart(_ label: String, restart: Bool = false) async -> CommandResult {
        var arguments = ["kickstart"]
        if restart { arguments.append("-k") }
        arguments.append("\(domain)/\(label)")
        return await Command.run(launchctl, arguments, timeout: 30)
    }

    /// Loads a unit whose plist exists but that launchd does not know (after a
    /// `launchctl bootout`, or a login that skipped it).
    @discardableResult
    public static func bootstrap(_ plist: URL) async -> CommandResult {
        await Command.run(launchctl, ["bootstrap", domain, plist.path], timeout: 30)
    }
}
