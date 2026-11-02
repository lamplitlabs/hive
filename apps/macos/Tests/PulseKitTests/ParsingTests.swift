import Foundation
import Testing
@testable import PulseKit

@Suite struct LauncherTests {
    // What crates/pulse-runner/src/service.rs writes as ~/.pulse/bin/pulse-hub.sh.
    static let launcher = """
    #!/bin/sh
    # Written by `pulse service install`. The service runs pulse through this script so that a
    # restart is a loop here, not a launchd/systemd throttle, and the crash guard has a place to
    # live. macOS holds pulse itself responsible for reading your folders (ADR-035).
    /Users/me/.pulse/bin/pulse hub --port 8765 --no-browser --restart-on-change &
    pid=$!
    trap 'kill -TERM "$pid" 2>/dev/null' TERM INT HUP
    wait "$pid"
    """

    @Test func readsThePortAndBinaryOfTheHubLine() {
        #expect(SetupReader.port(fromLauncher: Self.launcher) == 8765)
        #expect(SetupReader.binaryPath(fromLauncher: Self.launcher) == "/Users/me/.pulse/bin/pulse")
    }

    @Test func readsOtherPortSpellings() {
        #expect(SetupReader.port(fromLauncher: "/x/pulse hub --no-browser --port=9001 &") == 9001)
        #expect(SetupReader.port(fromLauncher: "/x/pulse --home /h hub -port 9002") == 9002)
    }

    @Test func ignoresCommentsAndOtherCommands() {
        let text = "# pulse hub --port 1234\n/x/pulse daemon --interval 300\n"
        #expect(SetupReader.port(fromLauncher: text) == nil)
        #expect(SetupReader.binaryPath(fromLauncher: text) == nil)
        #expect(SetupReader.port(fromLauncher: "/x/pulse hub --port 99999") == nil)
    }

    // `pulse service install` quotes a path with spaces the shell's way
    // (crates/pulse-runner/src/util.rs, shell_quote).
    @Test func readsAQuotedPathWithSpaces() {
        let quoted = "'/Volumes/My Disk/pulse/bin/pulse' hub --port 8790 --no-browser --restart-on-change &\n"
        #expect(SetupReader.binaryPath(fromLauncher: quoted) == "/Volumes/My Disk/pulse/bin/pulse")
        #expect(SetupReader.port(fromLauncher: quoted) == 8790)
        let doubleQuoted = "\"/Users/Jo Bloggs/.pulse/bin/pulse\" hub --port 8765 &"
        #expect(SetupReader.binaryPath(fromLauncher: doubleQuoted) == "/Users/Jo Bloggs/.pulse/bin/pulse")
        let escaped = "/Users/Jo\\ Bloggs/.pulse/bin/pulse hub --port 8765 &"
        #expect(SetupReader.binaryPath(fromLauncher: escaped) == "/Users/Jo Bloggs/.pulse/bin/pulse")
        #expect(SetupReader.shellWords("a 'b c' \"d e\" f\\ g # not this") == ["a", "b c", "d e", "f g"])
        #expect(SetupReader.shellWords("  a#b\t'it'\"'\"'s'  ") == ["a#b", "it's"])
    }
}

@Suite struct SetupReaderTests {
    let root: URL

    init() throws {
        root = FileManager.default.temporaryDirectory.appendingPathComponent("pulsekit-setup-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }

    func writeHubUnit(home: String, port: Int = 8765) throws -> URL {
        let agents = root.appendingPathComponent("LaunchAgents", isDirectory: true)
        try FileManager.default.createDirectory(at: agents, withIntermediateDirectories: true)
        let launcher = root.appendingPathComponent("pulse-hub.sh")
        try "\(home)/bin/pulse hub --port \(port) --no-browser --restart-on-change &\nwait\n"
            .write(to: launcher, atomically: true, encoding: .utf8)
        let plist: [String: Any] = [
            "Label": "pulse.hub",
            "ProgramArguments": ["/bin/sh", launcher.path],
            "EnvironmentVariables": ["PATH": "/custom/bin:/usr/bin", "PULSE_HOME": home, "LANG": "en_GB.UTF-8"],
        ]
        let data = try PropertyListSerialization.data(fromPropertyList: plist, format: .xml, options: 0)
        try data.write(to: agents.appendingPathComponent("pulse.hub.plist"))
        return agents
    }

    @Test func theServiceUnitDecidesHomeBinaryAndPort() throws {
        let agents = try writeHubUnit(home: "/srv/pulse-home", port: 8790)
        let setup = SetupReader.read(
            overrides: .init(), environment: ["PULSE_HOME": "/elsewhere"], userHome: root, launchAgents: agents
        )
        #expect(setup.home.path == "/srv/pulse-home")
        #expect(setup.binary.path == "/srv/pulse-home/bin/pulse")
        #expect(setup.port == 8790)
        #expect(setup.hubService?.environment["PATH"] == "/custom/bin:/usr/bin")
        #expect(setup.consoleURL.absoluteString == "http://127.0.0.1:8790/")
        #expect(setup.store.path == "/srv/pulse-home/pulse.sqlite")
    }

    @Test func overridesBeatTheUnit() throws {
        let agents = try writeHubUnit(home: "/srv/pulse-home")
        let setup = SetupReader.read(
            overrides: .init(pulseHome: "~/mine", pulseBinary: "/opt/pulse", hubURL: "http://127.0.0.1:8787", hubPort: 9000),
            environment: [:], userHome: root, launchAgents: agents
        )
        #expect(setup.home.path == root.appendingPathComponent("mine").path)
        #expect(setup.binary.path == "/opt/pulse")
        #expect(setup.port == 9000)
        #expect(setup.hubOverride?.absoluteString == "http://127.0.0.1:8787/")
        #expect(setup.consoleURL.absoluteString == "http://127.0.0.1:8787/")
    }

    // HubPort is the port for a console the app starts itself; a service's
    // console is where launchd serves it, whatever the setting says.
    @Test func theServicesPortBeatsHubPort() throws {
        let agents = try writeHubUnit(home: "/srv/pulse-home", port: 8790)
        let withService = SetupReader.read(
            overrides: .init(hubPort: 9000), environment: [:], userHome: root, launchAgents: agents
        )
        #expect(withService.hubService != nil)
        #expect(withService.port == 8790)
        #expect(withService.consoleURL.absoluteString == "http://127.0.0.1:8790/")

        let withoutService = SetupReader.read(
            overrides: .init(hubPort: 9000), environment: [:], userHome: root,
            launchAgents: root.appendingPathComponent("NoAgents", isDirectory: true)
        )
        #expect(withoutService.hubService == nil)
        #expect(withoutService.port == 9000)
    }

    @Test func aServiceForAnotherHomeIsNotThisHomesService() throws {
        let agents = try writeHubUnit(home: "/srv/pulse-home")
        let other = SetupReader.read(
            overrides: .init(pulseHome: "/tmp/other-home"), environment: [:], userHome: root, launchAgents: agents
        )
        #expect(other.home.path == "/tmp/other-home")
        #expect(other.hubService == nil)
        #expect(other.binary.path == "/tmp/other-home/bin/pulse")

        let same = SetupReader.read(
            overrides: .init(pulseHome: "/srv/pulse-home/"), environment: [:], userHome: root, launchAgents: agents
        )
        #expect(same.hubService != nil)
    }

    @Test func withoutAUnitTheEnvironmentThenTheDefaultHome() {
        let agents = root.appendingPathComponent("none", isDirectory: true)
        let fromEnv = SetupReader.read(overrides: .init(), environment: ["PULSE_HOME": "/env/home"], userHome: root, launchAgents: agents)
        #expect(fromEnv.home.path == "/env/home")
        #expect(fromEnv.hubService == nil)
        #expect(fromEnv.port == PulseSetup.servicePort)

        let fallback = SetupReader.read(overrides: .init(), environment: [:], userHome: root, launchAgents: agents)
        #expect(fallback.home.path == root.appendingPathComponent(".pulse").path)
        #expect(fallback.binary.path == root.appendingPathComponent(".pulse/bin/pulse").path)
    }

    @Test func aHubURLThatIsNotHTTPIsIgnored() {
        let agents = root.appendingPathComponent("none", isDirectory: true)
        let setup = SetupReader.read(overrides: .init(hubURL: "file:///tmp/x"), environment: [:], userHome: root, launchAgents: agents)
        #expect(setup.hubOverride == nil)
    }

    @Test func theChildGetsTheUnitsPathAndThisHome() throws {
        let agents = try writeHubUnit(home: "/srv/pulse-home")
        let setup = SetupReader.read(overrides: .init(), environment: [:], userHome: root, launchAgents: agents)
        let env = setup.childEnvironment(
            base: ["PATH": "/usr/bin:/bin", "HOME": "/Users/me", "__CFBundleIdentifier": "io.github.x"], loginPath: "/login/bin"
        )
        #expect(env["PATH"] == "/custom/bin:/usr/bin")
        #expect(env["PULSE_HOME"] == "/srv/pulse-home")
        #expect(env["LANG"] == "en_GB.UTF-8")
        #expect(env["HOME"] == "/Users/me")
        #expect(env["__CFBundleIdentifier"] == nil)
    }

    @Test func withoutAUnitTheLoginShellsPathThenAFallback() {
        let setup = PulseSetup(home: URL(fileURLWithPath: "/h"), binary: URL(fileURLWithPath: "/h/bin/pulse"))
        #expect(setup.childEnvironment(base: [:], loginPath: "/a:/b")["PATH"] == "/a:/b")
        let fallback = setup.childEnvironment(base: ["HOME": "/Users/me"], loginPath: nil)["PATH"] ?? ""
        #expect(fallback.hasPrefix("/Users/me/.pulse/bin:/Users/me/.cargo/bin"))
        #expect(fallback.contains("/opt/homebrew/bin"))
        #expect(fallback.hasSuffix("/sbin"))
    }
}

@Suite struct HubOutputTests {
    @Test func theLinesThatMatter() {
        #expect(HubLine.parse("Serving on http://127.0.0.1:8765 (127.0.0.1 only; writes: decide, settings; Ctrl-C to stop)")
            == .serving(URL(string: "http://127.0.0.1:8765/")!))
        #expect(HubLine.parse("handing over to the pulse.hub service") == .handingOver)
        #expect(HubLine.parse("the pulse.hub service now serves http://127.0.0.1:8765; this hub exits")
            == .handedOver(URL(string: "http://127.0.0.1:8765/")!))
        #expect(HubLine.parse("The hub could not listen") == .portTaken)
        #expect(HubLine.parse("No store at /Users/me/.pulse/pulse.sqlite") == .noStore)
        #expect(HubLine.parse("seeded 12 framework record(s) from bundle") == .other)
        #expect(HubLine.parse("Serving on nothing") == .other)
    }

    @Test func exitsAreClassifiedWithWhatWasPrinted() {
        func exited(_ code: Int32) -> Int32 { code << 8 }
        #expect(HubExit.classify(waitStatus: exited(0), sawHandover: true, sawPortTaken: false, sawNoStore: false) == .handedOver)
        #expect(HubExit.classify(waitStatus: exited(0), sawHandover: false, sawPortTaken: false, sawNoStore: false) == .stopped)
        #expect(HubExit.classify(waitStatus: exited(3), sawHandover: false, sawPortTaken: false, sawNoStore: false) == .binaryChanged)
        #expect(HubExit.classify(waitStatus: exited(1), sawHandover: false, sawPortTaken: true, sawNoStore: false) == .portTaken)
        #expect(HubExit.classify(waitStatus: exited(1), sawHandover: false, sawPortTaken: false, sawNoStore: false) == .failed(code: 1))
        #expect(HubExit.classify(waitStatus: exited(2), sawHandover: false, sawPortTaken: false, sawNoStore: true) == .noStore)
        #expect(HubExit.classify(waitStatus: exited(2), sawHandover: false, sawPortTaken: false, sawNoStore: false) == .failed(code: 2))
        #expect(HubExit.classify(waitStatus: 9, sawHandover: false, sawPortTaken: false, sawNoStore: false) == .killed(signal: 9))
        #expect(HubExit.classify(waitStatus: 0x80 | 11, sawHandover: false, sawPortTaken: false, sawNoStore: false) == .killed(signal: 11))
    }
}

@Suite struct LinkPolicyTests {
    let policy = LinkPolicy(console: URL(string: "http://127.0.0.1:8765/")!)

    @Test func consolePagesStay() {
        #expect(policy.target(for: URL(string: "http://127.0.0.1:8765/tasks/?project=x")!, newWindow: false) == .here)
        #expect(policy.target(for: URL(string: "http://127.0.0.1:8765/chat/?id=1")!, newWindow: true)
            == .newTab(URL(string: "http://127.0.0.1:8765/chat/?id=1")!))
    }

    @Test func localhostBecomesTheConsolesOwnHost() {
        #expect(policy.target(for: URL(string: "http://localhost:8765/records/?id=ADR-1#x")!, newWindow: false)
            == .redirect(URL(string: "http://127.0.0.1:8765/records/?id=ADR-1#x")!))
    }

    @Test func everythingElseLeaves() {
        let github = URL(string: "https://github.com/acme/app/pull/12")!
        #expect(policy.target(for: github, newWindow: true) == .browser(github))
        #expect(policy.target(for: github, newWindow: false) == .browser(github))
        // The MCP OAuth callback listener is another port on the same host.
        let callback = URL(string: "http://127.0.0.1:53123/callback?code=x")!
        #expect(policy.target(for: callback, newWindow: false) == .browser(callback))
        #expect(policy.target(for: URL(string: "https://127.0.0.1:8765/")!, newWindow: false)
            == .browser(URL(string: "https://127.0.0.1:8765/")!))
        let mail = URL(string: "mailto:owner@example.com")!
        #expect(policy.target(for: mail, newWindow: false) == .otherApp(mail, ask: false))
        let slack = URL(string: "slack://open")!
        #expect(policy.target(for: slack, newWindow: false) == .otherApp(slack, ask: true))
    }

    @Test func aClickedWebPageOpensBesideTheConsole() {
        let docs = URL(string: "https://docs.rs/reqwest/latest/")!
        // A plain click, whether the link asked for a new window or not.
        #expect(policy.target(for: docs, newWindow: false, click: true) == .beside(docs))
        #expect(policy.target(for: docs, newWindow: true, click: true) == .beside(docs))
        // The page an agent drove on a dev server: another port is not the console.
        let preview = URL(string: "http://127.0.0.1:3000/")!
        #expect(policy.target(for: preview, newWindow: true, click: true) == .beside(preview))
        // A console page clicked is still a console page.
        #expect(policy.target(for: URL(string: "http://localhost:8765/chat/")!, newWindow: false, click: true)
            == .redirect(URL(string: "http://127.0.0.1:8765/chat/")!))
        #expect(policy.target(for: URL(string: "http://127.0.0.1:8765/chat/")!, newWindow: true, click: true)
            == .newTab(URL(string: "http://127.0.0.1:8765/chat/")!))
        // Not a click (window.open, a script's redirect, ⌘-click): the default browser.
        #expect(policy.target(for: docs, newWindow: true, click: false) == .browser(docs))
        // Other schemes go where they always went.
        let mail = URL(string: "mailto:owner@example.com")!
        #expect(policy.target(for: mail, newWindow: false, click: true) == .otherApp(mail, ask: false))
        #expect(policy.target(for: URL(string: "javascript:alert(1)")!, newWindow: false, click: true) == .refuse)
    }

    @Test func filesScriptsAndBlobsNeverReplaceTheConsole() {
        for link in ["file:///Users/me/Desktop/report.pdf", "javascript:alert(1)", "data:text/html,hi", "blob:http://127.0.0.1:8765/1"] {
            #expect(policy.target(for: URL(string: link)!, newWindow: false) == .refuse, "\(link)")
        }
        #expect(policy.target(for: URL(string: "about:blank")!, newWindow: false) == .here)
        #expect(policy.target(for: URL(string: "about:blank")!, newWindow: true) == .refuse)
    }

    @Test func pagesMoveToANewConsoleOrigin() {
        let moved = LinkPolicy(console: URL(string: "http://127.0.0.1:50123/")!)
        #expect(moved.rebase(URL(string: "http://127.0.0.1:8765/tasks/?project=a#top")!).absoluteString
            == "http://127.0.0.1:50123/tasks/?project=a#top")
        #expect(moved.isConsole(URL(string: "http://localhost:50123/x")!))
        #expect(!moved.isConsole(URL(string: "http://127.0.0.1:8765/x")!))
    }
}

@Suite struct TypedAddressTests {
    func address(_ text: String) -> String? { TypedAddress.url(text)?.absoluteString }

    @Test func webAddressesStayAsTyped() {
        #expect(address("https://github.com/acme/app/pull/12") == "https://github.com/acme/app/pull/12")
        #expect(address("  http://127.0.0.1:3000/?x=1#top ") == "http://127.0.0.1:3000/?x=1#top")
        #expect(address("HTTP://Example.com") == "HTTP://Example.com")
    }

    @Test func hostsGetAScheme() {
        #expect(address("docs.rs/reqwest/latest") == "https://docs.rs/reqwest/latest")
        #expect(address("example.com") == "https://example.com")
        #expect(address("example.com:443/x") == "https://example.com:443/x")
        // A machine's own, and anything naming a port: a dev server speaks http.
        #expect(address("localhost:3000") == "http://localhost:3000")
        #expect(address("localhost") == "http://localhost")
        #expect(address("127.0.0.1:4123/posts/x") == "http://127.0.0.1:4123/posts/x")
        #expect(address("0.0.0.0:8000") == "http://0.0.0.0:8000")
        #expect(address("[::1]:3000/") == "http://[::1]:3000/")
        #expect(address("my-app:8080") == "http://my-app:8080")
        #expect(address("preview.local/") == "http://preview.local/")
        #expect(address("192.168.1.10:3000") == "http://192.168.1.10:3000")
    }

    @Test func wordsAndOtherSchemesAreNotAnAddress() {
        for text in ["", "   ", "hello", "retry after", "mailto:owner@example.com", "javascript:alert(1)",
                     "file:///Users/me/report.pdf", "slack://open", ".com", "example.", "host:port", "host:"] {
            #expect(TypedAddress.url(text) == nil, "\(text)")
        }
    }
}

@Suite struct ProbeTests {
    func body(_ json: String) -> Data { Data(json.utf8) }

    @Test func pulseSaysOkAndListsTheCommons() {
        #expect(HubProbe.interpret(status: 200, body: body(#"{"ok":true,"time":"t","projects":["app","commons"],"errors":{}}"#)) == .pulse)
    }

    @Test func otherAnswersAreAnotherProgram() {
        if case .notPulse = HubProbe.interpret(status: 200, body: body("<html>hi</html>")) {} else { Issue.record("html") }
        if case .notPulse = HubProbe.interpret(status: 200, body: body(#"{"ok":true,"projects":["app"]}"#)) {} else { Issue.record("no commons") }
        if case .notPulse = HubProbe.interpret(status: 500, body: body(#"{"error":"x"}"#)) {} else { Issue.record("500") }
        if case .notPulse = HubProbe.interpret(status: 404, body: body(#"{"error":"Not found","why":"","fix":""}"#)) {} else { Issue.record("404") }
    }

    @Test func nothingListeningIsUnreachable() async throws {
        // A port nothing listens on: bind one, learn its number, close it.
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        var address = sockaddr_in()
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        address.sin_port = 0
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        _ = withUnsafePointer(to: &address) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, length) } }
        _ = withUnsafeMutablePointer(to: &address) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(fd, $0, &length) } }
        let port = Int(UInt16(bigEndian: address.sin_port))
        close(fd)
        let result = await HubProbe(timeout: 2).check(URL(string: "http://127.0.0.1:\(port)/")!)
        guard case .unreachable = result else {
            Issue.record("expected unreachable, got \(result)")
            return
        }
    }
}

@Suite struct LaunchdTests {
    @Test func readsTheJobsOwnLines() {
        let printed = """
        gui/502/pulse.hub = {
        \tactive count = 1
        \tpath = /Users/me/Library/LaunchAgents/pulse.hub.plist
        \tstate = running
        \tenvironment = {
        \t\tPATH => /usr/bin
        \t\tpid = 1
        \t}
        \tpid = 63413
        \tlast exit code = 3
        }
        """
        #expect(LaunchdJob.parse(printOutput: printed, status: 0) == LaunchdJob(loaded: true, running: true, pid: 63413, lastExitCode: 3))
    }

    @Test func aStoppedJobAndAnUnknownOne() {
        let stopped = "gui/502/pulse.hub = {\n\tstate = not running\n\tlast exit code = 1: Operation not permitted\n}"
        #expect(LaunchdJob.parse(printOutput: stopped, status: 0) == LaunchdJob(loaded: true, running: false, pid: nil, lastExitCode: 1))
        #expect(LaunchdJob.parse(printOutput: "", status: 113) == .notLoaded)
    }
}

@Suite struct ShellTests {
    @Test func theMarkerLineIsThePath() {
        let output = "Welcome!\n\u{1B}]0;title\u{07}\n\(LoginShell.marker)/a/bin:/usr/bin\n"
        #expect(LoginShell.parse(output) == "/a/bin:/usr/bin")
        #expect(LoginShell.parse("nothing here") == nil)
        #expect(LoginShell.parse("\(LoginShell.marker)\n") == nil)
    }

    @Test func commandsReportStatusOutputAndDeadlines() async {
        let ok = await Command.run("/bin/sh", ["-c", "echo out; echo err >&2; exit 3"])
        #expect(ok.status == 3)
        #expect(ok.stdout == "out\n")
        #expect(ok.output == "err")
        let slow = await Command.run("/bin/sleep", ["5"], timeout: 0.3)
        #expect(slow.timedOut)
        let missing = await Command.run("/nonexistent/program", [])
        #expect(missing.status == 127)
    }

    @Test func aLogsTail() throws {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("tail-\(UUID().uuidString).log")
        try (1...40).map { "line \($0)" }.joined(separator: "\n").write(to: file, atomically: true, encoding: .utf8)
        #expect(LogTail.read(file, lines: 3) == "line 38\nline 39\nline 40")
        #expect(LogTail.read(file.appendingPathExtension("missing")) == nil)
    }
}
