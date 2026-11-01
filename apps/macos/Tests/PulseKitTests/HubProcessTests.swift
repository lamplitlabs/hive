import Darwin
import Foundation
import Testing
@testable import PulseKit

/// A stand-in `pulse` that says what the real hub says, in the order it says it.
private let fakePulse = """
#!/bin/sh
# argv: hub --port N --no-browser
port="$3"
case "$FAKE_MODE" in
serve)
  echo "seeded 3 framework record(s) from bundle"
  echo "Serving on http://127.0.0.1:$port (127.0.0.1 only; writes: decide; Ctrl-C to stop)"
  trap 'echo "stopping"; exit 0' TERM
  while :; do sleep 0.05; done ;;
handover)
  echo "Serving on http://127.0.0.1:$port (127.0.0.1 only; Ctrl-C to stop)"
  echo "handing over to the pulse.hub service"
  printf 'the pulse.hub service now serves http://127.0.0.1:%s; this hub exits' "$port"
  exit 0 ;;
taken)
  echo "The hub could not listen" >&2
  echo "  why: listen tcp4 127.0.0.1:$port: bind: address already in use" >&2
  exit 1 ;;
nostore)
  echo "No store at $PULSE_HOME/pulse.sqlite" >&2
  exit 2 ;;
changed)
  echo "Serving on http://127.0.0.1:$port (127.0.0.1 only; Ctrl-C to stop)"
  echo "pulse changed on disk; exiting 3 so the service restarts the hub" >&2
  exit 3 ;;
stubborn)
  echo "Serving on http://127.0.0.1:$port (127.0.0.1 only; Ctrl-C to stop)"
  trap '' TERM
  while :; do sleep 0.05; done ;;
esac
"""

@MainActor @Suite(.serialized) struct HubProcessTests {
    let folder: URL
    let binary: URL
    let log: URL

    init() throws {
        folder = FileManager.default.temporaryDirectory.appendingPathComponent("pulsekit-hub-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        binary = folder.appendingPathComponent("pulse")
        try fakePulse.write(to: binary, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: binary.path)
        log = folder.appendingPathComponent("logs/hub-app.log")
    }

    func launch(_ mode: String, port: Int = 8799) throws -> HubProcess {
        try HubProcess.launch(
            binary: binary, port: port,
            environment: ["FAKE_MODE": mode, "PATH": "/usr/bin:/bin", "PULSE_HOME": folder.path],
            log: log, workingDirectory: folder
        )
    }

    func eventually(_ seconds: Double = 5, _ condition: () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            if condition() { return true }
            try? await Task.sleep(nanoseconds: 20_000_000)
        }
        return condition()
    }

    @Test func servesThenStopsOnSIGTERM() async throws {
        let hub = try launch("serve")
        var lines: [HubLine] = []
        hub.onEvent = { if case .line(let line) = $0 { lines.append(line) } }
        #expect(await eventually { hub.servingURL != nil })
        #expect(hub.servingURL == URL(string: "http://127.0.0.1:8799/"))
        #expect(lines.contains(.serving(URL(string: "http://127.0.0.1:8799/")!)))
        await hub.terminate(grace: 5)
        #expect(hub.exit == .stopped)
        let written = try String(contentsOf: log, encoding: .utf8)
        #expect(written.contains("PULSE.app started"))
        #expect(written.contains("Serving on http://127.0.0.1:8799"))
        #expect(written.contains("stopped"))
    }

    @Test func aHandoverIsNotAStopEvenWithoutATrailingNewline() async throws {
        let hub = try launch("handover")
        #expect(await eventually { hub.exit != nil })
        #expect(hub.exit == .handedOver)
    }

    @Test func aTakenPortAndAMissingStoreAreRecognised() async throws {
        let taken = try launch("taken")
        #expect(await eventually { taken.exit != nil })
        #expect(taken.exit == .portTaken)
        let noStore = try launch("nostore")
        #expect(await eventually { noStore.exit != nil })
        #expect(noStore.exit == .noStore)
        let changed = try launch("changed")
        #expect(await eventually { changed.exit != nil })
        #expect(changed.exit == .binaryChanged)
    }

    @Test func aHubThatIgnoresSIGTERMIsKilled() async throws {
        let hub = try launch("stubborn")
        #expect(await eventually { hub.servingURL != nil })
        await hub.terminate(grace: 0.5)
        #expect(hub.exit == .killed(signal: SIGKILL))
    }

    @Test func theHubRunsAsItsOwnProcessGroup() async throws {
        let hub = try launch("serve")
        #expect(await eventually { hub.servingURL != nil })
        #expect(getpgid(hub.pid) == hub.pid)
        await hub.terminate(grace: 5)
    }

    @Test func theWatchdogStopsAHubTheAppLeftBehind() async throws {
        // A process the app started, and a watchdog whose pipe the "app"
        // drops without disarming it: what happens when the app crashes.
        let sleeper = spawnProcess(
            path: "/bin/sleep", arguments: ["/bin/sleep", "30"], environment: [:], workingDirectory: nil,
            stdin: .devNull, output: nil
        )
        guard case .success(let pid) = sleeper else {
            Issue.record("could not start sleep")
            return
        }
        let watchdog = try #require(Watchdog.arm(for: pid))
        watchdog.abandonForTesting()
        var status: Int32 = 0
        let ended = await eventually { waitpid(pid, &status, WNOHANG) == pid }
        #expect(ended)
        #expect(status & 0x7f == SIGTERM)
    }

    @Test func aDisarmedWatchdogLeavesTheProcessAlone() async throws {
        let sleeper = spawnProcess(
            path: "/bin/sleep", arguments: ["/bin/sleep", "30"], environment: [:], workingDirectory: nil,
            stdin: .devNull, output: nil
        )
        guard case .success(let pid) = sleeper else {
            Issue.record("could not start sleep")
            return
        }
        let watchdog = try #require(Watchdog.arm(for: pid))
        watchdog.disarm()
        try await Task.sleep(nanoseconds: 300_000_000)
        var status: Int32 = 0
        #expect(waitpid(pid, &status, WNOHANG) == 0, "still running")
        kill(pid, SIGKILL)
        _ = waitpid(pid, &status, 0)
    }
}
