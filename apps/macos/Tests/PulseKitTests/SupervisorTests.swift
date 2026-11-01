import Foundation
import Testing
@testable import PulseKit

/// A console the supervisor started, driven by the test.
@MainActor
final class FakeHub: OwnedHub {
    let port: Int
    var servingURL: URL?
    var exit: HubExit?
    var onEvent: ((HubProcess.Event) -> Void)?
    var terminated = false
    var hasExited: Bool { exit != nil }

    init(port: Int) {
        self.port = port
    }

    func serve() {
        let url = PulseSetup.consoleURL(port: port == 0 ? 50123 : port)
        servingURL = url
        onEvent?(.line(.serving(url)))
    }

    func end(_ how: HubExit) {
        guard exit == nil else { return }
        exit = how
        onEvent?(.exited(how))
    }

    func terminate(grace: TimeInterval) async {
        terminated = true
        end(.stopped)
    }

    func requestStop() {
        terminated = true
        end(.stopped)
    }

    func forceStop() {
        end(.killed(signal: 9))
    }
}

/// launchd, the port and the files, as the test sets them; time passes only
/// when the supervisor sleeps.
@MainActor
final class FakeWorld: HubWorld {
    var setup: PulseSetup
    var clock = Date(timeIntervalSince1970: 1_000_000)
    var files: Set<String> = []
    var job = LaunchdJob(loaded: true, running: true, pid: 10)
    /// What the console's port answers, asked each probe.
    var answer: (URL) -> ProbeResult = { _ in .unreachable("refused") }
    var onLaunch: (FakeHub) -> Void = { $0.serve() }
    var launched: [FakeHub] = []
    var launchPorts: [Int] = []
    var bootstraps = 0
    var kickstarts = 0
    var probes = 0
    var initResult = CommandResult(status: 0)
    var stamp = "1"

    init(service: Bool) {
        let home = URL(fileURLWithPath: "/fake/.pulse", isDirectory: true)
        var setup = PulseSetup(home: home, binary: home.appendingPathComponent("bin/pulse"))
        if service {
            setup.hubService = ServiceUnit(
                label: "pulse.hub", plist: URL(fileURLWithPath: "/fake/pulse.hub.plist"), launcher: nil, environment: [:]
            )
        }
        self.setup = setup
        files = [setup.binary.path, setup.store.path]
    }

    func readSetup() -> PulseSetup { setup }

    func probe(_ url: URL) async -> ProbeResult {
        probes += 1
        await Task.yield()
        return answer(url)
    }

    func job(_ label: String) async -> LaunchdJob { job }
    func kickstart(_ label: String, restart: Bool) async { kickstarts += 1 }
    func bootstrap(_ plist: URL) async { bootstraps += 1 }
    func fileExists(_ url: URL) -> Bool { files.contains(url.path) }
    func childEnvironment(for setup: PulseSetup) async -> [String: String] { ["PULSE_HOME": setup.home.path] }

    func launchHub(_ setup: PulseSetup, port: Int, environment: [String: String]) throws -> any OwnedHub {
        let hub = FakeHub(port: port)
        launched.append(hub)
        launchPorts.append(port)
        onLaunch(hub)
        return hub
    }

    func runInit(_ setup: PulseSetup, folder: URL, environment: [String: String]) async -> CommandResult {
        if initResult.succeeded { files.insert(setup.store.path) }
        return initResult
    }

    func binaryStamp(_ url: URL) -> String? { stamp }
    func logTail(_ url: URL) -> String? { nil }

    func sleep(seconds: Double) async throws {
        try Task.checkCancellation()
        clock = clock.addingTimeInterval(seconds)
        await Task.yield()
        try Task.checkCancellation()
    }

    func now() -> Date { clock }
}

@MainActor
func settle(_ supervisor: HubSupervisor, until condition: (HubState) -> Bool) async -> Bool {
    for _ in 0..<20_000 {
        if condition(supervisor.state) { return true }
        await Task.yield()
    }
    return condition(supervisor.state)
}

let console = URL(string: "http://127.0.0.1:8765/")!

func isReady(_ state: HubState) -> Bool { if case .ready = state { true } else { false } }

func problemKind(_ state: HubState) -> HubProblem.Kind? {
    if case .problem(let problem) = state { problem.kind } else { nil }
}

@MainActor @Suite struct ServiceTests {
    @Test func aServiceThatAnswersIsUsedAsIs() async {
        let world = FakeWorld(service: true)
        world.answer = { _ in .pulse }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        #expect(supervisor.state == .ready(console))
        #expect(world.launched.isEmpty)
        #expect(!supervisor.ownsRunningHub)
    }

    @Test func aRestartingServiceIsWaitedForNeverDoubled() async {
        let world = FakeWorld(service: true)
        var asked = 0
        world.answer = { _ in
            asked += 1
            return asked < 12 ? .unreachable("refused") : .pulse
        }
        let supervisor = HubSupervisor(world: world)
        var seen: [HubState] = []
        supervisor.observe { seen.append($0) }
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        #expect(world.launched.isEmpty)
        #expect(world.kickstarts == 0)
        #expect(world.bootstraps == 0)
        #expect(seen.contains(.connecting("PULSE is restarting…")))
    }

    @Test func anUnloadedServiceIsLoadedOnce() async {
        let world = FakeWorld(service: true)
        world.job = .notLoaded
        var asked = 0
        world.answer = { _ in
            asked += 1
            return asked < 30 ? .unreachable("refused") : .pulse
        }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        #expect(world.bootstraps == 1)
        #expect(world.launched.isEmpty)
    }

    @Test func aStoppedServiceIsStartedOnce() async {
        let world = FakeWorld(service: true)
        world.job = LaunchdJob(loaded: true, running: false, lastExitCode: 1)
        var asked = 0
        world.answer = { _ in
            asked += 1
            return asked < 30 ? .unreachable("refused") : .pulse
        }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        #expect(world.kickstarts == 1)
    }

    @Test func aServiceThatNeverAnswersIsAProblemWithItsLog() async {
        let world = FakeWorld(service: true)
        world.job = LaunchdJob(loaded: true, running: false, lastExitCode: 1)
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: { problemKind($0) != nil }))
        guard case .problem(let problem) = supervisor.state else { return }
        #expect(problem.kind == .serviceDown)
        #expect(problem.detail.contains("code 1"))
        #expect(problem.log == world.setup.serviceLog)
        #expect(world.launched.isEmpty)
    }

    @Test func anotherProgramOnThePortIsSaidPlainly() async {
        let world = FakeWorld(service: true)
        world.answer = { _ in .notPulse("HTTP 200, not PULSE's JSON") }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: { problemKind($0) != nil }))
        #expect(problemKind(supervisor.state) == .portBusy(8765))
    }

    @Test func aMissingBinaryIsSaidBeforeWaiting() async {
        let world = FakeWorld(service: true)
        world.files.remove(world.setup.binary.path)
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: { problemKind($0) != nil }))
        #expect(problemKind(supervisor.state) == .pulseMissing(world.setup.binary))
    }
}

@MainActor @Suite struct OwnedHubTests {
    @Test func aHubStartedInATerminalIsUsedNotOwned() async {
        let world = FakeWorld(service: false)
        world.answer = { _ in .pulse }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        #expect(world.launched.isEmpty)
        #expect(!supervisor.ownsRunningHub)
    }

    @Test func noServiceStartsTheAppsOwnConsoleOn8765() async {
        let world = FakeWorld(service: false)
        world.answer = { url in world.launched.last?.servingURL == url ? .pulse : .unreachable("refused") }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        #expect(world.launchPorts == [8765])
        #expect(supervisor.state == .ready(console))
        #expect(supervisor.ownsRunningHub)
    }

    @Test func missingPulseAndMissingStoreAreSetupSteps() async {
        let noBinary = FakeWorld(service: false)
        noBinary.files.remove(noBinary.setup.binary.path)
        let first = HubSupervisor(world: noBinary)
        first.start()
        #expect(await settle(first, until: { problemKind($0) != nil }))
        #expect(problemKind(first.state) == .pulseMissing(noBinary.setup.binary))

        let noStore = FakeWorld(service: false)
        noStore.files.remove(noStore.setup.store.path)
        let second = HubSupervisor(world: noStore)
        second.start()
        #expect(await settle(second, until: { problemKind($0) != nil }))
        #expect(problemKind(second.state) == .firstProject)
        #expect(noStore.launched.isEmpty)
    }

    @Test func theFirstProjectCreatesTheStoreThenTheConsoleStarts() async {
        let world = FakeWorld(service: false)
        world.files.remove(world.setup.store.path)
        world.answer = { url in world.launched.last?.servingURL == url ? .pulse : .unreachable("refused") }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: { problemKind($0) == .firstProject }))
        let result = await supervisor.createFirstProject(at: URL(fileURLWithPath: "/Users/me/app"))
        #expect(result.succeeded)
        #expect(await settle(supervisor, until: isReady))
        #expect(world.launched.count == 1)
    }

    @Test func aHubThatFindsNoStoreSendsTheOwnerToTheFirstProject() async {
        let world = FakeWorld(service: false)
        world.onLaunch = { $0.end(.noStore) }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: { problemKind($0) != nil }))
        #expect(problemKind(supervisor.state) == .firstProject)
    }

    @Test func anotherProgramOn8765MovesTheConsoleToAFreePort() async {
        let world = FakeWorld(service: false)
        world.answer = { url in
            if url == console { return .notPulse("HTTP 200") }
            return world.launched.last?.servingURL == url ? .pulse : .unreachable("refused")
        }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        #expect(world.launchPorts == [0])
        #expect(supervisor.state == .ready(URL(string: "http://127.0.0.1:50123/")!))
    }

    @Test func aPortTakenByPulseInTheMeantimeIsUsed() async {
        let world = FakeWorld(service: false)
        var portIsPulse = false
        world.onLaunch = { hub in
            portIsPulse = true
            hub.end(.portTaken)
        }
        world.answer = { _ in portIsPulse ? .pulse : .unreachable("refused") }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        #expect(world.launched.count == 1)
        #expect(!supervisor.ownsRunningHub)
    }

    @Test func theHandoverMovesTheAppToTheService() async {
        let world = FakeWorld(service: false)
        world.answer = { url in world.launched.last?.servingURL == url ? .pulse : .unreachable("refused") }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        let hub = world.launched[0]

        // The console's "Keep PULSE running": the service is installed, the
        // app's console hands over and exits 0; the service answers next.
        world.setup.hubService = ServiceUnit(
            label: "pulse.hub", plist: URL(fileURLWithPath: "/fake/pulse.hub.plist"), launcher: nil, environment: [:]
        )
        var serviceUp = false
        world.answer = { _ in serviceUp ? .pulse : .unreachable("refused") }
        hub.onEvent?(.line(.handingOver))
        hub.end(.handedOver)
        #expect(await settle(supervisor, until: { if case .reconnecting = $0 { true } else { false } }))
        serviceUp = true
        #expect(await settle(supervisor, until: isReady))
        #expect(world.launched.count == 1, "never a second console after a handover")
        #expect(!supervisor.ownsRunningHub)
    }

    @Test func aConsoleThatStopsIsStartedAgainThenGivenUpOn() async {
        let world = FakeWorld(service: false)
        world.answer = { url in
            guard let hub = world.launched.last, hub.exit == nil else { return .unreachable("refused") }
            return hub.servingURL == url ? .pulse : .unreachable("refused")
        }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))

        world.launched.last?.end(.killed(signal: 9))
        #expect(await settle(supervisor, until: { _ in world.launched.count == 2 }))
        #expect(await settle(supervisor, until: isReady))

        for expected in 3...4 {
            world.launched.last?.end(.failed(code: 1))
            #expect(await settle(supervisor, until: { _ in world.launched.count == expected }))
            #expect(await settle(supervisor, until: isReady))
        }
        world.launched.last?.end(.failed(code: 1))
        #expect(await settle(supervisor, until: { problemKind($0) != nil }))
        #expect(problemKind(supervisor.state) == .hubStopped)
    }

    @Test func quittingWithoutAwaitingStopsTheConsoleToo() async {
        let world = FakeWorld(service: false)
        world.answer = { url in world.launched.last?.servingURL == url ? .pulse : .unreachable("refused") }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        #expect(!supervisor.ownedHubStopped)
        supervisor.beginShutdown()
        #expect(supervisor.ownedHubStopped)
        #expect(world.launched[0].terminated)
        for _ in 0..<200 { await Task.yield() }
        #expect(world.launched.count == 1, "a stop at quit is not a crash to restart")
    }

    @Test func quittingStopsOnlyTheAppsOwnConsole() async {
        let world = FakeWorld(service: false)
        world.answer = { url in world.launched.last?.servingURL == url ? .pulse : .unreachable("refused") }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        await supervisor.shutdown()
        #expect(world.launched[0].terminated)
        #expect(world.launched.count == 1, "a stop at quit is not a crash to restart")
    }

    @Test func aNewBinaryRestartsTheAppsConsole() async {
        let world = FakeWorld(service: false)
        world.answer = { url in
            guard let hub = world.launched.last, hub.exit == nil else { return .unreachable("refused") }
            return hub.servingURL == url ? .pulse : .unreachable("refused")
        }
        let supervisor = HubSupervisor(world: world)
        supervisor.watchInterval = 1
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        world.stamp = "2"
        #expect(await settle(supervisor, until: { _ in world.launched.count == 2 }))
        #expect(world.launched[0].terminated)
        #expect(world.launched[0].exit != nil, "the old console is gone before the new one starts")
        #expect(await settle(supervisor, until: isReady))
    }

    @Test func aServiceInstalledFromATerminalGetsThePort() async {
        let world = FakeWorld(service: false)
        var serviceServes = false
        world.answer = { url in
            if serviceServes { return .pulse }
            guard let hub = world.launched.last, hub.exit == nil else { return .unreachable("refused") }
            return hub.servingURL == url ? .pulse : .unreachable("refused")
        }
        let supervisor = HubSupervisor(world: world)
        supervisor.watchInterval = 5
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        #expect(world.launched.count == 1)
        // `pulse service install` in a terminal: no handover, the service
        // waits for the port the app's console holds.
        world.setup.hubService = ServiceUnit(
            label: "pulse.hub", plist: URL(fileURLWithPath: "/fake/pulse.hub.plist"), launcher: nil, environment: [:]
        )
        let started = world.clock
        #expect(await settle(supervisor, until: { _ in world.launched.first?.terminated == true }))
        #expect(world.clock.timeIntervalSince(started) >= 60, "a console handover gets a minute first")
        serviceServes = true
        #expect(await settle(supervisor, until: isReady))
        #expect(world.launched.count == 1)
        #expect(!supervisor.ownsRunningHub)
    }
}

@MainActor @Suite struct WakeTests {
    @Test func aWaitThatSpannedSleepStartsAgain() async {
        let world = FakeWorld(service: true)
        world.job = LaunchdJob(loaded: true, running: false, lastExitCode: 1)
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: { problemKind($0) == .serviceDown }))
        world.answer = { _ in .pulse }
        supervisor.checkAfterWake()
        #expect(await settle(supervisor, until: isReady))
    }

    @Test func setupStepsWaitForTheOwner() async {
        let world = FakeWorld(service: false)
        world.files.remove(world.setup.store.path)
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: { problemKind($0) == .firstProject }))
        supervisor.checkAfterWake()
        for _ in 0..<200 { await Task.yield() }
        #expect(problemKind(supervisor.state) == .firstProject)
        #expect(world.launched.isEmpty)
    }
}

@MainActor @Suite struct ReconnectTests {
    @Test func aBlipIsReannouncedSoWindowsRetry() async {
        let world = FakeWorld(service: true)
        world.answer = { _ in .pulse }
        let supervisor = HubSupervisor(world: world)
        var readies = 0
        supervisor.observe { if case .ready = $0 { readies += 1 } }
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        supervisor.consoleLooksDown()
        #expect(await settle(supervisor, until: { _ in readies == 2 }))
    }

    @Test func aConsoleThatWentAwayIsReconnectingThenReady() async {
        let world = FakeWorld(service: true)
        world.answer = { _ in .pulse }
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: isReady))
        var down = 0
        world.answer = { _ in
            down += 1
            return down < 6 ? .unreachable("refused") : .pulse
        }
        supervisor.consoleLooksDown()
        #expect(await settle(supervisor, until: { if case .reconnecting(console, _) = $0 { true } else { false } }))
        #expect(await settle(supervisor, until: isReady))
        #expect(world.launched.isEmpty)
    }

    @Test func anOverrideIsWaitedForAndNeverStarted() async {
        let world = FakeWorld(service: false)
        let mock = URL(string: "http://127.0.0.1:8787/")!
        world.setup.hubOverride = mock
        let supervisor = HubSupervisor(world: world)
        supervisor.start()
        #expect(await settle(supervisor, until: { problemKind($0) != nil }))
        #expect(problemKind(supervisor.state) == .unreachable(mock))
        world.answer = { $0 == mock ? .pulse : .unreachable("refused") }
        supervisor.retry()
        #expect(await settle(supervisor, until: isReady))
        #expect(supervisor.state == .ready(mock))
        #expect(world.launched.isEmpty)
    }
}
