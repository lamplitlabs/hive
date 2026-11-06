"use client";

import Link from "next/link";
import { MessageCircleQuestionIcon, MessagesSquareIcon } from "lucide-react";
import { type Column } from "@/components/pulse/data-table";
import { CardGrid, ItemCard, ItemCardBody, ItemCardHeader, ItemCardMeta, ItemCardTitle } from "@/components/pulse/item-card";
import { KindTile } from "@/components/pulse/kind-tile";
import { Markdown } from "@/components/pulse/markdown";
import { SidePanel } from "@/components/pulse/side-panel";
import { Disclosure, EmptyState, ErrorNotice, Kv, SkeletonRows } from "@/components/pulse/panel";
import { Avatar, KindBadge, Mono, PostLink, Provenance, StatusBadge, ToneBadge, VoteBadge, When } from "@/components/pulse/who";
import { hrefs } from "@/lib/nav";
import { enc, projectPath, type PostDetail, type PostRef, type ThreadDetail, type ThreadRow } from "@/lib/api";
import { useApi, type Loaded } from "@/lib/use-api";
import { num, plural, sentence } from "@/lib/format";
import { postKindWord, statusWord, threadName } from "@/lib/words";

// Names as a person lists them: "ada", "ada and bo", "ada, bo and 3 others".
function nameList(items?: string[], cap = 3): string {
  const xs = (items || []).filter(Boolean).map(personName);
  if (!xs.length) return "";
  if (xs.length === 1) return xs[0];
  if (xs.length <= cap) return xs.slice(0, -1).join(", ") + " and " + xs[xs.length - 1];
  return xs.slice(0, cap).join(", ") + " and " + plural(xs.length - cap, "other");
}

const personName = (n: string) => (n === "human" || n === "owner" ? "you" : n);

// A thread is keyed "area/name" ("product/checkout-latency"); people read
// the name as a title ("Checkout latency") and the area as where it lives.
export function threadTitle(t: Pick<ThreadRow, "key" | "thread">): string {
  return t.thread && !t.thread.includes("/") ? threadName(t.thread) : threadName(t.key);
}

// A thread's state as one badge: "Open", or for a finished thread how it
// ended ("Closed, agreed") in a quiet grey - a closed thread is not an alarm.
export function ThreadState({ t }: { t: Pick<ThreadRow, "status" | "resolution"> }) {
  if (t.status === "closed" && t.resolution)
    return (
      <ToneBadge tone="neutral" dot title={"Closed, resolved as " + t.resolution}>
        {"Closed, " + statusWord(t.resolution).toLowerCase()}
      </ToneBadge>
    );
  return <StatusBadge status={t.status} />;
}

// How a post was resolved, as one quiet word ("Agreed").
function Resolution({ r }: { r?: string | null }) {
  return r ? (
    <ToneBadge tone="neutral" title="How this was resolved">
      {statusWord(r)}
    </ToneBadge>
  ) : null;
}

export function ThreadParticipants({ t }: { t: ThreadRow }) {
  const text = nameList(t.participants);
  return text ? <span className="text-[13px] text-muted-foreground anywhere">{"With " + text}</span> : null;
}

export function ThreadLastBy({ t }: { t: ThreadRow }) {
  if (!t.last_by && !t.last_kind) return null;
  const by = t.last_by ? personName(t.last_by) : "an agent";
  return <span className="text-[13px] text-muted-foreground anywhere">{(t.last_kind ? postKindWord(t.last_kind) + " by " : "Last post by ") + by}</span>;
}

// What a thread is waiting on, in words. These are agents asking agents, so
// they stay in the quiet warning tone - the marigold "you" is not theirs.
export function ThreadSignals({ t }: { t: ThreadRow }) {
  const claims = (t.active_claims || []).filter(Boolean);
  return (
    <>
      {t.open_questions > 0 ? <ToneBadge tone="warn">{plural(t.open_questions, "open question")}</ToneBadge> : null}
      {t.open_handoffs > 0 ? <ToneBadge tone="warn" title="Work an agent handed over for someone else to pick up">{t.open_handoffs === 1 ? "1 handover to pick up" : num(t.open_handoffs) + " handovers to pick up"}</ToneBadge> : null}
      {claims.length ? <ToneBadge tone="info">{"Picked up by " + nameList(claims, 2)}</ToneBadge> : null}
    </>
  );
}

export const THREAD_COLUMNS = (slug: string): Column<ThreadRow>[] => [
  {
    key: "thread",
    label: "Thread",
    className: "min-w-40 break-words",
    sort: (t) => t.key,
    render: (t) => (
      <Link href={hrefs.thread(slug, t.key)} title={"Thread " + t.key} className="font-medium underline-offset-2 hover:underline">
        {threadTitle(t)}
      </Link>
    ),
  },
  { key: "status", label: "Status", sort: (t) => t.status + (t.resolution || ""), render: (t) => <ThreadState t={t} /> },
  { key: "participants", label: "Who's in it", hide: "md", render: (t) => <span className="text-[13px] text-muted-foreground anywhere">{nameList(t.participants)}</span> },
  { key: "posts", label: "Posts", align: "right", hide: "sm", sort: (t) => t.posts, render: (t) => num(t.posts) },
  {
    key: "last",
    label: "Last post",
    sort: (t) => t.last,
    render: (t) => (
      <span className="grid gap-0.5">
        <When d={t.last} className="whitespace-nowrap" />
        <ThreadLastBy t={t} />
      </span>
    ),
  },
  { key: "signals", label: "Waiting on", render: (t) => <span className="inline-flex flex-wrap gap-1"><ThreadSignals t={t} /></span> },
  { key: "summary", label: "Latest", hide: "md", render: (t) => <span className="text-muted-foreground anywhere">{sentence(t.summary)}</span> },
];

// One thread as a card: its name as the title, the latest summary as the
// body, who is in it and what it waits on underneath. Opens the thread sheet
// through `onOpen`; without it the card is a plain link to the thread.
export function ThreadCard({ t, slug, onOpen, active }: { t: ThreadRow; slug: string; onOpen?: (key: string) => void; active?: boolean }) {
  const open = onOpen ? () => onOpen(t.key) : undefined;
  return (
    <ItemCard onOpen={open} href={open ? undefined : hrefs.thread(slug, t.key)} active={active} ariaLabel={"Thread " + threadTitle(t)}>
      <ItemCardHeader right={<When d={t.last} />}>
        <ThreadState t={t} />
        {t.area ? <span title={t.key}>{"In " + t.area}</span> : null}
      </ItemCardHeader>
      <ItemCardTitle chevron>{threadTitle(t)}</ItemCardTitle>
      {t.summary ? <ItemCardBody clamp={2}>{sentence(t.summary)}</ItemCardBody> : null}
      <ItemCardMeta>
        <ThreadParticipants t={t} />
        <span>{plural(t.posts, "post")}</span>
      </ItemCardMeta>
      {t.open_questions || t.open_handoffs || t.active_claims?.length ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <ThreadSignals t={t} />
        </div>
      ) : null}
    </ItemCard>
  );
}

export function ThreadCards({ rows, slug, onOpen, activeKey, cols = 2, empty }: { rows: ThreadRow[]; slug: string; onOpen?: (key: string) => void; activeKey?: string | null; cols?: 1 | 2 | 3; empty?: React.ReactNode }) {
  if (!rows.length) return <EmptyState icon={<MessagesSquareIcon className="size-[18px]" />}>{empty ?? "No thread here."}</EmptyState>;
  return (
    <CardGrid cols={cols}>
      {rows.map((t) => (
        <ThreadCard key={t.key} t={t} slug={slug} onOpen={onOpen} active={activeKey === t.key} />
      ))}
    </CardGrid>
  );
}

// Posts inside a sheet - a thread's posts, or the replies to one post - read
// as a conversation: who wrote it, what kind of post it is, when, then what
// they said. The thread is not repeated: the sheet header already names it.
function Conversation({ rows, slug }: { rows: PostRef[]; slug: string }) {
  return (
    <ol className="grid gap-5">
      {rows.map((p) => {
        const name = p.agent || p.name || p.session || "";
        const human = name === "human" || name === "owner";
        const meta = [p.runner, p.model].filter(Boolean).join(", ");
        return (
          <li key={p.id} className="flex min-w-0 gap-3">
            <Avatar seed={name || "?"} size={32} human={human} className="mt-0.5" />
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[13px] text-muted-foreground">
                <span className="font-semibold text-foreground anywhere" title={meta ? name + " (" + meta + ")" : name || undefined}>{human ? "You" : name || "An agent"}</span>
                <KindBadge kind={p.kind} />
                {p.vote ? <VoteBadge vote={p.vote} /> : null}
                <Resolution r={p.resolution} />
                <Provenance ok={p.provenance_ok} missing={p.provenance_missing} />
                <When d={p.date} className="ml-auto text-xs" />
              </div>
              <div className="mt-1.5 rounded-xl border bg-card px-4 py-3">
                <Markdown source={p.summary} className="text-sm leading-relaxed" />
                <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-[13px] text-muted-foreground">
                  {p.on_post ? (
                    <span>
                      In reply to <PostLink slug={slug} id={p.on_post} label="an earlier post" className="text-foreground" />
                    </span>
                  ) : null}
                  <PostLink slug={slug} id={p.id} label="Read the whole post" className="font-medium text-primary" />
                </div>
              </div>
            </div>
          </li>
        );
      })}
    </ol>
  );
}

// A thread or a post opens beside the board (spec "Components" - Side
// panel), 640 px for reading; below a desktop it is a sheet from the bottom.
// The props and the search params that open them are unchanged, so every
// link to a thread or a post still lands on it.
function PanelState({ children }: { children: React.ReactNode }) {
  return <div className="py-1">{children}</div>;
}

export function ThreadSheet({ slug, threadKey, onClose }: { slug: string; threadKey: string | null; onClose: () => void }) {
  const q = useApi<ThreadDetail>(threadKey ? projectPath(slug, "threads/" + enc(threadKey)) : null);
  const t = q.data?.thread;
  const fallback = threadKey ? threadTitle({ key: threadKey, thread: "" }) : "Thread";
  const who = t ? nameList(t.participants, 4) : "";
  return (
    <SidePanel
      open={!!threadKey}
      onClose={onClose}
      width="reading"
      lead={<KindTile kind="session" size={28} title="Thread" />}
      title={t ? threadTitle(t) : fallback}
      subtitle={
        t ? (
          <>
            <StatusBadge status={t.status} size="sm" className="mr-1.5 align-middle" />
            {plural(t.posts, "post") + (who ? " from " + who : "") + " in " + (t.area || "the board")}
            {t.last ? (
              <>
                {" · last "}
                <When d={t.last} />
              </>
            ) : null}
          </>
        ) : q.error ? (
          "A thread on the board"
        ) : (
          "Loading the thread…"
        )
      }
    >
      {threadKey ? <ThreadBody slug={slug} k={threadKey} q={q} /> : null}
    </SidePanel>
  );
}

function ThreadBody({ slug, k, q }: { slug: string; k: string; q: Loaded<ThreadDetail> }) {
  const { data, error, reload } = q;
  if (error && !data)
    return (
      <PanelState>
        <ErrorNotice error={error} onRetry={() => void reload()}>{"This thread didn't load (" + error + "). Check that it is on this project's board."}</ErrorNotice>
      </PanelState>
    );
  if (!data)
    return (
      <PanelState>
        <SkeletonRows n={6} />
      </PanelState>
    );
  const t = data.thread;
  return (
    <div className="grid gap-5">
      <div className="flex flex-wrap items-center gap-2 text-[13px] text-muted-foreground">
        <ThreadState t={t} />
        <span className="font-mono text-xs anywhere" title="The name agents use for this thread in their posts">
          {k}
        </span>
        {t.open_questions || t.open_handoffs || t.active_claims?.length ? <ThreadSignals t={t} /> : null}
      </div>
      {data.posts.length ? <Conversation rows={data.posts} slug={slug} /> : <EmptyState title="No posts yet">Agents haven&apos;t posted in this thread yet.</EmptyState>}
    </div>
  );
}

export function PostSheet({ slug, postId, onClose }: { slug: string; postId: string | null; onClose: () => void }) {
  const q = useApi<PostDetail>(postId ? projectPath(slug, "posts/" + enc(postId)) : null);
  const p = q.data;
  const name = p ? p.agent || p.name || p.session || "" : "";
  const human = name === "human" || name === "owner";
  return (
    <SidePanel
      open={!!postId}
      onClose={onClose}
      width="reading"
      lead={p ? <Avatar seed={name || "?"} size={28} human={human} /> : <KindTile kind="record" size={28} title="Post" />}
      title={p ? sentence(p.summary) : "Post"}
      subtitle={
        p ? (
          <>
            {(human ? "You" : name || "An agent") + " posted "}
            <When d={p.date} />
            {" in " + threadTitle({ key: p.thread_key, thread: p.thread || "" })}
          </>
        ) : q.error ? (
          "A post on the board"
        ) : (
          "Loading the post…"
        )
      }
    >
      {postId ? <PostBody slug={slug} q={q} /> : null}
    </SidePanel>
  );
}

function PostBody({ slug, q }: { slug: string; q: Loaded<PostDetail> }) {
  const { data: p, error, reload } = q;
  if (error && !p)
    return (
      <PanelState>
        <ErrorNotice error={error} onRetry={() => void reload()}>{"This post didn't load (" + error + "). It may belong to another project's board; open it from its thread."}</ErrorNotice>
      </PanelState>
    );
  if (!p)
    return (
      <PanelState>
        <SkeletonRows n={6} />
      </PanelState>
    );
  const fm = p.frontmatter || {};
  const meta = [p.runner, p.model].filter(Boolean).join(", ");
  return (
    <>
      <div className="flex flex-wrap items-center gap-2 text-[13px] text-muted-foreground">
        <KindBadge kind={p.kind} />
        {p.vote ? <VoteBadge vote={p.vote} /> : null}
        <Resolution r={p.resolution} />
        {p.project && p.project !== slug ? <span>{"From " + p.project}</span> : null}
        <span title={meta || undefined}>{meta ? "via " + meta : ""}</span>
        <Link href={hrefs.thread(slug, p.thread_key)} title={"Thread " + p.thread_key} className="font-medium text-foreground underline-offset-2 hover:underline anywhere">
          Open the thread
        </Link>
        <Provenance ok={p.provenance_ok} missing={p.provenance_missing} />
      </div>
      <div className="mt-5 grid gap-6">
        {p.on_post ? (
          <p className="text-[13px] text-muted-foreground">
            In reply to <PostLink slug={slug} id={p.on_post} label="an earlier post" className="font-medium text-foreground" />
          </p>
        ) : null}
        <article aria-label="What the post says">
          {(p.body || "").trim() ? <Markdown source={p.body} /> : <p className="text-sm text-muted-foreground">This post has no text beyond its summary.</p>}
        </article>
        <section aria-label="Replies">
          <h3 className="mb-3 flex items-center gap-2 text-base font-semibold">
            <MessageCircleQuestionIcon className="size-4 text-muted-foreground" aria-hidden />
            {p.responses.length === 1 ? "1 reply" : p.responses.length ? num(p.responses.length) + " replies" : "Replies"}
          </h3>
          {p.responses.length ? <Conversation rows={p.responses} slug={slug} /> : <p className="text-sm text-muted-foreground">No replies yet. Answers, opinions, votes and results that refer to this post show up here.</p>}
        </section>
        <Disclosure summary="Technical details" className="rounded-lg border bg-muted/30 px-4 py-3 text-sm" contentClassName="mt-3 grid gap-3">
            <Kv
              rows={[
                ["Post id", <Mono key="id">{p.id}</Mono>],
                ["Thread", <Mono key="t">{p.thread_key}</Mono>],
                ...(p.rel_path ? ([["File", <Mono key="f">{p.rel_path}</Mono>]] as [React.ReactNode, React.ReactNode][]) : []),
                ...Object.entries(fm).map(([k, v]): [React.ReactNode, React.ReactNode] => [k, k === "on" && v ? <PostLink key={k} slug={slug} id={String(v)} /> : <Mono key={k}>{String(v)}</Mono>]),
              ]}
            />
            <div>
              <div className="mb-1 text-[13px] text-muted-foreground">The post as written</div>
              <pre className="max-h-80 overflow-auto rounded-lg border bg-card p-3 font-mono text-[13px] whitespace-pre-wrap anywhere">{p.body || "(empty)"}</pre>
            </div>
        </Disclosure>
      </div>
    </>
  );
}
