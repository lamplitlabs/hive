// Hive pick helper - Pick & Polish for the hub's Browser tab.
//
// A dev page includes this script (<script src="http://HUB/pick.js"></script>,
// in development only); when the Hive hub shows the page in its Browser tab
// and you press Pick, the hub asks this script to start picking, you hover
// and click an element, and the script sends the hub what it is - tag, id,
// classes, text, a CSS path, nearest React components, its box - so the hub
// puts a chip in your message and the agent polishes exactly that element.
//
// Plain script, no dependencies, nothing stored, nothing sent anywhere but
// to the window that frames the page. Does nothing when the page is not
// framed. Safe to leave in a dev build; keep it out of production.
(function () {
  "use strict";
  if (typeof window === "undefined" || window.parent === window) return;
  var hub = null; // the hub's origin, learnt from its first message
  var on = false;
  var overlay = null;
  var label = null;
  var current = null;

  function send(msg) {
    if (hub) window.parent.postMessage(msg, hub);
  }

  // ----- describing an element ---------------------------------------------

  function cut(s, n) {
    s = String(s == null ? "" : s);
    return s.length > n ? s.slice(0, n) : s;
  }

  function classList(el) {
    var out = [];
    var cls = typeof el.className === "string" ? el.className : el.getAttribute("class") || "";
    cls.split(/\s+/).forEach(function (c) {
      if (c && out.length < 12) out.push(c);
    });
    return out;
  }

  // A CSS path that finds the element again: an id ends it; otherwise each
  // step is the tag with its position among same-tag siblings when needed.
  function selectorOf(el) {
    var parts = [];
    var node = el;
    while (node && node.nodeType === 1 && node !== document.documentElement && parts.length < 12) {
      var tag = node.tagName.toLowerCase();
      if (node.id && /^[A-Za-z][\w-]*$/.test(node.id)) {
        parts.unshift(tag + "#" + node.id);
        break;
      }
      var step = tag;
      var parent = node.parentElement;
      if (parent) {
        var same = Array.prototype.filter.call(parent.children, function (c) {
          return c.tagName === node.tagName;
        });
        if (same.length > 1) step += ":nth-of-type(" + (same.indexOf(node) + 1) + ")";
      }
      parts.unshift(step);
      if (tag === "body") break;
      node = parent;
    }
    return parts.join(" > ");
  }

  // React in development names its components on the fiber tree; the
  // innermost few are what the agent greps for.
  function componentsOf(el) {
    var key = null;
    for (var k in el) {
      if (k.indexOf("__reactFiber$") === 0 || k.indexOf("__reactInternalInstance$") === 0) {
        key = k;
        break;
      }
    }
    if (!key) return [];
    var out = [];
    var fiber = el[key];
    var guard = 0;
    while (fiber && out.length < 6 && guard++ < 400) {
      var t = fiber.type;
      var name = typeof t === "function" ? t.displayName || t.name : t && typeof t === "object" ? t.displayName || (t.render && (t.render.displayName || t.render.name)) || (t.type && (t.type.displayName || t.type.name)) : "";
      if (name && /^[A-Z]/.test(name) && out.indexOf(name) < 0) out.push(name);
      fiber = fiber.return;
    }
    return out;
  }

  function describe(el) {
    var r = el.getBoundingClientRect();
    var text = cut((el.innerText || el.textContent || "").replace(/\s+/g, " ").trim(), 400);
    var html = cut(el.outerHTML || "", 2000);
    var label = el.getAttribute("aria-label") || el.getAttribute("alt") || el.getAttribute("title") || el.getAttribute("placeholder") || el.getAttribute("name") || "";
    return {
      tag: el.tagName,
      id: cut(el.id || "", 120),
      classes: classList(el),
      text: text,
      html: html,
      selector: cut(selectorOf(el), 500),
      role: cut(el.getAttribute("role") || "", 40),
      label: cut(label, 200),
      href: cut(el.getAttribute("href") || "", 500),
      src: cut(el.getAttribute("src") || "", 500),
      components: componentsOf(el),
      rect: { x: Math.round(r.left + window.scrollX), y: Math.round(r.top + window.scrollY), w: Math.round(r.width), h: Math.round(r.height) },
    };
  }

  // ----- the overlay ---------------------------------------------------------

  function ensureOverlay() {
    if (overlay) return;
    overlay = document.createElement("div");
    overlay.setAttribute("data-hive-pick", "overlay");
    overlay.style.cssText = "position:fixed;z-index:2147483646;pointer-events:none;border:2px solid #0f8f8a;background:rgba(15,143,138,.12);border-radius:4px;box-shadow:0 0 0 1px rgba(255,255,255,.7);transition:all 60ms ease-out;display:none";
    label = document.createElement("div");
    label.setAttribute("data-hive-pick", "label");
    label.style.cssText = "position:fixed;z-index:2147483647;pointer-events:none;font:12px/1.4 ui-monospace,Menlo,Consolas,monospace;color:#fff;background:#0f8f8a;padding:2px 6px;border-radius:4px;white-space:nowrap;max-width:60vw;overflow:hidden;text-overflow:ellipsis;display:none";
    document.documentElement.appendChild(overlay);
    document.documentElement.appendChild(label);
  }

  function nameOf(el) {
    var cls = classList(el).slice(0, 2);
    return el.tagName.toLowerCase() + (el.id ? "#" + el.id : "") + (cls.length ? "." + cls.join(".") : "");
  }

  function highlight(el) {
    if (!overlay || !el) return;
    var r = el.getBoundingClientRect();
    overlay.style.display = "block";
    overlay.style.left = r.left + "px";
    overlay.style.top = r.top + "px";
    overlay.style.width = r.width + "px";
    overlay.style.height = r.height + "px";
    label.style.display = "block";
    label.textContent = nameOf(el) + "  " + Math.round(r.width) + "×" + Math.round(r.height);
    var top = r.top - 24;
    label.style.top = (top < 4 ? r.bottom + 4 : top) + "px";
    label.style.left = Math.max(4, r.left) + "px";
  }

  function clearHighlight() {
    if (overlay) overlay.style.display = "none";
    if (label) label.style.display = "none";
    current = null;
  }

  function targetAt(e) {
    var el = document.elementFromPoint(e.clientX, e.clientY);
    if (!el || el.getAttribute("data-hive-pick")) return null;
    if (el === document.documentElement || el === document.body) return null;
    return el;
  }

  // ----- picking --------------------------------------------------------------

  function onMove(e) {
    var el = targetAt(e);
    if (el !== current) {
      current = el;
      if (el) highlight(el);
      else clearHighlight();
    }
  }

  function swallow(e) {
    e.preventDefault();
    e.stopPropagation();
    if (e.stopImmediatePropagation) e.stopImmediatePropagation();
  }

  function onClick(e) {
    swallow(e);
    var el = targetAt(e) || current;
    if (!el) return;
    send({ type: "hive-pick-picked", url: location.href, title: document.title, element: describe(el) });
    stop();
  }

  function onKey(e) {
    if (e.key === "Escape") {
      swallow(e);
      send({ type: "hive-pick-cancel" });
      stop();
    }
  }

  var EVENTS = ["mousedown", "mouseup", "pointerdown", "pointerup", "auxclick", "dblclick", "contextmenu"];

  function start() {
    if (on) return;
    on = true;
    ensureOverlay();
    document.documentElement.style.cursor = "crosshair";
    document.addEventListener("mousemove", onMove, true);
    document.addEventListener("click", onClick, true);
    document.addEventListener("keydown", onKey, true);
    EVENTS.forEach(function (ev) {
      document.addEventListener(ev, swallow, true);
    });
  }

  function stop() {
    if (!on) return;
    on = false;
    document.documentElement.style.cursor = "";
    document.removeEventListener("mousemove", onMove, true);
    document.removeEventListener("click", onClick, true);
    document.removeEventListener("keydown", onKey, true);
    EVENTS.forEach(function (ev) {
      document.removeEventListener(ev, swallow, true);
    });
    clearHighlight();
  }

  // ----- the hub ----------------------------------------------------------------

  window.addEventListener("message", function (e) {
    var d = e.data;
    if (!d || typeof d !== "object" || d.type !== "hive-pick") return;
    if (e.source !== window.parent) return;
    // The first word from the hub names it; later ones must come from the same place.
    if (hub && e.origin !== hub) return;
    if (!hub) {
      hub = e.origin;
      send({ type: "hive-pick-hello", url: location.href, title: document.title });
    }
    if (d.on) start();
    else stop();
  });

  // Tell the hub the page is here (and where it is) on load and on every
  // SPA move, so the address bar follows. The hub's origin is only known
  // after it speaks; before that the broadcast goes to the parent as-is.
  function hello() {
    var msg = { type: "hive-pick-hello", url: location.href, title: document.title };
    window.parent.postMessage(msg, hub || "*");
  }
  hello();
  var seen = location.href;
  setInterval(function () {
    if (location.href !== seen) {
      seen = location.href;
      hello();
    }
  }, 500);
})();
