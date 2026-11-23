// A diagram drawn in text: a code block of a record, a brief or a message
// whose lines are made of box-drawing characters. The agents draw their
// decision records this way (TPL-adr asks for one under "At a glance"), since
// a record lives in the store and has nowhere to keep a picture file. The
// console closes the lines up and hands a screen reader the words.
//
// web/site/src/lib/diagram.ts holds the same two rules for the docs pages,
// with the checker a writer runs; keep the two in step.

const BOX_DRAWING = /[\u2500-\u257F]/;

/** Whether a fenced block is a diagram: plain text (no language, or `text`) that holds a box-drawing character. */
export function isDiagram(text: string, lang = ""): boolean {
  return (lang === "" || lang === "text") && BOX_DRAWING.test(text);
}

/** The words of a diagram in reading order, each once: what a screen reader gets instead of the line characters. */
export function diagramWords(text: string): string[] {
  const seen = new Set<string>();
  const words: string[] = [];
  for (const line of text.split("\n")) {
    for (const part of line.split(/[\u2500-\u257F\u2190-\u2193]+| {2,}/)) {
      const word = part.trim();
      if (word && !seen.has(word)) {
        seen.add(word);
        words.push(word);
      }
    }
  }
  return words;
}

/** The name of a diagram for a screen reader. */
export function diagramLabel(text: string): string {
  const words = diagramWords(text);
  return words.length ? `Diagram: ${words.join(", ")}` : "Diagram";
}
