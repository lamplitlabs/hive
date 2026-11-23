import NextLink from "next/link";
import type { ComponentProps } from "react";

/**
 * A link to another page of this site. A click navigates without a full
 * reload; nothing is fetched before the click. Next would otherwise fetch
 * every linked page as its link scrolls into view, and a docs page links to
 * twenty others.
 */
export default function Link(props: ComponentProps<typeof NextLink>) {
  return <NextLink prefetch={false} {...props} />;
}
