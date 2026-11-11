use std::collections::BTreeMap;

use crate::card_class::{
    CARD_CLASS_OPERATING_LAYER, CARD_CLASS_PRODUCT, card_about_loop, card_class,
};
use crate::gate::GATE_DUPLICATE;
use crate::gate::GATE_LOOP_CAP;
use crate::gate::GATE_OPERATING_LAYER_CAP;
use crate::gate::plan_gate_with;
use crate::tally::{Suggestion, TallyResult, duplicate_map, load_post};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanContext {
    pub duplicates: BTreeMap<String, String>,
    pub operating_layer_today: i32,
    pub operating_layer_cap: i32,
    pub operating_layer_cap_set: bool,
    /// The day's cap is reached, `plan.operating_layer_fill` is on, and no
    /// product card is plannable: voted operating-layer cards fill the idle
    /// slots instead of waiting for tomorrow.
    pub operating_layer_filled: bool,
    /// Improve jobs and experiments made today from cards that change the
    /// loop itself, against `plan.loop_per_day` (0: no cap). Never lifted.
    pub loop_today: i32,
    pub loop_cap: i32,
}

/// The loop cap's part of a plan context: `plan.loop_per_day` and the
/// improve jobs and experiments made today from cards about the loop itself
/// ([`card_about_loop`]), each card once however often it was retried. The
/// experiment planner gates with this part alone: the operating-layer cap
/// never held experiments.
pub fn loop_context(s: &hive_store::Store, project: &str, now: &str) -> PlanContext {
    let mut ctx = PlanContext {
        loop_cap: s.setting_int(hive_store::PLAN_LOOP_PER_DAY) as i32,
        ..PlanContext::default()
    };
    if ctx.loop_cap <= 0 || project.is_empty() || now.len() < 10 {
        return ctx;
    }
    let Ok(conn) = s.conn() else {
        return ctx;
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT DISTINCT j.source, COALESCE(p.summary, ''), COALESCE(p.body, '') FROM jobs j \
		JOIN posts p ON p.id = substr(j.source, ?) \
		WHERE j.project = ? AND j.kind IN ('improve', 'experiment') AND j.source LIKE 'suggestion:%' AND j.created_at LIKE ?",
    ) else {
        return ctx;
    };
    let rows = stmt.query_map(
        rusqlite::params![
            "suggestion:".len() as i64 + 1,
            project,
            format!("{}%", &now[..10])
        ],
        |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
    );
    if let Ok(rows) = rows {
        ctx.loop_today = rows
            .flatten()
            .filter(|(summary, body)| card_about_loop(summary, body))
            .count() as i32;
    }
    ctx
}

pub fn plan_context_for(
    s: &hive_store::Store,
    project: &str,
    now: &str,
    res: Option<&TallyResult>,
) -> PlanContext {
    let mut ctx = PlanContext {
        operating_layer_cap: s.setting_int("plan.operating_layer_per_day") as i32,
        ..loop_context(s, project, now)
    };
    ctx.operating_layer_cap_set = ctx.operating_layer_cap > 0;
    if let Some(res) = res {
        ctx.duplicates = res.duplicates.clone();
        if ctx.duplicates.is_empty() && !res.posts.is_empty() {
            ctx.duplicates = duplicate_map(&res.posts, &res.suggestions);
        }
    }
    if !ctx.operating_layer_cap_set || project.is_empty() || now.len() < 10 {
        return ctx;
    }
    let Ok(conn) = s.conn() else {
        return ctx;
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT COALESCE(p.summary, ''), COALESCE(p.body, '') FROM jobs j \
		JOIN posts p ON p.id = substr(j.source, ?) \
		WHERE j.project = ? AND j.kind = 'improve' AND j.source LIKE 'suggestion:%' AND j.created_at LIKE ?",
    ) else {
        return ctx;
    };
    let rows = stmt.query_map(
        rusqlite::params![
            "suggestion:".len() as i64 + 1,
            project,
            format!("{}%", &now[..10])
        ],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    );
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            if card_class(&row.0, &row.1) == CARD_CLASS_OPERATING_LAYER {
                ctx.operating_layer_today += 1;
            }
        }
    }
    if ctx.operating_layer_today >= ctx.operating_layer_cap
        && s.setting("plan.operating_layer_fill", "on") != "off"
        && let Some(res) = res
    {
        ctx.operating_layer_filled = !product_card_plannable(s, &ctx, &res.repo, &res.suggestions);
    }
    ctx
}

/// Whether any eligible product card passes the plan gates: the cap keeps
/// operating-layer cards waiting only while such a card competes for the slot.
fn product_card_plannable(
    s: &hive_store::Store,
    ctx: &PlanContext,
    project_path: &str,
    suggestions: &[Suggestion],
) -> bool {
    suggestions
        .iter()
        .filter(|sg| sg.status == "eligible")
        .any(|sg| {
            let Ok(post) = load_post(s, &sg.id) else {
                return false;
            };
            card_class(&post.summary, &post.body) == CARD_CLASS_PRODUCT
                && plan_gate_with(s, ctx.clone(), project_path, sg, Some(&post)).is_empty()
        })
}

pub fn duplicate_gate(id: &str) -> String {
    format!("{GATE_DUPLICATE} {id}").trim().to_string()
}

pub fn operating_layer_cap_gate(today: i32, cap: i32) -> String {
    format!(
        "{GATE_OPERATING_LAYER_CAP}: {today} of {cap} today; product cards go first (plan.operating_layer_per_day)"
    )
}

pub fn loop_cap_gate(today: i32, cap: i32) -> String {
    format!(
        "{GATE_LOOP_CAP}: {today} of {cap} today change the loop's own code; the projects' defects go first (plan.loop_per_day)"
    )
}

/// The idle-line note for a lifted cap.
pub fn operating_layer_filled_note(today: i32, cap: i32) -> String {
    format!("operating-layer cap {today} of {cap} lifted: no product card competes")
}
