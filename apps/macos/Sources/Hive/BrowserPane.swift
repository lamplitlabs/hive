import AppKit
import HiveKit
import WebKit
import os

/// The browser panel beside the console: the web page a link in the console
/// was clicked for - a pull request, docs, the page an agent read or drove on
/// a dev server - shown inside the window, the way the Copilot app opens a
/// page beside its chat, instead of in the default browser. It is a small
/// browser of its own (back, forward, reload, an address to type, a way out
/// to the default browser) and never the console's: a console page it is
/// sent to goes to the console, a file or script is refused, another app's
/// link asks first, and a download is the window's to note.
@MainActor
final class BrowserPane: NSView, WKNavigationDelegate, WKUIDelegate, NSTextFieldDelegate {
    let webView: WKWebView
    /// Which pages are the console's; nil while the console is unknown.
    var policy: LinkPolicy?
    var onConsolePage: ((URL) -> Void)?
    var onOtherApp: ((URL, _ ask: Bool) -> Void)?
    var onDownload: ((WKDownload) -> Void)?
    var onClose: (() -> Void)?

    private let bar = PaneBar()
    private let back = NSButton()
    private let forward = NSButton()
    private let reload = NSButton()
    private let address = NSTextField()
    private let outside = NSButton()
    private let close = NSButton()
    private let progress = NSProgressIndicator()
    private let note = NSTextField(wrappingLabelWithString: "")
    private var observations: [NSKeyValueObservation] = []
    private let log = Logger(subsystem: AppIdentity.bundleID, category: "beside")

    init(configuration: WKWebViewConfiguration) {
        webView = WKWebView(frame: .zero, configuration: configuration)
        super.init(frame: .zero)
        setAccessibilityRole(.group)
        setAccessibilityLabel("Browser panel")
        setUpWebView()
        setUpBar()
        layOut()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    private func setUpWebView() {
        webView.navigationDelegate = self
        webView.uiDelegate = self
        webView.allowsBackForwardNavigationGestures = true
        webView.allowsMagnification = true
        // A force-click preview would be a third page in the window.
        webView.allowsLinkPreview = false
        webView.underPageBackgroundColor = Palette.paper
        if #available(macOS 13.3, *), UserDefaults.standard.bool(forKey: "DeveloperExtras") {
            webView.isInspectable = true
        }
        observations = [
            webView.observe(\.url, options: [.new]) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.pageChanged() }
            },
            webView.observe(\.title, options: [.new]) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.pageChanged() }
            },
            webView.observe(\.isLoading, options: [.new]) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.loadingChanged() }
            },
            webView.observe(\.estimatedProgress, options: [.new]) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.loadingChanged() }
            },
            webView.observe(\.canGoBack, options: [.new]) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.pageChanged() }
            },
            webView.observe(\.canGoForward, options: [.new]) { [weak self] _, _ in
                MainActor.assumeIsolated { self?.pageChanged() }
            },
        ]
    }

    private func setUpBar() {
        for (button, symbol, label, selector) in [
            (back, "chevron.left", "Back", #selector(goBack)),
            (forward, "chevron.right", "Forward", #selector(goForward)),
            (reload, "arrow.clockwise", "Reload", #selector(reloadPage)),
            (outside, "safari", "Open in Browser", #selector(openOutside)),
            (close, "xmark", "Close Browser Panel", #selector(closePane)),
        ] {
            button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)
            button.bezelStyle = .texturedRounded
            button.isBordered = false
            button.target = self
            button.action = selector
            button.toolTip = label
            button.setAccessibilityLabel(label)
        }
        address.placeholderString = "Address"
        address.font = Fonts.text(12)
        address.bezelStyle = .roundedBezel
        address.controlSize = .small
        address.lineBreakMode = .byTruncatingTail
        address.cell?.isScrollable = true
        address.cell?.wraps = false
        address.usesSingleLineMode = true
        address.delegate = self
        address.target = self
        address.action = #selector(go)
        address.setAccessibilityLabel("Address")

        let row = NSStackView(views: [back, forward, reload, address, outside, close])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = 6
        row.edgeInsets = NSEdgeInsets(top: 0, left: 8, bottom: 0, right: 8)
        row.translatesAutoresizingMaskIntoConstraints = false
        address.setContentHuggingPriority(.defaultLow, for: .horizontal)
        address.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        bar.addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: bar.leadingAnchor),
            row.trailingAnchor.constraint(equalTo: bar.trailingAnchor),
            row.topAnchor.constraint(equalTo: bar.topAnchor),
            row.bottomAnchor.constraint(equalTo: bar.bottomAnchor),
        ])

        progress.style = .bar
        progress.isIndeterminate = false
        progress.minValue = 0
        progress.maxValue = 1
        progress.controlSize = .small
        progress.isHidden = true

        note.font = Fonts.text(13)
        note.textColor = Palette.muted
        note.alignment = .center
        note.isHidden = true
        note.isSelectable = true
    }

    private func layOut() {
        for view in [bar, webView, progress, note] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            bar.topAnchor.constraint(equalTo: topAnchor),
            bar.leadingAnchor.constraint(equalTo: leadingAnchor),
            bar.trailingAnchor.constraint(equalTo: trailingAnchor),
            bar.heightAnchor.constraint(equalToConstant: 38),

            webView.topAnchor.constraint(equalTo: bar.bottomAnchor),
            webView.leadingAnchor.constraint(equalTo: leadingAnchor),
            webView.trailingAnchor.constraint(equalTo: trailingAnchor),
            webView.bottomAnchor.constraint(equalTo: bottomAnchor),

            progress.topAnchor.constraint(equalTo: bar.bottomAnchor, constant: -2),
            progress.leadingAnchor.constraint(equalTo: leadingAnchor),
            progress.trailingAnchor.constraint(equalTo: trailingAnchor),

            note.centerXAnchor.constraint(equalTo: webView.centerXAnchor),
            note.centerYAnchor.constraint(equalTo: webView.centerYAnchor),
            note.widthAnchor.constraint(lessThanOrEqualTo: webView.widthAnchor, constant: -48),
        ])
    }

    // MARK: - What the window asks

    /// The page the panel shows, when it shows one.
    var page: URL? {
        guard let url = webView.url, url.scheme != "about" else { return nil }
        return url
    }

    func show(_ url: URL) {
        hideNote()
        webView.load(URLRequest(url: url))
    }

    /// Nothing to show: the page goes, with whatever it was playing.
    func clear() {
        webView.stopLoading()
        webView.load(URLRequest(url: URL(string: "about:blank")!))
        hideNote()
        address.stringValue = ""
    }

    func focusAddress() {
        window?.makeFirstResponder(address)
    }

    /// The window is closing: nothing calls back after this.
    func tearDown() {
        observations.removeAll()
        webView.stopLoading()
        webView.navigationDelegate = nil
        webView.uiDelegate = nil
    }

    // MARK: - The bar

    @objc private func goBack() { webView.goBack() }
    @objc private func goForward() { webView.goForward() }
    @objc private func reloadPage() {
        if page != nil { webView.reload() } else { go() }
    }

    @objc private func openOutside() {
        guard let url = page ?? TypedAddress.url(address.stringValue) else { return }
        NSWorkspace.shared.open(url)
    }

    @objc private func closePane() { onClose?() }

    @objc private func go() {
        guard let url = TypedAddress.url(address.stringValue) else {
            NSSound.beep()
            return
        }
        show(url)
        window?.makeFirstResponder(webView)
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        guard control === address, selector == #selector(NSResponder.cancelOperation(_:)) else { return false }
        address.stringValue = page?.absoluteString ?? ""
        window?.makeFirstResponder(webView)
        return true
    }

    private func pageChanged() {
        back.isEnabled = webView.canGoBack
        forward.isEnabled = webView.canGoForward
        let shown = page?.absoluteString ?? ""
        // Not under the owner's typing.
        if address.currentEditor() == nil { address.stringValue = shown }
        address.toolTip = shown.isEmpty ? nil : shown
        outside.isEnabled = !shown.isEmpty
        let title = webView.title ?? ""
        setAccessibilityLabel(title.isEmpty ? "Browser panel" : "Browser panel: \(title)")
    }

    private func loadingChanged() {
        progress.doubleValue = webView.estimatedProgress
        progress.isHidden = !webView.isLoading || webView.estimatedProgress >= 1
    }

    private func showNote(_ text: String) {
        note.stringValue = text
        note.isHidden = false
    }

    private func hideNote() {
        note.isHidden = true
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
        // A page's own frames are its business.
        if let frame = action.targetFrame, !frame.isMainFrame {
            decisionHandler(.allow)
            return
        }
        let scheme = url.scheme?.lowercased() ?? ""
        if scheme == "about" {
            decisionHandler(.allow)
            return
        }
        guard let policy else {
            decisionHandler(scheme == "http" || scheme == "https" ? .allow : .cancel)
            return
        }
        switch policy.target(for: url, newWindow: false, click: true) {
        case .beside, .browser:
            // A new window asked for (target=_blank, window.open) opens here.
            if action.targetFrame == nil {
                decisionHandler(.cancel)
                webView.load(URLRequest(url: url))
            } else {
                decisionHandler(.allow)
            }
        case .here, .redirect(_), .newTab(_):
            decisionHandler(.cancel)
            onConsolePage?(policy.canonicalConsoleURL(url))
        case .otherApp(let link, let ask):
            decisionHandler(.cancel)
            onOtherApp?(link, ask)
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
        onDownload?(download)
    }

    func webView(_ webView: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) {
        onDownload?(download)
    }

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        hideNote()
    }

    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        if let url = webView.url { log.notice("showing \(url.absoluteString, privacy: .public)") }
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        failed(error)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        failed(error)
    }

    private func failed(_ error: Error) {
        let error = error as NSError
        // Cancelled, or turned into a download: not a failure.
        if error.domain == NSURLErrorDomain, error.code == NSURLErrorCancelled { return }
        if error.domain == "WebKitErrorDomain", error.code == 102 || error.code == 204 { return }
        let failing = (error.userInfo[NSURLErrorFailingURLErrorKey] as? URL)?.absoluteString ?? "the page"
        log.error("load failed: \(error.domain, privacy: .public) \(error.code, privacy: .public) \(failing, privacy: .public)")
        showNote("\(failing) did not load: \(error.localizedDescription)")
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        log.error("the panel's web process ended")
        webView.reload()
    }

    // MARK: - Windows, panels, dialogs

    func webView(
        _ webView: WKWebView,
        createWebViewWith configuration: WKWebViewConfiguration,
        for action: WKNavigationAction,
        windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        // The panel is one page: what asks for a new window loads here.
        if let url = action.request.url, let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https" {
            webView.load(URLRequest(url: url))
        }
        return nil
    }

    func webViewDidClose(_ webView: WKWebView) {
        onClose?()
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

    /// The panel shows other people's pages: none of them gets a microphone
    /// or camera here (the console's own page, which does dictate, is the
    /// console window's; see ConsoleWindowController). Keep this a deny.
    func webView(
        _ webView: WKWebView,
        requestMediaCapturePermissionFor origin: WKSecurityOrigin,
        initiatedByFrame frame: WKFrameInfo,
        type: WKMediaCaptureType,
        decisionHandler: @escaping @MainActor @Sendable (WKPermissionDecision) -> Void
    ) {
        decisionHandler(.deny)
    }

    private func present(_ alert: NSAlert, then done: @escaping @MainActor (NSApplication.ModalResponse) -> Void) {
        if let window {
            alert.beginSheetModal(for: window) { response in done(response) }
        } else {
            done(alert.runModal())
        }
    }
}

/// The panel's bar: the console's card colour with a hairline under it, so
/// the panel reads as part of the window rather than a browser dropped in.
final class PaneBar: NSView {
    override var isOpaque: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        Palette.card.setFill()
        bounds.fill()
        Palette.border.setFill()
        let line = 1 / (window?.backingScaleFactor ?? 2)
        NSRect(x: 0, y: 0, width: bounds.width, height: line).fill()
    }
}
