import Foundation
import Testing
@testable import HiveKit

@Suite struct ContextMenuTests {
    static let policy = LinkPolicy(console: URL(string: "http://127.0.0.1:8765/")!)
    static let consoleLink = URL(string: "http://127.0.0.1:8765/tasks/abc")!
    static let webLink = URL(string: "https://github.com/lamplitlabs/hive/pull/37")!

    @Test func keepsTheTextCommandsAndDropsTheBrowsers() {
        for kept in ["WKMenuItemIdentifierCopy", "WKMenuItemIdentifierPaste", "WKMenuItemIdentifierLookUp",
                     "WKMenuItemIdentifierShareMenu", "WKMenuItemIdentifierCopyImage", "WKMenuItemIdentifierInspectElement"] {
            #expect(ContextMenu.edit(kept, link: nil, policy: Self.policy) == .keep)
        }
        for dropped in ["WKMenuItemIdentifierOpenLink", "WKMenuItemIdentifierDownloadLinkedFile", "WKMenuItemIdentifierReload",
                        "WKMenuItemIdentifierGoBack", "WKMenuItemIdentifierGoForward", "WKMenuItemIdentifierOpenImageInNewWindow"] {
            #expect(ContextMenu.edit(dropped, link: Self.consoleLink, policy: Self.policy) == .drop)
        }
        // Items WebKit does not identify (Services, spelling) are left alone.
        #expect(ContextMenu.edit(nil, link: nil, policy: Self.policy) == .keep)
    }

    @Test func namesTheOpenItemAfterWhereTheLinkGoes() {
        let open = ContextMenu.openLinkInNewWindow
        #expect(ContextMenu.edit(open, link: Self.consoleLink, policy: Self.policy) == .title("Open in New Tab"))
        #expect(ContextMenu.edit(open, link: URL(string: "http://localhost:8765/chat/")!, policy: Self.policy) == .title("Open in New Tab"))
        #expect(ContextMenu.edit(open, link: Self.webLink, policy: Self.policy) == .title("Open in Browser"))
        #expect(ContextMenu.edit(open, link: URL(string: "mailto:me@example.com")!, policy: Self.policy) == .title("Open Link"))
    }

    @Test func dropsTheOpenItemWithoutALinkItCanName() {
        let open = ContextMenu.openLinkInNewWindow
        // No report from the page (a subframe, or the report did not arrive).
        #expect(ContextMenu.edit(open, link: nil, policy: Self.policy) == .drop)
        // The console is not known yet.
        #expect(ContextMenu.edit(open, link: Self.consoleLink, policy: nil) == .drop)
        // A link the app never follows.
        #expect(ContextMenu.edit(open, link: URL(string: "javascript:void(0)")!, policy: Self.policy) == .drop)
    }

    @Test func copyLinkStaysExceptForARefusedLink() {
        let copy = ContextMenu.copyLink
        #expect(ContextMenu.edit(copy, link: Self.consoleLink, policy: Self.policy) == .keep)
        #expect(ContextMenu.edit(copy, link: Self.webLink, policy: Self.policy) == .keep)
        #expect(ContextMenu.edit(copy, link: nil, policy: Self.policy) == .keep)
        #expect(ContextMenu.edit(copy, link: URL(string: "javascript:void(0)")!, policy: Self.policy) == .drop)
    }

    @Test func tidiesSeparatorsLeftByTheEdits() {
        // separator, item, separator, separator, item, separator
        #expect(ContextMenu.keepsSeparators([true, false, true, true, false, true]) == [false, true, true, false, true, false])
        #expect(ContextMenu.keepsSeparators([true, true]) == [false, false])
        #expect(ContextMenu.keepsSeparators([false, true, false]) == [true, true, true])
        #expect(ContextMenu.keepsSeparators([]) == [])
    }
}
