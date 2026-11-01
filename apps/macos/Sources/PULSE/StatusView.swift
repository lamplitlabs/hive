import AppKit

/// What the window shows while the console itself cannot be shown.
struct StatusContent: Equatable {
    struct Action: Equatable {
        var title: String
        var id: String
        var primary = false
    }

    var title: String
    var detail: String
    var busy = false
    var actions: [Action] = []
    /// The end of a log, shown in the console's mono face.
    var logTail: String?
}

/// The console's brand mark (web/hub/src/components/shell/pulse-mark.tsx):
/// the pulse line on the mint-sky-violet tile.
final class PulseMarkView: NSView {
    override var intrinsicContentSize: NSSize { NSSize(width: 56, height: 56) }
    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        let side = min(bounds.width, bounds.height)
        let tile = CGRect(x: (bounds.width - side) / 2, y: (bounds.height - side) / 2, width: side, height: side)
        let radius = side * 9 / 32
        let shape = CGPath(roundedRect: tile, cornerWidth: radius, cornerHeight: radius, transform: nil)

        context.saveGState()
        context.addPath(shape)
        context.clip()
        let colors = Palette.brandStops.map(\.cgColor) as CFArray
        if let gradient = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: colors, locations: [0, 0.52, 1]) {
            // 135deg: top left to bottom right.
            context.drawLinearGradient(
                gradient, start: CGPoint(x: tile.minX, y: tile.minY), end: CGPoint(x: tile.maxX, y: tile.maxY), options: []
            )
        }
        context.restoreGState()

        context.addPath(shape)
        context.setStrokeColor(NSColor(white: 1, alpha: 0.25).cgColor)
        context.setLineWidth(1)
        context.strokePath()

        // The waveform at 78% of the tile, from a 256-unit box.
        let scale = side * 0.78 / 256
        let origin = CGPoint(x: tile.midX - 128 * scale, y: tile.midY - 128 * scale)
        let points: [(CGFloat, CGFloat)] = [(30, 135), (71, 135), (90, 88), (124, 189), (155, 112), (173, 135), (226, 135)]
        let line = CGMutablePath()
        for (index, point) in points.enumerated() {
            let p = CGPoint(x: origin.x + point.0 * scale, y: origin.y + point.1 * scale)
            if index == 0 { line.move(to: p) } else { line.addLine(to: p) }
        }
        context.addPath(line)
        context.setStrokeColor(NSColor.white.cgColor)
        context.setLineWidth(22 * scale)
        context.setLineCap(.round)
        context.setLineJoin(.round)
        context.strokePath()
    }
}

/// The screen over the web view while the console is starting, coming back,
/// or needs the owner: centred, in the console's colours and type.
final class StatusView: NSView {
    var onAction: ((String) -> Void)?

    private let mark = PulseMarkView()
    private let spinner = NSProgressIndicator()
    private let titleLabel = NSTextField(wrappingLabelWithString: "")
    private let detailLabel = NSTextField(wrappingLabelWithString: "")
    private let buttons = NSStackView()
    private let logScroll = NSScrollView()
    private let logText = NSTextView()
    private var content: StatusContent?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        setAccessibilityRole(.group)

        titleLabel.font = Fonts.text(22, weight: .semibold)
        titleLabel.textColor = Palette.ink
        titleLabel.alignment = .center
        titleLabel.setAccessibilityRole(.staticText)

        detailLabel.font = Fonts.text(14)
        detailLabel.textColor = Palette.muted
        detailLabel.alignment = .center
        detailLabel.isSelectable = true

        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.isDisplayedWhenStopped = false

        buttons.orientation = .horizontal
        buttons.spacing = 10
        buttons.alignment = .centerY

        logText.isEditable = false
        logText.isSelectable = true
        logText.font = Fonts.mono(11)
        logText.textColor = Palette.muted
        logText.backgroundColor = Palette.card
        logText.textContainerInset = NSSize(width: 10, height: 8)
        logText.isVerticallyResizable = true
        logText.autoresizingMask = [.width]
        logText.textContainer?.widthTracksTextView = true
        logScroll.documentView = logText
        logScroll.hasVerticalScroller = true
        logScroll.borderType = .noBorder
        logScroll.wantsLayer = true
        logScroll.layer?.cornerRadius = 10
        logScroll.layer?.borderWidth = 1
        logScroll.drawsBackground = true
        logScroll.backgroundColor = Palette.card

        let column = NSStackView(views: [mark, spinner, titleLabel, detailLabel, buttons, logScroll])
        column.orientation = .vertical
        column.alignment = .centerX
        column.spacing = 14
        column.setCustomSpacing(22, after: mark)
        column.setCustomSpacing(6, after: titleLabel)
        column.setCustomSpacing(22, after: detailLabel)
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)

        NSLayoutConstraint.activate([
            column.centerXAnchor.constraint(equalTo: centerXAnchor),
            column.centerYAnchor.constraint(equalTo: centerYAnchor, constant: -24),
            column.widthAnchor.constraint(equalToConstant: 460),
            column.leadingAnchor.constraint(greaterThanOrEqualTo: leadingAnchor, constant: 24),
            column.topAnchor.constraint(greaterThanOrEqualTo: topAnchor, constant: 24),
            mark.widthAnchor.constraint(equalToConstant: 56),
            mark.heightAnchor.constraint(equalToConstant: 56),
            titleLabel.widthAnchor.constraint(equalTo: column.widthAnchor),
            detailLabel.widthAnchor.constraint(equalTo: column.widthAnchor),
            logScroll.widthAnchor.constraint(equalTo: column.widthAnchor),
            logScroll.heightAnchor.constraint(equalToConstant: 150),
        ])
        column.setHuggingPriority(.required, for: .vertical)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }

    override func updateLayer() {
        layer?.backgroundColor = Palette.paper.cgColor
        logScroll.layer?.borderColor = Palette.border.cgColor
    }

    override var wantsUpdateLayer: Bool { true }

    // While shown, the web view underneath gets no clicks, scrolls or drops;
    // hidden or fading out, every event goes through to the console. (`point`
    // is in the superview's coordinates.)
    override func hitTest(_ point: NSPoint) -> NSView? {
        guard !isHidden, alphaValue > 0, frame.contains(point) else { return nil }
        return super.hitTest(point) ?? self
    }

    func show(_ next: StatusContent) {
        guard next != content else { return }
        content = next
        titleLabel.stringValue = next.title
        detailLabel.attributedStringValue = Self.detail(next.detail)
        detailLabel.alignment = .center
        if next.busy { spinner.startAnimation(nil) } else { spinner.stopAnimation(nil) }

        buttons.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for action in next.actions {
            let button = NSButton(title: action.title, target: self, action: #selector(clicked(_:)))
            button.identifier = NSUserInterfaceItemIdentifier(action.id)
            button.bezelStyle = .rounded
            button.controlSize = .large
            button.font = Fonts.text(13, weight: action.primary ? .semibold : .regular)
            if action.primary {
                button.keyEquivalent = "\r"
                button.bezelColor = Palette.primary
            }
            buttons.addArrangedSubview(button)
        }
        buttons.isHidden = next.actions.isEmpty

        if let tail = next.logTail, !tail.isEmpty {
            logText.string = tail
            logText.scrollToEndOfDocument(nil)
            logScroll.isHidden = false
        } else {
            logScroll.isHidden = true
        }
        setAccessibilityLabel(next.title)
        NSAccessibility.post(element: self, notification: .layoutChanged)
    }

    @objc private func clicked(_ sender: NSButton) {
        guard let id = sender.identifier?.rawValue else { return }
        onAction?(id)
    }

    /// `code` spans in the detail become the console's mono face.
    static func detail(_ text: String) -> NSAttributedString {
        let result = NSMutableAttributedString()
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        paragraph.lineSpacing = 3
        let plain: [NSAttributedString.Key: Any] = [.font: Fonts.text(14), .foregroundColor: Palette.muted, .paragraphStyle: paragraph]
        let code: [NSAttributedString.Key: Any] = [.font: Fonts.mono(13), .foregroundColor: Palette.ink, .paragraphStyle: paragraph]
        for (index, part) in text.components(separatedBy: "`").enumerated() {
            result.append(NSAttributedString(string: part, attributes: index % 2 == 1 ? code : plain))
        }
        return result
    }
}
