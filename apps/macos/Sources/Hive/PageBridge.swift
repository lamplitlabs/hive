import AppKit
import WebKit

/// The little the app needs to know from, and do to, the console page. The
/// script runs in its own content world: the page's scripts can neither see
/// it nor post to the app, and the page's CSP does not apply to it.
@MainActor
enum PageBridge {
    static let handlerName = "hiveShell"
    static let world = WKContentWorld.defaultClient

    /// Reports the page's theme and the colours along its top edge (the
    /// sidebar's and the canvas'), so the title bar above continues them.
    /// Colours go through a 1x1 canvas: computed styles keep the console's
    /// oklch() values, the canvas hands back sRGB bytes.
    static let script = """
    (() => {
      if (window.__hiveShellBridge) return;
      window.__hiveShellBridge = true;
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = 1;
      const ctx = canvas.getContext('2d', { willReadFrequently: true });
      const rgba = (css) => {
        if (!css || !ctx) return null;
        ctx.clearRect(0, 0, 1, 1);
        ctx.fillStyle = '#000';
        ctx.fillStyle = css;
        ctx.fillRect(0, 0, 1, 1);
        const d = ctx.getImageData(0, 0, 1, 1).data;
        return [d[0], d[1], d[2], d[3] / 255];
      };
      let last = '';
      const report = () => {
        const root = document.documentElement;
        const body = document.body;
        if (!root || !body) return;
        let choice = 'system';
        try { choice = localStorage.getItem('theme') || 'system'; } catch (_) {}
        let sidebar = null;
        const aside = document.querySelector('aside[aria-label="Sidebar"]');
        if (aside) {
          const style = getComputedStyle(aside);
          const box = aside.getBoundingClientRect();
          if (style.display !== 'none' && style.visibility !== 'hidden' && box.width > 0 && box.left <= 1 && box.top <= 1) {
            const border = parseFloat(style.borderRightWidth) || 0;
            sidebar = {
              width: box.right,
              color: rgba(style.backgroundColor),
              border: border > 0 ? rgba(style.borderRightColor) : null,
              borderWidth: border,
            };
          }
        }
        const message = {
          kind: 'chrome',
          dark: root.classList.contains('dark'),
          visible: document.visibilityState === 'visible',
          choice,
          canvas: rgba(getComputedStyle(body).backgroundColor),
          sidebar,
        };
        const text = JSON.stringify(message);
        if (text === last) return;
        last = text;
        window.webkit.messageHandlers.\(handlerName).postMessage(message);
      };
      let queued = false;
      const schedule = () => {
        if (queued) return;
        queued = true;
        const run = () => { queued = false; report(); };
        // Animation frames pause while the page is hidden; a timer does not.
        if (document.visibilityState === 'visible') requestAnimationFrame(run); else setTimeout(run, 50);
      };
      new MutationObserver(schedule).observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'style'] });
      new MutationObserver(schedule).observe(document.body, { childList: true, subtree: true });
      window.addEventListener('resize', schedule);
      window.addEventListener('storage', schedule);
      document.addEventListener('visibilitychange', schedule);
      schedule();
      // The link under the pointer at a right-click, for the app's own menu;
      // the report reaches the app before WebKit's menu does.
      document.addEventListener('contextmenu', (event) => {
        const node = event.target;
        const element = node instanceof Element ? node : node && node.parentElement;
        const anchor = element ? element.closest('a[href], area[href]') : null;
        let link = null;
        if (anchor) {
          try { link = new URL(anchor.getAttribute('href'), document.baseURI).href; } catch (_) {}
        }
        window.webkit.messageHandlers.\(handlerName).postMessage({ kind: 'contextmenu', link });
      }, true);
    })();
    """

    static var userScript: WKUserScript {
        WKUserScript(source: script, injectionTime: .atDocumentEnd, forMainFrameOnly: true, in: world)
    }

    /// Clicks the sidebar's own link, so the page navigates the way it does
    /// for a click (keeping its data and live stream); false when the link
    /// is not there.
    static func clickSidebarLink(_ path: String) -> String {
        let quoted = jsString(path)
        return """
        (() => {
          const link = document.querySelector('aside[aria-label="Sidebar"] a[href=' + JSON.stringify(\(quoted)) + ']');
          if (!link) return false;
          link.click();
          return true;
        })()
        """
    }

    /// The hard-stop strip on Sessions (the console has no For you page since
    /// TASK-129), without a reload when Sessions is open.
    static let showForYou = """
    (() => {
      if (location.pathname !== '/chat/') return false;
      const section = document.getElementById('hard-stops');
      if (section) section.scrollIntoView({ behavior: 'smooth', block: 'start' });
      history.replaceState(history.state, '', '#hard-stops');
      return true;
    })()
    """

    /// Presses one of the page's own buttons (`[data-palette-trigger]`, `[data-ask-trigger]`).
    static func press(_ selector: String) -> String {
        "(() => { const b = document.querySelector(\(jsString(selector))); if (!b) return false; b.click(); return true; })()"
    }

    /// What the theme toggle in the sidebar does (next-themes listens for the
    /// storage event), run in the page's own world.
    static func setTheme(_ choice: String) -> String {
        let value = jsString(choice)
        return """
        (() => {
          try { localStorage.setItem('theme', \(value)); } catch (_) { return false; }
          window.dispatchEvent(new StorageEvent('storage', { key: 'theme', newValue: \(value), storageArea: localStorage }));
          return true;
        })()
        """
    }

    static func jsString(_ text: String) -> String {
        let data = try? JSONSerialization.data(withJSONObject: [text])
        let array = data.flatMap { String(data: $0, encoding: .utf8) } ?? "[\"\"]"
        return String(array.dropFirst().dropLast())
    }
}

/// What the page reported about itself.
struct PageChrome: Equatable {
    struct Sidebar: Equatable {
        var width: CGFloat
        var color: NSColor
        var border: NSColor?
        var borderWidth: CGFloat
    }

    var dark: Bool
    /// What the page itself sees: hidden pages pause the console's refreshes.
    var visible: Bool
    /// The owner's choice in the console: light, dark or system.
    var choice: String
    var canvas: NSColor?
    var sidebar: Sidebar?

    init?(_ body: Any) {
        guard let message = body as? [String: Any], message["kind"] as? String == "chrome" else { return nil }
        dark = message["dark"] as? Bool ?? false
        visible = message["visible"] as? Bool ?? true
        let choice = message["choice"] as? String ?? "system"
        self.choice = ["light", "dark", "system"].contains(choice) ? choice : "system"
        canvas = Self.color(message["canvas"])
        if let side = message["sidebar"] as? [String: Any],
           let width = (side["width"] as? NSNumber).map({ CGFloat($0.doubleValue) }), width > 0, width < 2000,
           let color = Self.color(side["color"]) {
            let borderWidth = (side["borderWidth"] as? NSNumber).map { CGFloat($0.doubleValue) } ?? 0
            sidebar = Sidebar(width: width, color: color, border: Self.color(side["border"]), borderWidth: min(max(borderWidth, 0), 4))
        }
    }

    static func color(_ value: Any?) -> NSColor? {
        guard let parts = value as? [NSNumber], parts.count == 4 else { return nil }
        let alpha = CGFloat(parts[3].doubleValue)
        guard alpha > 0 else { return nil }
        return NSColor(
            srgbRed: CGFloat(parts[0].doubleValue) / 255,
            green: CGFloat(parts[1].doubleValue) / 255,
            blue: CGFloat(parts[2].doubleValue) / 255,
            alpha: min(alpha, 1)
        )
    }
}

/// What the page reported about a right-click: the link under the pointer, if any.
struct PageContextMenu: Equatable {
    var link: URL?

    init?(_ body: Any) {
        guard let message = body as? [String: Any], message["kind"] as? String == "contextmenu" else { return nil }
        link = (message["link"] as? String).flatMap { URL(string: $0) }
    }
}

/// WKUserContentController keeps its handlers strongly; this keeps the window weakly.
final class WeakScriptHandler: NSObject, WKScriptMessageHandler {
    private weak var target: (any WKScriptMessageHandler)?

    init(_ target: any WKScriptMessageHandler) {
        self.target = target
    }

    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        target?.userContentController(controller, didReceive: message)
    }
}
