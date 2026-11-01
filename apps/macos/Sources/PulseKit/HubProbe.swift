import Foundation

/// What answers at the console's address.
public enum ProbeResult: Equatable, Sendable {
    /// A PULSE hub: GET /api/v1/health said `ok` and lists the commons.
    case pulse
    /// Something else holds the port.
    case notPulse(String)
    /// Nothing answers (refused, reset, timed out).
    case unreachable(String)
}

/// Asks GET /api/v1/health, the hub's cheapest route (one store query, a few
/// milliseconds; crates/pulse-hub/src/read.rs).
public struct HubProbe: Sendable {
    public var timeout: TimeInterval

    public init(timeout: TimeInterval = 2.5) {
        self.timeout = timeout
    }

    static let session: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        config.urlCache = nil
        config.waitsForConnectivity = false
        config.httpMaximumConnectionsPerHost = 2
        return URLSession(configuration: config)
    }()

    public func check(_ console: URL) async -> ProbeResult {
        var request = URLRequest(url: console.appendingPathComponent("api/v1/health"))
        request.timeoutInterval = timeout
        request.cachePolicy = .reloadIgnoringLocalCacheData
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        do {
            let (data, response) = try await Self.session.data(for: request)
            guard let http = response as? HTTPURLResponse else {
                return .notPulse("not an HTTP answer")
            }
            return Self.interpret(status: http.statusCode, body: data)
        } catch {
            return .unreachable(Self.words(for: error))
        }
    }

    /// 200 with `{"ok": true, "projects": [..., "commons"]}` is PULSE; any
    /// other answer is another program on the port.
    public static func interpret(status: Int, body: Data) -> ProbeResult {
        guard let object = try? JSONSerialization.jsonObject(with: body),
              let health = object as? [String: Any] else {
            return .notPulse("HTTP \(status), not PULSE's JSON")
        }
        guard status == 200, health["ok"] as? Bool == true else {
            return .notPulse("HTTP \(status)")
        }
        guard let projects = health["projects"] as? [Any], projects.contains(where: { ($0 as? String) == "commons" }) else {
            return .notPulse("a health answer without PULSE's shared board")
        }
        return .pulse
    }

    static func words(for error: Error) -> String {
        let code = (error as? URLError)?.code
        switch code {
        case .cannotConnectToHost?: return "nothing is listening"
        case .timedOut?: return "no answer in time"
        case .networkConnectionLost?: return "the connection dropped"
        default: return error.localizedDescription
        }
    }
}
