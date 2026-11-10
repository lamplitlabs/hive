//! The self-improvement funnel: how a project's suggestions become changes,
//! stage by stage, and where they stall. Computed from the board snapshot's
//! suggestion rows (status, gate, job, checked) so the console and the plan
//! gate agree, and served as `funnel` on the project's evolution payload.
//!
//! A card is posted, names a check (or not), gathers votes until it is
//! agreed (`eligible`), waits on a plan gate until the planner may take it
//! (`plannable`), runs as an improve or experiment job (`in_flight`), and
//! ends `built`, `tried`, `carded`, `rejected`, `expired` or `closed`. The
//! alerts say, in the owner's words, why nothing moves when nothing does:
//! every agreed card is held, every card waits for votes, there are no
//! cards, or everything is settled.

use crate::server::JsonMap;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct Held {
    pub gate: String,
    pub count: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct Alert {
    pub code: String,
    pub count: i64,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct Funnel {
    pub posted: i64,
    pub checked: i64,
    pub needs_votes: i64,
    pub eligible: i64,
    pub plannable: i64,
    pub held: Vec<Held>,
    pub in_flight: i64,
    pub blocked: i64,
    pub carded: i64,
    pub built: i64,
    pub tried: i64,
    pub rejected: i64,
    pub expired: i64,
    pub closed: i64,
    pub require_check: bool,
    pub check_skips_quorum: bool,
    pub alerts: Vec<Alert>,
}

/// The funnel of one project's suggestion rows (the snapshot's, with their
/// `gate`, `plannable`, `job_status` and `checked` keys) under the store's
/// card rules.
pub(crate) fn funnel(rows: &[JsonMap], store: &pulse_store::Store) -> Funnel {
    let mut f = Funnel {
        require_check: store.setting_bool(pulse_store::EVOLVE_REQUIRE_CHECK),
        check_skips_quorum: store.setting_bool(pulse_store::EVOLVE_CHECK_SKIPS_QUORUM),
        ..Funnel::default()
    };
    let mut held: BTreeMap<String, i64> = BTreeMap::new();
    for row in rows {
        f.posted += 1;
        if row.get("checked").and_then(Value::as_bool).unwrap_or(false) {
            f.checked += 1;
        }
        let status = text(row, "status");
        match status.as_str() {
            "needs votes" => f.needs_votes += 1,
            "eligible" => f.eligible += 1,
            "carded" => f.carded += 1,
            "built" => f.built += 1,
            "tried" => f.tried += 1,
            "rejected" => f.rejected += 1,
            "expired" => f.expired += 1,
            "closed" => f.closed += 1,
            _ => {}
        }
        let job_status = text(row, "job_status");
        let in_flight = !text(row, "job").is_empty()
            && matches!(job_status.as_str(), "queued" | "leased" | "blocked");
        if in_flight {
            f.in_flight += 1;
            if job_status == "blocked" {
                f.blocked += 1;
            }
        }
        if status != "eligible" {
            continue;
        }
        if row
            .get("plannable")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            f.plannable += 1;
            continue;
        }
        let gate = text(row, "gate");
        if gate.is_empty() || in_flight {
            continue;
        }
        *held.entry(gate_label(&gate)).or_default() += 1;
    }
    let mut held: Vec<Held> = held
        .into_iter()
        .map(|(gate, count)| Held { gate, count })
        .collect();
    held.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.gate.cmp(&b.gate)));
    f.held = held;
    f.alerts = alerts(&f);
    f
}

/// The short name of a plan gate, from the gate's full wording
/// (`pulse_tally::GATE_*` and the formatted cap and duplicate gates).
pub(crate) fn gate_label(gate: &str) -> String {
    const LABELS: &[(&str, &str)] = &[
        ("net votes", "net votes"),
        ("names no check", "names no check"),
        ("addressed to the owner", "for the owner"),
        ("without evidence", "no evidence"),
        ("points at owner-only paths", "owner-only paths"),
        ("Signal path is absent", "waits for landing"),
        ("duplicate of", "duplicate"),
        ("operating-layer cap", "operating-layer cap"),
        ("loop cap", "loop cap"),
        ("check selects no test", "check selects no test"),
        ("File: names a path", "file gone"),
        ("File holds none", "file stale"),
    ];
    let gate = gate.trim();
    for (prefix, label) in LABELS {
        if gate.starts_with(prefix) {
            return (*label).to_string();
        }
    }
    let short: String = gate.chars().take(24).collect();
    short.trim_end().to_string()
}

fn alerts(f: &Funnel) -> Vec<Alert> {
    let mut out = Vec::new();
    if f.blocked > 0 {
        out.push(Alert {
            code: "blocked".into(),
            count: f.blocked,
            text: format!(
                "{} {} blocked; see the project's jobs.",
                f.blocked,
                if f.blocked == 1 {
                    "suggestion job is"
                } else {
                    "suggestion jobs are"
                }
            ),
        });
    }
    let stalled = f.plannable == 0 && f.in_flight == 0;
    if !stalled {
        return out;
    }
    if f.eligible > 0 {
        out.push(Alert {
            code: "held".into(),
            count: f.eligible,
            text: format!(
                "Every agreed suggestion is held ({}); nothing is ready to plan.",
                held_line(&f.held)
            ),
        });
    } else if f.needs_votes > 0 {
        out.push(Alert {
            code: "votes".into(),
            count: f.needs_votes,
            text: format!(
                "{} {} for votes and none is agreed; the next ballot decides.",
                f.needs_votes,
                if f.needs_votes == 1 {
                    "suggestion waits"
                } else {
                    "suggestions wait"
                }
            ),
        });
    } else if f.posted == 0 {
        out.push(Alert {
            code: "empty".into(),
            count: 0,
            text: "No suggestions yet: research posts them, agents vote, and the planner turns the agreed ones into jobs.".into(),
        });
    } else {
        out.push(Alert {
            code: "settled".into(),
            count: f.posted,
            text: format!(
                "Every suggestion is settled ({}) and nothing new is waiting.",
                settled_line(f)
            ),
        });
    }
    out
}

fn held_line(held: &[Held]) -> String {
    held.iter()
        .map(|h| format!("{} {}", h.count, h.gate))
        .collect::<Vec<_>>()
        .join(", ")
}

fn settled_line(f: &Funnel) -> String {
    let parts = [
        (f.built, "built"),
        (f.tried, "tried"),
        (f.carded, "carded"),
        (f.rejected, "turned down"),
        (f.expired, "expired"),
        (f.closed, "closed"),
    ];
    parts
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, word)| format!("{n} {word}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn text(row: &JsonMap, key: &str) -> String {
    match row.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(status: &str, gate: &str, plannable: bool, job_status: &str, checked: bool) -> JsonMap {
        let mut m = JsonMap::new();
        m.insert("status".into(), json!(status));
        m.insert("gate".into(), json!(gate));
        m.insert("plannable".into(), json!(plannable));
        m.insert(
            "job".into(),
            json!(if job_status.is_empty() { "" } else { "job-1" }),
        );
        m.insert("job_status".into(), json!(job_status));
        m.insert("checked".into(), json!(checked));
        m
    }

    fn store() -> crate::test_helpers_http::TestStore {
        crate::test_helpers_http::additions_store(true)
    }

    #[test]
    fn gate_labels_shorten_every_plan_gate() {
        assert_eq!(gate_label(pulse_tally::GATE_NET_VOTES), "net votes");
        assert_eq!(gate_label(pulse_tally::GATE_NO_CHECK), "names no check");
        assert_eq!(gate_label(pulse_tally::GATE_OWNER), "for the owner");
        assert_eq!(gate_label(pulse_tally::GATE_NO_EVIDENCE), "no evidence");
        assert_eq!(
            gate_label(pulse_tally::GATE_OWNER_PATHS),
            "owner-only paths"
        );
        assert_eq!(gate_label(pulse_tally::GATE_UNLANDED), "waits for landing");
        assert_eq!(
            gate_label(&pulse_tally::duplicate_gate("2026-x")),
            "duplicate"
        );
        assert_eq!(
            gate_label(&pulse_tally::operating_layer_cap_gate(3, 3)),
            "operating-layer cap"
        );
        assert_eq!(gate_label(&pulse_tally::loop_cap_gate(1, 1)), "loop cap");
        assert_eq!(
            gate_label(pulse_tally::GATE_NO_OP_CHECK),
            "check selects no test"
        );
        assert_eq!(gate_label(pulse_tally::GATE_FILE_GONE), "file gone");
        assert_eq!(gate_label(pulse_tally::GATE_FILE_STALE), "file stale");
        assert_eq!(
            gate_label("something else entirely, long"),
            "something else entirely,"
        );
    }

    #[test]
    fn the_funnel_counts_each_stage_and_groups_the_held_cards() {
        let s = store();
        let rows = vec![
            row("needs votes", "", false, "", false),
            row("eligible", "", true, "", true),
            row("eligible", pulse_tally::GATE_NO_CHECK, false, "", false),
            row("eligible", pulse_tally::GATE_NO_CHECK, false, "", false),
            row("eligible", pulse_tally::GATE_NET_VOTES, false, "", true),
            row("eligible", "queued as job job-1", false, "queued", true),
            row("eligible", "blocked as job job-1", false, "blocked", true),
            row("built", "", false, "", true),
            row("tried", "", false, "", false),
            row("rejected", "", false, "", false),
            row("expired", "", false, "", false),
            row("closed", "", false, "", false),
            row("carded", "", false, "", false),
        ];
        let f = funnel(&rows, &s);
        assert_eq!(f.posted, 13);
        assert_eq!(f.checked, 5);
        assert_eq!(f.needs_votes, 1);
        assert_eq!(f.eligible, 6);
        assert_eq!(f.plannable, 1);
        assert_eq!(
            f.held,
            vec![
                Held {
                    gate: "names no check".into(),
                    count: 2
                },
                Held {
                    gate: "net votes".into(),
                    count: 1
                },
            ]
        );
        assert_eq!(f.in_flight, 2);
        assert_eq!(f.blocked, 1);
        assert_eq!(
            (f.carded, f.built, f.tried, f.rejected, f.expired, f.closed),
            (1, 1, 1, 1, 1, 1)
        );
        // The defaults of the card rules.
        assert!(f.require_check);
        assert!(f.check_skips_quorum);
        // Work moves (one plannable, two in flight): only the blocked job
        // is worth a word.
        assert_eq!(f.alerts.len(), 1, "{:?}", f.alerts);
        assert_eq!(f.alerts[0].code, "blocked");
        assert_eq!(f.alerts[0].count, 1);
        assert!(f.alerts[0].text.starts_with("1 suggestion job is blocked"));
    }

    #[test]
    fn stall_alerts_name_the_stage_where_the_funnel_stopped() {
        let s = store();
        s.set_setting("human", pulse_store::EVOLVE_REQUIRE_CHECK, "off")
            .unwrap();
        // Every agreed card held.
        let f = funnel(
            &[
                row("eligible", pulse_tally::GATE_NO_CHECK, false, "", false),
                row("eligible", pulse_tally::GATE_OWNER, false, "", false),
                row("eligible", pulse_tally::GATE_NO_CHECK, false, "", false),
            ],
            &s,
        );
        assert!(!f.require_check);
        assert_eq!(f.alerts.len(), 1);
        assert_eq!(f.alerts[0].code, "held");
        assert_eq!(f.alerts[0].count, 3);
        assert_eq!(
            f.alerts[0].text,
            "Every agreed suggestion is held (2 names no check, 1 for the owner); nothing is ready to plan."
        );
        // Nothing agreed, cards waiting for votes.
        let f = funnel(
            &[
                row("needs votes", "", false, "", false),
                row("needs votes", "", false, "", true),
                row("rejected", "", false, "", false),
            ],
            &s,
        );
        assert_eq!(f.alerts.len(), 1);
        assert_eq!(f.alerts[0].code, "votes");
        assert_eq!(
            f.alerts[0].text,
            "2 suggestions wait for votes and none is agreed; the next ballot decides."
        );
        // No cards at all.
        let f = funnel(&[], &s);
        assert_eq!(f.alerts.len(), 1);
        assert_eq!(f.alerts[0].code, "empty");
        assert_eq!(f.alerts[0].count, 0);
        // Everything settled.
        let f = funnel(
            &[
                row("built", "", false, "", true),
                row("built", "", false, "", true),
                row("tried", "", false, "", false),
                row("rejected", "", false, "", false),
            ],
            &s,
        );
        assert_eq!(f.alerts.len(), 1);
        assert_eq!(f.alerts[0].code, "settled");
        assert_eq!(f.alerts[0].count, 4);
        assert_eq!(
            f.alerts[0].text,
            "Every suggestion is settled (2 built, 1 tried, 1 turned down) and nothing new is waiting."
        );
        // A plannable card means the funnel moves: no stall alert.
        let f = funnel(
            &[
                row("eligible", "", true, "", true),
                row("eligible", pulse_tally::GATE_NO_CHECK, false, "", false),
            ],
            &s,
        );
        assert!(f.alerts.is_empty(), "{:?}", f.alerts);
    }
}
