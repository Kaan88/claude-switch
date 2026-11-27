//! The check that finds a Claude Code update which draws its screens differently: Chat's readers
//! over fresh recordings (`tools/screen-recorder`, `recorder compat`), compared with what each
//! state is expected to read as (`tools/screen-recorder/expect/<script>.expect`).
//!
//! An expectation file has a `[state]` section per snapshot of its script and `key = value` lines
//! under it; `#` starts a comment. Only the keys a section names are compared. In a value `*`
//! stands for any text, runs of blanks count as one, and `\n` is a line break. The keys:
//!
//! - `kind`: what the screen is to Chat, the first that fits of `full-screen`, `blocked` (the
//!   list of calls auto mode blocked), `plan`,
//!   `permission`, `permission-unread` (looks like a prompt, cannot be answered), `rewind`,
//!   `rewind-actions`, `review`, `question`, `menu`, `panel`, `menu-unread`, `working`, `idle`
//!   (the empty input), `input` (the input holds text), `unknown`.
//! - `input`: the one-row input's text (`-` when it is not read), `send`: `yes` when a message may
//!   be sent now, `working`, `attention` (the tab asks for a permission), `esc`: `yes` or `no`;
//!   `mode`: the permission mode under the input (`-` for none).
//! - `title`: the question asked (a question, a permission prompt, the plan approval), a panel's
//!   title, the prompt a rewind quotes. `choices`: the rows joined with ` | `, `count`: how many,
//!   `highlight`: the highlighted row's text, `body`: a permission prompt's rows above its
//!   question, `request`: every row of the prompt from its top rule to its question, as the card
//!   for a prompt no pending call covers shows it (`-` when the top is off the screen),
//!   `tell` and `keep`: the declining row of a prompt and of the plan approval,
//!   `notes`: a question's descriptions, `rows`: a panel's rows, `answers`: a review's
//!   `question = answer` pairs.
//! - For the list of blocked calls: `title` is the tab's note, `choices` its calls, `body` the
//!   highlighted call's rows, `marks` each call's mark (`none`, `approve`, `retry`), `pending` the
//!   row saying what Esc will do (`-` for none) and `esc-does` the footer's words for it.
//! - `answer`: this text reads back exactly on the highlighted row ("Type something");
//!   `lines` and `wrapped`: the input shows this text of several lines, or this long line.
//!
//! `cargo test -p claude-switch cli_compat -- --ignored --nocapture` with `CS_COMPAT_RECORDINGS`
//! (a folder per script, `<script>/<state>.ansi`; absolute, as the test runs in the crate's
//! folder). `CS_COMPAT_EXPECT` names another expectations folder, `CS_COMPAT_ONLY` the scripts to
//! check (commas between), `CS_COMPAT_SUMMARY` a file for the result as JSON.
use std::path::Path;

use super::recorded::*;
use super::*;
use crate::view::chat::submission::screen;

const NONE: &str = "-";

fn yes(value: bool) -> String {
    if value { "yes" } else { "no" }.to_owned()
}

fn kind(s: &Screen) -> &'static str {
    if s.modes().alt_screen {
        "full-screen"
    } else if blocked(s).is_some() {
        "blocked"
    } else if plan(s).is_some() {
        "plan"
    } else if permission(s).is_some() {
        "permission"
    } else if permission_like(s) {
        "permission-unread"
    } else if rewind_list(s).is_some() {
        "rewind"
    } else if rewind_actions(s).is_some() {
        "rewind-actions"
    } else if review(s).is_some() {
        "review"
    } else if let Some(panel) = panel(s) {
        match panel {
            Panel { question: true, .. } => "question",
            Panel { menu: Some(_), .. } => "menu",
            _ => "panel",
        }
    } else if screen::menu(s) {
        "menu-unread"
    } else if screen::working(s) {
        "working"
    } else if screen::clean_prompt(s) {
        "idle"
    } else if screen::shows_some_input(s) {
        "input"
    } else {
        "unknown"
    }
}

/// The choices of whatever the screen shows, with the highlighted one.
fn choices(s: &Screen) -> (Vec<String>, Option<usize>) {
    let of = |menu: &Menu| (menu.texts(), menu.highlight());
    if let Some(list) = blocked(s) {
        of(&list.menu)
    } else if let Some(menu) = plan(s) {
        of(&menu)
    } else if let Some(prompt) = permission(s) {
        of(&prompt.menu)
    } else if let Some(list) = rewind_list(s) {
        (list.entries, Some(list.highlight))
    } else if let Some(actions) = rewind_actions(s) {
        (actions.choices, actions.highlight)
    } else if let Some(menu) = panel(s).and_then(|p| p.menu) {
        of(&menu)
    } else {
        menu(s).map_or((Vec::new(), None), |menu| of(&menu))
    }
}

fn title(s: &Screen) -> String {
    if let Some(list) = blocked(s) {
        list.note
    } else if let Some(menu) = plan(s) {
        menu.above_box(4).last().map(|q| (*q).to_owned()).unwrap_or_default()
    } else if let Some(prompt) = permission(s) {
        prompt.question
    } else if let Some(actions) = rewind_actions(s) {
        actions.quoted
    } else {
        panel(s).map(|p| p.title).unwrap_or_default()
    }
}

/// What the readers make of `key` on this screen; for the keys that ask whether a text shows,
/// `want` itself when it does.
fn fact(s: &Screen, key: &str, want: &str) -> String {
    let joined = |rows: Vec<String>| if rows.is_empty() { NONE.to_owned() } else { rows.join(" | ") };
    let pick = |i: Option<usize>| i.and_then(|i| choices(s).0.get(i).cloned()).unwrap_or_else(|| NONE.to_owned());
    let shown = |shows: bool, what: &str| if shows { want.to_owned() } else { format!("not shown ({what})") };
    match key {
        "kind" => kind(s).to_owned(),
        "input" => screen::prompt(s).unwrap_or_else(|| NONE.to_owned()),
        "send" => yes(screen::idle_slash_prompt(s, true)),
        "working" => yes(screen::working(s)),
        "attention" => yes(crate::attention::screen_permission_prompt(s)),
        "mode" => mode(s).unwrap_or(NONE).to_owned(),
        "title" => title(s),
        "choices" => joined(choices(s).0),
        "count" => choices(s).0.len().to_string(),
        "highlight" => pick(choices(s).1),
        "body" => joined(blocked(s).map(|b| b.detail).or_else(|| permission(s).map(|p| p.body)).unwrap_or_default()),
        "marks" => joined(
            blocked(s).map(|b| b.marks.iter().map(|m| format!("{m:?}").to_lowercase()).collect()).unwrap_or_default(),
        ),
        "pending" => blocked(s).and_then(|b| b.pending).unwrap_or_else(|| NONE.to_owned()),
        "esc-does" => blocked(s).map(|b| b.esc).unwrap_or_else(|| NONE.to_owned()),
        "request" => joined(permission(s).and_then(|p| permission_rows(s, &p)).unwrap_or_default()),
        "tell" => pick(permission(s).and_then(|p| p.tell)),
        "keep" => pick(plan(s).and_then(|menu| plan_keep(&menu))),
        "notes" => joined(panel(s).map(|p| p.notes).unwrap_or_default()),
        "rows" => joined(panel(s).map(|p| p.rows).unwrap_or_default()),
        "esc" => yes(panel(s).is_some_and(|p| p.esc)),
        "answers" => {
            joined(review(s).map(|r| r.answers.iter().map(|(q, a)| format!("{q} = {a}")).collect()).unwrap_or_default())
        }
        "answer" => {
            let menu = walk_menu(s);
            let at = menu.as_ref().and_then(Menu::highlight);
            let shows =
                menu.as_ref().zip(at).is_some_and(|(m, i)| crate::view::chat::question::entry_shows(m, i, want));
            let row =
                menu.zip(at).map(|(m, i)| m.entries[i].text.clone()).unwrap_or_else(|| "no highlighted row".into());
            shown(shows, &row)
        }
        "lines" => shown(screen::shows_lines(s, want), "the input's rows do not read it"),
        "wrapped" => shown(screen::shows_wrapped(s, want), "the input's rows do not read it"),
        _ => format!("unknown key {key}"),
    }
}

/// `text` matches `pattern`, where `*` is any text; blanks are collapsed on both sides.
fn matches(pattern: &str, text: &str) -> bool {
    let (pattern, text) = (collapse(pattern), collapse(text));
    let parts: Vec<&str> = pattern.split('*').collect();
    let Some((first, rest)) = parts.split_first() else { return false };
    let Some(mut left) = text.strip_prefix(first) else { return false };
    let Some((last, middle)) = rest.split_last() else { return left.is_empty() };
    for part in middle {
        let Some(at) = left.find(part) else { return false };
        left = &left[at + part.len()..];
    }
    left.ends_with(last)
}

/// An expectation file's sections in order: each state with its `key = value` lines.
fn sections(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            out.push((name.trim().to_owned(), Vec::new()));
        } else {
            let (key, value) = line.split_once('=').unwrap_or_else(|| panic!("`key = value` expected: {line}"));
            let section = out.last_mut().unwrap_or_else(|| panic!("a [state] line before: {line}"));
            section.1.push((key.trim().to_owned(), value.trim().replace("\\n", "\n")));
        }
    }
    out
}

/// One state's result: the keys that did not match, each with what was expected and read.
fn check(recording: &Path, wanted: &[(String, String)]) -> Result<Vec<(String, String, String)>, String> {
    let text = std::fs::read_to_string(recording).map_err(|_| "no recording".to_owned())?;
    let screen = recorded(&text.replace("\r\n", "\n"));
    Ok(wanted
        .iter()
        .filter_map(|(key, want)| {
            let got = fact(&screen, key, want);
            (!matches(want, &got)).then(|| (key.clone(), want.clone(), got))
        })
        .collect())
}

#[test]
#[ignore = "a tool: checks the recordings in CS_COMPAT_RECORDINGS against the expectations"]
fn cli_compat() {
    let recordings = std::path::PathBuf::from(std::env::var("CS_COMPAT_RECORDINGS").expect("CS_COMPAT_RECORDINGS"));
    // The repository's own expectations unless another folder is named.
    let expect = std::env::var("CS_COMPAT_EXPECT")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/screen-recorder/expect").to_owned());
    let expect = std::path::PathBuf::from(expect);
    let only: Vec<String> = std::env::var("CS_COMPAT_ONLY")
        .map(|names| names.split(',').map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()).collect())
        .unwrap_or_default();
    let mut files: Vec<_> = std::fs::read_dir(&expect)
        .expect("the expectations folder")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "expect"))
        .collect();
    files.sort();
    let (mut states, mut bad) = (Vec::new(), 0);
    for file in files {
        let script = file.file_stem().unwrap().to_string_lossy().into_owned();
        if !only.is_empty() && !only.contains(&script) {
            continue;
        }
        for (state, wanted) in sections(&std::fs::read_to_string(&file).unwrap()) {
            let name = format!("{script}/{state}");
            let result = check(&recordings.join(&script).join(format!("{state}.ansi")), &wanted);
            let (word, diffs) = match &result {
                Ok(diffs) if diffs.is_empty() => ("ok", Vec::new()),
                Ok(diffs) => ("mismatch", diffs.clone()),
                Err(why) => ("missing", vec![("recording".to_owned(), "a snapshot".to_owned(), why.clone())]),
            };
            match word {
                "ok" => println!("ok        {name}"),
                _ => {
                    bad += 1;
                    for (key, want, got) in &diffs {
                        println!("{:9} {name}  {key}: expected {want:?}, got {got:?}", word.to_uppercase());
                    }
                }
            }
            let diffs: Vec<_> = diffs
                .iter()
                .map(|(key, want, got)| serde_json::json!({ "key": key, "expected": want, "got": got }))
                .collect();
            states.push(serde_json::json!({ "script": script, "state": state, "result": word, "diffs": diffs }));
        }
    }
    let checked = states.len();
    println!("{} of {checked} states as expected", checked - bad);
    if let Ok(path) = std::env::var("CS_COMPAT_SUMMARY") {
        let summary = serde_json::json!({ "checked": checked, "mismatched": bad, "states": states });
        std::fs::write(path, serde_json::to_string_pretty(&summary).unwrap()).expect("write the summary");
    }
    assert!(checked > 0, "no expectations for {only:?} in {}", expect.display());
    assert_eq!(bad, 0, "{bad} of {checked} states do not read as expected");
}

/// Prints what the readers make of every recording under `CS_COMPAT_RECORDINGS`, in the form of
/// an expectation file: the start of a new one, and a look at what changed after an update.
/// `cargo test -p claude-switch compat_facts -- --ignored --nocapture`
#[test]
#[ignore = "a tool: prints the facts of the recordings in CS_COMPAT_RECORDINGS"]
fn compat_facts() {
    const KEYS: [&str; 20] = [
        "kind",
        "input",
        "send",
        "working",
        "attention",
        "mode",
        "title",
        "choices",
        "count",
        "highlight",
        "body",
        "request",
        "tell",
        "keep",
        "notes",
        "rows",
        "answers",
        "marks",
        "pending",
        "esc-does",
    ];
    let recordings = std::path::PathBuf::from(std::env::var("CS_COMPAT_RECORDINGS").expect("CS_COMPAT_RECORDINGS"));
    let mut files: Vec<_> = std::fs::read_dir(&recordings)
        .expect("the recordings folder")
        .flat_map(|e| {
            let path = e.unwrap().path();
            if path.is_dir() {
                std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()).collect()
            } else {
                vec![path]
            }
        })
        .filter(|p| p.extension().is_some_and(|e| e == "ansi"))
        .collect();
    files.sort();
    for file in files {
        let screen = recorded(&std::fs::read_to_string(&file).unwrap().replace("\r\n", "\n"));
        println!("\n# {}\n[{}]", file.display(), file.file_stem().unwrap().to_string_lossy());
        for key in KEYS {
            let got = fact(&screen, key, "");
            if !matches!(got.as_str(), NONE | "" | "no" | "0") || matches!(key, "kind" | "input" | "send") {
                println!("{key} = {got}");
            }
        }
    }
}

#[test]
fn a_pattern_matches_whole_texts_with_stars_for_any_text() {
    assert!(matches(
        "Yes, and always allow access to * from this project",
        "Yes, and always allow access to x\\y from this project"
    ));
    assert!(matches("a  b", "a b") && matches("*", "") && matches("*b*", "abc") && matches("a*", "a"));
    assert!(!matches("a", "ab") && !matches("a*c", "ab") && !matches("*b", "abc") && !matches("", "a"));
    assert!(matches("ab*ab", "abab") && !matches("ab*ab", "ab"));
}

#[test]
fn the_fixtures_read_as_the_states_the_check_names() {
    for (fixture, wanted) in [
        (PROMPT_IDLE, "kind = idle\ninput =\nsend = yes\nworking = no"),
        (PLAIN_TYPED, "kind = input\ninput = Reply with just the word: pong\nsend = no"),
        (WORKING_TOOL, "kind = working\nsend = no"),
        (PLAIN_PASTED_LINES, "kind = input\ninput = -\nlines = line one\\nline two\\nline three"),
        (ASK_SINGLE, "kind = question\ntitle = Which colour do you prefer?\nhighlight = Red (warm)\ncount = 5"),
        (ASK_OTHER_TYPED, "kind = question\nanswer = my own answer"),
        (FORM_MULTI, "kind = question\nchoices = [ ] Apple | [ ] Pear | [ ] Plum | [ ] Type something | Next | *"),
        (ASK_REVIEW, "kind = review\nanswers = Which size? = Small | Which speed? = Slow\nhighlight = Submit answers"),
        (PERM_BASH, "kind = permission\ntitle = Do you want to proceed?\ncount = 4\ntell = -\nattention = yes"),
        (PERM_AGENT_BASH, "kind = permission\nrequest = Bash command * from the general-purpose agent | *"),
        (PERM_TWO_AFTER, "kind = permission\nattention = yes\nbody = Bash command*| mkdir par_b"),
        (BLOCKED_ONE, "kind = blocked\nmarks = none\npending = -\nesc-does = close\nbody = mkdir probe_blocked_two"),
        (BLOCKED_ONE_APPROVED, "kind = blocked\nmarks = approve\npending = Esc will approve 1 blocked call"),
        (PLAN, "kind = plan\nkeep = Tell Claude what to change\nsend = no"),
        (MENU_MODEL, "kind = menu\ncount = 5\nesc = yes"),
        (MENU_CONFIG, "kind = panel\nrows = *Search settings*"),
        (REWIND_OPEN, "kind = rewind\nhighlight = (current)"),
        (super::recorded::REWIND_ACTIONS, "kind = rewind-actions"),
    ] {
        let wanted = sections(&format!("[state]\n{wanted}")).remove(0).1;
        let screen = recorded(fixture);
        for (key, want) in wanted {
            let got = fact(&screen, &key, &want);
            assert!(matches(&want, &got), "{key}: expected {want:?}, got {got:?}");
        }
    }
    // A text that is not there is a mismatch that says what the row holds.
    assert_eq!(fact(&recorded(ASK_OTHER_TYPED), "answer", "something else"), "not shown (my own answer)");
}
