import Foundation

public enum AppIdentity {
    /// The bundle identifier `scripts/build.sh` gives Hive.app; also the
    /// designated requirement it signs with, so macOS keeps its answers
    /// (notifications, folder access) across rebuilds.
    public static let defaultBundleID = "io.github.manishtiwari25.hive"

    public static var bundleID: String { Bundle.main.bundleIdentifier ?? defaultBundleID }

    public static var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
    }

    public static var build: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "dev"
    }
}

/// The PATH a login shell sets up, for a hive the app starts while no
/// service unit records one (`zsh -i -l -c`, like a new terminal window).
public enum LoginShell {
    static let marker = "__HIVE_PATH__="

    public static func path(shell: String?, timeout: TimeInterval = 6) async -> String? {
        let program = shell.flatMap { $0.isEmpty ? nil : $0 } ?? "/bin/zsh"
        let script = "printf '\\n\(marker)%s\\n' \"$PATH\""
        let result = await Command.run(program, ["-i", "-l", "-c", script], timeout: timeout)
        return parse(result.stdout)
    }

    /// The marker line's value; an interactive shell may print other things first.
    static func parse(_ output: String) -> String? {
        let line = output.split(whereSeparator: \.isNewline).last(where: { $0.hasPrefix(marker) })
        guard let value = line?.dropFirst(marker.count), !value.isEmpty else { return nil }
        return String(value).trimmingCharacters(in: .whitespaces)
    }
}

public enum LogTail {
    /// The last `lines` lines of a log, for the screen that says why the
    /// console did not start.
    public static func read(_ url: URL, lines: Int = 12, maxBytes: Int = 64 * 1024) -> String? {
        guard let handle = try? FileHandle(forReadingFrom: url) else { return nil }
        defer { try? handle.close() }
        guard let size = try? handle.seekToEnd(), size > 0 else { return nil }
        let start = size > UInt64(maxBytes) ? size - UInt64(maxBytes) : 0
        try? handle.seek(toOffset: start)
        guard let data = try? handle.readToEnd() else { return nil }
        let text = String(decoding: data, as: UTF8.self)
        let tail = text.split(whereSeparator: \.isNewline).suffix(lines).joined(separator: "\n")
        return tail.isEmpty ? nil : tail
    }
}
