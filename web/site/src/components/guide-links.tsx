import Link from "@/components/link";
import { H2 } from "@/components/page-hero";
import { GUIDES_DIR, requireDoc } from "@/lib/docs";

/** What the "On this page" list of a page calls this section. */
export const GUIDES_SECTION = { id: "guides", title: "Step by step" } as const;

/**
 * The guides that take a reader through what a page describes: each by its
 * title, with the sentence it opens with. The names are files of
 * docs/how-to/ without `.md`; one that is gone fails the build by name.
 */
export function GuideLinks({ names }: { names: readonly string[] }) {
  const guides = names.map((name) => requireDoc(`${GUIDES_DIR}/${name}`));
  return (
    <>
      <H2 id={GUIDES_SECTION.id}>{GUIDES_SECTION.title}</H2>
      <p>This page says how it works. The guides say what to click and what to type:</p>
      <ul>
        {guides.map((d) => (
          <li key={d.path}>
            <Link href={d.path}>{d.title}</Link>. {d.description}
          </li>
        ))}
      </ul>
    </>
  );
}
