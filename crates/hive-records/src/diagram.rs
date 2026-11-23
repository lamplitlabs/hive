//! Diagrams drawn in text, and what a decision record needs before the owner
//! reads it.
//!
//! A record lives in the store and has nowhere to keep a picture file, so a
//! decision record shows its change as a diagram drawn with box-drawing
//! characters inside a fenced block (`TPL-adr`, `WF-decision`). The console,
//! the site and GitHub all show such a block as it was written.
//! `web/site/src/lib/diagram.ts` holds the same rules for the pages under
//! `docs/`; keep the two in step.

/// The widest a diagram may be: the console's reading panel shows that many
/// characters of a code block without scrolling sideways.
pub const DIAGRAM_MAX_COLUMNS: usize = 72;

/// The words `TPL-adr` puts before its example diagram. A record that still
/// holds them shows the owner the template's drawing, which is about
/// another change.
pub const TEMPLATE_EXAMPLE_MARK: &str = "Replace this example";

/// One diagram of a Markdown text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagram {
    /// The line its fence opens on, 1-based.
    pub line: usize,
    pub rows: usize,
    pub columns: usize,
    /// What is badly drawn; empty when it is well drawn.
    pub problems: Vec<String>,
}

/// Which ways a line character reaches: up, down, left, right.
fn arms(ch: char) -> &'static str {
    match ch {
        '─' => "lr",
        '│' => "ud",
        '┌' => "dr",
        '┐' => "dl",
        '└' => "ur",
        '┘' => "ul",
        '├' => "udr",
        '┤' => "udl",
        '┬' => "dlr",
        '┴' => "ulr",
        '┼' => "udlr",
        _ => "",
    }
}

fn box_drawing(ch: char) -> bool {
    ('\u{2500}'..='\u{257F}').contains(&ch)
}

/// Whether a fenced block is a diagram: plain text (no language, or `text`)
/// that holds a box-drawing character.
pub fn is_diagram(text: &str, lang: &str) -> bool {
    (lang.is_empty() || lang == "text") && text.chars().any(box_drawing)
}

/// What is wrong with a diagram, one line per defect.
///
/// No line is wider than `max` columns, and the drawing uses only ASCII, the
/// eleven line characters of [`arms`] and the four arrows: those are the
/// characters the fonts of the console and the site hold at one width. A
/// drawing with a box in it (it holds a `┌`) is also checked for lines that
/// do not meet: a line that goes up or down meets a line, a corner or an
/// arrowhead in the same column of the next row, and a corner or a junction
/// meets a line on each side it reaches. A plain `─` may end at a space, so
/// a label can sit on a line. A listing of folders has no box and is left
/// alone.
pub fn check_diagram(text: &str, max: usize) -> Vec<String> {
    let rows: Vec<Vec<char>> = text
        .trim_end_matches('\n')
        .split('\n')
        .map(|line| line.chars().collect())
        .collect();
    let boxes = text.contains('┌');
    let at = |r: usize, c: usize, dr: isize, dc: isize| -> Option<char> {
        let r = r.checked_add_signed(dr)?;
        let c = c.checked_add_signed(dc)?;
        rows.get(r)?.get(c).copied()
    };
    let meets = |ch: Option<char>, way: char, head: char| {
        ch.is_some_and(|ch| ch == head || arms(ch).contains(way))
    };
    let mut problems = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        if row.len() > max {
            problems.push(format!(
                "line {} is {} columns wide; a diagram is at most {max}",
                r + 1,
                row.len()
            ));
        }
        for (c, &ch) in row.iter().enumerate() {
            let place = format!("line {}, column {}", r + 1, c + 1);
            let plain = (' '..='~').contains(&ch);
            let reach = arms(ch);
            if !plain && reach.is_empty() && !"→←↑↓".contains(ch) {
                let shown = if ch == '\t' {
                    "a tab".to_string()
                } else {
                    ch.to_string()
                };
                problems.push(format!(
                    "{place}: {shown} is not a character a diagram is drawn with (ASCII, ─ │ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼ and → ← ↑ ↓)"
                ));
                continue;
            }
            if reach.is_empty() || !boxes {
                continue;
            }
            if reach.contains('u') && !meets(at(r, c, -1, 0), 'd', '↑') {
                problems.push(format!(
                    "{place}: {ch} reaches up, and the row above has no line or arrowhead in that column"
                ));
            }
            if reach.contains('d') && !meets(at(r, c, 1, 0), 'u', '↓') {
                problems.push(format!(
                    "{place}: {ch} reaches down, and the row below has no line or arrowhead in that column"
                ));
            }
            if ch == '─' {
                continue;
            }
            if reach.contains('l') && !meets(at(r, c, 0, -1), 'r', '←') {
                problems.push(format!(
                    "{place}: {ch} reaches left, and nothing meets it there"
                ));
            }
            if reach.contains('r') && !meets(at(r, c, 0, 1), 'l', '→') {
                problems.push(format!(
                    "{place}: {ch} reaches right, and nothing meets it there"
                ));
            }
        }
    }
    problems
}

/// A line that opens a fenced block: its indentation, the fence character,
/// how many of it, and the language word after it.
fn fence_open(line: &str) -> Option<(&str, char, usize, &str)> {
    let rest = line.trim_start();
    let indent = &line[..line.len() - rest.len()];
    let mark = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = rest.chars().take_while(|c| *c == mark).count();
    if run < 3 {
        return None;
    }
    let info = rest[run..].trim_start();
    if info.contains('`') {
        return None;
    }
    let lang = info.split_whitespace().next().unwrap_or("");
    Some((indent, mark, run, lang))
}

fn fence_close(line: &str, mark: char, run: usize) -> bool {
    let rest = line.trim_start();
    let n = rest.chars().take_while(|c| *c == mark).count();
    n >= run && rest[n..].trim().is_empty()
}

/// Every diagram of a Markdown text: its fenced blocks (three or more
/// backticks or tildes, indented or not) that are diagrams. The indentation
/// of a block inside a list item is not part of the drawing. A fence left
/// open runs to the end of the text.
pub fn find_diagrams(markdown: &str, max: usize) -> Vec<Diagram> {
    let text = markdown.trim_start_matches('\u{FEFF}');
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((indent, mark, run, lang)) = fence_open(lines[i]) else {
            i += 1;
            continue;
        };
        let mut body: Vec<&str> = Vec::new();
        let mut j = i + 1;
        while j < lines.len() && !fence_close(lines[j], mark, run) {
            body.push(
                lines[j]
                    .strip_prefix(indent)
                    .unwrap_or_else(|| lines[j].trim_start()),
            );
            j += 1;
        }
        let drawing = body.join("\n");
        if is_diagram(&drawing, lang) {
            found.push(Diagram {
                line: i + 1,
                rows: body.len(),
                columns: body
                    .iter()
                    .map(|row| row.chars().count())
                    .max()
                    .unwrap_or(0),
                problems: check_diagram(&drawing, max),
            });
        }
        i = j + 1;
    }
    found
}

/// What stands between a proposed decision record and an owner who has not
/// seen the code: no `At a glance` section, no diagram, the template's
/// example left in place of one, or a diagram that is badly drawn. Empty
/// when the body has all of it. `hive record new adr`, `amend` and `draft`
/// print these as a note; they never refuse the write.
pub fn adr_reading_notes(body: &str) -> Vec<String> {
    let mut notes = Vec::new();
    let glance = body.lines().any(|line| {
        let line = line.trim();
        line.starts_with('#')
            && line
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case("At a glance")
    });
    if !glance {
        notes.push(
            "it has no `## At a glance` section: open with the decision asked for as a yes-or-no question, what happens today, what would happen instead, what the owner will notice, the cost and risk, and how to undo it"
                .to_string(),
        );
    }
    let diagrams = find_diagrams(body, DIAGRAM_MAX_COLUMNS);
    if diagrams.is_empty() {
        notes.push(format!(
            "it has no diagram: draw the change in a fenced `text` block, at most {DIAGRAM_MAX_COLUMNS} characters wide, with ASCII and ─ │ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼ → ← ↑ ↓ (today beside the proposal, or the flow with the new part marked)"
        ));
    }
    if body.contains(TEMPLATE_EXAMPLE_MARK) {
        notes.push(
            "it still holds the template's example diagram and the words above it: draw this decision's own change and remove them"
                .to_string(),
        );
    }
    for d in &diagrams {
        for p in &d.problems {
            notes.push(format!(
                "the diagram at line {} is badly drawn: {p}",
                d.line
            ));
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "\
┌────────┐  task   ┌────────┐
│ owner  │ ──────→ │ lead   │
└────────┘         └───┬────┘
                       │ jobs
                       ↓
                  ┌─────────┐
                  │ agents  │
                  └─────────┘";

    fn check(text: &str) -> Vec<String> {
        check_diagram(text, DIAGRAM_MAX_COLUMNS)
    }

    #[test]
    fn a_plain_block_with_a_line_character_is_a_diagram() {
        assert!(is_diagram(GOOD, ""));
        assert!(is_diagram(GOOD, "text"));
        assert!(!is_diagram("hive status\n", ""));
        assert!(
            !is_diagram("a → b\n", "text"),
            "an arrow alone is sample output"
        );
        assert!(!is_diagram(GOOD, "bash"), "a block with a language is code");
    }

    #[test]
    fn a_well_drawn_diagram_has_no_problems() {
        assert_eq!(check(GOOD), Vec::<String>::new());
        assert_eq!(check(&format!("{GOOD}\n\n")), Vec::<String>::new());
        assert_eq!(
            check("task ── planned ──→ jobs → pull request\n  ↓\n done"),
            Vec::<String>::new(),
            "a label may sit on a line"
        );
        let tree = ".\n├── crates\n│   └── hive-records\n└── docs";
        assert_eq!(
            check(tree),
            Vec::<String>::new(),
            "a listing has no box, so its lines may start under a name"
        );
    }

    #[test]
    fn a_line_that_drifts_or_a_box_left_open_is_named_with_its_place() {
        let drifted = GOOD.replace(
            "                       │ jobs",
            "                      │ jobs",
        );
        let problems = check(&drifted);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert_eq!(
            problems[0],
            "line 3, column 24: ┬ reaches down, and the row below has no line or arrowhead in that column"
        );
        assert!(problems[1].starts_with("line 4, column 23: │ reaches up"));
        assert_eq!(
            check("┌─────┐\n│ job │\n└─────"),
            [
                "line 2, column 7: │ reaches down, and the row below has no line or arrowhead in that column"
            ]
        );
        assert_eq!(
            check("┌──── ┐\n│ job │\n└─────┘"),
            ["line 1, column 7: ┐ reaches left, and nothing meets it there"]
        );
    }

    #[test]
    fn a_wide_line_and_a_character_the_fonts_lack_are_named() {
        let wide = format!("┌{0}┐\n└{0}┘", "─".repeat(DIAGRAM_MAX_COLUMNS));
        assert_eq!(
            check(&wide),
            [
                "line 1 is 74 columns wide; a diagram is at most 72",
                "line 2 is 74 columns wide; a diagram is at most 72"
            ]
        );
        assert!(
            check("── merge ─► main")[0]
                .starts_with("line 1, column 11: ► is not a character a diagram is drawn with")
        );
        assert!(check("─\tx")[0].starts_with("line 1, column 2: a tab is not a character"));
        assert!(check("╔═╗")[0].starts_with("line 1, column 1: ╔ is not a character"));
    }

    #[test]
    fn the_diagrams_of_a_markdown_text_are_found_by_the_line_their_fence_opens_on() {
        let mut md = vec!["---", "id: ADR-900", "---", "", "# T", "", "```text"];
        md.extend(GOOD.split('\n'));
        md.extend([
            "```",
            "",
            "```bash",
            "echo \"│\"",
            "```",
            "",
            "```",
            "hive status",
            "  → 3 running",
            "```",
            "",
            "1. A step:",
            "",
            "   ~~~",
            "   ┌──┐",
            "   │  │",
            "   └──",
            "   ~~~",
            "",
        ]);
        let found = find_diagrams(&md.join("\n"), DIAGRAM_MAX_COLUMNS);
        let shape: Vec<_> = found
            .iter()
            .map(|d| (d.line, d.rows, d.columns, d.problems.len()))
            .collect();
        assert_eq!(shape, [(7, 8, 29, 0), (29, 3, 4, 1)]);
        assert!(
            found[1].problems[0].starts_with("line 2, column 4: │ reaches down"),
            "the list's indentation is not part of the drawing: {:?}",
            found[1].problems
        );
        assert!(find_diagrams("No fence here ─ only prose.\n", DIAGRAM_MAX_COLUMNS).is_empty());
        let open = find_diagrams("```text\r\n┌─┐\r\n└─┘\r\n", DIAGRAM_MAX_COLUMNS);
        assert_eq!(
            open,
            [Diagram {
                line: 1,
                rows: 3,
                columns: 3,
                problems: vec![]
            }],
            "a fence left open runs to the end, and a CR is not a column"
        );
    }

    #[test]
    fn a_decision_record_is_told_what_the_owner_would_miss() {
        let bare =
            "# ADR-900: Cache the index\n\n## Context\n\nSlow.\n\n## Decision\n\nCache it.\n";
        let notes = adr_reading_notes(bare);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].starts_with("it has no `## At a glance` section"));
        assert!(notes[1].starts_with("it has no diagram"));

        let drawn = format!(
            "# ADR-900: Cache the index\n\n## At a glance\n\n- **Decide:** cache it?\n\n```text\n{GOOD}\n```\n\n## Context\n\nSlow.\n"
        );
        assert_eq!(adr_reading_notes(&drawn), Vec::<String>::new());

        let crooked = drawn.replace(
            "                       │ jobs",
            "                      │ jobs",
        );
        let notes = adr_reading_notes(&crooked);
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(notes[0].starts_with("the diagram at line 7 is badly drawn: line 3, column 24: ┬"));
    }

    /// Every `.md` file under `dir`, with its path relative to `root`.
    fn markdown_files(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                markdown_files(root, &path, out);
            } else if path.extension().is_some_and(|e| e == "md") {
                out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .display()
                        .to_string()
                        .replace('\\', "/"),
                );
            }
        }
    }

    #[test]
    fn every_diagram_in_the_repositorys_docs_is_well_drawn() {
        // The pages under docs/ (the site renders those; the README's own
        // overview is older than these rules and is not one of them). The
        // site checks the same when it is built (web/site, `npm test`), and
        // it is built only after a change has merged. This is the check a
        // change meets before it merges. A column that drifted by one does
        // not show in the Markdown; it shows on the page.
        let repo = hive_testutil::manifest_dir(env!("CARGO_MANIFEST_DIR")).join("../..");
        if !repo.join("docs").is_dir() {
            return; // the crate alone, outside its repository
        }
        let mut files = Vec::new();
        markdown_files(&repo, &repo.join("docs"), &mut files);
        files.sort();
        let mut drawn = 0;
        let mut problems = Vec::new();
        for file in &files {
            let Ok(text) = std::fs::read_to_string(repo.join(file)) else {
                continue;
            };
            for d in find_diagrams(&text, DIAGRAM_MAX_COLUMNS) {
                drawn += 1;
                for p in d.problems {
                    problems.push(format!("{file}:{}: {p}", d.line));
                }
            }
        }
        assert!(
            problems.is_empty(),
            "badly drawn (node web/site/scripts/check-diagrams.mjs lists every diagram):\n{}",
            problems.join("\n")
        );
        assert!(
            drawn > 0,
            "the docs hold no diagram at all; were they moved?"
        );
    }

    #[test]
    fn the_shipped_template_is_what_it_asks_for() {
        // TPL-adr is the body a new ADR starts from: it opens with the
        // section and carries a well-drawn example. The one note a record
        // created from it earns is that the example is still there.
        let bundle = crate::bundle::framework_bundle().unwrap();
        let body = |id: &str| {
            bundle
                .records
                .iter()
                .find(|r| r.id == id)
                .unwrap_or_else(|| panic!("the bundle has no {id}"))
                .body
                .clone()
        };
        let template = body("TPL-adr");
        let notes = adr_reading_notes(&template);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].starts_with("it still holds the template's example diagram"));
        assert_eq!(
            adr_reading_notes(&template.replace(TEMPLATE_EXAMPLE_MARK, "The change")),
            Vec::<String>::new(),
            "with the example replaced nothing is left to say"
        );
        let glance = template.find("## At a glance").unwrap();
        assert!(
            glance < template.find("## Context").unwrap(),
            "the owner reads At a glance first"
        );
        // The workflow and the skill tell the agent to write both, in the
        // words the note uses.
        for id in ["WF-decision", "SKILL-hive-decision"] {
            let text = body(id);
            assert!(
                text.contains("`## At a glance`") && text.contains("diagram in text"),
                "{id} does not ask for the section and its diagram"
            );
            assert!(
                text.contains(&format!("at most {DIAGRAM_MAX_COLUMNS} characters")),
                "{id} names another width than the checker"
            );
        }
    }
}
