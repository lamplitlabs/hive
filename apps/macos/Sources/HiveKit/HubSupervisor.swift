import Foundation
import os

/// Why the console cannot be shown, in the console's plain words.
public struct HubProblem: Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        /// No hive at this path: Hive is not installed (or moved).
        case hiveMissing(URL)
        /// No store yet: a first project (`hive init <folder>`) sets one up.
        case firstProject
        /// Another program answers on the console's port.
        case portBusy(Int)
        /// launchd's hive.hub would not come up.
        case serviceDown
        /// The console the app started kept stopping.
        case hubStopped
        /// The console at a `HubURL` override does not answer.
        case unreachable(URL)
    }

    public var kind: Kind
    public var title: String
    public var detail: String
    /// The log that says more, shown as a tail and openable in Console.
    public var log: URL?

    public init(kind: Kind, title: String, detail: String, log: URL? = nil) {
        self.kind = kind
        self.title = title
        self.detail = detail
        self.log = log
    }
}

public enum HubState: Equatable, Sendable {
    /// Finding or starting the console; the words say what is happening.
    case connecting(String)
    case ready(URL)
    /// The console answered before and does not now (a restart, an update, a
    /// handover); windows that still show a page keep it.
    case reconnecting(URL, String)
    case problem(HubProblem)

    public var consoleURL: URL? {
        switch self {
        case .ready(let url), .reconnecting(let url, _): return url
        case .connecting, .problem: return nil
        }
    }
}

/// Everything the supervisor does to the outside world, so its decisions can
/// be tested without launchd, a port or a hive binary.
@MainActor
public protocol HubWorld: AnyObject {
    func readSetup() -> HiveSetup
    func probe(_ url: URL) async -> ProbeResult
    func job(_ label: String) async -> LaunchdJob
    func kickstart(_ label: String, restart: Bool) async
    func bootstrap(_ plist: URL) async
    func fileExists(_ url: URL) -> Bool
    func childEnvironment(for setup: HiveSetup) async -> [String: String]
    func launchHub(_ setup: HiveSetup, port: Int, environment: [String: String]) throws -> any OwnedHub
    func runInit(_ setup: HiveSetup, folder: URL, environment: [String: String]) async -> CommandResult
    /// Changes when the binary is replaced (mtime and size).
    func binaryStamp(_ url: URL) -> String?
    func logTail(_ url: URL) -> String?
    func sleep(seconds: Double) async throws
    func now() -> Date
}

/// A console the app started; HubProcess in the app, a fake in tests.
@MainActor
public protocol OwnedHub: AnyObject {
    var port: Int { get }
    var servingURL: URL? { get }
    var exit: HubExit? { get }
    /// Ended, as far as the operating system knows (ahead of `exit`).
    var hasExited: Bool { get }
    var onEvent: ((HubProcess.Event) -> Void)? { get set }
    func terminate(grace: TimeInterval) async
    func requestStop()
    func forceStop()
}

extension HubProcess: OwnedHub {}

/// Keeps one console reachable for every window: the launchd service's when
/// `hive service install` set one up, a `hive hub` of the app's own when
/// not (until the console's "Keep Hive running" hands it over), or the one
/// the owner pointed the app at.
@MainActor
public final class HubSupervisor {
    public private(set) var state: HubState = .connecting("Looking for Hive…")
    public private(set) var setup: HiveSetup

    /// How long the service may take to answer again: up to 17 s of shutdown,
    /// launchd's 30 s throttle, then the store and conversations reopening.
    public var serviceGrace: TimeInterval = 90
    /// How long an app-started console may take to print its Serving line.
    public var startGrace: TimeInterval = 60
    /// How often the app's own console is checked for a new binary or a service.
    public var watchInterval: TimeInterval = 15

    private let world: any HubWorld
    private var observers: [UUID: (HubState) -> Void] = [:]
    private var flow: Task<Void, Never>?
    private var watcher: Task<Void, Never>?
    private var owned: (any OwnedHub)?
    /// A console the app is stopping to replace; none starts until it is gone.
    private var retiring: (any OwnedHub)?
    private var ownedStamp: String?
    /// The spawn flow decides what an early exit means; the event handler
    /// handles exits once the console is up.
    private var spawning = false
    private var stopping = false
    private var unexpectedExits: [Date] = []
    private var serviceSeenAt: Date?
    private let log = Logger(subsystem: AppIdentity.bundleID, category: "supervisor")

    public init(world: any HubWorld) {
        self.world = world
        self.setup = world.readSetup()
    }

    /// Whether quitting the app stops the console (it started it).
    public var ownsRunningHub: Bool { owned != nil && owned?.exit == nil }

    @discardableResult
    public func observe(_ observer: @escaping (HubState) -> Void) -> UUID {
        let token = UUID()
        observers[token] = observer
        observer(state)
        return token
    }

    public func removeObserver(_ token: UUID) {
        observers[token] = nil
    }

    public func start() {
        begin { await $0.connect() }
    }

    /// Try Again on the problem screen.
    public func retry() {
        unexpectedExits.removeAll()
        begin { await $0.connect() }
    }

    /// A window could not reach the console (a navigation failed to connect).
    public func consoleLooksDown() {
        guard case .ready(let url) = state else { return }
        begin { supervisor in
            if await supervisor.world.probe(url) == .hive {
                // A blip: say ready again so the window loads what it wanted.
                supervisor.become(.ready(url))
                return
            }
            supervisor.become(.reconnecting(url, "Reconnecting to Hive…"))
            await supervisor.connect()
        }
    }

    /// After sleep: the console may have restarted, the app's own may be gone,
    /// and a wait that spanned the sleep has outlived its deadline without
    /// the console having had the time; start that wait again.
    public func checkAfterWake() {
        switch state {
        case .ready:
            consoleLooksDown()
        case .connecting, .reconnecting:
            retry()
        case .problem(let problem):
            switch problem.kind {
            case .serviceDown, .hubStopped, .unreachable, .portBusy: retry()
            case .hiveMissing, .firstProject: break
            }
        }
    }

    /// Registers the first project (`hive init <folder>`), which creates the
    /// store, then starts the console.
    public func createFirstProject(at folder: URL) async -> CommandResult {
        let setup = world.readSetup()
        let environment = await world.childEnvironment(for: setup)
        let result = await world.runInit(setup, folder: folder, environment: environment)
        if result.succeeded { retry() }
        return result
    }

    /// Quitting: stop the console the app started. The service's is left alone.
    /// The hub drains its connections (5 s) and ends working conversations
    /// (12 s) on its way out; launchd gives it 20 s, so does the app.
    public static let stopGrace: TimeInterval = 20

    public func shutdown() async {
        beginShutdown()
        if let hub = owned, hub.exit == nil {
            await hub.terminate(grace: Self.stopGrace)
        }
    }

    /// Quitting, without awaiting anything: AppKit waits for the app's reply
    /// inside the call that asked it to quit, so the app polls
    /// `ownedHubStopped` from a run-loop timer instead of awaiting a task.
    public func beginShutdown() {
        stopping = true
        flow?.cancel()
        watcher?.cancel()
        if let hub = owned, hub.exit == nil {
            log.notice("stopping the app's console (port \(hub.port, privacy: .public))")
            hub.requestStop()
        }
    }

    /// True once the console the app started has ended (or there was none).
    public var ownedHubStopped: Bool {
        guard let hub = owned else { return true }
        return hub.exit != nil || hub.hasExited
    }

    /// The console outlived its grace: kill it.
    public func forceStopOwnedHub() {
        guard let hub = owned, hub.exit == nil else { return }
        log.error("the app's console outlived its grace; killing it")
        hub.forceStop()
    }

    // MARK: - Flows

    private func begin(_ body: @escaping @MainActor (HubSupervisor) async -> Void) {
        flow?.cancel()
        flow = Task { @MainActor [weak self] in
            guard let self else { return }
            await body(self)
        }
    }

    private func connect() async {
        if let old = retiring, !old.hasExited {
            say("Restarting Hive…")
            await old.terminate(grace: 20)
        }
        retiring = nil
        guard !Task.isCancelled else { return }
        setup = world.readSetup()
        if let url = setup.hubOverride {
            await connectOverride(url)
        } else if setup.hubService != nil, !ownsRunningHub {
            await connectService()
        } else {
            await connectOwned()
        }
    }

    /// The owner pointed the app at a console: wait for it, never start one.
    private func connectOverride(_ url: URL) async {
        say("Connecting to \(Self.address(url))…")
        if await waitForConsole(url, upTo: 30) {
            become(.ready(url))
        } else if !Task.isCancelled {
            become(.problem(HubProblem(
                kind: .unreachable(url),
                title: "Hive doesn't answer at \(Self.address(url))",
                detail: "The app was told to use the console at \(url.absoluteString) (the HubURL setting), and nothing answers there. Start that console, or remove the setting with `defaults delete \(AppIdentity.bundleID) HubURL`."
            )))
        }
    }

    /// launchd runs the console as hive.hub: wait out its restarts, start it
    /// when launchd has it stopped or unloaded, never start a second one.
    private func connectService() async {
        let url = setup.consoleURL
        switch await world.probe(url) {
        case .hive:
            become(.ready(url))
            return
        case .notHive:
            become(.problem(portBusyProblem(port: setup.port, service: true)))
            return
        case .unreachable:
            break
        }
        guard world.fileExists(setup.binary) else {
            become(.problem(missingProblem(setup.binary)))
            return
        }
        let deadline = world.now().addingTimeInterval(serviceGrace)
        var nudged = false
        var lastJob = LaunchdJob.notLoaded
        while !Task.isCancelled {
            let label = setup.hubService?.label ?? HiveSetup.hubLabel
            let job = await world.job(label)
            lastJob = job
            if !job.loaded, let plist = setup.hubService?.plist, !nudged {
                say("Starting Hive's console…")
                log.notice("\(label, privacy: .public) is not loaded; bootstrapping \(plist.path, privacy: .public)")
                await world.bootstrap(plist)
                nudged = true
            } else if job.loaded, !job.running, !nudged {
                say("Starting Hive's console…")
                log.notice("\(label, privacy: .public) is loaded but not running; kickstarting it")
                await world.kickstart(label, restart: false)
                nudged = true
            } else {
                say("Hive is restarting…")
            }
            let answer = await waitForAnswer(url, upTo: 8)
            if answer == .hive {
                become(.ready(url))
                return
            }
            if case .notHive = answer {
                become(.problem(portBusyProblem(port: setup.port, service: true)))
                return
            }
            if world.now() >= deadline { break }
        }
        guard !Task.isCancelled else { return }
        var detail = "Hive's console runs in the background on this Mac, and it hasn't answered for \(Int(serviceGrace)) seconds."
        if let code = lastJob.lastExitCode, !lastJob.running {
            detail += " It last stopped with code \(code)."
        }
        detail += " The end of its log is below. Restart the Console tries again; if it still won't start, `hive service install` in a terminal sets it up afresh."
        become(.problem(HubProblem(
            kind: .serviceDown, title: "Hive's console didn't start", detail: detail, log: setup.serviceLog
        )))
    }

    /// No service: use a console someone already started on the port, or
    /// start one, as `hive hub` in a terminal would.
    private func connectOwned() async {
        if let hub = owned, hub.exit == nil {
            let url = hub.servingURL ?? HiveSetup.consoleURL(port: hub.port)
            say("Waiting for Hive…")
            if await waitForConsole(url, upTo: 30) {
                become(.ready(url))
                return
            }
            guard !Task.isCancelled else { return }
            log.error("the app's console runs but has not answered for 30 s; restarting it")
            stopWatching()
            owned = nil
            await hub.terminate(grace: 10)
        }
        let preferred = HiveSetup.consoleURL(port: setup.port)
        var port = setup.port
        switch await world.probe(preferred) {
        case .hive:
            // Someone started one (a terminal's `hive hub`): use it, own nothing.
            become(.ready(preferred))
            return
        case .notHive(let what):
            log.notice("port \(self.setup.port, privacy: .public) holds another program (\(what, privacy: .public)); the console takes a free port")
            port = 0
        case .unreachable:
            break
        }
        guard !Task.isCancelled else { return }
        guard world.fileExists(setup.binary) else {
            become(.problem(missingProblem(setup.binary)))
            return
        }
        guard world.fileExists(setup.store) || world.fileExists(setup.legacyStore) else {
            become(.problem(firstProjectProblem()))
            return
        }
        await spawn(port: port)
    }

    private func spawn(port: Int) async {
        say("Starting Hive…")
        let environment = await world.childEnvironment(for: setup)
        guard !Task.isCancelled else { return }
        let hub: any OwnedHub
        do {
            hub = try world.launchHub(setup, port: port, environment: environment)
        } catch {
            become(.problem(HubProblem(
                kind: .hubStopped,
                title: "Hive didn't start",
                detail: "The app could not run \(setup.binary.path): \(error).",
                log: setup.appHubLog
            )))
            return
        }
        owned = hub
        ownedStamp = world.binaryStamp(setup.binary)
        spawning = true
        defer { spawning = false }
        hub.onEvent = { [weak self, weak hub] event in
            guard let self, let hub else { return }
            self.ownedEvent(event, from: hub)
        }

        let deadline = world.now().addingTimeInterval(startGrace)
        while hub.servingURL == nil, hub.exit == nil, world.now() < deadline {
            guard (try? await world.sleep(seconds: 0.1)) != nil else { return }
        }
        guard !Task.isCancelled else { return }

        if let url = hub.servingURL, hub.exit == nil {
            if await waitForConsole(url, upTo: 20) {
                become(.ready(url))
                startWatching()
                return
            }
            guard !Task.isCancelled else { return }
        }

        switch hub.exit {
        case .portTaken?:
            // Taken between the check and the bind: whoever took it may be Hive.
            owned = nil
            let preferred = HiveSetup.consoleURL(port: setup.port)
            if await world.probe(preferred) == .hive {
                become(.ready(preferred))
            } else if port != 0 {
                await spawn(port: 0)
            } else {
                become(.problem(stoppedProblem("could not listen on any port")))
            }
        case .noStore?:
            owned = nil
            become(.problem(firstProjectProblem()))
        case let ended?:
            owned = nil
            become(.problem(stoppedProblem(ended.words)))
        case nil:
            owned = nil
            await hub.terminate(grace: 5)
            become(.problem(stoppedProblem("did not start serving within \(Int(startGrace)) seconds")))
        }
    }

    /// The app's console after it came up: a handover, a stop, a crash.
    private func ownedEvent(_ event: HubProcess.Event, from hub: any OwnedHub) {
        guard hub === owned else { return }
        switch event {
        case .line(.handingOver):
            if let url = state.consoleURL { become(.reconnecting(url, "Handing Hive over to its service…")) }
        case .line:
            break
        case .exited(let ended):
            guard !spawning, !stopping else { return }
            owned = nil
            stopWatching()
            switch ended {
            case .handedOver:
                log.notice("the console moved to the hive.hub service")
                begin { await $0.connect() }
            default:
                unexpectedExits.append(world.now())
                unexpectedExits = unexpectedExits.filter { world.now().timeIntervalSince($0) < 120 }
                if unexpectedExits.count >= 4 {
                    become(.problem(stoppedProblem("\(ended.words), four times in two minutes")))
                    return
                }
                if let url = state.consoleURL { become(.reconnecting(url, "Hive stopped; starting it again…")) }
                let pause = Double(unexpectedExits.count)
                begin { supervisor in
                    guard (try? await supervisor.world.sleep(seconds: pause)) != nil else { return }
                    await supervisor.connect()
                }
            }
        }
    }

    /// While the app's console runs: restart it onto a new hive binary (the
    /// service does the same with --restart-on-change), and give the port to
    /// a service someone installed from a terminal.
    private func startWatching() {
        stopWatching()
        serviceSeenAt = nil
        watcher = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                guard let self, (try? await self.world.sleep(seconds: self.watchInterval)) != nil else { return }
                await self.watchOnce()
            }
        }
    }

    private func stopWatching() {
        watcher?.cancel()
        watcher = nil
    }

    private func watchOnce() async {
        guard let hub = owned, hub.exit == nil, case .ready(let url) = state else { return }
        let current = world.readSetup()
        if current.hubService != nil {
            // The console's own handover finishes in seconds; a service the
            // owner installed from a terminal waits for the port instead.
            let seen = serviceSeenAt ?? world.now()
            serviceSeenAt = seen
            if world.now().timeIntervalSince(seen) >= 60 {
                log.notice("a hive.hub service exists; stopping the app's console so it can serve")
                stopWatching()
                owned = nil
                retiring = hub
                become(.reconnecting(url, "Handing Hive over to its service…"))
                begin { await $0.connect() }
            }
            return
        }
        serviceSeenAt = nil
        let stamp = world.binaryStamp(setup.binary)
        if let stamp, let old = ownedStamp, stamp != old {
            log.notice("hive changed on disk; restarting the app's console onto it")
            stopWatching()
            owned = nil
            retiring = hub
            become(.reconnecting(url, "Restarting Hive on its new version…"))
            begin { await $0.connect() }
        }
    }

    // MARK: - Helpers

    private func become(_ next: HubState) {
        if next != state {
            log.notice("state: \(String(describing: next), privacy: .public)")
        }
        state = next
        for observer in observers.values { observer(next) }
    }

    /// New words for what is happening, keeping a console that was up as the one to come back to.
    private func say(_ words: String) {
        if let url = state.consoleURL {
            become(.reconnecting(url, words))
        } else {
            become(.connecting(words))
        }
    }

    private func waitForConsole(_ url: URL, upTo seconds: TimeInterval) async -> Bool {
        await waitForAnswer(url, upTo: seconds) == .hive
    }

    /// Probes until Hive or another program answers, or time runs out.
    private func waitForAnswer(_ url: URL, upTo seconds: TimeInterval) async -> ProbeResult {
        let deadline = world.now().addingTimeInterval(seconds)
        var pause = 0.25
        var last = ProbeResult.unreachable("not asked yet")
        while !Task.isCancelled {
            last = await world.probe(url)
            if case .unreachable = last {} else { return last }
            if world.now() >= deadline { return last }
            guard (try? await world.sleep(seconds: pause)) != nil else { return last }
            pause = min(pause * 1.6, 1.5)
        }
        return last
    }

    private func missingProblem(_ binary: URL) -> HubProblem {
        HubProblem(
            kind: .hiveMissing(binary),
            title: "Hive isn't installed on this Mac",
            detail: "The app runs hive from \(Self.tilde(binary.path)), and there is nothing there. Install it once from your Hive checkout with `cargo install --path crates/hive --root ~/.hive --locked`, or point the app at another hive."
        )
    }

    private func firstProjectProblem() -> HubProblem {
        HubProblem(
            kind: .firstProject,
            title: "Add your first project",
            detail: "Hive keeps everything it knows in \(Self.tilde(setup.home.path)), and that is empty so far. Choose a folder with a git checkout to start; it becomes your first project, as `hive init` does in a terminal."
        )
    }

    private func portBusyProblem(port: Int, service: Bool) -> HubProblem {
        HubProblem(
            kind: .portBusy(port),
            title: "Another program is using port \(port)",
            detail: service
                ? "Hive's console runs on 127.0.0.1:\(port), and something else answers there, so it can't start. Quit that program and try again."
                : "Something other than Hive answers on 127.0.0.1:\(port).",
            log: service ? setup.serviceLog : nil
        )
    }

    private func stoppedProblem(_ how: String) -> HubProblem {
        HubProblem(
            kind: .hubStopped,
            title: "Hive stopped",
            detail: "The console the app started \(how). The end of its log is below.",
            log: setup.appHubLog
        )
    }

    static func address(_ url: URL) -> String {
        guard let host = url.host else { return url.absoluteString }
        return url.port.map { "\(host):\($0)" } ?? host
    }

    static func tilde(_ path: String) -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        return path.hasPrefix(home + "/") ? "~" + path.dropFirst(home.count) : path
    }
}
