import AppKit
import WebKit

/// The console's web view: the right-click menu passes through the window
/// before it opens, so an app's menu shows where WebKit proposes a browser's
/// (HiveKit's ContextMenu decides).
final class ConsoleWebView: WKWebView {
    /// Receives the menu WebKit proposes, to edit in place.
    var willOpenContextMenu: ((NSMenu) -> Void)?

    override func willOpenMenu(_ menu: NSMenu, with event: NSEvent) {
        super.willOpenMenu(menu, with: event)
        willOpenContextMenu?(menu)
    }
}
