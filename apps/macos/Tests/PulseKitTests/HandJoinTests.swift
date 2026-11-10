import Darwin
import Foundation
import Testing
@testable import PulseKit

/// A brain that answers `POST /api/v1/hands/join` the way
/// crates/pulse-runner/src/dial.rs does: 403 with `error`/`why`/`fix` for a
/// code it did not issue (or one already used), 200 with a device id and
/// the executor for the one it did - once.
final class StubBrain: @unchecked Sendable {
    let port: Int
    let goodCode: String
    private let listener: Int32
    private let lock = NSLock()
    private var used = false
    private var bodies: [String] = []

    init(goodCode: String) throws {
        self.goodCode = goodCode
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { throw POSIXError(.EIO) }
        var yes: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout<Int32>.size))
        var address = sockaddr_in()
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        address.sin_port = 0
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        guard bound == 0, listen(fd, 8) == 0 else {
            close(fd)
            throw POSIXError(.EADDRINUSE)
        }
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        withUnsafeMutablePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { _ = getsockname(fd, $0, &length) }
        }
        listener = fd
        port = Int(UInt16(bigEndian: address.sin_port))
        Thread.detachNewThread { [self] in serve() }
    }

    var url: URL { URL(string: "http://127.0.0.1:\(port)")! }

    /// What the hands posted, in order.
    var received: [String] {
        lock.lock()
        defer { lock.unlock() }
        return bodies
    }

    func stop() {
        shutdown(listener, SHUT_RDWR)
        close(listener)
    }

    private func serve() {
        while true {
            let client = accept(listener, nil, nil)
            if client < 0 { return }
            handle(client)
            close(client)
        }
    }

    private func handle(_ client: Int32) {
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        var wanted: Int?
        var headerEnd = 0
        while true {
            let n = read(client, &buffer, buffer.count)
            if n <= 0 { break }
            data.append(buffer, count: n)
            if wanted == nil, let end = data.range(of: Data("\r\n\r\n".utf8)) {
                headerEnd = end.upperBound
                let head = String(decoding: data[..<end.lowerBound], as: UTF8.self).lowercased()
                let length = head.split(separator: "\r\n")
                    .first(where: { $0.hasPrefix("content-length:") })
                    .flatMap { Int($0.dropFirst("content-length:".count).trimmingCharacters(in: .whitespaces)) } ?? 0
                wanted = headerEnd + length
            }
            if let wanted, data.count >= wanted { break }
        }
        let head = String(decoding: data.prefix(headerEnd), as: UTF8.self)
        let body = String(decoding: data.dropFirst(headerEnd), as: UTF8.self)
        let (status, answer) = respond(requestLine: head.split(separator: "\r\n").first.map(String.init) ?? "", body: body)
        let payload = Data(answer.utf8)
        let reply = "HTTP/1.1 \(status) \(status == 200 ? "OK" : "Forbidden")\r\nContent-Type: application/json\r\nContent-Length: \(payload.count)\r\nConnection: close\r\n\r\n"
        var out = Data(reply.utf8)
        out.append(payload)
        out.withUnsafeBytes { _ = write(client, $0.baseAddress, out.count) }
    }

    private func respond(requestLine: String, body: String) -> (Int, String) {
        lock.lock()
        defer { lock.unlock() }
        bodies.append(body)
        guard requestLine.hasPrefix("POST /api/v1/hands/join ") else {
            return (404, #"{"error":"no such route"}"#)
        }
        let json = (try? JSONSerialization.jsonObject(with: Data(body.utf8))) as? [String: Any] ?? [:]
        let code = (json["code"] as? String ?? "").lowercased()
            .split(whereSeparator: { !("a"..."z").contains($0) }).joined(separator: "-")
        guard code == goodCode else {
            return (403, #"{"error":"That join code is not one this brain issued","why":"That join code is not one this brain issued","fix":"Check the six words, or ask the Machines page for a new code."}"#)
        }
        guard !used else {
            return (403, #"{"error":"That join code was already used","why":"That join code was already used","fix":"A code enrols one machine; ask the Machines page for a new one."}"#)
        }
        used = true
        return (200, #"{"device":"dev-0a1b2c3d4e5f","executor":"studio-mac"}"#)
    }
}

/// A stand-in `pulse-hand` for a machine with no Rust build at hand: the same
/// `join` as crates/pulse-hand-core/src/dial.rs - a device key from
/// ssh-keygen, the JSON post through curl, the same refusal line, and
/// device.json on success. `PULSE_HAND_BIN` (scripts/check.sh) swaps in the
/// real binary.
private let standInHand = #"""
#!/bin/sh
[ "$1" = join ] || { echo "pulse-hands: verb not allowed: $1" >&2; exit 1; }
shift
brain=""
while [ $# -gt 1 ]; do
  case "$1" in --brain) brain="$2"; shift 2 ;; *) shift ;; esac
done
code="$1"
home="${PULSE_HAND_HOME:?}"
mkdir -p "$home/work" && chmod 700 "$home"
[ -f "$home/device_key" ] || ssh-keygen -q -t ed25519 -N "" -C pulse-hand -f "$home/device_key" </dev/null
pub=$(cat "$home/device_key.pub")
out=$(printf '{"code":"%s","public_key":"%s","name":"stand-in","os":"macos","arch":"arm64","labels":"","workers":1,"workdir":"%s/work"}' \
  "$code" "$pub" "$home" | curl -sS -X POST -H 'Content-Type: application/json' --data-binary @- -w '\n%{http_code}' "${brain%/}/api/v1/hands/join" 2>&1)
rc=$?
if [ $rc -ne 0 ]; then echo "pulse-hand join: ${brain%/}/api/v1/hands/join: curl exit $rc: $out" >&2; exit 1; fi
status=$(printf '%s\n' "$out" | tail -n 1)
body=$(printf '%s\n' "$out" | sed '$d')
field() { printf '%s' "$body" | sed -n "s/.*\"$1\":\"\([^\"]*\)\".*/\1/p"; }
if [ "$status" != 200 ]; then
  echo "pulse-hand join: join refused ($status): $(field error) - $(field fix)" >&2
  exit 1
fi
printf '{\n  "brain": "%s",\n  "device": "%s",\n  "executor": "%s",\n  "workdir": "%s/work"\n}\n' \
  "${brain%/}" "$(field device)" "$(field executor)" "$home" > "$home/device.json"
chmod 600 "$home/device.json"
echo "joined ${brain%/} as $(field executor) (device $(field device)); jobs run under $home/work"
"""#

@Suite(.serialized) struct HandJoinTests {
    static let goodCode = "amber-river-cobalt-maple-orbit-lantern"

    /// A scratch hand home and the pulse-hand to join with: the real one
    /// when PULSE_HAND_BIN names it, else the stand-in.
    static func scratch() throws -> (HandSetup, URL) {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("pulse-hand-join-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let binary: URL
        if let real = ProcessInfo.processInfo.environment["PULSE_HAND_BIN"], !real.isEmpty {
            binary = URL(fileURLWithPath: real)
        } else {
            binary = dir.appendingPathComponent("pulse-hand")
            try standInHand.write(to: binary, atomically: true, encoding: .utf8)
            chmod(binary.path, 0o755)
        }
        return (HandSetup(home: dir.appendingPathComponent("hand", isDirectory: true), binary: binary), dir)
    }

    static func base() -> [String: String] {
        var env = ProcessInfo.processInfo.environment
        env.removeValue(forKey: "PULSE_BRAIN")
        return env
    }

    @Test func aWrongCodeShowsTheBrainsRefusalAndWritesNothing() async throws {
        let brain = try StubBrain(goodCode: Self.goodCode)
        defer { brain.stop() }
        let (setup, dir) = try Self.scratch()
        defer { try? FileManager.default.removeItem(at: dir) }

        let input = try JoinInput.parse(brain: brain.url.absoluteString, code: "Amber River Cobalt Maple Orbit Comet").get()
        let outcome = await HandJoin.run(input, setup: setup, base: Self.base())

        #expect(outcome == .refused(
            status: 403,
            why: "That join code is not one this brain issued",
            fix: "Check the six words, or ask the Machines page for a new code."
        ))
        #expect(outcome.words == "That join code is not one this brain issued. Check the six words, or ask the Machines page for a new code.")
        #expect(!FileManager.default.fileExists(atPath: setup.deviceFile.path))
        #expect(setup.device() == nil)
        #expect(brain.received.count == 1)
    }

    @Test func aGoodCodeWritesDeviceJSONAndIsGoodOnce() async throws {
        let brain = try StubBrain(goodCode: Self.goodCode)
        defer { brain.stop() }
        let (setup, dir) = try Self.scratch()
        defer { try? FileManager.default.removeItem(at: dir) }

        // The Machines page's whole line pasted into the code field.
        let input = try JoinInput.parse(brain: "", code: "pulse-hand join --brain \(brain.url.absoluteString)/ \(Self.goodCode)").get()
        #expect(input.brain == brain.url)
        let outcome = await HandJoin.run(input, setup: setup, base: Self.base())

        let device = try #require(setup.device())
        #expect(outcome == .joined(device))
        #expect(device.brain == brain.url.absoluteString)
        #expect(device.device == "dev-0a1b2c3d4e5f")
        #expect(device.executor == "studio-mac")
        #expect(device.workdir.hasPrefix(setup.home.path))
        let mode = try FileManager.default.attributesOfItem(atPath: setup.deviceFile.path)[.posixPermissions] as? NSNumber
        #expect(mode?.intValue == 0o600)

        // What the brain got: the code and the public half of a fresh ed25519 key, never the private one.
        let sent = try #require(brain.received.first)
        let body = try #require(try JSONSerialization.jsonObject(with: Data(sent.utf8)) as? [String: Any])
        #expect(body["code"] as? String == Self.goodCode)
        #expect((body["public_key"] as? String)?.hasPrefix("ssh-ed25519 ") == true)
        #expect(!sent.contains("PRIVATE KEY"))

        // A code enrols one machine.
        let again = await HandJoin.run(input, setup: setup, base: Self.base())
        #expect(again == .refused(
            status: 403,
            why: "That join code was already used",
            fix: "A code enrols one machine; ask the Machines page for a new one."
        ))
    }

    @Test func aBrainThatDoesNotAnswerSaysSo() async throws {
        let (setup, dir) = try Self.scratch()
        defer { try? FileManager.default.removeItem(at: dir) }
        let input = try JoinInput.parse(brain: "http://127.0.0.1:9", code: Self.goodCode).get()
        let outcome = await HandJoin.run(input, setup: setup, base: Self.base())
        guard case .unreachable(let why) = outcome else {
            Issue.record("expected unreachable, got \(outcome)")
            return
        }
        #expect(why.contains("127.0.0.1"))
        #expect(setup.device() == nil)
    }

    @Test func noPulseHandIsSaidPlainly() async throws {
        let input = try JoinInput.parse(brain: "https://pulse.example.com", code: Self.goodCode).get()
        let setup = HandSetup(home: URL(fileURLWithPath: "/nonexistent/hand"), binary: nil)
        let outcome = await HandJoin.run(input, setup: setup, base: [:])
        guard case .failed(let why) = outcome else {
            Issue.record("expected failed, got \(outcome)")
            return
        }
        #expect(why.contains("no pulse-hand"))
    }
}

@Suite struct JoinInputTests {
    @Test func readsTheCodeInAnySpelling() throws {
        for typed in ["amber river cobalt maple orbit lantern", "Amber-River-Cobalt-Maple-Orbit-Lantern", " amber  river\ncobalt-maple orbit,lantern "] {
            let input = try JoinInput.parse(brain: "https://pulse.example.com", code: typed).get()
            #expect(input.code == "amber-river-cobalt-maple-orbit-lantern")
        }
    }

    @Test func takesTheMachinesPagesWholeLine() throws {
        let line = "pulse-hand join --brain http://studio.local:8765 amber-river-cobalt-maple-orbit-lantern"
        let input = try JoinInput.parse(brain: "", code: line).get()
        #expect(input.brain.absoluteString == "http://studio.local:8765")
        #expect(input.code == "amber-river-cobalt-maple-orbit-lantern")
        // A brain typed in its own field wins over the pasted line's.
        let typed = try JoinInput.parse(brain: "https://pulse.example.com", code: line).get()
        #expect(typed.brain.absoluteString == "https://pulse.example.com")
        let equals = try JoinInput.parse(brain: "", code: "pulse-hand join --brain=https://b.example.com/ a b c d e f").get()
        #expect(equals.brain.absoluteString == "https://b.example.com")
    }

    @Test func readsTheBrainLikeAnAddress() {
        #expect(JoinInput.brainURL("pulse.example.com")?.absoluteString == "https://pulse.example.com")
        #expect(JoinInput.brainURL("https://pulse.example.com/")?.absoluteString == "https://pulse.example.com")
        #expect(JoinInput.brainURL("127.0.0.1:8765")?.absoluteString == "http://127.0.0.1:8765")
        #expect(JoinInput.brainURL("studio.local")?.absoluteString == "http://studio.local")
        #expect(JoinInput.brainURL("pulse") == nil)
        #expect(JoinInput.brainURL("ftp://pulse.example.com") == nil)
    }

    @Test func saysWhatIsMissing() {
        #expect(JoinInput.parse(brain: "", code: "a b c d e f") == .failure(.noBrain))
        #expect(JoinInput.parse(brain: "not an address", code: "a b c d e f") == .failure(.badBrain))
        #expect(JoinInput.parse(brain: "pulse.example.com", code: "") == .failure(.badCode(words: 0)))
        #expect(JoinInput.parse(brain: "pulse.example.com", code: "amber river cobalt") == .failure(.badCode(words: 3)))
        #expect(JoinInput.Problem.badCode(words: 3).words.contains("six words; that is 3"))
    }

    @Test func runsJoinOnlyInTheHandsHome() throws {
        let input = try JoinInput.parse(brain: "https://pulse.example.com", code: "a b c d e f").get()
        #expect(HandJoin.arguments(input) == ["join", "--brain", "https://pulse.example.com", "a-b-c-d-e-f"])
        let env = HandJoin.environment(base: ["PATH": "/usr/bin", "__CFBundleIdentifier": "x"], home: URL(fileURLWithPath: "/h/.pulse-hand"))
        #expect(env["PULSE_HAND_HOME"] == "/h/.pulse-hand")
        #expect(env["PULSE_HAND_JOIN_ONLY"] == "1")
        #expect(env["__CFBundleIdentifier"] == nil)
        #expect(HandJoin.environment(base: [:], home: URL(fileURLWithPath: "/h"))["PATH"] == "/usr/bin:/bin:/usr/sbin:/sbin")
    }

    @Test func readsWhatPulseHandSaid() {
        let refused = CommandResult(status: 1, stderr: "pulse-hand join: join refused (403): That join code has expired - Codes last ten minutes; ask the Machines page for a new one.\n")
        #expect(HandJoin.outcome(refused, device: nil) == .refused(
            status: 403, why: "That join code has expired", fix: "Codes last ten minutes; ask the Machines page for a new one."
        ))
        let old = CommandResult(status: 1, stderr: "pulse-hands: verb not allowed: join")
        guard case .failed(let words) = HandJoin.outcome(old, device: nil) else {
            Issue.record("an old pulse-hand is a failure")
            return
        }
        #expect(words.contains("older than joining with a code"))
        let slow = CommandResult(status: 15, timedOut: true)
        guard case .unreachable = HandJoin.outcome(slow, device: nil) else {
            Issue.record("a timeout is unreachable")
            return
        }
        let device = HandDevice(brain: "https://b", device: "dev-1", executor: "mac", workdir: "/w")
        #expect(HandJoin.outcome(CommandResult(status: 0, stdout: "joined"), device: device) == .joined(device))
        #expect(HandJoin.outcome(CommandResult(status: 0, stdout: "joined"), device: nil) != .joined(device))
    }
}

@Suite struct HandSetupTests {
    let user = URL(fileURLWithPath: "/Users/me", isDirectory: true)

    @Test func prefersTheBundledHand() {
        let app = URL(fileURLWithPath: "/Applications/PULSE.app")
        let setup = HandSetup.read(
            homeOverride: nil, binaryOverride: nil, environment: [:], userHome: user, bundle: app,
            isExecutable: { _ in true }
        )
        #expect(setup.home.path == "/Users/me/.pulse-hand")
        #expect(setup.binary?.path == "/Applications/PULSE.app/Contents/Helpers/pulse-hand")
    }

    @Test func overridesAndEnvironment() {
        let env = HandSetup.read(
            homeOverride: nil, binaryOverride: nil, environment: ["PULSE_HAND_HOME": "/srv/hand"], userHome: user, bundle: nil,
            isExecutable: { $0.path == "/Users/me/.cargo/bin/pulse-hand" }
        )
        #expect(env.home.path == "/srv/hand")
        #expect(env.binary?.path == "/Users/me/.cargo/bin/pulse-hand")
        let chosen = HandSetup.read(
            homeOverride: "~/hand", binaryOverride: "~/bin/pulse-hand", environment: ["PULSE_HAND_HOME": "/srv/hand"],
            userHome: user, bundle: nil, isExecutable: { _ in false }
        )
        #expect(chosen.home.path == "/Users/me/hand")
        #expect(chosen.binary?.path == "/Users/me/bin/pulse-hand")
        let none = HandSetup.read(homeOverride: nil, binaryOverride: nil, environment: [:], userHome: user, bundle: nil, isExecutable: { _ in false })
        #expect(none.binary == nil)
    }
}

@Suite(.serialized) struct DialAgentTests {
    final class Calls: @unchecked Sendable {
        var all: [[String]] = []
    }

    @Test func theAgentDialsAtLoginAndStaysDownWhenRefused() {
        let setup = HandSetup(home: URL(fileURLWithPath: "/Users/me/.pulse-hand"), binary: URL(fileURLWithPath: "/x/pulse-hand"))
        let plist = DialAgent.plist(binary: URL(fileURLWithPath: "/x/pulse-hand"), setup: setup, path: "/usr/bin:/bin")
        #expect(plist["Label"] as? String == "pulse.hand")
        #expect(plist["RunAtLoad"] as? Bool == true)
        #expect((plist["KeepAlive"] as? [String: Bool])?["SuccessfulExit"] == false)
        #expect((plist["EnvironmentVariables"] as? [String: String])?["PULSE_HAND_HOME"] == "/Users/me/.pulse-hand")
        #expect(plist["StandardErrorPath"] as? String == "/Users/me/.pulse-hand/dial.log")
        let args = plist["ProgramArguments"] as? [String] ?? []
        #expect(args.last == "/x/pulse-hand")
        #expect(args.contains { $0.contains("dial") && $0.contains("-eq 3") })
    }

    @Test func refusedDialExitsCleanlyOtherFailuresDoNot() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("pulse-dial-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        for (code, expected) in [(3, Int32(0)), (1, Int32(1)), (0, Int32(0))] {
            let fake = dir.appendingPathComponent("hand-\(code)")
            try "#!/bin/sh\n[ \"$1\" = dial ] || exit 9\nexit \(code)\n".write(to: fake, atomically: true, encoding: .utf8)
            chmod(fake.path, 0o755)
            let args = DialAgent.programArguments(binary: fake)
            let result = await Command.run(args[0], Array(args.dropFirst()), timeout: 10)
            #expect(result.status == expected, "dial exit \(code)")
        }
    }

    @Test func installWritesTheAgentCopiesABundledHandAndLoadsIt() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("pulse-agent-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        let helpers = dir.appendingPathComponent("PULSE.app/Contents/Helpers", isDirectory: true)
        try FileManager.default.createDirectory(at: helpers, withIntermediateDirectories: true)
        let bundled = helpers.appendingPathComponent("pulse-hand")
        try "#!/bin/sh\nexit 0\n".write(to: bundled, atomically: true, encoding: .utf8)
        chmod(bundled.path, 0o755)
        let setup = HandSetup(home: dir.appendingPathComponent("hand", isDirectory: true), binary: bundled)
        let agents = dir.appendingPathComponent("LaunchAgents", isDirectory: true)
        let calls = Calls()

        let problem = await DialAgent.install(setup: setup, agents: agents, path: "/usr/bin:/bin") { args in
            calls.all.append(args)
            return CommandResult(status: 0)
        }

        #expect(problem == nil)
        #expect(calls.all.map(\.first) == ["bootout", "bootstrap"])
        #expect(calls.all.last?.last == agents.appendingPathComponent("pulse.hand.plist").path)
        // The agent runs the hand's own copy, so moving PULSE.app does not stop it.
        #expect(DialAgent.installedArguments(agents: agents)?.last == setup.installedBinary.path)
        #expect(FileManager.default.isExecutableFile(atPath: setup.installedBinary.path))

        let failing = await DialAgent.install(setup: setup, agents: agents, path: "/usr/bin") { args in
            CommandResult(status: args.first == "bootstrap" ? 5 : 0, stderr: "Bootstrap failed: 5: Input/output error")
        }
        #expect(failing?.contains("Bootstrap failed") == true)
    }
}
