import Darwin
import Foundation
import os

/// A `hive hub` the app started because no service serves the console: the
/// same console `hive hub` in a terminal serves, which the console's own
/// "Keep Hive running" later hands over to the launchd service.
///
/// It runs as its own process group and is disclaimed, so macOS treats it as
/// responsible for itself: folder-access prompts name hive, whose stable
/// signature (ADR-035) the service's runs share, not the app. A small /bin/sh
/// watchdog stops it if the app ever dies without stopping it first.
@MainActor
public final class HubProcess {
    public enum Event: Equatable, Sendable {
        case line(HubLine)
        case exited(HubExit)
    }

    public let pid: pid_t
    public let port: Int
    public private(set) var servingURL: URL?
    public private(set) var exit: HubExit?
    public var onEvent: ((Event) -> Void)?

    private var sawHandover = false
    private var sawPortTaken = false
    private var sawNoStore = false
    private var exitWaiters: [CheckedContinuation<Void, Never>] = []
    private let output: OutputPump
    private let watchdog: Watchdog?
    /// Set by the reaper thread the moment waitpid returns, before the main
    /// queue hears of it: AppKit's wait for a quit reply does not run main-queue
    /// blocks, and the app still has to know the console has gone.
    private let reaped = ExitFlag()
    private static let log = Logger(subsystem: AppIdentity.bundleID, category: "hub")

    private init(pid: pid_t, port: Int, output: OutputPump, watchdog: Watchdog?) {
        self.pid = pid
        self.port = port
        self.output = output
        self.watchdog = watchdog
    }

    /// Starts `hive hub --port <port> --no-browser`. No `--restart-on-change`:
    /// that flag makes a hub claim to be the service, and the console's
    /// handover only runs on a hub that does not.
    public static func launch(
        binary: URL, port: Int, environment: [String: String], log logFile: URL, workingDirectory: URL
    ) throws -> HubProcess {
        let arguments = [binary.path, "hub", "--port", String(port), "--no-browser"]

        var outputFDs: [Int32] = [-1, -1]
        guard pipe(&outputFDs) == 0 else { throw SpawnError.system("pipe", errno) }
        _ = fcntl(outputFDs[0], F_SETFD, FD_CLOEXEC)
        _ = fcntl(outputFDs[1], F_SETFD, FD_CLOEXEC)

        let spawned = spawnProcess(
            path: binary.path,
            arguments: arguments,
            environment: environment,
            workingDirectory: workingDirectory,
            stdin: .devNull,
            output: outputFDs[1]
        )
        close(outputFDs[1])
        let pid: pid_t
        switch spawned {
        case .success(let value):
            pid = value
        case .failure(let error):
            close(outputFDs[0])
            throw error
        }

        appendLog(logFile, "--- \(timestamp()) Hive.app started \(arguments.joined(separator: " ")) (pid \(pid))\n")
        let watchdog = Watchdog.arm(for: pid)
        let pump = OutputPump(fd: outputFDs[0], log: logFile)
        let hub = HubProcess(pid: pid, port: port, output: pump, watchdog: watchdog)
        Self.log.notice("started hive hub pid \(pid, privacy: .public) on port \(port, privacy: .public)")

        // Lines and the exit reach the main queue in the order the pump saw
        // them: the exit is queued behind the last lines (the handover's
        // words decide what an exit 0 means).
        pump.start { lines in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { lines.forEach(hub.handle(line:)) }
            }
        }
        let reaper = Thread {
            var status: Int32 = 0
            while waitpid(pid, &status, 0) == -1 && errno == EINTR {}
            let waitStatus = status
            hub.reaped.set()
            watchdog?.disarm()
            pump.exited { lines in
                DispatchQueue.main.async {
                    MainActor.assumeIsolated {
                        lines.forEach(hub.handle(line:))
                        hub.handleExit(waitStatus: waitStatus)
                    }
                }
            }
        }
        reaper.name = "hive-hub-reaper"
        reaper.start()
        return hub
    }

    public var isRunning: Bool { exit == nil && !reaped.isSet }

    /// Whether the process has ended, even before the main queue has handled it.
    public nonisolated var hasExited: Bool { reaped.isSet }

    /// SIGTERM, then SIGKILL for the whole group after `grace`. The hub drains
    /// connections (5 s) and stops working conversations (12 s), which the
    /// next console carries on by itself.
    public func terminate(grace: TimeInterval = 20) async {
        guard isRunning else { return }
        requestStop()
        if await waitForExit(timeout: grace) { return }
        Self.log.error("hive hub pid \(self.pid, privacy: .public) outlived SIGTERM by \(grace, privacy: .public)s; killing its group")
        forceStop()
        _ = await waitForExit(timeout: 5)
    }

    /// SIGTERM, without waiting: the hub drains and exits by itself.
    public func requestStop() {
        guard !reaped.isSet else { return }
        Self.log.notice("asking hive hub pid \(self.pid, privacy: .public) to stop")
        kill(pid, SIGTERM)
    }

    /// SIGKILL for the hub and anything left in its group.
    public func forceStop() {
        guard !reaped.isSet else { return }
        kill(-pid, SIGKILL)
        kill(pid, SIGKILL)
    }

    /// True once the process has ended; false when `timeout` passed first.
    public func waitForExit(timeout: TimeInterval) async -> Bool {
        if !isRunning { return true }
        let timer = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(max(timeout, 0) * 1_000_000_000))
            self?.releaseWaiters()
        }
        await withCheckedContinuation { exitWaiters.append($0) }
        timer.cancel()
        return !isRunning
    }

    private func releaseWaiters() {
        let waiters = exitWaiters
        exitWaiters.removeAll()
        waiters.forEach { $0.resume() }
    }

    private func handle(line raw: String) {
        let line = HubLine.parse(raw)
        switch line {
        case .serving(let url): servingURL = url
        case .handingOver, .handedOver: sawHandover = true
        case .portTaken: sawPortTaken = true
        case .noStore: sawNoStore = true
        case .other: return
        }
        onEvent?(.line(line))
    }

    private func handleExit(waitStatus: Int32) {
        guard exit == nil else { return }
        let ended = HubExit.classify(
            waitStatus: waitStatus, sawHandover: sawHandover, sawPortTaken: sawPortTaken, sawNoStore: sawNoStore
        )
        exit = ended
        watchdog?.disarm()
        output.finish(note: "--- \(Self.timestamp()) hive hub (pid \(pid)) \(ended.words)\n")
        Self.log.notice("hive hub pid \(self.pid, privacy: .public) \(ended.words, privacy: .public)")
        releaseWaiters()
        onEvent?(.exited(ended))
    }

    nonisolated static func timestamp() -> String {
        ISO8601DateFormatter().string(from: Date())
    }

    nonisolated static func appendLog(_ file: URL, _ text: String) {
        let manager = FileManager.default
        try? manager.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        if !manager.fileExists(atPath: file.path) {
            manager.createFile(atPath: file.path, contents: nil)
        }
        guard !text.isEmpty, let handle = try? FileHandle(forWritingTo: file) else { return }
        defer { try? handle.close() }
        _ = try? handle.seekToEnd()
        try? handle.write(contentsOf: Data(text.utf8))
    }
}

public enum SpawnError: Error, CustomStringConvertible, Equatable {
    case system(String, Int32)

    public var description: String {
        switch self {
        case .system(let call, let code): return "\(call): \(String(cString: strerror(code)))"
        }
    }
}

/// A flag one thread sets and any thread reads.
final class ExitFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var value = false

    var isSet: Bool {
        lock.lock()
        defer { lock.unlock() }
        return value
    }

    func set() {
        lock.lock()
        value = true
        lock.unlock()
    }
}

enum SpawnInput {
    case devNull
    case fd(Int32)
}

/// posix_spawn with a fresh process group, only fds 0-2 inherited, and the
/// child made responsible for itself (the private but long-stable
/// `responsibility_spawnattrs_setdisclaim`, looked up at run time; without it
/// the child simply stays the app's).
func spawnProcess(
    path: String,
    arguments: [String],
    environment: [String: String],
    workingDirectory: URL?,
    stdin: SpawnInput,
    output: Int32?
) -> Result<pid_t, SpawnError> {
    var attributes: posix_spawnattr_t?
    posix_spawnattr_init(&attributes)
    defer { posix_spawnattr_destroy(&attributes) }
    posix_spawnattr_setflags(&attributes, Int16(POSIX_SPAWN_SETPGROUP | POSIX_SPAWN_CLOEXEC_DEFAULT | POSIX_SPAWN_SETSIGDEF))
    posix_spawnattr_setpgroup(&attributes, 0)
    // The app ignores SIGPIPE; its children start with the default again.
    var defaults = sigset_t()
    sigemptyset(&defaults)
    sigaddset(&defaults, SIGPIPE)
    posix_spawnattr_setsigdefault(&attributes, &defaults)
    disclaimResponsibility(&attributes)

    var actions: posix_spawn_file_actions_t?
    posix_spawn_file_actions_init(&actions)
    defer { posix_spawn_file_actions_destroy(&actions) }
    switch stdin {
    case .devNull: posix_spawn_file_actions_addopen(&actions, 0, "/dev/null", O_RDONLY, 0)
    case .fd(let fd): posix_spawn_file_actions_adddup2(&actions, fd, 0)
    }
    if let output {
        posix_spawn_file_actions_adddup2(&actions, output, 1)
        posix_spawn_file_actions_adddup2(&actions, output, 2)
    } else {
        posix_spawn_file_actions_addopen(&actions, 1, "/dev/null", O_WRONLY, 0)
        posix_spawn_file_actions_addopen(&actions, 2, "/dev/null", O_WRONLY, 0)
    }
    if let workingDirectory {
        posix_spawn_file_actions_addchdir_np(&actions, workingDirectory.path)
    }

    var argv: [UnsafeMutablePointer<CChar>?] = arguments.map { strdup($0) }
    argv.append(nil)
    var envp: [UnsafeMutablePointer<CChar>?] = environment.map { strdup("\($0.key)=\($0.value)") }
    envp.append(nil)
    defer {
        argv.forEach { free($0) }
        envp.forEach { free($0) }
    }
    var pid: pid_t = 0
    let rc = posix_spawn(&pid, path, &actions, &attributes, &argv, &envp)
    guard rc == 0 else { return .failure(.system("posix_spawn \(path)", rc)) }
    return .success(pid)
}

private typealias DisclaimFunction = @convention(c) (UnsafeMutablePointer<posix_spawnattr_t?>?, Int32) -> Int32

private func disclaimResponsibility(_ attributes: inout posix_spawnattr_t?) {
    // RTLD_DEFAULT is ((void *)-2) on Darwin.
    guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "responsibility_spawnattrs_setdisclaim") else {
        return
    }
    let disclaim = unsafeBitCast(symbol, to: DisclaimFunction.self)
    _ = disclaim(&attributes, 1)
}

/// `/bin/sh -c 'read -r word; [ "$word" = disarm ] || kill -TERM <pid>'`
/// reading a pipe only the app holds open: when the app dies, the pipe hits
/// EOF and the hub gets the SIGTERM the app would have sent.
final class Watchdog: @unchecked Sendable {
    private let lock = NSLock()
    private var writeFD: Int32

    private init(writeFD: Int32) {
        self.writeFD = writeFD
    }

    static func arm(for pid: pid_t) -> Watchdog? {
        var fds: [Int32] = [-1, -1]
        guard pipe(&fds) == 0 else { return nil }
        _ = fcntl(fds[0], F_SETFD, FD_CLOEXEC)
        _ = fcntl(fds[1], F_SETFD, FD_CLOEXEC)
        // Disarming a watchdog that already went must not raise SIGPIPE.
        _ = fcntl(fds[1], F_SETNOSIGPIPE, 1)
        let script = "read -r word; [ \"$word\" = disarm ] || kill -TERM \"$1\" 2>/dev/null"
        let spawned = spawnProcess(
            path: "/bin/sh",
            arguments: ["/bin/sh", "-c", script, "hive-app-watchdog", String(pid)],
            environment: ["PATH": "/usr/bin:/bin"],
            workingDirectory: URL(fileURLWithPath: "/"),
            stdin: .fd(fds[0]),
            output: nil
        )
        close(fds[0])
        guard case .success = spawned else {
            close(fds[1])
            return nil
        }
        return Watchdog(writeFD: fds[1])
    }

    /// The hub ended, or the app is stopping it itself: let the watchdog go.
    func disarm() {
        lock.lock()
        defer { lock.unlock() }
        guard writeFD >= 0 else { return }
        let word = Array("disarm\n".utf8)
        _ = word.withUnsafeBufferPointer { write(writeFD, $0.baseAddress, $0.count) }
        close(writeFD)
        writeFD = -1
    }

    /// What the app's death looks like to the watchdog: the pipe closes
    /// without the word.
    func abandonForTesting() {
        lock.lock()
        defer { lock.unlock() }
        guard writeFD >= 0 else { return }
        close(writeFD)
        writeFD = -1
    }

    deinit {
        disarm()
    }
}

/// Reads the hub's stdout and stderr (one pipe), appends every byte to the log
/// and hands complete lines to the app, in order, on one serial queue.
final class OutputPump: @unchecked Sendable {
    private let fd: Int32
    private let queue = DispatchQueue(label: "hive.hub.output")
    private var source: DispatchSourceRead?
    private var pending = Data()
    private var log: FileHandle?
    private var finished = false

    init(fd: Int32, log url: URL) {
        self.fd = fd
        _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
        HubProcess.appendLog(url, "")
        log = try? FileHandle(forWritingTo: url)
        _ = try? log?.seekToEnd()
    }

    func start(onLines: @escaping @Sendable ([String]) -> Void) {
        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
        source.setEventHandler { [weak self] in
            guard let self else { return }
            let lines = self.readAvailable(flushTail: false)
            if !lines.isEmpty { onLines(lines) }
        }
        self.source = source
        source.resume()
    }

    /// Runs after every line the pump already delivered: the last output,
    /// including a final line without a newline, then `then`.
    func exited(then deliver: @escaping @Sendable ([String]) -> Void) {
        queue.async { [self] in
            deliver(readAvailable(flushTail: true))
        }
    }

    func finish(note: String) {
        queue.async { [self] in
            guard !finished else { return }
            finished = true
            try? log?.write(contentsOf: Data(note.utf8))
            try? log?.close()
            log = nil
            // The hub's own children may still hold the pipe; stop listening anyway.
            source?.cancel()
            source = nil
            close(fd)
        }
    }

    private func readAvailable(flushTail: Bool) -> [String] {
        guard !finished else { return [] }
        var buffer = [UInt8](repeating: 0, count: 16 * 1024)
        while true {
            let count = read(fd, &buffer, buffer.count)
            if count > 0 {
                let chunk = Data(buffer[0..<count])
                try? log?.write(contentsOf: chunk)
                pending.append(chunk)
                continue
            }
            if count == 0 { source?.cancel() } // EOF: every writer is gone
            break
        }
        var lines: [String] = []
        while let newline = pending.firstIndex(of: UInt8(ascii: "\n")) {
            lines.append(String(decoding: pending[pending.startIndex..<newline], as: UTF8.self))
            pending.removeSubrange(pending.startIndex...newline)
        }
        if flushTail, !pending.isEmpty {
            lines.append(String(decoding: pending, as: UTF8.self))
            pending.removeAll()
        }
        return lines
    }
}
