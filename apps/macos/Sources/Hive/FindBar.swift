import AppKit
import WebKit

/// Find in the page (⌘F, ⌘G, ⇧⌘G): a small card over the top right of the
/// console. WKWebView has no system find bar on macOS, so this one drives
/// WKWebView.find(_:configuration:).
final class FindBar: NSView, NSSearchFieldDelegate {
    let field = NSSearchField()
    /// (text, backwards, again): `again` is false while typing, so the
    /// search starts at the current match instead of moving past it.
    var onFind: ((String, Bool, Bool) -> Void)?
    var onClose: (() -> Void)?

    private let result = NSTextField(labelWithString: "")
    private let previous = NSButton()
    private let next = NSButton()
    private let done = NSButton(title: "Done", target: nil, action: nil)

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.cornerRadius = 12
        layer?.borderWidth = 1
        shadow = NSShadow()
        layer?.shadowOpacity = 0.16
        layer?.shadowRadius = 14
        layer?.shadowOffset = CGSize(width: 0, height: -4)
        isHidden = true
        setAccessibilityRole(.group)
        setAccessibilityLabel("Find in page")

        field.placeholderString = "Find in page"
        field.font = Fonts.text(13)
        field.delegate = self
        // Typing searches through controlTextDidChange and Return through
        // doCommandBy; the field sends no action of its own, which would
        // search the same text a second time and skip a match.
        field.sendsWholeSearchString = true

        result.font = Fonts.text(12)
        result.textColor = Palette.muted

        for (button, symbol, label, selector) in [
            (previous, "chevron.up", "Previous match", #selector(findPrevious)),
            (next, "chevron.down", "Next match", #selector(findNext)),
        ] {
            button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)
            button.bezelStyle = .texturedRounded
            button.isBordered = false
            button.target = self
            button.action = selector
            button.toolTip = label
        }
        done.bezelStyle = .rounded
        done.controlSize = .small
        done.font = Fonts.text(12)
        done.target = self
        done.action = #selector(close)

        let row = NSStackView(views: [field, result, previous, next, done])
        row.orientation = .horizontal
        row.spacing = 6
        row.edgeInsets = NSEdgeInsets(top: 8, left: 10, bottom: 8, right: 10)
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor),
            row.trailingAnchor.constraint(equalTo: trailingAnchor),
            row.topAnchor.constraint(equalTo: topAnchor),
            row.bottomAnchor.constraint(equalTo: bottomAnchor),
            field.widthAnchor.constraint(greaterThanOrEqualToConstant: 200),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    override var wantsUpdateLayer: Bool { true }

    override func updateLayer() {
        layer?.backgroundColor = Palette.card.cgColor
        layer?.borderColor = Palette.border.cgColor
    }

    func open(with text: String? = nil) {
        if let text { field.stringValue = text }
        isHidden = false
        window?.makeFirstResponder(field)
        field.currentEditor()?.selectAll(nil)
    }

    func show(found: Bool) {
        result.stringValue = found || field.stringValue.isEmpty ? "" : "Not found"
    }

    @objc func findNext() { run(backwards: false) }
    @objc func findPrevious() { run(backwards: true) }

    @objc func close() {
        isHidden = true
        result.stringValue = ""
        onClose?()
    }

    private func run(backwards: Bool, again: Bool = true) {
        let text = field.stringValue
        guard !text.isEmpty else {
            result.stringValue = ""
            return
        }
        onFind?(text, backwards, again)
    }

    func controlTextDidChange(_ obj: Notification) {
        run(backwards: false, again: false)
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.insertNewline(_:)):
            let shift = NSApp.currentEvent?.modifierFlags.contains(.shift) ?? false
            run(backwards: shift)
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            close()
            return true
        default:
            return false
        }
    }
}
