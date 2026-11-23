import { docMediaPath } from "@/lib/markdown";

/** The pictures of the console are the docs' own: docs/how-to/images/, which the build copies into the export. */
export const SHOT_SIZE = { width: 1440, height: 900 } as const;

/** Where the export serves one of those pictures. A name that is no picture of the docs fails the build here. */
export function shotSrc(name: string): string {
  const src = docMediaPath(`docs/how-to/images/${name}`);
  if (!src) throw new Error(`${name} is not a picture under docs/how-to/images/`);
  return src;
}

/**
 * A picture of the console in a window frame that names the page, linked to
 * the picture at its own size. `large` is the same picture at twice the pixels, for a wide place
 * on a dense screen. `zoom` is for a small place: it shows the page's own
 * part from its top left corner, without the sidebar, so the words can be
 * read. A plain <img>: the export has no image service, and the two sizes
 * are files that exist.
 */
export function ConsoleShot({
  name,
  alt,
  title,
  large,
  sizes,
  zoom = false,
  className = "",
}: {
  name: string;
  alt: string;
  title: string;
  large?: string;
  sizes?: string;
  zoom?: boolean;
  className?: string;
}) {
  const src = shotSrc(name);
  return (
    // The link is named by what it shows: the page's name in the frame, then the picture's own words.
    <a
      href={large ?? src}
      title="Open the picture at full size"
      className={`night block overflow-hidden rounded-2xl border border-border bg-[#070b1a] shadow-pop transition-colors hover:border-teal ${className}`}
    >
      <span className="flex items-center gap-2 border-b border-border px-4 py-2.5">
        <span className="size-2.5 rounded-full bg-white/15" aria-hidden="true" />
        <span className="size-2.5 rounded-full bg-white/15" aria-hidden="true" />
        <span className="size-2.5 rounded-full bg-white/15" aria-hidden="true" />
        <span className="ml-2 truncate font-mono text-xs text-muted-foreground">{title}</span>
      </span>
      {/* The frame keeps the picture's shape; a zoomed picture is wider than it and starts right of the console's sidebar. */}
      <span className="block overflow-hidden" style={{ aspectRatio: `${SHOT_SIZE.width} / ${SHOT_SIZE.height}` }}>
        {/* eslint-disable-next-line @next/next/no-img-element -- a static export has no image optimiser; the sizes offered are real files */}
        <img
          src={src}
          srcSet={large ? `${src} ${SHOT_SIZE.width}w, ${large} ${SHOT_SIZE.width * 2}w` : undefined}
          sizes={large ? sizes : undefined}
          width={SHOT_SIZE.width}
          height={SHOT_SIZE.height}
          alt={alt}
          loading="lazy"
          decoding="async"
          className={zoom ? "-ml-[32%] block h-auto w-[170%] max-w-none" : "block h-auto w-full"}
        />
      </span>
    </a>
  );
}
