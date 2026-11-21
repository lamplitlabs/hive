#!/usr/bin/env python3
"""Cuts the site's two fonts down to what an English page shows.

    python3 scripts/subset-fonts.py            write the files, print the CSS
    python3 scripts/subset-fonts.py --check    exit 1 when the files or the CSS differ

Geist and Geist Mono (public/fonts/geist-*-variable.woff2) hold Latin,
extended Latin, Greek, Cyrillic and Vietnamese: about 70 KB each. A page of
this site shows Latin letters, punctuation, arrows, mathematical signs and
box drawing, and every page loads both fonts. So each family is served as
two faces (src/app/globals.css):

- geist-*-latin.woff2: the glyphs for LATIN below, with every OpenType
  feature and every weight of the whole font. Every page loads it.
- geist-*-variable.woff2: the whole font, as it was. Its unicode-range is
  the rest of what the font holds, so a browser fetches it only for a page
  that shows one of those characters. The text looks the same either way.

From U+2000 up the rest is listed code point by code point. A sign the fonts
do not hold (the docs use a few) must not be in the list: it would make the
browser fetch the whole font and find nothing in it.

The host keeps /fonts/ for a year (vercel.json). When LATIN changes, give the
subset files a new name: here, in globals.css and in src/app/layout.tsx.

Needs fonttools and brotli (python3 -m pip install fonttools brotli). The
build does not run this: the files it writes are committed.
"""
import io
import pathlib
import re
import sys

from fontTools import subset
from fontTools.ttLib import TTFont

SITE = pathlib.Path(__file__).resolve().parent.parent
FONTS = SITE / "public" / "fonts"
CSS = SITE / "src" / "app" / "globals.css"

# Basic Latin and Latin-1, general punctuation, the euro and trade mark signs,
# arrows and mathematical operators, box drawing.
LATIN = "U+0000-00FF, U+2000-206F, U+20AC, U+2122, U+2190-22FF, U+2500-257F"
FAMILIES = {"Geist": "geist-sans", "Geist Mono": "geist-mono"}
# Below this, a gap of a few code points inside a run of letters is closed.
EXACT_FROM = 0x2000
GAP = 8


def subset_bytes(source: pathlib.Path) -> bytes:
    options = subset.Options()
    options.flavor = "woff2"
    options.layout_features = ["*"]
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    options.notdef_outline = True
    # Without the glyph names macOS draws text under 20 pixels a little softer than
    # it draws the whole font (compared pixel by pixel in Edge 158 on macOS 27).
    options.glyph_names = True
    # The design-language list: nothing draws with it, and the subsetter cannot cut it.
    options.drop_tables += ["meta"]
    font = subset.load_font(str(source), options)
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=subset.parse_unicodes(LATIN.replace(" ", "")))
    subsetter.subset(font)
    out = io.BytesIO()
    font.flavor = "woff2"
    font.save(out)
    return out.getvalue()


def tables(data: bytes) -> dict[str, bytes]:
    """A font file's tables as they are once unpacked: two files that differ only in how hard they were packed are the same font."""
    font = TTFont(io.BytesIO(data))
    return {tag: font.getTableData(tag) for tag in font.reader.keys()}


def rest_range(source: pathlib.Path) -> str:
    """What the whole font holds outside LATIN, as a unicode-range."""
    latin = set(subset.parse_unicodes(LATIN.replace(" ", "")))
    points = sorted(set(TTFont(str(source)).getBestCmap()) - latin)
    runs: list[list[int]] = []
    for point in points:
        gap = GAP if point < EXACT_FROM else 0
        if runs and point - runs[-1][1] - 1 <= gap:
            runs[-1][1] = point
        else:
            runs.append([point, point])
    return ", ".join(f"U+{a:04X}" if a == b else f"U+{a:04X}-{b:04X}" for a, b in runs)


def css_ranges(css: str, family: str) -> list[str]:
    """The unicode-range of each @font-face of a family, in the order the CSS has them."""
    found = []
    for block in re.findall(r"@font-face\s*{([^}]*)}", css):
        if re.search(rf'font-family:\s*"{re.escape(family)}"\s*;', block):
            m = re.search(r"unicode-range:\s*([^;]+);", block)
            found.append(" ".join(m.group(1).split()) if m else "")
    return found


def main() -> int:
    check = "--check" in sys.argv[1:]
    css = CSS.read_text(encoding="utf-8")
    wrong = []
    for family, name in FAMILIES.items():
        whole = FONTS / f"{name}-variable.woff2"
        target = FONTS / f"{name}-latin.woff2"
        data = subset_bytes(whole)
        rest = rest_range(whole)
        if check:
            if not target.exists() or tables(target.read_bytes()) != tables(data):
                wrong.append(f"{target.relative_to(SITE)} is not what this script writes")
            if css_ranges(css, family) != [LATIN, rest]:
                wrong.append(f'the two unicode-range lines of "{family}" in {CSS.relative_to(SITE)} are not the ones below')
        else:
            target.write_bytes(data)
        print(f'{family}: {target.name} is {len(data)} bytes, {whole.name} is {whole.stat().st_size}.')
        print(f"  unicode-range: {LATIN};")
        print(f"  unicode-range: {rest};")
    for line in wrong:
        print(f"subset-fonts: {line}", file=sys.stderr)
    return 1 if wrong else 0


if __name__ == "__main__":
    sys.exit(main())
