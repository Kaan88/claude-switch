use super::*;
use crate::util::testing::TempDir;
use std::cell::RefCell;

fn excerpts(first: &str, last: &[&str]) -> Excerpts {
    Excerpts { first: Some(first.into()), last: last.iter().map(|s| (*s).to_owned()).collect() }
}

fn item(id: &str, active_ms: u64, first: &str) -> Item {
    Item { id: id.into(), active_ms, excerpts: excerpts(first, &["later prompt"]) }
}

fn digest(summary: &str, keywords: &[&str], subject: Option<&str>) -> Digest {
    Digest {
        basis: 1,
        active_ms: 1,
        engine: "claude".into(),
        summary: summary.into(),
        keywords: keywords.iter().map(|s| (*s).to_owned()).collect(),
        subject: subject.map(str::to_owned),
    }
}

/// A fake model: answers every requested id with a summary naming its first prompt.
fn fake(request: &str) -> Result<String, String> {
    let payload: Value = serde_json::from_str(request.lines().last().unwrap()).unwrap();
    Ok(payload
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let first = item["first"].as_str().unwrap_or_default();
            json!({"id": item["id"], "summary": format!("Worked on {first}"), "keywords": ["fake"], "subject": "fake area"})
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

#[test]
fn preferences_are_off_by_default_and_round_trip() {
    let off = Prefs::read(&Value::Null);
    assert!(!off.on() && off.engine == Engine::Claude && !off.group_topics);
    let prefs = Prefs { enabled: true, engine: Engine::OpenCode("prov/model-x".into()), group_topics: true };
    assert_eq!(Prefs::read(&prefs.json()), prefs);
    let bad = Prefs::read(&json!({"enabled": true, "model": "opencode:no slash"}));
    assert_eq!(bad.engine, Engine::Claude);
    assert_eq!(Engine::parse("claude"), Some(Engine::Claude));
    assert_eq!(Engine::parse("other"), None);
}

#[test]
fn the_index_request_carries_the_name_suggestion_payload_and_nothing_else() {
    let long = "é".repeat(600);
    let items = [excerpts(&long, &["one", "two", "three", "four"])];
    let request = index_request(&items, &["Build checks".into(), "far too many words here".into()]);
    let last = request.lines().last().unwrap();
    assert_eq!(last, json!(naming::excerpt_payload(&items)).to_string());
    let naming = naming::build_prompt(&items);
    assert_eq!(naming.lines().last().unwrap(), last, "the same excerpts as Suggest names");
    assert!(request.contains("Existing subjects: [\"Build checks\"]"));
    assert!(!request.contains("far too many"));
}

#[test]
fn indexing_sends_only_changed_sessions_in_batches_and_keeps_partial_work() {
    let mut cache = Cache::default();
    let items: Vec<_> = (0..45).map(|n| item(&format!("s{n}"), 10, &format!("prompt {n}"))).collect();
    let requests = RefCell::new(Vec::new());
    let mut saves = 0;
    let report = index(
        &mut cache,
        items,
        |_| true,
        &Engine::Claude,
        |r| {
            requests.borrow_mut().push(r.to_owned());
            fake(r)
        },
        || false,
        |_| saves += 1,
    )
    .unwrap();
    assert_eq!(requests.borrow().len(), 3, "20 + 20 + 5");
    assert_eq!(saves, 3);
    assert_eq!((report.sent, report.summarized, report.unchanged), (45, 45, 0));
    assert_eq!(cache.digests["s7"].summary, "Worked on prompt 7");
    assert_eq!(cache.digests["s7"].subject.as_deref(), Some("fake area"));
    assert!(requests.borrow()[1].contains("Existing subjects: [\"fake area\"]"));

    // Later activity with the same excerpts sends nothing; changed excerpts are sent again.
    let again = vec![item("s1", 20, "prompt 1"), item("s2", 20, "prompt 2 changed")];
    requests.borrow_mut().clear();
    let report = index(
        &mut cache,
        again,
        |_| true,
        &Engine::Claude,
        |r| {
            requests.borrow_mut().push(r.to_owned());
            fake(r)
        },
        || false,
        |_| {},
    )
    .unwrap();
    assert_eq!((report.sent, report.unchanged), (1, 1));
    assert!(requests.borrow()[0].contains("prompt 2 changed") && !requests.borrow()[0].contains("\"prompt 1\""));
    assert!(!cache.stale("s1", 20, "claude"));
    assert!(cache.stale("s3", 20, "claude"));

    // A failing second request keeps the first request's answers.
    let mut fresh = Cache::default();
    let items: Vec<_> = (0..30).map(|n| item(&format!("t{n}"), 1, &format!("p{n}"))).collect();
    let mut calls = 0;
    let err = index(
        &mut fresh,
        items,
        |_| true,
        &Engine::Claude,
        |r| {
            calls += 1;
            if calls == 1 { fake(r) } else { Err("offline".into()) }
        },
        || false,
        |_| {},
    )
    .unwrap_err();
    assert_eq!(err, "offline");
    assert_eq!(fresh.digests.len(), 20);

    // Cancelled before the first request: nothing is sent.
    let mut sent = false;
    let cancelled = index(
        &mut Cache::default(),
        vec![item("x", 1, "p")],
        |_| true,
        &Engine::Claude,
        |r| {
            sent = true;
            fake(r)
        },
        || true,
        |_| {},
    );
    assert!(cancelled.is_err() && !sent);
}

#[test]
fn only_valid_answers_for_requested_ids_are_kept() {
    let answer = [
        r#"```jsonl"#,
        r#"{"id":0,"summary":"Fixed the push hook","keywords":["git","push","git",""],"subject":"Git workflow"}"#,
        r#"{"id":1,"summary":"","keywords":[]}"#,
        r#"{"id":2,"summary":"First"}"#,
        r#"{"id":2,"summary":"Twice"}"#,
        r#"{"id":3,"summary":"Out of range"}"#,
        r#"{"id":4,"summary":"Bad subject","subject":"one two three four"}"#,
    ]
    .join("\n");
    let parsed = parse_index(&answer, 5, &["git workflow".into()]);
    assert_eq!(
        parsed[&0],
        ("Fixed the push hook".into(), vec!["git".into(), "push".into()], Some("git workflow".into()))
    );
    assert!(!parsed.contains_key(&1) && !parsed.contains_key(&2));
    assert_eq!(parsed[&3].0, "Out of range");
    assert_eq!(parsed[&4].2, None);
    assert!(!parse_index(&answer, 3, &[]).contains_key(&3));
}

#[test]
fn the_cache_round_trips_and_a_damaged_file_is_empty() {
    let t = TempDir::new("meaning-cache");
    let path = t.0.join("meaning.json");
    assert_eq!(Cache::load(&path), Cache::default());
    let mut cache = Cache::default();
    cache.digests.insert("a".into(), Digest { basis: u64::MAX, ..digest("Fixed push", &["git"], Some("Git")) });
    cache.remember(Answer {
        question: "Where did we fix push?".into(),
        engine: "claude".into(),
        basis: 7,
        text: "In session a.\nIt was the hook.".into(),
        ids: vec!["a".into()],
    });
    cache.save(&path).unwrap();
    assert_eq!(Cache::load(&path), cache);
    std::fs::write(&path, b"{not json").unwrap();
    assert_eq!(Cache::load(&path), Cache::default());
}

#[test]
fn a_typed_search_finds_a_session_by_what_it_was_about() {
    let push = digest("Fixed git push being rejected by the pre-push hook", &["git", "hooks"], Some("Git workflow"));
    let ui = digest("Made the settings dialog scroll", &["scrollbar", "layout"], Some("Settings UI"));
    assert!(score("where we fixed push", &push).is_some());
    assert!(score("where we fixed push", &ui).is_none());
    assert!(score("scrolling settings", &ui).is_some());
    assert!(score("hook", &push).is_some(), "prefix of a keyword");
    assert!(score("the we where", &push).is_none(), "stop words alone match nothing");
    assert!(score("git push fonts", &push).is_some(), "one of three words may miss");
    assert!(score("push fonts", &push).is_none());
}

#[test]
fn questions_are_answered_once_and_kept_for_the_same_summaries() {
    let mut cache = Cache::default();
    cache.digests.insert("a".into(), digest("Fixed git push", &["git"], None));
    cache.digests.insert("b".into(), digest("Changed fonts", &["fonts"], None));
    let set: Vec<_> =
        ask_set(&cache, ["b", "a", "missing"]).into_iter().map(|(id, d)| (id.to_owned(), d.clone())).collect();
    assert_eq!(set.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
    let refs: Vec<_> = set.iter().map(|(id, d)| (id.as_str(), d)).collect();
    let basis = ask_basis(&refs);
    let mut request = String::new();
    let answer = ask(&mut cache, "Where did we  fix push?", &set, &Engine::Claude, |r| {
        request = r.to_owned();
        Ok("```json\n{\"answer\":\"In the git session.\",\"ids\":[1,1,9]}\n```".into())
    })
    .unwrap();
    assert!(request.contains("Fixed git push") && request.contains("Question: \"Where did we fix push?\""));
    assert_eq!(answer.ids, ["a"]);
    assert_eq!(
        cache.answer("where did we fix PUSH?", "claude", basis).map(|a| a.text.as_str()),
        Some("In the git session.")
    );
    assert!(cache.answer("where did we fix push?", "opencode:p/m", basis).is_none());
    assert!(cache.answer("where did we fix push?", "claude", basis + 1).is_none());
    assert!(ask(&mut cache, "q", &set, &Engine::Claude, |_| Ok("no json".into())).is_err());
    assert!(ask(&mut cache, "q", &[], &Engine::Claude, |_| panic!("nothing to ask over")).is_err());
}

#[test]
fn subjects_sent_come_only_from_sessions_that_may_be_sent_now() {
    let mut cache = Cache::default();
    cache.digests.insert("shown".into(), digest("Fixed fonts", &[], Some("Fonts")));
    cache.digests.insert("hidden".into(), digest("Private thing", &[], Some("Secret project")));
    let mut request = String::new();
    index(
        &mut cache,
        vec![item("new", 1, "a new prompt")],
        |id| id != "hidden",
        &Engine::Claude,
        |r| {
            request = r.to_owned();
            fake(r)
        },
        || false,
        |_| {},
    )
    .unwrap();
    assert!(request.contains("Existing subjects: [\"Fonts\"]"), "{request}");
    assert!(!request.contains("Secret project"));
    // The question set is the caller's visible ids only.
    assert!(ask_set(&cache, ["shown", "new"]).iter().all(|(id, _)| *id != "hidden"));
}

#[test]
fn empty_and_unanswered_sessions_get_markers_and_are_not_sent_again_until_they_change() {
    let mut cache = Cache::default();
    let empty = Item { id: "empty".into(), active_ms: 5, excerpts: Excerpts::default() };
    let items = vec![empty, item("skipped", 5, "never answered"), item("ok", 5, "answered")];
    // A model that answers only the session whose prompt says "answered".
    let partial = |r: &str| {
        let lines: Vec<_> = fake(r)?.lines().filter(|l| l.contains("Worked on answered")).map(str::to_owned).collect();
        Ok(lines.join("\n"))
    };
    let report = index(&mut cache, items, |_| true, &Engine::Claude, partial, || false, |_| {}).unwrap();
    assert_eq!((report.sent, report.summarized, report.skipped, report.empty), (2, 1, 1, 1));
    for id in ["empty", "skipped"] {
        assert!(cache.digests[id].is_marker(), "{id}");
        assert!(!cache.stale(id, 5, "claude"), "{id} is not offered again");
        assert!(score("never answered", &cache.digests[id]).is_none());
    }
    assert!(ask_set(&cache, ["empty", "skipped", "ok"]).iter().all(|(id, _)| *id == "ok"));
    assert!(cache.subjects(|_| true).iter().all(|s| s == "fake area"));

    // Markers survive a save and load.
    let t = TempDir::new("meaning-markers");
    let path = t.0.join("meaning.json");
    cache.save(&path).unwrap();
    assert_eq!(Cache::load(&path), cache);

    // Later activity with the same prompts sends nothing; changed prompts are retried.
    let mut sent = Vec::new();
    let again = vec![
        Item { id: "empty".into(), active_ms: 9, excerpts: Excerpts::default() },
        item("skipped", 9, "never answered"),
    ];
    let report = index(
        &mut cache,
        again,
        |_| true,
        &Engine::Claude,
        |r| {
            sent.push(r.to_owned());
            fake(r)
        },
        || false,
        |_| {},
    )
    .unwrap();
    assert!(sent.is_empty() && report.unchanged == 1 && report.empty == 1);
    assert!(!cache.stale("empty", 9, "claude") && !cache.stale("skipped", 9, "claude"));
    index(&mut cache, vec![item("skipped", 12, "now changed")], |_| true, &Engine::Claude, fake, || false, |_| {})
        .unwrap();
    assert_eq!(cache.digests["skipped"].summary, "Worked on now changed");
}

#[test]
fn an_unreadable_answer_fails_the_run_and_marks_nothing_and_another_model_retries_markers() {
    let mut cache = Cache::default();
    let items: Vec<_> = (0..25).map(|n| item(&format!("s{n}"), 1, &format!("p{n}"))).collect();
    let mut calls = 0;
    let mut saves = 0;
    let err = index(
        &mut cache,
        items,
        |_| true,
        &Engine::Claude,
        |r| {
            calls += 1;
            if calls == 1 { fake(r) } else { Ok("I'm sorry, I can't help with that.".into()) }
        },
        || false,
        |_| saves += 1,
    )
    .unwrap_err();
    assert!(
        err.contains("could not be read") && err.contains("5 session(s)") && err.contains("20 summarized"),
        "{err}"
    );
    assert_eq!(cache.digests.len(), 20, "the first request's answers are kept");
    assert!((20..25).all(|n| !cache.digests.contains_key(&format!("s{n}"))), "no markers for the unread batch");
    assert_eq!(saves, 1);
    assert!(cache.stale("s22", 1, "claude"), "offered again on the next run");

    // A marker the model left (it answered others in the same request) is retried by another model.
    let empty = Item { id: "empty".into(), active_ms: 1, excerpts: Excerpts::default() };
    let only_first = |r: &str| Ok(fake(r)?.lines().next().unwrap_or_default().to_owned());
    index(
        &mut cache,
        vec![item("a", 1, "first"), item("b", 1, "second"), empty],
        |_| true,
        &Engine::Claude,
        only_first,
        || false,
        |_| {},
    )
    .unwrap();
    assert!(cache.digests["b"].is_marker() && !cache.stale("b", 1, "claude"));
    let other = Engine::OpenCode("prov/model".into()).key();
    assert!(cache.stale("b", 1, &other), "another model may answer it");
    assert!(!cache.stale("empty", 1, &other), "no model can answer a session with no prompt");
    assert!(!cache.stale("a", 1, &other), "a real summary is not redone for a model change");
    let mut sent = String::new();
    index(
        &mut cache,
        vec![item("b", 1, "second")],
        |_| true,
        &Engine::OpenCode("prov/model".into()),
        |r| {
            sent = r.to_owned();
            fake(r)
        },
        || false,
        |_| {},
    )
    .unwrap();
    assert!(sent.contains("second"));
    assert_eq!(cache.digests["b"].summary, "Worked on second");
}
