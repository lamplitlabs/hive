import AppKit
import HiveKit
import WebKit
import os

/// The app: one console supervisor shared by every window, the windows and
/// tabs, and the settings they share (page zoom, the console's theme, the
/// page to come back to).
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuItemValidation {
    static let zoomSteps: [CGFloat] = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3]

    let supervisor = HubSupervisor(world: LiveWorld())
    let downloads = Downloads()
    /// The console's notifications, shown as Mac notifications.
    let notifications = Notifications()
    private(set) var windows: [ConsoleWindowController] = []
    private let defaults = UserDefaults.standard
    /// `-EphemeralRun YES` (scripts/check.sh): remember nothing, and keep the
    /// console's storage in memory, so a test run leaves the owner's app as it was.
    let ephemeral = UserDefaults.standard.bool(forKey: "EphemeralRun")
    private var volatile: [String: Any] = [:]
    private let log = Logger(subsystem: AppIdentity.bundleID, category: "app")
    private var snapshotTaken = false
    /// `-Commands` not yet started: their clock starts with the first page.
    private var pendingCommands: [(at: Double, command: String)] = []

    // MARK: - Lifecycle

    func applicationWillFinishLaunching(_ notification: Notification) {
        // Also when run straight from `swift run`, outside a bundle.
        NSApp.setActivationPolicy(.regular)
        Fonts.register()
        NSApp.mainMenu = MainMenu.build()
        NSWindow.allowsAutomaticWindowTabbing = true
        // Before launch finishes, so a click on a notification that starts
        // Hive finds its page.
        notifications.open = { [weak self] path in self?.openFromNotification(path) }
        notifications.start()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let start = defaults.string(forKey: "OpenPath") ?? rememberedPage
        openWindow(path: start)
        supervisor.start()
        NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didWakeNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.supervisor.checkAfterWake() }
        }
        // A test run (scripts/check.sh) shows its window without taking focus.
        if !ephemeral {
            if #available(macOS 14, *) {
                NSApp.activate()
            } else {
                NSApp.activate(ignoringOtherApps: true)
            }
        }
        scheduleTestQuit()
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
        if !hasVisibleWindows {
            if let window = windows.first {
                window.showWindow(nil)
            } else {
                openWindow(path: rememberedPage)
            }
        }
        return true
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard supervisor.ownsRunningHub else { return .terminateNow }
        // The console the app started stops with it (as a terminal's
        // `hive hub` does with its window). Look gone at once, stop it, go.
        log.notice("quitting: stopping the console the app started")
        // Pages closed first: their live streams end now, so the console's
        // shutdown has no open request to wait out.
        windows.forEach { $0.unloadForQuit() }
        for window in NSApp.windows { window.orderOut(nil) }
        supervisor.beginShutdown()
        // AppKit waits for the reply inside terminate(_:), in its own run-loop
        // mode: a timer in the common modes keeps firing there, where a task
        // queued behind the caller (a quit from a task) would not. The hub
        // gets the time launchd would give it (HubSupervisor.stopGrace).
        let deadline = Date().addingTimeInterval(HubSupervisor.stopGrace)
        let timer = Timer(timeInterval: 0.1, repeats: true) { [weak self] timer in
            let done = MainActor.assumeIsolated { () -> Bool in
                guard let self else { return true }
                if !self.supervisor.ownedHubStopped, Date() < deadline { return false }
                if self.supervisor.ownedHubStopped {
                    self.log.notice("the app's console has stopped; quitting")
                } else {
                    self.supervisor.forceStopOwnedHub()
                }
                NSApp.reply(toApplicationShouldTerminate: true)
                return true
            }
            if done { timer.invalidate() }
        }
        RunLoop.main.add(timer, forMode: .common)
        return .terminateLater
    }

    func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool { true }

    func applicationDockMenu(_ sender: NSApplication) -> NSMenu? {
        let menu = NSMenu()
        menu.addItem(withTitle: "New Window", action: #selector(newConsoleWindow(_:)), keyEquivalent: "")
        return menu
    }

    // MARK: - Windows

    /// A click on one of the console's notifications: its page in the window
    /// in front (or the first one, or a new one), Hive in front.
    func openFromNotification(_ path: String) {
        let target = windows.first(where: { $0.window?.isKeyWindow == true })
            ?? windows.first(where: { $0.window?.isVisible == true })
            ?? windows.first
        if let target {
            target.showWindow(nil)
            target.navigate(to: path)
        } else {
            openWindow(path: path)
        }
        if #available(macOS 14, *) {
            NSApp.activate()
        } else {
            NSApp.activate(ignoringOtherApps: true)
        }
    }

    @discardableResult
    func openWindow(path: String? = nil, url: URL? = nil, tabbedWith host: ConsoleWindowController? = nil, background: Bool = false) -> ConsoleWindowController {
        let wanted = url ?? path.flatMap { pageURL($0) }
        let controller = ConsoleWindowController(app: self, wanted: wanted)
        controller.opener = host
        windows.append(controller)
        guard let window = controller.window else { return controller }
        if let hostWindow = host?.window {
            hostWindow.addTabbedWindow(window, ordered: .above)
            if background {
                window.orderBack(nil)
                hostWindow.makeKeyAndOrderFront(nil)
            } else {
                window.makeKeyAndOrderFront(nil)
            }
        } else {
            if windows.count == 1 {
                window.center()
                // A test run keeps the default size, whatever the owner's window is.
                if !ephemeral { window.setFrameAutosaveName("Hive.console") }
            } else if let previous = (windows.dropLast().last(where: { $0.window?.isKeyWindow == true }) ?? windows.dropLast().last)?.window {
                // Another console window's frame, not NSApp.keyWindow's: a
                // file panel or alert can be key.
                window.setFrame(previous.frame, display: false)
                window.setFrameTopLeftPoint(window.cascadeTopLeft(from: NSPoint(x: previous.frame.minX, y: previous.frame.maxY)))
            }
            controller.showWindow(nil)
        }
        return controller
    }

    func openTab(_ url: URL, from host: ConsoleWindowController, background: Bool) {
        openWindow(url: url, tabbedWith: host, background: background)
    }

    func windowClosed(_ controller: ConsoleWindowController) {
        windows.removeAll { $0 === controller }
    }

    /// A path on the console's origin, before the console is known.
    private func pageURL(_ path: String) -> URL? {
        let setup = supervisor.setup
        let base = setup.hubOverride ?? HiveSetup.consoleURL(port: setup.port)
        guard path.hasPrefix("/") else { return nil }
        return URL(string: path, relativeTo: base)?.absoluteURL
    }

    func openInOtherApp(_ url: URL, ask: Bool, window: NSWindow?) {
        guard ask else {
            NSWorkspace.shared.open(url)
            return
        }
        let alert = NSAlert()
        alert.messageText = "Open this link in another app?"
        alert.informativeText = url.absoluteString
        alert.addButton(withTitle: "Open")
        alert.addButton(withTitle: "Cancel")
        let open: @MainActor (NSApplication.ModalResponse) -> Void = { response in
            if response == .alertFirstButtonReturn { NSWorkspace.shared.open(url) }
        }
        if let window {
            alert.beginSheetModal(for: window) { open($0) }
        } else {
            open(alert.runModal())
        }
    }

    // MARK: - Shared settings

    /// Every window's console storage: the default one, or memory for a test run.
    lazy var dataStore: WKWebsiteDataStore = ephemeral ? .nonPersistent() : .default()

    private func stored(_ key: String) -> Any? {
        if ephemeral {
            return volatile[key] ?? defaults.volatileDomain(forName: UserDefaults.argumentDomain)[key]
        }
        return defaults.object(forKey: key)
    }

    private func store(_ value: Any, forKey key: String) {
        if ephemeral { volatile[key] = value } else { defaults.set(value, forKey: key) }
    }

    var pageZoom: CGFloat {
        let value = stored("PageZoom")
        let zoom = (value as? NSNumber)?.doubleValue ?? (value as? String).flatMap(Double.init) ?? 1
        return zoom > 0 ? CGFloat(zoom) : 1
    }

    func setPageZoom(_ zoom: CGFloat) {
        let clamped = min(max(zoom, Self.zoomSteps.first!), Self.zoomSteps.last!)
        store(Double(clamped), forKey: "PageZoom")
        windows.forEach { $0.applyZoom(clamped) }
    }

    func stepZoom(by direction: Int) {
        let current = pageZoom
        let next = direction > 0
            ? Self.zoomSteps.first(where: { $0 > current + 0.001 })
            : Self.zoomSteps.last(where: { $0 < current - 0.001 })
        if let next { setPageZoom(next) }
    }

    /// The owner's theme choice in the console (light, dark, system), so a new
    /// window opens in it before the page says so.
    var rememberedTheme: String { stored("ConsoleTheme") as? String ?? "system" }

    func rememberTheme(_ choice: String) {
        if choice != rememberedTheme { store(choice, forKey: "ConsoleTheme") }
    }

    func appearance(for choice: String) -> NSAppearance? {
        switch choice {
        case "light": return NSAppearance(named: .aqua)
        case "dark": return NSAppearance(named: .darkAqua)
        default: return nil
        }
    }

    /// Path, query and fragment of the last page, reopened at the next launch.
    var rememberedPage: String? { stored("LastPage") as? String }

    func rememberPage(_ url: URL) {
        guard var parts = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return }
        parts.scheme = nil
        parts.host = nil
        parts.port = nil
        if let page = parts.string, page.hasPrefix("/"), page != rememberedPage {
            store(page, forKey: "LastPage")
        }
    }

    // MARK: - What the status screen offers

    func logTail(_ url: URL?) -> String? {
        url.flatMap { LogTail.read($0) }
    }

    func openCurrentLog() {
        guard case .problem(let problem) = supervisor.state, let log = problem.log else { return }
        if FileManager.default.fileExists(atPath: log.path) {
            let console = URL(fileURLWithPath: "/System/Applications/Utilities/Console.app")
            NSWorkspace.shared.open([log], withApplicationAt: console, configuration: NSWorkspace.OpenConfiguration())
        } else {
            NSWorkspace.shared.activateFileViewerSelecting([log.deletingLastPathComponent()])
        }
    }

    func restartConsoleService() {
        Task { @MainActor in
            await Launchd.kickstart(supervisor.setup.hubService?.label ?? HiveSetup.hubLabel, restart: true)
            supervisor.retry()
        }
    }

    /// "Choose hive…": a hive binary somewhere else (kept as HiveBinary).
    func chooseBinary(for window: NSWindow?) {
        let panel = NSOpenPanel()
        panel.message = "Choose the hive program the app should run."
        panel.prompt = "Use This hive"
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.treatsFilePackagesAsDirectories = true
        panel.directoryURL = supervisor.setup.binary.deletingLastPathComponent()
        let chosen: @MainActor (NSApplication.ModalResponse) -> Void = { [weak self] response in
            guard let self, response == .OK, let url = panel.url else { return }
            guard FileManager.default.isExecutableFile(atPath: url.path) else {
                self.tell("That file is not a program", "Choose the hive binary, usually ~/.hive/bin/hive.", window: window)
                return
            }
            self.store(url.path, forKey: "HiveBinary")
            self.supervisor.retry()
        }
        if let window { panel.beginSheetModal(for: window) { chosen($0) } } else { chosen(panel.runModal()) }
    }

    /// "Choose a Folder…": `hive init <folder>` creates the store and the
    /// first project; the console starts after it.
    func chooseFirstProject(for window: NSWindow?) {
        let panel = NSOpenPanel()
        panel.message = "Choose a folder with a git checkout. It becomes your first Hive project."
        panel.prompt = "Add Project"
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = false
        panel.allowsMultipleSelection = false
        panel.directoryURL = FileManager.default.homeDirectoryForCurrentUser
        let chosen: @MainActor (NSApplication.ModalResponse) -> Void = { [weak self] response in
            guard let self, response == .OK, let folder = panel.url else { return }
            Task { @MainActor in
                let result = await self.supervisor.createFirstProject(at: folder)
                if !result.succeeded {
                    self.tell(
                        "Hive could not add \(folder.lastPathComponent)",
                        result.output.isEmpty ? "hive init exited with code \(result.status)." : result.output,
                        window: window
                    )
                }
            }
        }
        if let window { panel.beginSheetModal(for: window) { chosen($0) } } else { chosen(panel.runModal()) }
    }

    private func tell(_ title: String, _ detail: String, window: NSWindow?) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = detail
        if let window { alert.beginSheetModal(for: window) } else { alert.runModal() }
    }

    // MARK: - App-wide commands (also reached with no window open)

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        switch item.action {
        case #selector(showConsoleLog(_:)):
            return true
        default:
            return true
        }
    }

    @objc func newConsoleWindow(_ sender: Any?) {
        openWindow(path: "/")
    }

    /// Go-menu routes with no window open: a window on that page.
    @objc func goToRoute(_ sender: NSMenuItem) {
        guard let path = sender.representedObject as? String else { return }
        openWindow(path: path)
    }

    @objc func showConsoleLog(_ sender: Any?) {
        let setup = supervisor.setup
        let candidates = supervisor.ownsRunningHub ? [setup.appHubLog, setup.serviceLog] : [setup.serviceLog, setup.appHubLog]
        guard let log = candidates.first(where: { FileManager.default.fileExists(atPath: $0.path) }) else {
            tell("No console log yet", "Hive writes it to \(setup.logs.path) once its console has run.", window: NSApp.keyWindow)
            return
        }
        let console = URL(fileURLWithPath: "/System/Applications/Utilities/Console.app")
        NSWorkspace.shared.open([log], withApplicationAt: console, configuration: NSWorkspace.OpenConfiguration())
    }

    /// Help → Send Test Notification: a banner with the Mac's sound now, so
    /// the owner sees native delivery work, or a sheet saying why it cannot.
    @objc func sendTestNotification(_ sender: Any?) {
        let window = NSApp.keyWindow
        Task { @MainActor in
            if let (title, detail) = await notifications.sendTest() {
                tell(title, detail, window: window)
            }
        }
    }

    @objc func showHiveHelp(_ sender: Any?) {
        if let url = URL(string: "https://github.com/manishtiwari25/pulse/blob/main/docs/mac-app.md") {
            NSWorkspace.shared.open(url)
        }
    }

    @objc func showAbout(_ sender: Any?) {
        var lines = ["The Hive console in its own window."]
        if let url = supervisor.state.consoleURL {
            lines.append("Console: \(url.absoluteString)")
        }
        let credits = NSAttributedString(
            string: lines.joined(separator: "\n"),
            attributes: [.font: Fonts.text(11), .foregroundColor: NSColor.secondaryLabelColor]
        )
        NSApp.orderFrontStandardAboutPanel(options: [
            .applicationName: "Hive",
            .applicationVersion: AppIdentity.version,
            .version: AppIdentity.build,
            .credits: credits,
        ])
    }

    // MARK: - Test hooks

    /// `-SnapshotPath <png>`: once the first page has loaded (and
    /// `-SnapshotDelay` seconds more), write what the window shows; with
    /// `-QuitAfterSnapshot YES`, quit. scripts/check.sh uses these to look at
    /// the app without a screen recording permission.
    func pageFinished(in window: ConsoleWindowController) {
        startTestCommands()
        guard !snapshotTaken, let path = defaults.string(forKey: "SnapshotPath"), window === windows.first else { return }
        snapshotTaken = true
        let delay = defaults.double(forKey: "SnapshotDelay")
        Task { @MainActor [weak self, weak window] in
            try? await Task.sleep(nanoseconds: UInt64(max(delay, 1.5) * 1_000_000_000))
            guard let self, let window else { return }
            self.writeSnapshot(of: window, to: path)
        }
    }

    /// `-QuitAfter <s>` quits after that long, first writing `-FinalSnapshotPath
    /// <png>` when given. `-Commands "2:go /tasks/;4:appearance light;6:snapshot
    /// /tmp/a.png"` runs menu commands in the first window at those seconds
    /// after the first page has loaded, so a slow first launch does not shift
    /// them onto a page that is not there yet (scripts/check.sh drives the app
    /// this way).
    private func scheduleTestQuit() {
        for entry in (defaults.string(forKey: "Commands") ?? "").split(separator: ";") {
            let parts = entry.split(separator: ":", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            guard parts.count == 2, let at = Double(parts[0]) else { continue }
            pendingCommands.append((at: at, command: parts[1]))
        }
        let quitAfter = defaults.double(forKey: "QuitAfter")
        if quitAfter > 0 {
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: UInt64(quitAfter * 1_000_000_000))
                if let path = self.defaults.string(forKey: "FinalSnapshotPath"), let window = self.windows.first {
                    self.writeSnapshot(of: window, to: path, quitAfter: true)
                } else {
                    NSApp.terminate(nil)
                }
            }
        }
    }

    /// The first page has loaded: the commands' clock starts now.
    private func startTestCommands() {
        guard !pendingCommands.isEmpty else { return }
        let commands = pendingCommands
        pendingCommands.removeAll()
        log.notice("test hooks: the first page has loaded; \(commands.count, privacy: .public) commands scheduled")
        for (at, command) in commands {
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: UInt64(at * 1_000_000_000))
                self.runTestCommand(command)
            }
        }
    }

    private func runTestCommand(_ command: String) {
        let words = command.split(separator: " ", maxSplits: 1).map(String.init)
        guard let verb = words.first, let window = windows.first(where: { $0.window?.isKeyWindow == true }) ?? windows.first else {
            return
        }
        let argument = words.count > 1 ? words[1] : ""
        log.notice("test hook: \(command, privacy: .public)")
        let item = NSMenuItem()
        item.representedObject = argument
        switch verb {
        case "go": window.navigate(to: argument)
        case "foryou": window.showForYou(nil)
        case "back": window.consoleGoBack(nil)
        case "reload": window.reloadConsole(nil)
        case "appearance": window.setConsoleAppearance(item)
        case "find": window.findForTesting(argument)
        case "zoomin": window.zoomPageIn(nil)
        case "zoomout": window.zoomPageOut(nil)
        case "actualsize": window.actualPageSize(nil)
        case "search": window.openSearch(nil)
        case "newtab":
            if let base = window.webView.url, let target = URL(string: argument, relativeTo: base)?.absoluteURL {
                openTab(target, from: window, background: false)
            }
        case "beside":
            // A web page in the browser panel, or the panel away without one.
            if let target = URL(string: argument), target.host != nil { window.showBeside(target) } else { window.toggleBeside(nil) }
        case "snapshot": writeSnapshot(of: window, to: argument, quitAfter: false)
        case "click", "rightclick":
            // A real mouse click through AppKit's hit testing, at points from
            // the top left of the window's content; a right-click opens the
            // page's menu, which the run closes itself once it has been logged.
            let right = verb == "rightclick"
            let numbers = argument.split(separator: " ").compactMap { Double($0) }
            guard numbers.count == 2, let win = window.window, let content = win.contentView else { break }
            let point = NSPoint(x: numbers[0], y: content.bounds.height - numbers[1])
            let hit = content.hitTest(point)
            log.notice("test hook: \(right ? "right-click" : "click", privacy: .public) lands on \(hit.map { String(describing: type(of: $0)) } ?? "nothing", privacy: .public)")
            guard let hit else { break }
            let location = content.convert(point, to: nil)
            // Shown to whoever is watching; activation is only a request on
            // today's macOS, so the events go to the view that was hit, as
            // the window would hand them on when it is key (a click on a
            // window that is not key would only make it key).
            NSApp.activate(ignoringOtherApps: true)
            win.makeKeyAndOrderFront(nil)
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: 300_000_000)
                let types: [NSEvent.EventType] = right ? [.rightMouseDown, .rightMouseUp] : [.leftMouseDown, .leftMouseUp]
                for type in types {
                    if let event = NSEvent.mouseEvent(
                        with: type, location: location, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                        windowNumber: win.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1
                    ) {
                        switch type {
                        case .leftMouseDown: hit.mouseDown(with: event)
                        case .leftMouseUp: hit.mouseUp(with: event)
                        case .rightMouseDown: hit.rightMouseDown(with: event)
                        default: hit.rightMouseUp(with: event)
                        }
                    }
                }
                if right {
                    try? await Task.sleep(nanoseconds: 1_500_000_000)
                    window.dismissContextMenu()
                }
            }
        case "addlink":
            // A link to click (`addlink <url> [blank]`), pinned to the top
            // left of the page: the mock console has no web link of its own.
            let parts = argument.split(separator: " ").map(String.init)
            guard let href = parts.first else { return }
            let target = parts.count > 1 ? "_blank" : ""
            let add = "(() => { const a = document.createElement('a'); a.href = \(PageBridge.jsString(href)); if (\(PageBridge.jsString(target))) a.target = '_blank'; a.textContent = 'test link'; a.style.cssText = 'position:fixed;left:8px;top:8px;z-index:99999;padding:8px;background:#fff;color:#00f;font:14px sans-serif'; document.body.appendChild(a); return true; })()"
            window.webView.evaluateJavaScript(add, in: nil, in: PageBridge.world, completionHandler: nil)
        case "clicklink", "rightclicklink":
            // A real click on the sidebar's own link (or, failing that, any
            // link in the page): where it is comes from the page, in CSS
            // pixels from the web view's top left.
            let find = "(() => { const href = JSON.stringify(\(PageBridge.jsString(argument))); const a = document.querySelector('aside[aria-label=\"Sidebar\"] a[href=' + href + ']') || document.querySelector('a[href=' + href + ']'); if (!a) return null; const r = a.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; })()"
            window.webView.evaluateJavaScript(find, in: nil, in: PageBridge.world) { [weak self, weak window] result in
                guard let self, let window, let content = window.window?.contentView else { return }
                guard case .success(let value) = result, let center = value as? [Double], center.count == 2 else {
                    self.log.error("test hook: no link \(argument, privacy: .public)")
                    return
                }
                let zoom = Double(window.webView.pageZoom)
                let frame = window.webView.frame
                let x = Double(frame.minX) + center[0] * zoom
                let y = Double(content.bounds.height - frame.maxY) + center[1] * zoom
                self.runTestCommand("\(verb == "rightclicklink" ? "rightclick" : "click") \(x) \(y)")
            }
        case "quit": NSApp.terminate(nil)
        default: log.error("test hook: unknown command \(verb, privacy: .public)")
        }
    }

    private func writeSnapshot(of window: ConsoleWindowController, to path: String, quitAfter: Bool? = nil) {
        let quit = quitAfter ?? defaults.bool(forKey: "QuitAfterSnapshot")
        if quit {
            // A window that is never drawn never finishes a snapshot; a test run still ends.
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: 10_000_000_000)
                self.log.error("snapshot did not finish in 10 s; quitting")
                NSApp.terminate(nil)
            }
        }
        window.snapshot { [weak self] image in
            guard let self else { return }
            if let image, let tiff = image.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff),
               let png = rep.representation(using: .png, properties: [:]) {
                do {
                    try png.write(to: URL(fileURLWithPath: path))
                    self.log.notice("snapshot written to \(path, privacy: .public)")
                } catch {
                    self.log.error("snapshot failed: \(error.localizedDescription, privacy: .public)")
                }
            }
            if quit {
                NSApp.terminate(nil)
            }
        }
    }
}
