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
//! - `sent`: the input shows this message as Chat's echo check before Enter takes it into a
//!   running turn (as typed, wrapped, or as Claude Code's paste marker).
//! - `shows` and `hides`: every text of the value (` | ` between them) is somewhere on the screen,
//!   or none is (a resumed branch shows the turns before its cut and none after).
//!
//! A state can also be a file the script wrote instead of a snapshot, and its keys are about that:
//!
//! - A transcript copy (`tscopy`, `<state>.jsonl`), as the app's transcript reader reads it:
//!   `prompts` (each prompt, ` (during a turn)` after one sent while a turn ran, ` [N pictures]`),
//!   `pictures` (`yes` when every picture of such a message loads, `none` when it has none),
//!   `turn` (the prompt that started the newest turn: a message sent during a turn starts none),
//!   `answers` (Claude's answers), `thinking` (the `thinkingDisplay` values of records with
//!   thinking, `-` for none).
//! - A branch (`branch`, `<state>.json` and `<state>.id`): `resumed` (`yes` when Claude Code
//!   answered under the copy's id) and `words` (which of alpha, bravo and charlie the answer names).
//! - `claude --help` (`help`, `<state>.help.txt`): `flags` (the text itself when every flag of it is
//!   listed, else the missing ones).
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
        "shows" | "hides" => {
            let all: String = (0..s.rows()).map(|r| s.row_text(r) + "\n").collect();
            let wrong: Vec<&str> =
                want.split(" | ").filter(|part| all.contains(part.trim()) != (key == "shows")).collect();
            match (wrong.is_empty(), key) {
                (true, _) => want.to_owned(),
                (false, "shows") => format!("not shown: {}", wrong.join(" | ")),
                _ => format!("shown: {}", wrong.join(" | ")),
            }
        }
        "wrapped" => shown(screen::shows_wrapped(s, want), "the input's rows do not read it"),
        "sent" => {
            let typed = screen::prompt(s).as_deref() == Some(want) || screen::shows_wrapped(s, want);
            shown(typed || screen::shows_lines(s, want), "the input's rows do not read it")
        }
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

/// What the app's transcript reader makes of a transcript copy.
fn transcript_fact(path: &Path, key: &str) -> String {
    use claude_switch_core::transcript::{ItemKind, Reader};
    let Ok(mut reader) = Reader::open(path, 64 << 20) else { return "the transcript does not read".to_owned() };
    let items: Vec<_> = reader.items().map(|(id, item)| (id, item.clone())).collect();
    let joined = |parts: Vec<String>| if parts.is_empty() { NONE.to_owned() } else { parts.join(" | ") };
    match key {
        "prompts" => joined(
            items
                .iter()
                .filter_map(|(_, item)| match &item.kind {
                    ItemKind::Prompt { text, pictures, synthetic: false, queued } => {
                        let mut line = text.replace('\n', " ");
                        if *queued {
                            line.push_str(" (during a turn)");
                        }
                        if !pictures.is_empty() {
                            line.push_str(&format!(" [{} pictures]", pictures.len()));
                        }
                        Some(line)
                    }
                    _ => None,
                })
                .collect(),
        ),
        "pictures" => {
            let pictures: Vec<_> = items
                .iter()
                .flat_map(|(_, item)| match &item.kind {
                    ItemKind::Prompt { pictures, queued: true, .. } => pictures.clone(),
                    _ => Vec::new(),
                })
                .collect();
            if pictures.is_empty() {
                "none".to_owned()
            } else {
                yes(pictures.iter().all(|p| reader.load_picture(p).is_ok_and(|bytes| !bytes.is_empty())))
            }
        }
        "turn" => {
            let at = reader.chat().prompt;
            items
                .iter()
                .find_map(|(id, item)| match &item.kind {
                    ItemKind::Prompt { text, .. } if Some(id.offset) == at => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| NONE.to_owned())
        }
        "answers" => joined(
            items
                .iter()
                .filter_map(|(_, item)| match &item.kind {
                    ItemKind::Assistant { markdown } => Some(markdown.trim().replace('\n', " ")),
                    _ => None,
                })
                .collect(),
        ),
        "thinking" => {
            let mut seen: Vec<String> = Vec::new();
            for line in std::fs::read_to_string(path).unwrap_or_default().lines() {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { continue };
                let thinking = value["message"]["content"]
                    .as_array()
                    .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "thinking"));
                if value["type"] == "assistant" && thinking {
                    let display = value["thinkingDisplay"].as_str().unwrap_or("(none)").to_owned();
                    if !seen.contains(&display) {
                        seen.push(display);
                    }
                }
            }
            joined(seen)
        }
        _ => format!("unknown key {key}"),
    }
}

/// What a branch's print-mode answer says: `resumed`, `words`.
fn branch_fact(json: &Path, key: &str) -> String {
    let id = std::fs::read_to_string(json.with_extension("id")).unwrap_or_default();
    let Ok(text) = std::fs::read_to_string(json) else { return "no answer".to_owned() };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return format!("not JSON: {}", text.chars().take(200).collect::<String>());
    };
    match key {
        "resumed" => yes(value["is_error"] == false && value["session_id"].as_str() == Some(id.trim())),
        "words" => {
            let answer = value["result"].as_str().unwrap_or_default().to_lowercase();
            let words: Vec<&str> = ["alpha", "bravo", "charlie"].into_iter().filter(|w| answer.contains(w)).collect();
            if words.is_empty() { NONE.to_owned() } else { words.join(", ") }
        }
        _ => format!("unknown key {key}"),
    }
}

/// Whether `claude --help` lists every flag in `want`.
fn help_fact(help: &Path, key: &str, want: &str) -> String {
    let text = std::fs::read_to_string(help).unwrap_or_default();
    match key {
        "flags" => {
            let missing: Vec<&str> = want.split_whitespace().filter(|flag| !text.contains(flag)).collect();
            if missing.is_empty() { want.to_owned() } else { format!("missing {}", missing.join(" ")) }
        }
        _ => format!("unknown key {key}"),
    }
}

/// What a state's file says for a key, given the expected value.
type Reads = dyn Fn(&str, &str) -> String;

/// One state's result: the keys that did not match, each with what was expected and read. The
/// state is a snapshot (`<state>.ansi`) or a file the script wrote instead (see the module).
fn check(folder: &Path, state: &str, wanted: &[(String, String)]) -> Result<Vec<(String, String, String)>, String> {
    let file = |extension: &str| Some(folder.join(format!("{state}.{extension}"))).filter(|p| p.is_file());
    let read: Box<Reads> = if let Some(path) = file("ansi") {
        let text = std::fs::read_to_string(&path).map_err(|_| "no recording".to_owned())?;
        let screen = recorded(&text.replace("\r\n", "\n"));
        Box::new(move |key, want| fact(&screen, key, want))
    } else if let Some(path) = file("jsonl") {
        Box::new(move |key, _| transcript_fact(&path, key))
    } else if let Some(path) = file("json") {
        Box::new(move |key, _| branch_fact(&path, key))
    } else if let Some(path) = file("help.txt") {
        Box::new(move |key, want| help_fact(&path, key, want))
    } else {
        return Err("no recording".to_owned());
    };
    Ok(wanted
        .iter()
        .filter_map(|(key, want)| {
            let got = read(key, want);
            (!matches(want, &got)).then(|| (key.clone(), want.clone(), got))
        })
        .collect())
}

/// For `recorder compat`'s `handover`: a handover that starts with `CS_HANDOVER_TEXT`, long enough
/// to be read from its file, written to `CS_HANDOVER_DIR`, and the arguments the app gives a new
/// handover session for it (`compact_open::first_args`), as JSON to `CS_HANDOVER_OUT`.
#[test]
#[ignore = "a tool for recorder compat: writes a handover file and its session's arguments"]
fn cli_handover_args() {
    let dir = std::path::PathBuf::from(std::env::var("CS_HANDOVER_DIR").expect("CS_HANDOVER_DIR"));
    let mut text = format!("{}\n\n", std::env::var("CS_HANDOVER_TEXT").expect("CS_HANDOVER_TEXT"));
    for n in 1.. {
        if crate::compact_open::uses_file(&text) {
            break;
        }
        text.push_str(&format!("Background note {n}: nothing in it is a task.\n"));
    }
    std::fs::create_dir_all(&dir).expect("the handover folder");
    let saved = dir.join("handover.md");
    std::fs::write(&saved, &text).expect("the handover file");
    let id = crate::compact_open::session_id().expect("a new id");
    let args = crate::compact_open::first_args(&id, &text, &saved).expect("the arguments");
    let out = std::env::var("CS_HANDOVER_OUT").expect("CS_HANDOVER_OUT");
    std::fs::write(out, serde_json::to_string(&args).unwrap()).expect("the arguments' file");
}

/// For `recorder compat`'s `branch`: writes a branch of the transcript `CS_BRANCH_SOURCE` with the
/// app's own code, as Branch under a message does: `CS_BRANCH_AT` is `prompt TEXT` or
/// `answer TEXT` (the first prompt, or the last answer, whose text contains TEXT), cut where the
/// app cuts (`view::model::branch_cut`); the new id goes to `CS_BRANCH_OUT`.
#[test]
#[ignore = "a tool for recorder compat: writes a transcript copy"]
fn cli_branch_write() {
    use claude_switch_core::branch::write;
    use claude_switch_core::transcript::{ItemKind, Reader};
    let source = std::path::PathBuf::from(std::env::var("CS_BRANCH_SOURCE").expect("CS_BRANCH_SOURCE"));
    let at = std::env::var("CS_BRANCH_AT").expect("CS_BRANCH_AT");
    let (kind, text) = at.split_once(' ').expect("prompt TEXT or answer TEXT");
    let text = text.to_lowercase();
    let reader = Reader::open(&source, 64 << 20).expect("the transcript");
    let items: std::collections::BTreeMap<_, _> = reader.items().map(|(id, item)| (id, item.clone())).collect();
    let found = |(id, item): (&claude_switch_core::transcript::ItemId, &claude_switch_core::transcript::Item)| {
        let shown = match (&item.kind, kind) {
            (ItemKind::Prompt { text, .. }, "prompt") => text,
            (ItemKind::Assistant { markdown }, "answer") => markdown,
            _ => return None,
        };
        shown.to_lowercase().contains(&text).then_some(*id)
    };
    let item = if kind == "prompt" { items.iter().find_map(found) } else { items.iter().rev().find_map(found) };
    let item = item.expect("a message with the text");
    let cut = crate::view::model::branch_cut(&items, &reader.chat(), item).expect("a place a branch can be made");
    let old = source.file_stem().unwrap().to_string_lossy().into_owned();
    let new = crate::compact_open::session_id().expect("a new id");
    write(&source, &old, &new, cut).expect("the copy");
    std::fs::write(std::env::var("CS_BRANCH_OUT").expect("CS_BRANCH_OUT"), &new).expect("the id");
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
            let result = check(&recordings.join(&script), &state, &wanted);
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
        (
            PERM_READ_OUTSIDE,
            "kind = permission\ncount = 3\nhighlight = Yes\nattention = yes\nbody = Read file | Read(*h.md)",
        ),
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
    let idle = recorded(PROMPT_IDLE);
    let row = (0..idle.rows()).map(|r| idle.row_text(r)).find(|t| t.trim().len() > 3).unwrap();
    let some = row.trim().to_owned();
    assert_eq!(fact(&idle, "shows", &some), some);
    assert_eq!(fact(&idle, "hides", "no such words here"), "no such words here");
    assert_eq!(fact(&idle, "hides", &some), format!("shown: {some}"));
    assert_eq!(fact(&idle, "shows", "no such words here"), "not shown: no such words here");
    // A text that is not there is a mismatch that says what the row holds.
    assert_eq!(fact(&recorded(ASK_OTHER_TYPED), "answer", "something else"), "not shown (my own answer)");
}

#[test]
fn transcripts_branches_and_help_read_as_the_check_names_them() {
    use serde_json::json;
    let dir = crate::testing::Scratch::created("cs-compat-files");
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEUlEQVR42mO4WR6OFTEMLQkAcq9pwfhQlf4AAAAASUVORK5CYII=";
    let lines = [
        json!({"type":"user","uuid":"u1","message":{"role":"user","content":"ping it"}}),
        json!({"type":"assistant","thinkingDisplay":"updates","message":{"role":"assistant","stop_reason":"tool_use",
            "content":[{"type":"thinking","thinking":"","signature":"s"}]}}),
        json!({"type":"attachment","attachment":{"type":"queued_command","commandMode":"prompt","origin":{"kind":"human"},
            "prompt":[{"type":"text","text":"[Image #1] Also say delta."},
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":png}}]}}),
        json!({"type":"assistant","message":{"role":"assistant","stop_reason":"end_turn",
            "content":[{"type":"text","text":"delta"}]}}),
    ];
    let transcript = dir.join("transcript.jsonl");
    std::fs::write(&transcript, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    let read = |key| transcript_fact(&transcript, key);
    assert_eq!(read("prompts"), "ping it | [Image #1] Also say delta. (during a turn) [1 pictures]");
    assert_eq!(read("pictures"), "yes");
    assert_eq!(read("turn"), "ping it");
    assert_eq!(read("answers"), "delta");
    assert_eq!(read("thinking"), "updates");
    let branch = dir.join("branched.json");
    std::fs::write(dir.join("branched.id"), "0a1b2c3d-1111-4222-8333-444455556666\n").unwrap();
    let answer = json!({"is_error":false,"session_id":"0a1b2c3d-1111-4222-8333-444455556666","result":"Alpha"});
    std::fs::write(&branch, answer.to_string()).unwrap();
    assert_eq!((branch_fact(&branch, "resumed"), branch_fact(&branch, "words")), ("yes".into(), "alpha".into()));
    let help = dir.join("cli.help.txt");
    std::fs::write(&help, "  -r, --resume [value]\n  --fork-session\n").unwrap();
    assert_eq!(help_fact(&help, "flags", "--resume --fork-session"), "--resume --fork-session");
    assert_eq!(help_fact(&help, "flags", "--resume --gone"), "missing --gone");
    // The check finds a state's file by its name, a snapshot first.
    let wanted = vec![("pictures".to_owned(), "yes".to_owned())];
    assert_eq!(check(&dir, "transcript", &wanted), Ok(Vec::new()));
    assert!(check(&dir, "nothing", &wanted).is_err());
}
