import Foundation

/// What the browser panel's address field makes of what was typed: a web
/// address or nothing. There is no search engine behind it, so words are
/// not an address. A host is: with `http://` when it is a machine's own
/// (localhost, a loopback address, a `.local` name) or names a port - a dev
/// server at `127.0.0.1:3000`, say - and `https://` for the rest of the web.
public enum TypedAddress {
    public static func url(_ text: String) -> URL? {
        let typed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !typed.isEmpty, !typed.contains(" ") else { return nil }
        // A scheme in front (http:, mailto:, javascript:) - unless what
        // follows the colon is a port, which makes it a host (my-app:8080).
        if let range = typed.range(of: "^[A-Za-z][A-Za-z0-9+.-]*:", options: .regularExpression),
           typed.range(of: "^[A-Za-z][A-Za-z0-9+.-]*:[0-9]+([/?#].*)?$", options: .regularExpression) == nil {
            let scheme = typed[range].dropLast().lowercased()
            guard scheme == "http" || scheme == "https", let url = URL(string: typed), url.host != nil else { return nil }
            return url
        }
        let authority = String(typed.prefix { $0 != "/" && $0 != "?" && $0 != "#" })
        let host: String
        let port: Substring?
        if authority.hasPrefix("["), let close = authority.firstIndex(of: "]") {
            host = String(authority[...close]).lowercased()
            port = authority[authority.index(after: close)...].dropFirst()
        } else {
            let parts = authority.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
            host = parts.first.map { String($0).lowercased() } ?? ""
            port = parts.count > 1 ? parts[1] : nil
        }
        guard !host.isEmpty, !host.hasPrefix("."), !host.hasSuffix(".") else { return nil }
        if let port, port.isEmpty || !port.allSatisfy(\.isNumber) { return nil }
        let local = host == "localhost" || host == "127.0.0.1" || host == "0.0.0.0" || host == "[::1]"
            || host.hasSuffix(".local") || host.hasSuffix(".localhost")
        // A bare word is a host only with a dot or a port behind it.
        guard local || host.contains(".") || port != nil else { return nil }
        let scheme = local || (port != nil && port != "443") ? "http://" : "https://"
        guard let url = URL(string: scheme + typed), url.host != nil else { return nil }
        return url
    }
}
