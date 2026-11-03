import AppKit
import PulseKit
import WebKit
import os

/// One window (or tab) of the console: the page the hub serves, unchanged,
/// with a native screen over it only while the console cannot be shown.
@MainActor
final class ConsoleWindowController: NSWindowController, NSWindowDelegate, NSMenuItemValidation,
    WKNavigationDelegate, WKUIDelegate, WKScriptMessageHandler, WKScriptMessageHandlerWithReply {
    let webView: ConsoleWebView
    /// The web page beside the console (BrowserPane), hidden until a link asks for it.
    let beside: BrowserPane
    private let split = NSSplitView()
    /// The panel's width, kept while it is hidden so it comes back as it was.
    private var besideWidth: CGFloat = 0
    /// The panel's least width, in force only while it shows (a collapsed
    /// panel is as wide as nothing).
    private lazy var besideMinWidth = beside.widthAnchor.constraint(greaterThanOrEqualToConstant: 320)
    private unowned let app: AppDelegate
    private let strip = TitleStrip()
    private let status = StatusView()
    private let findBar = FindBar()
    private let toast = Toast()
    private var observations: [NSKeyValueObservation] = []
    private var stateToken: UUID?
    private var policy: LinkPolicy?
    /// The link under the pointer at the last right-click, as the page reported it.
    private var contextLink: (url: URL?, at: Date)?
    /// The right-click menu showing, for a test run to close.
    private weak var contextMenu: NSMenu?
    /// The console page this window wants to show; kept while the console is away.
    private(set) var wanted: URL?
    /// A console page is committed in the web view.
    private(set) var hasPage = false
    /// The last main-frame load could not reach the console.
    private var loadFailed = false
    /// The native screen covers the page (set at once; the fade only follows).
    private var covering = false
    private var crashes: [Date] = []
    /// New windows the page asked for recently (window.open, target=_blank).
    private var popups: [Date] = []
    /// Recent main-frame loads that could not connect.
    private var loadFailures: [Date] = []
    private(set) var chrome: PageChrome?
    /// The window a tab was opened from (⌘-click, target=_blank).
    weak var opener: ConsoleWindowController?
    /// WKNavigationAction.buttonNumber for the middle button (WebKit's
    /// WebEventFactory::toNSButtonNumber: left 1, right 2, middle 4).
    static let middleButton = 4
    private let log = Logger(subsystem: AppIdentity.bundleID, category: "window")

    init(app: AppDelegate, wanted: URL?) {
        self.app = app
        self.wanted = wanted

        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = app.dataStore
        configuration.applicationNameForUserAgent = "PULSE/\(AppIdentity.version)"
        // MCP sign-in calls window.open from an effect, not a click.
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = true
        // A loopback console: nothing to look up in Safe Browsing.
        configuration.preferences.isFraudulentWebsiteWarningEnabled = false
        configuration.userContentController.addUserScript(PageBridge.userScript)
        webView = ConsoleWebView(frame: .zero, configuration: configuration)

        // The panel is a browser: the console's storage (a sign-in made there
        // stays made), no bridge, WebKit's own warnings.
        let besideConfiguration = WKWebViewConfiguration()
        besideConfiguration.websiteDataStore = app.dataStore
        besideConfiguration.applicationNameForUserAgent = "PULSE/\(AppIdentity.version)"
        beside = BrowserPane(configuration: besideConfiguration)

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1320, height: 860),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        super.init(window: window)

        configuration.userContentController.add(
            WeakScriptHandler(self), contentWorld: PageBridge.world, name: PageBridge.handlerName
        )
        // The console's notifier (web/hub/src/components/shell/owner-notifier.tsx)
        // posts from the page's own world; only the console's main frame is heard.
        configuration.userContentController.addScriptMessageHandler(
            WeakReplyHandler(self), contentWorld: .page, name: NotifyRequest.handlerName
        )
        setUpWindow(window)
        setUpWebView()
        setUpBeside()
        layOut(in: window)

        stateToken = app.supervisor.observe { [weak self] state in self?.render(state) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    private func setUpWindow(_ window: NSWindow) {
        window.title = "PULSE"
        window.titlebarAppearsTransparent = true
        // The console names itself in its sidebar; a second "PULSE" beside the
        // traffic lights would not be the console's look. The title still
        // names the window in the Window menu, Mission Control and tabs.
        window.titleVisibility = .hidden
        window.tabbingMode = .automatic
        window.tabbingIdentifier = "PULSE.console"
        window.minSize = NSSize(width: 520, height: 420)
        window.collectionBehavior.insert(.fullScreenPrimary)
        window.isReleasedWhenClosed = false
        window.backgroundColor = Palette.paper
        window.appearance = app.appearance(for: app.rememberedTheme)
        window.delegate = self
    }

    private func setUpWebView() {
        webView.navigationDelegate = self
        webView.uiDelegate = self
        webView.allowsBackForwardNavigationGestures = true
        webView.allowsMagnification = false
        // A force-click preview would load other sites inside the app.
        webView.allowsLinkPreview = false
        webView.willOpenContextMenu = { [weak self] menu in self?.tailor(menu) }
        webView.pageZoom = app.pageZoom
        webView.underPageBackgroundColor = Palette.paper
        // Web Inspector on request, as in Safari:
        // `defaults write io.github.manishtiwari25.pulse DeveloperExtras -bool YES`.
        if #available(macOS 13.3, *), UserDefaults.standard.bool(forKey: "DeveloperExtras") {
            webView.isInspectable = true
        }
        // Transparent until the page paints, so the window's paper shows
        // instead of a white flash.
        Self.setFlag(webView, "_setDrawsBackground:", false)
        // A test run's window may sit behind the owner's; keep its page
        // rendering as if seen (WebKit SPI, used only for test runs).
        if app.ephemeral {
            Self.setFlag(webView, "_setWindowOcclusionDetectionEnabled:", false)
        }
        observations = [
            webView.observe(\.title, options: [.new]) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.updateTitle() }
            },
            webView.observe(\.url, options: [.new]) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.urlChanged() }
            },
        ]

        status.onAction = { [weak self] id in self?.statusAction(id) }
        findBar.onFind = { [weak self] text, backwards, again in self?.find(text, backwards: backwards, again: again) }
        findBar.onClose = { [weak self] in
            guard let self else { return }
            self.window?.makeFirstResponder(self.webView)
        }
    }

    private func setUpBeside() {
        beside.isHidden = true
        if app.ephemeral { Self.setFlag(beside.webView, "_setWindowOcclusionDetectionEnabled:", false) }
        beside.onConsolePage = { [weak self] url in
            guard let self else { return }
            self.log.notice("the panel was sent to a console page; the console shows it")
            self.webView.load(URLRequest(url: url))
        }
        beside.onOtherApp = { [weak self] url, ask in
            guard let self else { return }
            self.app.openInOtherApp(url, ask: ask, window: self.window)
        }
        beside.onDownload = { [weak self] download in
            guard let self else { return }
            self.app.downloads.track(download, from: self)
        }
        beside.onClose = { [weak self] in self?.hideBeside() }
    }

    /// Calls a BOOL setter WebKit may or may not have, through its
    /// implementation: key-value coding guesses accessor names and throws on
    /// a miss, and an exception during launch leaves the app half started.
    @discardableResult
    static func setFlag(_ object: NSObject, _ selectorName: String, _ value: Bool) -> Bool {
        let selector = NSSelectorFromString(selectorName)
        guard object.responds(to: selector), let method = class_getInstanceMethod(type(of: object), selector) else {
            return false
        }
        typealias Setter = @convention(c) (AnyObject, Selector, Bool) -> Void
        unsafeBitCast(method_getImplementation(method), to: Setter.self)(object, selector, value)
        return true
    }

    private func layOut(in window: NSWindow) {
        let content = NSView()
        window.contentView = content
        // The console, and beside it the browser panel while one is open.
        split.isVertical = true
        split.dividerStyle = .thin
        split.addArrangedSubview(webView)
        split.addArrangedSubview(beside)
        // The console gives way when the window is resized; the panel keeps its width.
        split.setHoldingPriority(NSLayoutConstraint.Priority(rawValue: 260), forSubviewAt: 1)
        for view in [strip, split, status, findBar, toast] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(view)
        }
        let consoleWidth = webView.widthAnchor.constraint(greaterThanOrEqualToConstant: 320)
        consoleWidth.priority = .defaultHigh
        // Below the title bar, or the whole window in full screen.
        let top = (window.contentLayoutGuide as? NSLayoutGuide)?.topAnchor ?? content.topAnchor
        NSLayoutConstraint.activate([
            strip.topAnchor.constraint(equalTo: content.topAnchor),
            strip.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            strip.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            strip.bottomAnchor.constraint(equalTo: top),

            split.topAnchor.constraint(equalTo: top),
            split.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            split.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            split.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            consoleWidth,

            status.topAnchor.constraint(equalTo: top),
            status.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            status.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            status.bottomAnchor.constraint(equalTo: content.bottomAnchor),

            findBar.topAnchor.constraint(equalTo: top, constant: 10),
            findBar.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -14),

            toast.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -18),
            toast.centerXAnchor.constraint(equalTo: content.centerXAnchor),
        ])
        status.isHidden = true
    }

    // MARK: - The console's state

    private func render(_ state: HubState) {
        switch state {
        case .ready(let url):
            let moved = policy.map { $0.console != url } ?? false
            policy = LinkPolicy(console: url)
            beside.policy = policy
            if !hasPage || loadFailed || moved {
                if !covering { showStatus(.opening) }
                loadWanted()
            } else {
                hideStatus()
            }
        case .connecting(let words), .reconnecting(_, let words):
            // A page that is up keeps showing; it says "Reconnecting" itself.
            if !hasPage || loadFailed {
                showStatus(.busy(words))
            }
        case .problem(let problem):
            showStatus(StatusContent(problem: problem, logTail: app.logTail(problem.log)))
        }
        updateTitle()
    }

    private func loadWanted() {
        guard let policy else { return }
        let target = wanted.map { policy.isConsole($0) ? $0 : policy.rebase($0) } ?? policy.console
        loadFailed = false
        log.notice("loading \(target.absoluteString, privacy: .public)")
        webView.load(URLRequest(url: target))
    }

    private func showStatus(_ content: StatusContent) {
        covering = true
        status.show(content)
        status.isHidden = false
        status.alphaValue = 1
        strip.canvas = Palette.paper
        strip.sidebar = nil
        updateTitle()
    }

    private func hideStatus() {
        guard covering else { return }
        covering = false
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.18
            status.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            MainActor.assumeIsolated {
                guard let self, !self.covering else { return }
                self.status.isHidden = true
                self.status.alphaValue = 1
            }
        })
        if let chrome { apply(chrome) }
        updateTitle()
        // Typing goes to the page again, also after selecting text on the cover.
        let responder = window?.firstResponder
        let onCover = (responder as? NSView).map { $0 === status || $0.isDescendant(of: status) } ?? false
        if responder == nil || responder === window || onCover {
            window?.makeFirstResponder(webView)
        }
    }

    private func statusAction(_ id: String) {
        switch id {
        case "retry": app.supervisor.retry()
        case "reload":
            crashes.removeAll()
            loadFailures.removeAll()
            loadWanted()
        case "choose-binary": app.chooseBinary(for: window)
        case "choose-folder": app.chooseFirstProject(for: window)
        case "open-log": app.openCurrentLog()
        case "restart-service": app.restartConsoleService()
        default: break
        }
    }

    private func updateTitle() {
        let title = covering ? "" : (webView.title ?? "")
        window?.title = title.isEmpty ? "PULSE" : title
    }

    private func urlChanged() {
        guard let url = webView.url, let policy, policy.isConsole(url) else { return }
        if url != wanted { log.notice("page \(url.path, privacy: .public)\(url.query.map { "?" + $0 } ?? "", privacy: .public)") }
        wanted = url
        if window?.isKeyWindow == true || app.windows.count == 1 {
            app.rememberPage(url)
        }
    }

    // MARK: - Navigation

    func webView(
        _ webView: WKWebView,
        decidePolicyFor action: WKNavigationAction,
        decisionHandler: @escaping @MainActor @Sendable (WKNavigationActionPolicy) -> Void
    ) {
        if action.shouldPerformDownload {
            decisionHandler(.download)
            return
        }
        guard let url = action.request.url else {
            decisionHandler(.cancel)
            return
        }
        guard let policy else {
            decisionHandler(.cancel)
            return
        }
        // A plain click on a link: no modifier, no middle button, no script.
        // It is what opens a web page beside the console; a ⌘-click or a
        // window.open goes to the default browser as before.
        let plainClick = action.navigationType == .linkActivated
            && action.modifierFlags.isDisjoint(with: [.command, .shift, .option, .control])
            && action.buttonNumber != Self.middleButton
        // Subframes hold the console's own files (the chat's artifact viewer:
        // sandboxed HTML, PDF). They never load another site inside the
        // console; a link clicked in one opens beside it, or in the browser.
        if let frame = action.targetFrame, !frame.isMainFrame {
            let scheme = url.scheme?.lowercased() ?? ""
            if policy.isConsole(url) || scheme == "about" || scheme == "blob" || scheme == "data" {
                decisionHandler(.allow)
                return
            }
            decisionHandler(.cancel)
            if action.navigationType == .linkActivated, scheme == "http" || scheme == "https" {
                if plainClick { showBeside(url) } else { NSWorkspace.shared.open(url) }
            }
            return
        }
        // Nor does a framed file steer the console's own window (target=_top
        // from a sandboxed frame has no origin): the hub's sandbox forbids
        // it, and the app does not take the frame's word either.
        if action.targetFrame?.isMainFrame == true, !action.sourceFrame.isMainFrame,
           !isConsoleOrigin(action.sourceFrame.securityOrigin) {
            log.notice("refused a navigation of the window from a framed file to \(url.absoluteString, privacy: .public)")
            decisionHandler(.cancel)
            return
        }
        // No target frame: target=_blank (WebKit asks here first, then
        // createWebViewWith); ⌘-click and middle-click ask for a new tab too.
        // buttonNumber is WebKit's own mask: 1 left, 2 right, 4 middle.
        let asksForNew = action.targetFrame == nil || (action.navigationType == .linkActivated
            && (action.modifierFlags.contains(.command) || action.buttonNumber == Self.middleButton))
        switch policy.target(for: url, newWindow: asksForNew, click: plainClick) {
        case .here:
            decisionHandler(.allow)
        case .redirect(let canonical):
            decisionHandler(.cancel)
            webView.load(URLRequest(url: canonical))
        case .newTab(let page):
            decisionHandler(.cancel)
            // ⌘-click opens behind (⇧ brings it forward), as in Safari.
            let behind = action.modifierFlags.contains(.command) && !action.modifierFlags.contains(.shift)
            app.openTab(page, from: self, background: behind)
        case .beside(let page):
            decisionHandler(.cancel)
            showBeside(page)
        case .browser(let page):
            decisionHandler(.cancel)
            NSWorkspace.shared.open(page)
        case .otherApp(let link, let ask):
            decisionHandler(.cancel)
            app.openInOtherApp(link, ask: ask, window: window)
        case .refuse:
            log.notice("refused a navigation to \(url.absoluteString, privacy: .public)")
            decisionHandler(.cancel)
        }
    }

    func webView(
        _ webView: WKWebView,
        decidePolicyFor response: WKNavigationResponse,
        decisionHandler: @escaping @MainActor @Sendable (WKNavigationResponsePolicy) -> Void
    ) {
        if let http = response.response as? HTTPURLResponse,
           let disposition = http.value(forHTTPHeaderField: "Content-Disposition"),
           disposition.lowercased().hasPrefix("attachment") {
            decisionHandler(.download)
            return
        }
        decisionHandler(response.canShowMIMEType ? .allow : .download)
    }

    func webView(_ webView: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) {
        downloadStarted(download)
    }

    func webView(_ webView: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) {
        downloadStarted(download)
    }

    /// A tab opened for a link that turned out to be a file has nothing to
    /// show: it closes again, as Safari's does, and the file's note goes to
    /// the window the link was in. The download itself belongs to the
    /// website data store and carries on.
    private func downloadStarted(_ download: WKDownload) {
        guard !hasPage, let home = opener.flatMap({ opener in app.windows.first { $0 === opener } })
            ?? app.windows.first(where: { $0 !== self && $0.hasPage }) else {
            app.downloads.track(download, from: self)
            return
        }
        app.downloads.track(download, from: home)
        log.notice("the new tab's page is a download; closing the tab")
        Task { @MainActor [weak self] in self?.window?.close() }
    }

    /// Whether a frame's origin is the console's own (served by the hub).
    private func isConsoleOrigin(_ origin: WKSecurityOrigin) -> Bool {
        guard let policy, let url = Self.url(of: origin) else { return false }
        return policy.isConsole(url)
    }

    /// A frame's origin as a URL the link policy can judge; nil for an
    /// opaque origin (a sandboxed frame, about:blank), which is nobody's.
    private static func url(of origin: WKSecurityOrigin) -> URL? {
        guard !origin.host.isEmpty else { return nil }
        var parts = URLComponents()
        parts.scheme = origin.protocol
        parts.host = origin.host
        parts.port = origin.port == 0 ? nil : origin.port
        parts.path = "/"
        return parts.url
    }

    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        guard let url = webView.url, policy?.isConsole(url) == true else { return }
        let first = !hasPage
        hasPage = true
        loadFailed = false
        if case .problem = app.supervisor.state {} else { hideStatus() }
        if first { window?.makeFirstResponder(webView) }
        log.notice("showing \(url.absoluteString, privacy: .public)")
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        app.pageFinished(in: self)
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        failed(error)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        failed(error)
    }

    private func failed(_ error: Error) {
        let error = error as NSError
        // Cancelled, or turned into a download or another app's link: not a failure.
        if error.domain == NSURLErrorDomain, error.code == NSURLErrorCancelled { return }
        if error.domain == "WebKitErrorDomain", error.code == 102 || error.code == 204 { return }
        let failing = error.userInfo[NSURLErrorFailingURLErrorKey] as? URL
        log.error("load failed: \(error.domain, privacy: .public) \(error.code, privacy: .public) \(failing?.absoluteString ?? "", privacy: .public)")
        guard error.domain == NSURLErrorDomain else { return }
        if let failing, policy?.isConsole(failing) == true { wanted = failing }
        loadFailed = true
        let now = Date()
        loadFailures = loadFailures.filter { now.timeIntervalSince($0) < 30 } + [now]
        if loadFailures.count >= 4, case .ready = app.supervisor.state {
            // PULSE answers its health check, yet this page will not load:
            // say so instead of retrying in a loop.
            showStatus(StatusContent(
                title: "This page didn't load",
                detail: "PULSE answers, but \(failing?.path ?? "the page") did not load: \(error.localizedDescription)",
                actions: [.init(title: "Try Again", id: "reload", primary: true)]
            ))
            return
        }
        // The supervisor finds out whether the console is down; its next
        // state decides when this window loads again.
        showStatus(.busy("Reconnecting to PULSE…"))
        app.supervisor.consoleLooksDown()
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        let now = Date()
        crashes = crashes.filter { now.timeIntervalSince($0) < 60 } + [now]
        if let url = webView.url, policy?.isConsole(url) == true { wanted = url }
        hasPage = false
        log.error("the page's web process ended (\(self.crashes.count, privacy: .public) in a minute)")
        if crashes.count >= 3 {
            showStatus(StatusContent(
                title: "This page keeps closing",
                detail: "WebKit ended the page three times in a minute. Reload to try once more.",
                actions: [.init(title: "Reload", id: "reload", primary: true)]
            ))
            return
        }
        if case .ready = app.supervisor.state { loadWanted() } else { render(app.supervisor.state) }
    }

    // MARK: - Windows, panels, files

    func webView(
        _ webView: WKWebView,
        createWebViewWith configuration: WKWebViewConfiguration,
        for action: WKNavigationAction,
        windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        guard let url = action.request.url, let policy else { return nil }
        let now = Date()
        popups = popups.filter { now.timeIntervalSince($0) < 10 } + [now]
        guard popups.count <= 5 else {
            log.error("ignored a new-window request: more than five in ten seconds")
            return nil
        }
        switch policy.target(for: url, newWindow: true) {
        case .newTab(let page): app.openTab(page, from: self, background: false)
        case .beside(let page): showBeside(page)
        case .browser(let page): NSWorkspace.shared.open(page)
        case .otherApp(let link, let ask): app.openInOtherApp(link, ask: ask, window: window)
        case .here, .redirect, .refuse: break
        }
        return nil
    }

    func webViewDidClose(_ webView: WKWebView) {
        window?.performClose(nil)
    }

    func webView(
        _ webView: WKWebView,
        runOpenPanelWith parameters: WKOpenPanelParameters,
        initiatedByFrame frame: WKFrameInfo,
        completionHandler: @escaping @MainActor @Sendable ([URL]?) -> Void
    ) {
        guard let window else {
            completionHandler(nil)
            return
        }
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = parameters.allowsMultipleSelection
        panel.canChooseDirectories = parameters.allowsDirectories
        panel.canChooseFiles = true
        panel.resolvesAliases = true
        panel.beginSheetModal(for: window) { response in
            completionHandler(response == .OK ? panel.urls : nil)
        }
    }

    func webView(
        _ webView: WKWebView,
        runJavaScriptAlertPanelWithMessage message: String,
        initiatedByFrame frame: WKFrameInfo,
        completionHandler: @escaping @MainActor @Sendable () -> Void
    ) {
        let alert = NSAlert()
        alert.messageText = message
        alert.addButton(withTitle: "OK")
        present(alert) { _ in completionHandler() }
    }

    func webView(
        _ webView: WKWebView,
        runJavaScriptConfirmPanelWithMessage message: String,
        initiatedByFrame frame: WKFrameInfo,
        completionHandler: @escaping @MainActor @Sendable (Bool) -> Void
    ) {
        let alert = NSAlert()
        alert.messageText = message
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Cancel")
        present(alert) { completionHandler($0 == .alertFirstButtonReturn) }
    }

    func webView(
        _ webView: WKWebView,
        runJavaScriptTextInputPanelWithPrompt prompt: String,
        defaultText: String?,
        initiatedByFrame frame: WKFrameInfo,
        completionHandler: @escaping @MainActor @Sendable (String?) -> Void
    ) {
        let alert = NSAlert()
        alert.messageText = prompt
        let input = NSTextField(string: defaultText ?? "")
        input.frame = NSRect(x: 0, y: 0, width: 280, height: 24)
        alert.accessoryView = input
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Cancel")
        present(alert) { completionHandler($0 == .alertFirstButtonReturn ? input.stringValue : nil) }
    }

    /// The console's own page may use the microphone: the chat's dictation
    /// button records there and posts the sound to the hub's own transcribe
    /// route on this machine (nothing said leaves it). A camera, a framed
    /// file and any other origin are refused; the answer is the app's, so
    /// WebKit asks nothing more and macOS asks once for the app itself.
    func webView(
        _ webView: WKWebView,
        requestMediaCapturePermissionFor origin: WKSecurityOrigin,
        initiatedByFrame frame: WKFrameInfo,
        type: WKMediaCaptureType,
        decisionHandler: @escaping @MainActor @Sendable (WKPermissionDecision) -> Void
    ) {
        let capture: MediaCapture
        switch type {
        case .microphone: capture = .microphone
        case .camera: capture = .camera
        case .cameraAndMicrophone: capture = .cameraAndMicrophone
        @unknown default:
            decisionHandler(.deny)
            return
        }
        guard let policy, let url = Self.url(of: origin),
              policy.allowsCapture(capture, from: url, mainFrame: frame.isMainFrame) else {
            log.notice("refused \(String(describing: capture), privacy: .public) capture for \(origin.host, privacy: .public):\(origin.port, privacy: .public)")
            decisionHandler(.deny)
            return
        }
        decisionHandler(.grant)
    }

    private func present(_ alert: NSAlert, then done: @escaping @MainActor (NSApplication.ModalResponse) -> Void) {
        if let window {
            alert.beginSheetModal(for: window) { response in done(response) }
        } else {
            done(alert.runModal())
        }
    }

    // MARK: - What the page tells the app

    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        guard message.frameInfo.isMainFrame else { return }
        if let report = PageContextMenu(message.body) {
            contextLink = (report.link, Date())
            return
        }
        guard let chrome = PageChrome(message.body) else { return }
        if chrome.visible != self.chrome?.visible {
            log.notice("the page is \(chrome.visible ? "visible" : "hidden", privacy: .public)")
        }
        apply(chrome)
    }

    /// The page's notifier: what macOS allows, and a notification to show.
    /// Only the console's own page in the main frame; anything else is refused.
    func userContentController(
        _ controller: WKUserContentController,
        didReceive message: WKScriptMessage,
        replyHandler: @escaping @MainActor @Sendable (Any?, String?) -> Void
    ) {
        guard message.frameInfo.isMainFrame, let policy,
              let origin = Self.url(of: message.frameInfo.securityOrigin), policy.isConsole(origin) else {
            replyHandler(nil, "not the console")
            return
        }
        guard let request = NotifyRequest(message.body) else {
            replyHandler(nil, "not a notification request")
            return
        }
        switch request {
        case .permission(let ask):
            Task { @MainActor in replyHandler(await self.app.notifications.permission(ask: ask), nil) }
        case .notify(let note):
            app.notifications.post(note)
            replyHandler(true, nil)
        }
    }

    private func apply(_ chrome: PageChrome) {
        self.chrome = chrome
        app.rememberTheme(chrome.choice)
        window?.appearance = app.appearance(for: chrome.choice)
        guard !covering else { return }
        if let canvas = chrome.canvas {
            strip.canvas = canvas
            webView.underPageBackgroundColor = canvas
            window?.backgroundColor = canvas
        }
        strip.sidebar = chrome.sidebar
        strip.zoom = webView.pageZoom
    }

    // MARK: - The right-click menu

    /// WebKit proposes a browser's menu; ContextMenu says what an app keeps
    /// of it. The link under the pointer is what the page reported a moment
    /// ago (the report travels ahead of the menu), and the open item that
    /// stays goes the way a ⌘-click goes, through createWebViewWith.
    private func tailor(_ menu: NSMenu) {
        let link = contextLink.flatMap { Date().timeIntervalSince($0.at) < 2 ? $0.url : nil }
        contextLink = nil
        for item in menu.items {
            switch ContextMenu.edit(item.identifier?.rawValue, link: link, policy: policy) {
            case .keep: break
            case .drop: menu.removeItem(item)
            case .title(let title): item.title = title
            }
        }
        let keeps = ContextMenu.keepsSeparators(menu.items.map(\.isSeparatorItem))
        for (item, keep) in zip(menu.items, keeps) where !keep { menu.removeItem(item) }
        contextMenu = menu
        let titles = menu.items.filter { !$0.isSeparatorItem }.map(\.title)
        log.notice("context menu: \(titles.isEmpty ? "(none)" : titles.joined(separator: " | "), privacy: .public)")
    }

    /// Test hook: closes the menu a right-click opened.
    func dismissContextMenu() {
        contextMenu?.cancelTracking()
        contextMenu = nil
    }

    // MARK: - Commands (menus)

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        let showing = hasPage && !loadFailed && policy != nil
        switch item.action {
        case #selector(consoleGoBack(_:)): return showing && webView.canGoBack
        case #selector(consoleGoForward(_:)): return showing && webView.canGoForward
        case #selector(zoomPageOut(_:)): return app.pageZoom > AppDelegate.zoomSteps.first!
        case #selector(zoomPageIn(_:)): return app.pageZoom < AppDelegate.zoomSteps.last!
        case #selector(actualPageSize(_:)): return app.pageZoom != 1
        case #selector(setConsoleAppearance(_:)):
            let choice = chrome?.choice ?? app.rememberedTheme
            item.state = (item.representedObject as? String) == choice ? .on : .off
            return showing
        case #selector(showFind(_:)), #selector(findNextMatch(_:)), #selector(findPreviousMatch(_:)),
             #selector(useSelectionForFind(_:)), #selector(printConsole(_:)), #selector(openSearch(_:)),
             #selector(openAsk(_:)), #selector(showForYou(_:)):
            return showing
        case #selector(openInBrowser(_:)), #selector(copyPageLink(_:)):
            return policy != nil
        case #selector(toggleBeside(_:)):
            item.title = beside.isHidden ? "Show Browser Panel" : "Hide Browser Panel"
            return true
        default:
            return true
        }
    }

    @objc func reloadConsole(_ sender: Any?) {
        if hasPage && !loadFailed {
            webView.reload()
        } else if case .ready = app.supervisor.state {
            loadWanted()
        } else {
            app.supervisor.retry()
        }
    }

    @objc func reloadConsoleFromOrigin(_ sender: Any?) {
        if hasPage && !loadFailed { webView.reloadFromOrigin() } else { reloadConsole(sender) }
    }

    @objc func consoleGoBack(_ sender: Any?) { webView.goBack() }
    @objc func consoleGoForward(_ sender: Any?) { webView.goForward() }

    @objc func zoomPageIn(_ sender: Any?) { app.stepZoom(by: 1) }
    @objc func zoomPageOut(_ sender: Any?) { app.stepZoom(by: -1) }
    @objc func actualPageSize(_ sender: Any?) { app.setPageZoom(1) }

    func applyZoom(_ zoom: CGFloat) {
        webView.pageZoom = zoom
        strip.zoom = zoom
    }

    /// Go menu: the sidebar's own link when the page has one, else a load.
    @objc func goToRoute(_ sender: NSMenuItem) {
        guard let path = sender.representedObject as? String else { return }
        navigate(to: path)
    }

    func navigate(to path: String) {
        guard let policy, let target = URL(string: path, relativeTo: policy.console)?.absoluteURL else {
            if let base = app.supervisor.setup.hubOverride ?? URL(string: "http://127.0.0.1:\(app.supervisor.setup.port)/") {
                wanted = URL(string: path, relativeTo: base)?.absoluteURL
            }
            return
        }
        guard hasPage && !loadFailed else {
            wanted = target
            if case .ready = app.supervisor.state { loadWanted() }
            return
        }
        window?.makeFirstResponder(webView)
        webView.evaluateJavaScript(PageBridge.clickSidebarLink(path), in: nil, in: PageBridge.world) { [weak self] result in
            if case .success(let clicked) = result, clicked as? Bool == true { return }
            self?.webView.load(URLRequest(url: target))
        }
    }

    @objc func showForYou(_ sender: Any?) {
        webView.evaluateJavaScript(PageBridge.showForYou, in: nil, in: PageBridge.world) { [weak self] result in
            if case .success(let done) = result, done as? Bool == true { return }
            self?.navigate(to: "/#for-you")
        }
    }

    @objc func openSearch(_ sender: Any?) { press("[data-palette-trigger]") }
    @objc func openAsk(_ sender: Any?) { press("[data-ask-trigger]") }

    private func press(_ selector: String) {
        window?.makeFirstResponder(webView)
        webView.evaluateJavaScript(PageBridge.press(selector), in: nil, in: PageBridge.world, completionHandler: nil)
    }

    @objc func setConsoleAppearance(_ sender: NSMenuItem) {
        guard let choice = sender.representedObject as? String else { return }
        // In the page's world, the way its own theme toggle does it.
        webView.evaluateJavaScript(PageBridge.setTheme(choice), in: nil, in: .page, completionHandler: nil)
    }

    @objc func openInBrowser(_ sender: Any?) {
        guard let url = (hasPage ? webView.url : nil) ?? wanted ?? policy?.console else { return }
        NSWorkspace.shared.open(url)
    }

    @objc func copyPageLink(_ sender: Any?) {
        guard let url = (hasPage ? webView.url : nil) ?? wanted ?? policy?.console else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(url.absoluteString, forType: .string)
        NSPasteboard.general.setString(url.absoluteString, forType: .URL)
    }

    @objc func printConsole(_ sender: Any?) {
        guard let window, let info = NSPrintInfo.shared.copy() as? NSPrintInfo else { return }
        info.horizontalPagination = .fit
        info.verticalPagination = .automatic
        info.isHorizontallyCentered = false
        info.isVerticallyCentered = false
        let operation = webView.printOperation(with: info)
        operation.showsPrintPanel = true
        operation.showsProgressPanel = true
        // WKWebView's print view prints blank pages without a frame of its own.
        operation.view?.frame = webView.bounds
        operation.runModal(for: window, delegate: nil, didRun: nil, contextInfo: nil)
    }

    @objc func showFind(_ sender: Any?) { findBar.open() }
    @objc func findNextMatch(_ sender: Any?) { findBar.isHidden ? findBar.open() : findBar.findNext() }
    @objc func findPreviousMatch(_ sender: Any?) { findBar.isHidden ? findBar.open() : findBar.findPrevious() }

    @objc func useSelectionForFind(_ sender: Any?) {
        webView.evaluateJavaScript("String(window.getSelection() || '')", in: nil, in: PageBridge.world) { [weak self] result in
            guard let self, case .success(let value) = result, let text = value as? String, !text.isEmpty else { return }
            self.findBar.open(with: String(text.prefix(200)))
        }
    }

    /// The find bar with this text, then the first match (a test hook).
    func findForTesting(_ text: String) {
        findBar.open(with: text)
        find(text, backwards: false, again: false)
    }

    /// WebKit searches from the selection: a new search text first moves the
    /// selection back to the start of the current match, so typing "ta" after
    /// "t" stays on the same word instead of jumping to the next one.
    private func find(_ text: String, backwards: Bool, again: Bool) {
        let configuration = WKFindConfiguration()
        configuration.backwards = backwards
        configuration.caseSensitive = false
        configuration.wraps = true
        let search = { [weak self] in
            guard let self else { return }
            self.webView.find(text, configuration: configuration) { [weak self] result in
                self?.log.notice("find: \(result.matchFound ? "found" : "not found", privacy: .public)")
                self?.findBar.show(found: result.matchFound)
            }
        }
        if again {
            search()
        } else {
            let collapse = "(() => { const s = window.getSelection(); if (s && s.rangeCount) s.collapseToStart(); })()"
            webView.evaluateJavaScript(collapse, in: nil, in: PageBridge.world) { _ in search() }
        }
    }

    // MARK: - The browser panel

    /// Shows a web page beside the console, in the panel.
    func showBeside(_ url: URL) {
        log.notice("beside \(url.absoluteString, privacy: .public)")
        beside.show(url)
        revealBeside()
        window?.makeFirstResponder(beside.webView)
    }

    /// View menu: the panel with nothing in it yet, or away.
    @objc func toggleBeside(_ sender: Any?) {
        if beside.isHidden {
            revealBeside()
            beside.focusAddress()
        } else {
            hideBeside()
        }
    }

    private func revealBeside() {
        guard beside.isHidden else { return }
        beside.isHidden = false
        besideMinWidth.isActive = true
        split.layoutSubtreeIfNeeded()
        // Two fifths of the window, within reason, the first time; the width
        // it had when it was put away after that, as far as the console allows.
        let width = split.bounds.width
        let pane = besideWidth > 0
            ? min(max(besideWidth, 320), max(width - 320 - split.dividerThickness, 320))
            : min(max(width * 0.42, 360), 720)
        split.setPosition(width - pane - split.dividerThickness, ofDividerAt: 0)
    }

    func hideBeside() {
        guard !beside.isHidden else { return }
        log.notice("beside away")
        besideWidth = beside.bounds.width
        beside.isHidden = true
        besideMinWidth.isActive = false
        beside.clear()
        window?.makeFirstResponder(webView)
    }

    override func newWindowForTab(_ sender: Any?) {
        app.openTab(policy?.console ?? PulseSetup.consoleURL(port: PulseSetup.servicePort), from: self, background: false)
    }

    /// The app is quitting: drop the page (and its open connections).
    func unloadForQuit() {
        webView.stopLoading()
        webView.loadHTMLString("", baseURL: nil)
        beside.webView.stopLoading()
        beside.webView.loadHTMLString("", baseURL: nil)
    }

    func showToast(_ text: String, button: String?, action: (() -> Void)?) {
        toast.show(text, button: button, action: action)
    }

    /// The window as the owner sees it (a test hook, see AppDelegate): the
    /// frame with its title bar, the page, and any native view over it. Web
    /// content does not draw into a cached view, so the page comes from
    /// WebKit's own snapshot, placed where the web view sits.
    func snapshot(completion: @escaping @MainActor (NSImage?) -> Void) {
        guard let window, let frameView = window.contentView?.superview else {
            completion(nil)
            return
        }
        let configuration = WKSnapshotConfiguration()
        configuration.afterScreenUpdates = true
        webView.takeSnapshot(with: configuration) { [weak self] page, _ in
            guard let self else {
                completion(nil)
                return
            }
            let compose: @MainActor (NSImage?) -> Void = { besidePage in
                let image = NSImage(size: frameView.bounds.size)
                image.lockFocus()
                Self.draw(frameView, in: frameView.bounds)
                if self.hasPage, let page { page.draw(in: self.webView.convert(self.webView.bounds, to: nil)) }
                if let besidePage { besidePage.draw(in: self.beside.webView.convert(self.beside.webView.bounds, to: nil)) }
                for view in [self.status, self.findBar, self.toast] as [NSView] where !view.isHidden {
                    Self.draw(view, in: view.convert(view.bounds, to: nil))
                }
                image.unlockFocus()
                completion(image)
            }
            // The panel's page is web content too: its own snapshot, when it shows.
            if self.beside.isHidden {
                compose(nil)
            } else {
                self.beside.webView.takeSnapshot(with: configuration) { besidePage, _ in compose(besidePage) }
            }
        }
    }

    private static func draw(_ view: NSView, in rect: NSRect) {
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
        view.cacheDisplay(in: view.bounds, to: rep)
        rep.draw(in: rect)
    }

    // MARK: - Window

    func windowWillClose(_ notification: Notification) {
        if let stateToken { app.supervisor.removeObserver(stateToken) }
        stateToken = nil
        observations.removeAll()
        webView.stopLoading()
        webView.navigationDelegate = nil
        webView.uiDelegate = nil
        webView.configuration.userContentController.removeAllScriptMessageHandlers()
        beside.tearDown()
        app.windowClosed(self)
    }

    func windowDidBecomeKey(_ notification: Notification) {
        if let url = webView.url, policy?.isConsole(url) == true { app.rememberPage(url) }
    }
}

extension StatusContent {
    static let opening = StatusContent(title: "Opening PULSE…", detail: "", busy: true)

    static func busy(_ words: String) -> StatusContent {
        StatusContent(title: words, detail: "", busy: true)
    }

    init(problem: HubProblem, logTail: String?) {
        self.init(title: problem.title, detail: problem.detail)
        self.logTail = logTail
        let retry = Action(title: "Try Again", id: "retry", primary: true)
        switch problem.kind {
        case .pulseMissing:
            actions = [retry, Action(title: "Choose pulse…", id: "choose-binary")]
        case .firstProject:
            actions = [Action(title: "Choose a Folder…", id: "choose-folder", primary: true)]
        case .serviceDown:
            actions = [retry, Action(title: "Restart the Console", id: "restart-service"), Action(title: "Open Log", id: "open-log")]
        case .portBusy, .hubStopped:
            actions = problem.log == nil ? [retry] : [retry, Action(title: "Open Log", id: "open-log")]
        case .unreachable:
            actions = [retry]
        }
    }
}
