// A picture of the docs, as a page shows it. src/lib/markdown.ts finds the
// picture, gives it the site's own address and its size, and hands it here for
// its markup; src/app/docs-shots.css draws it. Pure, so the node tests import
// it as it is.
import type { Element, ElementContent } from "hast";

const el = (tagName: string, properties: Element["properties"], children: ElementContent[] = []): Element => ({ type: "element", tagName, properties, children });

/** The widest a docs page shows a picture: the width of its column of text, in CSS pixels. */
const COLUMN_WIDTH = 816;

/** The two arrows that say a picture opens larger (Lucide's maximize-2, drawn here because this markup is not React's). */
const OPEN_ICON = el("svg", { viewBox: "0 0 24 24", width: 14, height: 14, fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round" }, [
  el("path", { d: "M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" }),
]);

/**
 * The element that takes the place of `img`, a picture kept under docs/ that
 * the site serves at `media`: framed, and a link to the picture at its own
 * size, named by what the picture shows, with a mark that says a click opens
 * it larger. A picture of a whole screen is shown well under its size on any
 * page, so the way to the full one has to be seen.
 */
export function shotElement(img: Element, media: string): Element {
  const alt = typeof img.properties.alt === "string" ? img.properties.alt : "";
  const name = alt ? `Open the picture at full size: ${alt}` : "Open the picture at full size";
  // A picture wider than the column is shown smaller than it is on every screen: its mark is always there.
  const large = typeof img.properties.width === "number" && img.properties.width > COLUMN_WIDTH;
  return el("a", { className: large ? ["shot", "shot-large"] : ["shot"], href: media, ariaLabel: name }, [img, el("span", { className: ["shot-open"], ariaHidden: "true" }, [OPEN_ICON, { type: "text", value: " Full size" }])]);
}

/** The picture a paragraph holds and nothing else, if it holds one: the link `shotElement` made. */
export function loneShot(paragraph: Element): Element | null {
  const parts = paragraph.children.filter((child) => !(child.type === "text" && child.value.trim() === ""));
  const [only] = parts;
  if (parts.length !== 1 || only.type !== "element" || only.tagName !== "a") return null;
  return Array.isArray(only.properties.className) && only.properties.className.includes("shot") ? only : null;
}

/**
 * A picture that stands alone in its paragraph is a figure: as wide as the
 * column, with the words that say what it shows under it. Those words are the
 * picture's own `alt`, which a screen reader has read already, so the caption
 * is hidden from it.
 */
export function shotFigure(shot: Element): Element {
  const img = shot.children.find((child): child is Element => child.type === "element" && child.tagName === "img");
  const alt = typeof img?.properties.alt === "string" ? img.properties.alt : "";
  const children: ElementContent[] = [shot];
  if (alt) children.push(el("figcaption", { ariaHidden: "true" }, [{ type: "text", value: alt }]));
  return el("figure", { className: ["shot-figure"] }, children);
}
