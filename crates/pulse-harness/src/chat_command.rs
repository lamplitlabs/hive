//! `pulse session` (alias `pulse chat`): the hub's sessions from the
//! command line - list them, read one, `attach` a project from inside a
//! session's own agent (the brief tells it to), and `wake` a session's
//! agent from a script it left running (the "I am done" signal of a long
//! job). The store, the wire and the hub's /chat URLs keep the older name.
use crate::conversation::{CONVERSATION_ENV, WAKE_ACTION, attach_workspace};
use anyhow::Result;
use pulse_cli::{Command, Context, FlagSet, errorf, register as cli_register, write_json};
use serde_json::json;
use std::io::Write;
use std::sync::Once;

static REGISTER: Once = Once::new();

const USAGE: &str = "pulse session list [--all] [--json]\n       pulse session show ID [--json]\n       pulse session attach SLUG [--id ID]\n       pulse session wake [--id ID] MESSAGE...";

pub fn register() {
    REGISTER.call_once(|| {
        cli_register(Command {
            name: "session".into(),
            group: "human".into(),
            summary:
                "the hub's sessions: list them, read one, attach a project from inside one, wake one's agent from a script"
                    .into(),
            usage: USAGE.into(),
            aliases: vec!["chat".into()],
            no_store: false,
            read_only: false,
            raw: false,
            run,
            flags: Some(flags),
        });
    });
}

fn flags(fs: &mut FlagSet) {
    fs.bool_var("all", false, "list archived sessions too");
    fs.string_var(
        "id",
        "",
        &format!("the session to attach or wake (default: ${CONVERSATION_ENV}, set inside a session's agent and in the shells it starts)"),
    );
}

/// The session a verb run from inside an agent's shell means: `--id`,
/// else the environment the harness sets there.
fn conversation_id(fs: &FlagSet, verb: &str) -> Result<String> {
    let mut id = fs.get_string("id");
    if id.is_empty() {
        id = std::env::var(CONVERSATION_ENV).unwrap_or_default();
    }
    if id.is_empty() {
        return Err(errorf(
            2,
            &format!("pulse session {verb}: which session?"),
            &format!("Run it from inside a session's agent (${CONVERSATION_ENV} is set there, and in the scripts it starts) or pass --id."),
            &format!("pulse session {verb} --id chat-... ..."),
        )
        .into());
    }
    Ok(id)
}

fn run(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    let Some(s) = c.store.clone() else {
        return Err(errorf(
            2,
            "pulse session: no store",
            "",
            "pulse init <project-path>",
        )
        .into());
    };
    let verb = args.first().map(String::as_str).unwrap_or("list");
    match (verb, args.len()) {
        ("list", 1) | ("list", 0) => {
            let rows = s.chat_list(fs.get_bool("all"), 200)?;
            if c.json {
                return write_json(c.out.as_mut(), &rows);
            }
            if rows.is_empty() {
                writeln!(c.out, "No sessions yet. Start one in the hub: Sessions.")?;
                return Ok(());
            }
            for r in rows {
                writeln!(
                    c.out,
                    "{}  {:<8} {:<16} {}",
                    r.id,
                    r.status,
                    if r.project.is_empty() {
                        "(no project)"
                    } else {
                        r.project.as_str()
                    },
                    r.title
                )?;
            }
            Ok(())
        }
        ("show", 2) => {
            let id = &args[1];
            let Some(row) = s.chat_get(id)? else {
                return Err(errorf(
                    2,
                    &format!("pulse session show: no session {id}"),
                    "",
                    "pulse session list --all",
                )
                .into());
            };
            let messages = s.chat_messages(id, 0)?;
            if c.json {
                return write_json(
                    c.out.as_mut(),
                    &json!({"conversation": row, "messages": messages}),
                );
            }
            writeln!(c.out, "{}  {}", row.id, row.title)?;
            writeln!(
                c.out,
                "status {}{}  project {}  branch {}  turns {}  harness session {}",
                row.status,
                if row.note.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", row.note)
                },
                if row.project.is_empty() {
                    "-"
                } else {
                    &row.project
                },
                if row.branch.is_empty() {
                    "-"
                } else {
                    &row.branch
                },
                row.turns,
                if row.session_id.is_empty() {
                    "-"
                } else {
                    &row.session_id
                },
            )?;
            if !row.cwd.is_empty() {
                writeln!(c.out, "cwd {}", row.cwd)?;
            }
            if row.lockdown {
                writeln!(
                    c.out,
                    "lockdown (sandbox): network off, writes confined to the worktree, no fetch/web_search/MCP, credentials unreadable"
                )?;
            }
            for m in messages {
                writeln!(c.out)?;
                writeln!(c.out, "[{}] {}", m.role, m.at)?;
                writeln!(c.out, "{}", m.content.trim_end())?;
            }
            Ok(())
        }
        ("wake", n) if n >= 2 => {
            let id = conversation_id(fs, "wake")?;
            let text = args[1..].join(" ");
            if text.trim().is_empty() {
                return Err(errorf(
                    2,
                    "pulse session wake: the message is empty",
                    "Say what ended and how, in one line; the agent reads the rest from your log file.",
                    "pulse session wake \"benchmark done: exit 0, results in /tmp/bench.csv\"",
                )
                .into());
            }
            let Some(row) = s.chat_get(&id)? else {
                return Err(errorf(
                    2,
                    &format!("pulse session wake: no session {id}"),
                    "",
                    "pulse session list --all",
                )
                .into());
            };
            if row.status == "archived" {
                return Err(errorf(
                    2,
                    &format!("pulse session wake: session {id} is archived"),
                    "Nothing listens there any more.",
                    "pulse session list",
                )
                .into());
            }
            let actor = crate::commands::actor_of(c);
            s.event(&actor, WAKE_ACTION, &id, json!({"text": text}))?;
            if c.json {
                return write_json(c.out.as_mut(), &json!({"id": id, "text": text}));
            }
            writeln!(
                c.out,
                "Woke session {id}: the hub gives its agent a turn on your message."
            )?;
            Ok(())
        }
        ("attach", 2) => {
            let slug = &args[1];
            let id = conversation_id(fs, "attach")?;
            let home = s.home.clone();
            let attached = attach_workspace(&s, &home, &id, slug)?;
            if c.json {
                return write_json(c.out.as_mut(), &attached);
            }
            writeln!(
                c.out,
                "Session {} now works in {} (project {}, branch {} from {}).",
                attached.id, attached.cwd, attached.project, attached.branch, attached.base
            )?;
            writeln!(
                c.out,
                "The move takes effect after this turn: finish your reply now; the next turn starts there by itself."
            )?;
            Ok(())
        }
        _ => Err(errorf(
            2,
            &format!(
                "pulse session: unknown or incomplete command {:?}",
                args.join(" ")
            ),
            "",
            USAGE,
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_cli::Buffer;
    use pulse_store::{Options, Store};

    fn store(home: &std::path::Path) -> Store {
        pulse_testutil::neutralize_runner_environment();
        Store::open(Options {
            home: home.display().to_string(),
            create: true,
            now: "2026-11-02T03:00:00Z".into(),
            ..Options::default()
        })
        .expect("store")
    }

    fn invoke(s: &Store, flags: &[(&str, &str)], args: &[&str]) -> (String, String, i32) {
        register();
        let out = Buffer::new();
        let err = Buffer::new();
        let mut c = Context {
            store: Some(s.clone()),
            home: s.home.clone(),
            out: Box::new(out.clone()),
            err: Box::new(err.clone()),
            json: false,
            quiet: false,
            color: false,
            agent: "build.sh".into(),
            project: "".into(),
            session: "".into(),
            now: "2026-11-02T03:00:00Z".into(),
            args: Vec::new(),
        };
        let cmd = pulse_cli::lookup("session").unwrap();
        let mut fs = FlagSet::new("session");
        (cmd.flags.unwrap())(&mut fs);
        for (name, value) in flags {
            fs.set(name, value).expect("flag");
        }
        let argv: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let code = match (cmd.run)(&mut c, &fs, &argv) {
            Ok(()) => 0,
            Err(e) => {
                if let Some(ce) = e.downcast_ref::<pulse_cli::Error>() {
                    let _ = write!(c.err, "{ce}");
                    ce.exit_code()
                } else {
                    let _ = write!(c.err, "{e}");
                    1
                }
            }
        };
        (out.string(), err.string(), code)
    }

    fn conversation(s: &Store, id: &str, status: &str) {
        s.chat_create(&pulse_store::Conversation {
            id: id.into(),
            title: "soak".into(),
            status: status.into(),
            created_by: "owner".into(),
            ..Default::default()
        })
        .expect("conversation");
    }

    fn wakes(s: &Store) -> Vec<pulse_bus::Event> {
        pulse_bus::since(
            s,
            0,
            pulse_bus::Filter {
                actions: vec![WAKE_ACTION.to_string()],
                ..Default::default()
            },
            0,
        )
        .expect("events")
    }

    #[test]
    fn session_is_the_name_and_chat_still_works() {
        register();
        let session = pulse_cli::lookup("session").expect("pulse session");
        let chat = pulse_cli::lookup("chat").expect("pulse chat still resolves");
        assert_eq!(session.name, "session");
        assert_eq!(chat.name, "session", "chat is an alias of session");
        assert_eq!(session.aliases, vec!["chat".to_string()]);
        assert!(session.usage.starts_with("pulse session list"));
        assert!(!session.summary.contains("conversation"));
    }

    #[test]
    fn wake_writes_the_bus_event_for_the_conversation_the_shell_belongs_to() {
        let _env = pulse_testutil::env_lock();
        let home = tempfile::tempdir().expect("tempdir");
        let s = store(home.path());
        conversation(&s, "chat-1", "idle");
        conversation(&s, "chat-2", "archived");

        // Without --id, the conversation is the one the harness named in
        // the shell's environment.
        let _guard = pulse_testutil::setenv(CONVERSATION_ENV, "chat-1");
        let (out, err, code) = invoke(&s, &[], &["wake", "build", "passed:", "212 tests ok"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.starts_with("Woke session chat-1:"), "{out}");
        let events = wakes(&s);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].subject, "chat-1");
        assert_eq!(events[0].actor, "build.sh");
        let detail: serde_json::Value = serde_json::from_str(&events[0].detail).unwrap();
        assert_eq!(detail["text"], "build passed: 212 tests ok");

        // --id wins over the environment; an archived conversation and an
        // unknown one are refused, and so is an empty message.
        let (_, err, code) = invoke(&s, &[("id", "chat-2")], &["wake", "done"]);
        assert_eq!(code, 2);
        assert!(err.contains("chat-2 is archived"), "{err}");
        let (_, err, code) = invoke(&s, &[("id", "chat-9")], &["wake", "done"]);
        assert_eq!(code, 2);
        assert!(err.contains("no session chat-9"), "{err}");
        let (_, err, code) = invoke(&s, &[], &["wake", "  "]);
        assert_eq!(code, 2);
        assert!(err.contains("the message is empty"), "{err}");
        let (_, err, code) = invoke(&s, &[], &["wake"]);
        assert_eq!(code, 2);
        assert!(err.contains("unknown or incomplete"), "{err}");
        assert_eq!(wakes(&s).len(), 1, "refusals write nothing");
    }

    #[test]
    fn wake_outside_a_conversation_says_which_one_it_needs() {
        let _env = pulse_testutil::env_lock();
        let home = tempfile::tempdir().expect("tempdir");
        let s = store(home.path());
        let _guard = pulse_testutil::setenv(CONVERSATION_ENV, "");
        let (_, err, code) = invoke(&s, &[], &["wake", "done"]);
        assert_eq!(code, 2);
        assert!(err.contains("which session?"), "{err}");
        assert!(err.contains("--id"), "{err}");
        assert!(wakes(&s).is_empty());
    }
}
