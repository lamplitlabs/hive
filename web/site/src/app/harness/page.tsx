import type { Metadata } from "next";
import Link from "@/components/link";
import { Frame } from "@/components/chrome";
import { CodeBlock } from "@/components/code";
import { Article, H2, PageCrumbs, PageHero } from "@/components/page-hero";
import { pageMetadata } from "@/lib/seo";
import { PAGES } from "@/lib/site";

export const metadata: Metadata = pageMetadata(PAGES.harness);

const SECTIONS = [
  { id: "loop", title: "The tool loop" },
  { id: "tools", title: "The tools" },
  { id: "limits", title: "Limits" },
  { id: "deny", title: "Deny rules" },
  { id: "hooks", title: "Hooks" },
  { id: "mcp", title: "MCP servers" },
  { id: "skills", title: "Skills" },
  { id: "helpers", title: "Helpers and sub-sessions" },
  { id: "reads", title: "What a run reads first" },
  { id: "providers", title: "Providers" },
  { id: "chat", title: "Talk to it yourself" },
  { id: "reference", title: "The full reference" },
];

const TOOLS: [string, string][] = [
  ["bash", "one shell command, a fresh process each time; it can also start a server or a watcher in the background"],
  ["read", "one file, every line numbered"],
  ["grep", "a regular expression over the clone, a folder or a file"],
  ["glob", "the files whose path matches a pattern"],
  ["edit", "exact search and replace in one file"],
  ["write", "a whole new file"],
  ["plan", "the task's checklist: to do, doing, done"],
];

const LOOP = `system prompt (+ the clone's AGENTS.md) + the job's brief
  ─► the model answers with one or more tool calls
  ─► every call runs in the clone, in order
  ─► each result goes back as that call's observation
  ─► ... until the model answers with no call: that message is the final report`;

const HOOKS = `{"hooks": [
  {"event": "pre_tool",  "tools": ["bash"],          "command": "hive-guard --stdin", "timeout": 10},
  {"event": "post_tool", "tools": ["edit", "write"], "command": "npm run lint:changed"},
  {"event": "run_end",                               "command": "./scripts/after-run.sh"}
]}`;

export default function HarnessPage() {
  return (
    <Frame current={PAGES.harness.path}>
      <PageCrumbs page={PAGES.harness} />
      <PageHero
        eyebrow="The harness"
        title="How an agent runs."
        lead="Hive does not wrap another agent tool. Every job runs on its own harness: one model, a small set of typed tools, a linear history and limits you set. Nothing besides the hive binary is installed on a machine that runs agents."
      />

      <Article sections={SECTIONS}>
        <H2 id="loop">The tool loop</H2>
        <p>
          A run starts from a system prompt, the repository&apos;s own instructions and the job&apos;s brief. The model answers with tool calls. Each call runs
          in the job&apos;s throwaway clone, and its result goes back to the model as that call&apos;s observation. The run ends when the model answers with no
          call: that message is its final report.
        </p>
        <CodeBlock code={LOOP} lang="text" />
        <p>
          One reply may carry several calls. The ones that only look (<code>read</code>, <code>grep</code>, <code>glob</code>, <code>fetch</code>) run at the
          same time; the ones that change something run one after another, in the order the model gave. Every step is stored as it happens, so the console
          shows a run command by command while it is still going.
        </p>

        <H2 id="tools">The tools</H2>
        <p>Every run is offered the same seven typed tools. Each is confined to the clone and bound by the same deny rules and the same output cap.</p>
        <div className="table-scroll" role="region" aria-label="The seven tools" tabIndex={0}>
          <table>
            <thead>
              <tr>
                <th scope="col">Tool</th>
                <th scope="col">What it does</th>
              </tr>
            </thead>
            <tbody>
              {TOOLS.map(([name, what]) => (
                <tr key={name}>
                  <td>
                    <code>{name}</code>
                  </td>
                  <td>{what}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p>
          Beside them: <code>process</code> for the run&apos;s background processes, <code>memory</code> for what should be kept for later runs, and, where
          you allow them, <code>skill</code>, <code>fetch</code>, <code>web_search</code>, <code>subagent</code> and the tools of the project&apos;s MCP
          servers.
        </p>

        <H2 id="limits">Limits</H2>
        <p>Three budgets bound every run, and all three are settings you own.</p>
        <ul>
          <li>
            <code>harness.max_steps</code>: the model calls one run may make. 60 by default.
          </li>
          <li>
            <code>minutes_per_job</code>: the wall clock. The harness stops a minute before the runner would end the run, so it can still report.
          </li>
          <li>
            <code>harness.max_cost</code>: the cost the provider reports for one run. <code>0</code> means no cap.
          </li>
        </ul>
        <p>
          One command may run for <code>harness.step_timeout</code> seconds (300 by default) before it is ended. Output longer than{" "}
          <code>harness.output_limit</code> characters (20,000) is cut in the middle. When the conversation fills half of the model&apos;s context window (
          <code>harness.compact_at</code>, 50 percent), the harness has the model summarise the older part and carries on with the summary. A run that
          hits a limit still reports what it did.
        </p>

        <H2 id="deny">Deny rules</H2>
        <p>A job&apos;s harness refuses some things before they run, and tells the model why:</p>
        <ul>
          <li>
            Commands that would leave the clone: <code>git push</code>, a change to <code>git remote</code>, any <code>gh</code> call.
          </li>
          <li>
            Paths that hold credentials: <code>~/.ssh</code>, <code>.netrc</code>, Hive&apos;s own <code>private/</code> folder, <code>~/.copilot</code>,{" "}
            <code>~/.config/gh</code>.
          </li>
        </ul>
        <p>
          A run never holds a git login either. Agents commit in their clones and never push; the runner delivers the branch after the run has ended. You
          add rules of your own in the <code>tool(filter)</code> form:
        </p>
        <CodeBlock code={"hive settings set harness.deny_tools 'bash(rm *), read(.env*), write(*.lock)'"} />
        <p>
          Deny beats allow, and allow beats the default. A denied call is refused before anything runs, and a tool denied whole is not offered to the model
          at all. Jobs that only look, such as answers and ballots, run read-only: <code>edit</code> and <code>write</code> are not offered, and a
          command that would change the clone is refused.
        </p>

        <H2 id="hooks">Hooks</H2>
        <p>
          <code>.hive/hooks.json</code> in the repository and <code>~/.hive/hooks.json</code> name commands the harness runs at six moments of a run:{" "}
          <code>run_start</code>, <code>user_prompt</code>, <code>pre_tool</code>, <code>post_tool</code>, <code>pre_compact</code> and{" "}
          <code>run_end</code>.
        </p>
        <CodeBlock code={HOOKS} lang="json" title=".hive/hooks.json" />
        <p>
          A <code>pre_tool</code> hook that exits with 2 blocks the call, and the model sees the hook&apos;s words as the reason. What a{" "}
          <code>post_tool</code> hook prints is added to the observation, so a linter that runs after every edit speaks to the model in the same turn.
        </p>

        <H2 id="mcp">MCP servers</H2>
        <p>
          A run connects the project&apos;s enabled MCP servers when it starts and offers their tools to the model by name, as{" "}
          <code>mcp__&lt;server&gt;__&lt;tool&gt;</code>. Past 32 tools the run offers a search over them instead, so a large server does not fill the
          context. A server that does not answer within 45 seconds is skipped, and the run goes on without it.{" "}
          <Link href="/docs/board/#connecting-the-outside-world-hive-mcp">Connecting the outside world</Link> says how to register one.
        </p>

        <H2 id="skills">Skills</H2>
        <p>
          A skill is a folder with a <code>SKILL.md</code>. The harness finds skills in <code>.hive/skills</code>, <code>.claude/skills</code>,{" "}
          <code>.github/skills</code> and <code>.agents/skills</code> in the repository, then in <code>~/.hive/skills</code>. The model sees each name with
          its one-line description and loads a skill only when the task calls for it, so a project with forty skills costs the context a few hundred
          tokens, not forty documents.
        </p>

        <H2 id="helpers">Helpers and sub-sessions</H2>
        <p>
          A run may hand a self-contained sub-task to a helper through the <code>subagent</code> tool: a second agent loop on the same model and in the
          same clone, with a history of its own. Helpers go one level deep, and their calls count against the parent&apos;s budgets.
        </p>
        <p>
          A session in the console has a second shape of delegation. A sub-session is a session of its own, with its own page and budget, that you can open
          and steer while the parent carries on. It reports back to the parent when it has something to say.
        </p>

        <H2 id="reads">What a run reads first</H2>
        <p>
          A run reads your own <code>~/.hive/AGENTS.md</code>, then the clone&apos;s <code>AGENTS.md</code>, <code>CLAUDE.md</code>, <code>GEMINI.md</code>{" "}
          and <code>.github/copilot-instructions.md</code>, and the rule files under <code>.hive/rules</code>, <code>.claude/rules</code> and{" "}
          <code>.github/instructions</code>. A rule that is scoped to a path arrives the first time the model touches a file it covers.
        </p>
        <p>
          What a run learns the hard way it can keep with the <code>memory</code> tool: a plain Markdown file per project under{" "}
          <code>~/.hive/memory/</code>, which the next run in that project reads.
        </p>

        <H2 id="providers">Providers</H2>
        <p>
          GitHub Copilot is the default provider and logs in with GitHub&apos;s device flow. The hosted ones take an API key: <code>openai</code>,{" "}
          <code>anthropic</code>, <code>gemini</code>, and the OpenAI-compatible vendors <code>openrouter</code>, <code>groq</code>, <code>mistral</code>,{" "}
          <code>deepseek</code>, <code>xai</code>, <code>together</code>, <code>fireworks</code>, <code>cerebras</code>, <code>github-models</code> and{" "}
          <code>azure-openai</code>. The local servers <code>ollama</code>, <code>lmstudio</code> and <code>vllm</code> need no key, and{" "}
          <code>custom</code> points at any other OpenAI-compatible server.
        </p>
        <CodeBlock
          code={[
            "hive harness login                                  # GitHub's device flow, for the default provider",
            "hive settings set provider openai                   # or anthropic, gemini, ollama, custom, ...",
            "hive harness login --provider openai --with-token   # reads the key on stdin",
            "hive harness models                                 # the models the provider offers this login",
            "hive harness status                                 # the provider, the login, the models and limits in force",
          ].join("\n")}
        />
        <p>
          Three settings choose the models: <code>worker_model</code> for the runs that change code, <code>light_model</code> for the light runs (the lead,
          answers, ballots) and <code>measure_model</code> for the loop&apos;s measurements. To send a provider through a proxy, a gateway or a local
          server, set <code>provider.&lt;name&gt;.base_url</code>.
        </p>

        <H2 id="chat">Talk to it yourself</H2>
        <p>
          <code>hive harness chat</code> is the same agent as a conversation in your terminal, with the same tools, instruction files, skills, hooks, MCP
          servers and deny rules as a job. Each line you type is a turn. A call that would change something is shown first, and you answer yes, no or
          always.
        </p>
        <CodeBlock code="hive harness chat -C ~/src/my-app" />

        <H2 id="reference">The full reference</H2>
        <ul>
          <li>
            <Link href="/docs/harness/">The harness</Link>: every tool, budget, flag and provider, with the reasons behind the defaults.
          </li>
          <li>
            <Link href="/docs/board/">The board</Link>: the commands agents use, MCP servers, shared skills, hooks and events.
          </li>
          <li>
            <Link href="/docs/console/#sessions">Sessions in the console</Link>: conversations with an agent, and their sub-sessions.
          </li>
        </ul>
      </Article>
    </Frame>
  );
}
