import Foundation

/// How a short command the app ran ended.
public struct CommandResult: Equatable, Sendable {
    public var status: Int32
    public var stdout: String
    public var stderr: String
    public var timedOut: Bool

    public init(status: Int32, stdout: String = "", stderr: String = "", timedOut: Bool = false) {
        self.status = status
        self.stdout = stdout
        self.stderr = stderr
        self.timedOut = timedOut
    }

    public var succeeded: Bool { status == 0 && !timedOut }
    /// stderr, else stdout, trimmed: what to show when it failed.
    public var output: String {
        let err = stderr.trimmingCharacters(in: .whitespacesAndNewlines)
        return err.isEmpty ? stdout.trimmingCharacters(in: .whitespacesAndNewlines) : err
    }
}

/// Runs a short command (launchctl, pulse init, a login shell) off the main
/// thread, with its output collected and a deadline.
public enum Command {
    public static func run(
        _ executable: String,
        _ arguments: [String],
        environment: [String: String]? = nil,
        currentDirectory: URL? = nil,
        timeout: TimeInterval = 20
    ) async -> CommandResult {
        await withCheckedContinuation { (continuation: CheckedContinuation<CommandResult, Never>) in
            DispatchQueue.global(qos: .userInitiated).async {
                continuation.resume(returning: runBlocking(
                    executable, arguments, environment: environment, currentDirectory: currentDirectory, timeout: timeout
                ))
            }
        }
    }

    static func runBlocking(
        _ executable: String,
        _ arguments: [String],
        environment: [String: String]?,
        currentDirectory: URL?,
        timeout: TimeInterval
    ) -> CommandResult {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        if let environment { process.environment = environment }
        if let currentDirectory { process.currentDirectoryURL = currentDirectory }
        let out = Pipe()
        let err = Pipe()
        process.standardOutput = out
        process.standardError = err
        process.standardInput = FileHandle.nullDevice

        let done = DispatchSemaphore(value: 0)
        process.terminationHandler = { _ in done.signal() }
        do {
            try process.run()
        } catch {
            return CommandResult(status: 127, stderr: "\(executable): \(error.localizedDescription)")
        }

        // Drain both pipes while the command runs so a chatty one never blocks.
        let collected = Collected()
        let group = DispatchGroup()
        for (pipe, isOut) in [(out, true), (err, false)] {
            group.enter()
            DispatchQueue.global(qos: .utility).async {
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                collected.set(data, stdout: isOut)
                group.leave()
            }
        }

        var timedOut = false
        if done.wait(timeout: .now() + timeout) == .timedOut {
            timedOut = true
            process.terminate()
            if done.wait(timeout: .now() + 2) == .timedOut {
                kill(process.processIdentifier, SIGKILL)
                done.wait()
            }
        }
        // A grandchild that kept the pipes open must not hold us forever.
        _ = group.wait(timeout: .now() + 2)
        let (stdout, stderr) = collected.text()
        return CommandResult(status: process.terminationStatus, stdout: stdout, stderr: stderr, timedOut: timedOut)
    }

    private final class Collected: @unchecked Sendable {
        private let lock = NSLock()
        private var out = Data()
        private var err = Data()

        func set(_ data: Data, stdout: Bool) {
            lock.lock()
            defer { lock.unlock() }
            if stdout { out = data } else { err = data }
        }

        func text() -> (String, String) {
            lock.lock()
            defer { lock.unlock() }
            return (String(decoding: out, as: UTF8.self), String(decoding: err, as: UTF8.self))
        }
    }
}
