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
    /// The join screen's fields (ADR-070 step 2): the brain and the code.
    var form: JoinFields?
}

/// The join screen's two fields and the line under them.
struct JoinFields: Equatable {
    var brain: String
    var code: String
    /// What happened or what is missing; red when `bad`.
    var message: String?
    var bad = false
    /// False while the join runs.
    var editable = true
    /// The first-launch screen of a Mac with no PULSE of its own: no Cancel,
    /// and it goes away once a PULSE answers.
    var firstLaunch = false
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
    private let form = NSStackView()
    private let brainField = NSTextField()
    private let codeField = NSTextField()
    private let formMessage = NSTextField(wrappingLabelWithString: "")
    private var content: StatusContent?

    /// What is typed in the join fields now.
    var typed: (brain: String, code: String) { (brainField.stringValue, codeField.stringValue) }

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

        setUpForm()
        let column = NSStackView(views: [mark, spinner, titleLabel, detailLabel, form, buttons, logScroll])
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
            form.widthAnchor.constraint(equalTo: column.widthAnchor),
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

    private func setUpForm() {
        func label(_ text: String) -> NSTextField {
            let label = NSTextField(labelWithString: text)
            label.font = Fonts.text(13, weight: .semibold)
            label.textColor = Palette.ink
            return label
        }
        func style(_ field: NSTextField, _ placeholder: String, _ name: String) {
            field.font = Fonts.text(14)
            field.placeholderString = placeholder
            field.bezelStyle = .roundedBezel
            field.controlSize = .large
            field.lineBreakMode = .byTruncatingTail
            field.usesSingleLineMode = true
            field.setAccessibilityLabel(name)
            field.target = self
            field.action = #selector(fieldReturned(_:))
        }
        style(brainField, "https://pulse.example.com", "Brain address")
        style(codeField, "six words, like amber river cobalt maple orbit lantern", "Join code")
        brainField.identifier = NSUserInterfaceItemIdentifier("join-brain-field")
        codeField.identifier = NSUserInterfaceItemIdentifier("join-code-field")
        formMessage.font = Fonts.text(13)
        formMessage.isSelectable = true
        let rows: [NSView] = [label("Brain"), brainField, label("Join code"), codeField, formMessage]
        rows.forEach(form.addArrangedSubview)
        form.orientation = .vertical
        form.alignment = .leading
        form.spacing = 6
        form.setCustomSpacing(14, after: brainField)
        form.setCustomSpacing(10, after: codeField)
        for view in [brainField, codeField, formMessage] {
            view.widthAnchor.constraint(equalTo: form.widthAnchor).isActive = true
        }
        form.isHidden = true
    }

    /// Return in a field: the brain moves on to the code; the code joins.
    @objc private func fieldReturned(_ sender: NSTextField) {
        if sender === brainField, codeField.stringValue.isEmpty {
            window?.makeFirstResponder(codeField)
        } else if content?.form?.editable == true, let primary = content?.actions.first(where: \.primary) {
            onAction?(primary.id)
        }
    }

    func show(_ next: StatusContent) {
        guard next != content else { return }
        let previous = content?.form
        content = next
        if let fields = next.form {
            if previous == nil || previous?.brain != fields.brain { brainField.stringValue = fields.brain }
            if previous == nil || previous?.code != fields.code { codeField.stringValue = fields.code }
            brainField.isEnabled = fields.editable
            codeField.isEnabled = fields.editable
            formMessage.stringValue = fields.message ?? ""
            formMessage.textColor = fields.bad ? Palette.bad : Palette.muted
            formMessage.isHidden = (fields.message ?? "").isEmpty
            form.isHidden = false
            if previous == nil {
                DispatchQueue.main.async { [weak self] in
                    guard let self, !self.form.isHidden else { return }
                    self.window?.makeFirstResponder(self.brainField.stringValue.isEmpty ? self.brainField : self.codeField)
                }
            }
        } else {
            form.isHidden = true
        }
        titleLabel.stringValue = next.title
        detailLabel.attributedStringValue = Self.detail(next.detail)
        detailLabel.alignment = .center
        if next.busy { spinner.startAnimation(nil) } else { spinner.stopAnimation(nil) }

        buttons.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for action in next.actions {
            let button = NSButton(title: action.title, target: self, action: #selector(clicked(_:)))
            button.identifier = NSUserInterfaceItemIdentifier(action.id)
            button.isEnabled = next.form?.editable ?? true
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
