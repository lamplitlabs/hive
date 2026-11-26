use anyhow::{Result, anyhow};
use regex::Regex;
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::sync::{LazyLock, Mutex};

pub(crate) type JsonMap = BTreeMap<String, Value>;

/// The row a lookup falls back to when a post id is unknown: every key is null.
static EMPTY_ROW: PostRow = PostRow([const { Value::Null }; POST_ROW_LEN]);

fn post_at<'a>(
    by_id: &BTreeMap<String, usize>,
    posts: &'a [PostRow],
    id: &str,
) -> Option<&'a PostRow> {
    by_id.get(id).map(|&i| &posts[i])
}

/// A row read by key: a `JsonMap`, a `PostRow`, or a reference to either.
pub(crate) trait Fields {
    fn field(&self, key: &str) -> Option<&Value>;
}

impl Fields for JsonMap {
    fn field(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

impl Fields for PostRow {
    fn field(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

impl<T: Fields + ?Sized> Fields for &T {
    fn field(&self, key: &str) -> Option<&Value> {
        (**self).field(key)
    }
}

/// `POST_ROW_FIELDS` in bytewise order - the order a `BTreeMap` row
/// serialized its keys in, so a row's JSON is unchanged.
const POST_ROW_KEYS: [&str; 27] = [
    "agent",
    "area",
    "branch",
    "date",
    "decider",
    "expires",
    "files",
    "has_read_line",
    "id",
    "in_area",
    "kind",
    "links",
    "model",
    "name",
    "on_post",
    "project",
    "provenance_missing",
    "provenance_ok",
    "rel_path",
    "resolution",
    "runner",
    "session",
    "summary",
    "text_hash",
    "thread",
    "thread_key",
    "vote",
];
const POST_ROW_LEN: usize = POST_ROW_KEYS.len();

/// One cached post: its `POST_ROW_FIELDS` values in `POST_ROW_KEYS` order,
/// in one array rather than a map. A `JsonMap` row cost four tree nodes and
/// 26 owned key strings on top of the values - some 3 KB per post - and a
/// hub serving a 60k-post board held almost 200 MB of that overhead alone.
#[derive(Debug, Clone)]
pub(crate) struct PostRow([Value; POST_ROW_LEN]);

impl PostRow {
    /// A row from values in `POST_ROW_FIELDS` order.
    fn new(values: Vec<Value>) -> PostRow {
        let mut slots = [const { Value::Null }; POST_ROW_LEN];
        for (key, value) in POST_ROW_FIELDS.split_whitespace().zip(values) {
            if let Ok(i) = POST_ROW_KEYS.binary_search(&key) {
                slots[i] = value;
            }
        }
        PostRow(slots)
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        POST_ROW_KEYS.binary_search(&key).ok().map(|i| &self.0[i])
    }

    #[cfg(test)]
    pub(crate) fn keys(&self) -> impl Iterator<Item = &'static str> {
        POST_ROW_KEYS.into_iter()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&'static str, &Value)> {
        POST_ROW_KEYS.into_iter().zip(self.0.iter())
    }

    /// The row as a map, for a response that adds keys to it.
    pub(crate) fn to_map(&self) -> JsonMap {
        self.iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect()
    }

    /// A row from a map: the `POST_ROW_FIELDS` it carries, null for the rest.
    #[cfg(test)]
    pub(crate) fn from_map(map: &JsonMap) -> PostRow {
        let mut slots = [const { Value::Null }; POST_ROW_LEN];
        for (i, key) in POST_ROW_KEYS.iter().enumerate() {
            if let Some(value) = map.get(*key) {
                slots[i] = value.clone();
            }
        }
        PostRow(slots)
    }
}

impl Serialize for PostRow {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(POST_ROW_LEN))?;
        for (key, value) in self.iter() {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

pub trait TallySource: Send + Sync {
    fn records(&self, project: &str, area: &str) -> Result<String>;

    /// The tally the records were rendered from, when the source has one:
    /// lets a snapshot reuse its posts and duplicates instead of loading
    /// and tallying the project again. `None` falls back to `records`.
    fn tally(&self, _project: &str, _area: &str) -> Result<Option<hive_tally::TallyResult>> {
        Ok(None)
    }
}

pub struct NativeTally {
    pub store: std::sync::Arc<hive_store::Store>,
}

impl TallySource for NativeTally {
    fn records(&self, project: &str, area: &str) -> Result<String> {
        hive_tally::records(
            &self.store,
            project,
            area,
            hive_tally::Thresholds::default(),
        )
    }

    fn tally(&self, project: &str, area: &str) -> Result<Option<hive_tally::TallyResult>> {
        Ok(Some(hive_tally::compute(
            &self.store,
            project,
            area,
            hive_tally::Thresholds::default(),
        )?))
    }
}

/// The fields of a cached post row. `frontmatter` and `body` stay in the
/// store and `Handler::post_detail` reads them for the one post a request
/// asks for, so a snapshot holds a few kilobytes per post rather than the
/// whole board text (see `snapshot`).
pub(crate) const POST_ROW_FIELDS: &str = "id rel_path area thread thread_key kind decider agent runner model session date branch expires on_post vote text_hash has_read_line resolution summary links files provenance_ok provenance_missing in_area name project";

#[derive(Debug, Clone, Default)]
pub(crate) struct BoardData {
    pub(crate) meta: JsonMap,
    pub(crate) params: JsonMap,
    /// One row per post with the `POST_ROW_FIELDS` keys; no body, no
    /// frontmatter. Everything the board derives from a post's body
    /// (proposal text, read lines, owner questions) is computed while the
    /// snapshot is built, from the store rows the tally was computed from.
    pub(crate) posts: Vec<PostRow>,
    /// Index of each post id into `posts`.
    pub(crate) by_id: BTreeMap<String, usize>,
    pub(crate) claims: Vec<JsonMap>,
    pub(crate) threads: Vec<JsonMap>,
    pub(crate) proposals: Vec<JsonMap>,
    /// Open questions addressed to the owner (`for_owner`, and the evolution
    /// loop's stop questions by construction): what waits for a yes or no
    /// that no worker may give.
    pub(crate) questions: Vec<JsonMap>,
    /// Quiet note beside the owner questions: how many open ones the agents
    /// are settling themselves (lead while the owner is away, or a setting),
    /// so the For you count is only what the owner alone can decide.
    pub(crate) questions_meta: JsonMap,
    pub(crate) suggestions: Vec<JsonMap>,
    pub(crate) amendments: Vec<JsonMap>,
    pub(crate) evolution: JsonMap,
}

impl crate::server::Handler {
    pub(crate) fn entry<S: AsRef<str>>(&self, slug: S) -> Result<hive_store::Project> {
        let slug = slug.as_ref();
        let store = self
            .store
            .as_deref()
            .ok_or_else(|| anyhow!("the store is unavailable"))?;
        if slug == hive_store::COMMONS_SLUG {
            let (mut value, _) = store.meta("commons_path")?;
            if value.is_empty() {
                value = Path::new(&store.home)
                    .join("commons")
                    .to_string_lossy()
                    .to_string();
            }
            return Ok(hive_store::Project {
                slug: slug.to_string(),
                name: slug.to_string(),
                path: value,
                status: "active".to_string(),
                ..Default::default()
            });
        }
        let Some(p) = store.project(slug)? else {
            return Err(hive_tally::NoProjectError { slug: slug.into() }.into());
        };
        if p.status != "active" {
            return Err(hive_tally::NoProjectError { slug: slug.into() }.into());
        }
        Ok(p)
    }

    pub(crate) fn snapshot(&self, project: &str, area: &str) -> Result<BoardData> {
        let store = self
            .store
            .as_deref()
            .ok_or_else(|| anyhow!("the store is unavailable"))?;
        let p = self.entry(project)?;
        // One tally serves the records, the post rows and the plan context;
        // the records-only path (test sources) loads posts a second time.
        let mut tally = match &self.source {
            Some(source) => source.tally(project, area)?,
            None => Some(hive_tally::compute(
                store,
                project,
                area,
                hive_tally::Thresholds::default(),
            )?),
        };
        let text = match (&tally, &self.source) {
            (Some(res), _) => res.records(),
            (None, Some(source)) => source.records(project, area)?,
            (None, None) => {
                hive_tally::records(store, project, area, hive_tally::Thresholds::default())?
            }
        };
        let raw_posts = match tally.as_mut() {
            Some(res) if area.is_empty() => std::mem::take(&mut res.posts),
            _ => hive_tally::posts_for(store, project, "")?,
        };
        let open_jobs = hive_tally::open_suggestion_jobs(store)?;
        let db_posts = raw_posts
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect::<BTreeMap<_, _>>();
        let mut d = BoardData {
            params: JsonMap::new(),
            by_id: BTreeMap::new(),
            // One row per store post: sized once so the vector never holds
            // the slack of a doubling.
            posts: Vec::with_capacity(db_posts.len()),
            claims: Vec::new(),
            threads: Vec::new(),
            proposals: Vec::new(),
            suggestions: Vec::new(),
            amendments: Vec::new(),
            ..Default::default()
        };
        let mut params = BTreeMap::<String, String>::new();
        let mut svotes = BTreeMap::<String, Vec<JsonMap>>::new();
        let mut avotes = BTreeMap::<String, Vec<JsonMap>>::new();
        let mut nokind = "0".to_string();
        for line in text.trim_end_matches('\n').split('\n') {
            if line.is_empty() {
                continue;
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.is_empty() {
                continue;
            }
            let kind = fields[0];
            let mut f = fields[1..].to_vec();
            while f.len() < 17 {
                f.push("");
            }
            match kind {
                "param" => {
                    params.insert(f[0].to_string(), f[1].to_string());
                }
                "nokind" => nokind = f[0].to_string(),
                "post" => {
                    let post = db_posts
                        .get(f[3])
                        .ok_or_else(|| anyhow!("tally references missing post {:?}", f[3]))?;
                    // Provenance: which identity fields the post left blank.
                    let mut missing = Vec::new();
                    for (key, value) in [
                        ("agent", &post.agent),
                        ("runner", &post.runner),
                        ("model", &post.model),
                        ("session", &post.session),
                        ("date", &post.date),
                        ("branch", &post.branch),
                    ] {
                        if value.is_empty() {
                            missing.push(key.to_string());
                        }
                    }
                    let row = PostRow::new(
                        vec![
                            json!(f[3]), json!(f[0]), json!(f[1]), json!(f[2]), json!(format!("{}/{}", f[1], f[2])),
                            json!(f[4]), json!(""), json!(f[5]), json!(post.runner), json!(post.model), json!(f[12]),
                            json!(f[6]), json!(post.branch), json!(f[7]), json!(f[8]), json!(f[9]), json!(f[13]),
                            json!(if f[14] == "1" { 1 } else { 0 }), json!(f[10]), json!(f[11]), json!(post.links),
                            json!(post.files), json!(if missing.is_empty() { 1 } else { 0 }),
                            json!(missing), json!(if area.is_empty() || area == f[1] { 1 } else { 0 }),
                            json!(truncate(&post.name, 40)), json!(truncate(&post.origin_project, 60)),
                        ],
                    );
                    d.by_id.insert(f[3].to_string(), d.posts.len());
                    d.posts.push(row);
                }
                "claim" => d.claims.push(object("id thread_key agent expires status summary", vals(&f[0..6]))),
                "thread" => {
                    let (area, thread) = f[0].split_once('/').unwrap_or((f[0], ""));
                    d.threads.push(object(
                        "key area thread posts last status resolution summary",
                        vec![json!(f[0]), json!(area), json!(thread), json!(num_str(f[1])), json!(f[2]), json!(f[3]), json!(f[4]), json!(f[5])],
                    ));
                }
                "proposal" => d.proposals.push(object("thread_key id agent date status decided_id decided_date summary", vals(&f[0..8]))),
                "suggestion" => d.suggestions.push(object(
                    "score date id thread_key up down voters status agent summary",
                    vec![json!(num_str(f[0])), json!(f[1]), json!(f[2]), json!(f[3]), json!(num_str(f[4])), json!(num_str(f[5])), json!(num_str(f[6])), json!(f[7]), json!(f[8]), json!(f[9])],
                )),
                "svote" => svotes.entry(f[0].to_string()).or_default().push(object("agent post_id date vote classification", vals(&f[1..6]))),
                "amendment" => d.amendments.push(object(
                    "date id thread_key status up down voters quorum pct_up identities uncounted_self uncounted_hash_mismatch uncounted_no_read_line text_hash section agent summary",
                    vec![json!(f[0]), json!(f[1]), json!(f[2]), json!(f[3]), json!(num_str(f[4])), json!(num_str(f[5])), json!(num_str(f[6])), json!(num_str(f[7])), json!(num_str(f[8])), json!(num_str(f[9])), json!(num_str(f[10])), json!(num_str(f[11])), json!(num_str(f[12])), json!(f[13]), json!(f[14]), json!(f[15]), json!(f[16])],
                )),
                "avote" => avotes.entry(f[0].to_string()).or_default().push(object("session agent post_id date vote classification", vals(&f[1..7]))),
                _ => return Err(anyhow!("unknown tally record kind {:?}", kind)),
            }
        }
        d.params = object(
            "area now quorum expiry_days stale_days amend_quorum amend_majority protected_sections constitution_present stop_present",
            vec![
                json!(params.get("area").cloned().unwrap_or_default()),
                json!(params.get("now").cloned().unwrap_or_default()),
                json!(num_str(
                    params.get("quorum").map(String::as_str).unwrap_or("")
                )),
                json!(num_str(
                    params.get("expiry_days").map(String::as_str).unwrap_or("")
                )),
                json!(num_str(
                    params.get("stale_days").map(String::as_str).unwrap_or("")
                )),
                json!(num_str(
                    params.get("amend_quorum").map(String::as_str).unwrap_or("")
                )),
                json!(params.get("amend_majority").cloned().unwrap_or_default()),
                json!(params.get("protected").cloned().unwrap_or_default()),
                json!(params.get("constitution_present").is_some_and(|v| v == "1")),
                json!(params.get("stop_present").is_some_and(|v| v == "1")),
            ],
        );
        let now = params.get("now").cloned().unwrap_or_default();
        let plan_fallback;
        let plan_res = match &tally {
            Some(res) if area.is_empty() => Some(res),
            _ => {
                plan_fallback = hive_tally::compute_light(
                    store,
                    project,
                    "",
                    hive_tally::Thresholds {
                        now: now.clone(),
                        ..Default::default()
                    },
                )
                .ok();
                plan_fallback.as_ref()
            }
        };
        let plan_ctx = hive_tally::plan_context_for(store, project, &now, plan_res);
        d.enrich_threads();
        d.collect_owner_questions(&decided_posts(store, project)?, &db_posts, Some(store));
        // The rows carry no body, so the body of a post that has a row comes
        // from the store post the row was built from (every row's id is a
        // `db_posts` key).
        let body_of = |id: &str| -> &str {
            if d.by_id.contains_key(id) {
                db_posts
                    .get(id)
                    .map(|p| p.body.trim_matches('\n'))
                    .unwrap_or("")
            } else {
                ""
            }
        };
        for row in &mut d.proposals {
            let id = str_key(row, "id");
            let post = post_at(&d.by_id, &d.posts, &id).unwrap_or(&EMPTY_ROW);
            let decided =
                post_at(&d.by_id, &d.posts, &str_key(row, "decided_id")).unwrap_or(&EMPTY_ROW);
            for key in ["name", "session", "rel_path", "kind"] {
                row.insert(key.to_string(), json!(str_key(post, key)));
            }
            row.insert("body".to_string(), json!(body_of(&id)));
            for key in ["summary", "agent", "name"] {
                row.insert(format!("decided_{key}"), json!(str_key(decided, key)));
            }
            row.insert(
                "proposal_text".to_string(),
                json!(proposal_text(&str_key(row, "body"))),
            );
            row.insert("can_decide".to_string(), json!(true));
        }
        order(&mut d.proposals, &["-date", "id"]);
        d.proposals.sort_by(|a, b| {
            let aa = str_key(a, "status") == "awaiting";
            let bb = str_key(b, "status") == "awaiting";
            bb.cmp(&aa)
        });
        d.suggestions.sort_by(suggestion_cmp);
        let mut rank = 0;
        for row in &mut d.suggestions {
            let id = str_key(row, "id");
            let post = post_at(&d.by_id, &d.posts, &id).unwrap_or(&EMPTY_ROW);
            row.insert("name".into(), json!(str_key(post, "name")));
            row.insert("session".into(), json!(str_key(post, "session")));
            let job = open_jobs.get(&id).cloned().unwrap_or_default();
            row.insert("job".into(), json!(job.id));
            row.insert("job_status".into(), json!(job.status));
            // Whether the card names a check (`Signal: check:<command>`): the
            // plan gate and the funnel read it, and the tally's records text
            // does not carry it.
            let checked = db_posts
                .get(&id)
                .is_some_and(|post| hive_tally::card_has_check(&post.body));
            row.insert("checked".into(), json!(checked));
            let mut gate = String::new();
            if str_key(row, "status") == "eligible" {
                let post_ref = db_posts.get(&id);
                gate = hive_tally::plan_gate_with(
                    store,
                    plan_ctx.clone(),
                    &p.path,
                    &hive_tally::Suggestion {
                        score: num_value(row.get("score").unwrap_or(&Value::Null)) as i32,
                        id: id.clone(),
                        key: str_key(row, "thread_key"),
                        agent: str_key(row, "agent"),
                        summary: str_key(row, "summary"),
                        job: str_key(row, "job"),
                        job_status: str_key(row, "job_status"),
                        checked,
                        ..Default::default()
                    },
                    post_ref,
                );
            }
            let plannable = str_key(row, "status") == "eligible" && gate.is_empty();
            row.insert("gate".into(), json!(gate));
            row.insert("plannable".into(), json!(plannable));
            row.insert("rank".into(), Value::Null);
            if suggestion_priority(&str_key(row, "status")) < 3 {
                rank += 1;
                row.insert("rank".into(), json!(rank));
            }
            let mut votes = svotes.remove(&id).unwrap_or_default();
            for vote in &mut votes {
                let post =
                    post_at(&d.by_id, &d.posts, &str_key(vote, "post_id")).unwrap_or(&EMPTY_ROW);
                vote.insert("reason".into(), json!(str_key(post, "summary")));
                vote.insert("name".into(), json!(str_key(post, "name")));
                vote.insert("session".into(), json!(str_key(post, "session")));
            }
            order(&mut votes, &["date", "agent"]);
            row.insert("votes".into(), json!(votes));
        }
        order(&mut d.amendments, &["date", "id"]);
        d.amendments.sort_by(|a, b| {
            let ac = str_key(a, "status").starts_with("closed:");
            let bc = str_key(b, "status").starts_with("closed:");
            ac.cmp(&bc)
        });
        for row in &mut d.amendments {
            let id = str_key(row, "id");
            let post = post_at(&d.by_id, &d.posts, &id).unwrap_or(&EMPTY_ROW);
            row.insert("name".into(), json!(str_key(post, "name")));
            row.insert("session".into(), json!(str_key(post, "session")));
            let mut votes = avotes.remove(&id).unwrap_or_default();
            for vote in &mut votes {
                let post_id = str_key(vote, "post_id");
                let post = post_at(&d.by_id, &d.posts, &post_id).unwrap_or(&EMPTY_ROW);
                vote.insert("reason".into(), json!(str_key(post, "summary")));
                vote.insert("name".into(), json!(str_key(post, "name")));
                let mut read_line = String::new();
                for line in body_of(&post_id).split('\n') {
                    if let Some(rest) = line.strip_prefix("Read:") {
                        read_line = rest.trim().to_string();
                        break;
                    }
                }
                vote.insert("read_line".into(), json!(read_line));
            }
            order(&mut votes, &["date", "session"]);
            row.insert("votes".into(), json!(votes));
        }
        d.evolution = self.evolution(project.to_string()).unwrap_or_else(|_| {
            object(
                "loop archive",
                vec![json!({"loop":"","stop":"","limits":""}), json!({"rows":[]})],
            )
        });
        // The suggestion funnel (how cards become changes, and where they
        // stall) is part of the self-improvement view, computed from the rows
        // above so the console and the gate agree.
        d.evolution.insert(
            "funnel".into(),
            json!(crate::funnel::funnel(&d.suggestions, store)),
        );
        let evo_summary = crate::evolution::evolution_summary(d.evolution.clone());
        d.meta = object(
            "schema_version built_at now repo repo_name remote branch head dirty board_present evolution_present board_params posts_without_kind last_error",
            vec![
                json!("3"),
                json!(store.now()),
                json!(now),
                json!(p.path),
                json!(
                    Path::new(&p.path)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                ),
                json!(git_value(
                    &p.path,
                    &["config", "--get", "remote.origin.url"]
                )),
                json!(git_value(&p.path, &["rev-parse", "--abbrev-ref", "HEAD"])),
                json!(git_value(&p.path, &["rev-parse", "--short", "HEAD"])),
                json!(if git_value(
                    &p.path,
                    &[
                        "status",
                        "--porcelain",
                        "--",
                        "docs/board",
                        "docs/evolution"
                    ]
                )
                .is_empty()
                {
                    "0"
                } else {
                    "1"
                }),
                json!(params.get("board_present").cloned().unwrap_or_default()),
                evo_summary
                    .get("evolution_present")
                    .cloned()
                    .unwrap_or_else(|| json!("")),
                json!(d.params.clone()),
                json!(nokind),
                json!(""),
            ],
        );
        if let Ok(metadata) = self.metadata() {
            for (k, v) in metadata {
                d.meta.insert(k, v);
            }
        }
        // The project's id and the name the owner gave it (the folder name
        // until `hive init --name` or the console's Rename), so a view built
        // from the snapshot alone can still name the project.
        d.meta.insert("slug".into(), json!(p.slug));
        d.meta
            .insert("name".into(), json!(crate::read::project_name(&p, &d.meta)));
        Ok(d)
    }

    /// The whole post row, `frontmatter` and `body` included, with the
    /// responses on it; empty for an unknown post. The cached rows carry
    /// neither field, so both are read from the store for the one post asked
    /// for (a primary-key lookup): the frontmatter map of the store post, the
    /// body without its surrounding newlines. A post deleted since the
    /// snapshot was built keeps the row's shape with an empty body and
    /// frontmatter until the watermark rebuilds the snapshot.
    pub(crate) fn post_detail(&self, d: &BoardData, id: &str) -> Result<JsonMap> {
        let mut row = d.post(id);
        if row.is_empty() {
            return Ok(row);
        }
        let store = self
            .store
            .as_deref()
            .ok_or_else(|| anyhow!("the store is unavailable"))?;
        let (frontmatter, body) = match load_post(store, id)? {
            Some(post) => (
                json!(post_frontmatter(&post)),
                json!(post.body.trim_matches('\n')),
            ),
            None => (json!(JsonMap::new()), json!("")),
        };
        row.insert("frontmatter".into(), frontmatter);
        row.insert("body".into(), body);
        Ok(row)
    }
}

impl BoardData {
    pub(crate) fn thread(&self, key: &str) -> JsonMap {
        for thread in &self.threads {
            if str_key(thread, "key") != key {
                continue;
            }
            let mut posts = self.posts.iter()
                .filter(|p| str_key(p, "thread_key") == key)
                .map(|p| pick(p, "id rel_path kind agent name project runner model session date branch expires on_post vote text_hash has_read_line resolution summary provenance_ok provenance_missing"))
                .collect::<Vec<_>>();
            order(&mut posts, &["date", "id"]);
            return object("thread posts", vec![json!(thread), json!(posts)]);
        }
        JsonMap::new()
    }

    pub(crate) fn enrich_threads(&mut self) {
        let mut threads = BTreeMap::<String, usize>::new();
        for (idx, thread) in self.threads.iter_mut().enumerate() {
            let key = str_key(thread, "key");
            threads.insert(key, idx);
            thread.insert("participants".into(), json!(Vec::<String>::new()));
            thread.insert("last_by".into(), json!(""));
            thread.insert("last_kind".into(), json!(""));
            thread.insert("open_questions".into(), json!(0));
            thread.insert("open_handoffs".into(), json!(0));
            thread.insert("active_claims".into(), json!(Vec::<String>::new()));
        }
        // Sort references so `self.posts` keeps its own order.
        let mut posts = self.posts.iter().collect::<Vec<_>>();
        order(&mut posts, &["date", "id"]);
        let mut seen = BTreeMap::<String, BTreeSet<String>>::new();
        let mut by_thread = BTreeMap::<String, Vec<&PostRow>>::new();
        for &post in &posts {
            let key = str_key(post, "thread_key");
            let Some(&idx) = threads.get(&key) else {
                continue;
            };
            by_thread.entry(key.clone()).or_default().push(post);
            let agent = str_key(post, "agent");
            if !agent.is_empty() && seen.entry(key.clone()).or_default().insert(agent.clone()) {
                let mut values = self.threads[idx]
                    .get("participants")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                values.push(json!(agent));
                self.threads[idx].insert("participants".into(), Value::Array(values));
            }
            if str_key(post, "date") >= str_key(&self.threads[idx], "last") {
                self.threads[idx].insert("last_by".into(), json!(str_key(post, "agent")));
                self.threads[idx].insert("last_kind".into(), json!(str_key(post, "kind")));
            }
        }
        for claim in &self.claims {
            if str_key(claim, "status") != "active" {
                continue;
            }
            let key = str_key(claim, "thread_key");
            if let Some(&idx) = threads.get(&key) {
                let mut values = self.threads[idx]
                    .get("active_claims")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                values.push(json!(str_key(claim, "agent")));
                self.threads[idx].insert("active_claims".into(), Value::Array(values));
            }
        }
        let answered = posts
            .iter()
            .filter(|p| {
                matches!(str_key(p, "kind").as_str(), "answer" | "result")
                    && !str_key(p, "on_post").is_empty()
            })
            .map(|p| str_key(p, "on_post"))
            .collect::<BTreeSet<_>>();
        for (key, thread_posts) in by_thread {
            let Some(&idx) = threads.get(&key) else {
                continue;
            };
            if str_key(&self.threads[idx], "status") == "closed" {
                continue;
            }
            for (i, post) in thread_posts.iter().enumerate() {
                match str_key(post, "kind").as_str() {
                    "question"
                        if str_key(post, "area") != "jobs"
                            && !answered.contains(&str_key(post, "id"))
                            && !later_hub_answer_date(&thread_posts, i) =>
                    {
                        inc(&mut self.threads[idx], "open_questions");
                    }
                    "handoff" if !later_hub_kind(&thread_posts, i, &["claim", "result"]) => {
                        inc(&mut self.threads[idx], "open_handoffs");
                    }
                    _ => {}
                }
            }
        }
    }

    /// Questions that wait for the owner: posts marked `for_owner` - a
    /// question, or a finding, handoff or suggestion an agent addressed to
    /// the owner, such as the security watcher's "a major upgrade: your
    /// call" - and the evolution loop's stop questions (asks for the owner by
    /// construction). One is open until the owner decides it, an answer
    /// lands on it or later in its thread, or its thread closes. A stop
    /// question ignores a closure dated before it: decisions close the
    /// loop-gate thread, and the loop keeps posting new stops into it.
    /// Card-needed asks are the planner's own work since ADR-044 and never
    /// wait for the owner. `db_posts` holds the store posts the rows were
    /// built from: the owner mark and the body live there, not in the rows.
    pub(crate) fn collect_owner_questions(
        &mut self,
        decided: &BTreeSet<String>,
        db_posts: &BTreeMap<String, hive_tally::Post>,
        store: Option<&hive_store::Store>,
    ) {
        let major_updates_auto =
            store.is_some_and(|s| s.setting("dependencies.major_updates", "ask") == "auto");
        // The watcher's major-upgrade findings, read from its
        // `security.major` events: the one test the runner uses too.
        let majors = store
            .and_then(|s| s.conn().ok())
            .and_then(|c| hive_board::security_major_findings(&c, None).ok())
            .unwrap_or_default();
        let threads = self
            .threads
            .iter()
            .map(|t| {
                (
                    str_key(t, "key"),
                    (str_key(t, "status"), str_key(t, "last")),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut posts = self.posts.iter().collect::<Vec<_>>();
        order(&mut posts, &["date", "id"]);
        let answered = posts
            .iter()
            .filter(|p| str_key(p, "kind") == "answer" && !str_key(p, "on_post").is_empty())
            .map(|p| str_key(p, "on_post"))
            .collect::<BTreeSet<_>>();
        let mut by_thread = BTreeMap::<String, Vec<&PostRow>>::new();
        for &post in &posts {
            by_thread
                .entry(str_key(post, "thread_key"))
                .or_default()
                .push(post);
        }
        let mut rows = Vec::new();
        for (key, thread_posts) in &by_thread {
            let (status, last) = threads.get(key).cloned().unwrap_or_default();
            for (i, post) in thread_posts.iter().enumerate() {
                let kind = str_key(post, "kind");
                if !OWNER_ASK_KINDS.contains(&kind.as_str()) || str_key(post, "area") == "jobs" {
                    continue;
                }
                let summary = str_key(post, "summary");
                let stop = kind == "question" && summary == hive_store::EVOLUTION_STOP_SUMMARY;
                let id = str_key(post, "id");
                let stored = db_posts.get(&id);
                let for_owner = stored.is_some_and(|p| p.for_owner);
                if !(stop || for_owner) {
                    continue;
                }
                // A later answer in the thread settles a question; a
                // finding on a running thread (one security thread holds
                // many) only its own answer or the owner's decision.
                if decided.contains(&id)
                    || answered.contains(&id)
                    || (kind == "question" && later_hub_answer_date(thread_posts, i))
                {
                    continue;
                }
                let body = stored
                    .map(|p| p.body.trim_matches('\n'))
                    .unwrap_or_default();
                if stop && body.contains(hive_evolve::CARD_NEEDED) {
                    continue;
                }
                // Why only the owner can decide it (money, a destructive
                // act, credentials, publication, Tier 2 ...); empty for a
                // routine ask the lead decides (TASK-129). The Sessions
                // strip lists the rows whose `decider` is `owner`.
                let mut hard_stop = hive_board::hard_stop_ask(&kind, &summary, body);
                // A question the daemon asked (a restart drain, a sign-in,
                // a lead seat) is never the lead's: the daemon answers it
                // itself once the owner acts.
                if hard_stop.is_empty() && kind == "question" && str_key(post, "agent") == "daemon"
                {
                    hard_stop = "was asked by the daemon".into();
                }
                // A question goes with its thread; a finding, handoff or
                // suggestion on a running thread (one security thread
                // holds many) is settled by its own answer or decision, or
                // by a result that closes the thread as a whole - never by
                // the owner's decision on a sibling, whose result post
                // names that sibling.
                let resolved = if stop {
                    thread_posts[i + 1..]
                        .iter()
                        .any(|p| !str_key(p, "resolution").is_empty())
                } else if kind == "question" {
                    status == "closed"
                } else {
                    thread_posts.iter().any(|p| {
                        let on = str_key(p, "on_post");
                        !str_key(p, "resolution").is_empty() && (on.is_empty() || on == id)
                    })
                };
                if resolved {
                    continue;
                }
                // TASK-129: the owner answers with one pick. An ask that
                // names no answers, or none recommended, goes back to the
                // project's lead, which writes them or decides it; only the
                // loop's stop (a hard stop) stays the owner's whatever it says.
                let major = majors.contains(&id);
                if major && !major_updates_auto && hard_stop.is_empty() {
                    hard_stop =
                        "is a major dependency upgrade (dependencies.major_updates = ask)".into();
                }
                // A major-upgrade finding the watcher posted before it wrote
                // a `Recommended:` line gets the watcher's own answers: it
                // is the owner's and is never handed to the lead for lacking
                // them (PR #278 review).
                let options = if major {
                    hive_board::ask_options(&hive_board::with_security_major_answers(body))
                } else {
                    hive_board::ask_options(body)
                };
                let decider = match question_decider(&hard_stop, major, major_updates_auto) {
                    "owner" if !stop && !options.ready() => "lead",
                    who => who,
                };
                rows.push(object(
                    "id thread_key area thread kind agent name session date summary body on_post stop stale thread_last decider hard_stop options recommended_why needs_options",
                    vec![
                        json!(id),
                        json!(key),
                        json!(str_key(post, "area")),
                        json!(str_key(post, "thread")),
                        json!(kind),
                        json!(str_key(post, "agent")),
                        json!(str_key(post, "name")),
                        json!(str_key(post, "session")),
                        json!(str_key(post, "date")),
                        json!(str_key(post, "summary")),
                        json!(body),
                        json!(str_key(post, "on_post")),
                        json!(stop),
                        json!(status == "stale"),
                        json!(last),
                        json!(decider),
                        json!(hard_stop),
                        json!(options.options),
                        json!(options.why),
                        json!(if options.ready() { String::new() } else { options.missing() }),
                    ],
                ));
            }
        }
        order(&mut rows, &["-date", "id"]);
        rows.sort_by_key(|r| !r.get("stop").and_then(Value::as_bool).unwrap_or(false));
        let delegated = rows
            .iter()
            .filter(|r| r.get("decider").and_then(Value::as_str).unwrap_or("owner") != "owner")
            .count();
        if delegated > 0 {
            self.questions_meta.insert(
                "agents_deciding".into(),
                json!({
                    "count": delegated,
                    "note": format!("{delegated} more the agents are deciding (the lead decides routine asks and writes out the answers of a hard stop that names none; or a setting answers them)"),
                }),
            );
        }
        self.questions = rows;
    }

    pub(crate) fn post_by_id(&self, id: &str) -> Option<&PostRow> {
        post_at(&self.by_id, &self.posts, id)
    }

    /// The cached row of `id` with the responses on it, or an empty map for
    /// an unknown post. The row has no `body` or `frontmatter`: the read API
    /// serves posts through `Handler::post_detail`, which adds them.
    pub(crate) fn post(&self, id: &str) -> JsonMap {
        let Some(p) = self.post_by_id(id) else {
            return JsonMap::new();
        };
        let mut result = p.to_map();
        let mut responses = self
            .posts
            .iter()
            .filter(|row| str_key(row, "on_post") == id)
            .map(|row| pick(row, "id kind agent name session date vote summary"))
            .collect::<Vec<_>>();
        order(&mut responses, &["date"]);
        result.insert("responses".into(), json!(responses));
        result
    }

    pub(crate) fn overview(&self) -> JsonMap {
        let (mut active, mut stale) = (Vec::new(), Vec::new());
        for claim in &self.claims {
            let mut row = claim.clone();
            let post = self.post_by_id(&str_key(&row, "id"));
            row.insert(
                "name".into(),
                json!(post.map(|p| str_key(p, "name")).unwrap_or_default()),
            );
            row.insert(
                "session".into(),
                json!(post.map(|p| str_key(p, "session")).unwrap_or_default()),
            );
            // When the claim was made: Working now says since when, not until.
            row.insert(
                "date".into(),
                json!(post.map(|p| str_key(p, "date")).unwrap_or_default()),
            );
            match str_key(&row, "status").as_str() {
                "active" => active.push(row),
                "expired" | "no expiry" => stale.push(row),
                _ => {}
            }
        }
        order(&mut active, &["expires"]);
        order(&mut stale, &["-expires"]);
        let mut open = self
            .threads
            .iter()
            .filter(|t| matches!(str_key(t, "status").as_str(), "open" | "stale"))
            .cloned()
            .collect::<Vec<_>>();
        order(&mut open, &["-last"]);
        // The twelve newest posts: sort references, then pick only those twelve.
        let mut by_date = self.posts.iter().collect::<Vec<_>>();
        order(&mut by_date, &["-date"]);
        let recent = by_date
            .iter()
            .take(12)
            .map(|p| {
                pick(
                    p,
                    "id thread_key kind agent name session date summary provenance_ok",
                )
            })
            .collect::<Vec<_>>();
        let mut kinds = BTreeMap::<String, i64>::new();
        let mut areas = BTreeSet::<String>::new();
        // One row per agent + "\x00" + session; the map walks them in key
        // order, which the stable sort by -last below keeps for ties.
        let mut agents = BTreeMap::<String, JsonMap>::new();
        let mut without = 0;
        for p in &self.posts {
            let mut kind = str_key(p, "kind");
            if kind.is_empty() {
                kind = "(none)".into();
            }
            *kinds.entry(kind).or_default() += 1;
            areas.insert(str_key(p, "area"));
            without += 1 - num_value(p.get("provenance_ok").unwrap_or(&Value::Null));
            let key = format!("{}\x00{}", str_key(p, "agent"), str_key(p, "session"));
            let row = agents.entry(key).or_insert_with(|| {
                object(
                    "agent session name posts last",
                    vec![
                        p.get("agent").cloned().unwrap_or(Value::Null),
                        p.get("session").cloned().unwrap_or(Value::Null),
                        Value::Null,
                        json!(0),
                        json!(""),
                    ],
                )
            });
            let posts = num_value(row.get("posts").unwrap_or(&Value::Null)) + 1;
            row.insert("posts".into(), json!(posts));
            if str_key(p, "date") > str_key(row, "last") {
                row.insert("last".into(), p.get("date").cloned().unwrap_or(Value::Null));
            }
            let name = str_key(p, "name");
            if !name.is_empty() && name > str_key(row, "name") {
                row.insert("name".into(), p.get("name").cloned().unwrap_or(Value::Null));
            }
        }
        let mut agent_rows = agents.into_values().collect::<Vec<_>>();
        order(&mut agent_rows, &["-last"]);
        agent_rows.truncate(12);
        let mut evo = crate::evolution::evolution_summary(self.evolution.clone());
        if let Some(v) = evo.get("missing_files").cloned() {
            evo.insert(
                "missing_files".into(),
                json!(hive_store::gojson::to_string(&v).unwrap_or_else(|_| "null".into())),
            );
        }
        object(
            "board_present posts_without_kind quorum claims_active claims_expired threads_open threads_stale threads_closed posts posts_without_provenance suggestions_eligible suggestions_plannable suggestions_needing_votes suggestions_rejected suggestions_expired amendments_open amendments_passed proposals_awaiting proposals_decided active_claims stale_claims open_threads recent_posts evolution kinds areas agents",
            vec![
                json!(str_key(&self.meta, "board_present") == "1"),
                json!(num_value(
                    self.meta.get("posts_without_kind").unwrap_or(&Value::Null)
                )),
                self.params.get("quorum").cloned().unwrap_or(Value::Null),
                json!(active.len()),
                json!(stale.len()),
                json!(open.len()),
                json!(count_status(&self.threads, &["stale"])),
                json!(count_status(&self.threads, &["closed"])),
                json!(self.posts.len()),
                json!(without),
                json!(count_status(&self.suggestions, &["eligible"])),
                json!(count_plannable(&self.suggestions)),
                json!(count_status(&self.suggestions, &["needs votes"])),
                json!(count_status(&self.suggestions, &["rejected"])),
                json!(count_status(&self.suggestions, &["expired"])),
                json!(
                    self.amendments
                        .iter()
                        .filter(|a| !str_key(a, "status").starts_with("closed:"))
                        .count()
                ),
                json!(
                    self.amendments
                        .iter()
                        .filter(|a| str_key(a, "status").starts_with("passed"))
                        .count()
                ),
                json!(count_status(&self.proposals, &["awaiting"])),
                json!(count_status(&self.proposals, &["accepted", "declined"])),
                json!(active),
                json!(stale),
                json!(open),
                json!(recent),
                json!(evo),
                json!(kinds),
                json!(areas.into_iter().collect::<Vec<_>>()),
                json!(agent_rows),
            ],
        )
    }

    pub(crate) fn threads(&self) -> Vec<JsonMap> {
        let mut rows = self.threads.clone();
        order(&mut rows, &["-last"]);
        rows.sort_by_key(|r| match str_key(r, "status").as_str() {
            "open" => 0,
            "stale" => 1,
            "closed" => 2,
            _ => 3,
        });
        rows
    }

    pub(crate) fn notices(&self) -> JsonMap {
        let suggestions = count_status(&self.suggestions, &["needs votes"]);
        let amendments = self
            .amendments
            .iter()
            .filter(|a| !str_key(a, "status").starts_with("closed:"))
            .count() as i64;
        let proposals = count_status(&self.proposals, &["awaiting"]);
        let slug = str_key(&self.meta, "slug");
        let name = str_key(&self.meta, "name");
        let (slug, name) = match (slug.is_empty(), name.is_empty()) {
            (false, false) => (slug, name),
            (false, true) => (slug.clone(), slug),
            _ => {
                let repo = str_key(&self.meta, "repo_name");
                (repo.clone(), repo)
            }
        };
        object(
            "ballot votes_open proposals_awaiting brain brains commons_present commons_base commons_questions",
            vec![
                json!([object(
                    "slug base name brain suggestions amendments proposals",
                    vec![
                        json!(slug),
                        json!(""),
                        json!(name),
                        json!(false),
                        json!(suggestions),
                        json!(amendments),
                        json!(proposals)
                    ]
                )]),
                json!(suggestions + amendments),
                json!(proposals),
                json!(""),
                json!(Vec::<String>::new()),
                Value::Null,
                json!(""),
                Value::Null,
            ],
        )
    }
}

/// Who answers an ask addressed to the owner. A security finding about a
/// major upgrade (`major`: the watcher's `security.major` event names it) is
/// the `setting`'s call under `dependencies.major_updates = auto`, and the
/// owner's under `ask` (its `hard_stop` says so); an ask at a hard stop (`hard_stop` names why, from
/// [`hive_board::hard_stop_ask`]) waits for the `owner`; every other ask is
/// routine and the project's `lead` decides it on the owner's behalf,
/// whatever `owner.presence` says (TASK-129).
fn question_decider(hard_stop: &str, major: bool, major_updates_auto: bool) -> &'static str {
    if major && major_updates_auto {
        return "setting";
    }
    if !hard_stop.is_empty() {
        return "owner";
    }
    "lead"
}

pub(crate) fn str_value(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn num_value(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0)
}

pub(crate) fn object(keys: &str, values: Vec<Value>) -> JsonMap {
    let mut result = JsonMap::new();
    for (key, value) in keys.split_whitespace().zip(values) {
        result.insert(key.to_string(), value);
    }
    result
}

pub(crate) fn pick<R: Fields + ?Sized>(row: &R, keys: &str) -> JsonMap {
    let mut out = JsonMap::new();
    for key in keys.split_whitespace() {
        out.insert(
            key.to_string(),
            row.field(key).cloned().unwrap_or(Value::Null),
        );
    }
    out
}

pub(crate) fn order<R: Fields>(rows: &mut [R], fields: &[&str]) {
    // str() of a field without allocating when it already is a string.
    fn key<'a, R: Fields>(row: &'a R, field: &str) -> std::borrow::Cow<'a, str> {
        match row.field(field) {
            Some(Value::String(s)) => std::borrow::Cow::Borrowed(s.as_str()),
            Some(v) => std::borrow::Cow::Owned(str_value(v)),
            None => std::borrow::Cow::Borrowed(""),
        }
    }
    rows.sort_by(|a, b| {
        for field in fields {
            let desc = field.starts_with('-');
            let field = field.trim_start_matches('-');
            let av = key(a, field);
            let bv = key(b, field);
            if av != bv {
                return if desc { bv.cmp(&av) } else { av.cmp(&bv) };
            }
        }
        std::cmp::Ordering::Equal
    });
}

/// Runs git with a five-second deadline. Stdout is read on its own thread
/// while the command runs, so a long listing never fills the pipe and stalls
/// git, and a command still running at the deadline is killed. Any failure
/// reads as "" - an unborn checkout has no branch, a missing remote is normal.
/// Not being able to start git at all is not normal (no `git`, or the hub
/// out of open files - then every project reads as missing from disk), so
/// that goes to the hub's log, at most once a minute.
pub(crate) fn git_value(repo: &str, args: &[&str]) -> String {
    let mut command = Command::new(hive_gitenv::git_program());
    command.arg("-C").arg(repo).args(args);
    command.env_clear();
    for kv in hive_gitenv::env() {
        if let Some((k, v)) = kv.split_once('=') {
            command.env(k, v);
        }
    }
    // Nothing here reads the child's stderr, so it must not reach the hub's
    // own log either.
    let mut child = match command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            if GIT_SPAWN_LOG.take() {
                eprintln!("hive hub: could not run git for {repo}: {err}");
            }
            return String::new();
        }
    };
    let Some(mut stdout) = child.stdout.take() else {
        return String::new();
    };
    let reader = std::thread::Builder::new()
        .name("hive-hub-git".into())
        .spawn(move || {
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut stdout, &mut out).map(|_| out)
        });
    let start = std::time::Instant::now();
    let mut nap = std::time::Duration::from_micros(250);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() >= std::time::Duration::from_secs(5) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => {
                std::thread::sleep(nap);
                nap = (nap * 2).min(std::time::Duration::from_millis(10));
            }
            Err(_) => break None,
        }
    };
    let out = match reader.map(|r| r.join()) {
        Ok(Ok(Ok(out))) => out,
        _ => return String::new(),
    };
    match status {
        Some(status) if status.success() => String::from_utf8_lossy(&out).trim().to_string(),
        _ => String::new(),
    }
}

/// One log line a minute for git failing to start, however many
/// projects ask in that minute.
static GIT_SPAWN_LOG: LogGate = LogGate::new(std::time::Duration::from_secs(60));

/// Lets a message through once per `every`.
pub(crate) struct LogGate {
    every: std::time::Duration,
    last: Mutex<Option<std::time::Instant>>,
}

impl LogGate {
    pub(crate) const fn new(every: std::time::Duration) -> Self {
        Self {
            every,
            last: Mutex::new(None),
        }
    }

    /// True when the message may go out now; the gate closes for `every`.
    pub(crate) fn take(&self) -> bool {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        match *last {
            Some(at) if now.duration_since(at) < self.every => false,
            _ => {
                *last = Some(now);
                true
            }
        }
    }
}

pub(crate) fn present(p: &hive_store::Project) -> bool {
    Path::new(&p.path).is_dir()
        && git_value(&p.path, &["rev-parse", "--is-inside-work-tree"]) == "true"
}

pub(crate) fn truncate(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

pub(crate) fn post_frontmatter(p: &hive_tally::Post) -> JsonMap {
    let mut fm = object(
        "kind agent runner model session date branch summary",
        vec![
            json!(p.kind),
            json!(p.agent),
            json!(p.runner),
            json!(p.model),
            json!(p.session),
            json!(p.date),
            json!(p.branch),
            json!(p.summary),
        ],
    );
    for (key, value) in [
        ("expires", &p.expires),
        ("on", &p.on_post),
        ("vote", &p.vote),
        ("resolution", &p.resolution),
        ("text_hash", &p.text_hash),
        ("links", &p.links),
        ("files", &p.files),
        ("name", &p.name),
        ("project", &p.origin_project),
    ] {
        if !value.is_empty() {
            fm.insert(key.into(), json!(value));
        }
    }
    if p.for_owner {
        fm.insert("for_owner".into(), json!("true"));
    }
    fm.insert("id".into(), json!(p.id));
    fm.insert("area".into(), json!(p.area));
    fm.insert("thread".into(), json!(p.thread));
    fm.insert("path".into(), json!(format!("store:{}", p.id)));
    fm
}

static PROPOSAL_HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\W*proposal for the human").unwrap());
static PROPOSAL_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\W*proposal for the human[^:]*:\W*").unwrap());

pub(crate) fn proposal_text(body: &str) -> String {
    let mut lines = body.split('\n').collect::<Vec<_>>();
    let mut start = None;
    for (i, line) in lines.iter().enumerate() {
        if PROPOSAL_HEADING.is_match(line.trim()) {
            start = Some(i);
            break;
        }
    }
    let mut para = Vec::new();
    if let Some(i) = start {
        let first = PROPOSAL_PREFIX
            .replace(line_trim(lines[i]), "")
            .trim()
            .to_string();
        if !first.is_empty() {
            para.push(first);
        }
        lines = lines[i + 1..].to_vec();
    }
    for line in lines {
        let line = line.trim();
        if start.is_none() && line.starts_with('#') {
            continue;
        }
        if line.is_empty() {
            if !para.is_empty() {
                break;
            }
            continue;
        }
        para.push(line.to_string());
    }
    let mut text = para.join(" ");
    if start.is_some() {
        text = text.trim().trim_matches(['*', ' ']).trim().to_string();
    }
    truncate(&text, 600)
}

fn line_trim(s: &str) -> &str {
    s.trim()
}

pub(crate) fn later_hub_kind(posts: &[&PostRow], index: usize, kinds: &[&str]) -> bool {
    let want = kinds.iter().copied().collect::<BTreeSet<_>>();
    posts[index + 1..]
        .iter()
        .any(|post| want.contains(str_key(post, "kind").as_str()))
}

pub(crate) fn later_hub_answer_date(posts: &[&PostRow], index: usize) -> bool {
    let date = str_key(posts[index], "date");
    posts[index + 1..]
        .iter()
        .any(|post| str_key(post, "kind") == "answer" && str_key(post, "date") > date)
}

/// The post kinds that wait for the owner when marked `for_owner`: an ask,
/// or a report that ends in the owner's call. Answers, results, votes and
/// opinions never do.
pub(crate) const OWNER_ASK_KINDS: &[&str] = &["question", "finding", "handoff", "suggestion"];

/// The posts of a project the owner (or the lead on the owner's behalf) has
/// already decided; a decided question no longer waits for anyone.
pub(crate) fn decided_posts(store: &hive_store::Store, project: &str) -> Result<BTreeSet<String>> {
    let conn = store.conn()?;
    let mut stmt = conn.prepare("SELECT post_id FROM decisions WHERE project = ?")?;
    let rows = stmt.query_map([project], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<BTreeSet<_>, _>>()?)
}

/// One store post as `hive_tally::posts_for` would load it, or `None` when
/// no post has that id. The read API hydrates a post's body and frontmatter
/// from this instead of keeping them in every cached row.
pub(crate) fn load_post(store: &hive_store::Store, id: &str) -> Result<Option<hive_tally::Post>> {
    use rusqlite::OptionalExtension;
    let conn = store.conn()?;
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {} FROM posts WHERE id = ?",
        hive_tally::POST_COLS
    ))?;
    let post = stmt
        .query_row([id], |row| {
            Ok(hive_tally::Post {
                id: row.get(0)?,
                project: row.get(1)?,
                area: row.get(2)?,
                thread: row.get(3)?,
                kind: row.get(4)?,
                agent: row.get(5)?,
                runner: row.get(6)?,
                model: row.get(7)?,
                session: row.get(8)?,
                date: row.get(9)?,
                branch: row.get(10)?,
                expires: row.get(11)?,
                on_post: row.get(12)?,
                vote: row.get(13)?,
                text_hash: row.get(14)?,
                has_read_line: row.get(15)?,
                resolution: row.get(16)?,
                summary: row.get(17)?,
                body: row.get(18)?,
                links: row.get(19)?,
                files: row.get(20)?,
                name: row.get(21)?,
                origin_project: row.get(22)?,
                extra: row.get(23)?,
                origin: row.get(24)?,
                created_at: row.get(25)?,
                ..Default::default()
            })
        })
        .optional()?;
    post.map(|mut p| {
        p.for_owner =
            hive_store::post_for_owner(&p.extra).map_err(|e| anyhow!("post {}: {e}", p.id))?;
        Ok(p)
    })
    .transpose()
}

pub(crate) fn count_status(rows: &[JsonMap], statuses: &[&str]) -> i64 {
    rows.iter()
        .filter(|row| {
            statuses
                .iter()
                .any(|status| str_key(row, "status") == *status)
        })
        .count() as i64
}

pub(crate) fn count_plannable(rows: &[JsonMap]) -> i64 {
    rows.iter()
        .filter(|row| {
            row.get("plannable")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count() as i64
}

pub(crate) fn sorted_keys<V>(m: &BTreeMap<String, V>) -> Vec<String> {
    m.keys().cloned().collect()
}

fn vals(values: &[&str]) -> Vec<Value> {
    values.iter().map(|v| json!(*v)).collect()
}

fn num_str(s: &str) -> i64 {
    s.parse().unwrap_or(0)
}

fn str_key<R: Fields + ?Sized>(row: &R, key: &str) -> String {
    row.field(key).map(str_value).unwrap_or_default()
}

fn inc(row: &mut JsonMap, key: &str) {
    let n = num_value(row.get(key).unwrap_or(&Value::Null)) + 1;
    row.insert(key.to_string(), json!(n));
}

fn suggestion_priority(status: &str) -> i32 {
    match status {
        "eligible" => 0,
        "needs votes" => 1,
        "rejected" => 2,
        "expired" => 3,
        _ => 3,
    }
}

fn suggestion_cmp(a: &JsonMap, b: &JsonMap) -> std::cmp::Ordering {
    suggestion_priority(&str_key(a, "status"))
        .cmp(&suggestion_priority(&str_key(b, "status")))
        .then_with(|| {
            num_value(b.get("score").unwrap_or(&Value::Null))
                .cmp(&num_value(a.get("score").unwrap_or(&Value::Null)))
        })
        .then_with(|| str_key(a, "date").cmp(&str_key(b, "date")))
        .then_with(|| str_key(a, "id").cmp(&str_key(b, "id")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers_http as http;
    use serde_json::json;
    use std::process::Command;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .arg("init")
            .arg(dir.path())
            .output()
            .unwrap();
        dir
    }

    // Output() reads stdout while git runs; the earlier poll-then-read loop
    // hung on any listing longer than the pipe buffer until its 5 s kill and
    // then returned "" for a command that had succeeded.
    #[test]
    fn git_value_reads_output_longer_than_a_pipe_buffer() {
        let dir = repo();
        let repo = dir.path().to_str().unwrap();
        for i in 0..3000 {
            std::fs::write(
                dir.path().join(format!("untracked-file-number-{i:05}.txt")),
                "x",
            )
            .unwrap();
        }
        let start = std::time::Instant::now();
        let listing = git_value(repo, &["status", "--porcelain", "--untracked-files=all"]);
        assert!(start.elapsed() < std::time::Duration::from_secs(4));
        assert!(listing.len() > 64 * 1024, "{} bytes", listing.len());
        assert_eq!(listing.lines().count(), 3000);
        assert_eq!(
            git_value(repo, &["rev-parse", "--is-inside-work-tree"]),
            "true"
        );
        // An unborn branch has no HEAD: git fails on stderr, the value is "".
        assert_eq!(git_value(repo, &["rev-parse", "--short", "HEAD"]), "");
    }

    fn join(store: &hive_store::Store, name: &str) -> hive_store::Agent {
        hive_board::join_agent(
            store,
            hive_board::JoinInput {
                name: name.to_string(),
                runner: "test".to_string(),
                model: "test".to_string(),
                session: format!("s-{name}"),
                ..Default::default()
            },
        )
        .unwrap()
        .0
    }

    /// A realistic row's values, in `POST_ROW_FIELDS` order.
    fn sample_row_values() -> Vec<Value> {
        vec![
            json!("20261012T092211Z-copilot-fable-1012-hub"),
            json!("docs/board/runner/disk-floor/20261012T092211Z-copilot-fable-1012-hub.md"),
            json!("runner"),
            json!("disk-floor"),
            json!("runner/disk-floor"),
            json!("finding"),
            json!("owner"),
            json!("copilot-fable-1012-hub"),
            json!("copilot"),
            json!("claude-fable-5.1"),
            json!("95b0148f-1d3e-4993-8e59-e81b902cbfe2"),
            json!("2026-10-12T09:22:11Z"),
            json!("main"),
            json!(""),
            json!(""),
            json!(""),
            json!(""),
            json!(0),
            json!(""),
            json!("Disk floor: the runner refused new work under 10 GB free; 17.7 GB reclaimed."),
            json!(""),
            json!(""),
            json!(1),
            json!(Vec::<String>::new()),
            json!(1),
            json!("copilot-fable-1012-hub"),
            json!("amistio-template"),
        ]
    }

    // The hub keeps one row per post of every project in memory; the map a
    // row used to be cost four tree nodes and 26 owned keys per post over
    // the values themselves (a 60k-post board held nearly 200 MB of that).
    #[test]
    fn post_row_costs_no_heap_beyond_its_values() {
        let values = sample_row_values();
        // Other tests allocate on their own threads: the smallest of many
        // short windows is this thread's own cost.
        let measure = |build: &dyn Fn(Vec<Value>) -> usize| {
            (0..64)
                .map(|_| {
                    let values = values.clone();
                    let before = crate::allocated_bytes();
                    let keep = build(values);
                    let after = crate::allocated_bytes();
                    std::hint::black_box(keep);
                    after - before
                })
                .min()
                .unwrap()
        };
        let row_bytes = measure(&|values| PostRow::new(values).0.len());
        let map_bytes = measure(&|values| object(POST_ROW_FIELDS, values).len());
        assert_eq!(row_bytes, 0, "a row allocates nothing beyond its values");
        assert!(
            map_bytes > 2048,
            "the map it replaces allocated {map_bytes} bytes"
        );
        // The same keys, the same values, the same JSON.
        let row = PostRow::new(values.clone());
        let map = object(POST_ROW_FIELDS, values);
        assert_eq!(row.to_map(), map);
        assert_eq!(
            hive_store::gojson::to_vec(&row).unwrap(),
            hive_store::gojson::to_vec(&map).unwrap()
        );
        assert_eq!(row.get("kind"), Some(&json!("finding")));
        assert_eq!(row.get("body"), None);
        assert_eq!(str_key(&row, "thread_key"), "runner/disk-floor");
        assert_eq!(str_key(&EMPTY_ROW, "thread_key"), "");
        let mut rows = vec![&row, &EMPTY_ROW];
        order(&mut rows, &["date"]);
        assert!(std::ptr::eq(rows[0], &EMPTY_ROW));
    }

    #[test]
    fn post_row_keys_are_the_fields_in_map_order() {
        let mut fields = POST_ROW_FIELDS.split_whitespace().collect::<Vec<_>>();
        fields.sort_unstable();
        assert_eq!(POST_ROW_KEYS.to_vec(), fields);
    }

    #[test]
    fn store_suggestion_vote_reason_uses_summary() {
        let _serial = crate::test_helpers::serial();
        for (vote, summary) in [
            ("up", "up: fewer retries"),
            ("down", "down: no measured improvement"),
            ("up", "up"),
            ("down", "down"),
        ] {
            let s = http::additions_store(false);
            http::additions_project(&s, "one");
            join(&s, "proposer");
            join(&s, "voter");
            let suggestion = hive_board::create_post(
                &s,
                hive_board::PostInput {
                    kind: "suggestion".into(),
                    project: "one".into(),
                    area: "governance".into(),
                    thread: "vote-reasons".into(),
                    agent: "proposer".into(),
                    session: "s-proposer".into(),
                    summary: "Show why agents voted.".into(),
                    ..Default::default()
                },
            )
            .unwrap();
            let vote_post = hive_board::create_post(
                &s,
                hive_board::PostInput {
                    kind: "vote".into(),
                    project: "one".into(),
                    area: "governance".into(),
                    thread: "vote-reasons".into(),
                    agent: "voter".into(),
                    session: "s-voter".into(),
                    on: suggestion.id.clone(),
                    vote: vote.into(),
                    summary: summary.into(),
                    body: "## Vote\n\nBody detail is not the displayed reason.\n".into(),
                    ..Default::default()
                },
            )
            .unwrap();
            let data = http::handler(&s).snapshot("one", "").unwrap();
            assert_eq!(data.suggestions.len(), 1);
            assert_eq!(str_key(&data.suggestions[0], "id"), suggestion.id);
            let votes = data.suggestions[0]["votes"].as_array().unwrap();
            assert_eq!(votes.len(), 1);
            assert_eq!(votes[0]["post_id"], vote_post.id);
            assert_eq!(votes[0]["vote"], vote);
            assert_eq!(votes[0]["classification"], "counted");
            assert_eq!(votes[0]["reason"], summary);
        }
    }

    #[test]
    fn store_amendment_vote_reason_uses_summary() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        http::additions_project(&s, "one");
        http::additions_record(
            &s,
            "EVO-constitution",
            "one",
            "evolution",
            "constitution",
            "## Workflow\n\nRun one check.\n",
        );
        join(&s, "proposer");
        join(&s, "voter");
        let amendment = hive_board::create_post(&s, hive_board::PostInput {
            kind: "amendment".into(),
            project: "one".into(),
            area: "governance".into(),
            thread: "checks".into(),
            agent: "proposer".into(),
            session: "s-proposer".into(),
            summary: "Run both checks before committing.".into(),
            body: "## Current text\n\n```\nRun one check.\n```\n\n## Proposed text\n\n```\nRun both checks.\n```\n".into(),
            ..Default::default()
        }).unwrap();
        let summary = "Both checks catch more regressions.";
        let restatement = "The workflow runs both checks instead of one.";
        let vote = hive_board::create_post(
            &s,
            hive_board::PostInput {
                kind: "vote".into(),
                project: "one".into(),
                area: "governance".into(),
                thread: "checks".into(),
                agent: "voter".into(),
                session: "s-voter".into(),
                on: amendment.id.clone(),
                vote: "up".into(),
                text_hash: amendment.text_hash.clone(),
                summary: summary.into(),
                body: format!(
                    "## Vote\n\nBody detail is not the displayed reason.\n\nRead: {restatement}\n"
                ),
                ..Default::default()
            },
        )
        .unwrap();
        let data = http::handler(&s).snapshot("one", "").unwrap();
        assert_eq!(data.amendments.len(), 1);
        assert_eq!(str_key(&data.amendments[0], "id"), amendment.id);
        let votes = data.amendments[0]["votes"].as_array().unwrap();
        assert_eq!(votes.len(), 1);
        assert_eq!(votes[0]["post_id"], vote.id);
        assert_eq!(votes[0]["classification"], "counted");
        assert_eq!(votes[0]["reason"], summary);
        assert_eq!(votes[0]["read_line"], restatement);
    }

    // The cached rows carry neither body nor frontmatter; what the board
    // derives from a body is still derived, and the post route reads both
    // from the store for the one post it serves.
    #[test]
    fn cached_rows_leave_body_and_frontmatter_to_the_post_route() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        let repo = repo();
        hive_board::add_project(
            &s,
            repo.path().to_str().unwrap(),
            "One",
            "",
            false,
            "",
            "human",
            "one",
        )
        .unwrap();
        join(&s, "alpha");
        join(&s, "bravo");
        let post = |kind: &str,
                    thread: &str,
                    agent: &str,
                    for_owner: bool,
                    summary: &str,
                    body: &str,
                    on: &str,
                    resolution: &str,
                    date: &str| {
            hive_board::create_post(
                &s,
                hive_board::PostInput {
                    kind: kind.into(),
                    project: "one".into(),
                    area: "api".into(),
                    thread: thread.into(),
                    agent: agent.into(),
                    session: format!("s-{agent}"),
                    for_owner,
                    summary: summary.into(),
                    body: body.into(),
                    on: on.into(),
                    resolution: resolution.into(),
                    date: date.into(),
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let ask = post(
            "question",
            "limits",
            "alpha",
            true,
            "Which limit?",
            "\nPick one.\n\n",
            "",
            "",
            "2026-09-06T08:00:00Z",
        );
        let answer = post(
            "answer",
            "limits",
            "bravo",
            false,
            "The low one",
            "Low.",
            &ask.id,
            "",
            "2026-09-06T08:01:00Z",
        );
        post(
            "result",
            "ship",
            "alpha",
            false,
            "Ship it",
            "## Proposal for the human: ship the fix\n\nIt is ready.\n",
            "",
            "agreed",
            "2026-09-06T08:02:00Z",
        );
        let h = http::handler(&s);
        let data = h.snapshot("one", "").unwrap();
        let fields = POST_ROW_FIELDS.split_whitespace().collect::<BTreeSet<_>>();
        assert_eq!(data.posts.len(), 3);
        for row in &data.posts {
            assert_eq!(row.keys().collect::<BTreeSet<_>>(), fields, "{row:#?}");
        }
        assert_eq!(data.proposals.len(), 1);
        assert_eq!(
            data.proposals[0]["body"],
            "## Proposal for the human: ship the fix\n\nIt is ready."
        );
        assert_eq!(data.proposals[0]["proposal_text"], "ship the fix");

        let detail = h.post_detail(&data, &ask.id).unwrap();
        assert_eq!(detail["body"], "Pick one.");
        assert_eq!(detail["frontmatter"]["kind"], "question");
        assert_eq!(detail["frontmatter"]["for_owner"], "true");
        assert_eq!(detail["frontmatter"]["path"], format!("store:{}", ask.id));
        let responses = detail["responses"].as_array().unwrap();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["id"], answer.id);
        assert!(h.post_detail(&data, "missing").unwrap().is_empty());
        let plain = h.post_detail(&data, &answer.id).unwrap();
        assert!(plain["frontmatter"].get("for_owner").is_none());

        let (status, _, raw) = crate::test_helpers_http::request(
            h.clone_for_test(),
            "GET",
            &format!("/api/v1/projects/one/posts/{}", ask.id),
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        let served: JsonMap = serde_json::from_str(&raw).unwrap();
        let mut want = fields.clone();
        want.extend(["body", "frontmatter", "responses"]);
        assert_eq!(
            served.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            want
        );
        assert_eq!(served["body"], "Pick one.");
        assert_eq!(served["frontmatter"]["for_owner"], "true");
        assert_eq!(served["in_area"], 1);

        let (status, _, raw) = crate::test_helpers_http::request(
            h.clone_for_test(),
            "GET",
            "/api/v1/projects/one/threads/api/limits",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        let thread: JsonMap = serde_json::from_str(&raw).unwrap();
        let rows = thread["posts"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.get("body").is_none()));
        assert!(rows.iter().all(|r| r.get("frontmatter").is_none()));
    }

    #[test]
    fn thread_rows_carry_enriched_status() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        http::additions_project(&s, "one");
        join(&s, "alpha");
        join(&s, "bravo");
        let question = hive_board::create_post(
            &s,
            hive_board::PostInput {
                kind: "question".into(),
                project: "one".into(),
                area: "api".into(),
                thread: "rate-limits".into(),
                agent: "alpha".into(),
                summary: "Why 429?".into(),
                date: "2026-09-06T08:00:00Z".into(),
                ..Default::default()
            },
        )
        .unwrap();
        hive_board::create_post(
            &s,
            hive_board::PostInput {
                kind: "handoff".into(),
                project: "one".into(),
                area: "api".into(),
                thread: "rate-limits".into(),
                agent: "alpha".into(),
                summary: "Please take it".into(),
                date: "2026-09-06T08:01:00Z".into(),
                ..Default::default()
            },
        )
        .unwrap();
        hive_board::create_post(
            &s,
            hive_board::PostInput {
                kind: "claim".into(),
                project: "one".into(),
                area: "api".into(),
                thread: "rate-limits".into(),
                agent: "bravo".into(),
                summary: "On it".into(),
                expires: "2026-09-06T13:00:00Z".into(),
                date: "2026-09-06T08:02:00Z".into(),
                ..Default::default()
            },
        )
        .unwrap();
        hive_board::create_post(
            &s,
            hive_board::PostInput {
                kind: "answer".into(),
                project: "one".into(),
                area: "api".into(),
                thread: "answered".into(),
                agent: "bravo".into(),
                summary: "Named answer".into(),
                on: question.id,
                date: "2026-09-06T08:03:00Z".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let data = http::handler(&s).snapshot("one", "").unwrap();
        let thread = data.thread("api/rate-limits")["thread"]
            .as_object()
            .unwrap()
            .clone();
        let participants = thread["participants"]
            .as_array()
            .unwrap()
            .iter()
            .map(str_value)
            .collect::<Vec<_>>();
        assert_eq!(participants.join(","), "alpha,bravo");
        assert_eq!(thread["last_by"], "bravo");
        assert_eq!(thread["last_kind"], "claim");
        assert_eq!(thread["open_questions"], 0);
        assert_eq!(thread["open_handoffs"], 0);
        assert_eq!(thread["active_claims"].as_array().unwrap()[0], "bravo");
    }

    #[test]
    fn suggestion_rows_carry_jobs_and_plan_gates() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        // These suggestions name no check; the rows' shape is under test, not
        // the card-proof gate (evolve.require_check).
        s.set_setting("human", hive_store::EVOLVE_REQUIRE_CHECK, "off")
            .unwrap();
        let repo = repo();
        let project = hive_board::add_project(
            &s,
            repo.path().to_str().unwrap(),
            "One",
            "",
            false,
            "",
            "human",
            "one",
        )
        .unwrap();
        let agents = ["alpha", "bravo", "charlie", "delta"]
            .into_iter()
            .map(|n| (n.to_string(), join(&s, n)))
            .collect::<std::collections::BTreeMap<_, _>>();
        let suggest = |agent: &str, thread: &str, summary: &str, for_owner: bool| {
            hive_board::create_post(
                &s,
                hive_board::PostInput {
                    kind: "suggestion".into(),
                    project: project.slug.clone(),
                    area: "product".into(),
                    thread: thread.into(),
                    agent_row: Some(agents[agent].clone()),
                    summary: summary.into(),
                    body: "Suggestion body.".into(),
                    for_owner,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let vote = |agent: &str, area: &str, thread: &str, on_id: &str, direction: &str| {
            hive_board::create_post(
                &s,
                hive_board::PostInput {
                    kind: "vote".into(),
                    project: project.slug.clone(),
                    area: area.into(),
                    thread: thread.into(),
                    agent_row: Some(agents[agent].clone()),
                    on: on_id.into(),
                    vote: direction.into(),
                    summary: format!("Voting {direction}"),
                    body: "Vote body.".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        };
        let net_zero = suggest("alpha", "net-zero", "Use offset scans for exports", false);
        vote("bravo", "product", "net-zero", &net_zero.id, "up");
        vote("charlie", "product", "net-zero", &net_zero.id, "down");
        let plannable = suggest("bravo", "owner-cache", "Release candidate cache fix", false);
        vote("alpha", "product", "owner-cache", &plannable.id, "up");
        vote("charlie", "product", "owner-cache", &plannable.id, "up");
        let blocked = suggest(
            "charlie",
            "blocked-work",
            "Add retry budget to API calls",
            false,
        );
        vote("alpha", "product", "blocked-work", &blocked.id, "up");
        vote("bravo", "product", "blocked-work", &blocked.id, "up");
        let owner = suggest("delta", "support-window", "Choose the support window", true);
        vote("alpha", "product", "support-window", &owner.id, "up");
        vote("bravo", "product", "support-window", &owner.id, "up");
        let job = hive_board::add_job(
            &s,
            hive_board::JobInput {
                project: project.slug.clone(),
                kind: "task".into(),
                title: "Carry blocked suggestion".into(),
                source: format!("suggestion:{}", blocked.id),
                created_by: "human".into(),
                ..Default::default()
            },
        )
        .unwrap()
        .0;
        s.conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET status='blocked' WHERE id=?",
                [job.id.as_str()],
            )
            .unwrap();
        let data = http::handler(&s).snapshot(&project.slug, "").unwrap();
        let by_id = data
            .suggestions
            .iter()
            .map(|r| (str_key(r, "id"), r.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(by_id[&net_zero.id]["status"], "needs votes");
        assert_eq!(by_id[&plannable.id]["plannable"], true);
        assert_eq!(by_id[&blocked.id]["job"], job.id);
        assert_eq!(by_id[&blocked.id]["job_status"], "blocked");
        assert_eq!(
            by_id[&blocked.id]["gate"],
            format!("blocked as job {}", job.id)
        );
        assert_eq!(by_id[&owner.id]["gate"], hive_tally::GATE_OWNER);
        assert_eq!(data.overview()["suggestions_plannable"], 1);
        // The rows say whether the card names a check, and the evolution
        // payload carries the funnel the rows add up to.
        assert_eq!(by_id[&plannable.id]["checked"], false);
        let funnel = &data.evolution["funnel"];
        assert_eq!(funnel["posted"], 4, "{funnel}");
        assert_eq!(funnel["checked"], 0);
        assert_eq!(funnel["needs_votes"], 1);
        assert_eq!(funnel["eligible"], 3);
        assert_eq!(funnel["plannable"], 1);
        assert_eq!(funnel["in_flight"], 1);
        assert_eq!(funnel["blocked"], 1);
        assert_eq!(
            funnel["held"],
            json!([{"gate": "for the owner", "count": 1}])
        );
        assert_eq!(funnel["require_check"], false);
        assert_eq!(funnel["alerts"][0]["code"], "blocked");
        assert_eq!(funnel["alerts"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn a_card_with_a_check_counts_as_checked_and_a_stalled_funnel_raises_an_alert() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        let repo = repo();
        let project = hive_board::add_project(
            &s,
            repo.path().to_str().unwrap(),
            "One",
            "",
            false,
            "",
            "human",
            "one",
        )
        .unwrap();
        let agents = ["alpha", "bravo", "charlie"]
            .into_iter()
            .map(|n| (n.to_string(), join(&s, n)))
            .collect::<std::collections::BTreeMap<_, _>>();
        let suggest = |agent: &str, thread: &str, summary: &str, body: &str| {
            hive_board::create_post(
                &s,
                hive_board::PostInput {
                    kind: "suggestion".into(),
                    project: project.slug.clone(),
                    area: "product".into(),
                    thread: thread.into(),
                    agent_row: Some(agents[agent].clone()),
                    summary: summary.into(),
                    body: body.into(),
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let vote = |agent: &str, thread: &str, on_id: &str, direction: &str| {
            hive_board::create_post(
                &s,
                hive_board::PostInput {
                    kind: "vote".into(),
                    project: project.slug.clone(),
                    area: "product".into(),
                    thread: thread.into(),
                    agent_row: Some(agents[agent].clone()),
                    on: on_id.into(),
                    vote: direction.into(),
                    summary: format!("Voting {direction}"),
                    body: "Vote body.".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        };
        // Agreed, but it names no check: held by the card-proof rule.
        let unchecked = suggest(
            "alpha",
            "offset-scans",
            "Use offset scans for exports",
            "Class: product\nFile: src/export.rs\nVariable: scan\nHypothesis: faster exports\nSignal: src/export.rs",
        );
        vote("bravo", "offset-scans", &unchecked.id, "up");
        vote("charlie", "offset-scans", &unchecked.id, "up");
        // Names a check but was voted down: the check does not override a
        // net-down vote.
        let checked = suggest(
            "bravo",
            "parser-guard",
            "Empty input to the parser returns an error",
            "Class: product\nFile: src/parser.rs\nVariable: empty-input guard\nHypothesis: an empty input returns Err\nSignal: check:cargo test -p a parses",
        );
        vote("alpha", "parser-guard", &checked.id, "down");
        vote("charlie", "parser-guard", &checked.id, "down");
        let data = http::handler(&s).snapshot(&project.slug, "").unwrap();
        let by_id = data
            .suggestions
            .iter()
            .map(|r| (str_key(r, "id"), r.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(by_id[&unchecked.id]["checked"], false);
        assert_eq!(by_id[&unchecked.id]["status"], "eligible");
        assert_eq!(by_id[&unchecked.id]["gate"], hive_tally::GATE_NO_CHECK);
        assert_eq!(by_id[&checked.id]["checked"], true);
        assert_eq!(by_id[&checked.id]["status"], "rejected");
        let funnel = &data.evolution["funnel"];
        assert_eq!(funnel["posted"], 2, "{funnel}");
        assert_eq!(funnel["checked"], 1);
        assert_eq!(funnel["eligible"], 1);
        assert_eq!(funnel["plannable"], 0);
        assert_eq!(funnel["rejected"], 1);
        assert_eq!(
            funnel["held"],
            json!([{"gate": "names no check", "count": 1}])
        );
        assert_eq!(funnel["require_check"], true);
        assert_eq!(funnel["check_skips_quorum"], true);
        assert_eq!(funnel["alerts"].as_array().map(Vec::len), Some(1));
        assert_eq!(funnel["alerts"][0]["code"], "held");
        assert_eq!(
            funnel["alerts"][0]["text"],
            "Every agreed suggestion is held (1 names no check); nothing is ready to plan."
        );
    }

    #[test]
    fn owner_questions_wait_until_decided_answered_or_resolved() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        let repo = repo();
        hive_board::add_project(
            &s,
            repo.path().to_str().unwrap(),
            "One",
            "",
            false,
            "",
            "human",
            "one",
        )
        .unwrap();
        join(&s, "alpha");
        join(&s, "daemon");
        let post = |kind: &str,
                    thread: &str,
                    for_owner: bool,
                    summary: &str,
                    body: &str,
                    on: &str,
                    resolution: &str,
                    date: &str| {
            hive_board::create_post(
                &s,
                hive_board::PostInput {
                    kind: kind.into(),
                    project: "one".into(),
                    area: if thread == "loop-gate" || thread.starts_with("job-") {
                        "evolution".into()
                    } else {
                        "api".into()
                    },
                    thread: thread.into(),
                    agent: if summary == hive_store::EVOLUTION_STOP_SUMMARY {
                        "daemon".into()
                    } else {
                        "alpha".into()
                    },
                    for_owner,
                    summary: summary.into(),
                    body: body.into(),
                    on: on.into(),
                    resolution: resolution.into(),
                    date: date.into(),
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let stop = hive_store::EVOLUTION_STOP_SUMMARY;
        // A worker question is not the owner's.
        post(
            "question",
            "plain",
            false,
            "Why 429?",
            "",
            "",
            "",
            "2026-09-06T08:00:00Z",
        );
        // A marked question waits for the owner.
        let ask = post(
            "question",
            "owner",
            true,
            "Which provider pays?",
            "Pick one.\n\nOption: Stripe - cards\nOption: Paddle - merchant of record\nRecommended: Stripe - fewer moving parts",
            "",
            "",
            "2026-09-06T08:01:00Z",
        );
        // A marked question that got its answer does not.
        let done = post(
            "question",
            "settled",
            true,
            "Keep the old name?",
            "",
            "",
            "",
            "2026-09-06T08:02:00Z",
        );
        post(
            "answer",
            "settled",
            false,
            "Yes, keep it",
            "",
            &done.id,
            "",
            "2026-09-06T08:03:00Z",
        );
        // The loop-gate thread: an old decided stop closed it; the new stop still waits.
        let old = post(
            "question",
            "loop-gate",
            true,
            stop,
            "an instruction-shaped post was detected",
            "",
            "",
            "2026-09-06T08:04:00Z",
        );
        hive_board::decide(&s, &old.id, "yes", "not a command", "human").unwrap();
        // The decision's result post is dated now; the new stop comes after it.
        let after = (chrono::DateTime::parse_from_rfc3339(&s.now()).unwrap()
            + chrono::Duration::seconds(60))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let gate = post(
            "question",
            "loop-gate",
            true,
            stop,
            "experiment job-1 failed with a hard-stop marker\nThe workflow was not bypassed.",
            "",
            "",
            &after,
        );
        // A stop on a suggestion whose thread closed afterwards is moot.
        post(
            "question",
            "job-closed",
            true,
            stop,
            "evolution constraint violation: immutable path",
            "",
            "",
            "2026-09-06T08:07:00Z",
        );
        post(
            "result",
            "job-closed",
            false,
            "Closed",
            "",
            "",
            "dropped",
            "2026-09-06T08:08:00Z",
        );
        // Card-needed asks are the planner's own work since ADR-044.
        post(
            "question",
            "job-card",
            true,
            stop,
            &format!("{}; name exactly one File:", hive_evolve::CARD_NEEDED),
            "",
            "",
            "2026-09-06T08:09:00Z",
        );
        // A finding addressed to the owner waits too (the security
        // watcher's "a major upgrade: your call"); one for the agents does
        // not. A later answer on a sibling finding of the same running
        // thread settles only that sibling.
        let major = post(
            "finding",
            "dependabot",
            true,
            "next needs 17.0.0 to fix #20, a major upgrade from 16.3.8: your call",
            "Dependabot #20.\n\nYes: Upgrade it\nNo: Leave it to me\nRecommended: Yes - it fixes the alert\n\nYes queues the upgrade.",
            "",
            "",
            "2026-09-06T07:00:00Z",
        );
        let sibling = post(
            "finding",
            "dependabot",
            true,
            "Dependabot #30: no patched version of hono exists yet",
            "",
            "",
            "",
            "2026-09-06T07:01:00Z",
        );
        post(
            "answer",
            "dependabot",
            false,
            "Dependabot #30 closed on GitHub; nothing to decide",
            "",
            &sibling.id,
            "",
            "2026-09-06T07:02:00Z",
        );
        // A second open finding of the same running thread: the owner's
        // decision on the first must not hide it.
        let unfixable = post(
            "finding",
            "dependabot",
            true,
            "Dependabot #31: no patched version of node-forge exists yet",
            "Yes: Look for a way around it\nNo: Keep watching\nRecommended: No - a fix is likely soon",
            "",
            "",
            "2026-09-06T07:01:30Z",
        );
        post(
            "finding",
            "dependabot",
            false,
            "Alerts read: 3 open",
            "",
            "",
            "",
            "2026-09-06T07:03:00Z",
        );

        let h = http::handler(&s);
        let data = h.snapshot("one", "").unwrap();
        let ids = data
            .questions
            .iter()
            .map(|q| str_key(q, "id"))
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                gate.id.clone(),
                ask.id.clone(),
                unfixable.id.clone(),
                major.id.clone()
            ],
            "{:#?}",
            data.questions
        );
        assert_eq!(data.questions[0]["stop"], true);
        assert_eq!(data.questions[0]["thread_key"], "evolution/loop-gate");
        assert_eq!(data.questions[1]["stop"], false);
        assert_eq!(data.questions[1]["kind"], "question");
        assert!(
            data.questions[1]["body"]
                .as_str()
                .unwrap()
                .starts_with("Pick one.")
        );
        assert_eq!(data.questions[3]["kind"], "finding");
        assert_eq!(data.questions[3]["stop"], false);
        assert_eq!(data.questions[3]["thread_key"], "api/dependabot");

        let view = crate::read::ProjectView {
            project: s.project("one").unwrap().unwrap(),
            data: Some(std::sync::Arc::new(data)),
            error: String::new(),
            present: true,
            mark: String::new(),
        };
        // Only the stopped loop is at a hard stop (ADR-043); the other three
        // are routine asks the lead decides (TASK-129), listed but not
        // counted as waiting on the owner.
        let deciders = view
            .data
            .as_ref()
            .unwrap()
            .questions
            .iter()
            .map(|q| str_key(q, "decider"))
            .collect::<Vec<_>>();
        assert_eq!(deciders, vec!["owner", "lead", "lead", "lead"]);
        let summary = h.summary(&view);
        assert_eq!(summary["owner_questions"], 1);
        let org = h.org(std::slice::from_ref(&view));
        assert_eq!(org["totals"]["owner_questions"], 1);
        let rows = org["questions"].as_array().unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0]["slug"], "one");
        assert_eq!(rows[0]["base"], "/p/one");
        assert!(org["projects"][0].get("questions").is_none());
        let flat = crate::read::org_questions(std::slice::from_ref(&view));
        assert_eq!(flat.len(), 4);
        assert_eq!(str_key(&flat[0], "id"), gate.id);

        let (status, _, raw) = crate::test_helpers_http::request(
            h.clone_for_test(),
            "GET",
            "/api/v1/projects/one/questions",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        let listed: Vec<JsonMap> = serde_json::from_str(&raw).unwrap();
        assert_eq!(listed.len(), 4);

        hive_board::decide(&s, &ask.id, "yes", "the first one", "human").unwrap();
        hive_board::decide(&s, &gate.id, "yes", "reviewed", "human").unwrap();
        // The owner's yes on the finding queues the job that acts on it.
        let decided = hive_board::decide(&s, &major.id, "yes", "go ahead", "human").unwrap();
        let job = decided.job.expect("a yes on a finding queues its job");
        assert!(
            job.title
                .starts_with("Act on the human's yes: next needs 17.0.0")
        );
        // Its result post carries a resolution on the shared thread; the
        // sibling finding still waits.
        let data = h.snapshot("one", "").unwrap();
        let ids = data
            .questions
            .iter()
            .map(|q| str_key(q, "id"))
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![unfixable.id.clone()], "{:#?}", data.questions);
        hive_board::decide(&s, &unfixable.id, "no", "keep watching", "human").unwrap();
        let data = h.snapshot("one", "").unwrap();
        assert!(data.questions.is_empty(), "{:#?}", data.questions);
    }

    #[test]
    fn owner_questions_split_by_decider() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        let repo = repo();
        hive_board::add_project(
            &s,
            repo.path().to_str().unwrap(),
            "One",
            "",
            false,
            "",
            "human",
            "one",
        )
        .unwrap();
        join(&s, "alpha");
        let post = |kind: &str, thread: &str, summary: &str, body: &str, date: &str| {
            hive_board::create_post(
                &s,
                hive_board::PostInput {
                    kind: kind.into(),
                    project: "one".into(),
                    area: "api".into(),
                    thread: thread.into(),
                    agent: "alpha".into(),
                    for_owner: true,
                    summary: summary.into(),
                    body: body.into(),
                    on: "".into(),
                    resolution: "".into(),
                    date: date.into(),
                    ..Default::default()
                },
            )
            .unwrap()
        };
        // One of each class: an ask at a hard stop (publication) only the
        // owner decides, a routine question and a handoff the lead decides
        // whatever owner.presence says, and a security major-upgrade finding
        // a setting answers under `auto`.
        let own = post(
            "question",
            "own",
            "Put the new wording on hivefleet.dev now?",
            "Publishing the site goes live for everyone.\n\nYes: Publish it\nNo: Hold it\nRecommended: Hold it - the wording is not reviewed yet",
            "2026-09-06T08:00:00Z",
        );
        let routine = post(
            "question",
            "routine",
            "Ship the rename?",
            "Yes: Rename it\nNo: Keep it\nRecommended: Yes - nothing else uses the old name",
            "2026-09-06T08:01:00Z",
        );
        let lead = post(
            "handoff",
            "lead",
            "Run the queued cleanup?",
            "The lead's call.\n\nYes: Run it\nNo: Leave it\nRecommended: Yes - it is Tier 0",
            "2026-09-06T08:02:00Z",
        );
        let setting = post(
            "finding",
            "dependabot",
            "next needs 17.0.0 to fix #20, a major upgrade from 16.3.8: your call",
            "Dependabot #20.\n\nYes: Upgrade it\nNo: Leave it to me\nRecommended: No - the checks have not run on 17",
            "2026-09-06T08:03:00Z",
        );
        // An older major-upgrade finding names no answers; a finding that
        // only says "major upgrade" without the watcher's event is not one.
        let older = post(
            "finding",
            "dependabot-old",
            "react needs 19.0.0 to fix #7, a major upgrade from 18.3.1: your call",
            "Dependabot #7 in package-lock.json.",
            "2026-09-06T08:04:00Z",
        );
        let lookalike = post(
            "finding",
            "notes",
            "Plan the major upgrade of the build tool next quarter",
            "Nothing urgent.\n\nYes: Plan it\nNo: Leave it\nRecommended: No - not yet",
            "2026-09-06T08:05:00Z",
        );
        for (p, n) in [(&setting, 20), (&older, 7)] {
            s.event(
                "daemon",
                "security.major",
                "one",
                hive_store::Detail::json(serde_json::json!({"post": p.id, "number": n})).unwrap(),
            )
            .unwrap();
        }

        let h = http::handler(&s);
        let view_of = |data: BoardData| crate::read::ProjectView {
            project: s.project("one").unwrap().unwrap(),
            data: Some(std::sync::Arc::new(data)),
            error: String::new(),
            present: true,
            mark: String::new(),
        };

        // Nothing waits on the owner by default (TASK-129): present or
        // away, only the publication is the owner's, with the hard stop
        // named for the Sessions strip; the rest are the lead's.
        for presence in [hive_store::OWNER_PRESENT, hive_store::OWNER_AWAY] {
            s.set_setting("human", "owner.presence", presence).unwrap();
            let data = h.snapshot("one", "").unwrap();
            let by_id: std::collections::BTreeMap<String, (String, String)> = data
                .questions
                .iter()
                .map(|q| {
                    (
                        str_key(q, "id"),
                        (str_key(q, "decider"), str_key(q, "hard_stop")),
                    )
                })
                .collect();
            assert_eq!(by_id.len(), 6, "{presence}: {:#?}", data.questions);
            assert_eq!(
                by_id[&own.id],
                ("owner".to_string(), "is about publication".to_string()),
                "{presence}"
            );
            // Under `ask` a major upgrade is the owner's (PR #278 review),
            // the older one too, with the watcher's answers.
            let major = "is a major dependency upgrade (dependencies.major_updates = ask)";
            for id in [&setting.id, &older.id] {
                assert_eq!(
                    by_id[id],
                    ("owner".to_string(), major.to_string()),
                    "{presence}"
                );
            }
            for id in [&routine.id, &lead.id, &lookalike.id] {
                assert_eq!(by_id[id], ("lead".to_string(), String::new()), "{presence}");
            }
            let old_row = data
                .questions
                .iter()
                .find(|q| str_key(q, "id") == older.id)
                .unwrap();
            assert_eq!(old_row["options"][0]["label"], "Upgrade it");
            assert_eq!(old_row["options"][0]["recommended"], true);
            assert_eq!(old_row["needs_options"], "");
            assert_eq!(
                data.questions_meta["agents_deciding"]["count"],
                serde_json::json!(3),
                "{presence}"
            );
            assert_eq!(
                h.summary(&view_of(data))["owner_questions"],
                3,
                "{presence}"
            );
        }

        // Major upgrades on auto: the finding is the setting's, leaving only
        // the hard stop for the owner.
        s.set_setting("human", "dependencies.major_updates", "auto")
            .unwrap();
        let data = h.snapshot("one", "").unwrap();
        let by_id: std::collections::BTreeMap<String, String> = data
            .questions
            .iter()
            .map(|q| (str_key(q, "id"), str_key(q, "decider")))
            .collect();
        assert_eq!(by_id[&own.id], "owner");
        assert_eq!(by_id[&routine.id], "lead");
        assert_eq!(by_id[&lead.id], "lead");
        assert_eq!(by_id[&setting.id], "setting");
        assert_eq!(by_id[&older.id], "setting");
        assert_eq!(by_id[&lookalike.id], "lead");
        assert_eq!(
            data.questions_meta["agents_deciding"]["count"],
            serde_json::json!(5)
        );
        let view = view_of(data);
        assert_eq!(h.summary(&view)["owner_questions"], 1);
        let org = h.org(std::slice::from_ref(&view));
        assert_eq!(org["totals"]["owner_questions"], 1);

        // TASK-129: the owner's row (a hard stop) carries its answers and
        // the Recommended one, recommended first.
        let data = h.snapshot("one", "").unwrap();
        let row = data
            .questions
            .iter()
            .find(|q| str_key(q, "id") == own.id)
            .unwrap();
        assert_eq!(row["hard_stop"], "is about publication");
        assert_eq!(row["options"][0]["label"], "Hold it");
        assert_eq!(row["options"][0]["recommended"], true);
        assert_eq!(row["options"][1]["label"], "Publish it");
        assert_eq!(row["recommended_why"], "the wording is not reviewed yet");
        assert_eq!(row["needs_options"], "");
    }

    #[test]
    fn owner_questions_without_answers_go_to_the_lead() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        let repo = repo();
        hive_board::add_project(
            &s,
            repo.path().to_str().unwrap(),
            "One",
            "",
            false,
            "",
            "human",
            "one",
        )
        .unwrap();
        join(&s, "alpha");
        let post = |thread: &str, summary: &str, body: &str, date: &str| {
            hive_board::create_post(
                &s,
                hive_board::PostInput {
                    kind: "question".into(),
                    project: "one".into(),
                    area: "api".into(),
                    thread: thread.into(),
                    agent: "alpha".into(),
                    for_owner: true,
                    summary: summary.into(),
                    body: body.into(),
                    date: date.into(),
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let ready = post(
            "ready",
            "Publish the docs site today?",
            "Option: Publish it - the guide goes live\nOption: Hold it - it stays a draft\nRecommended: Hold it - the screenshots are stale",
            "2026-09-06T08:01:00Z",
        );
        let bare = post(
            "bare",
            "Which sandbox should the fixtures hit?",
            "Two exist; tell me which.",
            "2026-09-06T08:02:00Z",
        );
        let unmarked = post(
            "unmarked",
            "Ship it?",
            "Yes: Ship it\nNo: Wait",
            "2026-09-06T08:03:00Z",
        );
        let money = post(
            "money",
            "Raise compute.budget_credits to 500?",
            "The queue is long.",
            "2026-09-06T08:04:00Z",
        );
        let h = http::handler(&s);
        let data = h.snapshot("one", "").unwrap();
        let row = |id: &str| {
            data.questions
                .iter()
                .find(|q| str_key(q, "id") == id)
                .cloned()
                .unwrap()
        };
        // Only a hard stop that names its answers and a recommended one is
        // the owner's; the routine asks are the lead's whatever they name,
        // and a spend decision without answers too: the lead may not take
        // it (hive decide refuses), so it asks the owner again with the
        // answers written out.
        assert_eq!(str_key(&row(&ready.id), "decider"), "owner");
        assert_eq!(str_key(&row(&bare.id), "decider"), "lead");
        assert!(str_key(&row(&bare.id), "needs_options").contains("no answers"));
        assert_eq!(str_key(&row(&unmarked.id), "decider"), "lead");
        assert!(str_key(&row(&unmarked.id), "needs_options").contains("no recommended"));
        assert_eq!(str_key(&row(&money.id), "decider"), "lead");
        assert_eq!(
            data.questions_meta["agents_deciding"]["count"],
            serde_json::json!(3)
        );
    }

    #[test]
    fn org_owner_presence_and_plannable_totals() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        let h = http::handler(&s);
        let evolution =
            json!({"loop":{"loop":"","stop":"","limits":""},"archive":{"rows":[],"record":null}});
        let data = BoardData {
            meta: object(
                "repo_name branch head dirty built_at board_present",
                vec![
                    json!("one"),
                    json!("main"),
                    json!("abc123"),
                    json!("0"),
                    json!(s.now()),
                    json!("1"),
                ],
            ),
            params: JsonMap::new(),
            evolution: serde_json::from_value(evolution).unwrap(),
            suggestions: vec![
                object("status plannable", vec![json!("eligible"), json!(true)]),
                object("status plannable", vec![json!("eligible"), json!(false)]),
                object("status plannable", vec![json!("needs votes"), json!(false)]),
            ],
            ..Default::default()
        };
        let org = h.org(json!([{"project":{"slug":"one"},"data":data.overview()}]));
        assert_eq!(org["owner_presence"], hive_store::OWNER_PRESENT);
        assert_eq!(org["totals"]["votes_plannable"], 1);
    }
}
