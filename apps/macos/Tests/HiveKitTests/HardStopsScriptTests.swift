import Foundation
import JavaScriptCore
import Testing
@testable import HiveKit

/// Runs the menu's script against a stand-in page: the menu item only does
/// nothing when the script claims it scrolled to a strip that is not there.
@Suite struct HardStopsScriptTests {
    /// The script's answer, and whether it scrolled and rewrote the address,
    /// on a page at `pathname` that renders the strip or not.
    static func run(pathname: String, strip: Bool) -> (answer: Bool, scrolled: Bool, hash: Bool) {
        let js = JSContext()!
        js.evaluateScript("""
        var scrolled = false, hashed = false;
        var location = { pathname: '\(pathname)' };
        var history = { state: null, replaceState: function (s, t, u) { hashed = u === '#hard-stops'; } };
        var document = { getElementById: function (id) {
          return \(strip) && id === 'hard-stops' ? { scrollIntoView: function () { scrolled = true; } } : null;
        } };
        """)
        let answer = js.evaluateScript(HardStopsScript.source)!.toBool()
        return (answer, js.evaluateScript("scrolled")!.toBool(), js.evaluateScript("hashed")!.toBool())
    }

    @Test func scrollsToTheStripWhenItIsOnScreen() {
        let r = Self.run(pathname: "/chat/", strip: true)
        #expect(r.answer && r.scrolled && r.hash)
    }

    @Test func answersFalseOnSessionsWithoutTheStrip() {
        // A conversation is open (or nothing waits): Sessions renders no
        // strip, so the app must load /chat/#hard-stops itself.
        let r = Self.run(pathname: "/chat/", strip: false)
        #expect(!r.answer && !r.scrolled && !r.hash)
    }

    @Test func answersFalseOnAnotherPage() {
        let r = Self.run(pathname: "/tasks/", strip: true)
        #expect(!r.answer && !r.scrolled)
        #expect(HardStopsScript.path == "/chat/#hard-stops")
    }
}
