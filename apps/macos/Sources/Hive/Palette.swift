import AppKit
import CoreText

/// The console's own colours (web/hub/src/app/globals.css, oklch converted to
/// sRGB), for what the app paints itself: the window before the page does,
/// the title bar above it, and the screens shown while the console is away.
enum Palette {
    static let paper = dynamic(light: 0xF3F8F6, dark: 0x09111B)
    static let sidebar = dynamic(light: 0xF7FBFA, dark: 0x0C141F)
    static let card = dynamic(light: 0xFFFFFF, dark: 0x101925)
    static let ink = dynamic(light: 0x192437, dark: 0xE7ECF0)
    static let muted = dynamic(light: 0x545F6C, dark: 0x9EA9B4)
    static let primary = dynamic(light: 0x007A71, dark: 0x5BCAB9)
    /// --bad: oklch(0.52 0.19 27) light, oklch(0.76 0.15 25) dark.
    static let bad = dynamic(light: 0xBE2323, dark: 0xFF8880)
    static let border = NSColor(name: nil) { appearance in
        appearance.isDark ? NSColor(white: 1, alpha: 0.09) : rgb(0xDCE3E4)
    }

    /// The brand tile behind the hive line (--gradient-brand).
    static let brandStops: [NSColor] = [rgb(0x50F4C5), rgb(0x11BFF0), rgb(0x877AF7)]

    static func dynamic(light: UInt32, dark: UInt32) -> NSColor {
        NSColor(name: nil) { appearance in appearance.isDark ? rgb(dark) : rgb(light) }
    }

    static func rgb(_ hex: UInt32, alpha: CGFloat = 1) -> NSColor {
        NSColor(
            srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255,
            alpha: alpha
        )
    }
}

extension NSAppearance {
    var isDark: Bool { bestMatch(from: [.aqua, .darkAqua]) == .darkAqua }
}

/// Atkinson Hyperlegible Next and Mono, the console's typefaces, from the
/// bundle (`scripts/build.sh` copies web/hub/public/fonts); the system font
/// when they are missing (`swift run`).
enum Fonts {
    static func register() {
        guard let folder = Bundle.main.resourceURL?.appendingPathComponent("Fonts", isDirectory: true),
              let files = try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil) else {
            return
        }
        for file in files where file.pathExtension == "woff2" || file.pathExtension == "ttf" || file.pathExtension == "otf" {
            CTFontManagerRegisterFontsForURL(file as CFURL, .process, nil)
        }
    }

    static func text(_ size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
        let name: String
        switch weight {
        case .semibold: name = "AtkinsonHyperlegibleNext-Regular_SemiBold"
        case .bold, .heavy, .black: name = "AtkinsonHyperlegibleNext-Regular_Bold"
        case .medium: name = "AtkinsonHyperlegibleNext-Regular_Medium"
        default: name = "AtkinsonHyperlegibleNext-Regular"
        }
        return NSFont(name: name, size: size) ?? .systemFont(ofSize: size, weight: weight)
    }

    static func mono(_ size: CGFloat) -> NSFont {
        NSFont(name: "AtkinsonHyperlegibleMono-ExtraLight_Regular", size: size)
            ?? .monospacedSystemFont(ofSize: size, weight: .regular)
    }
}
