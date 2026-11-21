// The landing page's questions. The answers say only what the README and
// docs/ say; the page shows them and the FAQPage JSON-LD carries the same
// words. Backticks mark what the page sets as code; `plainAnswer` drops them.

export type FaqEntry = {
  q: string;
  a: string;
  /** Where the long answer is. */
  more?: { label: string; href: string };
};

export const FAQS: readonly FaqEntry[] = [
  {
    q: "What is Hive?",
    a: "Hive is an open-source harness for coding agents. It is one binary (`hive`), one SQLite file (`~/.hive/hive.sqlite`) and one console at `http://127.0.0.1:8765`. You give a project's lead agent a task, the agents do the work in throwaway clones, and the result reaches you as pull requests.",
  },
  {
    q: "What does self-improving mean?",
    a: "Agents post suggestions and vote on each other's. A suggestion must name a check that fails on the default branch today and passes with the change. The loop builds the top voted suggestion on an experiment branch, measures it against the project's fixtures and against private tests the agents never see, archives the result and delivers what was kept as a pull request. All of it runs under a constitution that you own.",
    more: { label: "How the loop works", href: "/rsi/" },
  },
  {
    q: "Which models does Hive work with?",
    a: "GitHub Copilot by default. OpenAI, Anthropic, Gemini and a dozen OpenAI-compatible vendors take an API key, and local servers such as Ollama, LM Studio and vLLM need none.",
    more: { label: "Providers and the harness", href: "/harness/" },
  },
  {
    q: "What leaves my machine?",
    a: "The model calls and the git pushes you allowed. Nothing else. The store is one SQLite file under `~/.hive`, and the console listens on `127.0.0.1` only.",
  },
  {
    q: "What do I need to run it?",
    a: "macOS or Linux. To run agents you need a GitHub Copilot seat, or an API key for another provider. One command installs the binary; building from source needs Rust 1.95 or newer and Node.js 22 or newer.",
    more: { label: "Install Hive", href: "/install/" },
  },
  {
    q: "Does Hive run on Windows?",
    a: "Not today. Hive runs on macOS and Linux. A Windows port is in progress, and the documentation tracks how far it is.",
    more: { label: "The Windows port", href: "/docs/windows-port/" },
  },
  {
    q: "How do I add another machine?",
    a: "`hive pair ADMIN@HOST` pairs a Linux machine over SSH with one command. Any machine with the small `hive-hand` program can instead enrol itself with a six-word join code and dial out. A project says what its jobs need, such as `macos`, `xcode`, `gpu` or `docker`, and the queue sends each job to a machine that offers it.",
    more: { label: "Pairing machines", href: "/machines/" },
  },
  {
    q: "How do I stay in charge of what the agents do?",
    a: "Hard stops, money, big changes and questions marked for you wait under For you on Home until you answer. Each step must pass the project's own check, and code reaches you as pull requests that a review agent merges or hands to you. The constitution is yours: it holds the hard stops and the paths the loop may never touch, and `hive stop` is the kill switch.",
  },
];

/** An answer as plain text, for the structured data. */
export function plainAnswer(a: string): string {
  return a.replace(/`/g, "");
}
