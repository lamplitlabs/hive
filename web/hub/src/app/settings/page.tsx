"use client";

import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { BellIcon, ChevronRightIcon, CloudIcon, CpuIcon, GaugeIcon, GitBranchIcon, KeyRoundIcon, PaletteIcon, PlayIcon, PlugIcon, SearchIcon, ServerIcon, Settings2Icon, ShieldAlertIcon, SlidersHorizontalIcon, SparklesIcon, UserRoundIcon } from "lucide-react";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SectionHeading } from "@/components/pulse/item-card";
import { PageHeader } from "@/components/pulse/page-header";
import { EmptyState, ErrorNotice, Notice, SkeletonPage } from "@/components/pulse/panel";
import { AppearanceSection } from "@/components/settings/appearance-section";
import { RunningStrip } from "@/components/settings/running";
import { BackupSetup, CloudSetup } from "@/components/settings/cloud-setup";
import { CredentialRow } from "@/components/settings/credential";
import { GitAppStrip } from "@/components/settings/git-app";
import { HARNESS_SETTING_KEYS, HarnessSettingsCard, isHarnessSetting } from "@/components/settings/harness";
import { NotificationsCard } from "@/components/settings/notifications";
import { PauseCard, SettingRow, setShowKeys, useShowKeys } from "@/components/settings/setting-row";
import { groupIcon, type GroupIcon } from "@/components/settings/settings-state";
import type { Runner, Setting, SettingsPayload } from "@/lib/api";
import { capWarning, runsTodayOf } from "@/lib/cap-warning-state";
import { COMPUTE_PROVIDERS, settingHelp, settingLabel } from "@/lib/setting-labels";
import { useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";

type Icon = React.ComponentType<{ className?: string; strokeWidth?: number; "aria-hidden"?: boolean }>;

// The lucide component behind each heading icon name settings-state picks
// (groupIcon), so the heading of every group and nav item is one lookup.
const ICONS: Record<GroupIcon, Icon> = {
  Settings2: Settings2Icon,
  UserRound: UserRoundIcon,
  Palette: PaletteIcon,
  Cpu: CpuIcon,
  Play: PlayIcon,
  Bell: BellIcon,
  KeyRound: KeyRoundIcon,
  Cloud: CloudIcon,
  ShieldAlert: ShieldAlertIcon,
  Gauge: GaugeIcon,
  GitBranch: GitBranchIcon,
  Server: ServerIcon,
  Sparkles: SparklesIcon,
  Plug: PlugIcon,
  SlidersHorizontal: SlidersHorizontalIcon,
};
const iconOf = (id: string): Icon => ICONS[groupIcon(id)];

interface Cluster {
  title: string;
  keys: string[];
  // A cluster with a provider shows only while compute.provider names it (ADR-042 D6).
  provider?: string;
  // A form drawn above the cluster's rows (the backup target picker).
  form?: (canEdit: boolean) => React.ReactNode;
}

interface Group {
  id: string;
  title: string;
  note: string;
  keys: string[];
  match?: (name: string) => boolean;
  // Clusters inside a section: a heading and the keys under it.
  clusters?: Cluster[];
  // The rows most people need. The rest wait behind "Show N more".
  primary?: string[];
  // A read-only block above the rows (This machine's "which PULSE runs").
  strip?: (canEdit: boolean) => React.ReactNode;
}

// The sections, in the order the owner needs them: the brake, you, your
// notifications, the harness, then the caps (Limits), machines, self-improvement, this machine
// and the advanced provider addresses. Keys the store knows that no section
// claims fall into "Other", so a new cap the runner learns never disappears
// from the owner's view. Anchors (#stop #owner #notify #harness #runner #compute #git
// #evolve #daemon #provider #other) are linked from other pages.
const RUNNER_CLUSTERS: Cluster[] = [
  { title: "How much work runs", keys: ["stop", "capacity", "jobs_per_day", "minutes_per_job", "prune_after_days"] },
  { title: "Merging finished work", keys: ["landing", "landing.push", "push.credential", "landing.ui_report", "landing.auto_stash", "landing.base_red_retry_minutes", "landing.verify_memo", "landing.verify_memo_hours", "verify.lock"] },
  { title: "Dependencies", keys: ["dependencies.security_updates", "dependencies.major_updates", "dependencies.min_release_age_days", "dependencies.alert_check_minutes"] },
  { title: "Checks on GitHub", keys: ["pipeline.watch", "pipeline.wait_hours"] },
  { title: "Reviewing pull requests", keys: ["review.merge", "review.max_files", "review.max_lines"] },
  { title: "Backups", form: (canEdit) => <BackupSetup canEdit={canEdit} />, keys: ["backup.to", "backup.every_hours", "backup.keep", "backup.subscription"] },
  { title: "Votes on suggestions", keys: ["quorum", "expiry_days", "stale_days", "claim_hours", "board.stale_days", "lead.ballot_hours", "lead.ballot_cap"] },
  { title: "Lead agents", keys: ["lead.idle_hours", "lead.idle_max_jobs", "lead.stuck_thread_jobs", "lead.research_minutes", "lead.research_per_day", "lead.idle_factor", "plan.operating_layer_per_day", "plan.operating_layer_fill"] },
  { title: "Rule changes", keys: ["amend_quorum", "amend_majority"] },
];
const COMPUTE_CLUSTERS: Cluster[] = [
  { title: "Limits", keys: ["compute.provider", "compute.max_vms", "compute.max_hours_per_day", "compute.auto_approve", "compute.budget_credits"] },
  { title: "Azure details", provider: "azure", keys: ["compute.azure.auth", "compute.azure.tenant", "compute.azure.client_id", "compute.azure.client_secret", "compute.azure.subscription", "compute.azure.resource_group", "compute.azure.location", "compute.azure.vm_size", "compute.azure.image"] },
  { title: "AWS details", provider: "aws", keys: ["compute.aws.profile", "compute.aws.region", "compute.aws.instance_type", "compute.aws.image", "compute.aws.subnet", "compute.aws.security_group"] },
  { title: "Google Cloud details", provider: "gcp", keys: ["compute.gcp.project", "compute.gcp.zone", "compute.gcp.machine_type", "compute.gcp.image", "compute.gcp.network"] },
  { title: "Multipass details (virtual machines on this computer)", provider: "multipass", keys: ["compute.multipass.size", "compute.multipass.image"] },
  { title: "When a machine refuses to log in", keys: ["compute.self_heal", "compute.self_retire", "compute.retire_grace_hours"] },
];
const DAEMON_CLUSTERS: Cluster[] = [
  { title: "Updates", keys: ["daemon.self_update", "daemon.self_update_retry_minutes", "daemon.self_update_settle_minutes", "daemon.restart_drain_minutes", "update.latest_url"] },
  { title: "Staying awake", keys: ["daemon.keep_awake"] },
  { title: "What this machine offers", keys: ["machine.labels", "machine.reserve"] },
  { title: "Failed work", keys: ["daemon.recover", "daemon.recover_days", "daemon.recover_limit"] },
  { title: "Cleanup and disk space", keys: ["daemon.sweep", "daemon.min_free_mb", "daemon.sweep_grace_minutes", "daemon.sweep_branch_days", "daemon.chat_worktree_keep_days", "daemon.sweep_keep_days", "daemon.log_keep_days", "daemon.log_max_mb", "daemon.transcript_keep_days", "daemon.artifact_keep_days", "daemon.sweep_runners", "daemon.runner_keep_days", "daemon.vacuum_free_mb", "daemon.archive_days", "daemon.build_cache", "daemon.build_cache_keep_days", "hygiene.cargo_target", "hygiene.cargo_target_hours"] },
  { title: "Agent check-ins", keys: ["heartbeat.dead_after"] },
];
const keysOf = (cs: Cluster[]) => cs.flatMap((c) => c.keys);

const GROUPS: Group[] = [
  {
    id: "owner",
    title: "You",
    note: "Tell PULSE whether you're around to answer.",
    keys: ["owner.name", "owner.presence", "records.auto_adopt"],
    match: (n) => n.startsWith("owner.") || n.startsWith("records."),
  },
  {
    id: "runner",
    title: "Limits",
    note: "How much agents may do on this machine, how finished work gets merged, and how suggestions get votes.",
    keys: keysOf(RUNNER_CLUSTERS),
    match: (n) => n.startsWith("lead.") || n.startsWith("landing.") || n.startsWith("plan.") || n.startsWith("board.") || n.startsWith("verify.") || n.startsWith("dependencies."),
    clusters: RUNNER_CLUSTERS,
    primary: ["capacity", "jobs_per_day", "minutes_per_job", "landing", "landing.push", "push.credential", "landing.ui_report", "quorum"],
  },
  {
    id: "compute",
    title: "Machines",
    note: "Limits for the extra machines agents may ask for, and where those machines come from. Cloud sign-ins stay on this machine.",
    keys: keysOf(COMPUTE_CLUSTERS),
    match: (n) => n.startsWith("compute."),
    clusters: COMPUTE_CLUSTERS,
    primary: ["compute.provider", "compute.max_vms", "compute.max_hours_per_day", "compute.auto_approve", "compute.budget_credits"],
    strip: (canEdit) => <CloudSetup canEdit={canEdit} />,
  },
  {
    id: "git",
    title: "Git",
    note: "How jobs push and open pull requests. With a GitHub App, each job gets a one-hour token for its one repository; without one, git and gh use this computer's own logins.",
    keys: ["git.github_app.app_id", "git.github_app.installation_id", "git.github_app.private_key_ref"],
    match: (n) => n.startsWith("git."),
    strip: () => <GitAppStrip />,
  },
  {
    id: "evolve",
    title: "Self-improvement",
    note: "Experiments where agents try changes and keep the ones that score better on tests. Your private tests stay hidden from agents.",
    keys: ["evolve.mode", "evolve.loop", "evolve.heldout", "evolve.tolerance", "evolve.limits", "evolve.heldout_dir", "evolve.draft_hours"],
    match: (n) => n.startsWith("evolve.") || n.startsWith("baseline.") || n.startsWith("lane."),
    primary: ["evolve.mode", "evolve.loop", "evolve.heldout", "evolve.tolerance"],
  },
  {
    id: "daemon",
    title: "This machine",
    note: "Which PULSE runs here, how it updates itself, retries failed work and cleans up after runs.",
    keys: keysOf(DAEMON_CLUSTERS),
    match: (n) => n.startsWith("daemon.") || n.startsWith("heartbeat.") || n.startsWith("hygiene.") || n.startsWith("machine."),
    clusters: DAEMON_CLUSTERS,
    primary: ["daemon.self_update", "daemon.keep_awake", "daemon.recover", "daemon.sweep", "daemon.min_free_mb", "machine.labels", "machine.reserve"],
    strip: () => <RunningStrip />,
  },
  {
    id: "provider",
    title: "Provider addresses",
    note: "Advanced. The address each model provider's calls go to, for a gateway, a proxy or a server of your own. Connecting a provider under Harness sets the one you use; only the chosen provider's address counts.",
    keys: [
      "provider.openai.base_url",
      "provider.anthropic.base_url",
      "provider.gemini.base_url",
      "provider.azure-openai.base_url",
      "provider.azure-openai.tenant",
      "provider.openrouter.base_url",
      "provider.groq.base_url",
      "provider.mistral.base_url",
      "provider.deepseek.base_url",
      "provider.xai.base_url",
      "provider.together.base_url",
      "provider.fireworks.base_url",
      "provider.cerebras.base_url",
      "provider.github-models.base_url",
      "provider.ollama.base_url",
      "provider.lmstudio.base_url",
      "provider.vllm.base_url",
    ],
    match: (n) => n.startsWith("provider."),
  },
];
const OTHER: Group = { id: "other", title: "Other", note: "Settings this page doesn't group yet. Each one shows PULSE's own description.", keys: [] };

// The section picker, in page order. Settings shows one section at a time
// (the URL hash names it, so links like /settings/#harness land there); a
// search shows every section with a match. General holds the brake, You,
// Appearance and Notifications, whose own anchors (#stop #owner
// #appearance #notify) open it. Icons come from groupIcon (settings-state).
const NAV: { id: string; title: string; icon: Icon }[] = [
  { id: "general", title: "General", icon: iconOf("general") },
  { id: "appearance", title: "Appearance", icon: iconOf("appearance") },
  { id: "harness", title: "Harness", icon: iconOf("harness") },
  { id: "runner", title: "Limits", icon: iconOf("runner") },
  { id: "compute", title: "Machines", icon: iconOf("compute") },
  { id: "git", title: "Git", icon: iconOf("git") },
  { id: "evolve", title: "Self-improvement", icon: iconOf("evolve") },
  { id: "daemon", title: "This machine", icon: iconOf("daemon") },
  { id: "provider", title: "Provider addresses", icon: iconOf("provider") },
  { id: "other", title: "Other", icon: iconOf("other") },
];
const PAGE_OF: Record<string, string> = { stop: "general", owner: "general", appearance: "general", notify: "general" };
const pageOf = (id: string) => PAGE_OF[id] ?? id;

// The hash as a tiny store: the section links are plain anchors, so the
// browser's own hashchange drives which section shows (Back works too).
function subscribeHash(cb: () => void) {
  window.addEventListener("hashchange", cb);
  return () => window.removeEventListener("hashchange", cb);
}
const useHash = () => useSyncExternalStore(subscribeHash, () => window.location.hash.slice(1), () => "");

// What the Harness section answers to in the search box besides its own
// settings (matched like every other row): the card's headings and words.
// The same for the Notifications card, which keeps its choices in this
// browser rather than in PULSE's settings.
const NOTIFY_WORDS = "notifications notify notification alerts alert sound chime bell desktop browser mac question waits for you finished done reply";
// Appearance is browser-only (no setting rows), so a search reaches it by its words.
const APPEARANCE_WORDS = "appearance theme dark light system mode density compact comfortable font geist readable atkinson text size larger default look colours colors";

const HARNESS_WORDS = "harness sign in sign-in signin sign out login log in github microsoft entra tenant device code copilot token api key openai anthropic claude gemini google azure openrouter groq mistral deepseek xai grok together fireworks cerebras ollama lmstudio vllm model models model provider limits per run run mode and transcript size built-in native copilot cli credits cost system prompt preview context window 1m million tokens reasoning effort thinking";

// Mirrors ModelSettings, ComputeSettings, EvolveSettings and DaemonSettings in the store (crates/pulse-store).
// These rows are not inserted at Create, so when the hub omits one the
// screen still lists it at its effective (default) value and the owner can
// set it from here.
const EXTRA_DEFAULTS: Omit<Setting, "updated_at" | "updated_by">[] = [
  { name: "owner.presence", value: "present", default: "present", kind: "choice", min: null, choices: ["present", "away"], description: "present: proposals, compute requests and owner questions wait for you; away: each project's lead decides the board's proposals on your behalf with its best judgement, recorded as the lead's decision and never as yours - hard stops, money (compute), Tier 2 decisions (ADRs), the constitution's protected sections and the kill switch still wait for you" },
  { name: "light_model", value: "", default: "", kind: "text", min: null, choices: null, description: "the model of the light runs - lead, answer, ballot, reflect, onboard and clean jobs, the sealed held-out call and the login check; empty means worker_model" },
  { name: "measure_model", value: "", default: "", kind: "text", min: null, choices: null, description: "the model the self-improvement loop measures fixtures with - every baseline, experiment and landing re-measure; empty means worker_model. It is part of the evaluator: a change re-measures every baseline, and a pinned value keeps the baselines when worker_model changes" },
  { name: "ask.model", value: "", default: "", kind: "text", min: null, choices: null, description: "the model Ask answers the owner's questions with (console Ask, pulse ask); empty means light_model, then worker_model, then the provider's default chat model" },
  { name: "worker_reasoning", value: "", default: "", kind: "choice", min: null, choices: ["", "default", "minimal", "low", "medium", "high", "xhigh"], description: "the reasoning effort of the runs that change code - task, improve and experiment jobs - and of a worker-class agent; empty means harness.reasoning; default sends nothing and leaves the model's own setting" },
  { name: "light_reasoning", value: "", default: "", kind: "choice", min: null, choices: ["", "default", "minimal", "low", "medium", "high", "xhigh"], description: "the reasoning effort of the light runs - lead, answer, ballot, reflect, onboard and clean jobs - and of a light-class agent; empty means harness.reasoning; default sends nothing" },
  { name: "measure_reasoning", value: "", default: "", kind: "choice", min: null, choices: ["", "default", "minimal", "low", "medium", "high", "xhigh"], description: "the reasoning effort the self-improvement loop measures fixtures with; empty means worker_reasoning, then harness.reasoning; default sends nothing. Like measure_model it is part of the evaluator: a change re-measures every baseline" },
  { name: "compute.provider", value: "azure", default: "azure", kind: "choice", min: null, choices: ["azure", "aws", "gcp", "multipass"], description: "where agents get more machines from: azure (az login), aws (aws configure / aws sso login), gcp (gcloud auth login) or multipass (local VMs on the hub machine); each provider reads its own compute.<provider>.* settings" },
  { name: "compute.max_vms", value: "0", default: "0", kind: "count", min: 0, choices: null, description: "VMs that may exist at once across all requests; 0 means compute is off" },
  { name: "compute.max_hours_per_day", value: "0", default: "0", kind: "count", min: 0, choices: null, description: "VM-hours the runner may approve per UTC day; 0 means none" },
  { name: "compute.auto_approve", value: "0", default: "0", kind: "switch", min: null, choices: null, description: "1 lets within-cap requests start without the owner; 0 holds every request for a decision" },
  { name: "compute.budget_credits", value: "0", default: "0", kind: "count", min: 0, choices: null, description: "credits the compute requests may spend in total; 0 means no budget" },
  { name: "compute.self_heal", value: "1", default: "1", kind: "switch", min: null, choices: null, description: "1: a refused login is replaced from the executor token on file (pulse executor token); 0: never" },
  { name: "compute.self_retire", value: "1", default: "1", kind: "switch", min: null, choices: null, description: "1: a machine PULSE created whose login stays refused is stopped and retired, then deleted after the grace hours; 0: it is left as it is" },
  { name: "compute.retire_grace_hours", value: "24", default: "24", kind: "count", min: 1, choices: null, description: "hours a retired machine stays stopped before it is deleted; starting it again at the provider (the finding names the command) and pulse executor activate within them keep it" },
  { name: "compute.azure.subscription", value: "", default: "", kind: "identifier", min: null, choices: null, description: "Azure subscription id the VMs are created in" },
  { name: "compute.azure.resource_group", value: "", default: "", kind: "identifier", min: null, choices: null, description: "resource group that holds the executor VMs" },
  { name: "compute.azure.location", value: "", default: "", kind: "identifier", min: null, choices: null, description: "Azure region, e.g. westeurope" },
  { name: "compute.azure.vm_size", value: "Standard_B2ms", default: "Standard_B2ms", kind: "identifier", min: null, choices: null, description: "VM size used when a request does not name one" },
  { name: "compute.azure.image", value: "Ubuntu2404", default: "Ubuntu2404", kind: "identifier", min: null, choices: null, description: "image alias the VMs boot from" },
  { name: "compute.aws.profile", value: "", default: "", kind: "identifier", min: null, choices: null, description: "AWS CLI profile the runner provisions with; empty means the CLI's default (auth is aws configure or aws sso login on this machine)" },
  { name: "compute.aws.region", value: "", default: "", kind: "identifier", min: null, choices: null, description: "AWS region the instances are created in, e.g. eu-west-1" },
  { name: "compute.aws.instance_type", value: "t3.medium", default: "t3.medium", kind: "identifier", min: null, choices: null, description: "EC2 instance type for a provisioned hand, e.g. t3.medium" },
  { name: "compute.aws.image", value: "/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id", default: "/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id", kind: "identifier", min: null, choices: null, description: "AMI id (ami-...) or an SSM public parameter path resolved at launch; the default is Canonical's current Ubuntu 24.04 amd64 image" },
  { name: "compute.aws.subnet", value: "", default: "", kind: "identifier", min: null, choices: null, description: "subnet id the instances join; empty means the default VPC's default subnet in the region" },
  { name: "compute.aws.security_group", value: "", default: "", kind: "identifier", min: null, choices: null, description: "security group id the instances get; empty means a group named pulse-hands that allows ssh in, created when missing" },
  { name: "compute.gcp.project", value: "", default: "", kind: "identifier", min: null, choices: null, description: "Google Cloud project the instances are created in; empty means gcloud's current project (auth is gcloud auth login on this machine)" },
  { name: "compute.gcp.zone", value: "", default: "", kind: "identifier", min: null, choices: null, description: "Compute Engine zone, e.g. europe-west4-a" },
  { name: "compute.gcp.machine_type", value: "e2-medium", default: "e2-medium", kind: "identifier", min: null, choices: null, description: "machine type for a provisioned hand, e.g. e2-medium" },
  { name: "compute.gcp.image", value: "ubuntu-2404-lts-amd64", default: "ubuntu-2404-lts-amd64", kind: "identifier", min: null, choices: null, description: "Ubuntu image family in ubuntu-os-cloud, e.g. ubuntu-2404-lts-amd64, or a full image path projects/P/global/images/I" },
  { name: "compute.gcp.network", value: "", default: "", kind: "identifier", min: null, choices: null, description: "VPC network the instances join; empty means default. A firewall rule pulse-hands-ssh (tcp 22 to the tag pulse-hands) is created on it when missing" },
  { name: "compute.multipass.size", value: "cpus=2,mem=4G,disk=32G", default: "cpus=2,mem=4G,disk=32G", kind: "identifier", min: null, choices: null, description: "size of a local multipass hand as cpus=N,mem=SIZE,disk=SIZE, e.g. cpus=2,mem=4G,disk=32G" },
  { name: "compute.multipass.image", value: "24.04", default: "24.04", kind: "identifier", min: null, choices: null, description: "multipass image a local hand launches from, e.g. 24.04" },
  { name: "evolve.loop", value: "0", default: "0", kind: "switch", min: null, choices: null, description: "1 lets pulse-evolve run cycles back to back; 0 keeps the loop off" },
  { name: "evolve.limits", value: "none", default: "none", kind: "text", min: null, choices: null, description: "caps the constitution writes for the loop, or none" },
  { name: "evolve.tolerance", value: "0.02", default: "0.02", kind: "text", min: null, choices: null, description: "fraction a measured check may regress before a cycle is discarded" },
  { name: "evolve.heldout", value: "auto", default: "auto", kind: "choice", min: null, choices: ["auto", "owner"], description: "auto: the daemon writes the private held-out set itself, sealed and labelled generated; owner: you write it from the console (ADR-033)" },
  // Owner-private: the hub answers "[owner-private]" instead of the real path once set.
  { name: "evolve.heldout_dir", value: "", default: "", kind: "identifier", min: null, choices: null, description: "path to the owner's held-out evaluators; never shown to agents", secret: true },
  { name: "evolve.draft_hours", value: "24", default: "24", kind: "count", min: 0, choices: null, description: "hours between the drafting jobs the planner queues for a project whose constitution or evaluators is still the seeded template text (ADR-046); 0 leaves drafting to the console's Draft it for me button" },
  { name: "daemon.self_update", value: "on", default: "on", kind: "choice", min: null, choices: ["on", "off"], description: "on: after a landing in the brain the daemon rebuilds pulse from the clean checkout, smoke-tests it, installs it and restarts after its runs in flight; off: builds are yours (ADR-035; the launcher turns it off after a crash-loop)" },
  { name: "daemon.self_update_retry_minutes", value: "60", default: "60", kind: "count", min: 1, choices: null, description: "minutes the daemon waits before building again after a failed build or smoke test" },
  { name: "daemon.keep_awake", value: "off", default: "off", kind: "choice", min: null, choices: ["off", "on", "always"], description: "on: while the daemon runs and the machine is plugged in it holds this machine awake the way Copilot CLI's keep-alive does (macOS: caffeinate -d -i -s, so the display stays on and the screen saver stays off too; Linux: systemd-inhibit), so overnight runs are not lost to sleep, and the console shifts to its awake theme; on battery a Mac holds nothing until the charger is back. always: holds on battery too, display included. off: the machine sleeps as its power settings say" },
  { name: "machine.labels", value: "", default: "", kind: "text", min: null, choices: null, description: "what this machine offers beyond its OS and architecture, as words a project or job can need (pulse.yaml needs:, pulse job needs): xcode and docker are found on their own; add the rest here, comma-separated, such as gpu, android or a tool's name" },
  { name: "machine.reserve", value: "off", default: "off", kind: "choice", min: null, choices: ["off", "on"], description: "on: this machine takes only the jobs that need something it offers (a project with needs: [macos] on the only Mac), so the rest of the queue goes to the other machines; off: it takes any job it can run" },
  { name: "heartbeat.dead_after", value: "300", default: "300", kind: "count", min: 30, choices: null, description: "seconds an agent may go without a heartbeat before it counts as dead: every pulse command run as an agent beats, so do its hooks, its job lease and the runner supervising its run; a dead worker's run is closed as lost and its job queued again, a dead session simply shows as dead until it beats again; never under 30" },
  { name: "landing.push", value: "off", default: "off", kind: "choice", min: null, choices: ["off", "main", "pr"], description: "main: after a landing the daemon pushes the project's default branch to its upstream, fast-forward only, with the git credentials this machine already has - or with push.credential when it is set - and pushes what landed earlier and was not pushed yet; a failed push shows in the landing line of pulse status and the hub. pr: the daemon pushes the default branch to the pulse/landed branch on the upstream instead and keeps one pull request open from it, so every landing waits for your merge on the forge. off: you push - or ask for a pull request from a task's sheet (ADR-037)" },
  { name: "push.credential", value: "", default: "", kind: "reference", min: null, choices: null, description: "where the daemon's own SSH push key lives, as a reference and never the key itself: op://VAULT/ITEM/FIELD?ssh-format=openssh (1Password, read with a service-account token kept by `pulse cred token`), keychain://SERVICE/ACCOUNT (a macOS Keychain item) or file:///ABSOLUTE/PATH (a 0600 file). The key exists on disk only for the seconds of a push; when it cannot be resolved the landing is held with the reason and the daemon never falls back to your credentials. Empty: push with what git already uses. Set it with `pulse cred set`, which probes the remote first (ADR-041)" },
];

function withExtraDefaults(rows: Setting[]): Setting[] {
  const have = new Set(rows.map((s) => s.name));
  const missing = EXTRA_DEFAULTS.filter((d) => !have.has(d.name)).map((d) => ({ ...d, updated_at: "", updated_by: "default" }));
  return missing.length ? [...rows, ...missing] : rows;
}

const ON_OFF = { on: "On", off: "Off" };
const SETTING_LABELS: Record<string, { on?: string; off?: string; choices?: Record<string, string> }> = {
  stop: { on: "Paused: nothing new starts", off: "Running" },
  "owner.presence": { choices: { present: "I'm here", away: "I'm away: lead agents decide for me" } },
  landing: { choices: { agents: "Agents merge it themselves", human: "I merge it myself" } },
  "landing.push": { choices: { off: "Don't push, I push myself", main: "Push the main branch", pr: "Open a pull request" } },
  "landing.ui_report": { choices: { require: "Wait for the report", warn: "Merge, and say it has none", off: "Don't ask for one" } },
  "landing.auto_stash": { choices: ON_OFF },
  "dependencies.security_updates": { on: "On: agents fix the alerts, one small update at a time", off: "Off: the alerts wait for me" },
  "dependencies.major_updates": { choices: { ask: "Ask me: a major upgrade waits in For you with Yes / No", auto: "Automatic: agents do major upgrades like any other fix" } },
  "pipeline.watch": { choices: { off: "Off: I watch the checks myself", watch: "Tell me when a check fails", fix: "Tell me and have an agent fix it" } },
  "review.merge": { choices: { auto: "Agents merge small, safe pull requests themselves", ask: "Ask me: every merge waits for me" } },
  "compute.provider": { choices: { azure: "Azure", aws: "AWS", gcp: "Google Cloud", multipass: "Multipass (on this computer)" } },
  "compute.auto_approve": { on: "On: requests within the limits start by themselves", off: "Off: every request waits for you" },
  "compute.self_heal": { on: "On: PULSE gives it a fresh login", off: "Off: a refused login stays refused" },
  "compute.self_retire": { on: "On: PULSE stops it, then deletes it later", off: "Off: the machine is left running" },
  "evolve.loop": { on: "On: experiments run back to back", off: "Off: no experiments run" },
  "evolve.heldout": { choices: { auto: "PULSE writes them", owner: "I write them myself" } },
  "daemon.self_update": { choices: ON_OFF },
  "daemon.keep_awake": { choices: { off: "Off: the machine sleeps as usual", on: "On while plugged in", always: "On, even on battery" } },
  "machine.reserve": { choices: { off: "Off: takes any job it can run", on: "On: only the jobs that need what it offers" } },
  "daemon.recover": { choices: ON_OFF },
  "daemon.sweep": { choices: ON_OFF },
};

// Anchor margins: on phones the shell header and the section picker stick
// to the top, so a jump lands below both.
const ANCHOR = "scroll-mt-16 lg:scroll-mt-4";

// The rows of one section. The push key has its own row (a picker, not a
// text field). Only the chosen compute provider's details show - unless the
// owner is searching, when every match counts. Big sections show their
// common rows first and fold the rest behind "Show N more". `warnings`
// names the rows whose value can stop work today (jobs_per_day) and what
// to say about a value.
type Warnings = Record<string, (value: string) => string>;

function GroupRows({ g, rows, canEdit, provider, searching, expanded, onToggle, warnings }: { g: Group; rows: Setting[]; canEdit: boolean; provider: string; searching: boolean; expanded: boolean; onToggle: () => void; warnings?: Warnings }) {
  const row = (s: Setting) => (s.name === "push.credential" ? <CredentialRow key={s.name} s={s} canEdit={canEdit} /> : <SettingRow key={s.name} s={s} canEdit={canEdit} labels={SETTING_LABELS[s.name]} warn={warnings?.[s.name]} />);
  const primary = g.primary;
  const folded = rows.filter((s) => primary && !primary.includes(s.name)).length;
  // Folding one row away saves nothing; show it.
  const open = searching || expanded || !primary || folded < 2;
  const visible = (s: Setting) => open || !!primary?.includes(s.name);
  const strip = g.strip && !searching ? <div className="border-b">{g.strip(canEdit)}</div> : null;
  const toggle =
    !searching && primary && folded >= 2 ? (
      <button type="button" onClick={onToggle} aria-expanded={expanded} className="flex w-full items-center gap-2 border-t px-4 py-3 text-left text-sm font-medium text-primary hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring sm:px-5">
        <ChevronRightIcon className={cn("size-4 transition-transform duration-150 motion-reduce:transition-none", expanded && "rotate-90")} aria-hidden />
        {expanded ? "Show fewer settings" : "Show " + folded + " more " + (folded === 1 ? "setting" : "settings")}
      </button>
    ) : null;
  const clusters = g.clusters;
  if (!clusters)
    return (
      <>
        {strip}
        <div className="divide-y">{rows.filter(visible).map(row)}</div>
        {toggle}
      </>
    );
  const loose = rows.filter((s) => !clusters.some((c) => c.keys.includes(s.name))).filter(visible);
  const others = clusters.filter((c) => c.provider && c.provider !== provider).map((c) => COMPUTE_PROVIDERS[c.provider!] ?? c.provider!);
  const chosenName = COMPUTE_PROVIDERS[provider] ?? provider;
  const blocks: { title: string; note?: string; warn?: boolean; rows: Setting[]; form?: React.ReactNode }[] = [];
  for (const c of clusters) {
    if (c.provider && c.provider !== provider && !searching) continue;
    const inCluster = rows.filter((s) => c.keys.includes(s.name)).filter(visible);
    const form = c.form && !searching ? c.form(canEdit) : undefined;
    if (!inCluster.length && !form) continue;
    const chosen = c.provider && c.provider === provider;
    const note = chosen && others.length && !searching ? "Only " + chosenName + "'s details are shown. Pick another option under Where extra machines come from to set that one up." : c.provider && !chosen ? "Not in use: these apply only when " + (COMPUTE_PROVIDERS[c.provider] ?? c.provider) + " is chosen." : undefined;
    blocks.push({ title: c.title, note, warn: !!c.provider && !chosen, rows: inCluster, form });
  }
  if (loose.length) blocks.push({ title: "More " + g.title.toLowerCase(), rows: loose });
  return (
    <>
      {strip}
      {blocks.map((b, i) => (
        <div key={b.title}>
          <div className={cn("grid gap-0.5 px-4 pb-1 sm:px-5", i === 0 ? "pt-4" : "border-t pt-5")}>
            <h3 className="text-sm font-semibold text-foreground">{b.title}</h3>
            {b.note ? <p className={cn("text-[13px] leading-relaxed anywhere", b.warn ? "text-warn" : "text-muted-foreground")}>{b.note}</p> : null}
          </div>
          {b.form ? <div>{b.form}</div> : null}
          <div className="divide-y">{b.rows.map(row)}</div>
        </div>
      ))}
      {toggle}
    </>
  );
}

function Section({ id, title, note, count, children }: { id: string; title: string; note?: React.ReactNode; count?: string; children: React.ReactNode }) {
  return (
    <section id={id} aria-label={title} className={ANCHOR}>
      <SectionHeading icon={iconOf(id)} note={note} count={count}>
        {title}
      </SectionHeading>
      {children}
    </section>
  );
}

// The section picker: a sticky list on the left from lg up, a row of chips
// that sticks under the phone header below that. Each item is a plain
// anchor to its section's hash; the section in view (or, while searching,
// the one scrolled to) is marked aria-current in both.
function SectionNav({ items, active, stopped }: { items: { id: string; title: string; icon: Icon; hit: boolean; count?: number }[]; active: string; stopped: boolean }) {
  const chips = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = chips.current?.querySelector<HTMLElement>('[aria-current="true"]');
    if (!el || !chips.current || chips.current.offsetParent === null) return;
    const box = chips.current;
    const left = el.offsetLeft - box.clientWidth / 2 + el.clientWidth / 2;
    box.scrollTo({ left: Math.max(0, left), behavior: "smooth" });
  }, [active]);
  // The brake's state rides on General: red while all work is paused.
  const dot = (id: string) => (id === "general" && stopped ? <span className="size-2 shrink-0 rounded-full bg-bad" aria-label="All work is paused" role="img" /> : null);
  return (
    <>
      <nav aria-label="Settings sections" className="sticky top-[calc(3.5rem+1px+env(safe-area-inset-top))] z-30 -mx-4 mb-5 border-b bg-background sm:-mx-6 lg:hidden">
        <div ref={chips} className="scroll-strip flex gap-1 px-4 py-2 sm:px-6">
          {items.map((n) => (
            <Button
              key={n.id}
              asChild
              variant="ghost"
              size="sm"
              className={cn("shrink-0 whitespace-nowrap", active === n.id ? "bg-muted font-medium text-link hover:text-link" : "text-muted-foreground", !n.hit && "pointer-events-none opacity-40")}
            >
              <a href={"#" + n.id} aria-current={active === n.id ? "true" : undefined} aria-disabled={!n.hit || undefined}>
                {dot(n.id)}
                {n.title}
                {n.count !== undefined ? <span className="tnum text-xs text-muted-foreground">{n.count}</span> : null}
              </a>
            </Button>
          ))}
        </div>
      </nav>
      <nav aria-label="Settings sections" className="sticky top-6 hidden self-start lg:block">
        <ul className="grid gap-0.5">
          {items.map((n) => {
            const Icon = n.icon;
            const on = active === n.id;
            return (
              <li key={n.id}>
                <a
                  href={"#" + n.id}
                  aria-current={on ? "true" : undefined}
                  aria-disabled={!n.hit || undefined}
                  className={cn(
                    "flex min-w-0 items-center gap-2.5 rounded-md border-l-2 py-2 pl-3 pr-3 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    on ? "border-link bg-muted/60 font-medium text-link" : "border-transparent text-muted-foreground hover:bg-muted/60 hover:text-foreground",
                    !n.hit && "pointer-events-none opacity-40",
                  )}
                >
                  <Icon aria-hidden className={cn("size-4 shrink-0", on ? "text-link" : "text-muted-foreground", n.id === "general" && stopped && "text-bad")} />
                  <span className="min-w-0 flex-1 truncate">{n.title}</span>
                  {n.count !== undefined ? <span className="tnum text-xs text-muted-foreground">{n.count}</span> : dot(n.id)}
                </a>
              </li>
            );
          })}
        </ul>
      </nav>
    </>
  );
}

export default function SettingsPage() {
  const q = useApi<SettingsPayload>("settings", { refresh: 30000 });
  // Today's run count, for the daily-cap row: a jobs_per_day at or under it
  // stops every new run until midnight UTC, and the row must say so rather
  // than let the fleet idle in silence. Same payload the Agents page reads.
  const runner = useApi<Runner>("runner", { refresh: 30000 });
  const runsToday = runsTodayOf(runner.data);
  const warnings = useMemo<Warnings>(() => ({ jobs_per_day: (value) => capWarning(value, runsToday) }), [runsToday]);
  const [find, setFind] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const [spied, setSpied] = useState("");
  const showKeys = useShowKeys();
  const hash = useHash();
  const groups = useMemo(() => {
    const all = q.data ? withExtraDefaults(q.data.settings) : [];
    const used = new Set<string>(all.filter((s) => isHarnessSetting(s.name)).map((s) => s.name));
    const out: { g: Group; rows: Setting[] }[] = [];
    for (const g of GROUPS) {
      const rows: Setting[] = [];
      for (const k of g.keys) {
        const s = all.find((x) => x.name === k);
        if (s && !used.has(s.name)) {
          rows.push(s);
          used.add(s.name);
        }
      }
      if (g.match)
        for (const s of all)
          if (!used.has(s.name) && g.match(s.name)) {
            rows.push(s);
            used.add(s.name);
          }
      if (rows.length) out.push({ g, rows });
    }
    const rest = all.filter((s) => !used.has(s.name)).sort((a, b) => a.name.localeCompare(b.name));
    if (rest.length) out.push({ g: OTHER, rows: rest });
    return out;
  }, [q.data]);

  const d = q.data;
  const stop = d?.settings.find((s) => s.name === "stop");
  const stopped = stop?.value === "1";
  const needle = find.trim().toLowerCase();
  // A key also answers to its words: "max steps" finds harness.max_steps.
  const match = (s: Setting) => !needle || s.name.toLowerCase().includes(needle) || s.name.toLowerCase().replace(/[._]+/g, " ").includes(needle) || settingLabel(s.name).toLowerCase().includes(needle) || settingHelp(s.name).toLowerCase().includes(needle) || s.description.toLowerCase().includes(needle);
  const provider = d?.settings.find((s) => s.name === "compute.provider")?.value || "azure";
  // Sleeping provider rows leave the count too, unless a search wakes them.
  const awake = (s: Setting) => !!needle || !GROUPS.some((g) => g.clusters?.some((c) => c.provider && c.provider !== provider && c.keys.includes(s.name)));
  // The Harness card lists its own settings; they search and count like the rest.
  // minutes_per_job shows there too but is counted once, under Limits.
  const everything = d ? withExtraDefaults(d.settings) : [];
  const harnessRows = HARNESS_SETTING_KEYS.map((k) => everything.find((s) => s.name === k)).filter((s): s is Setting => !!s);
  const harnessHits = harnessRows.filter(match).length;
  const harnessShown = !needle || harnessHits > 0 || HARNESS_WORDS.includes(needle);
  const notifyShown = !needle || NOTIFY_WORDS.includes(needle);
  const appearanceShown = !needle || APPEARANCE_WORDS.includes(needle);
  // The kill switch has its own card at the top, so it leaves the Limits section.
  const killShown = !!stop && (match(stop) || "pause stop kill switch emergency brake resume".includes(needle));
  const base = groups.map(({ g, rows }) => ({ g, rows: rows.filter((s) => (!stop || s.name !== "stop") && awake(s)) }));
  const total = base.reduce((n, x) => n + x.rows.length, 0) + (stop ? 1 : 0) + harnessRows.length;
  const shown = base.map(({ g, rows }) => ({ g, all: rows.length, rows: rows.filter(match) })).filter((x) => x.rows.length);
  const matched = shown.reduce((n, x) => n + x.rows.length, 0) + (killShown ? 1 : 0) + harnessHits;
  const hits: Record<string, number> = { ...Object.fromEntries(shown.map((x) => [x.g.id, x.rows.length])), harness: harnessHits };
  const generalHit = killShown || notifyShown || appearanceShown || !!hits.owner;
  const present = new Set<string>(["general", "appearance", "harness", ...groups.map((x) => x.g.id)]);
  const navItems = NAV.filter((n) => present.has(n.id)).map((n) => {
    const hit = !needle || (n.id === "general" ? generalHit : n.id === "appearance" ? appearanceShown : n.id === "harness" ? harnessShown : !!hits[n.id]);
    const count = needle && hit ? (n.id === "general" ? (killShown ? 1 : 0) + (hits.owner ?? 0) : hits[n.id]) : undefined;
    return { ...n, hit, count: count || undefined };
  });
  // Without a search one section shows: the hash names it, General by default.
  const page = present.has(pageOf(hash)) ? pageOf(hash) : "general";
  const show = (id: string) => !!needle || pageOf(id) === page;
  // While searching every section with a hit shows, in page order.
  const order = needle ? [killShown ? "stop" : "", hits.owner ? "owner" : "", appearanceShown ? "appearance" : "", notifyShown ? "notify" : "", harnessShown ? "harness" : "", ...shown.map((x) => x.g.id).filter((id) => id !== "owner")].filter(Boolean).join(" ") : "";
  // Appearance lives on the General page (#appearance scrolls to its card) but
  // has its own nav item, so the marker names it rather than General.
  const navOf = (id: string) => (id === "appearance" ? id : pageOf(id));
  const current = needle ? navOf(spied || order.split(" ")[0] || "") : present.has(pageOf(hash)) ? navOf(hash) : "general";

  // Opening a section starts at its top; an anchor inside General (#notify) scrolls to itself.
  useEffect(() => {
    if (needle || !q.data) return;
    if (PAGE_OF[hash]) document.getElementById(hash)?.scrollIntoView();
    else window.scrollTo({ top: 0 });
  }, [hash, needle, q.data]);

  // While searching, mark the section in view: the last one whose top has
  // passed the upper part of the screen (or the last at the very bottom).
  useEffect(() => {
    if (!order) return;
    const ids = order.split(" ");
    let frame = 0;
    const measure = () => {
      frame = 0;
      const line = window.innerWidth >= 1024 ? 120 : 150;
      let cur = ids[0]!;
      for (const id of ids) {
        const el = document.getElementById(id);
        if (el && el.getBoundingClientRect().top <= line) cur = id;
      }
      if (window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 4) cur = ids[ids.length - 1]!;
      setSpied(cur);
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(measure);
    };
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll);
    return () => {
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
      if (frame) cancelAnimationFrame(frame);
    };
  }, [order]);

  const title = <PageHeader title="Settings" subtitle="How PULSE runs" />;
  if (q.error && !d)
    return (
      <>
        {title}
        <ErrorNotice error={q.error} onRetry={() => void q.reload()} />
      </>
    );
  if (!d) return <SkeletonPage rows={4} />;

  const toggle = (id: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const groupSection = ({ g, rows, all }: { g: Group; rows: Setting[]; all: number }) => (
    <Section key={g.id} id={g.id} title={g.title} note={g.note} count={needle ? rows.length + " of " + all : undefined}>
      <div className="min-w-0 overflow-hidden rounded-lg border bg-card">
        <GroupRows g={g} rows={rows} canEdit={d.can_edit} provider={provider} searching={!!needle} expanded={expanded.has(g.id)} onToggle={() => toggle(g.id)} warnings={warnings} />
      </div>
    </Section>
  );

  return (
    <>
      {title}
      <div className="mb-6 flex min-w-0 flex-wrap items-center gap-x-4 gap-y-2">
        <div className="relative w-full max-w-xl">
          <SearchIcon className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden />
          <Input
            type="search"
            value={find}
            onChange={(e) => {
              setFind(e.target.value);
              setSpied("");
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape") setFind("");
            }}
            placeholder="Search all settings"
            aria-label="Search all settings"
            aria-describedby="settings-search-hint"
            className="h-9 rounded-md bg-card pl-9 text-sm"
          />
        </div>
        <p id="settings-search-hint" className="min-w-0 flex-1 text-[13px] text-muted-foreground" aria-live="polite">
          {needle ? (
            matched || (!harnessShown && !notifyShown && !appearanceShown) ? (
              <span className="tnum">
                {matched} of {total} settings match
              </span>
            ) : (
              ((names) => "No single setting matches, but the " + names.join(" and ") + (names.length > 1 ? " sections do." : " section does."))([appearanceShown ? "Appearance" : "", notifyShown ? "Notifications" : "", harnessShown ? "Harness" : ""].filter(Boolean))
            )
          ) : (
            "Search by name, by what it does, or by its key."
          )}
        </p>
        <label className="flex shrink-0 items-center gap-2 text-[13px] text-muted-foreground">
          <Switch checked={showKeys} onCheckedChange={setShowKeys} aria-label="Show setting keys" />
          Show setting keys
        </label>
      </div>

      <div className="min-w-0 lg:grid lg:grid-cols-[13.5rem_minmax(0,1fr)] lg:gap-10">
        <SectionNav items={navItems} active={current} stopped={stopped} />

        <div className="grid min-w-0 gap-10">
          {!d.available || (d.available && !d.can_edit) || (stopped && page !== "general") ? (
            <div className="grid gap-2 empty:hidden">
              {!d.available ? <Notice tone="warn">Settings aren&apos;t available on this console, so this page is read-only.</Notice> : null}
              {d.available && !d.can_edit ? <Notice tone="info">This console can&apos;t change settings. Open the console on the machine PULSE runs on to change them.</Notice> : null}
              {stopped && page !== "general" ? (
                <Notice tone="bad">
                  All work is paused. Agents won&apos;t start anything new until you resume it under{" "}
                  <a href="#stop" className="font-medium underline underline-offset-2">
                    General
                  </a>
                  .
                </Notice>
              ) : null}
            </div>
          ) : null}

          {killShown && stop && show("stop") ? (
            <section id="stop" aria-label="Pause all work" className={ANCHOR}>
              <h2 className="sr-only">Pause all work</h2>
              <PauseCard s={stop} canEdit={d.can_edit} />
            </section>
          ) : null}

          {shown.filter((x) => x.g.id === "owner" && show("owner")).map(groupSection)}

          {appearanceShown && show("appearance") ? (
            <section id="appearance" aria-label="Appearance" className={ANCHOR}>
              <AppearanceSection icon={iconOf("appearance")} />
            </section>
          ) : null}

          {notifyShown && show("notify") ? (
            <Section id="notify" title="Notifications" note="Hear about it when something waits for you, or when a task or session finishes. For this browser only.">
              <NotificationsCard />
            </Section>
          ) : null}

          {harnessShown && show("harness") ? (
            <Section id="harness" title="Harness" note="The model provider, the models and the limits of each agent run." count={needle && harnessHits ? harnessHits + " of " + harnessRows.length : undefined}>
              <HarnessSettingsCard />
            </Section>
          ) : null}

          {shown.filter((x) => x.g.id !== "owner" && show(x.g.id)).map(groupSection)}

          {needle && !matched && !harnessShown && !notifyShown && !appearanceShown ? (
            <EmptyState
              icon={<SearchIcon className="size-[18px]" />}
              title={<>No setting matches &ldquo;{find.trim()}&rdquo;</>}
              action={
                <Button variant="outline" size="sm" onClick={() => setFind("")}>
                  Clear the search
                </Button>
              }
            >
              Try a plain word like push, model, region or disk, or a key such as compute.max_vms.
            </EmptyState>
          ) : null}
        </div>
      </div>
    </>
  );
}
