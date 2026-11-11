import Foundation

/// What the console page asks of the app's notifications: the page's
/// notifier (web/hub/src/components/shell/owner-notifier.tsx) posts these to
/// the `hiveNotify` handler. The page decides what is news and when; the app
/// only shows it the Mac's way and says what macOS allows.
public enum NotifyRequest: Equatable, Sendable {
    /// Whether Hive may notify: `ask` asks macOS when nobody asked yet (the
    /// owner just turned notifications on). The answer is the page's word for
    /// it: granted, denied, default (not asked yet) or unsupported.
    case permission(ask: Bool)
    /// One notification to show.
    case notify(PageNotification)

    public static let handlerName = "hiveNotify"

    public init?(_ body: Any) {
        guard let message = body as? [String: Any], let kind = message["kind"] as? String else { return nil }
        switch kind {
        case "permission":
            self = .permission(ask: message["ask"] as? Bool ?? false)
        case "notify":
            guard let note = PageNotification(message) else { return nil }
            self = .notify(note)
        default:
            return nil
        }
    }
}

public struct PageNotification: Equatable, Sendable {
    /// The page's id for it: a second one with the same tag replaces the first.
    public var tag: String
    public var title: String
    public var body: String
    /// The console page a click opens, always a path on the console itself.
    public var path: String
    public var sound: Bool
    /// The page was in front and showed its own note when it asked.
    public var focused: Bool

    static let maxTitle = 120
    static let maxBody = 400

    public init(tag: String, title: String, body: String, path: String, sound: Bool, focused: Bool) {
        self.tag = tag
        self.title = title
        self.body = body
        self.path = path
        self.sound = sound
        self.focused = focused
    }

    init?(_ message: [String: Any]) {
        let title = Self.clip(message["title"] as? String ?? "", Self.maxTitle)
        guard !title.isEmpty else { return nil }
        let tag = Self.clip(message["tag"] as? String ?? "", 200)
        self.init(
            tag: tag.isEmpty ? UUID().uuidString : tag,
            title: title,
            body: Self.clip(message["body"] as? String ?? "", Self.maxBody),
            path: Self.consolePath(message["path"] as? String ?? "/"),
            sound: message["sound"] as? Bool ?? true,
            focused: message["focused"] as? Bool ?? false
        )
    }

    /// Help → Send Test Notification: one the page could have sent, read from
    /// the same message the console posts to `hiveNotify`, with the sound and
    /// the banner even while Hive is in front, so one click shows the owner
    /// what macOS does with a real one. A click opens Settings → Notifications.
    public static var test: PageNotification {
        PageNotification([
            "kind": "notify", "tag": "test", "title": "Hive notifications work",
            "body": "This is how Hive tells you when something waits for you or work finishes.",
            "path": "/settings/#notify", "sound": true, "focused": false,
        ])!
    }

    /// The notification's identifier in Notification Center.
    public var identifier: String { "hive.notify." + tag }

    /// What macOS does with it while Hive is the frontmost app: when the page
    /// already showed its note, only the sound; otherwise (another Hive
    /// window is in front) the banner too. In the background macOS shows it
    /// as the owner's settings for Hive say.
    public var inFront: (banner: Bool, sound: Bool) {
        (banner: !focused, sound: sound)
    }

    /// A path on the console, never a URL elsewhere: anything that is not a
    /// plain absolute path (a scheme, `//host`, a backslash) opens the home page.
    public static func consolePath(_ raw: String) -> String {
        let path = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard path.hasPrefix("/"), !path.hasPrefix("//"), !path.contains("\\"),
              !path.contains(where: { $0.isNewline || $0 == "\0" }),
              let url = URL(string: path), url.scheme == nil, url.host == nil else {
            return "/"
        }
        return path
    }

    /// One line, at most `limit` characters, ending in … when cut.
    static func clip(_ text: String, _ limit: Int) -> String {
        let line = text.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        guard line.count > limit else { return line }
        return String(line.prefix(limit - 1)).trimmingCharacters(in: .whitespaces) + "…"
    }
}
