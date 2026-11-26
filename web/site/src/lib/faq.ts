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
    q: "What does Hive improve, and how?",
    a: "Every project Hive works on, and Hive's own source, which is one such project. Agents post suggestions and vote on each other's. A suggestion must name a check that fails on the default branch today and passes with the change, and the runner builds a voted suggestion as an ordinary job that must turn that check from failing to passing. Measured experiments are off until you turn them on (`evolve.loop`). With the loop on, it builds the top voted suggestion on an experiment branch, measures it against the project's fixtures and against private tests kept outside every repository, archives the result and delivers what was kept as a pull request. All of it runs under a constitution that you own.",
    more: { label: "How the loop works", href: "/rsi/" },
  },
  {
    q: "Which models does Hive work with?",
    a: "GitHub Copilot by default. OpenAI, Anthropic, Gemini and ten OpenAI-compatible vendors take an API key, and local servers such as Ollama, LM Studio and vLLM need none.",
    more: { label: "Providers and the harness", href: "/harness/" },
  },
  {
    q: "What leaves my machine?",
    a: "Hive has no server of its own. In ordinary use the machine talks to the model provider you chose, to your git host for fetches, pushes and pull requests (through `gh`), and to package registries for the release dates of dependencies. A job's own commands reach whatever they reach: an agent's shell commands and the project's build and check run with the network open. A session with an agent is offered web search and sends the search words to DuckDuckGo; jobs do not search unless you set `harness.web_search` to `on`, and `off` turns it off everywhere. Backups, the daily check for new releases, fetching web pages, rented machines and store hooks with a URL stay off until you turn them on. The store is one SQLite file under `~/.hive`, and the console listens on `127.0.0.1` only.",
    more: { label: "Security notes", href: "/docs/security/" },
  },
  {
    q: "What do I need to run it?",
    a: "macOS or Linux; on Windows, WSL. A model: a GitHub Copilot seat, an API key for another provider, or a local server. To deliver work as pull requests: `git`, and the GitHub CLI (`gh`) logged in as a GitHub user who can see the repository. Hive opens, reads and merges pull requests through `gh`. One command installs the binary; building from source needs Rust 1.95 or newer and Node.js 22 or newer.",
    more: { label: "Install Hive", href: "/install/" },
  },
  {
    q: "Does Hive run on Windows?",
    a: "Yes, inside WSL, the Linux that Windows runs. The install line is the same there, the console opens in your Windows browser, and the services run under the Linux's systemd. Hive does not run on Windows itself yet: that port compiles, and the documentation tracks how far it is.",
    more: { label: "Run Hive on Windows", href: "/docs/how-to/windows/" },
  },
  {
    q: "How do I add another machine?",
    a: "`hive pair ADMIN@HOST --name NAME` sets up a Linux machine over the SSH login of an admin there. It copies a Linux `hive` to the machine, and the machine takes work once it has a model login of its own. Any machine with the small `hive-hand` program can instead enrol itself with a six-word join code and dial out; it reaches your console through an SSH tunnel or a proxy that you set up. A project says what its jobs need, such as `macos`, `xcode`, `gpu` or `docker`, and the queue sends each job to a machine that offers it.",
    more: { label: "Pairing machines", href: "/machines/" },
  },
  {
    q: "How do I stay in charge of what the agents do?",
    a: "Hard stops - money, what cannot be undone, credentials, publication - wait for your answer on Sessions, where the console opens, and every question marked for you waits on its project's For you tab. One question does not wait for ever: when the loop pauses because its fixtures tell nothing apart and a day passes with no answer, it answers that question itself. Each step must pass the project's own check, and code reaches you as pull requests that a review agent merges or hands to you. The constitution is yours: it holds the hard stops and the paths the loop may never touch, and `hive stop` halts the runner. These are rules the agents are held to, not a sandbox: a job runs as your user, and the event log records who changed what.",
  },
];

/** An answer as plain text, for the structured data. */
export function plainAnswer(a: string): string {
  return a.replace(/`/g, "");
}
