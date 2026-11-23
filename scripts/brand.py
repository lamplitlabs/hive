#!/usr/bin/env python3
"""The Hive brand kit: one geometry, every asset.

    python3 scripts/brand.py            write the assets into the tree
    python3 scripts/brand.py --print    print the mark's paths (what the
                                        console, the site and the Mac app
                                        hold as constants)

The mark is a hexagon cut into three brackets around a hexagonal core, with a
node in each gap: the brain, and the machines paired with it. It is drawn on
a 256 box. Everything else here - the favicon, the app icon, the README
banner, the social cards - is that mark placed on a field.

SVG files are written with the standard library alone. The PNG files (social
cards, touch icons) need `rsvg-convert` (librsvg) on the PATH; without it
they are skipped and the script says so.

Where the same shape is held as constants, change both:
    web/hub/src/components/shell/hive-mark.tsx
    web/site/src/components/hive-mark.tsx
    apps/macos/Sources/Hive/StatusView.swift   (HiveMarkView)
"""
import math
import os
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

MINT, SKY, VIOLET = "#50f4c5", "#11bff0", "#877af7"  # the console's brand stops
NIGHT, NIGHT_2 = "#060a14", "#0c1530"                # the field the mark glows on


def hexpts(cx, cy, r, rot=-90):
    return [(cx + r * math.cos(math.radians(rot + 60 * i)), cy + r * math.sin(math.radians(rot + 60 * i))) for i in range(6)]


def f(n):
    s = f"{n:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def poly(pts):
    return "M" + "L".join(f"{f(x)} {f(y)}" for x, y in pts) + "z"


# The mark: a hexagon of radius 96 about (128, 128). The corners at 12, 4 and
# 8 o'clock keep an arm along each side; the other three are open and hold a
# node each.
RADIUS, ARM, STROKE, NODE, CORE = 96, 58, 22, 13.5, 28
CORNERS = hexpts(128, 128, RADIUS)


def _along(a, b, d):
    length = math.dist(a, b)
    return (a[0] + (b[0] - a[0]) * d / length, a[1] + (b[1] - a[1]) * d / length)


def _bracket(i):
    v, before, after = CORNERS[i], CORNERS[(i - 1) % 6], CORNERS[(i + 1) % 6]
    s, e = _along(v, before, ARM), _along(v, after, ARM)
    return f"M{f(s[0])} {f(s[1])}L{f(v[0])} {f(v[1])}L{f(e[0])} {f(e[1])}"


BRACKETS = [_bracket(i) for i in (0, 2, 4)]
NODES = [poly(hexpts(CORNERS[i][0], CORNERS[i][1], NODE)) for i in (1, 3, 5)]
CORE_PATH = poly(hexpts(128, 128, CORE))


def mark(color, stroke=STROKE):
    brackets = "".join(f'<path d="{d}"/>' for d in BRACKETS)
    solids = "".join(f'<path d="{d}"/>' for d in NODES) + f'<path d="{CORE_PATH}"/>'
    return (f'<g fill="none" stroke="{color}" stroke-width="{f(stroke)}" stroke-linejoin="miter" stroke-miterlimit="6">{brackets}</g>'
            f'<g fill="{color}">{solids}</g>')


def sweep(name, x1, y1, x2, y2):
    return (f'<linearGradient id="{name}" gradientUnits="userSpaceOnUse" x1="{f(x1)}" y1="{f(y1)}" x2="{f(x2)}" y2="{f(y2)}">'
            f'<stop offset="0" stop-color="{MINT}"/><stop offset="0.52" stop-color="{SKY}"/><stop offset="1" stop-color="{VIOLET}"/></linearGradient>')


def glow(name, spread):
    return (f'<filter id="{name}" x="-40%" y="-40%" width="180%" height="180%"><feGaussianBlur stdDeviation="{spread}" result="b"/>'
            f'<feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge></filter>')


def night(name):
    return f'<linearGradient id="{name}" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{NIGHT_2}"/><stop offset="1" stop-color="{NIGHT}"/></linearGradient>'


def favicon():
    """32 px and below: the mark a little heavier, no glow, on the night tile."""
    scale, stroke = 0.84, 27
    off = 128 * (1 - scale)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="32" height="32">'
            f'<defs>{sweep("g", 24, 24, 232, 232)}{night("n")}</defs>'
            f'<rect width="256" height="256" rx="60" fill="url(#n)"/>'
            f'<g transform="translate({f(off)} {f(off)}) scale({scale})">{mark("url(#g)", stroke)}</g></svg>')


def touch_icon():
    """The mark glowing on the night tile, full bleed: the touch icons."""
    scale = 0.72
    off = 128 * (1 - scale)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="512" height="512">'
            f'<defs>{sweep("g", 24, 24, 232, 232)}{night("n")}{glow("glow", 5)}</defs>'
            f'<rect width="256" height="256" fill="url(#n)"/>'
            f'<g transform="translate({f(off)} {f(off)}) scale({scale})" filter="url(#glow)">{mark("url(#g)")}</g></svg>')


def app_icon():
    """The Mac app's icon: an 824 pt tile on a 1024 pt canvas, the macOS grid.
    apps/macos/scripts/build.sh renders it into AppIcon.icns with sips, which
    draws no filters, so the mark is drawn crisp here, with no glow."""
    scale = 824 * 0.74 / 256
    off = 512 - 128 * scale
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">\n'
            f'  <!-- The Hive mark (scripts/brand.py) on the macOS icon grid: an 824 pt tile on a\n'
            f'       1024 pt canvas. scripts/build.sh renders it into AppIcon.icns. -->\n'
            f'  <defs>{sweep("g", 24, 24, 232, 232)}{night("n")}{sweep("edge", 100, 100, 924, 924)}</defs>\n'
            f'  <rect x="100" y="100" width="824" height="824" rx="185" fill="url(#n)"/>\n'
            f'  <rect x="103" y="103" width="818" height="818" rx="182" fill="none" stroke="url(#edge)" stroke-opacity="0.45" stroke-width="6"/>\n'
            f'  <g transform="translate({f(off)} {f(off)}) scale({f(scale)})">{mark("url(#g)")}</g>\n'
            f'</svg>')


# The word: four letters as strokes on a cap line. The V is the mark's top
# bracket turned over; the E is three bars with no stem.
def wordmark(x, y, cap=108, stroke=17, ink="#f3f7ff", accent="url(#gw)", gap=0.36):
    k = cap / 100
    w, g, sw = 76 * k, gap * 100 * k, stroke * k
    cx = x
    parts = [f'<path d="M{f(cx)} {f(y)}v{f(cap)}M{f(cx + w)} {f(y)}v{f(cap)}M{f(cx)} {f(y + cap / 2)}h{f(w)}"/>']
    cx += w + g
    parts.append(f'<path d="M{f(cx)} {f(y)}v{f(cap)}"/>')
    cx += g
    drop = cap - sw * 0.35
    parts.append(f'<path d="M{f(cx)} {f(y)}l{f(w / 2)} {f(drop)}l{f(w / 2)} {f(-drop)}" stroke="{accent}"/>')
    cx += w + g
    e = w * 0.86
    parts.append(f'<path d="M{f(cx)} {f(y + sw / 2)}h{f(e)}M{f(cx)} {f(y + cap / 2)}h{f(e * 0.78)}M{f(cx)} {f(y + cap - sw / 2)}h{f(e)}"/>')
    cx += e
    return (f'<g fill="none" stroke="{ink}" stroke-width="{f(sw)}" stroke-linejoin="miter" stroke-miterlimit="8">' + "".join(parts) + "</g>", cx - x)


MONO = "ui-monospace, 'SF Mono', SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace"
TAGLINE = "THE SELF-IMPROVING AGENT HARNESS"
SUBLINE = "open source  \u00b7  one binary  \u00b7  your own machines"
DESCRIPTION = ("The Hive mark: a hexagon cut into three brackets around a core, with a node in each gap. "
               "Beside it the word HIVE and the line: the self-improving agent harness.")

# The lockup on its own box: the mark, the word, two lines.
LOCK_W, LOCK_H = 840, 224


def lockup():
    ms = LOCK_H / 256
    wx = LOCK_H + 72
    word, width = wordmark(wx, 10)
    text = (f'<text x="{f(wx)}" y="168" fill="#cfe3ff" font-family="{MONO}" font-size="21" font-weight="600" letter-spacing="4.3">{TAGLINE}</text>'
            f'<text x="{f(wx)}" y="202" fill="#7f93b5" font-family="{MONO}" font-size="17" letter-spacing="1.4">{SUBLINE}</text>')
    return f'<g transform="scale({f(ms)})" filter="url(#glow)">{mark("url(#gm)")}</g>{word}{text}', wx, width


def honeycomb(w, h, r=34):
    dx, dy = r * math.sqrt(3), r * 1.5
    cells, row, y = [], 0, -r
    while y < h + r:
        x = (dx / 2 if row % 2 else 0) - dx
        while x < w + dx:
            cells.append(poly(hexpts(x, y, r)))
            x += dx
        y += dy
        row += 1
    return f'<path d="{"".join(cells)}" fill="none" stroke="#9fd8ff" stroke-opacity="0.07" stroke-width="1"/>'


def machines(points):
    """A few paired machines: nodes and the links between them."""
    links = ((0, 1), (1, 2), (2, 3), (3, 4), (4, 0), (0, 2), (4, 2))
    lines = "".join(f'<path d="M{f(points[i][0])} {f(points[i][1])}L{f(points[j][0])} {f(points[j][1])}"/>' for i, j in links)
    dots = "".join(f'<path d="{poly(hexpts(x, y, 7 if i else 11))}"/>' for i, (x, y) in enumerate(points))
    return (f'<g stroke="{SKY}" stroke-opacity="0.35" stroke-width="1.5" fill="none">{lines}</g>'
            f'<g fill="{VIOLET}" fill-opacity="0.9" filter="url(#soft)">{dots}</g>')


def field(w, h, rx=0, strip=False):
    """The lockup on the night field. A strip (the README banner) sets it to
    the left with the machines beside it; a card centres it."""
    s = 1 if strip else min(w * 0.8 / LOCK_W, h * 0.56 / LOCK_H)
    x0 = 68 if strip else (w - LOCK_W * s) / 2
    y0 = (h - LOCK_H * s) / 2
    lock, wx, width = lockup()
    if strip:
        points = [(w * 0.845, h * 0.27), (w * 0.925, h * 0.19), (w * 0.957, h * 0.50), (w * 0.895, h * 0.77), (w * 0.815, h * 0.86)]
    else:
        points = [(w * 0.80, h * 0.13), (w * 0.90, h * 0.07), (w * 0.955, h * 0.20), (w * 0.93, h * 0.045), (w * 0.86, h * 0.21)]
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" role="img" aria-labelledby="hive-title hive-desc">'
            f'<title id="hive-title">Hive</title><desc id="hive-desc">{DESCRIPTION}</desc>'
            f'<defs>{sweep("gm", 24, 24, 232, 232)}{sweep("gw", wx, 0, wx + width, 120)}{glow("glow", 7)}{glow("soft", 2.5)}{night("bg")}'
            f'<radialGradient id="a" cx="0.16" cy="0.5" r="0.5"><stop offset="0" stop-color="{SKY}" stop-opacity="0.22"/><stop offset="1" stop-color="{SKY}" stop-opacity="0"/></radialGradient>'
            f'<radialGradient id="b" cx="0.88" cy="0.3" r="0.45"><stop offset="0" stop-color="{VIOLET}" stop-opacity="0.24"/><stop offset="1" stop-color="{VIOLET}" stop-opacity="0"/></radialGradient>'
            f'<linearGradient id="fade" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#fff" stop-opacity="0.25"/><stop offset="0.55" stop-color="#fff" stop-opacity="0.5"/><stop offset="1" stop-color="#fff"/></linearGradient>'
            f'<mask id="m"><rect width="{w}" height="{h}" fill="url(#fade)"/></mask>'
            f'<clipPath id="c"><rect width="{w}" height="{h}" rx="{rx}"/></clipPath></defs>'
            f'<g clip-path="url(#c)"><rect width="{w}" height="{h}" fill="url(#bg)"/>'
            f'<g mask="url(#m)">{honeycomb(w, h)}</g>'
            f'<rect width="{w}" height="{h}" fill="url(#a)"/><rect width="{w}" height="{h}" fill="url(#b)"/>'
            f'{machines(points)}'
            f'<g transform="translate({f(x0)} {f(y0)}) scale({f(s)})">{lock}</g>'
            f'</g><rect x="0.75" y="0.75" width="{f(w - 1.5)}" height="{f(h - 1.5)}" rx="{rx}" fill="none" stroke="#9fd8ff" stroke-opacity="0.16" stroke-width="1.5"/></svg>')


SVG = {
    ".github/assets/hive-logo.svg": lambda: field(1200, 360, rx=28, strip=True),
    "web/site/src/app/icon.svg": favicon,
    "web/hub/src/app/icon.svg": favicon,
    "apps/macos/Resources/AppIcon.svg": app_icon,
}

# (path, source, width, height)
PNG = [
    (".github/assets/social-preview.png", lambda: field(1280, 640), 1280, 640),
    ("web/site/public/og.png", lambda: field(1200, 630), 1200, 630),
    ("web/site/src/app/apple-icon.png", touch_icon, 180, 180),
    ("web/site/public/icon-192.png", touch_icon, 192, 192),
    ("web/site/public/icon-512.png", touch_icon, 512, 512),
]


def main():
    if "--print" in sys.argv:
        print("brackets:", *BRACKETS, sep="\n  ")
        print("nodes:", *NODES, sep="\n  ")
        print("core:\n  " + CORE_PATH)
        print(f"stroke: {STROKE}   bracket length: {ARM * 2}")
        return
    for rel, make in SVG.items():
        path = os.path.join(ROOT, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as out:
            out.write(make() + "\n")
        print("wrote", rel)
    rsvg = shutil.which("rsvg-convert")
    if not rsvg:
        print("rsvg-convert is not on the PATH: the PNG files were left as they are (" + ", ".join(p[0] for p in PNG) + ")")
        return
    for rel, make, w, h in PNG:
        path = os.path.join(ROOT, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        subprocess.run([rsvg, "-w", str(w), "-h", str(h), "-o", path], input=make().encode("utf-8"), check=True)
        print("wrote", rel)


if __name__ == "__main__":
    main()
