//! `/api/v1/memory`: what the agents remembered across runs - the Markdown
//! files under `~/.pulse/memory/` that [`pulse_harness::memory`] reads into
//! every run's system prompt. The owner sees each file as a list of dated
//! entries (one per project, one for the user that every project reads, one
//! shared by conversations without a project), can add a fact the way an
//! agent's `memory remember` does, and can forget an entry that no longer
//! holds. The files stay plain text the owner may also edit by hand.

use crate::additions::{addition_failure, addition_method, addition_not_found, addition_string};
use crate::httpx::{Request, ResponseWriter};
use crate::server::{Handler, read_payload, write_error, write_json};
use pulse_harness::memory::{
    MEMORY_DIR, MEMORY_FILE_LIMIT, MEMORY_TEXT_LIMIT, Memory, Scope, USER_FILE, split_date,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

impl Handler {
    pub(crate) fn addition_memory(&self, w: &mut ResponseWriter, r: &Request, parts: Vec<String>) {
        match parts.as_slice() {
            [_] => {
                if !addition_method(w, r, vec!["GET".to_string(), "POST".to_string()]) {
                    return;
                }
                if r.method == "POST" {
                    self.memory_remember(w, r);
                } else {
                    self.memory_list(w);
                }
            }
            [_, action] if action == "forget" => {
                if addition_method(w, r, vec!["POST".to_string()]) {
                    self.memory_forget(w, r);
                }
            }
            _ => addition_not_found(w),
        }
    }

    fn memory_list(&self, w: &mut ResponseWriter) {
        let Some(store) = self.store.as_deref() else {
            store_missing(w);
            return;
        };
        let projects = match store.projects(true) {
            Ok(p) => p,
            Err(e) => {
                addition_failure(w, e);
                return;
            }
        };
        let on = store.setting("harness.memory", "on") != "off";
        write_json(w, 200, memory_payload(&store.home, &projects, on));
    }

    fn memory_remember(&self, w: &mut ResponseWriter, r: &Request) {
        let (payload, ok) = read_payload(w, r);
        if !ok {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            store_missing(w);
            return;
        };
        let (text, ok) = addition_string(w, &payload, "text", true);
        if !ok {
            return;
        }
        let Some((memory, scope)) = self.memory_target(w, &payload) else {
            return;
        };
        match memory.remember(scope, &text) {
            Ok(change) => {
                let _ = store.event(
                    pulse_store::HUMAN_AGENT,
                    "memory.remembered",
                    memory.key(),
                    json!({"scope": scope.as_str(), "file": memory.shown_path(scope), "count": change.count, "from": "hub"}),
                );
                write_json(
                    w,
                    201,
                    json!({"file": file_json(&memory, scope, memory.key(), file_kind(memory.key()), ""), "count": change.count}),
                );
            }
            Err(e) => memory_failure(w, e),
        }
    }

    fn memory_forget(&self, w: &mut ResponseWriter, r: &Request) {
        let (payload, ok) = read_payload(w, r);
        if !ok {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            store_missing(w);
            return;
        };
        let (text, ok) = addition_string(w, &payload, "text", true);
        if !ok {
            return;
        }
        let Some((memory, scope)) = self.memory_target(w, &payload) else {
            return;
        };
        match memory.forget(Some(scope), &text) {
            Ok(change) => {
                let _ = store.event(
                    pulse_store::HUMAN_AGENT,
                    "memory.forgotten",
                    memory.key(),
                    json!({"scope": scope.as_str(), "file": memory.shown_path(scope), "count": change.count, "from": "hub"}),
                );
                write_json(
                    w,
                    200,
                    json!({"file": file_json(&memory, scope, memory.key(), file_kind(memory.key()), ""), "count": change.count}),
                );
            }
            Err(e) => memory_failure(w, e),
        }
    }

    /// The file a write names: `{"scope":"user"}` is the user's file;
    /// `{"key":"demo-app"}` (or `"scope":"project"` with a key) is a
    /// project's or the conversations' file. A project key must be one the
    /// list shows, so a typo cannot start a stray file.
    fn memory_target(
        &self,
        w: &mut ResponseWriter,
        payload: &crate::additions::JsonMap,
    ) -> Option<(Memory, Scope)> {
        let store = self.store.as_deref()?;
        let (scope_text, ok) = addition_string(w, payload, "scope", false);
        if !ok {
            return None;
        }
        let (key, ok) = addition_string(w, payload, "key", false);
        if !ok {
            return None;
        }
        let key = key.trim().to_string();
        let scope = if scope_text.trim().is_empty() {
            if key.is_empty() || key == "user" {
                Scope::User
            } else {
                Scope::Project
            }
        } else {
            match Scope::parse(&scope_text) {
                Ok(s) => s,
                Err(e) => {
                    write_error(
                        w,
                        400,
                        "Invalid scope".to_string(),
                        format!("{e}."),
                        "Send scope \"user\" or \"project\" (with the project's key).".to_string(),
                    );
                    return None;
                }
            }
        };
        if scope == Scope::User {
            return Some((Memory::new(&store.home, "user"), Scope::User));
        }
        if key.is_empty() {
            write_error(
                w,
                400,
                "A key is required".to_string(),
                "A project memory needs the key of the file (the project's slug).".to_string(),
                "Send key: \"<slug>\" or scope: \"user\".".to_string(),
            );
            return None;
        }
        let projects = match store.projects(true) {
            Ok(p) => p,
            Err(e) => {
                addition_failure(w, e);
                return None;
            }
        };
        let known = projects.iter().any(|p| memory_key(&p.slug) == key)
            || key == pulse_harness::conversation::SHARED_MEMORY_KEY
            || Memory::new(&store.home, &key)
                .path(Scope::Project)
                .is_file();
        if !known {
            write_error(
                w,
                404,
                format!("No memory {key}"),
                "No project or memory file goes by that key.".to_string(),
                "Pick a file the memory list shows.".to_string(),
            );
            return None;
        }
        Some((Memory::new(&store.home, &key), Scope::Project))
    }
}

/// The key a registered project's memory file is named by: its slug, the
/// way the harness slugs it ([`pulse_harness::memory::project_key`] with a
/// project given).
fn memory_key(slug: &str) -> String {
    pulse_harness::memory::slug(slug)
}

/// Which kind of file a key names: the user's, the conversations' shared
/// one, or a project's.
fn file_kind(key: &str) -> &'static str {
    if key == "user" {
        "user"
    } else if key == pulse_harness::conversation::SHARED_MEMORY_KEY {
        "conversations"
    } else {
        "project"
    }
}

/// Every memory file: the user's first, then the conversations' shared one,
/// then one per registered project (even when empty, so the owner can add
/// to it), then any other file in the directory (a project since removed,
/// a key from before the project was registered).
pub(crate) fn memory_payload(home: &str, projects: &[pulse_store::Project], on: bool) -> Value {
    let dir = Path::new(home).join(MEMORY_DIR);
    let mut files = Vec::new();
    let user = Memory::new(home, "user");
    files.push(file_json(&user, Scope::User, "user", "user", ""));
    let shared = pulse_harness::conversation::SHARED_MEMORY_KEY;
    files.push(file_json(
        &Memory::new(home, shared),
        Scope::Project,
        shared,
        "conversations",
        "",
    ));
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    seen.insert(USER_FILE.trim_end_matches(".md").to_string(), ());
    seen.insert(shared.to_string(), ());
    for p in projects {
        let key = memory_key(&p.slug);
        if seen.insert(key.clone(), ()).is_some() {
            continue;
        }
        files.push(file_json(
            &Memory::new(home, &key),
            Scope::Project,
            &key,
            "project",
            &p.slug,
        ));
    }
    let mut extra: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.strip_suffix(".md").map(str::to_string)
                })
                .filter(|key| !seen.contains_key(key))
                .collect()
        })
        .unwrap_or_default();
    extra.sort();
    for key in extra {
        files.push(file_json(
            &Memory::new(home, &key),
            Scope::Project,
            &key,
            "project",
            "",
        ));
    }
    let total: usize = files
        .iter()
        .filter_map(|f| f["entries"].as_array().map(Vec::len))
        .sum();
    json!({
        "on": on,
        "dir": tilde(&dir),
        "limits": {"text": MEMORY_TEXT_LIMIT, "file": MEMORY_FILE_LIMIT},
        "total": total,
        "files": files,
    })
}

/// One file: its key, kind, project (when registered), path, size, when it
/// last changed, and its entries oldest first with the day each was kept.
fn file_json(memory: &Memory, scope: Scope, key: &str, kind: &str, project: &str) -> Value {
    let path = memory.path(scope);
    let meta = std::fs::metadata(&path).ok();
    let size = meta.as_ref().map_or(0, |m| m.len());
    let updated_at = meta
        .and_then(|m| m.modified().ok())
        .map(|t| {
            chrono::DateTime::<chrono::Utc>::from(t)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .unwrap_or_default();
    let entries: Vec<Value> = memory
        .entries(scope)
        .iter()
        .map(|e| {
            let (day, text) = split_date(e);
            json!({"day": day.unwrap_or(""), "text": text, "line": e})
        })
        .collect();
    json!({
        "key": key,
        "kind": kind,
        "project": project,
        "scope": scope.as_str(),
        "path": memory.shown_path(scope),
        "exists": path.is_file(),
        "size": size,
        "updated_at": updated_at,
        "entries": entries,
    })
}

fn tilde(path: &Path) -> String {
    let shown = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && shown.starts_with(&home) => {
            format!("~{}", &shown[home.len()..])
        }
        _ => shown,
    }
}

fn store_missing(w: &mut ResponseWriter) {
    write_error(
        w,
        503,
        "The store is unavailable".to_string(),
        "The hub has no open PULSE store.".to_string(),
        "Run pulse init, then restart pulse hub.".to_string(),
    );
}

/// The memory module's own refusals as what / why / fix: a memory too long
/// or empty, a full file, an ambiguous or missing forget.
fn memory_failure(w: &mut ResponseWriter, err: String) {
    let what = err.strip_prefix("memory: ").unwrap_or(&err).to_string();
    let (code, why, fix) = if what.contains("too long for one memory") {
        (
            400,
            "A memory is one fact, not an essay.".to_string(),
            format!("Keep it under {MEMORY_TEXT_LIMIT} characters."),
        )
    } else if what.contains("nothing to remember") || what.contains("nothing to forget") {
        (
            400,
            "The text was empty.".to_string(),
            "Write the fact to keep, or enough of the one to forget.".to_string(),
        )
    } else if what.contains("is full") {
        (
            409,
            "The file has reached its size limit.".to_string(),
            "Forget what no longer holds before adding more.".to_string(),
        )
    } else if what.contains("nothing remembered contains") {
        (
            404,
            "No entry in that file contains the text.".to_string(),
            "Reload the page; another run may have forgotten it already.".to_string(),
        )
    } else if what.contains("memories contain") {
        (
            409,
            "More than one entry matches, so none was removed.".to_string(),
            "Give more of the one to forget.".to_string(),
        )
    } else {
        (
            500,
            "The memory file could not be written.".to_string(),
            "Check the file's permissions and try again.".to_string(),
        )
    };
    write_error(w, code, what, why, fix);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers_http as http;

    fn call(
        s: &pulse_store::Store,
        method: &str,
        path: &str,
        body: &str,
    ) -> (u16, serde_json::Map<String, Value>) {
        http::additions_request(http::handler(s), method, path, body)
    }

    #[test]
    fn memory_route_lists_every_file_and_writes_the_way_the_tool_does() {
        let s = http::additions_store(false);
        http::additions_project(&s, "demo-app");
        let dir = Path::new(&s.home).join(MEMORY_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("demo-app.md"),
            "# Memory for demo-app\n\n- [2026-01-02] tests need cargo test -p app\n- hand written, no date\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("old-project.md"),
            "- [2026-03-04] gone but remembered\n",
        )
        .unwrap();

        let (code, v) = call(&s, "GET", "/api/v1/memory", "");
        assert_eq!(code, 200, "{v:?}");
        assert_eq!(v["on"], true);
        assert_eq!(v["total"], 3);
        assert!(v["dir"].as_str().unwrap().ends_with("/memory"), "{v:?}");
        assert_eq!(v["limits"]["text"], MEMORY_TEXT_LIMIT);
        let files = v["files"].as_array().unwrap();
        let keys: Vec<&str> = files.iter().map(|f| f["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["user", "conversations", "demo-app", "old-project"]);
        let kinds: Vec<&str> = files.iter().map(|f| f["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["user", "conversations", "project", "project"]);
        assert_eq!(files[0]["exists"], false);
        assert_eq!(files[0]["entries"].as_array().unwrap().len(), 0);
        let demo = &files[2];
        assert_eq!(demo["project"], "demo-app");
        assert_eq!(demo["exists"], true);
        assert!(
            demo["path"]
                .as_str()
                .unwrap()
                .ends_with("/memory/demo-app.md")
        );
        assert!(demo["size"].as_u64().unwrap() > 0);
        assert!(
            demo["updated_at"].as_str().unwrap().ends_with('Z'),
            "{demo:?}"
        );
        let entries = demo["entries"].as_array().unwrap();
        assert_eq!(entries[0]["day"], "2026-01-02");
        assert_eq!(entries[0]["text"], "tests need cargo test -p app");
        assert_eq!(
            entries[0]["line"],
            "[2026-01-02] tests need cargo test -p app"
        );
        assert_eq!(entries[1]["day"], "");
        assert_eq!(entries[1]["text"], "hand written, no date");
        assert_eq!(files[3]["project"], "");

        // Remember for the user: the file appears with today's date on the entry.
        let (code, v) = call(
            &s,
            "POST",
            "/api/v1/memory",
            r#"{"scope":"user","text":"  prefers  short replies "}"#,
        );
        assert_eq!(code, 201, "{v:?}");
        assert_eq!(v["count"], 1);
        assert_eq!(v["file"]["key"], "user");
        let e = &v["file"]["entries"][0];
        assert_eq!(e["text"], "prefers short replies");
        assert_eq!(e["day"].as_str().unwrap().len(), 10);
        assert!(Path::new(&s.home).join("memory/user.md").is_file());

        // Remember for a project by key; the same text twice is kept once.
        for _ in 0..2 {
            let (code, v) = call(
                &s,
                "POST",
                "/api/v1/memory",
                r#"{"key":"demo-app","text":"the API lives in crates/api"}"#,
            );
            assert_eq!(code, 201, "{v:?}");
            assert_eq!(v["count"], 3);
        }

        // Forget one by part of its text, scoped to the file.
        let (code, v) = call(
            &s,
            "POST",
            "/api/v1/memory/forget",
            r#"{"key":"demo-app","text":"hand written"}"#,
        );
        assert_eq!(code, 200, "{v:?}");
        assert_eq!(v["count"], 2);
        let texts: Vec<&str> = v["file"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["text"].as_str().unwrap())
            .collect();
        assert_eq!(
            texts,
            [
                "tests need cargo test -p app",
                "the API lives in crates/api"
            ]
        );

        // The writes leave events the feed can show.
        let (code, v) = call(&s, "GET", "/api/v1/events?limit=10", "");
        assert_eq!(code, 200, "{v:?}");
        let actions: Vec<&str> = v["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["action"].as_str().unwrap())
            .collect();
        assert!(actions.contains(&"memory.remembered"), "{actions:?}");
        assert!(actions.contains(&"memory.forgotten"), "{actions:?}");
    }

    #[test]
    fn memory_route_refuses_bad_writes_with_what_why_fix() {
        let s = http::additions_store(false);
        http::additions_project(&s, "demo-app");
        let (code, v) = call(
            &s,
            "POST",
            "/api/v1/memory",
            r#"{"scope":"user","text":"  "}"#,
        );
        assert_eq!(code, 400, "{v:?}");
        assert_eq!(v["error"], "Invalid text");

        let long = "x".repeat(MEMORY_TEXT_LIMIT + 1);
        let (code, v) = call(
            &s,
            "POST",
            "/api/v1/memory",
            &format!(r#"{{"scope":"user","text":"{long}"}}"#),
        );
        assert_eq!(code, 400, "{v:?}");
        assert!(v["error"].as_str().unwrap().contains("too long"), "{v:?}");
        assert!(
            v["fix"]
                .as_str()
                .unwrap()
                .contains(&MEMORY_TEXT_LIMIT.to_string())
        );

        let (code, v) = call(&s, "POST", "/api/v1/memory", r#"{"key":"nope","text":"x"}"#);
        assert_eq!(code, 404, "{v:?}");
        assert_eq!(v["error"], "No memory nope");

        let (code, v) = call(
            &s,
            "POST",
            "/api/v1/memory",
            r#"{"scope":"project","text":"x"}"#,
        );
        assert_eq!(code, 400, "{v:?}");
        assert_eq!(v["error"], "A key is required");

        let (code, v) = call(
            &s,
            "POST",
            "/api/v1/memory",
            r#"{"scope":"team","text":"x"}"#,
        );
        assert_eq!(code, 400, "{v:?}");
        assert_eq!(v["error"], "Invalid scope");

        // Forgetting what is not there, and what matches twice.
        let (code, v) = call(
            &s,
            "POST",
            "/api/v1/memory/forget",
            r#"{"key":"demo-app","text":"never kept"}"#,
        );
        assert_eq!(code, 404, "{v:?}");
        for text in ["alpha one", "alpha two"] {
            let body = format!(r#"{{"key":"demo-app","text":"{text}"}}"#);
            let (code, _) = call(&s, "POST", "/api/v1/memory", &body);
            assert_eq!(code, 201);
        }
        let (code, v) = call(
            &s,
            "POST",
            "/api/v1/memory/forget",
            r#"{"key":"demo-app","text":"alpha"}"#,
        );
        assert_eq!(code, 409, "{v:?}");
        assert!(
            v["error"].as_str().unwrap().contains("2 memories contain"),
            "{v:?}"
        );

        let (code, v) = call(&s, "GET", "/api/v1/memory/forget", "");
        assert_eq!(code, 405, "{v:?}");
        let (code, _) = call(&s, "GET", "/api/v1/memory/nope", "");
        assert_eq!(code, 404);

        // The conversations' shared file is a valid target before it exists.
        let (code, v) = call(
            &s,
            "POST",
            "/api/v1/memory",
            r#"{"key":"conversations","text":"one for every chat"}"#,
        );
        assert_eq!(code, 201, "{v:?}");
        assert_eq!(v["file"]["kind"], "conversations");

        // The setting off is reported, the files still listed.
        s.set_setting(pulse_store::HUMAN_AGENT, "harness.memory", "off")
            .unwrap();
        let (code, v) = call(&s, "GET", "/api/v1/memory", "");
        assert_eq!(code, 200, "{v:?}");
        assert_eq!(v["on"], false);
    }
}
