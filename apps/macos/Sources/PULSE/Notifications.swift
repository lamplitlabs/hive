import AppKit
import PulseKit
import UserNotifications
import WebKit
import os

/// The console's notifications as Mac notifications. The page decides what
/// is news (web/hub/src/lib/owner-notify.ts) and posts it to the
/// `pulseNotify` handler; this shows it through Notification Center, with
/// the Mac's sound, and opens the page it names when the owner clicks it.
@MainActor
final class Notifications: NSObject, UNUserNotificationCenterDelegate {
    /// Opens a console path in a window, bringing PULSE to the front.
    var open: ((String) -> Void)?
    private let log = Logger(subsystem: AppIdentity.bundleID, category: "notify")

    /// Notification Center only serves a bundled app: `swift run` outside
    /// PULSE.app has no bundle identifier, and asking for the centre there throws.
    let available: Bool = Bundle.main.bundleIdentifier != nil && Bundle.main.bundleURL.pathExtension == "app"

    private var center: UNUserNotificationCenter? { available ? .current() : nil }

    /// Early, so a click on a notification that launches PULSE reaches us.
    func start() {
        center?.delegate = self
    }

    /// What macOS allows, as the page's notifier words it: granted, denied,
    /// default (not asked yet) or unsupported. `ask` asks when nobody has.
    func permission(ask: Bool) async -> String {
        guard let center else { return "unsupported" }
        let status = await center.notificationSettings().authorizationStatus
        switch status {
        case .authorized, .provisional:
            return "granted"
        case .denied:
            return "denied"
        case .notDetermined:
            guard ask else { return "default" }
            do {
                let granted = try await center.requestAuthorization(options: [.alert, .sound])
                log.notice("notifications \(granted ? "allowed" : "refused", privacy: .public) by the owner")
                return granted ? "granted" : "denied"
            } catch {
                log.error("could not ask for notifications: \(error.localizedDescription, privacy: .public)")
                return "default"
            }
        @unknown default:
            return "default"
        }
    }

    func post(_ note: PageNotification) {
        guard let center else { return }
        let content = UNMutableNotificationContent()
        content.title = note.title
        content.body = note.body
        content.sound = note.sound ? .default : nil
        content.threadIdentifier = "pulse.console"
        content.userInfo = ["path": note.path, "banner": note.inFront.banner]
        let request = UNNotificationRequest(identifier: note.identifier, content: content, trigger: nil)
        center.add(request) { [log] error in
            if let error { log.error("could not post a notification: \(error.localizedDescription, privacy: .public)") }
        }
    }

    // MARK: - UNUserNotificationCenterDelegate

    /// PULSE in front: the page already showed its note when its window has
    /// focus, so only the sound; otherwise the banner too.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping @Sendable (UNNotificationPresentationOptions) -> Void
    ) {
        let content = notification.request.content
        var options: UNNotificationPresentationOptions = content.sound == nil ? [] : [.sound]
        if content.userInfo["banner"] as? Bool ?? true { options.formUnion([.banner, .list]) }
        completionHandler(options)
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping @Sendable () -> Void
    ) {
        let path = PageNotification.consolePath(response.notification.request.content.userInfo["path"] as? String ?? "/")
        let opens = response.actionIdentifier == UNNotificationDefaultActionIdentifier
        Task { @MainActor in
            if opens { self.open?(path) }
            completionHandler()
        }
    }
}

/// The page's notifier talks to the app through this handler, in the page's
/// own world (unlike PageBridge's): the console's scripts post to it and read
/// the answer. WKUserContentController keeps it strongly; it keeps the window weakly.
final class WeakReplyHandler: NSObject, WKScriptMessageHandlerWithReply {
    private weak var target: (any WKScriptMessageHandlerWithReply)?

    init(_ target: any WKScriptMessageHandlerWithReply) {
        self.target = target
    }

    func userContentController(
        _ controller: WKUserContentController,
        didReceive message: WKScriptMessage,
        replyHandler: @escaping @MainActor @Sendable (Any?, String?) -> Void
    ) {
        guard let target else {
            replyHandler(nil, "the window is gone")
            return
        }
        target.userContentController(controller, didReceive: message, replyHandler: replyHandler)
    }
}
