//! `/api/v1/chat`: the hub's conversations - ask an agent to work, watch
//! it, answer its questions, see what it made.
//!
//! The agents run inside the hub process (one worker thread per
//! conversation with a turn in flight) through
//! [`pulse_harness::conversation::Manager`]; this module is the HTTP skin:
//! rows and messages from the store, live state from the manager, steps
//! from the harness session the conversation currently runs, the
//! artifacts folder as files, the branch's changes from git.
use crate::additions::{
    ADDITION_WRITES, addition_failure, addition_method, addition_not_found, addition_string,
};
use crate::httpx::{Request, ResponseWriter};
use crate::server::{Handler, JsonMap, write_error, write_json};
use hyper::header::{self, HeaderValue};
use pulse_harness::conversation::{ImageAttachment, Manager};
use pulse_store::Conversation;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// The manager, built on first use from the handler's store; tests seed it
/// with a pinned provider.
pub(crate) type Chats = Arc<Mutex<Option<Arc<Manager>>>>;

/// The most bytes one artifact may be served at.
pub(crate) const ARTIFACT_LIMIT: u64 = 20 * 1024 * 1024;
/// How many conversations a list returns.
const LIST_LIMIT: i64 = 200;
/// How large a `/message` body may be: every inline image a message may
/// carry at its largest, base64 (4/3), plus the text.
const MESSAGE_BODY_LIMIT: usize = pulse_harness::conversation::ATTACHMENTS_PER_MESSAGE
    * pulse_harness::conversation::IMAGE_LIMIT
    / 3
    * 4
    + crate::server::JSON_BODY_LIMIT;

/// How often the hub looks for conversations that have sat idle long
/// enough to let their agents go.
const IDLE_RELEASE_TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// Lets go of the agents of conversations idle for `chat.idle_release_minutes`
/// or longer ([`Manager::release_idle`]), every [`IDLE_RELEASE_TICK`],
/// until the hub stops. The setting is read on each pass, so a change
/// takes effect without a restart; 0 keeps every agent.
pub(crate) fn idle_release_loop(ctx: &pulse_bus::CancellationToken, chats: &Chats) {
    // `done` is a receiver that disconnects when the token is cancelled, so
    // the wait ends early on a shutdown instead of at the next tick.
    let done = ctx.done();
    loop {
        let _ = done.recv_timeout(IDLE_RELEASE_TICK);
        if ctx.is_cancelled() {
            return;
        }
        let manager = chats.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(m) = manager {
            m.release_idle_by_setting();
        }
    }
}

/// How long the wake loop waits on the bus between checks of the stop flag.
const WAKE_RECV: std::time::Duration = std::time::Duration::from_secs(15);

/// Turns every `chat.wake` event (`pulse chat wake`, run by a script an
/// agent left running) into a turn of the conversation it names
/// ([`Manager::wake`]), until the hub stops. The bus sequence delivered up
/// to is kept in the store (`chat.wake.cursor`), so a wake sent while no
/// hub ran reaches its agent when the next hub starts.
pub(crate) fn wake_loop(
    ctx: &pulse_bus::CancellationToken,
    store: &pulse_store::Store,
    chats: &Chats,
) {
    use pulse_harness::conversation::{WAKE_ACTION, WAKE_CURSOR_KEY};
    let since = match store
        .meta(WAKE_CURSOR_KEY)
        .ok()
        .filter(|(_, found)| *found)
        .and_then(|(v, _)| v.trim().parse::<i64>().ok())
    {
        Some(seq) => seq,
        None => {
            // First hub with the feature: nothing could have woken a
            // conversation before now, so start here and say so, so the
            // next start has a cursor to resume from.
            let seq = pulse_bus::last_id(store).unwrap_or(0);
            let _ = store.set_meta(WAKE_CURSOR_KEY, &seq.to_string());
            seq
        }
    };
    let tail = pulse_bus::tail(
        ctx.clone(),
        store.clone(),
        since,
        pulse_bus::Filter {
            actions: vec![WAKE_ACTION.to_string()],
            ..Default::default()
        },
        std::time::Duration::from_secs(1),
    );
    loop {
        if ctx.is_cancelled() {
            return;
        }
        let event = match tail.recv_timeout(WAKE_RECV) {
            Ok(event) => event,
            Err(err) if err.is_timeout() => continue,
            Err(_) => return,
        };
        let manager = chats.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(m) = manager {
            deliver_wake(&m, &event);
        }
        let _ = store.set_meta(WAKE_CURSOR_KEY, &event.id.to_string());
    }
}

/// Hands one `chat.wake` event to its conversation; a wake for a
/// conversation that is gone or archived is dropped with a log line, not
/// an error that stops the loop.
pub(crate) fn deliver_wake(m: &Manager, event: &pulse_bus::Event) {
    let text = serde_json::from_str::<Value>(&event.detail)
        .ok()
        .and_then(|d| d.get("text").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| event.detail.clone());
    if let Err(err) = m.wake(&event.subject, &text, &event.actor) {
        eprintln!("chat wake {}: {err:#}", event.subject);
    }
}

impl Handler {
    pub(crate) fn chats(&self) -> Option<Arc<Manager>> {
        self.open_chats().map(|(m, _, _)| m)
    }

    /// The manager, built now when it was not yet, with the ids of the
    /// conversations whose turn the last hub left cut and of those whose
    /// turn it stopped to restart and that carry on now (both empty when
    /// the manager already existed). A new manager holds no runtime, so
    /// rows that still say "running" are the last hub's: they go idle with
    /// a note before anything is served.
    pub(crate) fn open_chats(&self) -> Option<(Arc<Manager>, Vec<String>, Vec<String>)> {
        let store = self.store.as_deref()?;
        let mut slot = self.chats.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = slot.as_ref() {
            return Some((m.clone(), Vec::new(), Vec::new()));
        }
        let manager = Manager::new(&store.home, store.clone());
        let cut = manager.recover();
        let carried = manager.carry_on_restarted();
        let manager = Arc::new(manager);
        *slot = Some(manager.clone());
        Some((manager, cut, carried))
    }

    pub(crate) fn addition_chat(&self, w: &mut ResponseWriter, r: &Request, parts: Vec<String>) {
        if !self.addition_available(w, r, vec!["conversations".to_string()]) {
            return;
        }
        let Some(chats) = self.chats() else {
            addition_not_found(w);
            return;
        };
        match parts.as_slice() {
            [_] => {
                if !addition_method(w, r, vec!["GET".to_string(), "POST".to_string()]) {
                    return;
                }
                if r.method == "POST" {
                    self.chat_create(w, r, &chats);
                } else {
                    self.chat_list(w, r, &chats);
                }
            }
            [_, id] => {
                if addition_method(w, r, vec!["GET".to_string()]) {
                    self.chat_show(w, r, &chats, id);
                }
            }
            [_, id, action] if action == "diff" => {
                if addition_method(w, r, vec!["GET".to_string()]) {
                    self.chat_diff(w, &chats, id);
                }
            }
            [_, id, action] if action == "upload" => {
                if addition_method(w, r, vec!["POST".to_string()]) {
                    self.chat_upload(w, r, &chats, id);
                }
            }
            [_, id, action] => {
                if addition_method(w, r, vec!["POST".to_string()]) {
                    self.chat_action(w, r, &chats, id, action);
                }
            }
            [_, id, kind, name] if kind == "artifacts" || kind == "attachments" => {
                if addition_method(w, r, vec!["GET".to_string(), "HEAD".to_string()]) {
                    self.chat_file(w, r, &chats, id, kind, name);
                }
            }
            [_, id, kind, message_n, n] if kind == "image" => {
                if addition_method(w, r, vec!["GET".to_string(), "HEAD".to_string()]) {
                    self.chat_image(w, r, &chats, id, message_n, n);
                }
            }
            _ => addition_not_found(w),
        }
    }

    fn chat_list(&self, w: &mut ResponseWriter, r: &Request, chats: &Manager) {
        let Some(store) = self.store.as_deref() else {
            return;
        };
        let all = matches!(r.query("all").as_str(), "1" | "true" | "yes");
        let rows = match store.chat_list(all, LIST_LIMIT) {
            Ok(rows) => rows,
            Err(e) => {
                addition_failure(w, e);
                return;
            }
        };
        let waiting = chats.waiting_to_land(&rows);
        let reviewing = chats.reviewing(&rows);
        let mut prs = ProjectPrs::new(store);
        let items: Vec<Value> = rows
            .iter()
            .map(|row| {
                conversation_json(
                    row,
                    chats,
                    waiting.contains(&row.id),
                    reviewing.contains(&row.id),
                    prs.of(row),
                )
            })
            .collect();
        // The tasks the page groups conversations under (chat_tasks); an
        // older store without the task tables lists none.
        let tasks = match crate::chat_tasks::task_groups(store, all) {
            Ok(tasks) => tasks,
            Err(e) => {
                addition_failure(w, e);
                return;
            }
        };
        write_json(w, 200, json!({"conversations": items, "tasks": tasks}));
    }

    fn chat_create(&self, w: &mut ResponseWriter, r: &Request, chats: &Manager) {
        let (payload, ok) = crate::server::read_payload(w, r);
        if !ok {
            return;
        }
        let (text, ok) = addition_string(w, &payload, "text", true);
        if !ok {
            return;
        }
        let (title, ok) = addition_string(w, &payload, "title", false);
        if !ok {
            return;
        }
        let (mut project, ok) = addition_string(w, &payload, "project", false);
        if !ok {
            return;
        }
        if project.is_empty() {
            project = self.project.clone();
        }
        let (model, ok) = addition_string(w, &payload, "model", false);
        if !ok {
            return;
        }
        let (reasoning, ok) = addition_string(w, &payload, "reasoning", false);
        if !ok {
            return;
        }
        let Some(auto) = payload_bool(w, &payload, "auto_approve", true) else {
            return;
        };
        let Some(lockdown) = payload_bool(w, &payload, "lockdown", false) else {
            return;
        };
        let (parent, ok) = addition_string(w, &payload, "parent", false);
        if !ok {
            return;
        }
        let _lock = ADDITION_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        // A sub-conversation of `parent` (the owner starting one from the
        // parent's page) takes the parent's workspace and settings; the
        // other fields are the parent's.
        let created = if parent.is_empty() {
            chats.create_with_lockdown(
                &title,
                &project,
                &model,
                &reasoning,
                auto,
                lockdown,
                &text,
                pulse_store::HUMAN_AGENT,
            )
        } else {
            chats.create_child(&parent, &title, &text)
        };
        match created {
            Ok(row) => write_json(
                w,
                201,
                json!({"conversation": conversation_json_one(self.store.as_deref(), &row, chats)}),
            ),
            Err(e) => addition_failure(w, e),
        }
    }

    fn chat_show(&self, w: &mut ResponseWriter, r: &Request, chats: &Manager, id: &str) {
        let Some(store) = self.store.as_deref() else {
            return;
        };
        let Some(row) = self.chat_row(w, id) else {
            return;
        };
        // Messages count from 1 and steps from 0, so "everything" is
        // `since=0` for messages and `after=-1` for steps.
        let (since, ok) = query_int(w, r.query("since"), "since", 0, 0);
        if !ok {
            return;
        }
        let (after, ok) = query_int(w, r.query("after"), "after", -1, -1);
        if !ok {
            return;
        }
        let live = chats.live(&row);
        let session_id = if live.session_id.is_empty() {
            row.session_id.clone()
        } else {
            live.session_id.clone()
        };
        let messages = match store.chat_messages(id, since) {
            Ok(m) => m,
            Err(e) => {
                addition_failure(w, e);
                return;
            }
        };
        let steps = if session_id.is_empty() {
            Vec::new()
        } else {
            match store.harness_steps(&session_id, after) {
                Ok(s) => s,
                Err(e) => {
                    addition_failure(w, e);
                    return;
                }
            }
        };
        let artifacts = chats.artifacts(id);
        let changes = chats.changes(&row);
        // The family: the conversation this one is a sub-conversation of,
        // and its own sub-conversations (every one, put-away ones too,
        // each with its live state).
        let parent_title = if row.parent.is_empty() {
            String::new()
        } else {
            store
                .chat_get(&row.parent)
                .ok()
                .flatten()
                .map(|p| p.title)
                .unwrap_or_default()
        };
        let children = match store.chat_children(id, true) {
            Ok(rows) => rows,
            Err(e) => {
                addition_failure(w, e);
                return;
            }
        };
        // One git question for the family: the row and its children share
        // the row's branch, and the page words each from the same answer.
        let mut family = children.clone();
        family.push(row.clone());
        let waiting = chats.waiting_to_land(&family);
        let reviewing = chats.reviewing(&family);
        let mut prs = ProjectPrs::new(store);
        let children: Vec<Value> = children
            .iter()
            .map(|c| {
                conversation_json(
                    c,
                    chats,
                    waiting.contains(&c.id),
                    reviewing.contains(&c.id),
                    prs.of(c),
                )
            })
            .collect();
        let mut conversation = serde_json::to_value(&row).unwrap_or(Value::Null);
        if let Value::Object(map) = &mut conversation {
            map.insert(
                "waiting_to_land".to_string(),
                Value::Bool(waiting.contains(&row.id)),
            );
            map.insert(
                "reviewing".to_string(),
                Value::Bool(reviewing.contains(&row.id)),
            );
            map.insert("pr".to_string(), prs.of(&row));
        }
        write_json(
            w,
            200,
            json!({
                "conversation": conversation,
                "live": live,
                "session_id": session_id,
                "messages": messages,
                "since": since,
                "steps": steps,
                "after": after,
                "artifacts": artifacts,
                "changes": changes,
                "parent_title": parent_title,
                "children": children,
            }),
        );
    }

    fn chat_diff(&self, w: &mut ResponseWriter, chats: &Manager, id: &str) {
        let Some(row) = self.chat_row(w, id) else {
            return;
        };
        let diff = chats.diff(&row).unwrap_or_default();
        write_json(
            w,
            200,
            json!({"id": id, "branch": row.branch, "diff": diff}),
        );
    }

    /// `POST /api/v1/chat/ID/upload?name=FILE` with the file's bytes as the
    /// body: keeps it for a message to carry (`attachments` on `message`).
    fn chat_upload(&self, w: &mut ResponseWriter, r: &Request, chats: &Manager, id: &str) {
        if !crate::server::same_origin_write(w, r) {
            return;
        }
        if self.chat_row(w, id).is_none() {
            return;
        }
        let name = r.query("name");
        if name.trim().is_empty() {
            write_error(
                w,
                400,
                "Missing name".to_string(),
                "The upload needs the file's name in the query string.".to_string(),
                "Send POST /api/v1/chat/ID/upload?name=FILE with the bytes as the body."
                    .to_string(),
            );
            return;
        }
        if r.body.is_empty() || r.body.len() > pulse_harness::conversation::ATTACHMENT_LIMIT {
            write_error(
                w,
                413,
                "File too large or empty".to_string(),
                format!(
                    "The body is {} bytes; attachments go from 1 to {} bytes.",
                    r.body.len(),
                    pulse_harness::conversation::ATTACHMENT_LIMIT
                ),
                "Attach a smaller file, or put it in the project and name its path.".to_string(),
            );
            return;
        }
        let _lock = ADDITION_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        match chats.save_attachment(id, &name, &r.body) {
            Ok(a) => write_json(w, 201, json!({"attachment": a})),
            Err(e) => addition_failure(w, e),
        }
    }

    fn chat_action(
        &self,
        w: &mut ResponseWriter,
        r: &Request,
        chats: &Manager,
        id: &str,
        action: &str,
    ) {
        let limit = if action == "message" || action == "steer" {
            MESSAGE_BODY_LIMIT
        } else {
            crate::server::JSON_BODY_LIMIT
        };
        let (payload, ok) = crate::server::read_payload_up_to(w, r, limit);
        if !ok {
            return;
        }
        let Some(row) = self.chat_row(w, id) else {
            return;
        };
        let _lock = ADDITION_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let result = match action {
            "message" | "steer" => {
                let (text, ok) = addition_string(w, &payload, "text", false);
                if !ok {
                    return;
                }
                let Some(attachments) = payload_names(w, &payload, "attachments") else {
                    return;
                };
                let Some(images) = payload_images(w, &payload, "images") else {
                    return;
                };
                // A steer reaches the agent inside the running turn; a
                // message waits behind it.
                match (action, images.is_empty()) {
                    ("steer", true) => chats.steer_with(id, &text, &attachments),
                    ("steer", false) => chats.steer_with_images(id, &text, &attachments, &images),
                    (_, true) => chats.send_with(id, &text, &attachments),
                    (_, false) => chats.send_with_images(id, &text, &attachments, &images),
                }
            }
            "answer" => {
                let (text, ok) = addition_string(w, &payload, "text", true);
                if !ok {
                    return;
                }
                chats.answer(id, &text)
            }
            "approve" => {
                let (decision, ok) = addition_string(w, &payload, "decision", true);
                if !ok {
                    return;
                }
                let (reason, ok) = addition_string(w, &payload, "reason", false);
                if !ok {
                    return;
                }
                chats.approve(id, decision.trim(), &reason)
            }
            "cancel" => chats.cancel(id),
            "archive" => chats.archive(id),
            "close" => chats.close(id),
            "restart" => chats.restart_session(id),
            "auto" => {
                let Some(on) = payload_bool(w, &payload, "on", true) else {
                    return;
                };
                chats.set_auto_approve(id, on)
            }
            "lockdown" => {
                let Some(on) = payload_bool(w, &payload, "on", true) else {
                    return;
                };
                chats.set_lockdown(id, on)
            }
            "attach" => {
                let (project, ok) = addition_string(w, &payload, "project", true);
                if !ok {
                    return;
                }
                match chats.attach(id, project.trim()) {
                    Ok(attached) => {
                        if let Some(row) = self.chat_row(w, id) {
                            write_json(
                                w,
                                200,
                                json!({"conversation": conversation_json_one(self.store.as_deref(), &row, chats), "attached": attached}),
                            );
                        }
                        return;
                    }
                    Err(e) => Err(e),
                }
            }
            "title" => {
                let (title, ok) = addition_string(w, &payload, "title", true);
                if !ok {
                    return;
                }
                chats.set_title(id, title.trim())
            }
            "model" => {
                let (model, ok) = addition_string(w, &payload, "model", false);
                if !ok {
                    return;
                }
                chats.set_model(id, &model)
            }
            "reasoning" => {
                let (reasoning, ok) = addition_string(w, &payload, "reasoning", false);
                if !ok {
                    return;
                }
                chats.set_reasoning(id, &reasoning)
            }
            _ => {
                addition_not_found(w);
                return;
            }
        };
        let _ = row;
        match result {
            Ok(()) => {
                if let Some(row) = self.chat_row(w, id) {
                    write_json(
                        w,
                        200,
                        json!({"conversation": conversation_json_one(self.store.as_deref(), &row, chats)}),
                    );
                }
            }
            Err(e) => addition_failure(w, e),
        }
    }

    fn chat_file(
        &self,
        w: &mut ResponseWriter,
        r: &Request,
        chats: &Manager,
        id: &str,
        kind: &str,
        name: &str,
    ) {
        if self.chat_row(w, id).is_none() {
            return;
        }
        let path = if kind == "attachments" {
            chats.attachment_path(id, name)
        } else {
            chats.artifact_path(id, name)
        };
        let Some(path) = path else {
            let what = if kind == "attachments" {
                "attachment"
            } else {
                "artifact"
            };
            write_error(
                w,
                404,
                format!("No such {what}"),
                format!("{name} is not a file in this session's {kind} folder."),
                "Open the session and pick a file from its list.".to_string(),
            );
            return;
        };
        serve_artifact_file(w, r, name, &path);
    }

    /// One image of one message: `GET /api/v1/chat/{id}/image/{n}/{i}` is
    /// the `i`-th image of message `n`, served with the mime it arrived
    /// with. Nothing to delete: images go with the conversation.
    fn chat_image(
        &self,
        w: &mut ResponseWriter,
        r: &Request,
        chats: &Arc<Manager>,
        id: &str,
        message_n: &str,
        n: &str,
    ) {
        let Ok(message_n) = message_n.parse::<i64>() else {
            addition_not_found(w);
            return;
        };
        let Ok(n) = n.parse::<usize>() else {
            addition_not_found(w);
            return;
        };
        match chats.image(id, message_n, n) {
            Ok(Some((mime, data))) => {
                if let Ok(v) = HeaderValue::from_str(&mime) {
                    w.header_mut().insert(header::CONTENT_TYPE, v);
                }
                w.header_mut().insert(
                    header::X_CONTENT_TYPE_OPTIONS,
                    HeaderValue::from_static("nosniff"),
                );
                w.header_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                crate::httpx::serve_content(w, r, "image", &data);
            }
            Ok(None) => addition_not_found(w),
            Err(e) => addition_failure(w, e.to_string()),
        }
    }

    fn chat_row(&self, w: &mut ResponseWriter, id: &str) -> Option<Conversation> {
        let store = self.store.as_deref()?;
        match store.chat_get(id) {
            Ok(Some(row)) => Some(row),
            Ok(None) => {
                write_error(
                    w,
                    404,
                    "No such session".to_string(),
                    format!("There is no session {id}."),
                    "Pick one from the list.".to_string(),
                );
                None
            }
            Err(e) => {
                addition_failure(w, e);
                None
            }
        }
    }
}

/// The row with its live state, whether its branch waits for its pull
/// request (commits the project's default branch lacks;
/// `Manager::waiting_to_land`), `reviewing` (a landing of the older flow is
/// merging its branch) and `pr`: where the pull request opened from its
/// branch stands (`ProjectPrs::of`), null without one. The page groups by
/// `pr` first and falls back to `waiting_to_land`.
fn conversation_json(
    row: &Conversation,
    chats: &Manager,
    waiting_to_land: bool,
    reviewing: bool,
    pr: Value,
) -> Value {
    let live = chats.live(row);
    let mut v = serde_json::to_value(row).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut v {
        map.insert(
            "live".to_string(),
            serde_json::to_value(&live).unwrap_or(Value::Null),
        );
        map.insert("waiting_to_land".to_string(), Value::Bool(waiting_to_land));
        map.insert("reviewing".to_string(), Value::Bool(reviewing));
        map.insert("pr".to_string(), pr);
    }
    v
}

/// `conversation_json` for one row, asking git about its branch.
fn conversation_json_one(
    store: Option<&pulse_store::Store>,
    row: &Conversation,
    chats: &Manager,
) -> Value {
    let one = std::slice::from_ref(row);
    let waiting = chats.waiting_to_land(one);
    let reviewing = chats.reviewing(one);
    conversation_json(
        row,
        chats,
        waiting.contains(&row.id),
        reviewing.contains(&row.id),
        store
            .map(|s| ProjectPrs::new(s).of(row))
            .unwrap_or(Value::Null),
    )
}

/// The projects' pull requests, read once per project while a list is
/// worded: `pr_states_for` is one query over the project's events, and the
/// list of two hundred conversations would otherwise run it per row.
struct ProjectPrs<'a> {
    store: &'a pulse_store::Store,
    by_project: BTreeMap<String, Vec<pulse_tasks::PrState>>,
}

impl<'a> ProjectPrs<'a> {
    fn new(store: &'a pulse_store::Store) -> Self {
        Self {
            store,
            by_project: BTreeMap::new(),
        }
    }

    /// The conversation's pull request, newest first: the one opened for
    /// the conversation itself (`kind: proposed`) or from its branch. A
    /// list must not fail because the events could not be read, so an
    /// error reads as no pull request.
    fn of(&mut self, row: &Conversation) -> Value {
        if row.project.is_empty() {
            return Value::Null;
        }
        let prs = self
            .by_project
            .entry(row.project.clone())
            .or_insert_with(|| {
                pulse_tasks::pr_states_for(self.store, &row.project).unwrap_or_default()
            });
        prs.iter()
            .find(|p| p.conversation == row.id || (!row.branch.is_empty() && p.head == row.branch))
            .map(|p| serde_json::to_value(p).unwrap_or(Value::Null))
            .unwrap_or(Value::Null)
    }
}

fn payload_bool(
    w: &mut ResponseWriter,
    payload: &JsonMap,
    key: &str,
    fallback: bool,
) -> Option<bool> {
    match payload.get(key) {
        None | Some(Value::Null) => Some(fallback),
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::String(s)) => match s.trim() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => {
                invalid_bool(w, key);
                None
            }
        },
        Some(Value::Number(n)) => Some(n.as_i64().unwrap_or(0) != 0),
        _ => {
            invalid_bool(w, key);
            None
        }
    }
}

fn invalid_bool(w: &mut ResponseWriter, key: &str) {
    write_error(
        w,
        400,
        format!("Invalid {key}"),
        format!("{key} must be true or false."),
        format!("Send {key} as a JSON boolean."),
    );
}

/// An optional list of file names under `key` (a JSON array of strings).
fn payload_names(w: &mut ResponseWriter, payload: &JsonMap, key: &str) -> Option<Vec<String>> {
    match payload.get(key) {
        None | Some(Value::Null) => Some(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                match item.as_str() {
                    Some(s) if !s.trim().is_empty() => out.push(s.trim().to_string()),
                    _ => {
                        invalid_names(w, key);
                        return None;
                    }
                }
            }
            Some(out)
        }
        _ => {
            invalid_names(w, key);
            None
        }
    }
}

fn invalid_names(w: &mut ResponseWriter, key: &str) {
    write_error(
        w,
        400,
        format!("Invalid {key}"),
        format!("{key} must be a list of file names."),
        format!("Send {key} as a JSON array of the names the upload returned."),
    );
}

/// The `images` of a message body: a list of `{mime, data}` with the bytes
/// base64. Missing means none. `None` back (after an error is written) when
/// the shape is wrong.
fn payload_images(
    w: &mut ResponseWriter,
    payload: &JsonMap,
    key: &str,
) -> Option<Vec<ImageAttachment>> {
    match payload.get(key) {
        None | Some(Value::Null) => Some(Vec::new()),
        Some(v) => match serde_json::from_value::<Vec<ImageAttachment>>(v.clone()) {
            Ok(images) => Some(images),
            Err(e) => {
                write_error(
                    w,
                    400,
                    format!("Invalid {key}"),
                    format!(
                        "{key} must be a list of images, each a {{mime, data}} with the bytes base64: {e}"
                    ),
                    format!(
                        "Send {key} as a JSON array like [{{\"mime\": \"image/png\", \"data\": \"<base64>\"}}]."
                    ),
                );
                None
            }
        },
    }
}

fn query_int(
    w: &mut ResponseWriter,
    value: String,
    name: &str,
    default: i64,
    min: i64,
) -> (i64, bool) {
    if value.is_empty() {
        return (default, true);
    }
    match value.parse::<i64>() {
        Ok(n) if n >= min => (n, true),
        _ => {
            write_error(
                w,
                400,
                format!("Invalid {name}"),
                format!("{name} must be a whole number of at least {min}."),
                format!("Use a valid {name} query parameter."),
            );
            (default, false)
        }
    }
}

pub(crate) fn looks_like_text(data: &[u8]) -> bool {
    let head = &data[..data.len().min(4096)];
    !head.contains(&0) && std::str::from_utf8(head).is_ok()
}

/// Serve one artifact file (a conversation's or a job's) at `path`, already
/// resolved inside its folder: 413 above [`ARTIFACT_LIMIT`], the mime by
/// extension (text/plain or octet-stream when unknown) and the headers that
/// let the hub page frame it without the file reaching the hub's origin.
pub(crate) fn serve_artifact_file(
    w: &mut ResponseWriter,
    r: &Request,
    name: &str,
    path: &std::path::Path,
) {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size > ARTIFACT_LIMIT {
        write_error(
            w,
            413,
            "Artifact too large".to_string(),
            format!("{name} is {size} bytes; the hub serves files up to {ARTIFACT_LIMIT} bytes."),
            format!("Open it from disk: {}", path.display()),
        );
        return;
    }
    let Ok(data) = std::fs::read(path) else {
        addition_not_found(w);
        return;
    };
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut mime = crate::httpx::mime_type_by_extension(&ext);
    if mime.is_empty() {
        mime = if looks_like_text(&data) {
            "text/plain; charset=utf-8".to_string()
        } else {
            "application/octet-stream".to_string()
        };
    }
    if let Ok(v) = HeaderValue::from_str(&mime) {
        w.header_mut().insert(header::CONTENT_TYPE, v);
    }
    w.header_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    w.header_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // The chat page frames artifacts (an HTML page in a sandboxed
    // iframe, a PDF in the browser's viewer), so the hub's blanket
    // "never frame" headers give way to same-origin framing here. A page
    // an agent wrote still runs with no access to the hub's origin; a
    // PDF keeps its viewer (CSP `sandbox` would switch plugins off).
    w.header_mut().insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("SAMEORIGIN"),
    );
    let csp = if mime.starts_with("text/html") || mime.contains("svg") {
        "sandbox allow-scripts"
    } else {
        "default-src 'none'; style-src 'unsafe-inline'"
    };
    w.header_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(csp),
    );
    crate::httpx::serve_content(w, r, name, &data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers_http::{additions_project, additions_store, handler, json_body};
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use std::time::Duration;

    fn fake_handler(
        store: &pulse_store::Store,
        script: Value,
    ) -> (Handler, pulse_testutil::SetenvGuard) {
        pulse_provider_fake::register();
        let path = std::path::Path::new(&store.home).join("script.json");
        std::fs::write(&path, json!({"replies": script}).to_string()).expect("script");
        let guard = pulse_testutil::setenv("PULSE_HARNESS_FAKE_SCRIPT", path.display().to_string());
        let h = handler(store);
        let manager = Manager::new(&store.home, store.clone()).with_provider("fake", "fake-1");
        *h.chats.lock().unwrap() = Some(Arc::new(manager));
        (h, guard)
    }

    fn call(h: &Handler, method: &str, path: &str, body: Value) -> (u16, JsonMap, String) {
        let body = if body.is_null() { json!({}) } else { body };
        crate::test_helpers_http::request(h.clone(), method, path, &json_body(body), &[])
    }

    fn wait_status(h: &Handler, id: &str, want: &[&str]) -> JsonMap {
        for _ in 0..400 {
            let (code, body, _) = call(h, "GET", &format!("/api/v1/chat/{id}"), Value::Null);
            assert_eq!(code, 200);
            let status = body["live"]["status"].as_str().unwrap_or("").to_string();
            let queued = body["live"]["queued"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0);
            if want.contains(&status.as_str()) && queued == 0 {
                return body;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("conversation {id} never reached {want:?}");
    }

    #[test]
    fn create_show_message_and_archive_over_http() {
        let _env = pulse_testutil::env_lock();
        let ts = additions_store(false);
        let (h, _script) =
            fake_handler(&ts, json!([{"content": "Hi there."}, {"content": "Done."}]));
        let (code, body, raw) = call(
            &h,
            "POST",
            "/api/v1/chat",
            json!({"text": "hello agent", "title": ""}),
        );
        assert_eq!(code, 201, "{raw}");
        let id = body["conversation"]["id"].as_str().unwrap().to_string();
        assert_eq!(body["conversation"]["title"], "hello agent");
        assert!(body["conversation"]["live"]["status"].is_string());
        let shown = wait_status(&h, &id, &["idle"]);
        let roles: Vec<&str> = shown["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["user", "assistant"]);
        assert_eq!(shown["messages"][1]["content"], "Hi there.");
        assert!(!shown["session_id"].as_str().unwrap().is_empty());
        assert!(
            !shown["steps"].as_array().unwrap().is_empty(),
            "live steps come from the harness session"
        );
        assert_eq!(shown["after"], -1, "no after means every step, from n = 0");
        assert_eq!(shown["steps"][0]["n"], 0, "the first step is included");
        let (code, _, _) = call(
            &h,
            "GET",
            &format!("/api/v1/chat/{id}?after=-2"),
            Value::Null,
        );
        assert_eq!(code, 400);
        assert!(shown["changes"].is_null(), "no project, no branch");
        // Incremental fetch: only what is new.
        let (_, inc, _) = call(
            &h,
            "GET",
            &format!("/api/v1/chat/{id}?since=2&after=100"),
            Value::Null,
        );
        assert_eq!(inc["messages"].as_array().unwrap().len(), 0);
        assert_eq!(inc["steps"].as_array().unwrap().len(), 0);
        assert_eq!(inc["since"], 2);
        // A second message, then the list shows the conversation idle.
        let (code, _, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/message"),
            json!({"text": "more"}),
        );
        assert_eq!(code, 200, "{raw}");
        let shown = wait_status(&h, &id, &["idle"]);
        assert_eq!(shown["messages"].as_array().unwrap().len(), 4);
        assert_eq!(shown["conversation"]["turns"], 2);
        let (code, list, _) = call(&h, "GET", "/api/v1/chat", Value::Null);
        assert_eq!(code, 200);
        let items = list["conversations"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["live"]["status"], "idle");
        // Title, then archive; archived rows leave the default list.
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/title"),
            json!({"title": "Greetings"}),
        );
        assert_eq!(code, 200);
        let (code, body, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/archive"),
            Value::Null,
        );
        assert_eq!(code, 200);
        assert_eq!(body["conversation"]["status"], "archived");
        assert_eq!(body["conversation"]["title"], "Greetings");
        let (_, list, _) = call(&h, "GET", "/api/v1/chat", Value::Null);
        assert!(list["conversations"].as_array().unwrap().is_empty());
        let (_, list, _) = call(&h, "GET", "/api/v1/chat?all=1", Value::Null);
        assert_eq!(list["conversations"].as_array().unwrap().len(), 1);
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/message"),
            json!({"text": "x"}),
        );
        assert_eq!(code, 409, "an archived conversation takes no messages");
        std::thread::sleep(Duration::from_millis(100));
    }

    /// A steer while the turn runs goes into the turn (the agent takes it
    /// before its next model call), not behind it: the live state lists it
    /// under `steered`, never `queued`, and the row is in the transcript at
    /// once, marked. Steering an idle conversation is a plain message.
    #[test]
    fn a_steer_over_http_reaches_the_running_turn() {
        let _env = pulse_testutil::env_lock();
        let ts = additions_store(false);
        let (h, _script) = fake_handler(
            &ts,
            json!([
                {"content": "First."},
                {"command": "sleep 1"},
                {"content": "Second."},
                {"content": "Third."}
            ]),
        );
        let (code, body, raw) = call(&h, "POST", "/api/v1/chat", json!({"text": "one"}));
        assert_eq!(code, 201, "{raw}");
        let id = body["conversation"]["id"].as_str().unwrap().to_string();
        wait_status(&h, &id, &["idle"]);
        let (code, _, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/message"),
            json!({"text": "two"}),
        );
        assert_eq!(code, 200, "{raw}");
        for _ in 0..400 {
            let (_, body, _) = call(&h, "GET", &format!("/api/v1/chat/{id}"), Value::Null);
            if body["live"]["status"] == "running" {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        std::thread::sleep(Duration::from_millis(200));
        let (code, body, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/steer"),
            json!({"text": "actually, b"}),
        );
        assert_eq!(code, 200, "{raw}");
        let live = &body["conversation"]["live"];
        assert_eq!(live["queued"], json!([]), "a steer does not queue: {live}");
        assert_eq!(live["steered"], json!(["actually, b"]), "{live}");
        let shown = wait_status(&h, &id, &["idle"]);
        let rows: Vec<(&str, &str)> = shown["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["role"].as_str().unwrap(), m["content"].as_str().unwrap()))
            .collect();
        assert_eq!(
            rows,
            [
                ("user", "one"),
                ("assistant", "First."),
                ("user", "two"),
                ("user", "actually, b"),
                ("assistant", "Second."),
            ],
            "the steer ran inside the turn"
        );
        assert_eq!(shown["messages"][3]["extra"]["steered"], true);
        assert_eq!(shown["live"]["steered"], json!([]));
        // Idle, a steer is a message of its own.
        let (code, _, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/steer"),
            json!({"text": "four"}),
        );
        assert_eq!(code, 200, "{raw}");
        let shown = wait_status(&h, &id, &["idle"]);
        let msgs = shown["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 7, "{msgs:#?}");
        assert_eq!(msgs[5]["content"], "four");
        assert!(msgs[5]["extra"]["steered"].is_null(), "{:?}", msgs[5]);
        assert_eq!(msgs[6]["content"], "Third.");
        std::thread::sleep(Duration::from_millis(100));
    }

    /// Polls the conversation until its last message is `content` (the
    /// agent's reply to a turn it was given from outside), returning the
    /// full view.
    fn wait_last(h: &Handler, id: &str, content: &str) -> JsonMap {
        for _ in 0..400 {
            let (code, body, _) = call(h, "GET", &format!("/api/v1/chat/{id}"), Value::Null);
            assert_eq!(code, 200);
            let last = body["messages"]
                .as_array()
                .and_then(|m| m.last())
                .map(|m| m["content"].clone());
            if last.as_ref().and_then(Value::as_str) == Some(content) {
                return body;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("conversation {id} never answered {content:?}");
    }

    #[test]
    fn a_chat_wake_event_on_the_bus_gives_the_agent_a_turn() {
        use pulse_harness::conversation::{WAKE_ACTION, WAKE_CURSOR_KEY};
        let _env = pulse_testutil::env_lock();
        let ts = additions_store(false);
        let (h, _script) = fake_handler(
            &ts,
            json!([
                {"content": "Started the soak; I will hear from it."},
                {"content": "The soak passed; closing."},
                {"content": "Second wake seen."}
            ]),
        );
        let (code, body, raw) = call(
            &h,
            "POST",
            "/api/v1/chat",
            json!({"text": "run the soak", "title": ""}),
        );
        assert_eq!(code, 201, "{raw}");
        let id = body["conversation"]["id"].as_str().unwrap().to_string();
        wait_status(&h, &id, &["idle"]);

        // A wake written while no loop ran (the hub was down) is picked up
        // from where the last hub's loop left its cursor.
        let stopped_at = pulse_bus::last_id(&ts).expect("last id");
        ts.set_meta(WAKE_CURSOR_KEY, &stopped_at.to_string())
            .expect("cursor");
        ts.event(
            "soak.sh",
            WAKE_ACTION,
            &id,
            json!({"text": "soak passed: 0 errors"}),
        )
        .expect("event");
        let ctx = pulse_bus::CancellationToken::new();
        let loop_ctx = ctx.clone();
        let loop_store = ts.store.clone();
        let loop_chats = h.chats.clone();
        let worker = std::thread::spawn(move || wake_loop(&loop_ctx, &loop_store, &loop_chats));
        let shown = wait_last(&h, &id, "The soak passed; closing.");
        let msgs = shown["messages"].as_array().unwrap();
        let note = msgs
            .iter()
            .find(|m| m["role"] == "note" && m["extra"]["wake"].is_object())
            .unwrap_or_else(|| panic!("no wake note in {msgs:?}"));
        assert_eq!(note["extra"]["wake"]["from"], "soak.sh");
        assert!(
            note["content"]
                .as_str()
                .unwrap()
                .contains("soak passed: 0 errors"),
            "{note}"
        );
        for _ in 0..200 {
            if ts
                .meta(WAKE_CURSOR_KEY)
                .ok()
                .is_some_and(|(_, found)| found)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let (cursor, found) = ts.meta(WAKE_CURSOR_KEY).expect("meta");
        assert!(found, "the loop keeps where it got to");
        let cursor: i64 = cursor.parse().expect("cursor is a sequence number");
        assert!(cursor > 0);

        // A second wake while the loop runs: delivered too, the cursor moves.
        ts.event(
            "soak.sh",
            WAKE_ACTION,
            &id,
            json!({"text": "report written"}),
        )
        .expect("event");
        wait_last(&h, &id, "Second wake seen.");
        let (after, _) = ts.meta(WAKE_CURSOR_KEY).expect("meta");
        assert!(after.parse::<i64>().unwrap() > cursor);

        // A wake for a conversation that is not there is logged, not fatal:
        // the loop goes on and stops with the token.
        ts.event("x", WAKE_ACTION, "chat-nowhere", json!({"text": "hello?"}))
            .expect("event");
        std::thread::sleep(Duration::from_millis(50));
        ctx.cancel();
        worker.join().expect("the wake loop ends with the token");
    }

    #[test]
    fn questions_and_approvals_round_trip_over_http() {
        let _env = pulse_testutil::env_lock();
        let ts = additions_store(false);
        let (h, _script) = fake_handler(
            &ts,
            json!([
                {"tool": "question", "args": {"question": "Which colour?", "choices": ["red", "blue"]}},
                {"tool": "write", "args": {"path": "colour.txt", "content": "blue\n"}},
                {"content": "Saved blue."}
            ]),
        );
        let (code, body, _) = call(
            &h,
            "POST",
            "/api/v1/chat",
            json!({"text": "pick a colour", "auto_approve": false}),
        );
        assert_eq!(code, 201);
        let id = body["conversation"]["id"].as_str().unwrap().to_string();
        let shown = wait_status(&h, &id, &["waiting"]);
        assert_eq!(shown["live"]["pending"]["kind"], "question");
        assert_eq!(shown["live"]["pending"]["question"], "Which colour?");
        assert_eq!(shown["live"]["pending"]["choices"][1], "blue");
        let (code, _, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/approve"),
            json!({"decision": "allow"}),
        );
        assert_eq!(code, 409, "no approval waits yet: {raw}");
        let (code, _, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/answer"),
            json!({"text": "blue"}),
        );
        assert_eq!(code, 200, "{raw}");
        // Next stop: the write wants an approval.
        let mut shown = wait_status(&h, &id, &["waiting"]);
        for _ in 0..200 {
            if shown["live"]["pending"]["kind"] == "approval" {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
            shown = wait_status(&h, &id, &["waiting"]);
        }
        assert_eq!(shown["live"]["pending"]["kind"], "approval");
        assert_eq!(shown["live"]["pending"]["tool"], "write");
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/approve"),
            json!({"decision": "allow"}),
        );
        assert_eq!(code, 200);
        let shown = wait_status(&h, &id, &["idle"]);
        let roles: Vec<&str> = shown["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(
            roles,
            ["user", "question", "answer", "approval", "assistant"]
        );
        assert_eq!(shown["messages"][3]["extra"]["decision"], "allow");
        let cwd = shown["conversation"]["cwd"].as_str().unwrap();
        assert_eq!(
            std::fs::read_to_string(std::path::Path::new(cwd).join("colour.txt")).unwrap(),
            "blue\n"
        );
        // Auto-approve flips on and sticks on the row.
        let (code, body, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/auto"),
            json!({"on": true}),
        );
        assert_eq!(code, 200);
        assert_eq!(body["conversation"]["auto_approve"], true);
        assert_eq!(body["conversation"]["live"]["auto_approve"], true);
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/archive"),
            Value::Null,
        );
        assert_eq!(code, 200);
        std::thread::sleep(Duration::from_millis(100));
    }

    #[test]
    fn a_restart_waits_for_working_conversations_and_a_new_hub_recovers_cut_ones() {
        let _env = pulse_testutil::env_lock();
        let _serial = crate::test_helpers::serial();
        let old_poll = {
            let mut poll = crate::register::RESTART_POLL.lock().expect("restart poll");
            let old = *poll;
            *poll = Duration::from_millis(10);
            old
        };
        struct Restore(Duration);
        impl Drop for Restore {
            fn drop(&mut self) {
                *crate::register::RESTART_POLL.lock().expect("restart poll") = self.0;
            }
        }
        let _restore = Restore(old_poll);
        let ts = additions_store(false);
        let (h, _script) = fake_handler(
            &ts,
            json!([{"content": "First."}, {"command": "sleep 2"}, {"content": "Done."}]),
        );
        let ctx = pulse_bus::CancellationToken::new();
        // Nothing works: the drain returns at once with nothing to say.
        assert_eq!(
            crate::register::drain_conversations(&ctx, &h.chats, Duration::from_secs(5)),
            ""
        );
        let (code, body, _) = call(
            &h,
            "POST",
            "/api/v1/chat",
            json!({"text": "take your time"}),
        );
        assert_eq!(code, 201);
        let id = body["conversation"]["id"].as_str().unwrap().to_string();
        wait_status(&h, &id, &["idle"]);
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/message"),
            json!({"text": "go on"}),
        );
        assert_eq!(code, 200);
        wait_status(&h, &id, &["running"]);
        // Capped: the hub goes anyway, and says so.
        let started = std::time::Instant::now();
        let line = crate::register::drain_conversations(&ctx, &h.chats, Duration::from_millis(50));
        assert!(
            line.ends_with("for 1 conversation(s) and stopped them to restart"),
            "{line}"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        // Within the cap: the drain lasts until the turn is over.
        let line = crate::register::drain_conversations(&ctx, &h.chats, Duration::from_secs(20));
        assert!(line.ends_with("for 1 conversation(s) to finish"), "{line}");
        assert!(
            started.elapsed() >= Duration::from_millis(1500),
            "{:?}",
            started.elapsed()
        );
        let shown = wait_status(&h, &id, &["idle"]);
        assert_eq!(shown["messages"].as_array().unwrap().len(), 4);

        // The hub dies with a turn in flight: the row keeps saying running.
        // A handler built over the same store recovers it as it opens.
        ts.store.chat_set_status(&id, "running", "").unwrap();
        let fresh = handler(&ts);
        let (_, cut, carried) = fresh.open_chats().expect("manager");
        assert_eq!(cut, [id.as_str()]);
        assert!(carried.is_empty(), "a cut turn is left to the owner");
        let (code, shown, _) = call(&fresh, "GET", &format!("/api/v1/chat/{id}"), Value::Null);
        assert_eq!(code, 200);
        assert_eq!(shown["live"]["status"], "idle");
        let messages = shown["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 5);
        let note = &messages[4];
        assert_eq!(note["role"], "note");
        assert_eq!(note["extra"]["restart"], true);
        assert_eq!(note["extra"]["session"], shown["session_id"]);
        assert!(
            note["content"]
                .as_str()
                .unwrap()
                .starts_with("PULSE restarted while the agent was working"),
            "{note}"
        );
        let (_, again, _) = fresh.open_chats().expect("manager");
        assert!(again.is_empty(), "the manager is built once");
    }

    #[test]
    fn artifacts_are_listed_and_served_safely() {
        let _env = pulse_testutil::env_lock();
        let ts = additions_store(false);
        let (h, _script) = fake_handler(&ts, json!([{"content": "ok"}]));
        let (code, body, _) = call(&h, "POST", "/api/v1/chat", json!({"text": "make a report"}));
        assert_eq!(code, 201);
        let id = body["conversation"]["id"].as_str().unwrap().to_string();
        wait_status(&h, &id, &["idle"]);
        let chats = h.chats().unwrap();
        let dir = chats.artifacts_dir(&id);
        std::fs::write(dir.join("report.html"), "<h1>hi</h1>").unwrap();
        std::fs::write(dir.join("notes.md"), "# notes\n").unwrap();
        std::fs::write(dir.join("data.bin"), [0u8, 1, 2, 3]).unwrap();
        let (_, shown, _) = call(&h, "GET", &format!("/api/v1/chat/{id}"), Value::Null);
        let names: Vec<&str> = shown["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"report.html"));
        let srv = crate::test_helpers_http::server(h.clone());
        let client = crate::test_helpers::client();
        let resp = client
            .get(format!(
                "{}/api/v1/chat/{id}/artifacts/report.html",
                srv.url
            ))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        assert!(
            resp.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        assert_eq!(
            resp.headers()["content-security-policy"],
            "sandbox allow-scripts"
        );
        // The chat page frames artifacts; the hub's blanket DENY gives way.
        assert_eq!(resp.headers()["x-frame-options"], "SAMEORIGIN");
        assert_eq!(resp.text().unwrap(), "<h1>hi</h1>");
        let resp = client
            .get(format!("{}/api/v1/chat/{id}/artifacts/data.bin", srv.url))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(resp.headers()["content-type"], "application/octet-stream");
        assert_eq!(resp.headers()["x-frame-options"], "SAMEORIGIN");
        assert!(
            !resp.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("frame-ancestors"),
            "a non-page artifact may be framed by the hub page"
        );
        let resp = client
            .get(format!(
                "{}/api/v1/chat/{id}/artifacts/missing.txt",
                srv.url
            ))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 404);
        let resp = client
            .get(format!(
                "{}/api/v1/chat/{id}/artifacts/..%2Fscript.json",
                srv.url
            ))
            .send()
            .unwrap();
        assert_ne!(resp.status().as_u16(), 200, "no path escapes");
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/archive"),
            Value::Null,
        );
        assert_eq!(code, 200);
        std::thread::sleep(Duration::from_millis(100));
    }

    #[test]
    fn attachments_upload_ride_with_a_message_and_the_model_can_change() {
        let _env = pulse_testutil::env_lock();
        let ts = additions_store(false);
        let (h, _script) = fake_handler(&ts, json!([{"content": "ok"}, {"content": "read it"}]));
        let (code, _, raw) = call(
            &h,
            "POST",
            "/api/v1/chat",
            json!({"text": "hello", "model": "no spaces"}),
        );
        assert_eq!(code, 400, "{raw}");
        let (code, body, raw) = call(
            &h,
            "POST",
            "/api/v1/chat",
            json!({"text": "hello", "model": "fake-1"}),
        );
        assert_eq!(code, 201, "{raw}");
        let id = body["conversation"]["id"].as_str().unwrap().to_string();
        assert_eq!(body["conversation"]["model"], "fake-1");
        wait_status(&h, &id, &["idle"]);

        let srv = crate::test_helpers_http::server(h.clone());
        let client = crate::test_helpers::client();
        let upload = |name: &str, bytes: &[u8]| {
            client
                .post(format!("{}/api/v1/chat/{id}/upload?name={name}", srv.url))
                .body(bytes.to_vec())
                .send()
                .unwrap()
        };
        let resp = upload("", b"x");
        assert_eq!(resp.status().as_u16(), 400, "a name is required");
        let resp = upload("notes.txt", b"");
        assert_eq!(resp.status().as_u16(), 413, "an empty body is refused");
        let resp = upload("..%2Fscript.json", b"evil");
        assert_eq!(resp.status().as_u16(), 400, "no path escapes");
        let resp = upload("my%20notes.txt", b"remember the milk");
        assert_eq!(resp.status().as_u16(), 201);
        let saved: Value = resp.json().unwrap();
        assert_eq!(saved["attachment"]["name"], "my notes.txt");
        assert_eq!(saved["attachment"]["size"], 17);
        let resp = upload("my%20notes.txt", b"again");
        assert_eq!(resp.status().as_u16(), 201);
        let again: Value = resp.json().unwrap();
        assert_eq!(again["attachment"]["name"], "my notes-2.txt");
        // A cross-site page cannot push files in.
        let resp = client
            .post(format!("{}/api/v1/chat/{id}/upload?name=x.txt", srv.url))
            .header("Origin", "http://evil.example")
            .body(b"x".to_vec())
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 403);

        // The upload is served back, under its own route only.
        let resp = client
            .get(format!(
                "{}/api/v1/chat/{id}/attachments/my%20notes.txt",
                srv.url
            ))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(resp.text().unwrap(), "remember the milk");
        let resp = client
            .get(format!(
                "{}/api/v1/chat/{id}/artifacts/my%20notes.txt",
                srv.url
            ))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 404);
        let resp = client
            .get(format!(
                "{}/api/v1/chat/{id}/attachments/missing.txt",
                srv.url
            ))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 404);
        let missing = resp.text().unwrap();
        assert!(
            missing.contains("missing.txt is not a file in this session's attachments folder."),
            "{missing}"
        );

        // The message names the uploads; unknown names and bad shapes are refused.
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/message"),
            json!({"text": "read", "attachments": "my notes.txt"}),
        );
        assert_eq!(code, 400);
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/message"),
            json!({"text": "read", "attachments": ["nope.txt"]}),
        );
        assert_eq!(code, 400);
        let (code, _, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/message"),
            json!({"attachments": ["my notes.txt", "my notes-2.txt"]}),
        );
        assert_eq!(code, 200, "{raw}");
        let shown = wait_status(&h, &id, &["idle"]);
        let msgs = shown["messages"].as_array().unwrap();
        let user = msgs.iter().rev().find(|m| m["role"] == "user").unwrap();
        assert_eq!(user["content"], "Look at the attached files.");
        assert_eq!(user["extra"]["attachments"][0]["name"], "my notes.txt");
        assert_eq!(user["extra"]["attachments"][1]["name"], "my notes-2.txt");

        // The model changes for the next turn; a bad id is refused.
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/model"),
            json!({"model": "bad model"}),
        );
        assert_eq!(code, 400);
        let (code, body, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/model"),
            json!({"model": "fake-2"}),
        );
        assert_eq!(code, 200, "{raw}");
        assert_eq!(body["conversation"]["model"], "fake-2");
        // So does the reasoning effort; a word outside the list is refused.
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/reasoning"),
            json!({"reasoning": "extreme"}),
        );
        assert_eq!(code, 400);
        let (code, body, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/reasoning"),
            json!({"reasoning": "high"}),
        );
        assert_eq!(code, 200, "{raw}");
        assert_eq!(body["conversation"]["reasoning"], "high");
        // Lockdown (sandbox) mode flips the same way and shows live.
        assert_eq!(body["conversation"]["lockdown"], false);
        let (code, body, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/lockdown"),
            json!({"on": true}),
        );
        assert_eq!(code, 200, "{raw}");
        assert_eq!(body["conversation"]["lockdown"], true);
        assert_eq!(body["conversation"]["live"]["lockdown"], true);
        let (code, body, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/lockdown"),
            json!({"on": false}),
        );
        assert_eq!(code, 200, "{raw}");
        assert_eq!(body["conversation"]["lockdown"], false);
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/lockdown"),
            json!({"on": "maybe"}),
        );
        assert_eq!(code, 400);
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/archive"),
            Value::Null,
        );
        assert_eq!(code, 200);
        std::thread::sleep(Duration::from_millis(100));
    }

    /// The owner finishes a conversation from the hub: it reads closed with
    /// the owner's line in the transcript, stays in the open list, and the
    /// next message reopens it. The first message may name the effort.
    #[test]
    fn close_finishes_a_conversation_and_a_message_reopens_it() {
        let _env = pulse_testutil::env_lock();
        let ts = additions_store(false);
        let (h, _script) = fake_handler(&ts, json!([{"content": "ok"}, {"content": "again"}]));
        let (code, body, raw) = call(
            &h,
            "POST",
            "/api/v1/chat",
            json!({"text": "hi", "reasoning": "low", "lockdown": true}),
        );
        assert_eq!(code, 201, "{raw}");
        assert_eq!(body["conversation"]["reasoning"], "low");
        assert_eq!(
            body["conversation"]["lockdown"], true,
            "the first message may start the conversation in lockdown"
        );
        assert_eq!(body["conversation"]["live"]["lockdown"], true);
        let id = body["conversation"]["id"].as_str().unwrap().to_string();
        wait_status(&h, &id, &["idle"]);
        let (code, body, raw) = call(&h, "POST", &format!("/api/v1/chat/{id}/close"), Value::Null);
        assert_eq!(code, 200, "{raw}");
        assert_eq!(body["conversation"]["status"], "closed");
        assert_eq!(body["conversation"]["live"]["status"], "closed");
        let shown = wait_status(&h, &id, &["closed"]);
        let msgs = shown["messages"].as_array().unwrap();
        let last = msgs.last().unwrap();
        assert_eq!(last["role"], "note");
        assert_eq!(last["extra"]["closed"], "owner");
        let (code, body, _) = call(&h, "GET", "/api/v1/chat", Value::Null);
        assert_eq!(code, 200);
        assert_eq!(body["conversations"].as_array().unwrap().len(), 1);
        // Closing again changes nothing; a message reopens.
        let (code, _, _) = call(&h, "POST", &format!("/api/v1/chat/{id}/close"), Value::Null);
        assert_eq!(code, 200);
        let (code, _, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/message"),
            json!({"text": "one more"}),
        );
        assert_eq!(code, 200, "{raw}");
        let shown = wait_status(&h, &id, &["idle"]);
        assert_eq!(shown["conversation"]["status"], "idle");
        let msgs = shown["messages"].as_array().unwrap();
        assert_eq!(msgs.last().unwrap()["content"], "again");
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/archive"),
            Value::Null,
        );
        assert_eq!(code, 200);
        std::thread::sleep(Duration::from_millis(100));
    }

    /// `POST /chat/ID/restart` ends the session in memory and the next
    /// message reloads the conversation from its saved transcript, which
    /// the harness repairs on the way in: a transcript carrying a tool
    /// call whose arguments the gateway cut mid-stream (what used to stick
    /// a conversation on `Invalid JSON format in tool call arguments`)
    /// goes back to the provider with `{}` for it, and the turn succeeds.
    #[test]
    fn restart_reloads_a_conversation_from_its_transcript_repaired() {
        let _env = pulse_testutil::env_lock();
        let ts = additions_store(false);
        let (h, _script) = fake_handler(&ts, json!([{"content": "ok"}, {"content": "again"}]));
        let (code, body, raw) = call(&h, "POST", "/api/v1/chat", json!({"text": "hi"}));
        assert_eq!(code, 201, "{raw}");
        let id = body["conversation"]["id"].as_str().unwrap().to_string();
        let shown = wait_status(&h, &id, &["idle"]);
        let session = shown["session_id"].as_str().unwrap().to_string();
        assert!(!session.is_empty());
        // Let the worker stand down before the restart.
        let (code, body, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/restart"),
            Value::Null,
        );
        assert_eq!(code, 200, "{raw}");
        assert_eq!(body["conversation"]["status"], "idle");
        assert_eq!(body["conversation"]["live"]["status"], "idle");
        assert_eq!(
            body["conversation"]["session_id"], session,
            "the row keeps the session whose transcript the next message loads"
        );
        let shown = wait_status(&h, &id, &["idle"]);
        let msgs = shown["messages"].as_array().unwrap();
        let note = msgs.last().unwrap();
        assert_eq!(note["role"], "note");
        assert_eq!(note["extra"]["restarted"], "owner");
        assert_eq!(note["extra"]["repaired"], 0);
        assert!(
            note["content"]
                .as_str()
                .unwrap()
                .starts_with("The session was restarted from the transcript."),
            "{note}"
        );
        // The session's memory is what its trajectory holds; the stuck
        // conversation's last message was a cut `edit` call with the
        // result the old harness gave it.
        let path = std::path::Path::new(&ts.home)
            .join("harness")
            .join(format!("{session}.json"));
        let mut traj: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let cut = r#"{"path": "AGENTS.md", "replace": "  with the GitHub noreply address.\n\n## Rollback Planning""#;
        let messages = traj["messages"].as_array_mut().unwrap();
        messages.push(json!({"role": "assistant", "tool_calls": [{"id": "toolu_01D9XWUQtuVBTWixzvum3zYd", "name": "edit", "arguments": cut}]}));
        messages.push(json!({"role": "tool", "content": "edit: arguments are not JSON: EOF while parsing an object at line 1 column 93", "tool_call_id": "toolu_01D9XWUQtuVBTWixzvum3zYd", "name": "edit"}));
        std::fs::write(&path, serde_json::to_vec(&traj).unwrap()).unwrap();
        // Restarting with nothing open in memory counts what the load
        // will repair.
        let (code, _, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/restart"),
            Value::Null,
        );
        assert_eq!(code, 200, "{raw}");
        let shown = wait_status(&h, &id, &["idle"]);
        let note = shown["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert_eq!(note["extra"]["repaired"], 1);
        assert!(
            note["content"]
                .as_str()
                .unwrap()
                .ends_with("; it repairs 1 tool call whose arguments are not JSON."),
            "{note}"
        );
        let (code, _, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/message"),
            json!({"text": "land it"}),
        );
        assert_eq!(code, 200, "{raw}");
        let shown = wait_status(&h, &id, &["idle"]);
        let new_session = shown["session_id"].as_str().unwrap().to_string();
        assert_ne!(new_session, session, "a new session was opened");
        let msgs = shown["messages"].as_array().unwrap();
        assert_eq!(msgs.last().unwrap()["role"], "assistant");
        assert_eq!(
            msgs.last().unwrap()["content"],
            "ok",
            "the turn succeeded (the fake's script starts over in the new session)"
        );
        let steps = shown["steps"].as_array().unwrap();
        assert!(
            steps.iter().any(|st| st["kind"] == "note"
                && st["content"].as_str().unwrap_or("").starts_with(
                    "repaired 1 carried-over tool call whose arguments were not JSON (edit toolu_01D9XWUQtuVBTWixzvum3zYd)"
                )),
            "{steps:?}"
        );
        // What the provider got is what the new session's trajectory holds:
        // the cut call with `{}`, nothing that is not JSON.
        let traj: Value = serde_json::from_slice(
            &std::fs::read(
                std::path::Path::new(&ts.home)
                    .join("harness")
                    .join(format!("{new_session}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        let mut seen_cut = false;
        for m in traj["messages"].as_array().unwrap() {
            for c in m["tool_calls"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                let args = c["arguments"].as_str().unwrap_or("");
                assert!(
                    serde_json::from_str::<Value>(args).is_ok(),
                    "arguments sent back are JSON: {c}"
                );
                if c["id"] == "toolu_01D9XWUQtuVBTWixzvum3zYd" {
                    seen_cut = true;
                    assert_eq!(args, "{}");
                }
            }
        }
        assert!(seen_cut, "the cut call was carried over");
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/archive"),
            Value::Null,
        );
        assert_eq!(code, 200);
        let (code, _, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/restart"),
            Value::Null,
        );
        assert_eq!(
            code, 409,
            "an archived conversation has nothing to restart: {raw}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    /// `POST /chat` with `parent` starts a sub-conversation in the parent's
    /// workspace; the parent's page lists it under `children` and the
    /// child's names its `parent_title`; the child's reply reaches the
    /// parent as a report note; the list carries `parent` on every row.
    #[test]
    fn a_sub_conversation_is_started_shown_and_reports_over_http() {
        let _env = pulse_testutil::env_lock();
        let ts = additions_store(false);
        let (h, _script) = fake_handler(
            &ts,
            json!([{"content": "First."}, {"content": "Second."}, {"content": "Third."}]),
        );
        let (code, body, raw) = call(
            &h,
            "POST",
            "/api/v1/chat",
            json!({"text": "plan the release", "title": "Release"}),
        );
        assert_eq!(code, 201, "{raw}");
        let pid = body["conversation"]["id"].as_str().unwrap().to_string();
        assert_eq!(body["conversation"]["parent"], "");
        let parent = wait_status(&h, &pid, &["idle"]);
        let cwd = parent["conversation"]["cwd"].as_str().unwrap().to_string();
        assert!(!cwd.is_empty());
        let (code, body, raw) = call(
            &h,
            "POST",
            "/api/v1/chat",
            json!({"text": "check the changelog", "title": "Changelog", "parent": pid}),
        );
        assert_eq!(code, 201, "{raw}");
        let cid = body["conversation"]["id"].as_str().unwrap().to_string();
        assert_eq!(body["conversation"]["parent"], pid);
        assert_eq!(body["conversation"]["cwd"], cwd);
        let (code, _, raw) = call(
            &h,
            "POST",
            "/api/v1/chat",
            json!({"text": "x", "parent": "chat-missing"}),
        );
        assert_eq!(code, 409, "{raw}");
        assert!(raw.contains("no such conversation"), "{raw}");
        let child = wait_status(&h, &cid, &["idle"]);
        assert_eq!(child["parent_title"], "Release");
        assert_eq!(child["children"].as_array().unwrap().len(), 0);
        // The child's reply is a report in the parent, which ran on it.
        let mut parent = JsonMap::new();
        for _ in 0..400 {
            parent = wait_status(&h, &pid, &["idle"]);
            let got = parent["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["extra"]["report"]["child"] == cid);
            if got {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let msgs = parent["messages"].as_array().unwrap();
        let report = msgs
            .iter()
            .find(|m| m["extra"]["report"]["child"] == cid)
            .unwrap_or_else(|| panic!("no report: {msgs:#?}"));
        assert_eq!(report["role"], "note");
        assert_eq!(report["content"], "First.");
        assert_eq!(report["extra"]["report"]["kind"], "replied");
        assert_eq!(report["extra"]["report"]["title"], "Changelog");
        assert!(
            msgs.iter()
                .any(|m| m["extra"]["child"] == cid && m["role"] == "note"),
            "the parent notes the start: {msgs:#?}"
        );
        let children = parent["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["id"], cid);
        assert_eq!(children[0]["title"], "Changelog");
        assert!(children[0]["live"]["status"].is_string());
        assert_eq!(parent["parent_title"], "");
        let (code, body, _) = call(&h, "GET", "/api/v1/chat", Value::Null);
        assert_eq!(code, 200);
        let rows = body["conversations"].as_array().unwrap();
        let row = rows.iter().find(|r| r["id"] == cid).unwrap();
        assert_eq!(row["parent"], pid);
        std::thread::sleep(Duration::from_millis(100));
    }

    #[test]
    fn images_post_stores_the_bytes_and_the_image_route_serves_them() {
        let _env = pulse_testutil::env_lock();
        let ts = additions_store(false);
        let (h, _script) = fake_handler(&ts, json!([{"content": "ok"}, {"content": "saw it"}]));
        let (code, body, raw) = call(&h, "POST", "/api/v1/chat", json!({"text": "hi"}));
        assert_eq!(code, 201, "{raw}");
        let id = body["conversation"]["id"].as_str().unwrap().to_string();
        wait_status(&h, &id, &["idle"]);
        let png = b"\x89PNG fake bytes";
        let b64 = STANDARD.encode(png);
        let (code, _, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/message"),
            json!({"text": "see", "images": [{"mime": "image/png", "data": b64}]}),
        );
        assert_eq!(code, 200, "{raw}");
        let shown = wait_status(&h, &id, &["idle"]);
        let msgs = shown["messages"].as_array().unwrap();
        let user = msgs.iter().rev().find(|m| m["role"] == "user").unwrap();
        assert_eq!(user["content"], "see");
        assert_eq!(user["extra"]["images"][0]["name"], "image-1.png");
        assert_eq!(user["extra"]["images"][0]["mime"], "image/png");
        let n = user["n"].as_i64().unwrap();

        // The image route serves the stored bytes with the stored mime.
        let srv = crate::test_helpers_http::server(h.clone());
        let client = crate::test_helpers::client();
        let resp = client
            .get(format!("{}/api/v1/chat/{id}/image/{n}/0", srv.url))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(resp.headers()["content-type"], "image/png");
        let got = resp.bytes().unwrap();
        assert_eq!(got.as_ref(), png);
        for (path, want) in [
            (format!("/api/v1/chat/{id}/image/{n}/1"), 404),
            (format!("/api/v1/chat/{id}/image/9999/0"), 404),
            (format!("/api/v1/chat/{id}/image/x/0"), 404),
            // a missing conversation answers like the rest of the chat
            // surface: 409 with "no such conversation".
            ("/api/v1/chat/missing/image/1/0".to_string(), 409),
        ] {
            let resp = client.get(format!("{}{path}", srv.url)).send().unwrap();
            assert_eq!(resp.status().as_u16(), want, "{path}");
        }

        // Bad shapes are refused before anything is stored.
        for body in [
            json!({"text": "x", "images": "one.png"}),
            json!({"text": "x", "images": [{"mime": "image/gif", "data": b64}]}),
            json!({"text": "x", "images": [{"mime": "image/png", "data": "nope!!!"}]}),
            json!({"text": "x", "images": [{"mime": "image/png", "data": STANDARD.encode(vec![0u8; pulse_harness::conversation::IMAGE_LIMIT + 1])}]}),
        ] {
            let (code, _, _) = call(&h, "POST", &format!("/api/v1/chat/{id}/message"), body);
            assert_eq!(code, 400);
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    #[test]
    fn a_project_conversation_reports_its_branch_and_attach_validates() {
        let _env = pulse_testutil::env_lock();
        let _nosys = pulse_testutil::setenv("GIT_CONFIG_NOSYSTEM", "1");
        let _noglobal = pulse_testutil::setenv("GIT_CONFIG_GLOBAL", "/dev/null");
        let ts = additions_store(false);
        additions_project(&ts, "demo");
        let path = std::path::Path::new(&ts.home).join("demo");
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
        ] {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(&args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        }
        std::fs::write(path.join("README.md"), "hello\n").unwrap();
        for args in [vec!["add", "."], vec!["commit", "-q", "-m", "first"]] {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(&args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        }
        ts.conn()
            .unwrap()
            .execute(
                "UPDATE projects SET default_branch = 'main' WHERE slug = 'demo'",
                [],
            )
            .unwrap();
        let (h, _script) = fake_handler(
            &ts,
            json!([
                {"command": "printf 'more\\n' >> README.md && git add README.md && git -c user.email=a@b.c -c user.name=a commit -q -m 'chat change' && echo done"},
                {"content": "Committed."}
            ]),
        );
        let (code, _, raw) = call(
            &h,
            "POST",
            "/api/v1/chat",
            json!({"text": "x", "project": "nope"}),
        );
        assert_eq!(code, 409, "unknown project: {raw}");
        let (code, body, raw) = call(
            &h,
            "POST",
            "/api/v1/chat",
            json!({"text": "add a line", "project": "demo"}),
        );
        assert_eq!(code, 201, "{raw}");
        let id = body["conversation"]["id"].as_str().unwrap().to_string();
        let shown = wait_status(&h, &id, &["idle"]);
        assert_eq!(shown["conversation"]["project"], "demo");
        assert_eq!(shown["conversation"]["branch"], format!("pulse/chat-{id}"));
        assert_eq!(shown["changes"]["commits"], 1, "{}", shown["changes"]);
        assert!(
            shown["changes"]["stat"]
                .as_str()
                .unwrap()
                .contains("README.md")
        );
        // A commit main lacks: the conversation waits to land, on its page
        // and in the list alike.
        assert_eq!(
            shown["conversation"]["waiting_to_land"], true,
            "{}",
            shown["conversation"]
        );
        let (code, list, raw) = call(&h, "GET", "/api/v1/chat", Value::Null);
        assert_eq!(code, 200, "{raw}");
        let listed = list["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id.as_str())
            .expect("listed");
        assert_eq!(listed["waiting_to_land"], true, "{raw}");
        assert!(listed["pr"].is_null(), "no pull request yet: {raw}");
        // The daemon proposes the branch as a pull request and the reviewer
        // hands it to the owner: the page reads both from `pr`, found by
        // the conversation's id or its branch.
        pulse_bus::emit(
            &ts.store,
            "daemon",
            "land.pr",
            "demo",
            format!(
                r#"{{"project":"demo","kind":"proposed","conversation":"{id}","number":4,"url":"https://github.com/o/r/pull/4","head":"pulse/chat-{id}","base":"main","title":"Chat: add a line","opened":true}}"#
            ),
        )
        .unwrap();
        pulse_bus::emit(
            &ts.store,
            "daemon",
            "review.owner",
            "demo",
            r#"{"project":"demo","number":4,"url":"https://github.com/o/r/pull/4","reasons":["no tests"],"note":"the change has no test"}"#,
        )
        .unwrap();
        let (code, list, raw) = call(&h, "GET", "/api/v1/chat", Value::Null);
        assert_eq!(code, 200, "{raw}");
        let listed = list["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id.as_str())
            .expect("listed");
        assert_eq!(listed["pr"]["number"], 4, "{raw}");
        assert_eq!(listed["pr"]["state"], "needs_owner");
        assert_eq!(listed["pr"]["reasons"][0], "no tests");
        assert_eq!(listed["pr"]["can_merge"], true);
        let (code, shown, raw) = call(&h, "GET", &format!("/api/v1/chat/{id}"), Value::Null);
        assert_eq!(code, 200, "{raw}");
        assert_eq!(
            shown["conversation"]["pr"]["url"],
            "https://github.com/o/r/pull/4"
        );
        assert_eq!(
            shown["conversation"]["pr"]["note"],
            "the change has no test"
        );
        let (code, diff, _) = call(&h, "GET", &format!("/api/v1/chat/{id}/diff"), Value::Null);
        assert_eq!(code, 200);
        assert!(diff["diff"].as_str().unwrap().contains("+more"));
        let (code, _, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/attach"),
            json!({"project": "nope"}),
        );
        assert_eq!(code, 409, "{raw}");
        let (code, body, raw) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/attach"),
            json!({"project": "demo"}),
        );
        assert_eq!(code, 200, "{raw}");
        assert_eq!(
            body["attached"]["created"], false,
            "the worktree already exists"
        );
        let (code, _, _) = call(
            &h,
            "POST",
            &format!("/api/v1/chat/{id}/archive"),
            Value::Null,
        );
        assert_eq!(code, 200);
        let (code, _, raw) = call(&h, "GET", "/api/v1/chat/nope", Value::Null);
        assert_eq!(code, 404);
        assert!(
            raw.contains("No such session") && raw.contains("There is no session nope."),
            "the hub names a session the way its Sessions page does: {raw}"
        );
        let (code, _, _) = call(&h, "POST", "/api/v1/chat", json!({"text": ""}));
        assert_eq!(code, 400);
        std::thread::sleep(Duration::from_millis(100));
    }

    // The list carries the tasks the page groups conversations under: each
    // with its thread with the lead summed up and one session per step the
    // lead planned - a queued step with no agent yet, a running one with
    // its harness session. A task finished long ago is listed only with
    // `all=1`; the newest change sorts first.
    #[test]
    fn the_list_groups_a_tasks_sessions_and_keeps_old_finished_tasks_for_all() {
        let _env = pulse_testutil::env_lock();
        let ts = additions_store(false);
        additions_project(&ts, "one");
        let h = handler(&ts);
        let task = pulse_tasks::new(&ts, "one", "Ship the spec", "the brief", 10, "human").unwrap();
        let old = pulse_tasks::new(&ts, "one", "Long done", "the brief", 10, "human").unwrap();
        let conn = ts.conn().unwrap();
        conn.execute(
            "UPDATE tasks SET status='in_progress', lead='one-lead' WHERE id=?",
            [&task.id],
        )
        .unwrap();
        conn.execute(
            "UPDATE tasks SET status='done', updated_at='2026-01-01T00:00:00Z' WHERE id=?",
            [&old.id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, project, task_id, author, body, at) VALUES ('MSG-1', 'one', ?, 'one-lead', 'Planned 2 steps.', '2026-10-31T10:00:00Z')",
            [&task.id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO jobs (id, project, kind, title, source, status, lease_agent, created_at, branch) VALUES \
             ('job-a', 'one', 'task', 'Write the page', ?1, 'leased', 'iris', '2026-10-31T10:01:00Z', ''), \
             ('job-b', 'one', 'task', 'Add the test', ?1 || ':ui', 'queued', '', '2026-10-31T10:02:00Z', ''), \
             ('job-c', 'one', 'task', 'Wire it up', ?1 || ':api', 'done', 'iris', '2026-10-31T10:03:00Z', 'pulse/iris/job-c')",
            [format!("task:{}", task.id)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO harness_sessions (id, job_id, agent, project, status, steps, cost, started_at, updated_at) VALUES \
             ('run-a', 'job-a', 'iris', 'one', 'running', 12, 0.25, '2026-10-31T10:05:00Z', '2026-10-31T10:09:00Z')",
            [],
        )
        .unwrap();
        drop(conn);

        let (code, body, raw) = call(&h, "GET", "/api/v1/chat", Value::Null);
        assert_eq!(code, 200, "{raw}");
        let tasks = body["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1, "the old finished task stays out: {raw}");
        let g = &tasks[0];
        assert_eq!(g["id"], task.id);
        assert_eq!(g["lead"], "one-lead");
        assert_eq!(g["messages"], 1);
        assert_eq!(g["last_author"], "one-lead");
        assert_eq!(g["needs_you"], false);
        assert_eq!(
            g["last_at"], "2026-10-31T10:09:00Z",
            "the newest change of any session"
        );
        let sessions = g["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions[0]["job"], "job-a");
        assert_eq!(sessions[0]["session"], "run-a");
        assert_eq!(sessions[0]["agent"], "iris");
        assert_eq!(sessions[0]["status"], "running");
        assert_eq!(sessions[0]["job_status"], "leased");
        assert_eq!(sessions[0]["steps"], 12);
        assert_eq!(sessions[1]["job"], "job-b");
        assert_eq!(sessions[1]["title"], "Add the test");
        assert_eq!(sessions[1]["session"], "");
        assert_eq!(sessions[1]["job_status"], "queued");
        assert_eq!(sessions[1]["branch"], "");
        // A done step keeps its branch until the landing clears it: the
        // page lists the step, and the task, as waiting to land.
        assert_eq!(sessions[2]["job"], "job-c");
        assert_eq!(sessions[2]["job_status"], "done");
        assert_eq!(sessions[2]["branch"], "pulse/iris/job-c");
        assert!(g["pr"].is_null(), "no pull request yet: {raw}");

        // The task's pull request, once opened, comes with the group.
        pulse_bus::emit(
            &ts.store,
            "daemon",
            "land.pr",
            "one",
            format!(
                r#"{{"project":"one","kind":"task","task":"{}","number":5,"url":"https://github.com/o/r/pull/5","head":"pulse/task/{}","base":"main","opened":true}}"#,
                task.id, task.id
            ),
        )
        .unwrap();
        pulse_bus::emit(
            &ts.store,
            "daemon",
            "review.queued",
            "one",
            r#"{"project":"one","number":5,"url":"https://github.com/o/r/pull/5","job":"JOB-rev","kind":"task","reasons":[]}"#,
        )
        .unwrap();
        let (code, body, raw) = call(&h, "GET", "/api/v1/chat", Value::Null);
        assert_eq!(code, 200, "{raw}");
        let g = &body["tasks"].as_array().unwrap()[0];
        assert_eq!(g["pr"]["number"], 5, "{raw}");
        assert_eq!(g["pr"]["state"], "in_review");

        let (code, body, raw) = call(&h, "GET", "/api/v1/chat?all=1", Value::Null);
        assert_eq!(code, 200, "{raw}");
        let ids: Vec<&str> = body["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec![task.id.as_str(), old.id.as_str()]);
    }
}
