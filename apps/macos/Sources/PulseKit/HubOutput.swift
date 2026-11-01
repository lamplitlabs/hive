import Foundation

/// What a line `pulse hub` printed means to the app. The wording is the hub's
/// own (crates/pulse-hub/src/register.rs); only these lines matter, the rest
/// is log text.
public enum HubLine: Equatable, Sendable {
    /// `Serving on http://127.0.0.1:8765 (127.0.0.1 only; …; Ctrl-C to stop)`
    case serving(URL)
    /// `handing over to the pulse.hub service`: the console's "Keep PULSE
    /// running" installed the service, which takes the port next.
    case handingOver
    /// `the pulse.hub service now serves http://127.0.0.1:8765; this hub exits`
    case handedOver(URL)
    /// `The hub could not listen`: something else holds the port.
    case portTaken
    /// `No store at <home>/pulse.sqlite`: nothing was set up on this Mac yet.
    case noStore
    case other

    public static func parse(_ raw: String) -> HubLine {
        let line = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if line.hasPrefix("Serving on "), let url = firstURL(in: line) {
            return .serving(url)
        }
        if line.hasPrefix("handing over to the pulse.hub service") {
            return .handingOver
        }
        if line.hasPrefix("the pulse.hub service now serves"), let url = firstURL(in: line) {
            return .handedOver(url)
        }
        if line.hasPrefix("The hub could not listen") {
            return .portTaken
        }
        if line.hasPrefix("No store at ") {
            return .noStore
        }
        return .other
    }

    /// The first http://127.0.0.1:<port> in the line, as the console's root.
    static func firstURL(in line: String) -> URL? {
        guard let start = line.range(of: "http://127.0.0.1:") else { return nil }
        let digits = line[start.upperBound...].prefix(while: \.isNumber)
        guard let port = Int(digits), (1...65535).contains(port) else { return nil }
        return PulseSetup.consoleURL(port: port)
    }
}

/// Why a console the app started has ended.
public enum HubExit: Equatable, Sendable {
    /// Exit 0 after the handover lines: the service serves the console now.
    case handedOver
    /// Exit 0: a SIGTERM or SIGINT stopped it.
    case stopped
    /// Exit 3: the pulse binary changed on disk (`--restart-on-change`).
    case binaryChanged
    /// Exit 1 after `The hub could not listen`.
    case portTaken
    /// Exit 2 after `No store at …`.
    case noStore
    /// Any other exit code (2 is bad flags, 1 an unexpected error).
    case failed(code: Int32)
    /// Ended by a signal it does not handle (SIGKILL, a crash).
    case killed(signal: Int32)

    /// Classifies a waitpid(2) status together with what the hub printed.
    public static func classify(waitStatus: Int32, sawHandover: Bool, sawPortTaken: Bool, sawNoStore: Bool) -> HubExit {
        let signal = waitStatus & 0x7f
        if signal != 0 && signal != 0x7f {
            return .killed(signal: signal)
        }
        let code = (waitStatus >> 8) & 0xff
        switch code {
        case 0: return sawHandover ? .handedOver : .stopped
        case 3: return .binaryChanged
        case 1 where sawPortTaken: return .portTaken
        case 2 where sawNoStore: return .noStore
        default: return .failed(code: code)
        }
    }

    /// Plain words for the log and the failure screen.
    public var words: String {
        switch self {
        case .handedOver: return "handed the console over to the service"
        case .stopped: return "stopped"
        case .binaryChanged: return "stopped because pulse changed on disk"
        case .portTaken: return "could not listen: the port is taken"
        case .noStore: return "found no PULSE store"
        case .failed(let code): return "exited with code \(code)"
        case .killed(let signal): return "was killed by signal \(signal)"
        }
    }
}
