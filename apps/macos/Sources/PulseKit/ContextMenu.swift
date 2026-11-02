import Foundation

/// The right-click menu over the console. WebKit proposes a browser's menu;
/// the app keeps what a text field or an image offers (copy, paste, look up,
/// share, spelling, speech, the inspector), drops what only a browser does
/// with a page or a link (open, open in a new window, download, back,
/// forward, reload), and names the one link command it answers itself the
/// way it answers a ⌘-click: a console page opens in a new tab, any other
/// link in the browser.
public enum ContextMenu {
    /// What the app does with one of WebKit's items.
    public enum Edit: Equatable, Sendable {
        case keep
        case drop
        /// Kept, under this title.
        case title(String)
    }

    /// WebKit names its items after WKMenuItemIdentifiersPrivate.h; the
    /// identifier is the name.
    public static let openLinkInNewWindow = "WKMenuItemIdentifierOpenLinkInNewWindow"
    public static let copyLink = "WKMenuItemIdentifierCopyLink"

    /// What a browser offers and an app has menus and link handling for.
    static let browserOnly: Set<String> = [
        "WKMenuItemIdentifierOpenLink",
        "WKMenuItemIdentifierDownloadLinkedFile",
        "WKMenuItemIdentifierOpenImageInNewWindow",
        "WKMenuItemIdentifierOpenFrameInNewWindow",
        "WKMenuItemIdentifierOpenMediaInNewWindow",
        "WKMenuItemIdentifierDownloadMedia",
        "WKMenuItemIdentifierCopyMediaLink",
        "WKMenuItemIdentifierGoBack",
        "WKMenuItemIdentifierGoForward",
        "WKMenuItemIdentifierReload",
        "WKMenuItemIdentifierStop",
    ]

    /// The edit for the item WebKit identifies as `identifier` (nil for items
    /// without one, which stay). `link` is the link under the pointer as the
    /// page reported it - nil when there is none, or no report came - and
    /// `policy` the window's link policy, nil while the console is unknown.
    public static func edit(_ identifier: String?, link: URL?, policy: LinkPolicy?) -> Edit {
        guard let identifier else { return .keep }
        switch identifier {
        case openLinkInNewWindow:
            guard let link, let policy, let title = openTitle(policy.target(for: link, newWindow: true)) else {
                return .drop
            }
            return .title(title)
        case copyLink:
            // A link the app refuses (javascript:, file:) has nothing worth copying.
            if let link, let policy, case .refuse = policy.target(for: link, newWindow: true) { return .drop }
            return .keep
        default:
            return browserOnly.contains(identifier) ? .drop : .keep
        }
    }

    static func openTitle(_ target: LinkTarget) -> String? {
        switch target {
        case .newTab: return "Open in New Tab"
        case .browser: return "Open in Browser"
        case .otherApp: return "Open Link"
        // A new-window request is never a click, so .beside does not come here.
        case .here, .redirect, .refuse, .beside: return nil
        }
    }

    /// Which items stay once the separators are tidy after the edits: none
    /// first, none last, none next to another. `isSeparator` describes the
    /// items in order.
    public static func keepsSeparators(_ isSeparator: [Bool]) -> [Bool] {
        var keeps = [Bool](repeating: true, count: isSeparator.count)
        // Whether the last item kept so far is a separator; nil before the first.
        var afterSeparator: Bool?
        for (index, separator) in isSeparator.enumerated() {
            if separator {
                if afterSeparator == false { afterSeparator = true } else { keeps[index] = false }
            } else {
                afterSeparator = false
            }
        }
        if afterSeparator == true, let last = keeps.lastIndex(of: true) { keeps[last] = false }
        return keeps
    }
}
