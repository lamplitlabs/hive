import fs from "node:fs";
import path from "node:path";
import type { CSSProperties, ReactNode } from "react";
import { Maximize2Icon } from "lucide-react";
import { docsRoot } from "@/lib/docs";
import { IMAGE_HEADER_BYTES, imageSize } from "@/lib/image-size";
import type { ImageSize } from "@/lib/image-size";
import { docMediaPath } from "@/lib/markdown";

/** The pictures of the console are the docs' own: docs/how-to/images/, which the build copies into the export. */
const SHOTS_DIR = "how-to/images";

/** Where the export serves one of those pictures. A name that is no picture of the docs fails the build here. */
export function shotSrc(name: string): string {
  const src = docMediaPath(`docs/${SHOTS_DIR}/${name}`);
  if (!src) throw new Error(`${name} is not a picture under docs/${SHOTS_DIR}/`);
  return src;
}

/** The picture's own size, read from the file, so the page keeps its place while it loads whatever shape a retake gives it. */
export function shotSize(name: string): ImageSize {
  const file = path.join(docsRoot(), SHOTS_DIR, name);
  const fd = fs.openSync(file, "r");
  try {
    const head = Buffer.alloc(IMAGE_HEADER_BYTES);
    const size = imageSize(head.subarray(0, fs.readSync(fd, head, 0, head.length, 0)));
    if (!size) throw new Error(`${file} is not a picture whose size can be read`);
    return size;
  } finally {
    fs.closeSync(fd);
  }
}

type Side = "left" | "right" | "top" | "bottom";
const SIDES: readonly Side[] = ["left", "right", "top", "bottom"];

/**
 * The part of a picture a place shows. `x` and `y` are its top left corner
 * and `w` its width, each as a part of the picture (0 to 1), so a crop reads
 * as the measure it was taken from: `{ x: 264 / 1440, ... }` starts at the
 * 264th of 1440 points. `ratio` is the shape of the place, width over height;
 * the height follows from it.
 *
 * A crop should start and end where something in the picture does: after the
 * sidebar, at a column's border, at a dialog's edge. `hard` names the sides
 * that do. Every other side that hides some of the picture fades out, so a
 * cut through a table reads as "the screen goes on" and not as a mistake.
 */
export type Crop = { x: number; y: number; w: number; ratio: number; hard?: readonly Side[] };

const clamp = (n: number, low: number, high: number) => Math.min(Math.max(n, low), high);
const pct = (n: number) => `${Math.round(n * 10000) / 100}%`;

/**
 * Where the picture sits in its place for one crop: its width and its left
 * edge in parts of the place's width, its top edge in parts of the place's
 * height (what CSS percentages mean there), and how much of the picture each
 * side hides. A picture never shows less than the whole place and never past
 * its own edge: a crop that asks for more is moved back inside.
 */
export function cropBox(size: ImageSize, crop: Crop): { width: number; left: number; top: number; hidden: Record<Side, number> } {
  const shape = size.width / size.height;
  // No wider than the picture, and no taller: a place of this shape that shows `w` of the width shows `w * shape / ratio` of the height.
  const w = clamp(crop.w, 0.02, Math.min(1, crop.ratio / shape));
  const h = (w * shape) / crop.ratio;
  const x = clamp(crop.x, 0, 1 - w);
  const y = clamp(crop.y, 0, 1 - h);
  return { width: 1 / w, left: -x / w, top: -y / h, hidden: { left: x, right: 1 - w - x, top: y, bottom: 1 - h - y } };
}

/** The sides of a crop that fade: the ones that hide some of the picture, but for those the crop calls a real edge. */
function fading(size: ImageSize, crop: Crop): Set<Side> {
  const { hidden } = cropBox(size, crop);
  return new Set(SIDES.filter((side) => hidden[side] > 0.004 && !crop.hard?.includes(side)));
}

/**
 * A picture of the console, linked to the picture at its own size.
 *
 * A screen of 1440 points drawn 600 wide is a grey texture: nobody reads it.
 * So a place smaller than the picture shows a part of it (`crop`), large
 * enough to read, and a phone may show a smaller part than a desk
 * (`cropSmall`, below 640 points). Without a crop the whole picture is shown.
 * One <img> either way: the stylesheet moves it (`.shot-view` in globals.css).
 */
export function ConsoleShot({
  name,
  alt,
  bar,
  crop,
  cropSmall,
  large,
  sizes,
  eager = false,
  className = "",
}: {
  name: string;
  alt: string;
  /** A line above the picture, such as the address the console is open at. */
  bar?: ReactNode;
  crop?: Crop;
  cropSmall?: Crop;
  /** The same picture at twice the pixels (a file under public/), for a place that draws it near its own width or wider. */
  large?: string;
  /** How wide the picture is drawn, as the `sizes` of an <img>: the browser picks between the two files by it. */
  sizes?: string;
  /** For the one picture a page opens with: fetched at once, not when it scrolls into view. */
  eager?: boolean;
  className?: string;
}) {
  const src = shotSrc(name);
  const size = shotSize(name);
  const whole: Crop = { x: 0, y: 0, w: 1, ratio: size.width / size.height };
  const wide = crop ?? whole;
  const small = cropSmall ?? wide;
  const [w, s] = [cropBox(size, wide), cropBox(size, small)];
  const [fadeWide, fadeSmall] = [fading(size, wide), fading(size, small)];
  const place = {
    "--shot-ratio": String(small.ratio),
    "--shot-width": pct(s.width),
    "--shot-left": pct(s.left),
    "--shot-top": pct(s.top),
    "--shot-ratio-wide": String(wide.ratio),
    "--shot-width-wide": pct(w.width),
    "--shot-left-wide": pct(w.left),
    "--shot-top-wide": pct(w.top),
  } as CSSProperties;
  return (
    // The link is named by its picture's words; the title says what a click does.
    <a href={large ?? src} title="Open the picture at full size" className={`shot-frame group ${className}`}>
      {bar && (
        <span className="flex items-center gap-2 border-b border-white/10 px-4 py-2 font-mono text-xs text-white/60">
          <span className="size-2 rounded-full bg-[#50f4c5]" aria-hidden="true" />
          <span className="truncate">{bar}</span>
        </span>
      )}
      <span className="shot-view" style={place}>
        {/* eslint-disable-next-line @next/next/no-img-element -- a static export has no image optimiser; the sizes offered are real files */}
        <img
          src={src}
          srcSet={large ? `${src} ${size.width}w, ${large} ${size.width * 2}w` : undefined}
          sizes={large ? sizes : undefined}
          width={size.width}
          height={size.height}
          alt={alt}
          loading={eager ? "eager" : "lazy"}
          fetchPriority={eager ? "high" : undefined}
          decoding="async"
        />
        {SIDES.filter((side) => fadeSmall.has(side) || fadeWide.has(side)).map((side) => (
          // One fade for a side, shown at the widths where that side cuts the picture.
          <span key={side} className={`shot-fade shot-fade-${side} ${fadeSmall.has(side) ? "" : "max-sm:hidden"} ${fadeWide.has(side) ? "" : "sm:hidden"}`} aria-hidden="true" />
        ))}
        <span className="shot-open" aria-hidden="true">
          <Maximize2Icon className="size-3.5" /> Full size
        </span>
      </span>
    </a>
  );
}
