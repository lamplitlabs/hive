import Foundation
import Testing
@testable import PulseKit

@Suite struct NotifyRequestTests {
    @Test func readsAPermissionQuestion() {
        #expect(NotifyRequest(["kind": "permission", "ask": true]) == .permission(ask: true))
        #expect(NotifyRequest(["kind": "permission"]) == .permission(ask: false))
    }

    @Test func readsANotification() {
        let body: [String: Any] = [
            "kind": "notify", "tag": "question:q1", "title": "Question from granite",
            "body": "Which port?\n(demo)", "path": "/project/?slug=demo&tab=for-you", "sound": false, "focused": true,
        ]
        let expected = PageNotification(
            tag: "question:q1", title: "Question from granite", body: "Which port? (demo)",
            path: "/project/?slug=demo&tab=for-you", sound: false, focused: true
        )
        #expect(NotifyRequest(body) == .notify(expected))
        #expect(expected.identifier == "pulse.notify.question:q1")
    }

    @Test func refusesWhatIsNotTheConsolesMessage() {
        #expect(NotifyRequest("notify") == nil)
        #expect(NotifyRequest(["kind": "chrome"]) == nil)
        #expect(NotifyRequest(["title": "no kind"]) == nil)
        #expect(NotifyRequest(["kind": "notify", "title": "   "]) == nil, "a notification needs a title")
    }

    @Test func defaultsTheSoundOnAndGivesAnUntaggedOneAnId() {
        guard case .notify(let note)? = NotifyRequest(["kind": "notify", "title": "Reply ready"]) else {
            Issue.record("not a notification")
            return
        }
        #expect(note.sound)
        #expect(!note.focused)
        #expect(note.path == "/")
        #expect(!note.tag.isEmpty)
    }

    @Test func clipsLongWords() {
        guard case .notify(let note)? = NotifyRequest(["kind": "notify", "title": String(repeating: "t", count: 300), "body": String(repeating: "b ", count: 400)]) else {
            Issue.record("not a notification")
            return
        }
        #expect(note.title.count == 120)
        #expect(note.title.hasSuffix("…"))
        #expect(note.body.count <= 400)
        #expect(note.body.hasSuffix("…"))
    }

    @Test func aClickOnlyOpensConsolePaths() {
        #expect(PageNotification.consolePath("/chat/?id=chat-1") == "/chat/?id=chat-1")
        #expect(PageNotification.consolePath("/#for-you") == "/#for-you")
        #expect(PageNotification.consolePath("https://example.com/") == "/")
        #expect(PageNotification.consolePath("//example.com/x") == "/")
        #expect(PageNotification.consolePath("/\\example.com") == "/")
        #expect(PageNotification.consolePath("javascript:alert(1)") == "/")
        #expect(PageNotification.consolePath("chat/") == "/")
        #expect(PageNotification.consolePath("/a\nb") == "/")
        #expect(PageNotification.consolePath("") == "/")
    }

    @Test func theTestNotificationIsOneThePageCouldSendWithBannerAndSound() {
        let note = PageNotification.test
        #expect(note.inFront == (banner: true, sound: true), "the test shows its banner and plays the sound even in front")
        #expect(note.path == "/settings/#notify", "a click opens Settings → Notifications")
        #expect(note.identifier == "pulse.notify.test", "a second test replaces the first")
        #expect(!note.title.isEmpty && !note.body.isEmpty)
    }

    @Test func inFrontOnlyTheSoundWhenThePageShowedItsNote() {
        let note = PageNotification(tag: "t", title: "T", body: "", path: "/", sound: true, focused: true)
        #expect(note.inFront == (banner: false, sound: true))
        var behind = note
        behind.focused = false
        behind.sound = false
        #expect(behind.inFront == (banner: true, sound: false))
    }
}
