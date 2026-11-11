import AppKit
import HiveKit
import WebKit
import os

/// Saves what the console hands over as a download (chat artifacts, a run's
/// transcript) into ~/Downloads under a name that is not taken yet.
@MainActor
final class Downloads: NSObject, WKDownloadDelegate {
    private struct Pending {
        var destination: URL
        weak var window: ConsoleWindowController?
    }

    /// Downloads in flight; their destinations are taken though not yet on disk.
    private var pending: [ObjectIdentifier: Pending] = [:]
    private let log = Logger(subsystem: AppIdentity.bundleID, category: "downloads")

    func track(_ download: WKDownload, from window: ConsoleWindowController) {
        download.delegate = self
        pending[ObjectIdentifier(download)] = Pending(destination: URL(fileURLWithPath: "/dev/null"), window: window)
    }

    func download(
        _ download: WKDownload,
        decideDestinationUsing response: URLResponse,
        suggestedFilename: String,
        completionHandler: @escaping @MainActor @Sendable (URL?) -> Void
    ) {
        let folder = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Downloads", isDirectory: true)
        let reserved = Set(pending.values.map(\.destination.path))
        let destination = Self.unique(in: folder, name: Self.safeName(suggestedFilename), taken: reserved)
        pending[ObjectIdentifier(download), default: Pending(destination: destination, window: nil)].destination = destination
        completionHandler(destination)
    }

    func downloadDidFinish(_ download: WKDownload) {
        guard let done = pending.removeValue(forKey: ObjectIdentifier(download)) else { return }
        let file = done.destination
        log.info("downloaded \(file.path, privacy: .public)")
        // Bounces the Downloads stack in the Dock, as Safari's downloads do.
        DistributedNotificationCenter.default().post(name: .init("com.apple.DownloadFileFinished"), object: file.path)
        done.window?.showToast("Downloaded \(file.lastPathComponent)", button: "Show in Finder") {
            NSWorkspace.shared.activateFileViewerSelecting([file])
        }
    }

    func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) {
        let failed = pending.removeValue(forKey: ObjectIdentifier(download))
        log.error("download failed: \(error.localizedDescription, privacy: .public)")
        failed?.window?.showToast("The download failed: \(error.localizedDescription)", button: nil, action: nil)
    }

    /// No path separators, no hidden files, nothing empty.
    static func safeName(_ suggested: String) -> String {
        var name = suggested.replacingOccurrences(of: "/", with: "-").replacingOccurrences(of: ":", with: "-")
        name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        while name.hasPrefix(".") { name.removeFirst() }
        return name.isEmpty ? "download" : name
    }

    /// `name.ext`, else `name 2.ext`, `name 3.ext`… (WebKit refuses an
    /// existing file, and two downloads of one name may run at once).
    static func unique(in folder: URL, name: String, taken: Set<String> = []) -> URL {
        let manager = FileManager.default
        func free(_ url: URL) -> Bool { !manager.fileExists(atPath: url.path) && !taken.contains(url.path) }
        var candidate = folder.appendingPathComponent(name)
        guard !free(candidate) else { return candidate }
        let base = (name as NSString).deletingPathExtension
        let ext = (name as NSString).pathExtension
        for n in 2...9999 {
            let next = ext.isEmpty ? "\(base) \(n)" : "\(base) \(n).\(ext)"
            candidate = folder.appendingPathComponent(next)
            if free(candidate) { return candidate }
        }
        return folder.appendingPathComponent("\(UUID().uuidString)-\(name)")
    }
}
