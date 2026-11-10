import Foundation

/// Where a link the console follows opens.
public enum LinkTarget: Equatable, Sendable {
    /// A console page: load it in this window.
    case here
    /// A console page addressed as localhost: load the 127.0.0.1 form instead,
    /// so the page keeps its origin (writes and storage depend on it).
    case redirect(URL)
    /// A console page that asked for a new window (target=_blank, ⌘-click).
    case newTab(URL)
    /// A web page the owner clicked: the browser panel beside the console,
    /// the way the Copilot app opens a page beside its chat.
    case beside(URL)
    /// A web page: the default browser. The console signs in to MCP servers
    /// and model providers this way (window.open from a script, never a
    /// click), and their callbacks reach the hub there; a ⌘-click asks for
    /// it too.
    case browser(URL)
    /// mailto: and other apps' links: the system's handler.
    case otherApp(URL, ask: Bool)
    /// file:, javascript:, data:, blob: and anything without a scheme never
    /// replace the console. A file dropped outside the chat composer is one.
    case refuse
}

/// What a page asks to record (WebKit's WKMediaCaptureType, without WebKit).
public enum MediaCapture: Equatable, Sendable {
    case microphone
    case camera
    case cameraAndMicrophone
}

/// Decides where main-frame navigations and new-window requests go; subframes
/// (the chat's artifact viewer) are the console's own business, except that
/// a link clicked in one goes where a clicked link goes.
public struct LinkPolicy: Equatable, Sendable {
    public var console: URL

    public init(console: URL) {
        self.console = console
    }

    /// `newWindow`: the page asked for one (target=_blank, window.open) or
    /// the click did (⌘, middle button). `click`: a plain click on a link -
    /// no modifier, no script - which is what opens a web page beside the
    /// console; anything else sends it to the default browser.
    public func target(for url: URL, newWindow: Bool, click: Bool = false) -> LinkTarget {
        guard let scheme = url.scheme?.lowercased() else { return .refuse }
        switch scheme {
        case "http", "https":
            guard isConsole(url) else { return click ? .beside(url) : .browser(url) }
            let canonical = canonicalConsoleURL(url)
            if newWindow { return .newTab(canonical) }
            return canonical == url ? .here : .redirect(canonical)
        case "about":
            // about:blank and about:srcdoc in place; a blank popup has nothing to show.
            return newWindow ? .refuse : .here
        case "mailto":
            return .otherApp(url, ask: false)
        case "file", "javascript", "data", "blob", "webkit-fake-url", "applewebdata", "x-apple-data-detectors":
            return .refuse
        default:
            return .otherApp(url, ask: true)
        }
    }

    /// Served by the console's hub: same port, loopback host.
    public func isConsole(_ url: URL) -> Bool {
        guard let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https",
              scheme == (console.scheme?.lowercased() ?? "http"),
              let host = url.host?.lowercased(), let consoleHost = console.host?.lowercased() else {
            return false
        }
        guard port(of: url) == port(of: console) else { return false }
        if host == consoleHost { return true }
        return Self.isLoopback(host) && Self.isLoopback(consoleHost)
    }

    /// The same page on the console's own host spelling.
    public func canonicalConsoleURL(_ url: URL) -> URL {
        guard var parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let host = console.host, url.host?.lowercased() != host.lowercased() else {
            return url
        }
        parts.host = host
        return parts.url ?? url
    }

    /// A console page under a console that moved (another port after a
    /// restart): the same path, query and fragment on the new origin.
    public func rebase(_ url: URL) -> URL {
        guard var parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let consoleParts = URLComponents(url: console, resolvingAgainstBaseURL: false) else {
            return console
        }
        parts.scheme = consoleParts.scheme
        parts.host = consoleParts.host
        parts.port = consoleParts.port
        return parts.url ?? console
    }

    /// Whether a page may record. Only the console's own page, in the main
    /// frame, gets the microphone alone: the chat's dictation button records
    /// and posts the sound to the hub's own transcribe route on this machine.
    /// A camera, a framed file (an artifact in the chat's viewer) and any
    /// other origin get nothing.
    public func allowsCapture(_ capture: MediaCapture, from origin: URL, mainFrame: Bool) -> Bool {
        capture == .microphone && mainFrame && isConsole(origin)
    }

    func port(of url: URL) -> Int {
        url.port ?? (url.scheme?.lowercased() == "https" ? 443 : 80)
    }

    static func isLoopback(_ host: String) -> Bool {
        host == "127.0.0.1" || host == "localhost" || host == "::1" || host == "[::1]"
    }
}
