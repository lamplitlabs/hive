use crate::store::{HUMAN_AGENT, Store, input_err};
use anyhow::Result;
use regex::Regex;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::LazyLock;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingRule {
    pub kind: String,
    pub min: i64,
    pub has_min: bool,
    pub choices: Vec<String>,
    pub max: f64,
    pub has_max: bool,
    pub description: String,
}
impl SettingRule {
    fn new(
        kind: &str,
        min: Option<i64>,
        choices: &[&str],
        max: Option<f64>,
        description: &str,
    ) -> Self {
        Self {
            kind: kind.into(),
            min: min.unwrap_or(0),
            has_min: min.is_some(),
            choices: choices.iter().map(|s| (*s).into()).collect(),
            max: max.unwrap_or(0.0),
            has_max: max.is_some(),
            description: description.into(),
        }
    }
}

macro_rules! map { ($($k:literal => $v:literal,)+) => {{ let mut m=BTreeMap::new(); $(m.insert($k.to_string(), $v.to_string());)+ m }} }
macro_rules! rules { ($($k:literal => ($kind:literal, $min:expr, [$($c:literal),*], $max:expr, $d:literal),)+) => {{ let mut m=BTreeMap::new(); $(m.insert($k.to_string(), SettingRule::new($kind, $min, &[$($c),*], $max, $d));)+ m }} }

/// Settings earlier releases wrote that no longer mean anything: the
/// `copilot-cli` run mode and what it alone read. `ensure_records` deletes
/// their rows so a store that set them lists nothing it cannot explain.
pub const RETIRED_SETTINGS: &[&str] = &["harness", "max_ai_credits", "runner_cmd"];

/// Concurrency settings the capacity model now works out from the machines
/// (hive-runner capacity.rs): a stored value is swept and ignored, and
/// setting one says it is automatic now.
pub const AUTOMATIC_SETTINGS: &[&str] = &[
    "workers",
    "parallel_per_project",
    "daemon.max_jobs",
    "daemon.max_load",
    "daemon.min_free_mem_mb",
];

/// What the `runner` column of an agent the runner spawns says: every
/// worker runs through the built-in harness.
pub const HARNESS_RUNNER: &str = "hive";

pub static DEFAULT_SETTINGS: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
    map! {
    "amend_majority"=>"2/3","amend_quorum"=>"40","chat.idle_release_minutes"=>"15","chat.resume_after_provider_error_secs"=>"90","chat.sub_depth"=>"2","chat.sub_open"=>"8","claim_hours"=>"4","expiry_days"=>"14","jobs_per_day"=>"2","inbox.retire_days"=>"7","landing"=>"agents","minutes_per_job"=>"10","prune_after_days"=>"14","quorum"=>"2","records.auto_adopt"=>"0","stale_days"=>"30","stop"=>"0","voice.language"=>"auto","voice.model"=>"nemotron-3.5-asr-streaming-0.6b","worker_model"=>"","capacity"=>"auto",
    }
});
pub static SETTING_RULES: LazyLock<BTreeMap<String, SettingRule>> = LazyLock::new(|| {
    rules! {
        "jobs_per_day"=>("count-or-none",Some(0),[],None,"distinct jobs the fleet may start per UTC day on every executor together, a hard stop until midnight once reached; the owner's own tasks, answers and lead sittings run past it; 0 or none means no daily cap"),
        "chat.idle_release_minutes"=>("count",Some(0),[],None,"minutes a conversation may sit idle (no turn running, nothing queued, no question open) before the hub lets its agent go: the session's provider and MCP connections and their child processes close, its trajectory stays on disk and the next message resumes the conversation from it; 0 keeps every agent for as long as the hub runs"),
        "chat.resume_after_provider_error_secs"=>("count",Some(0),[],None,"seconds a conversation waits before it resumes a turn a transport provider_error (a failed send or a broken reply body) cut short, once per failure; 0 leaves the conversation idle for the next message as before"),
        "chat.sub_depth"=>("count",Some(0),[],None,"levels deep a conversation may start sub-conversations: at 2 a conversation's sub-conversations may start their own and those may not; 0 sets no depth limit (each level is a session of its own, and a brief that tells its sub-conversation to delegate the same way runs on until a step or cost budget stops it)"),
        "chat.sub_open"=>("count",Some(0),[],None,"sub-conversations one conversation may have open (not finished or archived) at a time, each a live session sharing the parent's worktree; 0 sets no limit"),
        "voice.model"=>("choice",None,["nemotron-3.5-asr-streaming-0.6b","nemotron-speech-streaming-en-0.6b"],None,"the speech model the hub's microphone transcribes with, run on this machine through ONNX Runtime (no audio leaves it): nemotron-3.5-asr-streaming-0.6b understands 25+ languages and picks the one spoken (or takes voice.language); nemotron-speech-streaming-en-0.6b is English only and a little lighter. Either is about 2.5 GB, fetched once into HIVE_HOME/voice by the microphone's first use or `hive voice setup`"),
        "voice.language"=>("text",None,[],None,"the language the multilingual speech model is told to expect: auto lets it pick from the audio; a tag such as en-US, de-DE, ja-JP or zh-CN fixes it, which is more accurate when you always speak one language. The English model ignores this"),
        "inbox.retire_days"=>("count",Some(0),[],None,"days before daemon maintenance may close an open owner question only when it is an older duplicate of a newer open question or its job was cancelled; 0 turns the owner-inbox retire sweep off"),
        "minutes_per_job"=>("count",Some(0),[],None,"wall-clock minutes one worker run may take before the runner stops it"),
        "capacity"=>("text",None,[],None,"how many runs this machine takes at once: auto (the default) works it out from the hardware - the smaller of the physical cores and one run per 3 GB of RAM - and backs off live under load or low memory; a number caps this machine at that many (0 keeps it idle while remote hands work); every paired machine sizes itself the same way and the fleet adds them up. To pause the fleet use hive stop"),
        "prune_after_days"=>("count",Some(1),[],None,"days without useful output after which an agent is pruned (its history stays)"),
        "claim_hours"=>("count",Some(1),[],None,"hours a board claim stays active before it is treated as stale"),
        "quorum"=>("count",Some(1),[],None,"distinct voters a suggestion needs before it is eligible for the queue"),
        "expiry_days"=>("count",Some(1),[],None,"days a suggestion may wait for votes before it expires"),
        "stale_days"=>("count",Some(1),[],None,"days after which an untouched thread counts as stale"),
        "amend_quorum"=>("count",Some(1),[],None,"distinct sessions an amendment needs; may be raised by vote, lowered only by a human"),
        "amend_majority"=>("fraction",None,[],None,"share of counted votes an amendment needs, as a fraction such as 2/3"),
        "worker_model"=>("text",None,[],None,"the model of the runs that change code - task, improve and experiment jobs - of the loop's measurements while measure_model is empty, and of every other run while light_model is empty; empty means the runner's default"),
        "stop"=>("switch",None,["0","1"],None,"the kill switch: 1 stops the runner after the runs in flight finish (hive stop / hive start)"),
        "landing"=>("choice",None,["agents","human"],None,"agents: verified branches merge into the default branch by themselves; human: they wait for you (ADR-024)"),
        "records.auto_adopt"=>("switch",None,["0","1"],None,"1: while the owner is away the daemon adopts waiting constitution and evaluator drafts in its own name - the revision note and a record.auto_adopted event name the daemon and this setting, and the owner can revoke it with hive record amend; 0 (default): drafts wait for the owner (ADR-046)"),
    }
});

pub const IMPLEMENTATION_JOB_KINDS: &[&str] = &["task", "improve", "experiment"];
/// The job kinds whose runs look but do not touch: the harness runs them
/// read-only (no edit or write tool, commands that would change the clone
/// refused, a change that slips through put back).
pub const READ_ONLY_JOB_KINDS: &[&str] = &["answer", "ballot"];
pub static MODEL_SETTINGS: LazyLock<BTreeMap<String, String>> = LazyLock::new(
    || map! {"light_model"=>"","measure_model"=>"","ask.model"=>"","decide_model"=>"","decide_provider"=>"","decide.mode"=>"on","decide.timeout_seconds"=>"30","worker_reasoning"=>"","light_reasoning"=>"","measure_reasoning"=>"",},
);
pub static MODEL_RULES: LazyLock<BTreeMap<String, SettingRule>> = LazyLock::new(|| {
    rules! {
        "light_model"=>("text",None,[],None,"the model of the light runs - lead, answer, ballot, reflect, onboard and clean jobs, the sealed held-out call and the login check; empty means worker_model"),
        "measure_model"=>("text",None,[],None,"the model the self-improvement loop measures fixtures with - every baseline, experiment and landing re-measure; empty means worker_model. It is part of the evaluator: a change re-measures every baseline, and a pinned value keeps the baselines when worker_model changes"),
        "decide_model"=>("text",None,[],None,"the model of the decide role (ADR-072): one call with no tools that answers yes, no or pass on a routine ask the lead may decide; empty means measure_model, then light_model"),
        "decide_provider"=>("choice",None,["","copilot","openai","anthropic","gemini","azure-openai","openrouter","groq","mistral","deepseek","xai","together","fireworks","cerebras","github-models","ollama","lmstudio","vllm","custom"],None,"the model provider of the decide role, one of the provider choices; empty means provider, the hub's own. Only the decide role reads it: runs that change code and reviews stay on provider"),
        "decide.mode"=>("choice",None,["off","shadow","on"],None,"the decide role (ADR-072): on - each daemon pass asks decide_model for a yes, no or pass on every open routine ask the lead may decide, and a yes or a no is decided under the lead's name; shadow - the answer is recorded as a decide.shadow event and the lead still decides; off - no call, the lead decides as before. A pass, an error, a late or unreadable answer leaves the ask to the lead"),
        "decide.timeout_seconds"=>("count",Some(1),[],None,"seconds the decide role waits for its answer before the ask is left to the lead (the request deadline)"),
        "ask.model"=>("text",None,[],None,"the model Ask answers the owner's questions with (console Ask, hive ask); empty means light_model, then worker_model, then the provider's default chat model"),
        "worker_reasoning"=>("choice",None,["","default","minimal","low","medium","high","xhigh","max"],None,"the reasoning effort of the runs that change code - task, improve and experiment jobs - and of a worker-class agent; empty means harness.reasoning; default sends nothing and leaves the model's own setting"),
        "light_reasoning"=>("choice",None,["","default","minimal","low","medium","high","xhigh","max"],None,"the reasoning effort of the light runs - lead, answer, ballot, reflect, onboard and clean jobs - and of a light-class agent; empty means harness.reasoning; default sends nothing"),
        "measure_reasoning"=>("choice",None,["","default","minimal","low","medium","high","xhigh","max"],None,"the reasoning effort the self-improvement loop measures fixtures with; empty means worker_reasoning, then harness.reasoning; default sends nothing. Like measure_model it is part of the evaluator: a change re-measures every baseline"),
    }
});
pub static HARNESS_SETTINGS: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
    map! {
    "provider"=>"copilot","provider.retry_after_cap_s"=>"60","provider.max_body_mb"=>"8","harness.max_steps"=>"60","harness.task_max_steps"=>"","harness.max_cost"=>"0","harness.keep_observations"=>"5","harness.system_prompt"=>"","harness.step_timeout"=>"300","harness.commit_timeout"=>"1500","harness.output_limit"=>"20000","harness.format_retries"=>"1","harness.context_window"=>"0","harness.instructions"=>"on","harness.mcp_tools"=>"on","harness.skills"=>"on","harness.hooks"=>"on","harness.subagents"=>"on","harness.memory"=>"on","harness.tasks"=>"on","harness.teach"=>"off","harness.web_search"=>"chat","harness.fetch_allow"=>"","harness.compact_at"=>"50","harness.compact_model"=>"","harness.trim_keep_recent"=>"8","harness.trim_over"=>"4000","provider.max_request_mb"=>"1","harness.reasoning"=>"default","harness.allow_tools"=>"","harness.deny_tools"=>"","models.allowed"=>"",
    // One endpoint per API-key provider (the registry name is the middle
    // segment); azure-openai has no default because the resource is the owner's.
    "provider.openai.base_url"=>"https://api.openai.com/v1","provider.anthropic.base_url"=>"https://api.anthropic.com","provider.gemini.base_url"=>"https://generativelanguage.googleapis.com","provider.azure-openai.base_url"=>"","provider.openrouter.base_url"=>"https://openrouter.ai/api/v1","provider.groq.base_url"=>"https://api.groq.com/openai/v1","provider.mistral.base_url"=>"https://api.mistral.ai/v1","provider.deepseek.base_url"=>"https://api.deepseek.com/v1","provider.xai.base_url"=>"https://api.x.ai/v1","provider.together.base_url"=>"https://api.together.xyz/v1","provider.fireworks.base_url"=>"https://api.fireworks.ai/inference/v1","provider.cerebras.base_url"=>"https://api.cerebras.ai/v1","provider.github-models.base_url"=>"https://models.github.ai/inference","provider.ollama.base_url"=>"http://127.0.0.1:11434/v1","provider.lmstudio.base_url"=>"http://127.0.0.1:1234/v1","provider.vllm.base_url"=>"http://127.0.0.1:8000/v1","provider.custom.base_url"=>"","provider.custom.no_key"=>"off","provider.custom.api_key_header"=>"",
    // The Entra tenant the azure-openai device login signs in to.
    "provider.azure-openai.tenant"=>"organizations",
    }
});
pub static HARNESS_RULES: LazyLock<BTreeMap<String, SettingRule>> = LazyLock::new(|| {
    rules! {
        "provider"=>("choice",None,["copilot","openai","anthropic","gemini","azure-openai","openrouter","groq","mistral","deepseek","xai","together","fireworks","cerebras","github-models","ollama","lmstudio","vllm","custom"],None,"the model provider of the native harness; copilot and github-models sign in with GitHub and azure-openai with Microsoft (a device code: hive harness login --provider <name> or the console's Harness card); every provider but copilot also takes an API key - pasted in the Harness card or read by hive harness login --provider <name> --with-token - or borrows it from the environment; ollama, lmstudio and vllm need none; custom is any other OpenAI-compatible server, described by provider.custom.base_url, provider.custom.no_key and provider.custom.api_key_header; the login lives on this machine, never in the store"),
        "provider.openai.base_url"=>("text",None,[],None,"the OpenAI-compatible endpoint the openai provider calls: OpenAI itself or any server that speaks its chat completions API; OPENAI_BASE_URL in the environment stands in while this keeps its default; the other vendors have their own provider.<name>.base_url"),
        "provider.anthropic.base_url"=>("text",None,[],None,"the Anthropic Messages API endpoint the anthropic provider calls; ANTHROPIC_BASE_URL in the environment stands in while this keeps its default"),
        "provider.gemini.base_url"=>("text",None,[],None,"the Google Gemini generateContent endpoint the gemini provider calls; GEMINI_BASE_URL in the environment stands in while this keeps its default"),
        "provider.azure-openai.base_url"=>("text",None,[],None,"the Azure OpenAI resource the azure-openai provider calls, as https://<resource>.openai.azure.com/openai/v1 (a bare resource root gets /openai/v1 added; a key goes in the api-key header, a Microsoft sign-in as a bearer token); AZURE_OPENAI_ENDPOINT in the environment stands in while this is empty; there is no default - the resource is yours"),
        "provider.azure-openai.tenant"=>("text",None,[],None,"the Microsoft Entra tenant the azure-openai provider's Microsoft sign-in (device code) uses: organizations for any work or school account, common, consumers, or a tenant id or domain; AZURE_OPENAI_TENANT_ID or AZURE_TENANT_ID in the environment stands in while this keeps its default; the account needs the Cognitive Services OpenAI User role on the resource"),
        "provider.openrouter.base_url"=>("text",None,[],None,"the OpenRouter endpoint the openrouter provider calls; OPENROUTER_BASE_URL in the environment stands in while this keeps its default"),
        "provider.groq.base_url"=>("text",None,[],None,"the Groq OpenAI-compatible endpoint the groq provider calls; GROQ_BASE_URL in the environment stands in while this keeps its default"),
        "provider.mistral.base_url"=>("text",None,[],None,"the Mistral endpoint the mistral provider calls; MISTRAL_BASE_URL in the environment stands in while this keeps its default"),
        "provider.deepseek.base_url"=>("text",None,[],None,"the DeepSeek endpoint the deepseek provider calls; DEEPSEEK_BASE_URL in the environment stands in while this keeps its default"),
        "provider.xai.base_url"=>("text",None,[],None,"the xAI Grok endpoint the xai provider calls; XAI_BASE_URL in the environment stands in while this keeps its default"),
        "provider.together.base_url"=>("text",None,[],None,"the Together AI endpoint the together provider calls; TOGETHER_BASE_URL in the environment stands in while this keeps its default"),
        "provider.fireworks.base_url"=>("text",None,[],None,"the Fireworks AI inference endpoint the fireworks provider calls; FIREWORKS_BASE_URL in the environment stands in while this keeps its default"),
        "provider.cerebras.base_url"=>("text",None,[],None,"the Cerebras endpoint the cerebras provider calls; CEREBRAS_BASE_URL in the environment stands in while this keeps its default"),
        "provider.github-models.base_url"=>("text",None,[],None,"the GitHub Models inference endpoint the github-models provider calls with a GitHub personal access token that has the models:read scope (GITHUB_MODELS_TOKEN, GH_TOKEN or GITHUB_TOKEN when none is pasted); GITHUB_MODELS_BASE_URL in the environment stands in while this keeps its default"),
        "provider.ollama.base_url"=>("text",None,[],None,"the local Ollama server the ollama provider calls, at its OpenAI-compatible /v1; OLLAMA_HOST or OLLAMA_BASE_URL in the environment stands in while this keeps its default; no key needed"),
        "provider.lmstudio.base_url"=>("text",None,[],None,"the local LM Studio server the lmstudio provider calls; LMSTUDIO_BASE_URL in the environment stands in while this keeps its default; no key needed"),
        "provider.vllm.base_url"=>("text",None,[],None,"the local vLLM server the vllm provider calls; VLLM_BASE_URL in the environment stands in while this keeps its default; no key needed"),
        "provider.custom.base_url"=>("text",None,[],None,"the OpenAI-compatible endpoint the custom provider calls, as https://<host>/v1 - a gateway, a proxy or a server none of the named vendors covers; HIVE_CUSTOM_BASE_URL in the environment stands in while this is empty; there is no default - the server is yours; the key is pasted in the Harness card, read by hive harness login --provider custom --with-token, or HIVE_CUSTOM_API_KEY in the environment"),
        "provider.custom.no_key"=>("choice",None,["on","off"],None,"on: the custom provider calls its server without a key, the way ollama, lmstudio and vllm do; off: a key is required and a call without one fails before it is made"),
        "provider.custom.api_key_header"=>("text",None,[],None,"the header the custom provider's key travels in, raw: for a server that wants x-api-key or api-key instead of Authorization: Bearer <key>; empty or Authorization keeps the bearer convention"),
        "provider.retry_after_cap_s"=>("count",Some(0),[],None,"maximum seconds a provider Retry-After or rate-limit reset header may hold one retry before jitter; the backoff still floors the wait"),
        "provider.max_body_mb"=>("count",Some(1),[],None,"maximum MiB read from a successful provider response body before the call fails as non-retryable"),
        "harness.max_steps"=>("count-or-none",Some(1),[],None,"model calls one native-harness run (or one chat turn) may make before it stops; none/off/unlimited lifts the cap and leaves the run to its wall-time budget, its cost cap and the context window, which compaction keeps it under"),
        "harness.task_max_steps"=>("count-or-none",None,[],None,"model calls a task, improve or experiment run may make before it stops, in the shape of the per-kind model settings; none/off/unlimited (or the empty default) leaves these kinds on harness.max_steps, which still covers every other kind"),
        "harness.max_cost"=>("number",Some(0),[],None,"provider-reported cost one native-harness run may spend before it stops (0 = no cap); the unit is the provider's - Copilot reports usage.cost only when the vendor sends it"),
        "harness.keep_observations"=>("count",Some(1),[],None,"observations kept verbatim in the model's context once the prompt is past the compaction threshold and compaction has not made room; older ones collapse to their first and last lines, and nothing collapses while the prompt is under the threshold"),
        "harness.system_prompt"=>("text",None,[],None,"Go text/template for the harness system prompt; empty = the built-in one (hive harness prompt --default prints it)"),
        "harness.step_timeout"=>("count",Some(10),[],None,"seconds one command of a native-harness run may take before it is killed"),
        "harness.commit_timeout"=>("count",Some(60),[],None,"seconds a git commit, merge, rebase, cherry-pick or am that runs the clone's pre-commit verify gate may take before it is killed; never below harness.step_timeout and capped by the run's remaining minutes"),
        "harness.output_limit"=>("count",Some(1000),[],None,"characters of one command's output kept for the model; longer output is cut in the middle"),
        "harness.format_retries"=>("count",Some(0),[],None,"prose-only native-harness replies corrected without consuming harness.max_steps before later format errors count normally"),
        "harness.context_window"=>("count",Some(0),[],None,"tokens of context the native harness assumes the model takes: harness.compact_at is a share of it and past 80% of it older observations collapse harder; 0 reads the window from the provider's model list (1M for the Claude and Gemini models that take it, Copilot's per-model limit), and a model that lists none is taken as 128000"),
        "harness.instructions"=>("choice",None,["on","off"],None,"on: a native-harness run reads the owner's ~/.hive/AGENTS.md and the clone's repository instruction files - AGENTS.md, CLAUDE.md, GEMINI.md, .github/copilot-instructions.md, the rules under .hive/rules, .claude/rules and .github/instructions - into its system prompt (each once, 16 KB per file, 32 KB in all; hive harness prompt -C <clone> shows the result), and hands path-scoped rules and nested AGENTS.md files to the model as it touches the files they cover; off: the task alone speaks for the repository"),
        "harness.mcp_tools"=>("choice",None,["on","off"],None,"on: a native-harness run with a store connects the project's enabled MCP servers (hive mcp) at its start and offers their tools to the model as mcp__<server>__<tool>; past 32 tools it offers mcp_search and loads what the model looks up; a server that does not answer within 45 s is noted and skipped; off: workers call servers through hive mcp call only"),
        "harness.skills"=>("choice",None,["on","off"],None,"on: a native-harness run offers the skill tool, which loads one skill's SKILL.md (and its support files) when the model asks: the clone's .hive/skills, .claude/skills, .github/skills and .agents/skills folders, the owner's ~/.hive/skills, and the enabled hive skill registry entries; only the names and descriptions sit in the prompt; off: skills reach the model through the brief alone"),
        "harness.tasks"=>("choice",None,["on","off"],None,"on: a native-harness run with a store offers the task tool (create, list, show): the agent of a hub conversation, a job's worker or an RSI run files work for the fleet on the board under its own name - a task like the owner's, with the conversation or job it came from as its origin - and the project's lead plans it on its next pass; an agent's task needs a brief, may not repeat an open title, and an author may have 10 open per project; a read-only run lists and shows but files nothing; off: the tool is not offered"),
        "harness.memory"=>("choice",None,["on","off"],None,"on: a native-harness run reads what earlier runs and chats remembered - ~/.hive/memory/<project>.md for the project and ~/.hive/memory/user.md for the owner - into its system prompt and offers the memory tool (remember, forget, list) so the model can keep a fact for later; a read-only run reads but cannot write; off: nothing is read or offered"),
        "harness.teach"=>("choice",None,["on","off"],None,"on: teacher mode - a native-harness run or chat still does the work, but explains the why of each design or coding choice it makes as it goes (what it chose, the alternative it passed over, the reason) so a junior reading along learns from it; a worker reports those as LESSON: lines, which the runner posts to the project board as lesson posts and lists in the run report; off: the agent works without the commentary"),
        "harness.web_search"=>("choice",None,["chat","on","off"],None,"chat: hive harness chat offers the web_search tool (DuckDuckGo without a key, Brave Search with BRAVE_SEARCH_API_KEY in the environment or ~/.hive/private/providers/brave.json) and jobs do not; on: jobs too; off: nowhere; --web-search / --no-web-search for one run"),
        "harness.hooks"=>("choice",None,["on","off"],None,"on: a native-harness run runs the owner's hooks from the clone's .hive/hooks.json and ~/.hive/hooks.json: run_start output joins the system prompt, pre_tool hooks may block a call (exit 2, the reason goes to the model), post_tool output is appended to the tool result, run_end hears the outcome; in a chat session user_prompt hooks see each message before the model (their output travels with it; exit 2 keeps the message back) and pre_compact hooks speak to the summariser before the conversation is compacted (exit 2 declines it); each hook gets a JSON payload on stdin and runs in the clone with the run's timeout; off: no hooks run"),
        "harness.subagents"=>("choice",None,["on","off"],None,"on: a native-harness run offers the subagent tool: a bounded helper run on a self-contained sub-task in the same clone, with the same provider, budget, skills and hooks, whose report comes back as the tool result; it keeps at least 4 of the run's model calls and 60 s of its minutes for the main agent and cannot start helpers of its own; its steps show one level deeper in the session; off: the tool is not offered"),
        "harness.fetch_allow"=>("text",None,[],None,"hosts a native-harness run's fetch tool may read web pages from, comma- or space-separated, with *.example.com for a domain's subdomains and * for any host; the page comes back as text (2 MB, 30 s, 5 redirects within the allowed hosts; HTML becomes Markdown-like text with its links); empty turns the tool off"),
        "harness.compact_at"=>("count",Some(0),[],Some(95.0),"percent of harness.context_window (or the model's reported window) at which a native-harness run or chat turn asks the model to summarise the older part of the conversation and replaces it with that summary, keeping the system prompt, the task and the last 6 messages; it runs again each time the prompt grows back to the threshold, carrying the earlier summary forward, before any observation collapses, and a request past provider.max_request_mb compacts whatever the percent; the summary call does not count against harness.max_steps and runs at most once every 3 calls; 0 turns the token threshold off and leaves compaction to the request size, to a provider's refusal of a request as too large, and to /compact"),
        "harness.compact_model"=>("text",None,[],None,"the model that writes the compaction summaries of a native-harness run or chat (see harness.compact_at and provider.max_request_mb); empty means the run's own model. A cheaper model saves money on the providers that bill per token - openai, anthropic, gemini and the OpenAI-compatible vendors - where each summary reads up to about 30,000 tokens of transcript; Copilot bills per request, so there it saves time, not credits"),
        "harness.trim_keep_recent"=>("count",Some(0),[],None,"tool results a native-harness run or chat sends whole in each request, counting back from the latest; older results longer than harness.trim_over characters go to the model as their first and last 750 characters around a marker, and the run tells the model to rerun a command when it needs a trimmed result. The conversation the run keeps, the trajectory and the session's steps stay whole, so compaction summaries and the console still see everything. Measured over three days of owner task runs, results older than the eight most recent held 84% of the result characters of an average 79,000-token request; 0 turns trimming off"),
        "harness.trim_over"=>("count",Some(0),[],None,"characters an older tool result may hold before a request trims it (see harness.trim_keep_recent); 4,000 is about the 80th percentile of measured results and the results above it hold two thirds of the result characters; 0 turns trimming off"),
        "provider.max_request_mb"=>("number",Some(0),[],None,"MiB one request to the provider may run to - the conversation with its tool results and images, plus the tool schemas, as the harness estimates them - before a native-harness run or chat turn compacts the older part of the conversation whatever the token count says, so a long conversation stays under what the gateway takes: Copilot times out reading a body of about 1.5 MiB (Timed out reading request body, HTTP 408) long before a 1M-token window is full. A request a provider still refuses as too large (HTTP 408 or 413, context_length_exceeded, prompt is too long) is compacted and sent again, then its older observations collapsed and sent once more, before the turn stops; 0 leaves compaction to harness.compact_at alone"),
        "harness.allow_tools"=>("text",None,[],None,"permission rules for the calls of an interactive harness session (hive harness chat) that run without the user's approval, comma- or newline-separated, each a tool name or tool(filter) with * and ? wildcards: bash(git *), edit(src/*), write, fetch(docs.rs), mcp__github__*; --allow-tool adds to the list and a deny rule wins over an allow; a job asks nobody, so this does not change jobs"),
        "harness.deny_tools"=>("text",None,[],None,"permission rules for the calls a native-harness run refuses outright, in jobs and sessions alike, comma- or newline-separated, each a tool name or tool(filter) with * and ? wildcards: bash(git push *), read(.env*), write(*.lock), subagent; a tool denied whole is not offered to the model; --deny-tool adds to the list; the job policy line (no push, no gh, no credentials) stays on top of these"),
        "harness.reasoning"=>("choice",None,["default","minimal","low","medium","high","xhigh","max"],None,"the reasoning effort a native-harness run asks of the model, sent as reasoning_effort to the OpenAI-compatible providers (openai, azure-openai, openrouter, groq, mistral, deepseek, xai, together, fireworks, cerebras, github-models, ollama, lmstudio, vllm, custom) and to copilot for the models that take it; default sends nothing and leaves the model's own setting; a model that does not take the field rejects the call, so set this per project with its model in mind"),
        "models.allowed"=>("text",None,[],None,"the models a custom agent file (.hive/agents/<name>.md, .claude/agents, .github/agents, an agent record) may ask for with its model key, comma- or newline-separated, * for a run of characters: kimi-k3, claude-*; empty lets any model the provider accepts through; a model outside the list is ignored with a note and the run keeps its routing (run.model, the role's model class), a subagent its parent's model"),
    }
});
pub const COMPUTE_PROVIDERS: &[&str] = &["azure", "aws", "gcp", "multipass"];
pub static COMPUTE_SIZE_SETTINGS: LazyLock<BTreeMap<String, String>> = LazyLock::new(
    || map! {"azure"=>"compute.azure.vm_size","aws"=>"compute.aws.instance_type","gcp"=>"compute.gcp.machine_type","multipass"=>"compute.multipass.size",},
);
pub static COMPUTE_SETTINGS: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
    map! {
    "compute.provider"=>"azure","compute.azure.subscription"=>"","compute.azure.resource_group"=>"","compute.azure.location"=>"","compute.azure.vm_size"=>"Standard_B2ms","compute.azure.image"=>"Ubuntu2404","compute.azure.auth"=>"login","compute.azure.tenant"=>"","compute.azure.client_id"=>"","compute.azure.client_secret"=>"","compute.aws.profile"=>"","compute.aws.region"=>"","compute.aws.instance_type"=>"t3.medium","compute.aws.image"=>"/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id","compute.aws.subnet"=>"","compute.aws.security_group"=>"","compute.gcp.project"=>"","compute.gcp.zone"=>"","compute.gcp.machine_type"=>"e2-medium","compute.gcp.image"=>"ubuntu-2404-lts-amd64","compute.gcp.network"=>"","compute.multipass.size"=>"cpus=2,mem=4G,disk=32G","compute.multipass.image"=>"24.04","compute.max_vms"=>"0","compute.max_hours_per_day"=>"0","compute.auto_approve"=>"0","compute.budget_credits"=>"0","compute.self_heal"=>"1","compute.self_retire"=>"1","compute.retire_grace_hours"=>"24",
    }
});
pub static COMPUTE_RULES: LazyLock<BTreeMap<String, SettingRule>> = LazyLock::new(|| {
    let mut m = rules! {
        "compute.azure.subscription"=>("identifier",None,[],None,"Azure subscription id or name the runner provisions in (auth is az login on this machine)"),
        "compute.azure.resource_group"=>("identifier",None,[],None,"Azure resource group that holds the provisioned hands"),
        "compute.azure.location"=>("identifier",None,[],None,"Azure region, e.g. westeurope"),
        "compute.azure.vm_size"=>("identifier",None,[],None,"VM size for a provisioned hand, e.g. Standard_B2ms"),
        "compute.azure.image"=>("identifier",None,[],None,"VM image alias or URN, e.g. Ubuntu2404"),
        "compute.azure.auth"=>("choice",None,["login","service-principal"],None,"how this machine signs in to Azure for machines and az:// backups: login (az login with your own account, the default) or service-principal (az login --service-principal as the app compute.azure.client_id in compute.azure.tenant; the runner refuses to provision or back up while az is signed in as anyone else)"),
        "compute.azure.tenant"=>("identifier",None,[],None,"Microsoft Entra tenant id (or domain) of the service principal; read only when compute.azure.auth is service-principal"),
        "compute.azure.client_id"=>("identifier",None,[],None,"application (client) id of the service principal; read only when compute.azure.auth is service-principal"),
        "compute.azure.client_secret"=>("reference",None,[],None,"where the service principal's secret lives, as a reference and never the secret itself: op://VAULT/ITEM/FIELD, keychain://SERVICE/ACCOUNT or file:///ABSOLUTE/PATH. Hive never reads it; the console fills it into the az login --service-principal command you run on this machine"),
        "compute.aws.profile"=>("identifier",None,[],None,"AWS CLI profile the runner provisions with; empty means the CLI's default (auth is aws configure or aws sso login on this machine)"),
        "compute.aws.region"=>("identifier",None,[],None,"AWS region the instances are created in, e.g. eu-west-1"),
        "compute.aws.instance_type"=>("identifier",None,[],None,"EC2 instance type for a provisioned hand, e.g. t3.medium"),
        "compute.aws.image"=>("identifier",None,[],None,"AMI id (ami-...) or an SSM public parameter path resolved at launch; the default is Canonical's current Ubuntu 24.04 amd64 image"),
        "compute.aws.subnet"=>("identifier",None,[],None,"subnet id the instances join; empty means the default VPC's default subnet in the region"),
        "compute.aws.security_group"=>("identifier",None,[],None,"security group id the instances get; empty means a group named hive-hands that allows ssh in, created when missing"),
        "compute.gcp.project"=>("identifier",None,[],None,"Google Cloud project the instances are created in; empty means gcloud's current project (auth is gcloud auth login on this machine)"),
        "compute.gcp.zone"=>("identifier",None,[],None,"Compute Engine zone, e.g. europe-west4-a"),
        "compute.gcp.machine_type"=>("identifier",None,[],None,"machine type for a provisioned hand, e.g. e2-medium"),
        "compute.gcp.image"=>("identifier",None,[],None,"Ubuntu image family in ubuntu-os-cloud, e.g. ubuntu-2404-lts-amd64, or a full image path projects/P/global/images/I"),
        "compute.gcp.network"=>("identifier",None,[],None,"VPC network the instances join; empty means default. A firewall rule hive-hands-ssh (tcp 22 to the tag hive-hands) is created on it when missing"),
        "compute.multipass.size"=>("identifier",None,[],None,"size of a local multipass hand as cpus=N,mem=SIZE,disk=SIZE, e.g. cpus=2,mem=4G,disk=32G"),
        "compute.multipass.image"=>("identifier",None,[],None,"multipass image a local hand launches from, e.g. 24.04"),
        "compute.max_vms"=>("count",Some(0),[],None,"provisioned machines that may exist at once; 0 means agents cannot get compute"),
        "compute.max_hours_per_day"=>("count",Some(0),[],None,"machine-hours agents may consume per UTC day across provisioned hands; 0 means none"),
        "compute.auto_approve"=>("switch",None,["0","1"],None,"1: requests under the caps are approved without the owner; 0: every request becomes a proposal"),
        "compute.budget_credits"=>("count",Some(0),[],None,"AI credits the provisioned hands may spend per UTC day together; 0 means no extra budget"),
        "compute.self_heal"=>("switch",None,["0","1"],None,"1: the daemon places the executor token (hive executor token) on a drained executor whose login is refused, once per drain; 0: it leaves the login to you (ADR-034)"),
        "compute.self_retire"=>("switch",None,["0","1"],None,"1: the daemon deallocates a VM it created whose login stays refused and retires its executor row, then deletes the VM after compute.retire_grace_hours; 0: it only reports (ADR-034)"),
        "compute.retire_grace_hours"=>("count",Some(1),[],None,"hours a retired VM stays stopped before the daemon deletes it; starting it again at the provider (the finding names the command) and `hive executor activate NAME` within them keep it"),
    };
    m.insert("compute.provider".to_string(), SettingRule::new("choice", None, COMPUTE_PROVIDERS, None, "where agents get more machines from: azure (az login), aws (aws configure / aws sso login), gcp (gcloud auth login) or multipass (local VMs on the hub machine); each provider reads its own compute.<provider>.* settings"));
    m
});
pub static EVOLVE_SETTINGS: LazyLock<BTreeMap<String, String>> = LazyLock::new(
    || map! {"evolve.loop"=>"0","evolve.limits"=>"none","evolve.tolerance"=>"0.02","evolve.land_neutral"=>"on","evolve.experiments_in_flight"=>"2","evolve.require_check"=>"on","evolve.check_skips_quorum"=>"on","evolve.heldout_dir"=>"","evolve.heldout"=>"auto","evolve.draft_hours"=>"24","evolve.measure_parallel"=>"4","evolve.flat_cycles"=>"6","evolve.heal"=>"on","evolve.flat_wait_hours"=>"24","evolve.baseline_ttl_hours"=>"24","evolve.parent_rate"=>"0.5","evolve.outcome_hours"=>"24","evolve.mode"=>"fixtures","evolve.canary_share"=>"20","evolve.canaries_per_island"=>"3","evolve.keep_days"=>"7","evolve.guard_margin"=>"10","evolve.min_trials"=>"40","evolve.promote_confidence"=>"0.95","evolve.replay_set"=>"20","evolve.trigger_failures"=>"5","lane.retry_minutes"=>"20","lane.short_retry_minutes"=>"2","baseline.retry_hours"=>"6",},
);
pub static EVOLVE_RULES: LazyLock<BTreeMap<String, SettingRule>> = LazyLock::new(|| {
    rules! {
        "evolve.loop"=>("switch",None,["0","1"],None,"1: hive evolve cycle may run experiments; 0: the loop is off (STOP is the stop setting)"),
        "evolve.limits"=>("text",None,[],None,"none, or a budget such as cycles=20,hours=48 after which the loop stops by itself"),
        "evolve.tolerance"=>("decimal",None,[],Some(1.0),"how far a variant's score may fall below the baseline before it is discarded, a share between 0 and 1 such as 0.02; a gain of at least this much is improved, anything in between is a neutral tie that is not landed (unless the card's own check rose), is no win and earns no credit"),
        "evolve.heldout_dir"=>("identifier",None,[],None,"owner-private directory outside every repository holding held-out fixtures; never quoted to agents"),
        "evolve.heldout"=>("choice",None,["auto","owner"],None,"auto: the daemon writes the private held-out set itself, sealed and labelled generated; owner: you write it from the console (ADR-033)"),
        "evolve.draft_hours"=>("count",Some(0),[],None,"hours between the drafting jobs the planner queues for a project whose constitution or evaluators is still the seeded template text (ADR-046); 0 leaves drafting to the console's Draft it for me button"),
        "evolve.measure_parallel"=>("count",Some(0),[],None,"fixtures one measurement may run at once; 0 or 1 keeps fixture measurement sequential. Each fixture's build gets the cores divided by this (CARGO_BUILD_JOBS) and every measurement process runs at nice 10"),
        "evolve.flat_cycles"=>("count",Some(0),[],None,"experiment cycles in a row that all tie (variant score = baseline score) before the loop pauses with one owner question on the project's evolution/loop-gate thread; a fixture enabled since, or an answer or decision on that question, lets it run again; at least 2, and 0 turns the gate off"),
        "evolve.land_neutral"=>("choice",None,["on","off"],None,"on: an experiment that measured as a tie - no fixture moved past evolve.tolerance either way - lands like an improvement, without the credit an improvement earns; off: only an improvement lands, and a tie is archived neutral with its branch dropped. A variant that was never measured, or regressed, never lands"),
        "evolve.require_check"=>("choice",None,["on","off"],None,"on: a card must name a check - `Signal: check:<command>` - that fails on the default branch today and passes once its change is in; a card without one is held and never becomes an experiment or an improve job, and an experiment lands only when its check turned from failing to passing (or the fixtures improved) with no fixture regressed, so a tie never lands whatever evolve.land_neutral says; off: a card needs no check and ties follow evolve.land_neutral"),
        "evolve.check_skips_quorum"=>("choice",None,["on","off"],None,"on: a card that names a check needs no votes to be tried - the check decides - and a net-down vote still rejects it; off: every card waits for quorum (`quorum` distinct voters, more up than down)"),
        "evolve.experiments_in_flight"=>("count",Some(1),[],Some(8.0),"how many experiments one project may have in flight at once; each measures on clones of its own against the same baseline, so more in flight tries more voted cards a day for more of the machine"),
        "evolve.heal"=>("choice",None,["on","off"],None,"on: when the flat-fixture gate pauses a project, the planner first heals the evaluator set itself - derives metric fixtures from the project's own tree (test declarations, TODO markers) and re-arms a derived fixture whose count reached its scale - and posts what it did as a finding on evolution/loop-gate; off: the gate waits for the owner alone"),
        "evolve.flat_wait_hours"=>("count",Some(0),[],None,"hours the flat-fixture gate's owner question may stay unanswered before the planner reviews it itself (an answer post in the loop's name that lets the project run evolve.flat_cycles more cycles; it asks again after that many more tied cycles); 0 waits for the owner"),
        "evolve.baseline_ttl_hours"=>("count",Some(0),[],None,"hours an experiment lane may reuse a measured default-branch baseline for the same commit and fixture hash; 0 means cached baselines never expire"),
        "evolve.parent_rate"=>("decimal",None,[],Some(1.0),"share of experiments the planner builds on an archived variant (a discarded branch's patch, chosen by its lineage's productivity) instead of the default branch; 0 keeps every experiment on the default branch, 1 always builds on the archive when it has a candidate"),
        "evolve.outcome_hours"=>("count",Some(1),[],None,"hours after a landing that changed the improver's own crates before the daemon measures its outcome - landings per worker-hour, first pass, lead time across all projects - against the same window before it, and posts the delta as a result on the landing's thread"),
        "evolve.mode"=>("choice",None,["fixtures","variants"],None,"what the self-improvement loop evolves (ADR-049): fixtures keeps the measured experiment loop on product cards, as before; variants evolves the agent program instead - overlays of agent specs, skills, instructions and allowlisted settings canary on a share of real task and improve jobs, the store's own records of kept landings, tokens, failures and owner interventions are their fitness, and a sequential test promotes or retires them; every variant path is off under fixtures"),
        "evolve.canary_share"=>("count",Some(0),[],Some(100.0),"percent of the trials on a project with live canary variants that the router hands to a canary instead of the incumbent, under evolve.mode=variants; 0 routes every job to the incumbent"),
        "evolve.canaries_per_island"=>("count",Some(0),[],None,"live canary variants one project (an island) may carry at once under evolve.mode=variants; 0 lets none canary"),
        "evolve.keep_days"=>("count",Some(1),[],None,"days after a landing before it counts as kept: a landed commit no later commit on the default branch has reverted by then is kept, the primary fitness of a variant (kept landings per million tokens); the revert scan looks back twice this far"),
        "evolve.guard_margin"=>("count",Some(0),[],Some(100.0),"percent, relative, by which a variant may worsen a guard - failed-job rate, owner interventions per kept landing, verify flake rate - against the incumbent before it cannot be promoted, and a replay worse than the incumbent's by more than this retires it"),
        "evolve.min_trials"=>("count",Some(1),[],None,"trials with a known kept status a canary variant needs before the promotion test may promote or retire it"),
        "evolve.promote_confidence"=>("decimal",None,[],Some(1.0),"posterior probability, between 0 and 1, that a canary's kept landings per million tokens beat the incumbent's before it is promoted, such as 0.95"),
        "evolve.replay_set"=>("count",Some(0),[],None,"recorded past trials of a project a new variant replays in throwaway clones (verify, no merge) before it may canary, scored against the incumbent's replay of the same set; 0 skips replay"),
        "evolve.trigger_failures"=>("count",Some(1),[],None,"trials on one project failing for the same cause within 7 days before the daemon queues a job that proposes a variant to fix it, under evolve.mode=variants"),
        "lane.retry_minutes"=>("count",Some(1),[],None,"minutes the experiment lane waits before retrying a failed or deferred measured experiment"),
        "lane.short_retry_minutes"=>("count",Some(1),[],None,"minutes the experiment lane waits before retrying a transient lane failure, lock or STOP hold"),
        "baseline.retry_hours"=>("count",Some(1),[],None,"hours the daemon waits before planning another baseline measurement after a failed non-transient baseline job"),
    }
});
pub static DAEMON_SETTINGS: LazyLock<BTreeMap<String, String>> = LazyLock::new(
    || map! {"verify.lock"=>"off","daemon.self_update"=>"on","brain.code"=>"clone","executor.transport"=>"local","daemon.self_update_retry_minutes"=>"60","daemon.self_update_settle_minutes"=>"60","daemon.recover"=>"on","daemon.recover_limit"=>"3","daemon.recover_days"=>"7","daemon.min_free_mb"=>"10240","daemon.setup_mb"=>"1024","daemon.unfit_reprobe_minutes"=>"60","daemon.sweep"=>"on","daemon.sweep_grace_minutes"=>"30","daemon.sweep_branch_days"=>"7","daemon.chat_worktree_keep_days"=>"3","daemon.sweep_keep_days"=>"30","daemon.transcript_keep_days"=>"14","daemon.artifact_keep_days"=>"30","daemon.sweep_runners"=>"","daemon.runner_keep_days"=>"30","daemon.log_keep_days"=>"14","daemon.log_max_mb"=>"32","daemon.vacuum_free_mb"=>"256","daemon.archive_days"=>"90","daemon.build_cache"=>"on","daemon.build_cache_keep_days"=>"7","landing.push"=>"off","landing.auto_stash"=>"on","landing.ui_report"=>"require","landing.base_red_retry_minutes"=>"10","landing.setup_repair"=>"on","landing.merge_check"=>"on","hygiene.cargo_target"=>"on","hygiene.cargo_target_hours"=>"24","landing.verify_memo"=>"on","landing.verify_memo_hours"=>"24","push.credential"=>"","git.github_app.app_id"=>"","git.github_app.installation_id"=>"","git.github_app.private_key_ref"=>"","heartbeat.dead_after"=>"300","board.stale_days"=>"7","daemon.planner_cache_ttl_seconds"=>"600","daemon.restart_drain_minutes"=>"30","daemon.restart_max_minutes"=>"90","daemon.restart_hold"=>"on","daemon.hold_seconds"=>"120","daemon.pass_seconds"=>"60","daemon.stage_budget_seconds"=>"60","daemon.nice"=>"5","daemon.build_jobs"=>"0","daemon.keep_awake"=>"off","dependencies.security_updates"=>"on","dependencies.min_release_age_days"=>"3","dependencies.alert_check_minutes"=>"60","dependencies.major_updates"=>"ask","pipeline.watch"=>"fix","pipeline.wait_hours"=>"3","review.merge"=>"auto","review.max_files"=>"20","review.max_lines"=>"600","review.merge_wait_tries"=>"5","review.merge_wait_hours"=>"6","machine.labels"=>"","machine.reserve"=>"off","backup.to"=>"","update.latest_url"=>"","hub.allow_remote"=>"","hub.hosts"=>"","update.channel"=>"auto","backup.every_hours"=>"24","backup.keep"=>"14","backup.subscription"=>"",},
);
pub static DAEMON_RULES: LazyLock<BTreeMap<String, SettingRule>> = LazyLock::new(|| {
    rules! {
        "daemon.self_update"=>("choice",None,["on","off","release"],None,"on: after a landing in the brain the daemon rebuilds hive from the clean checkout, smoke-tests it, installs it and restarts after its runs in flight; release: the daemon installs each newer published release that update.latest_url names (checked against its SHA256SUMS, signed, smoke-tested) and restarts after its runs in flight, never building from source - for a machine that holds no Hive source, such as one installed from a release; off: builds are yours (ADR-035; the launcher turns it off after a crash-loop). Neither mode writes over a hive that a package manager such as Homebrew installed: that manager upgrades it (docs/release.md)"),
        "brain.code"=>("choice",None,["clone","none"],None,"clone: the brain clones a project to run its jobs, check scripts and landing experiments under HIVE_HOME/work; none: no project code is ever checked out on the brain - its own slot takes only the project lead's jobs and the reflection (kinds lead and reflect), which run in an empty workspace and plan from the board, the task text and the diffs hands report; every job that works in the code waits for a hand, check and watch scripts are held, and a review job, which needs the code and the store both, has no machine to run on yet (ADR-070 rule 6, docs/security.md)"),
        "daemon.self_update_retry_minutes"=>("count",Some(1),[],None,"minutes the daemon waits before building again after a failed build or smoke test"),
        "daemon.self_update_settle_minutes"=>("count",Some(0),[],None,"minutes the brain's HEAD must stand unchanged before the daemon rebuilds hive onto it, so a burst of landings builds once rather than once per landing (default 60: every rebuild ends in a restart window that refuses the long kinds of run); 0 builds as soon as HEAD differs"),
        "daemon.recover"=>("choice",None,["on","off"],None,"on: the daemon recovers failed jobs by itself - a transient failure runs again after a growing pause, a failure in the work becomes one corrected job carrying the failure, a structural one waits for its own fix - and writes the repeated causes down as memory records that later briefs carry; off: a failed job stays failed until `hive job retry` (ADR-036)"),
        "daemon.recover_limit"=>("count",Some(0),[],None,"recoveries one failed job may receive (the same job run again or a corrected job queued for it) before the daemon holds it for the lead and the owner; 0 holds every failure at once"),
        "daemon.recover_days"=>("count",Some(1),[],None,"days a failed job stays eligible for recovery and counts toward a repeated-failure lesson"),
        "daemon.min_free_mb"=>("count",Some(0),[],None,"megabytes an executor's workdir filesystem must have free for its probe to pass and for a run's verify to start; under it the probe note says `under N free on <workdir>` and the hand takes no job until space returns; the default leaves room for a build and the clones beside it, since a run that hits ENOSPC mid-build is lost work; 0 turns the check off"),
        "daemon.setup_mb"=>("count",Some(0),[],None,"megabytes of work-filesystem space one run's clone and setup are taken to need until runs of the project have measured it (runs.work_mb); each tick the daemon starts no more local runs than the free space above daemon.min_free_mb pays for at the largest footprint an active project has shown, one run always when none is in flight; 0 turns the admission off and leaves the floor alone"),
        "daemon.unfit_reprobe_minutes"=>("count",Some(0),[],None,"minutes before the daemon re-probes an executor/project held by an unfit mark while queued work waits; repeated failures double this wait up to 24h, and 0 turns the daemon re-probe off"),
        "daemon.sweep"=>("choice",None,["on","off"],None,"on: every ten minutes the daemon removes what runs, landings and agents left on this machine and nothing holds any more - clones, keys and bundles of finished jobs under HIVE_HOME/work, landing worktrees whose process is gone, holding refs, run logs and patches past daemon.sweep_keep_days, hive/* or evolve/* branches no job will land (saved as a patch first), and worktrees a project checkout lists whose work is on the default branch (its merged branch goes with it) or that sat untouched for daemon.sweep_branch_days (the branch stays); a run in flight, a branch waiting to land, a locked worktree, uncommitted changes and the owner's own files are never touched. off: `hive sweep` by hand"),
        "daemon.sweep_grace_minutes"=>("count",Some(0),[],None,"minutes a finished job's clone, key or bundle, a dead landing's worktree and a clean project worktree whose every commit is on the default branch must sit untouched before the sweep removes them; a lost run's leftovers go at once"),
        "daemon.sweep_branch_days"=>("count",Some(0),[],None,"days since its last commit before a hive/* or evolve/* branch that no job will land is removed, its commits saved under HIVE_HOME/patches first unless they are already on the default branch; also the days a clean worktree of a project with commits not on the default branch sits untouched before the worktree alone goes (its branch stays; a detached HEAD is saved as a patch first)"),
        "daemon.chat_worktree_keep_days"=>("count",Some(0),[],None,"days a hub conversation stays closed or archived before the sweep removes its clean checkout under HIVE_HOME/work/chat/<id> to give the disk back; its branch, artifacts and attachments stay, and the owner's next message there checks the branch out again (and the project reinstalls its dependencies); a dirty checkout, or a closed one whose branch has commits not on the default branch, is held; 0 keeps every conversation checkout"),
        "daemon.sweep_keep_days"=>("count",Some(1),[],None,"days saved patches and work files of jobs this store does not know are kept before the sweep removes them"),
        "daemon.transcript_keep_days"=>("count",Some(0),[],None,"days a harness session's step transcript is kept after the session ends - its harness_steps rows, its harness.step notification events and its HIVE_HOME/harness/<session>.json file - before the sweep removes them in small batches; the session row (counts, tokens, cost, submission) stays for good; 0 keeps every transcript"),
        "daemon.artifact_keep_days"=>("count",Some(0),[],None,"days a finished job's artifacts folder (HIVE_HOME/work/jobs/<job>/artifacts: the screenshots and reports its worker left for the owner, shown in the hub beside the job) is kept after the job finishes before the sweep removes it; an empty folder of a finished job goes at once, a folder of a job the store does not know goes after daemon.sweep_keep_days; 0 keeps every folder"),
        "daemon.sweep_runners"=>("text",None,[],None,"the other agent programs on this machine whose old sessions the sweep removes, by id, comma-separated: copilot (Copilot CLI, ~/.copilot/session-state), claude (Claude Code, ~/.claude/projects with the folders kept beside each transcript), codex (Codex, ~/.codex/sessions), gemini (Gemini CLI, ~/.gemini/tmp/*/chats); all names every one Hive finds and knows how to prune, none means the owner declined and the hub stops asking, empty means Hive only reports what it found and the hub asks. A session goes after daemon.runner_keep_days without a change, never while its program holds it, and never the program's settings, index or credentials; OpenCode is reported, never pruned"),
        "daemon.runner_keep_days"=>("count",Some(1),[],None,"days a session of an agent program listed in daemon.sweep_runners stays after its last change before the sweep removes it; the disk-floor ladder does not shorten this window, since these are the owner's conversations rather than Hive's leftovers"),
        "daemon.archive_days"=>("count",Some(0),[],None,"days ledger rows stay in hive.sqlite before the maintenance thread moves them, once a month, to hive-archive.sqlite next to it (`hive store archive --dry-run` shows what would go): the event kinds nothing reads back (votes, posts, run starts and finishes, resumes), finished runs no brief, harness session, evolution entry or post names, and harness steps of ended sessions; never fewer days than daemon.recover_days, twice evolve.keep_days or, for steps, daemon.transcript_keep_days; nothing in Hive reads the side file except hive verify and a report that attaches it; 0 keeps every row in the store"),
        "daemon.log_keep_days"=>("count",Some(1),[],None,"days run logs and daemon stall/signal logs under HIVE_HOME/logs are kept before the sweep removes them"),
        "daemon.log_max_mb"=>("count",Some(0),[],None,"megabytes daemon.log and hub.log may grow to before the sweep copies each to <name>.1 and truncates it in place (the launcher opens them in append mode, so the live process keeps writing); 0 never caps them"),
        "daemon.vacuum_free_mb"=>("count",Some(0),[],None,"megabytes of free pages the store may hold before the sweep gives them back to the file system: a store in incremental auto-vacuum mode frees a slice each pass, one that is not is converted with one full VACUUM only while no run or harness session is in flight and the disk has room for a copy; 0 never vacuums"),
        "daemon.build_cache"=>("choice",None,["on","off"],None,"on: a local run of a Cargo project builds into a shared per-project target directory under HIVE_HOME/cache/cargo-target/<slug>/<lane> (one lane per run in flight, so concurrent runs never share one) instead of a cold target inside its throwaway clone, unless the project's recipe sets CARGO_TARGET_DIR itself; off: every clone builds from scratch"),
        "daemon.build_cache_keep_days"=>("count",Some(1),[],None,"days a shared build-cache lane may sit unused before the sweep removes it; lanes of projects the store no longer lists go at once, and dead units and stale incremental caches inside a lane are pruned like any shared Cargo target"),
        "hygiene.cargo_target"=>("choice",None,["on","off"],None,"on: the daemon sweep prunes dead or idle workspace-crate units, stale incremental caches and debug objects no binary's debug map names from shared Cargo target directories named by active projects' CARGO_TARGET_DIR verify commands, only under the project checkout or HIVE_HOME and only while verify/cargo locks are free; off: leave Cargo target directories alone"),
        "hygiene.cargo_target_hours"=>("count",Some(1),[],None,"hours an incremental cache directory, or a workspace crate's compiled unit, must sit unmodified before the hygiene sweep removes it from a shared Cargo target; registry and git dependencies are never pruned for idleness"),
        "landing.push"=>("choice",None,["off","main","pr"],None,"main: after a landing the daemon pushes the project's default branch to its upstream, fast-forward only, with the git credentials this machine already has - or with push.credential when it is set - and pushes what landed earlier and was not pushed yet; a failed push shows in the landing line of hive status and the hub. pr: the daemon pushes the default branch to the hive/landed branch on the upstream instead and keeps one pull request open from it (gh, logged in as the user the daemon runs as), so every landing waits for your merge on the forge; a conflict with the upstream shows on the pull request. off: you push - or ask for a pull request from a task's sheet or with hive land --pr (ADR-037)"),
        "landing.auto_stash"=>("choice",None,["on","off"],None,"on: before landing, the daemon stashes uncommitted changes in the checkout with a clear message so the landing can proceed; off: a dirty checkout defers landing (ADR-024)"),
        "landing.ui_report"=>("choice",None,["require","warn","off"],None,"require: a change that touches a screen (a page, a view, a stylesheet) reaches the default branch only with its before/after report - report.html, which hive report builds from report.json and the screenshots in the agent's artifacts folder; a worker that finished without one gets one more turn to make it, a local landing waits for it, a pull request without one is the owner's to merge and a session is asked for it before its pull request opens; warn: the change lands without one and the landing says so; off: no report is asked for (a screenshot is still welcome); landing.ui_report.<project slug> sets it for one project"),
        "verify.lock"=>("choice",None,["off","project","machine"],None,"off: a worker commit, a finished run and a landing each verify whenever they are ready; project: verifications of one project take turns on a machine, so checkouts sharing one build cache (a Cargo target directory, a Gradle cache) never rewrite each other's artifacts mid-run; machine: every verification on the machine takes turns. While verifications take turns, a project runs only as many improve and task jobs at once as its measured check time fits into harness.step_timeout (its idle line names the lanes)"),
        "landing.base_red_retry_minutes"=>("count",Some(1),[],None,"minutes a project waits before the daemon re-checks landings after the default branch itself fails verify"),
        "landing.verify_memo"=>("choice",None,["on","off"],None,"on: skip a run's own check, a local landing's and a base verify when this user recently verified the identical Git tree with the identical verify command (the pre-commit hook's memo under ~/.hive/verified/); off restores the old always-verify behaviour"),
        "landing.verify_memo_hours"=>("count",Some(0),[],None,"hours a verified-tree memo remains valid for landing/base verify skips; default 24"),
        "heartbeat.dead_after"=>("count",Some(30),[],None,"seconds an agent may go without a heartbeat before it counts as dead: every hive command run as an agent beats, so do its hooks, its job lease and the runner supervising its run; a dead worker's run is closed as lost and its job queued again, a dead session simply shows as dead until it beats again; never under 30"),
        "board.stale_days"=>("count",Some(1),[],None,"days before the daemon retires an open suggestion whose non-empty files list names only paths absent at the project HEAD"),
        "daemon.planner_cache_ttl_seconds"=>("count",Some(60),[],None,"seconds an unchanged planner/tally snapshot may stay in memory; watermarks invalidate it earlier when board, job, landing, record or setting state changes"),
        "daemon.restart_drain_minutes"=>("count",Some(0),[],None,"minutes a daemon whose binary changed on disk keeps dispatching and landing while runs are in flight before it stops starting runs and exits with code 3 after them; it exits as soon as nothing is in flight, stops dispatching once less than minutes_per_job + 5 of the window is left, and 0 stops dispatching at once (the old behaviour)"),
        "daemon.restart_max_minutes"=>("count",Some(0),[],None,"minutes a pending restart may hold the handover open in all, from the moment it is armed (a newer binary on disk, or the owner's ask without now): once they are up nothing new starts, the owner's work included, and the daemon exits with code 3 once the runs in flight end or hold as daemon.restart_hold says. The drain (daemon.restart_drain_minutes, 30) and one more drain of stay for the owner's queued work make an hour; the half hour past it lets an owner's run started late in the window get on before it is held. Each owner's run started while a restart is pending anchors the window again, so without this bound a queue of the owner's work that never empties holds the restart for good (on 2026-11-26 one armed at 05:42Z had not come by 12:07Z). It bounds the self-update build too: the build waits for queued fleet work the restart would refuse to start first for this long at most, counted from the first pass that waited (or the arming of a restart already pending), then builds and the queued work starts under the next daemon (on 2026-11-26 a priority-0 audit held every build for seven hours); 0 is no bound (the old behaviour, and the build waits an hour)"),
        "daemon.restart_hold"=>("choice",None,["on","off"],None,"on: a daemon whose restart drain (daemon.restart_drain_minutes) ends with runs still in flight asks them to hold as Restart now does - the harness saves its conversation, exits held and the next daemon carries the job on in the same clone - and waits only for runs past their harness (in their check, or a custom worker), which run to their end; off: it waits for every run to end (to 2026-11-04 the brain's self-updates waited 0.2 to 4.3 hours each, twelve hours over three days with nothing dispatched)"),
        "daemon.hold_seconds"=>("count",Some(0),[],None,"seconds a daemon asked to restart now (Settings, Running, Restart now) waits for its runs in flight to hold - the harness saves its conversation, exits held and the job's next run carries on in the same clone - before it stops the runs still going, whose jobs return to the queue and start over; then it exits with code 3 so the service restarts it"),
        "daemon.pass_seconds"=>("count",Some(0),[],None,"least seconds between two planning passes of the daemon loop (leads, tasks, amendments, recoveries, lessons, evolution, schedules, planner, idle notes, executors, maintenance); the ticks in between, every 15 seconds while runs are in flight and whenever a run ends or the bus wakes the loop, only reap, watch the stop switch and the restart drain, start approved compute and refill free slots; 0 plans on every tick, and a shorter daemon interval caps it"),
        "daemon.stage_budget_seconds"=>("count",Some(0),[],None,"seconds each stage of the daemon's background passes may run - the landing maintenance pass (delivery, reviews, sweep, reclaim, hygiene, outcomes, revert scan, re-probes, binary check, backup) and the chores pass off the tick (Dependabot alerts, pipeline verdicts, compute); a stage over it stops between two projects, never mid-write, records a daemon.slow-stage event with its duration and what it left, and the next pass starts there; 0 lets every stage run to its end"),
        "dependencies.security_updates"=>("choice",None,["on","off"],None,"on: for every active project with a GitHub remote the daemon reads the repository's open Dependabot alerts (through the gh CLI) once per dependencies.alert_check_minutes and queues one security update job per affected manifest, which moves each vulnerable package to the newest patched version that is at least dependencies.min_release_age_days old and stays within its compatible range (a fix that needs a major upgrade is reported to the owner instead); the landing checks the same rule on the branch. off: alerts are not read"),
        "dependencies.min_release_age_days"=>("count",Some(0),[],None,"days a dependency version must have been published before hive adopts it, checked against the version's registry (npm, crates.io, PyPI, the Go proxy); a security update waits for the patched version to be this old, and a branch that pins younger versions in a lockfile is held until they are; 0 turns the rule off"),
        "dependencies.alert_check_minutes"=>("count",Some(5),[],None,"minutes between two reads of a project's Dependabot alerts"),
        "machine.labels"=>("text",None,[],None,"what this machine offers beyond its OS and architecture, as words a project or job can need (hive.yaml needs:, hive job needs): xcode and docker are found on their own; add the rest here, comma-separated, such as gpu, android or a tool's name"),
        "executor.transport"=>("choice",None,["local","local-hand"],None,"local: this machine runs its own jobs in a clone under the hive home, as it always has; local-hand: it leases them to the hive-hand binary installed beside hive over a pipe (ADR-070 step 1b) - the hand makes the clone in HIVE_HOME/hands, runs the worker there and, once the commits pass the identity check, pushes the job's branch to the upstream with the key push.credential names, handed to it for that one push; jobs that work the store (lead, review, suggestion review) stay local, and a hand push that fails is left to the delivery pass"),
        "machine.reserve"=>("choice",None,["off","on"],None,"on: this machine takes only the jobs that need something it offers (a project with needs: [macos] on the only Mac), so the rest of the queue goes to the other machines; off: it takes any job it can run"),
        "pipeline.watch"=>("choice",None,["off","watch","fix"],None,"fix: after every push the daemon makes (landing.push main, a pull request from hive/landed) it watches the GitHub Actions runs of the pushed commit through the gh CLI; a red pipeline is a finding for the owner on the project's pipeline thread and one fix job for the project (source pipeline:<slug>, at most three a day), a green one is recorded (pipeline.passed). watch: the finding without the job. off: pushes are not watched"),
        "landing.merge_check"=>("choice",None,["on","off"],None,"on: before a task's pull request opens, the daemon compiles the tree its finished branches make together (cargo check --workspace --all-targets, Cargo projects only); a tree that does not compile goes to a fix job on the task, then to you, instead of onto the forge where every job built on it fails its check. off: branches that merge without a conflict open the pull request as they are"),
        "landing.setup_repair"=>("choice",None,["on","off"],None,"on: when the setup the environment recipe (or the lockfile heuristic) describes fails at a commit of the default branch - in the landing's worktree, or twice the same way in a worker's clone - the daemon plans one repair job for the project (source setup:<slug>, at most three a day) that fixes the fault where it is (the recipe record, the verify command, a lockfile that outran its manifest) and proves the setup passes there; the repair job runs without the recipe's setup and through a setup unfit mark, and a landing whose setup passes clears that mark. off: the landing retries on landing.setup_retry_minutes and the owner fixes it"),
        "pipeline.wait_hours"=>("count",Some(1),[],None,"hours a push's workflow runs may stay queued or running before the daemon stops waiting on them and tells the owner (pipeline.gave-up)"),
        "review.merge"=>("choice",None,["auto","ask"],None,"auto: the review agent merges a Hive pull request itself when it stays off guarded paths, the check is green and the whole diff is what the title and body say; anything else goes to the owner with the review's findings (size alone hands nothing to the owner). ask: the agent still reviews every pull request and leaves its findings, but every merge waits for the owner"),
        "review.max_files"=>("count",Some(1),[],None,"files a pull request may touch before the review agent reads it file by file; it still merges one whose every file is what the title and body say, and hands the owner what it cannot account for (size alone hands nothing to the owner)"),
        "review.max_lines"=>("count",Some(1),[],None,"lines (added plus deleted, not counting generated files such as lockfiles and build output) a pull request may change before the review agent reads it file by file, as for review.max_files"),
        "review.merge_wait_tries"=>("count",Some(1),[],None,"times the forge may fail a reviewer's approved merge (a 5xx, a rate limit, a dropped connection) before the merge stops waiting: the pull request goes to the owner and the project's lead is told once, with the forge's last answer"),
        "review.merge_wait_hours"=>("count",Some(1),[],None,"hours a reviewer's approved merge may wait for its checks or for the forge before it stops waiting: the pull request goes to the owner and the project's lead is told once, with the forge's last answer"),
        "dependencies.major_updates"=>("choice",None,["ask","auto"],None,"ask: a security fix that needs a major upgrade (the first patched version is outside the installed version's compatible range) is put to the owner as a finding with a Yes (queue the upgrade) and a No; auto: the daemon queues it like any other security update, moving the package to the newest version of the patched range that is dependencies.min_release_age_days old, the worker making the code changes the new major needs and the project's checks deciding"),
        "backup.to"=>("text",None,[],None,"where copies of the store (hive.sqlite) go, off this machine: a directory that already exists (/Volumes/Kingston/hive, ~/Library/Mobile Documents/com~apple~CloudDocs/hive - a drive that is not mounted is refused, never a folder made on the boot disk), az://ACCOUNT/CONTAINER[/PREFIX] (Azure Blob Storage through the az CLI you are logged in to; the account key is read for the seconds of the upload and never written, the login is used when it cannot be), or `run: COMMAND {file} {name}` (rclone, aws s3 cp, scp, ... with the gzip file's path and name filled in). Each backup is a consistent copy (VACUUM INTO) checked with quick_check, gzipped, its sha256 noted; `hive backup` takes one now. Empty: no backups"),
        "backup.every_hours"=>("count",Some(0),[],None,"hours between two backups the daemon takes to backup.to on its maintenance pass; a failed one is tried again after an hour; 0 leaves backups to `hive backup`"),
        "backup.keep"=>("count",Some(0),[],None,"copies of this machine's store kept at a directory or Azure destination, the newest first; older ones are removed after each backup; 0 keeps every copy"),
        "backup.subscription"=>("text",None,[],None,"the Azure subscription (name or id) that holds the az:// account in backup.to, when it is not the az CLI's default one: every az call a backup makes (keys list, upload, list, delete) carries it as --subscription, so the key is found and the login's token is cut for that tenant. Empty: the CLI's default subscription"),
        "update.channel"=>("choice",None,["auto","source","release"],None,"how this machine gets a newer Hive. source: the daemon rebuilds hive from the brain project's checkout after a landing (daemon.self_update); release: hive update downloads the published release named by update.latest_url, checks it against the release's SHA256SUMS and installs it, every command says once per release that one is out, and the daemon never rebuilds from source. auto (the default): source when a brain project is registered and cargo is on PATH, release otherwise"),
        "hub.allow_remote"=>("text",None,[],None,"the origin of a front you run yourself (https://team.example.com) the console may be reached through from anywhere; Hive ships no front (ADR-071): hive hub --front-port N then opens a second 127.0.0.1 listener for the front's relay, every read through it needs this setting and every write through it is refused unless it carries a passkey signature of a registered member (/api/v1/members, GET /api/v1/nonce) that the brain verifies itself, so the front cannot forge one. It counts only when the owner set it, never an agent. Empty (the default): nothing is served to the front and the console stays 127.0.0.1-only"),
        "hub.hosts"=>("text",None,[],None,"extra host names the console answers to, comma-separated (for example hive.example.com, when a proxy of yours forwards that name to 127.0.0.1); empty: localhost and IP addresses only. The console refuses every other Host header, so a web page that points a name of its own at 127.0.0.1 (DNS rebinding) cannot reach it; the routes a hand calls are exempt. Counts only when the owner set it"),
        "update.latest_url"=>("text",None,[],None,"where the daemon looks, at most once a day, for the latest Hive release: an http(s) URL answering JSON - a GitHub releases/latest URL (https://api.github.com/repos/OWNER/REPO/releases/latest), a GitHub releases list, or {\"version\": \"v1.2.3\", \"url\": \"https://...\"}. When it names a version newer than the running one, hive version and the console say so with the release link. The check downloads and installs nothing; hive update installs the release (update.channel, docs/release.md). Empty (the default): no check and no network by itself; only hive update, when you run it yourself, then looks at Hive's own releases (https://api.github.com/repos/lamplitlabs/hive/releases/latest) for that one run, and says so first"),
        "daemon.nice"=>("count",Some(0),[],None,"nice level the daemon's own process runs at from startup (setpriority), under the owner's foreground work at 0 and over its job children at 10, so planning, sweeps and landing passes yield to what the owner is doing while the loop still comes before the fleet's builds; 0 leaves the level the service started it at"),
        "daemon.build_jobs"=>("count",Some(0),[],None,"parallel compile jobs a build the daemon causes may use: exported as CARGO_BUILD_JOBS to its job children, their pre-commit verification and the landing verification when the environment does not set it, and passed as -j to the self-update build; 0 is half the physical cores, so no single build takes the whole machine"),
        "daemon.keep_awake"=>("choice",None,["off","on","always"],None,"on: while the daemon runs and the machine is plugged in it holds this machine awake the way Copilot CLI's keep-alive does (macOS: caffeinate -d -i -s, so the display stays on and the screen saver stays off too; Linux: systemd-inhibit), so overnight runs are not lost to sleep, and the console shifts to its awake theme; on battery a Mac holds nothing until the charger is back. always: holds on battery too, display included. off: the machine sleeps as its power settings say. A paused or stopped daemon holds nothing, and closing a laptop's lid still sleeps a Mac without an external display. Each tick the daemon also notices when the machine slept anyway (the wall clock jumped while the loop waited) and says for how long"),
        "push.credential"=>("reference",None,[],None,"where the daemon's own SSH push key lives, as a reference and never the key itself: op://VAULT/ITEM/FIELD?ssh-format=openssh (1Password, read with a service-account token kept by `hive cred token`), keychain://SERVICE/ACCOUNT (a macOS Keychain item) or file:///ABSOLUTE/PATH (a 0600 file). The key exists on disk only for the seconds of a push; when it cannot be resolved the landing is held with the reason and the daemon never falls back to your credentials. Empty: push with what git already uses. Set it with `hive cred set`, which probes the remote first (ADR-041)"),
        "git.github_app.app_id"=>("identifier",None,[],None,"the numeric App ID of the GitHub App Hive reads and writes repositories as (ADR-070 step 3), from the App's settings page on GitHub; with git.github_app.installation_id and git.github_app.private_key_ref set, the brain mints a one-hour installation token scoped to the one repository a job needs, hands it to the run through a git credential helper that lives as long as the run, and pushes and opens pull requests with it before it tries this machine's own logins. Empty: no App, git and gh use what this machine is logged in to"),
        "git.github_app.installation_id"=>("identifier",None,[],None,"the numeric id of the GitHub App's installation on your account or organization (the number at the end of github.com/settings/installations/N); the brain exchanges the App's signed JWT for tokens of this installation. Empty: no App"),
        "git.github_app.private_key_ref"=>("reference",None,[],None,"where the GitHub App's private key (the .pem GitHub downloads once) lives, as a reference and never the key itself: op://VAULT/ITEM/FIELD (1Password), keychain://SERVICE/ACCOUNT (a macOS Keychain item) or file:///ABSOLUTE/PATH (a 0600 file). Only the brain reads it, for the seconds it takes to sign a ten-minute JWT; jobs never see it, only the one-hour repository tokens it buys. Empty: no App"),
    }
});
pub static LEAD_SETTINGS: LazyLock<BTreeMap<String, String>> = LazyLock::new(
    || map! {"lead.idle_hours"=>"6","lead.idle_max_jobs"=>"3","lead.stuck_thread_jobs"=>"3","lead.research_minutes"=>"60","lead.research_per_day"=>"12","lead.idle_factor"=>"3","lead.ballot_hours"=>"1","lead.ballot_cap"=>"10","plan.operating_layer_per_day"=>"3","plan.operating_layer_fill"=>"on","plan.loop_per_day"=>"1","schedule.min_minutes"=>"30","schedule.max_per_project"=>"10","schedule.script_timeout_minutes"=>"20","schedule.scripts_per_tick"=>"2","manifest.load"=>"on","lead.suggest_days"=>"30","toolkit.suggest_days"=>"30","goal.review_days"=>"7",},
);
pub static LEAD_RULES: LazyLock<BTreeMap<String, SettingRule>> = LazyLock::new(|| {
    rules! {
        "lead.idle_hours"=>("count",Some(0),[],None,"hours a project may sit with no job queued, leased or created before its lead is asked to find the next work by itself and tell the owner; research, ballot and reflect jobs the planner seeds on its own do not count as the project moving; 0 turns the standing brief off"),
        "lead.idle_max_jobs"=>("count",Some(1),[],None,"worker jobs one idle lead run may queue"),
        "lead.stuck_thread_jobs"=>("count",Some(0),[],None,"finished worker jobs one board thread may seed in a row (from its suggestions, handoffs and questions) that commit nothing - read-only triage, claims, questions - before the planner stops seeding that thread and asks the lead to decide it; a job from the thread that commits resets the count; 0 turns the cap off"),
        "lead.research_minutes"=>("count",Some(0),[],None,"minutes a project must sit with nothing queued or leased, no research job and no job that decided work (ballots and reflects do not count) created, before the planner seeds one research job (while the fleet is idle a free lane under parallel_per_project is enough: research then runs beside in-flight work): a worker investigates one focus in turn (the product as its user sees it, the code that changes most, the checks, the loop's report, the docs, the board, the memory) and ends with one to three experiment cards and a vote on every card below quorum; 0 turns research off (ADR-045)"),
        "lead.research_per_day"=>("count",Some(1),[],None,"research jobs one project may receive per UTC day"),
        "lead.idle_factor"=>("count",Some(1),[],None,"while fewer than half the local worker slots hold a lease and no job waits in the queue the fleet is idle: research waits lead.research_minutes divided by this (never below 10 minutes) with lead.research_per_day grown by it, a ballot job may come every lead.ballot_hours divided by this (never below 10 minutes), and a project's lead gets its standing brief after lead.idle_hours divided by this (never below 1 hour), so idle slots turn into cards, votes and queued work instead of waiting; 1 keeps the plain cadences"),
        "lead.ballot_hours"=>("count",Some(1),[],None,"hours a card may wait below the vote quorum before the planner seeds a ballot job, at most one per project per that many hours, and only when nothing else is queued"),
        "lead.ballot_cap"=>("count",Some(1),[],None,"maximum number of below-quorum cards a single ballot job asks a worker to vote on; closest-to-quorum cards are listed first"),
        "plan.operating_layer_per_day"=>("count",Some(0),[],None,"operating-layer improve jobs the planner may create from suggestion cards for one project per UTC day; product cards are unaffected and are planned first; 0 turns this cap off"),
        "plan.loop_per_day"=>("count",Some(0),[],None,"improve jobs and experiments the planner may make for one project per UTC day from cards that change Hive's own self-improvement loop - crates/hive-evolve, crates/hive-tally and the runner's research, ballot, planning, experiment and brief code - so the loop spends its slots on the projects' defects rather than on itself; never lifted, whatever else waits; 0 turns this cap off"),
        "plan.operating_layer_fill"=>("choice",None,["on","off"],None,"on: once the day's operating-layer cap is reached, voted operating-layer cards are still planned while no product card of the project passes the plan gates, so idle workers do the voted work instead of waiting for tomorrow; the cap holds again the moment a product card competes. off: the cap holds whatever competes"),
        "schedule.min_minutes"=>("count",Some(5),[],None,"the shortest cadence a schedule may have (`hive schedule add --every`); a standing job that runs more often than this is a loop, not a schedule"),
        "schedule.max_per_project"=>("count",Some(0),[],None,"active or paused schedules one project may hold at once; 0 turns the cap off"),
        "schedule.script_timeout_minutes"=>("count",Some(1),[],None,"minutes a check or watch script (`hive schedule add --kind check|watch --script`) may run before the daemon kills it and counts the run as a failure (a red check, a broken watcher)"),
        "schedule.scripts_per_tick"=>("count",Some(1),[],None,"check and watch scripts the daemon runs in one tick, the longest overdue first; the rest wait for the next tick, and none runs while the machine is saturated"),
        "manifest.load"=>("choice",None,["on","off"],None,"on: the daemon reads each project's own Hive files, .hive/automations/<name>.yaml and .hive/hive.yaml on the default branch, and the SKILL.md folders beside them (.hive/skills, .claude/skills, .github/skills, .agents/skills) whenever a commit changes them: the schedules the files name are added, updated, paused and ended in the store (hive schedule list shows them with their origin), the skills join the project's hive skill registry, and what a file gets wrong is a problem on the project board, never a stopped schedule; off: the files are left to the native harness, which still offers the skill folders to the model"),
        "lead.suggest_days"=>("count",Some(0),[],None,"days before the daemon asks the owner again to seat a role lead (`hive lead add`) for a concern of a project nobody owns - a self-improvement loop with no rsi lead, code landing without docs and no docs lead, a dependency manifest with no security lead, no check or watcher and no monitor lead; one question per project per pass, on the board thread lead/<role>; 0 turns the suggestions off"),
        "toolkit.suggest_days"=>("count",Some(0),[],None,"days before the daemon asks the owner again to connect a tool (an MCP server) or add a skill that a project's shape asks for and it does not have - a GitHub remote with no github tool, a UI with no browser, Postgres in the dependencies with no schema reader, migrations with no recipe for them (`hive toolkit` lists them); one question per project at a time, on the board thread toolkit/tool-<name> or toolkit/skill-<name>; 0 turns the suggestions off"),
        "goal.review_days"=>("count",Some(1),[],None,"days between the lead's readings of the project goal: `hive goal review` and the hub's goal panel score the last work against the goal from the store (what landed, what passed its checks, how much of it speaks to the goal's words, how the pace compares with the window before) and blend in the lead's latest reading (a finding on the board thread goal/review with a `Score: NN` line) when it is this recent; when none is, the lead's brief asks for one, and the lead may propose a new goal (an opinion on goal/amend whose first line is `goal: amend`) that the owner takes with one yes"),
    }
});
pub const OWNER_PRESENT: &str = "present";
pub const OWNER_AWAY: &str = "away";
pub static OWNER_SETTINGS: LazyLock<BTreeMap<String, String>> =
    LazyLock::new(|| map! {"owner.name"=>"","owner.presence"=>"present",});
/// The most characters `owner.name` keeps: a display name, not a biography.
pub const OWNER_NAME_MAX: usize = 64;
pub const OWNER_PATHS: &[&str] = &[
    "docs/evolution/constitution.md",
    "docs/evolution/evaluators.md",
    "docs/evolution/STOP",
    "docs/decisions/",
    ".github/",
    ".hive/agents/",
    ".hive/roles/",
    ".hive/kinds/",
    // The same folders under the project folder's name from before the
    // rename (`legacy::LEGACY_PROJECT_DIR`): read alike, so closed alike.
    ".pulse/agents/",
    ".pulse/roles/",
    ".pulse/kinds/",
];
pub static OWNER_RULES: LazyLock<BTreeMap<String, SettingRule>> = LazyLock::new(|| {
    rules! {
        "owner.name"=>("text",None,[],None,"what the console calls you: the greeting on Home and the owner row of the sidebar use it; one line of at most 64 characters, trimmed; empty means the console says you"),
        "owner.presence"=>("choice",None,["present","away"],None,"present: proposals, compute requests and owner questions wait for you, and hive asks before it adds a tool, a skill or a lead seat; away: each project's lead decides the board's proposals on your behalf with its best judgement, recorded as the lead's decision and never as yours, hive adds the toolkit a project's shape asks for on its own (a drafted skill, a tool that needs no secret) and says so on the board, and lead seats are not asked about (the hub's Leads gallery shows them) - hard stops, money (compute), Tier 2 decisions (ADRs), the constitution's protected sections and the kill switch still wait for you"),
    }
});
pub const NONE_WORDS: &[&str] = &["none", "off", "unlimited"];
/// Count settings that are a percent: their rule's max is enforced. (Other
/// count rules carry a max as documentation only, and enforcing it there
/// would refuse values stores already hold.)
const PERCENT_SETTINGS: &[&str] = &["evolve.canary_share", "evolve.guard_margin"];

pub fn default_settings() -> BTreeMap<String, String> {
    DEFAULT_SETTINGS.clone()
}
pub fn compute_size_setting(provider: &str) -> String {
    COMPUTE_SIZE_SETTINGS
        .get(provider.trim())
        .cloned()
        .unwrap_or_else(|| "compute.azure.vm_size".into())
}
pub fn human_actor(actor: &str) -> bool {
    actor == HUMAN_AGENT || actor.starts_with("human:")
}
/// The SQL form of [`human_actor`] for `column`: the one definition of a
/// human actor the evolve gates (STOP, hard-stop, failed-experiment questions)
/// share, so an answer counts as human by the same rule everywhere. The
/// returned fragment is parenthesised and binds no parameters.
pub fn human_actor_sql(column: &str) -> String {
    format!("({column} = '{HUMAN_AGENT}' OR {column} LIKE 'human:%')")
}
pub fn setting_names() -> Vec<String> {
    sorted_keys(&DEFAULT_SETTINGS)
}
pub fn compute_setting_names() -> Vec<String> {
    sorted_keys(&COMPUTE_SETTINGS)
}
pub fn model_setting_names() -> Vec<String> {
    sorted_keys(&MODEL_SETTINGS)
}
pub fn harness_setting_names() -> Vec<String> {
    sorted_keys(&HARNESS_SETTINGS)
}
pub fn evolve_setting_names() -> Vec<String> {
    sorted_keys(&EVOLVE_SETTINGS)
}
pub fn daemon_setting_names() -> Vec<String> {
    sorted_keys(&DAEMON_SETTINGS)
}
pub fn lead_setting_names() -> Vec<String> {
    sorted_keys(&LEAD_SETTINGS)
}
pub fn owner_setting_names() -> Vec<String> {
    sorted_keys(&OWNER_SETTINGS)
}
pub fn extra_setting_names() -> Vec<String> {
    let mut n = model_setting_names();
    n.extend(harness_setting_names());
    n.extend(compute_setting_names());
    n.extend(evolve_setting_names());
    n.extend(daemon_setting_names());
    n.extend(lead_setting_names());
    n.extend(owner_setting_names());
    n
}
fn sorted_keys(m: &BTreeMap<String, String>) -> Vec<String> {
    m.keys().cloned().collect()
}
fn rule_sorted_keys(m: &BTreeMap<String, SettingRule>) -> Vec<String> {
    m.keys().cloned().collect()
}

/// The per-project incumbent pointer of the agent-program loop (ADR-049):
/// `evolve.incumbent.<slug>` names the variant every job of that project runs
/// unless the router picks a canary; unset or empty means `base`, the empty
/// overlay. It is a family keyed by project like `project.goal.<slug>`, so it
/// is not in the listed catalogue; promotion and rollback move it.
pub const INCUMBENT_PREFIX: &str = "evolve.incumbent.";
/// The variant a project runs while its incumbent pointer is unset.
pub const BASE_VARIANT: &str = "base";
/// The incumbent pointer setting of `slug`.
pub fn incumbent_setting(slug: &str) -> String {
    format!("{INCUMBENT_PREFIX}{slug}")
}
static INCUMBENT_SLUG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9._-]{0,127}$").unwrap());
static VARIANT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$").unwrap());
/// Whether `id` has the shape of a variant id (the pointer's value rule).
pub fn valid_variant_id(id: &str) -> bool {
    VARIANT_ID.is_match(id)
}
fn incumbent_rule(name: &str) -> Option<SettingRule> {
    let slug = name.strip_prefix(INCUMBENT_PREFIX)?;
    INCUMBENT_SLUG.is_match(slug).then(|| {
        SettingRule::new(
            "variant",
            None,
            &[],
            None,
            "the variant of the agent program this project's jobs run unless the router picks a canary (ADR-049); empty means base, the empty overlay; promotion and rollback move it",
        )
    })
}

/// Whether a change that touches a screen must bring its before/after
/// report (`hive report`) before it lands: `require`, `warn` or `off`.
pub const UI_REPORT: &str = "landing.ui_report";
/// Whether a measured tie lands (evolve.land_neutral).
pub const EVOLVE_LAND_NEUTRAL: &str = "evolve.land_neutral";
/// Cards a project may have planned per day that change the loop itself.
pub const PLAN_LOOP_PER_DAY: &str = "plan.loop_per_day";
/// Whether a card must name a check that turns from failing to passing.
pub const EVOLVE_REQUIRE_CHECK: &str = "evolve.require_check";
/// Whether a card that names a check skips the vote quorum.
pub const EVOLVE_CHECK_SKIPS_QUORUM: &str = "evolve.check_skips_quorum";
/// How many experiments one project may have in flight.
pub const EVOLVE_EXPERIMENTS_IN_FLIGHT: &str = "evolve.experiments_in_flight";
/// The per-project form of `landing.ui_report`: `landing.ui_report.<slug>`
/// takes the same values for one project and, unset, the project follows
/// the machine's setting. A family keyed by project like the incumbent
/// pointer, so it is not in the listed catalogue.
pub const UI_REPORT_PREFIX: &str = "landing.ui_report.";
/// The values `landing.ui_report` takes.
pub const UI_REPORT_MODES: &[&str] = &["require", "warn", "off"];
/// The per-project `landing.ui_report` setting of `slug`.
pub fn ui_report_setting(slug: &str) -> String {
    format!("{UI_REPORT_PREFIX}{slug}")
}
fn ui_report_rule(name: &str) -> Option<SettingRule> {
    let slug = name.strip_prefix(UI_REPORT_PREFIX)?;
    INCUMBENT_SLUG.is_match(slug).then(|| {
        SettingRule::new(
            "choice",
            None,
            UI_REPORT_MODES,
            None,
            "this project's own landing.ui_report: require, warn or off; unset, the project follows landing.ui_report",
        )
    })
}

fn known_setting(name: &str) -> Option<SettingRule> {
    if let Some(rule) = incumbent_rule(name).or_else(|| ui_report_rule(name)) {
        return Some(rule);
    }
    if DEFAULT_SETTINGS.contains_key(name) {
        Some(
            SETTING_RULES
                .get(name)
                .cloned()
                .unwrap_or_else(|| SettingRule::new("text", None, &[], None, "")),
        )
    } else {
        COMPUTE_RULES
            .get(name)
            .or_else(|| MODEL_RULES.get(name))
            .or_else(|| HARNESS_RULES.get(name))
            .or_else(|| EVOLVE_RULES.get(name))
            .or_else(|| DAEMON_RULES.get(name))
            .or_else(|| LEAD_RULES.get(name))
            .or_else(|| OWNER_RULES.get(name))
            .cloned()
    }
}
fn default_setting(name: &str) -> String {
    DEFAULT_SETTINGS
        .get(name)
        .or_else(|| COMPUTE_SETTINGS.get(name))
        .or_else(|| MODEL_SETTINGS.get(name))
        .or_else(|| HARNESS_SETTINGS.get(name))
        .or_else(|| EVOLVE_SETTINGS.get(name))
        .or_else(|| DAEMON_SETTINGS.get(name))
        .or_else(|| LEAD_SETTINGS.get(name))
        .or_else(|| OWNER_SETTINGS.get(name))
        .cloned()
        .unwrap_or_default()
}

pub fn secret_shaped(value: &str) -> bool {
    static RES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        vec![
            Regex::new(r"-----BEGIN").unwrap(),
            Regex::new(r"://[^/\s:@]+:[^@\s]+@").unwrap(),
            Regex::new(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}").unwrap(),
            Regex::new(r"\bAKIA[0-9A-Z]{12,}").unwrap(),
            Regex::new(r"\b(?:ghp_|github_pat_|sk-)[A-Za-z0-9_-]{8,}").unwrap(),
        ]
    });
    if RES.iter().any(|r| r.is_match(value)) {
        return true;
    }
    // A long run of token characters is a credential unless the word it sits
    // in is a path or a URL. The board's daily vocabulary is full of long
    // repository-relative paths (`src/web-app/lib/use-context-intake`), so
    // the word is judged, not the whole sentence, and a word made of
    // slash-separated segments is a path wherever it starts - provided it
    // has three or more segments or a `-`, `_` or `.` in it, which a base64
    // string never has next to a `/`.
    static LONG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/_=-]{40,}").unwrap());
    static PATH_OR_URL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(/|\./|\.\./|~|\$|[A-Za-z][A-Za-z0-9+.-]*://[^@\s]*$)").unwrap()
    });
    static SEGMENTS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_.@-]+(/[A-Za-z0-9_.@-]+)+/?$").unwrap());
    // A store id - a post (`20261012T092238Z-copilot-fable-1012-hub-2`), a
    // job (`job-20261029T090659Z-06fae0`), a run, a session - is a UTC stamp
    // followed by the agent's name, and the name alone can carry it past the
    // token length. No credential begins with a timestamp, so the stamp
    // marks the word as an id wherever the text quotes one.
    static STORE_ID: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:[a-z]+-)?[0-9]{8}T[0-9]{6}Z(?:-[A-Za-z0-9_.-]+)+$").unwrap()
    });
    // A shell assignment (`CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/a/long/path}`)
    // is judged by its value: the name is vocabulary, the value may be a
    // path, a `$VAR` or a token. A name as long as a token is no name - a
    // base64 string ends in `=` too.
    static ASSIGNMENT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^([A-Za-z_][A-Za-z0-9_]{0,38})=(.*)$").unwrap());
    // A snake_case identifier - a Rust test name such as
    // `an_idle_sleep_under_a_hold_wakes_when_the_load_drops`, alone or under
    // its module path (`daemon::tests::...`) - is code vocabulary however
    // long it runs: lowercase words joined by `_` or `::`, four or more of
    // them, none longer than a word. A token is mixed-case or one long run.
    static IDENTIFIER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[a-z][a-z0-9]{0,19}(?:(?:_|::)[a-z0-9]{1,20}){3,}$").unwrap()
    });
    if !LONG.is_match(value) {
        return false;
    }
    let path_like = |word: &str| {
        PATH_OR_URL.is_match(word)
            || STORE_ID.is_match(word)
            || IDENTIFIER.is_match(word)
            || (SEGMENTS.is_match(word)
                && (word.matches('/').count() >= 2 || word.contains(['-', '_', '.'])))
    };
    value.split_whitespace().any(|word| {
        let word = word.trim_matches(|c: char| {
            matches!(
                c,
                '`' | '\''
                    | '"'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | ','
                    | ';'
                    | ':'
                    | '.'
            )
        });
        let word = ASSIGNMENT
            .captures(word)
            .and_then(|c| c.get(2))
            .map_or(word, |v| v.as_str().trim_matches(['"', '\'']));
        // A Markdown rule or a table's divider is a run of `-`, `=` or `_`
        // as long as a token; no credential is made of punctuation alone.
        LONG.is_match(word) && word.chars().any(|c| c.is_ascii_alphanumeric()) && !path_like(word)
    })
}
fn py_repr(s: &str) -> String {
    if s.contains('\'') && !s.contains('"') {
        format!("\"{s}\"")
    } else {
        format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}
pub fn majority(value: &str) -> Result<(i64, i64)> {
    let re = Regex::new(r"^\s*(\d+)\s*/\s*(\d+)\s*$").unwrap();
    let Some(c) = re.captures(value) else {
        return Err(input_err(format!(
            "A majority must be a fraction such as 2/3, not {}",
            py_repr(value)
        )));
    };
    let n: i64 = c[1].parse().unwrap();
    let d: i64 = c[2].parse().unwrap();
    if d == 0 {
        return Err(input_err(format!(
            "A majority must be a fraction such as 2/3, not {}",
            py_repr(value)
        )));
    }
    Ok((n, d))
}

/// `hub.allow_remote`: empty or `off` turns it off; otherwise an https
/// origin - scheme, host and an optional port, nothing more - lower-cased,
/// with a trailing slash dropped.
fn allow_remote_origin(text: &str) -> Result<String> {
    if text.is_empty() || text.eq_ignore_ascii_case("off") {
        return Ok(String::new());
    }
    let refuse = || {
        input_err(format!(
            "hub.allow_remote takes the https origin of a front you run, such as https://team.example.com, or empty to keep the console 127.0.0.1-only; not {}",
            py_repr(text)
        ))
    };
    let lower = text.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .ok_or_else(refuse)?
        .trim_end_matches('/');
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (rest, None),
    };
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if !host.contains('.') || !host.split('.').all(label_ok) {
        return Err(refuse());
    }
    if let Some(p) = port
        && !p.parse::<u16>().is_ok_and(|n| n > 0)
    {
        return Err(refuse());
    }
    Ok(format!("https://{rest}"))
}

/// `hub.hosts`: empty or `off` lists no name; otherwise host names separated
/// by commas - letters, digits, dots and dashes, with no scheme, port or
/// path - stored lower-cased, each once, as `a, b`.
fn hub_host_names(text: &str) -> Result<String> {
    if text.is_empty() || text.eq_ignore_ascii_case("off") {
        return Ok(String::new());
    }
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    let mut names: Vec<String> = Vec::new();
    for name in text.split(',') {
        let name = name.trim().to_ascii_lowercase();
        if name.len() > 253 || !name.split('.').all(label_ok) {
            return Err(input_err(format!(
                "hub.hosts takes host names separated by commas, such as hive.example.com, console.example.org (letters, digits, dots and dashes; no scheme, no port, no path), or empty for localhost and IP addresses only; not {}",
                py_repr(text)
            )));
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    Ok(names.join(", "))
}

pub fn validate_setting(name: &str, value: &str) -> Result<String> {
    if AUTOMATIC_SETTINGS.contains(&name) {
        return Err(input_err(format!(
            "{name} is automatic now: each machine works out its runs from its cores and memory and the fleet adds them up (hive status shows the numbers); set capacity to cap this machine, or hive stop to pause"
        )));
    }
    let Some(rule) = known_setting(name) else {
        let msg = if name.starts_with("compute.") {
            format!(
                "Unknown setting {}. Compute settings: {}",
                py_repr(name),
                compute_setting_names().join(", ")
            )
        } else if name.starts_with("harness.") {
            format!(
                "Unknown setting {}. Harness settings: {}",
                py_repr(name),
                rule_sorted_keys(&HARNESS_RULES).join(", ")
            )
        } else if name.starts_with(INCUMBENT_PREFIX) {
            format!(
                "Unknown setting {}. The incumbent pointer is {INCUMBENT_PREFIX}<project slug>, the slug in lower case",
                py_repr(name)
            )
        } else if name.starts_with("evolve.") {
            format!(
                "Unknown setting {}. Evolve settings: {}, {INCUMBENT_PREFIX}<project slug>",
                py_repr(name),
                evolve_setting_names().join(", ")
            )
        } else if name.starts_with("owner.") {
            format!(
                "Unknown setting {}. Owner settings: {}",
                py_repr(name),
                owner_setting_names().join(", ")
            )
        } else {
            format!(
                "Unknown setting {}. Known: {}",
                py_repr(name),
                setting_names().join(", ")
            )
        };
        return Err(input_err(msg));
    };
    let text = value.trim();
    if name == "voice.language" {
        // `auto` or a BCP 47-shaped tag (de, de-DE, haw-US); whether the
        // model has a prompt for it is the voice engine's check, which
        // names the tags it knows.
        let shape = Regex::new(r"^(?i)[a-z]{2,3}(-[a-z]{2})?$").unwrap();
        if text.eq_ignore_ascii_case("auto") {
            return Ok("auto".into());
        }
        if !shape.is_match(text) {
            return Err(input_err(format!(
                "voice.language takes auto or a language tag such as en-US, de-DE or ja-JP, not {}",
                py_repr(text)
            )));
        }
        let (lang, region) = text.split_once('-').unwrap_or((text, ""));
        return Ok(if region.is_empty() {
            lang.to_ascii_lowercase()
        } else {
            format!(
                "{}-{}",
                lang.to_ascii_lowercase(),
                region.to_ascii_uppercase()
            )
        });
    }
    if name == "owner.name" {
        // One line, single-spaced, at most OWNER_NAME_MAX characters; empty
        // is the default and means the console says "you".
        let name_text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if name_text.chars().count() > OWNER_NAME_MAX {
            return Err(input_err(format!(
                "owner.name is a display name of at most {OWNER_NAME_MAX} characters, not {}",
                py_repr(&name_text)
            )));
        }
        return Ok(name_text);
    }
    if name == "git.github_app.app_id" || name == "git.github_app.installation_id" {
        // GitHub numbers its Apps and installations; anything else would
        // only fail later, at the first token exchange.
        if !text.is_empty() && (text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit())) {
            return Err(input_err(format!(
                "{name} takes the number GitHub shows for it, such as 123456, or empty for no App; not {}",
                py_repr(text)
            )));
        }
        return Ok(text.to_string());
    }
    if name == "hub.allow_remote" {
        return allow_remote_origin(text);
    }
    if name == "hub.hosts" {
        return hub_host_names(text);
    }
    if name == "update.latest_url" {
        let url = text.to_string();
        if url.is_empty() {
            return Ok(url);
        }
        if !(url.starts_with("https://") || url.starts_with("http://"))
            || url.len() <= "https://".len()
            || url.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(input_err(format!(
                "update.latest_url takes an http(s) URL that answers JSON, such as https://api.github.com/repos/OWNER/REPO/releases/latest, or empty for no check; not {}",
                py_repr(text)
            )));
        }
        if secret_shaped(&url) {
            return Err(input_err(
                "update.latest_url looks like it carries a credential; Hive never stores secrets - use a public URL",
            ));
        }
        return Ok(url);
    }
    if name == "capacity" {
        let lower = text.to_ascii_lowercase();
        if lower.is_empty() || lower == "auto" || lower == "off" {
            return Ok("auto".into());
        }
        return match lower.parse::<i64>() {
            Ok(n) if n >= 0 => Ok(n.to_string()),
            _ => Err(input_err(format!(
                "capacity takes auto or a whole number of runs (0 keeps this machine idle), not {}",
                py_repr(text)
            ))),
        };
    }
    if name == "harness.system_prompt" && text.contains("{{if}}") {
        return Err(input_err(
            "harness.system_prompt is not a valid Go text/template: template: prompt:1: missing value for if",
        ));
    }
    match rule.kind.as_str() {
        "count" | "count-or-none" | "credits" => {
            let lower = text.to_ascii_lowercase();
            if rule.kind == "count-or-none" && NONE_WORDS.contains(&lower.as_str()) {
                return Ok(lower);
            }
            let n: i64 = text.parse().map_err(|_| {
                let extra = if rule.kind == "count-or-none" {
                    ", or one of none/off/unlimited"
                } else {
                    ""
                };
                input_err(format!(
                    "{name} takes a whole number{extra}, not {}",
                    py_repr(text)
                ))
            })?;
            if rule.has_min && n < rule.min {
                return Err(input_err(format!(
                    "{name} must be at least {}, not {n}",
                    rule.min
                )));
            }
            if PERCENT_SETTINGS.contains(&name) && rule.has_max && n as f64 > rule.max {
                return Err(input_err(format!(
                    "{name} is a percent: it cannot exceed {}, not {n}",
                    trim_float(rule.max)
                )));
            }
            if name == "evolve.flat_cycles" && n == 1 {
                return Err(input_err(
                    "evolve.flat_cycles is a streak of at least 2 tied cycles, or 0 to turn the gate off; not 1",
                ));
            }
            Ok(n.to_string())
        }
        "fraction" => {
            let (n, d) = majority(text)?;
            if n > d {
                return Err(input_err(format!(
                    "{name} is a share of the votes: the numerator cannot exceed the denominator ({text})"
                )));
            }
            Ok(format!("{n}/{d}"))
        }
        "decimal" => {
            let f: f64 = text.parse().map_err(|_| {
                input_err(format!(
                    "{name} takes a decimal number such as 0.02, not {}",
                    py_repr(text)
                ))
            })?;
            if !f.is_finite() {
                return Err(input_err(format!(
                    "{name} takes a decimal number such as 0.02, not {}",
                    py_repr(text)
                )));
            }
            if f < 0.0 {
                return Err(input_err(format!("{name} cannot be negative, not {text}")));
            }
            if rule.has_max && f > rule.max {
                return Err(input_err(format!(
                    "{name} is a share: it cannot exceed {}, not {text}",
                    trim_float(rule.max)
                )));
            }
            Ok(trim_float(f))
        }
        "number" => {
            let f: f64 = text
                .parse()
                .map_err(|_| input_err(format!("{name} takes a number, not {}", py_repr(text))))?;
            if !f.is_finite() {
                return Err(input_err(format!(
                    "{name} takes a number, not {}",
                    py_repr(text)
                )));
            }
            if rule.has_min && f < rule.min as f64 {
                return Err(input_err(format!(
                    "{name} must be at least {}, not {text}",
                    rule.min
                )));
            }
            Ok(trim_float(f))
        }
        "switch" | "choice" => {
            let lower = text.to_ascii_lowercase();
            if rule.choices.iter().any(|c| c == &lower) {
                Ok(lower)
            } else {
                let choices: Vec<&str> = rule
                    .choices
                    .iter()
                    .map(|c| if c.is_empty() { "empty" } else { c.as_str() })
                    .collect();
                Err(input_err(format!(
                    "{name} takes one of {}, not {}",
                    choices.join(", "),
                    py_repr(text)
                )))
            }
        }
        "variant" => {
            if text.is_empty() || VARIANT_ID.is_match(text) {
                Ok(text.into())
            } else {
                Err(input_err(format!(
                    "{name} takes a variant id such as base or one `hive evolve variant list` shows, or empty for base; not {}",
                    py_repr(text)
                )))
            }
        }
        "command" => {
            if text.is_empty() {
                Err(input_err(format!("{name} cannot be empty")))
            } else {
                Ok(text.into())
            }
        }
        "identifier" => {
            if secret_shaped(text) {
                Err(input_err(format!(
                    "{name} looks like a credential; Hive never stores secrets - authenticate on this machine (az login, aws configure, gcloud auth login) and keep the setting to an identifier"
                )))
            } else {
                Ok(text.into())
            }
        }
        "reference" => {
            if text.is_empty() {
                return Ok(String::new());
            }
            if hive_cred::is_private_key(text.as_bytes()) || secret_shaped(text) {
                return Err(input_err(format!(
                    "{name} takes a reference to the key, never the key itself (ADR-041): {}",
                    hive_cred::SHAPES
                )));
            }
            let r = hive_cred::parse(text).map_err(|e| input_err(format!("{name}: {e}")))?;
            Ok(r.to_string())
        }
        _ => Ok(text.into()),
    }
}
fn trim_float(f: f64) -> String {
    let s = format!("{f}");
    s
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct SettingRow {
    pub name: String,
    pub value: String,
    pub updated_at: String,
    pub updated_by: String,
}

impl Store {
    pub fn run_model(&self, kind: &str) -> String {
        if IMPLEMENTATION_JOB_KINDS.contains(&kind) {
            self.setting("worker_model", "").trim().to_string()
        } else {
            self.light_model()
        }
    }

    /// The model for a run of `kind` whose role asks for a model class of
    /// its own: `worker` runs on the worker model, `light` on the light one,
    /// anything else on the kind's own.
    pub fn run_model_class(&self, kind: &str, class: &str) -> String {
        match class.trim() {
            "worker" => self.setting("worker_model", "").trim().to_string(),
            "light" => self.light_model(),
            _ => self.run_model(kind),
        }
    }

    /// The step budget for one run of `kind`, in the shape of the per-kind
    /// model settings: a task, improve or experiment run may raise its budget
    /// with `harness.task_max_steps` without giving the short kinds the same,
    /// and every kind falls back to `harness.max_steps` while that is empty.
    pub fn run_max_steps(&self, kind: &str) -> String {
        let per_kind = self.setting("harness.task_max_steps", "");
        let per_kind = per_kind.trim();
        let override_set =
            !per_kind.is_empty() && !NONE_WORDS.contains(&per_kind.to_ascii_lowercase().as_str());
        if IMPLEMENTATION_JOB_KINDS.contains(&kind) && override_set {
            per_kind.to_string()
        } else {
            self.setting("harness.max_steps", "")
        }
    }

    pub fn light_model(&self) -> String {
        let light = self.setting("light_model", "").trim().to_string();
        if light.is_empty() {
            self.setting("worker_model", "").trim().to_string()
        } else {
            light
        }
    }
    /// The model of the decide role (ADR-072, Amendment 1): `decide_model`,
    /// else `measure_model`, else `light_model` (which itself falls back to
    /// `worker_model`). Empty when none is set: the provider's default.
    pub fn decide_model(&self) -> String {
        for name in ["decide_model", "measure_model"] {
            let v = self.setting(name, "").trim().to_string();
            if !v.is_empty() {
                return v;
            }
        }
        self.light_model()
    }
    /// The provider of the decide role: `decide_provider`, else the hub's
    /// own `provider`.
    pub fn decide_provider(&self) -> String {
        let own = self.setting("decide_provider", "").trim().to_string();
        if !own.is_empty() {
            return own;
        }
        let main = self.setting("provider", "").trim().to_string();
        if main.is_empty() {
            "copilot".into()
        } else {
            main
        }
    }
    /// `decide.mode`: off, shadow or on; a value the store does not know
    /// reads as the default, on.
    pub fn decide_mode(&self) -> String {
        let v = self.setting("decide.mode", "").trim().to_ascii_lowercase();
        match v.as_str() {
            "off" | "shadow" | "on" => v,
            _ => "on".into(),
        }
    }
    /// The model the loop measures fixtures with: `measure_model`, or
    /// `worker_model` while that is empty.
    pub fn measure_model(&self) -> String {
        let measure = self.setting("measure_model", "").trim().to_string();
        if measure.is_empty() {
            self.setting("worker_model", "").trim().to_string()
        } else {
            measure
        }
    }

    /// The reasoning effort for a run of `kind`: `worker_reasoning` for the
    /// kinds that change code, `light_reasoning` for the rest, and
    /// `harness.reasoning` while the role's own is empty. `default` (or
    /// empty) means the run sends nothing and the model decides.
    pub fn run_reasoning(&self, kind: &str) -> String {
        let role = if IMPLEMENTATION_JOB_KINDS.contains(&kind) {
            "worker_reasoning"
        } else {
            "light_reasoning"
        };
        self.reasoning_or_default(role)
    }

    /// [`Store::run_reasoning`] for a run whose role asks for a model class
    /// of its own (`worker`, `light`), in the shape of
    /// [`Store::run_model_class`].
    pub fn run_reasoning_class(&self, kind: &str, class: &str) -> String {
        match class.trim() {
            "worker" => self.reasoning_or_default("worker_reasoning"),
            "light" => self.reasoning_or_default("light_reasoning"),
            _ => self.run_reasoning(kind),
        }
    }

    /// The reasoning effort the loop measures fixtures with:
    /// `measure_reasoning`, then `worker_reasoning`, then
    /// `harness.reasoning`.
    pub fn measure_reasoning(&self) -> String {
        let measure = self.setting("measure_reasoning", "").trim().to_string();
        if measure.is_empty() {
            self.reasoning_or_default("worker_reasoning")
        } else {
            measure
        }
    }

    fn reasoning_or_default(&self, role: &str) -> String {
        let own = self.setting(role, "").trim().to_string();
        if own.is_empty() {
            self.setting("harness.reasoning", "default")
                .trim()
                .to_string()
        } else {
            own
        }
    }
    pub fn provider_settings(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for n in SETTING_RULES.keys().chain(HARNESS_RULES.keys()) {
            if n.starts_with("provider.") {
                out.insert(n.clone(), self.setting(n, ""));
            }
        }
        out
    }
    pub fn setting(&self, key: &str, def: &str) -> String {
        if let Ok(conn) = self.conn()
            && let Ok(Some(v)) = conn
                .query_row("SELECT value FROM settings WHERE name = ?", [key], |r| {
                    r.get::<_, String>(0)
                })
                .optional()
        {
            return v;
        }
        if !def.is_empty() {
            def.to_string()
        } else {
            default_setting(key)
        }
    }
    /// `landing.ui_report` for project `slug`: its own
    /// `landing.ui_report.<slug>` when set, else the machine's; a value the
    /// store does not know reads as the default, `require`.
    pub fn ui_report_mode(&self, slug: &str) -> String {
        let own = if slug.trim().is_empty() {
            String::new()
        } else {
            self.setting(&ui_report_setting(slug.trim()), "-")
        };
        let v = if own.trim().is_empty() || own == "-" {
            self.setting(UI_REPORT, "")
        } else {
            own
        };
        let v = v.trim().to_ascii_lowercase();
        if UI_REPORT_MODES.contains(&v.as_str()) {
            v
        } else {
            "require".into()
        }
    }
    pub fn setting_int(&self, key: &str) -> i64 {
        self.setting(key, "")
            .parse()
            .unwrap_or_else(|_| default_setting(key).parse().unwrap_or(0))
    }
    pub fn setting_bool(&self, key: &str) -> bool {
        matches!(self.setting(key, "").as_str(), "1" | "true" | "on" | "yes")
    }
    pub fn setting_float(&self, key: &str) -> f64 {
        self.setting(key, "")
            .parse()
            .unwrap_or_else(|_| default_setting(key).parse().unwrap_or(0.0))
    }
    pub fn owner_away(&self) -> bool {
        let Ok(conn) = self.conn() else {
            return false;
        };
        let row: rusqlite::Result<(String, String)> = conn.query_row(
            "SELECT value, updated_by FROM settings WHERE name = 'owner.presence'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        );
        matches!(row, Ok((ref v, ref by)) if v==OWNER_AWAY && human_actor(by))
    }
    /// The front's origin when the owner set `hub.allow_remote` (ADR-070
    /// rule 5); None while it is empty or an agent was the last to set it,
    /// so no agent can open the console to the front.
    pub fn hub_allow_remote(&self) -> Option<String> {
        let conn = self.conn().ok()?;
        let (value, by): (String, String) = conn
            .query_row(
                "SELECT value, updated_by FROM settings WHERE name = 'hub.allow_remote'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()?;
        (!value.is_empty() && human_actor(&by)).then_some(value)
    }
    /// The host names the owner listed in `hub.hosts`, lower-cased; none
    /// while it is empty or an agent was the last to set it, so no agent
    /// can add a name the console answers to.
    pub fn hub_hosts(&self) -> Vec<String> {
        let Ok(conn) = self.conn() else {
            return Vec::new();
        };
        let row: rusqlite::Result<(String, String)> = conn.query_row(
            "SELECT value, updated_by FROM settings WHERE name = 'hub.hosts'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        );
        match row {
            Ok((value, by)) if human_actor(&by) => value
                .split(',')
                .map(|name| name.trim().to_ascii_lowercase())
                .filter(|name| !name.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    }
    pub fn settings(&self) -> Result<BTreeMap<String, String>> {
        let rows = self.setting_rows()?;
        Ok(rows.into_iter().map(|r| (r.name, r.value)).collect())
    }
    pub fn setting_rows(&self) -> Result<Vec<SettingRow>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT name, value, updated_at, updated_by FROM settings ORDER BY name")?;
        let rows = stmt.query_map([], |r| {
            Ok(SettingRow {
                name: r.get(0)?,
                value: r.get(1)?,
                updated_at: r.get(2)?,
                updated_by: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
    pub fn set_setting(&self, actor: &str, key: &str, value: &str) -> Result<()> {
        let normalized = validate_setting(key, value)?;
        let tx = self.begin()?;
        tx.execute("INSERT INTO settings (name, value, updated_at, updated_by) VALUES (?, ?, ?, ?) ON CONFLICT(name) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at, updated_by = excluded.updated_by", params![key, normalized, self.now(), actor])?;
        tx.event(actor, "setting", key, normalized.as_str())?;
        tx.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, Store};

    #[test]
    fn test_settings() {
        let _serial = crate::test_helpers::serial();
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(Options {
            home: dir.path().display().to_string(),
            create: true,
            now: "2026-09-06T10:00:00Z".into(),
            ..Options::default()
        })
        .unwrap();
        assert_eq!(s.setting("jobs_per_day", ""), "2");
        assert_eq!(validate_setting("jobs_per_day", " none ").unwrap(), "none");
        assert!(
            validate_setting("amend_majority", "0.05")
                .unwrap_err()
                .to_string()
                .contains("A majority must be a fraction")
        );
        s.set_setting("human", "evolve.tolerance", "1e-3").unwrap();
        assert_eq!(s.setting("evolve.tolerance", ""), "0.001");
        s.set_setting("human", "owner.presence", "away").unwrap();
        assert!(s.owner_away());
        s.set_setting("agent", "owner.presence", "away").unwrap();
        assert!(!s.owner_away());
        assert_eq!(compute_size_setting("moon"), "compute.azure.vm_size");
        assert!(secret_shaped("ghp_abcdefghijklmnop"));
    }

    #[test]
    fn hub_allow_remote_takes_an_https_origin_and_counts_only_from_the_owner() {
        let _serial = crate::test_helpers::serial();
        assert_eq!(default_setting("hub.allow_remote"), "");
        assert!(daemon_setting_names().contains(&"hub.allow_remote".to_string()));
        for (given, want) in [
            ("", ""),
            ("off", ""),
            (" https://Team.Example.com/ ", "https://team.example.com"),
            (
                "https://hive.example.co.uk:8443",
                "https://hive.example.co.uk:8443",
            ),
        ] {
            assert_eq!(
                validate_setting("hub.allow_remote", given).unwrap(),
                want,
                "{given}"
            );
        }
        for bad in [
            "http://team.example.com",
            "https://team.example.com/path",
            "https://user@team.example.com",
            "https://localhost",
            "https://team.example.com:0",
            "https://-bad.example.com",
            "team.example.com",
        ] {
            let err = validate_setting("hub.allow_remote", bad)
                .unwrap_err()
                .to_string();
            assert!(err.contains("https origin"), "{bad}: {err}");
        }
        unsafe { std::env::set_var("HIVE_JOB", "") };
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(Options {
            home: dir.path().display().to_string(),
            create: true,
            ..Options::default()
        })
        .unwrap();
        assert_eq!(s.hub_allow_remote(), None);
        s.set_setting("cinder", "hub.allow_remote", "https://team.example.com")
            .unwrap();
        assert_eq!(s.hub_allow_remote(), None, "an agent cannot open the hub");
        s.set_setting("human", "hub.allow_remote", "https://team.example.com")
            .unwrap();
        assert_eq!(
            s.hub_allow_remote().as_deref(),
            Some("https://team.example.com")
        );
        s.set_setting("human", "hub.allow_remote", "").unwrap();
        assert_eq!(s.hub_allow_remote(), None);
    }

    #[test]
    fn hub_hosts_takes_host_names_and_counts_only_from_the_owner() {
        let _serial = crate::test_helpers::serial();
        assert_eq!(default_setting("hub.hosts"), "");
        assert!(daemon_setting_names().contains(&"hub.hosts".to_string()));
        assert_eq!(DAEMON_RULES["hub.hosts"].kind, "text");
        for (given, want) in [
            ("", ""),
            ("off", ""),
            (
                "hive.example.com, console.example.org",
                "hive.example.com, console.example.org",
            ),
            (
                " console.example.org ,hive.example.com,console.example.org ",
                "console.example.org, hive.example.com",
            ),
            // Upper case is not refused: the name is stored lower-cased.
            ("EVIL.example", "evil.example"),
            ("hive", "hive"),
        ] {
            assert_eq!(
                validate_setting("hub.hosts", given).unwrap(),
                want,
                "{given}"
            );
        }
        for bad in [
            "https://x",
            "x:8765",
            "x/y",
            "*.example.com",
            "hive..example.com",
            "-hive.example.com",
            "hive.example.com,",
            "hive example.com",
            "hive_box",
            "[::1]",
        ] {
            let err = validate_setting("hub.hosts", bad).unwrap_err().to_string();
            assert!(
                err.contains("host names separated by commas, such as hive.example.com"),
                "{bad}: {err}"
            );
        }
        unsafe { std::env::set_var("HIVE_JOB", "") };
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(Options {
            home: dir.path().display().to_string(),
            create: true,
            ..Options::default()
        })
        .unwrap();
        assert!(s.hub_hosts().is_empty());
        s.set_setting("cinder", "hub.hosts", "hive.example.com")
            .unwrap();
        assert!(s.hub_hosts().is_empty(), "an agent cannot add a name");
        s.set_setting(
            "human",
            "hub.hosts",
            "Hive.Example.com, console.example.org",
        )
        .unwrap();
        assert_eq!(s.hub_hosts(), ["hive.example.com", "console.example.org"]);
        s.set_setting("cinder", "hub.hosts", "hive.example.com, evil.example")
            .unwrap();
        assert!(
            s.hub_hosts().is_empty(),
            "an agent's value does not count, whatever the owner listed before"
        );
        s.set_setting("human:dashboard", "hub.hosts", "hive.example.com")
            .unwrap();
        assert_eq!(s.hub_hosts(), ["hive.example.com"]);
        s.set_setting("human", "hub.hosts", "").unwrap();
        assert!(s.hub_hosts().is_empty());
    }

    #[test]
    fn voice_settings_take_a_model_id_and_a_language_tag() {
        assert_eq!(
            default_setting("voice.model"),
            "nemotron-3.5-asr-streaming-0.6b"
        );
        assert_eq!(default_setting("voice.language"), "auto");
        assert_eq!(
            validate_setting("voice.model", " nemotron-speech-streaming-en-0.6b ").unwrap(),
            "nemotron-speech-streaming-en-0.6b"
        );
        let err = validate_setting("voice.model", "whisper")
            .unwrap_err()
            .to_string();
        assert!(err.contains("voice.model takes one of nemotron-3.5-asr-streaming-0.6b, nemotron-speech-streaming-en-0.6b"), "{err}");
        assert_eq!(validate_setting("voice.language", "AUTO").unwrap(), "auto");
        assert_eq!(
            validate_setting("voice.language", "de-de").unwrap(),
            "de-DE"
        );
        assert_eq!(validate_setting("voice.language", "EN").unwrap(), "en");
        assert_eq!(
            validate_setting("voice.language", "haw-US").unwrap(),
            "haw-US"
        );
        for bad in ["", "english", "en_US", "en-USA", "en-US-x"] {
            let err = validate_setting("voice.language", bad)
                .unwrap_err()
                .to_string();
            assert!(err.contains("takes auto or a language tag"), "{bad}: {err}");
        }
    }

    #[test]
    fn secret_shape_spares_paths_and_keeps_tokens() {
        // Repository-relative paths, alone or inside a sentence, are the
        // board's vocabulary, not credentials - however long they run.
        for text in [
            "src/web-fachsprachtrainer/lib/use-context-intake",
            "Infrastructure/Voice/Realtime/AzureRealtimeApiClient",
            "File: src/web-fachsprachtrainer/components/user-menu.tsx (the menu)",
            "parity of `regions/redemittel/minutes/dashboard/recall-cards` with the FSP shim",
            "/usr/local/bin:/opt/homebrew/bin/some/very/long/path/that/keeps/going/on",
            "https://example.test/a/very/long/path/that/keeps/going/on/and/on?x=1",
            "env CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/Users/me/.hive/cache/cargo-target-land/my-project} cargo test",
            "PATH=\"/opt/homebrew/opt/node@22/bin:/opt/homebrew/opt/postgresql@16/bin:$PATH\"",
        ] {
            assert!(!secret_shaped(text), "{text}");
        }
        // Long opaque tokens stay credentials, in a sentence or next to a slash.
        for text in [
            "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789AbCdEfGh",
            "token AbCdEfGhIjKlMnOpQrStUvWxYz0123456789AbCdEfGh= set",
            "AbCdEfGhIjKlMnOpQrSt/UvWxYz0123456789AbCdEfGh",
            "export API_TOKEN=AbCdEfGhIjKlMnOpQrStUvWxYz0123456789AbCdEfGh",
            "API_TOKEN='AbCdEfGhIjKlMnOpQrStUvWxYz0123456789AbCdEfGh'",
            "-----BEGIN RSA PRIVATE KEY-----",
            "https://user:pass@example.test/",
        ] {
            assert!(secret_shaped(text), "{text}");
        }
    }

    #[test]
    fn secret_shape_spares_rules_made_of_punctuation() {
        // A SKILL.md with a horizontal rule or a wide table divider held
        // three of kp-and-fsp's repository skills as "a credential".
        for text in [
            &"-".repeat(70),
            &format!("## Steps\n\n{}\n\nThen run it.", "-".repeat(175)),
            &format!("|{}|{}|", "-".repeat(44), "-".repeat(12)),
            &"=".repeat(60),
            &"_".repeat(48),
        ] {
            assert!(!secret_shaped(text), "{text}");
        }
        // One letter among the punctuation is still judged as a token.
        assert!(secret_shaped(&format!(
            "{}a{}",
            "-".repeat(30),
            "-".repeat(30)
        )));
    }

    #[test]
    fn secret_shape_spares_store_ids_however_long_the_agent_name() {
        // A post id is the stamp plus the agent's name; an agent called
        // `copilot-fable-1012-hub-2` carries it to 41 characters, past the
        // token length, and the experiment archive refused the row for it
        // (the lane re-measured that experiment 32 times for nothing).
        for text in [
            "20261012T092238Z-copilot-fable-1012-hub-2",
            "Signal: 20261012T092238Z-copilot-fable-1012-hub-2 (the card)",
            "job-20261029T090659Z-06fae0-amistio-template-lead-docs",
            "run-20261030T010059Z-ce363e-fable-fleet-1006-b-3",
            "`20261030T042815Z-fable-cli-1029-rsi-2`, then 20261030T042831Z-amistio-template-lead-docs.",
        ] {
            assert!(!secret_shaped(text), "{text}");
        }
        // A stamp does not launder what follows it when that is no id.
        for text in [
            "20261012T092238Z-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789AbCdEfGh=",
            "20261012T092238Z AbCdEfGhIjKlMnOpQrStUvWxYz0123456789AbCdEfGh",
            "2026101T092238Z-copilot-fable-1012-hub-2-and-a-longer-tail",
        ] {
            assert!(secret_shaped(text), "{text}");
        }
    }

    #[test]
    fn secret_shape_spares_snake_case_identifiers() {
        // Experiment card 20261103T200329Z-sable-3-2 named a 52-character
        // test in its Variable and Signal lines; the planner refused it as
        // "experiment card must not contain credentials" and the whole
        // experiment lane waited on it.
        for text in [
            "have an_idle_sleep_under_a_hold_wakes_when_the_load_drops wait for a wake",
            "check:cargo test -p hive-runner --lib an_idle_sleep_under_a_hold_wakes_when_the_load_drops",
            "daemon::tests::restart_drain_holds_queued_work_while_the_run_in_flight_finishes",
            "`a_project_conversation_reports_its_branch_and_attach_validates`",
        ] {
            assert!(!secret_shaped(text), "{text}");
        }
        // Tokens stay tokens: mixed case, one long lowercase run, or a
        // Stripe-style key with a long opaque tail.
        for text in [
            "abcdefghijklmnopqrstuvwxyz0123456789abcd",
            "sk_live_51habcdefghijklmnopqrstuvwxyz0123456789",
            "an_idle_sleep_under_a_hold_AbCdEfGhIjKlMnOpQrSt",
            "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789AbCdEfGh",
        ] {
            assert!(secret_shaped(text), "{text}");
        }
    }
}

#[cfg(test)]
mod go_settings_tests {
    use super::*;
    use crate::{Options, Store};
    use std::fs;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static TEST_LOCK: Mutex<()> = Mutex::new(());
    fn set_job_empty() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap();
        unsafe { std::env::set_var("HIVE_JOB", "") };
        guard
    }
    struct TestStore {
        _dir: TempDir,
        store: Store,
    }
    impl std::ops::Deref for TestStore {
        type Target = Store;
        fn deref(&self) -> &Self::Target {
            &self.store
        }
    }
    fn store(now: &str) -> TestStore {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(Options {
            home: dir.path().display().to_string(),
            create: true,
            now: now.into(),
            ..Options::default()
        })
        .unwrap();
        TestStore { _dir: dir, store }
    }

    #[test]
    fn decimal_setting() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-09-06T10:00:00Z");
        for (input, want) in [
            ("0.05", "0.05"),
            (" .5 ", "0.5"),
            ("0", "0"),
            ("1e-3", "0.001"),
            ("1", "1"),
        ] {
            s.set_setting("human", "evolve.tolerance", input).unwrap();
            assert_eq!(s.setting("evolve.tolerance", ""), want);
        }
        for (input, want) in [
            (
                "abc",
                "evolve.tolerance takes a decimal number such as 0.02, not 'abc'",
            ),
            (
                "NaN",
                "evolve.tolerance takes a decimal number such as 0.02, not 'NaN'",
            ),
            (
                "Inf",
                "evolve.tolerance takes a decimal number such as 0.02, not 'Inf'",
            ),
            ("-0.1", "evolve.tolerance cannot be negative, not -0.1"),
            (
                "1.5",
                "evolve.tolerance is a share: it cannot exceed 1, not 1.5",
            ),
        ] {
            assert_eq!(
                s.set_setting("human", "evolve.tolerance", input)
                    .unwrap_err()
                    .to_string(),
                want
            );
        }
        assert_eq!(
            s.set_setting("human", "amend_majority", "0.05")
                .unwrap_err()
                .to_string(),
            "A majority must be a fraction such as 2/3, not '0.05'"
        );
    }

    #[test]
    fn heldout_mode_setting() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-09-08T10:00:00Z");
        assert_eq!(s.setting("evolve.heldout", ""), "auto");
        for (input, want) in [("owner", "owner"), (" Owner ", "owner"), ("auto", "auto")] {
            s.set_setting("human", "evolve.heldout", input).unwrap();
            assert_eq!(s.setting("evolve.heldout", ""), want);
        }
        for input in ["off", "1", ""] {
            assert!(
                s.set_setting("human", "evolve.heldout", input)
                    .unwrap_err()
                    .to_string()
                    .contains("evolve.heldout takes one of auto, owner")
            );
        }
        assert!(
            EVOLVE_RULES["evolve.heldout"]
                .description
                .contains("ADR-033")
        );
    }

    #[test]
    fn flat_cycles_setting() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-09-08T10:00:00Z");
        assert_eq!(s.setting("evolve.flat_cycles", ""), "6");
        assert_eq!(s.setting_int("evolve.flat_cycles"), 6);
        for (input, want) in [("0", "0"), ("2", "2"), (" 12 ", "12")] {
            s.set_setting("human", "evolve.flat_cycles", input).unwrap();
            assert_eq!(s.setting("evolve.flat_cycles", ""), want);
        }
        for (input, message) in [
            (
                "1",
                "at least 2 tied cycles, or 0 to turn the gate off; not 1",
            ),
            ("-1", "evolve.flat_cycles must be at least 0, not -1"),
            ("six", "evolve.flat_cycles takes a whole number"),
            ("", "evolve.flat_cycles takes a whole number"),
        ] {
            let err = s
                .set_setting("human", "evolve.flat_cycles", input)
                .unwrap_err()
                .to_string();
            assert!(err.contains(message), "{input:?}: {err}");
        }
        assert_eq!(s.setting("evolve.flat_cycles", ""), "12");
        let rule = &EVOLVE_RULES["evolve.flat_cycles"];
        assert_eq!(rule.kind, "count");
        assert!(
            rule.description.contains("0 turns the gate off")
                && rule.description.contains("evolution/loop-gate"),
            "{}",
            rule.description
        );
    }

    #[test]
    fn heal_settings() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-09-08T10:00:00Z");
        assert_eq!(s.setting("evolve.heal", ""), "on");
        assert_eq!(s.setting_int("evolve.flat_wait_hours"), 24);
        s.set_setting("human", "evolve.heal", "off").unwrap();
        assert_eq!(s.setting("evolve.heal", ""), "off");
        let err = s
            .set_setting("human", "evolve.heal", "maybe")
            .unwrap_err()
            .to_string();
        assert!(err.contains("evolve.heal"), "{err}");
        s.set_setting("human", "evolve.flat_wait_hours", "0")
            .unwrap();
        assert_eq!(s.setting_int("evolve.flat_wait_hours"), 0);
        let err = s
            .set_setting("human", "evolve.flat_wait_hours", "-2")
            .unwrap_err()
            .to_string();
        assert!(err.contains("at least 0"), "{err}");
        assert!(
            EVOLVE_RULES["evolve.heal"]
                .description
                .contains("derives metric fixtures")
                && EVOLVE_RULES["evolve.flat_wait_hours"]
                    .description
                    .contains("0 waits for the owner")
        );
    }

    #[test]
    fn daemon_self_update_settings() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-09-08T10:00:00Z");
        assert_eq!(s.setting("daemon.self_update", ""), "on");
        assert_eq!(s.setting("daemon.self_update_retry_minutes", ""), "60");
        s.set_setting("daemon", "daemon.self_update", " OFF ")
            .unwrap();
        assert_eq!(s.setting("daemon.self_update", ""), "off");
        assert!(
            s.set_setting("human", "daemon.self_update", "1")
                .unwrap_err()
                .to_string()
                .contains("daemon.self_update takes one of on, off, release")
        );
        // ADR-070 step 6: a hosted brain installs releases, never builds.
        s.set_setting("human", "daemon.self_update", "Release")
            .unwrap();
        assert_eq!(s.setting("daemon.self_update", ""), "release");
        s.set_setting("human", "daemon.self_update", "off").unwrap();
        // brain.code: clone by default; none keeps project code off the brain.
        assert_eq!(s.setting("brain.code", ""), "clone");
        s.set_setting("human", "brain.code", "none").unwrap();
        assert_eq!(s.setting("brain.code", ""), "none");
        assert!(
            s.set_setting("human", "brain.code", "shallow")
                .unwrap_err()
                .to_string()
                .contains("brain.code takes one of clone, none")
        );
        assert!(DAEMON_RULES["brain.code"].description.contains("ADR-070"));
        assert!(
            s.set_setting("human", "daemon.self_update_retry_minutes", "0")
                .is_err()
        );
        s.set_setting("human", "daemon.self_update_retry_minutes", "15")
            .unwrap();
        assert_eq!(s.setting("daemon.self_update_retry_minutes", ""), "15");
        let names = extra_setting_names();
        let lead_start = names.len() - LEAD_SETTINGS.len() - OWNER_SETTINGS.len();
        let segment = names[lead_start - DAEMON_SETTINGS.len()..lead_start].join(" ");
        for name in [
            "daemon.self_update",
            "daemon.self_update_retry_minutes",
            "landing.push",
            "push.credential",
        ] {
            assert!(segment.contains(name));
        }
        assert!(
            DAEMON_RULES["daemon.self_update"]
                .description
                .contains("ADR-035")
        );
        assert_eq!(s.setting("landing.push", ""), "off");
        assert_eq!(s.setting("landing.base_red_retry_minutes", ""), "10");
        assert!(
            s.set_setting("human", "landing.base_red_retry_minutes", "0")
                .is_err()
        );
        s.set_setting("human", "landing.base_red_retry_minutes", "3")
            .unwrap();
        assert_eq!(s.setting("landing.base_red_retry_minutes", ""), "3");
        assert!(
            s.set_setting("human", "landing.push", "origin")
                .unwrap_err()
                .to_string()
                .contains("landing.push takes one of off, main, pr")
        );
        s.set_setting("human", "landing.push", " Main ").unwrap();
        assert_eq!(s.setting("landing.push", ""), "main");
        s.set_setting("human", "landing.push", "pr").unwrap();
        assert_eq!(s.setting("landing.push", ""), "pr");
        assert!(DAEMON_RULES["landing.push"].description.contains("ADR-037"));
        assert_eq!(s.setting("push.credential", ""), "");
        assert!(
            DAEMON_RULES["push.credential"]
                .description
                .contains("ADR-041")
        );
        s.set_setting(
            "human",
            "push.credential",
            " op://Hive/hive push key/private key?ssh-format=openssh ",
        )
        .unwrap();
        assert_eq!(
            s.setting("push.credential", ""),
            "op://Hive/hive push key/private key?ssh-format=openssh"
        );
        for reference in [
            "keychain://hive/push-key",
            "file:///Users/me/.hive/secrets/push_key",
        ] {
            s.set_setting("human", "push.credential", reference)
                .unwrap();
            assert_eq!(s.setting("push.credential", ""), reference);
        }
        assert!(
            s.set_setting(
                "human",
                "push.credential",
                "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----"
            )
            .unwrap_err()
            .to_string()
            .contains("never the key itself")
        );
        assert!(
            s.set_setting("human", "push.credential", "ghp_abcdefghijklmnop")
                .unwrap_err()
                .to_string()
                .contains("never the key itself")
        );
        assert!(
            s.set_setting("human", "push.credential", "vault://x/y")
                .unwrap_err()
                .to_string()
                .contains("unknown scheme")
        );
        assert!(
            s.set_setting("human", "push.credential", "file://relative")
                .unwrap_err()
                .to_string()
                .contains("absolute path")
        );
        s.set_setting("human", "push.credential", "").unwrap();
        assert_eq!(s.setting("push.credential", ""), "");
    }

    #[test]
    fn github_app_settings_take_numbers_and_a_key_reference_only() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-11-10T10:00:00Z");
        for name in [
            "git.github_app.app_id",
            "git.github_app.installation_id",
            "git.github_app.private_key_ref",
        ] {
            assert_eq!(s.setting(name, ""), "", "{name} defaults to empty");
            assert_eq!(DAEMON_SETTINGS[name], "", "{name} defaults to empty");
            assert!(extra_setting_names().contains(&name.to_string()));
        }
        s.set_setting("human", "git.github_app.app_id", " 123456 ")
            .unwrap();
        assert_eq!(s.setting("git.github_app.app_id", ""), "123456");
        assert!(
            s.set_setting("human", "git.github_app.installation_id", "my-org")
                .unwrap_err()
                .to_string()
                .contains("takes the number GitHub shows")
        );
        s.set_setting("human", "git.github_app.installation_id", "987")
            .unwrap();
        assert!(
            s.set_setting(
                "human",
                "git.github_app.private_key_ref",
                "-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----"
            )
            .unwrap_err()
            .to_string()
            .contains("never the key itself")
        );
        s.set_setting(
            "human",
            "git.github_app.private_key_ref",
            "file:///etc/hive/app.pem",
        )
        .unwrap();
        assert_eq!(
            s.setting("git.github_app.private_key_ref", ""),
            "file:///etc/hive/app.pem"
        );
        s.set_setting("human", "git.github_app.app_id", "").unwrap();
        assert_eq!(s.setting("git.github_app.app_id", ""), "");
    }

    #[test]
    fn ui_report_is_required_by_default_and_set_per_project() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-11-07T10:00:00Z");
        assert_eq!(s.setting(UI_REPORT, ""), "require");
        assert!(extra_setting_names().contains(&UI_REPORT.to_string()));
        assert_eq!(s.ui_report_mode("web"), "require");
        s.set_setting("human", UI_REPORT, "warn").unwrap();
        assert_eq!(s.ui_report_mode("web"), "warn");
        // One project of its own; the others follow the machine's value.
        s.set_setting("human", &ui_report_setting("web"), "OFF")
            .unwrap();
        assert_eq!(s.ui_report_mode("web"), "off");
        assert_eq!(s.ui_report_mode("api"), "warn");
        assert_eq!(s.ui_report_mode(""), "warn");
        // The family is not in the listed catalogue, and takes only the three values.
        assert!(!extra_setting_names().contains(&ui_report_setting("web")));
        let err = s
            .set_setting("human", &ui_report_setting("web"), "maybe")
            .unwrap_err()
            .to_string();
        assert!(err.contains("takes one of"), "{err}");
        assert!(s.set_setting("human", UI_REPORT, "sometimes").is_err());
    }

    #[test]
    fn daemon_sweep_settings() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-09-08T10:00:00Z");
        for (key, want) in [
            ("daemon.sweep", "on"),
            ("daemon.sweep_grace_minutes", "30"),
            ("daemon.sweep_branch_days", "7"),
            ("daemon.chat_worktree_keep_days", "3"),
            ("daemon.sweep_keep_days", "30"),
            ("daemon.transcript_keep_days", "14"),
            ("daemon.artifact_keep_days", "30"),
            ("daemon.sweep_runners", ""),
            ("daemon.runner_keep_days", "30"),
            ("daemon.log_keep_days", "14"),
            ("daemon.log_max_mb", "32"),
            ("daemon.vacuum_free_mb", "256"),
            ("daemon.archive_days", "90"),
            ("daemon.build_cache", "on"),
            ("daemon.build_cache_keep_days", "7"),
        ] {
            assert_eq!(s.setting(key, ""), want);
            assert!(extra_setting_names().contains(&key.to_string()));
        }
        // The other agent programs: any text names them, the window is a day at least.
        s.set_setting("human", "daemon.sweep_runners", "copilot, claude")
            .unwrap();
        assert_eq!(s.setting("daemon.sweep_runners", ""), "copilot, claude");
        s.set_setting("human", "daemon.sweep_runners", "none")
            .unwrap();
        assert!(
            s.set_setting("human", "daemon.runner_keep_days", "0")
                .is_err()
        );
        s.set_setting("human", "daemon.runner_keep_days", "7")
            .unwrap();
        for key in [
            "daemon.transcript_keep_days",
            "daemon.artifact_keep_days",
            "daemon.log_max_mb",
            "daemon.vacuum_free_mb",
            "daemon.archive_days",
        ] {
            s.set_setting("human", key, "0").unwrap();
            assert!(s.set_setting("human", key, "-1").is_err());
        }
        for key in ["daemon.log_keep_days", "daemon.build_cache_keep_days"] {
            assert!(s.set_setting("human", key, "0").is_err());
            s.set_setting("human", key, "3").unwrap();
        }
        assert!(
            s.set_setting("human", "daemon.build_cache", "maybe")
                .is_err()
        );
        s.set_setting("human", "daemon.build_cache", "off").unwrap();
        assert_eq!(s.setting("daemon.build_cache", ""), "off");
        assert!(
            s.set_setting("human", "daemon.sweep", "maybe")
                .unwrap_err()
                .to_string()
                .contains("daemon.sweep takes one of on, off")
        );
        s.set_setting("human", "daemon.sweep", " OFF ").unwrap();
        assert_eq!(s.setting("daemon.sweep", ""), "off");
        for key in [
            "daemon.sweep_grace_minutes",
            "daemon.sweep_branch_days",
            "daemon.chat_worktree_keep_days",
        ] {
            s.set_setting("human", key, "0").unwrap();
            assert!(s.set_setting("human", key, "-1").is_err());
        }
        assert!(
            s.set_setting("human", "daemon.sweep_keep_days", "0")
                .is_err()
        );
        s.set_setting("human", "daemon.sweep_keep_days", "90")
            .unwrap();
        assert_eq!(s.setting("daemon.sweep_keep_days", ""), "90");
    }

    #[test]
    fn lead_idle_settings() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-09-08T10:00:00Z");
        assert_eq!(s.setting("lead.idle_hours", ""), "6");
        assert_eq!(s.setting("lead.idle_max_jobs", ""), "3");
        assert!(
            s.set_setting("human", "lead.idle_hours", "x")
                .unwrap_err()
                .to_string()
                .contains("takes a whole number")
        );
        assert!(s.set_setting("human", "lead.idle_hours", "-1").is_err());
        s.set_setting("human", "lead.idle_hours", "0").unwrap();
        assert_eq!(s.setting("lead.idle_hours", ""), "0");
        assert!(s.set_setting("human", "lead.idle_max_jobs", "0").is_err());
        s.set_setting("human", "lead.idle_max_jobs", "4").unwrap();
        assert_eq!(s.setting("lead.idle_max_jobs", ""), "4");
        let names = extra_setting_names();
        let lead_start = names.len() - OWNER_SETTINGS.len() - LEAD_SETTINGS.len();
        assert_eq!(
            &names[lead_start..lead_start + 5],
            [
                "goal.review_days",
                "lead.ballot_cap",
                "lead.ballot_hours",
                "lead.idle_factor",
                "lead.idle_hours"
            ]
        );
        assert_eq!(s.setting("baseline.retry_hours", ""), "6");
        assert_eq!(s.setting("lane.retry_minutes", ""), "20");
        assert_eq!(s.setting("lane.short_retry_minutes", ""), "2");
        for key in [
            "baseline.retry_hours",
            "lane.retry_minutes",
            "lane.short_retry_minutes",
        ] {
            assert!(s.set_setting("human", key, "0").is_err());
        }
        s.set_setting("human", "lane.retry_minutes", "7").unwrap();
        s.set_setting("human", "lane.short_retry_minutes", "3")
            .unwrap();
        s.set_setting("human", "baseline.retry_hours", "2").unwrap();
        assert_eq!(s.setting("lane.retry_minutes", ""), "7");
        assert_eq!(s.setting("lane.short_retry_minutes", ""), "3");
        assert_eq!(s.setting("baseline.retry_hours", ""), "2");
        assert_eq!(s.setting("lead.research_minutes", ""), "60");
        assert!(
            s.set_setting("human", "lead.research_per_day", "0")
                .is_err()
        );
        assert!(s.set_setting("human", "lead.ballot_hours", "0").is_err());
        s.set_setting("human", "lead.research_minutes", "0")
            .unwrap();
    }

    #[test]
    fn light_model_setting() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-09-08T10:00:00Z");
        assert_eq!(s.setting("light_model", ""), "");
        let n: i64 = s
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE name = 'light_model'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
        for kind in [
            "task",
            "improve",
            "experiment",
            "lead",
            "answer",
            "ballot",
            "reflect",
            "onboard",
            "clean",
            "baseline",
            "review",
        ] {
            assert_eq!(s.run_model(kind), "");
        }
        s.set_setting("human", "worker_model", " claude-fable-5.1 ")
            .unwrap();
        assert_eq!(s.run_model("lead"), "claude-fable-5.1");
        s.set_setting("human", "light_model", " claude-sonnet-5 ")
            .unwrap();
        for kind in IMPLEMENTATION_JOB_KINDS {
            assert_eq!(s.run_model(kind), "claude-fable-5.1");
        }
        for kind in [
            "lead", "answer", "ballot", "reflect", "onboard", "clean", "baseline", "review", "",
        ] {
            assert_eq!(s.run_model(kind), "claude-sonnet-5");
        }
        s.set_setting("human", "light_model", "").unwrap();
        assert_eq!(s.run_model("lead"), "claude-fable-5.1");
        assert_eq!(
            &extra_setting_names()[..9],
            [
                "ask.model",
                "decide.mode",
                "decide.timeout_seconds",
                "decide_model",
                "decide_provider",
                "light_model",
                "light_reasoning",
                "measure_model",
                "measure_reasoning"
            ]
        );
        assert!(
            MODEL_RULES["light_model"]
                .description
                .contains("worker_model")
        );
        assert!(MODEL_RULES["ask.model"].description.contains("light_model"));
        s.set_setting("human", "ask.model", " gpt-5-mini ").unwrap();
        assert_eq!(s.setting("ask.model", ""), "gpt-5-mini");
        assert_eq!(
            s.run_model("lead"),
            "claude-fable-5.1",
            "ask.model touches no run kind"
        );
        assert!(
            SETTING_RULES["worker_model"]
                .description
                .contains("light_model")
        );
    }

    #[test]
    fn measure_model_setting() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-10-26T10:00:00Z");
        assert_eq!(s.measure_model(), "");
        s.set_setting("human", "worker_model", " claude-fable-5.1 ")
            .unwrap();
        assert_eq!(s.measure_model(), "claude-fable-5.1");
        s.set_setting("human", "light_model", "claude-sonnet-5.5")
            .unwrap();
        assert_eq!(s.measure_model(), "claude-fable-5.1");
        s.set_setting("human", "measure_model", " claude-haiku-5.5 ")
            .unwrap();
        assert_eq!(s.measure_model(), "claude-haiku-5.5");
        for kind in ["task", "improve", "experiment"] {
            assert_eq!(s.run_model(kind), "claude-fable-5.1");
        }
        assert_eq!(s.run_model("lead"), "claude-sonnet-5.5");
        s.set_setting("human", "measure_model", "").unwrap();
        assert_eq!(s.measure_model(), "claude-fable-5.1");
        assert!(
            MODEL_RULES["measure_model"]
                .description
                .contains("worker_model")
        );
        assert!(
            SETTING_RULES["worker_model"]
                .description
                .contains("measure_model")
        );
    }

    #[test]
    fn decide_settings_fall_back_to_measure_light_and_the_hub_provider() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-10-26T10:00:00Z");
        assert_eq!(s.decide_mode(), "on", "on by default (Amendment 1)");
        assert_eq!(s.setting("decide.timeout_seconds", ""), "30");
        assert_eq!(s.decide_model(), "");
        s.set_setting("human", "worker_model", "big").unwrap();
        assert_eq!(
            s.decide_model(),
            "big",
            "light_model falls back to worker_model"
        );
        s.set_setting("human", "light_model", "light").unwrap();
        assert_eq!(s.decide_model(), "light");
        s.set_setting("human", "measure_model", "measure").unwrap();
        assert_eq!(s.decide_model(), "measure");
        s.set_setting("human", "decide_model", " small ").unwrap();
        assert_eq!(s.decide_model(), "small");
        assert_eq!(
            s.run_model("lead"),
            "light",
            "decide_model touches no run kind"
        );
        assert_eq!(s.decide_provider(), "copilot");
        s.set_setting("human", "provider", "openai").unwrap();
        assert_eq!(s.decide_provider(), "openai");
        s.set_setting("human", "decide_provider", "anthropic")
            .unwrap();
        assert_eq!(s.decide_provider(), "anthropic");
        assert_eq!(s.setting("provider", ""), "openai");
        assert!(
            s.set_setting("human", "decide_provider", "nonesuch")
                .is_err()
        );
        let provider_choices = &HARNESS_RULES["provider"].choices;
        for c in provider_choices {
            assert!(
                MODEL_RULES["decide_provider"].choices.contains(c),
                "decide_provider lacks {c}"
            );
        }
        for mode in ["off", "shadow", "on"] {
            s.set_setting("human", "decide.mode", mode).unwrap();
            assert_eq!(s.decide_mode(), mode);
        }
        assert!(s.set_setting("human", "decide.mode", "maybe").is_err());
        assert!(
            s.set_setting("human", "decide.timeout_seconds", "0")
                .is_err()
        );
        s.set_setting("human", "decide.timeout_seconds", "45")
            .unwrap();
        assert_eq!(s.setting("decide.timeout_seconds", ""), "45");
    }

    #[test]
    fn reasoning_effort_settings_fall_back_by_role() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-10-30T10:00:00Z");
        // Nothing set: every role follows harness.reasoning.
        assert_eq!(s.run_reasoning("task"), "default");
        assert_eq!(s.run_reasoning("lead"), "default");
        assert_eq!(s.measure_reasoning(), "default");
        s.set_setting("human", "harness.reasoning", "medium")
            .unwrap();
        assert_eq!(s.run_reasoning("task"), "medium");
        assert_eq!(s.run_reasoning_class("answer", "worker"), "medium");
        assert_eq!(s.measure_reasoning(), "medium");
        s.set_setting("human", "worker_reasoning", " High ")
            .unwrap();
        for kind in ["task", "improve", "experiment"] {
            assert_eq!(s.run_reasoning(kind), "high");
        }
        assert_eq!(s.run_reasoning("lead"), "medium");
        assert_eq!(s.run_reasoning_class("lead", "worker"), "high");
        assert_eq!(s.run_reasoning_class("task", "light"), "medium");
        assert_eq!(
            s.measure_reasoning(),
            "high",
            "measuring follows the worker"
        );
        s.set_setting("human", "light_reasoning", "low").unwrap();
        assert_eq!(s.run_reasoning("lead"), "low");
        s.set_setting("human", "measure_reasoning", "xhigh")
            .unwrap();
        assert_eq!(s.measure_reasoning(), "xhigh");
        s.set_setting("human", "worker_reasoning", "").unwrap();
        assert_eq!(s.run_reasoning("task"), "medium");
        let err = s
            .set_setting("human", "worker_reasoning", "extreme")
            .unwrap_err()
            .to_string();
        assert!(err.contains("empty, default, minimal"), "{err}");
    }

    /// Claude models offer `max` above `xhigh`; every effort setting takes
    /// it, so an owner can choose it (a model without it gets the nearest
    /// effort it offers, see `hive_provider::nearest_reasoning`).
    #[test]
    fn reasoning_effort_settings_accept_max() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-10-31T06:00:00Z");
        for name in [
            "harness.reasoning",
            "worker_reasoning",
            "light_reasoning",
            "measure_reasoning",
        ] {
            s.set_setting("human", name, " Max ")
                .unwrap_or_else(|e| panic!("{name} refused max: {e}"));
            assert_eq!(s.setting(name, ""), "max", "{name}");
        }
        for kind in ["task", "improve", "experiment"] {
            assert_eq!(s.run_reasoning(kind), "max");
        }
        assert_eq!(s.run_reasoning("lead"), "max");
        assert_eq!(s.measure_reasoning(), "max");
        // The settings list the efforts lowest first, max above xhigh.
        let err = s
            .set_setting("human", "worker_reasoning", "extreme")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("empty, default, minimal, low, medium, high, xhigh, max"),
            "{err}"
        );
    }

    #[test]
    fn owner_presence_setting() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-09-08T10:00:00Z");
        assert_eq!(s.setting("owner.presence", ""), OWNER_PRESENT);
        assert!(!s.owner_away());
        s.set_setting("human", "owner.presence", " Away ").unwrap();
        assert_eq!(s.setting("owner.presence", ""), OWNER_AWAY);
        assert!(s.owner_away());
        assert!(
            s.set_setting("human", "owner.presence", "sometimes")
                .unwrap_err()
                .to_string()
                .contains("owner.presence takes one of present, away")
        );
        for actor in ["proj-lead", "aurora", "daemon"] {
            s.set_setting(actor, "owner.presence", "away").unwrap();
            assert!(!s.owner_away());
        }
        s.set_setting("human:dashboard", "owner.presence", "away")
            .unwrap();
        assert!(s.owner_away());
        for actor in ["human", "human:dashboard", "human:cli"] {
            assert!(human_actor(actor));
        }
        for actor in ["", "humanoid", "proj-lead", "daemon", "hive"] {
            assert!(!human_actor(actor));
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            let sql = format!(
                "SELECT {} FROM (SELECT ? AS actor)",
                human_actor_sql("actor")
            );
            let hit: bool = conn.query_row(&sql, [actor], |r| r.get(0)).unwrap();
            assert!(!hit, "{actor}");
        }
        for actor in ["human", "human:dashboard", "human:cli"] {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            let sql = format!(
                "SELECT {} FROM (SELECT ? AS actor)",
                human_actor_sql("actor")
            );
            let hit: bool = conn.query_row(&sql, [actor], |r| r.get(0)).unwrap();
            assert!(hit, "{actor}");
        }
        assert!(
            validate_setting("owner.mood", "away")
                .unwrap_err()
                .to_string()
                .contains("Owner settings: owner.name, owner.presence")
        );
        let names = extra_setting_names();
        assert_eq!(names[names.len() - 1], "owner.presence");
        assert_eq!(names[names.len() - 2], "owner.name");
    }

    #[test]
    fn owner_name_setting() {
        let _serial = crate::test_helpers::serial();
        let _guard = set_job_empty();
        let s = store("2026-09-08T10:00:00Z");
        assert_eq!(s.setting("owner.name", ""), "");
        assert_eq!(OWNER_RULES["owner.name"].kind, "text");
        assert_eq!(
            validate_setting("owner.name", "  Manish ").unwrap(),
            "Manish"
        );
        assert_eq!(
            validate_setting("owner.name", " Manish \t  Kumar\n").unwrap(),
            "Manish Kumar"
        );
        assert_eq!(validate_setting("owner.name", "   ").unwrap(), "");
        assert_eq!(validate_setting("owner.name", "Zoë").unwrap(), "Zoë");
        let longest = "a".repeat(OWNER_NAME_MAX);
        assert_eq!(validate_setting("owner.name", &longest).unwrap(), longest);
        let err = validate_setting("owner.name", &"a".repeat(OWNER_NAME_MAX + 1))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("owner.name is a display name of at most 64 characters"),
            "{err}"
        );
        s.set_setting("human:dashboard", "owner.name", " Manish ")
            .unwrap();
        assert_eq!(s.setting("owner.name", ""), "Manish");
        s.set_setting("human", "owner.name", "").unwrap();
        assert_eq!(s.setting("owner.name", ""), "");
        assert!(
            validate_setting("owner.mood", "x")
                .unwrap_err()
                .to_string()
                .contains("Owner settings: owner.name, owner.presence")
        );
    }

    #[test]
    fn update_latest_url_is_off_by_default_and_takes_a_web_url() {
        let _serial = crate::test_helpers::serial();
        assert_eq!(DAEMON_SETTINGS["update.latest_url"], "");
        assert!(daemon_setting_names().contains(&"update.latest_url".to_string()));
        assert_eq!(validate_setting("update.latest_url", "  ").unwrap(), "");
        let gh = "https://api.github.com/repos/lamplitlabs/hive/releases/latest";
        assert_eq!(
            validate_setting("update.latest_url", &format!(" {gh} ")).unwrap(),
            gh
        );
        assert_eq!(
            validate_setting("update.latest_url", "http://127.0.0.1:8080/latest.json").unwrap(),
            "http://127.0.0.1:8080/latest.json"
        );
        for bad in [
            "file:///etc/passwd",
            "api.github.com/repos/o/r/releases/latest",
            "https://",
            "https://example.test/a b",
        ] {
            let err = validate_setting("update.latest_url", bad)
                .unwrap_err()
                .to_string();
            assert!(err.contains("takes an http(s) URL"), "{bad}: {err}");
        }
        let err = validate_setting("update.latest_url", "https://user:pass@example.test/x")
            .unwrap_err()
            .to_string();
        assert!(err.contains("credential"), "{err}");
    }

    #[test]
    fn mock_settings_list_every_store_setting() {
        let _serial = crate::test_helpers::serial();
        let Ok(raw) = fs::read_to_string("web/hub/mock/api/settings.json") else {
            return;
        };
        let doc: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
            panic!(
                "{} {} {} {}: {e}",
                "json:\"settings\"", "json:\"name\"", "json:\"kind\"", "json:\"default\""
            )
        });
        let mut mock = BTreeMap::new();
        for row in doc["settings"].as_array().unwrap() {
            let name = row["name"].as_str().unwrap().to_string();
            assert!(
                mock.insert(
                    name,
                    (
                        row["kind"].as_str().unwrap().to_string(),
                        row["default"].as_str().unwrap().to_string()
                    )
                )
                .is_none()
            );
        }
        for (settings, rules) in [
            (&*DAEMON_SETTINGS, &*DAEMON_RULES),
            (&*EVOLVE_SETTINGS, &*EVOLVE_RULES),
            (&*LEAD_SETTINGS, &*LEAD_RULES),
            (&*OWNER_SETTINGS, &*OWNER_RULES),
            (&*HARNESS_SETTINGS, &*HARNESS_RULES),
        ] {
            for name in sorted_keys(settings) {
                let got = mock
                    .get(&name)
                    .unwrap_or_else(|| panic!("mock lacks {name} setting {name}"));
                let want_kind = &rules[&name].kind;
                if !want_kind.is_empty() {
                    assert_eq!(&got.0, want_kind, "{name}");
                }
                assert_eq!(&got.1, &settings[&name], "{name}");
            }
        }
        let mut known = BTreeMap::new();
        for name in DEFAULT_SETTINGS.keys() {
            known.insert(name.clone(), true);
        }
        for name in extra_setting_names() {
            known.insert(name, true);
        }
        for name in mock.keys() {
            assert!(
                known.contains_key(name),
                "mock lists {name} which the store does not know"
            );
        }
    }
}
