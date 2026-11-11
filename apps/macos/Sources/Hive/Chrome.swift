import AppKit

/// The strip under the transparent title bar. It continues the page's top
/// edge (the sidebar's colour over the sidebar, the canvas over the rest), so
/// the window reads as the console with traffic lights rather than a browser
/// frame around it.
final class TitleStrip: NSView {
    var canvas: NSColor = Palette.paper { didSet { needsDisplay = true } }
    var sidebar: PageChrome.Sidebar? { didSet { needsDisplay = true } }
    /// CSS pixels to points: the web view's page zoom.
    var zoom: CGFloat = 1 { didSet { needsDisplay = true } }

    override var isOpaque: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        canvas.setFill()
        bounds.fill()
        guard let sidebar else { return }
        let width = min(bounds.width, sidebar.width * zoom)
        sidebar.color.setFill()
        NSRect(x: 0, y: 0, width: width, height: bounds.height).fill(using: .sourceOver)
        if let border = sidebar.border, sidebar.borderWidth > 0 {
            let line = max(sidebar.borderWidth * zoom, 1 / (window?.backingScaleFactor ?? 2))
            border.setFill()
            NSRect(x: width - line, y: 0, width: line, height: bounds.height).fill(using: .sourceOver)
        }
    }

    // A double-click on the title bar area zooms or minimizes, as the system setting says.
    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2 {
            window?.performDoubleClickAction()
        } else {
            window?.performDrag(with: event)
        }
    }
}

extension NSWindow {
    /// "Double-click a window's title bar to…" (System Settings, Desktop & Dock).
    func performDoubleClickAction() {
        let action = UserDefaults.standard.string(forKey: "AppleActionOnDoubleClick") ?? "Maximize"
        switch action {
        case "Minimize": performMiniaturize(nil)
        case "None": break
        default: performZoom(nil)
        }
    }
}

/// A short note at the bottom of the window ("Downloaded trajectory.json").
final class Toast: NSView {
    private let label = NSTextField(labelWithString: "")
    private let button = NSButton(title: "", target: nil, action: nil)
    private var hideTask: Task<Void, Never>?
    private var onButton: (() -> Void)?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.cornerRadius = 12
        layer?.borderWidth = 1
        shadow = NSShadow()
        layer?.shadowOpacity = 0.18
        layer?.shadowRadius = 12
        layer?.shadowOffset = CGSize(width: 0, height: -4)
        isHidden = true

        label.font = Fonts.text(13)
        label.textColor = Palette.ink
        label.lineBreakMode = .byTruncatingMiddle
        button.bezelStyle = .inline
        button.font = Fonts.text(12, weight: .semibold)
        button.target = self
        button.action = #selector(pressed)

        let row = NSStackView(views: [label, button])
        row.orientation = .horizontal
        row.spacing = 12
        row.edgeInsets = NSEdgeInsets(top: 10, left: 14, bottom: 10, right: 10)
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor),
            row.trailingAnchor.constraint(equalTo: trailingAnchor),
            row.topAnchor.constraint(equalTo: topAnchor),
            row.bottomAnchor.constraint(equalTo: bottomAnchor),
            label.widthAnchor.constraint(lessThanOrEqualToConstant: 380),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    override var wantsUpdateLayer: Bool { true }

    override func updateLayer() {
        layer?.backgroundColor = Palette.card.cgColor
        layer?.borderColor = Palette.border.cgColor
    }

    func show(_ text: String, button title: String?, action: (() -> Void)?) {
        label.stringValue = text
        button.title = title ?? ""
        button.isHidden = title == nil
        onButton = action
        isHidden = false
        alphaValue = 1
        setAccessibilityLabel(text)
        NSAccessibility.post(element: self, notification: .announcementRequested, userInfo: [
            .announcement: text, .priority: NSAccessibilityPriorityLevel.medium.rawValue,
        ])
        hideTask?.cancel()
        hideTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 6_000_000_000)
            guard let self, !Task.isCancelled else { return }
            NSAnimationContext.runAnimationGroup({ context in
                context.duration = 0.25
                self.animator().alphaValue = 0
            }, completionHandler: {
                MainActor.assumeIsolated { self.isHidden = true }
            })
        }
    }

    @objc private func pressed() {
        onButton?()
        isHidden = true
    }
}
