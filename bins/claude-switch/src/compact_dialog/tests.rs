use super::*;
use crate::dialogs;

#[test]
fn preview_names_the_continuation_without_editing_markdown_and_refuses_a_blank_name() {
    let mut st = app();
    let now = Instant::now();
    let source = st.selected_session().unwrap().clone();
    assert!(st.marks.rename(&source.id, &source.file, "Custom source"));
    let mut dialog = Compact::new(&st, source, None);
    dialog.apply(&mut st, Ok(Update::Preview("## Goal\n\nContinue the work.".into())));
    assert_eq!(dialog.new_name.text(), "Custom source · continued");
    dialog.key(&KeyName::Tab, Mods::default(), None);
    dialog.text_input("My edited name 😺");
    assert_eq!(dialog.new_name.text(), "My edited name 😺");
    assert_eq!(dialog.preview.text, "## Goal\n\nContinue the work.");
    dialog.new_name.select_all();
    dialog.key(&KeyName::Backspace, Mods::default(), None);
    assert!(dialog.click(&mut st, now, OK, 0.0), "invalid names leave the editor open");
    assert!(dialog.message.as_ref().unwrap().1);
    dialog.text_input("A valid name");
    assert!(!dialog.click(&mut st, now, OK, 0.0));
    assert!(st.tabs.list.is_empty(), "demo does not launch Claude");
}
#[test]
fn saved_agent_is_preselected_and_can_be_overridden_with_plain_claude() {
    let mut st = app();
    st.config.compact.set_default_agent("main-reviewer").unwrap();
    let mut dialog = Compact::new(&st, st.selected_session().unwrap().clone(), Some("main"));
    dialog.apply(&mut st, Ok(Update::Preview("handover".into())));
    assert_eq!(dialog.agent_name(), Some("main-reviewer"), "even before discovery");
    let now = Instant::now();
    dialog.click(&mut st, now, AGENT, 0.0);
    assert_eq!(dialog.count(), 2, "the discovered entry replaces the settings-only entry");
    assert_eq!(dialog.key(&KeyName::Enter, Mods::default(), None), Some(OPTION + 1));
    assert!(dialog.agents[0].description.contains("Synthetic"));
    dialog.click(&mut st, now, OPTION, 0.0);
    assert_eq!(dialog.agent_name(), None, "Default (Claude) explicitly adds no flag");
    dialog.click(&mut st, now, ACCOUNT, 0.0);
    dialog.click(&mut st, now, OPTION + 1, 0.0);
    assert_eq!(dialog.agent_name(), Some("main-reviewer"), "an account change resets to Settings");
    dialog.click(&mut st, now, AGENT, 0.0);
    assert_eq!(dialog.count(), 2, "the other account lacks it: not offered");
    assert_eq!(dialog.agent_name(), None, "and not passed");
    let (notice, error) = dialog.message.clone().unwrap();
    assert!(notice.contains("“main-reviewer”") && notice.contains("plain Claude") && !error, "{notice}");
}

#[test]
fn a_missing_saved_agent_is_checked_before_launch_even_without_opening_the_picker() {
    let mut st = app();
    st.config.compact.set_default_agent("deepseek-flash").unwrap();
    let mut dialog = Compact::new(&st, st.selected_session().unwrap().clone(), Some("main"));
    assert_eq!(dialog.agent_name(), Some("deepseek-flash"));
    let notice = dialog.check_agents_now(&st).unwrap();
    assert!(notice.contains("deepseek-flash"), "{notice}");
    assert_eq!(dialog.agent_name(), None);
    assert_eq!(dialog.check_agents_now(&st), None, "checked once");
    st.config.compact.set_default_agent("main-reviewer").unwrap();
    let mut dialog = Compact::new(&st, st.selected_session().unwrap().clone(), Some("main"));
    assert_eq!(dialog.check_agents_now(&st), None);
    assert_eq!(dialog.agent_name(), Some("main-reviewer"), "a present default is kept");
}

#[test]
fn creating_a_handover_dialog_reads_another_windows_saved_default() {
    let scratch = crate::testing::Scratch::new("cs-agent-default");
    let root = scratch.to_path_buf();
    let roots = claude_switch_core::Roots { home: root.join("home"), appdata: root.join("appdata") };
    let mut st = app();
    let session = st.selected_session().unwrap().clone();
    st.source = Source::Real(roots.clone());
    assert!(st.config.compact.default_agent.is_none());
    claude_switch_core::config::update_compact(&roots, |p| p.set_default_agent("reviewer")).unwrap();
    // The history route constructs the same picker without finding or running OpenCode.
    st.ask_compact_for(session, None, true);
    assert_eq!(compact(&st).agent_name(), Some("reviewer"));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn preview_offers_default_first_and_changing_account_clears_the_explicit_agent() {
    let mut st = app();
    let now = Instant::now();
    let Some(Dialog::Compact(mut dialog)) = st.dialog.take() else { panic!("expected handover dialog") };
    dialog.apply(&mut st, Ok(Update::Preview("An editable handover".into())));
    assert_eq!(dialog.agent_label(), "Default (Claude)");
    assert_eq!(dialog.agent_name(), None);
    dialog.click(&mut st, now, AGENT, 0.0);
    assert_eq!(dialog.count(), 2);
    assert_eq!(dialog.key(&KeyName::Enter, Mods::default(), None), Some(OPTION));
    dialog.key(&KeyName::Down, Mods::default(), None);
    let chosen = dialog.key(&KeyName::Enter, Mods::default(), None).unwrap();
    dialog.click(&mut st, now, chosen, 0.0);
    assert_eq!(dialog.agent_name(), Some("main-reviewer"));
    dialog.click(&mut st, now, ACCOUNT, 0.0);
    dialog.click(&mut st, now, OPTION + 1, 0.0);
    assert_eq!(dialog.agent_name(), None);
    dialog.click(&mut st, now, AGENT, 0.0);
    assert_eq!(dialog.agents[0].name, "work-reviewer");
    assert_eq!(dialog.preview.text, "An editable handover");
}

fn app() -> AppState {
    let mut st = AppState::new(Source::Demo, Some(false), Default::default());
    st.apply_scan(crate::demo::scan(&st.config));
    st.select_first();
    st.command(Instant::now(), COMMAND, None);
    st
}

fn compact(st: &AppState) -> &Compact {
    let Some(Dialog::Compact(dialog)) = &st.dialog else { panic!("expected handover dialog") };
    dialog
}

#[test]
fn real_source_preview_and_cancel_leave_no_handover_history() {
    let scratch = crate::testing::Scratch::new("cs-preview-cancel");
    let root = scratch.to_path_buf();
    std::fs::create_dir(&root).unwrap();
    let roots = claude_switch_core::Roots { home: root.clone(), appdata: root.clone() };
    let mut st = app();
    let Some(Dialog::Compact(mut dialog)) = st.dialog.take() else { panic!("expected handover dialog") };
    let id = dialog.session.id.clone();
    st.source = Source::Real(roots.clone());
    dialog.apply(&mut st, Ok(Update::Preview("private unedited preview".into())));
    assert!(dialog.saved.is_none());
    dialog.preview.set("redacted".into());
    st.dialog = Some(Dialog::Compact(dialog));
    dialogs::click(&mut st, Instant::now(), CANCEL, 0.0);
    assert!(st.dialog.is_none());
    assert!(compact::list(&roots, &id).unwrap().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn demo_requires_provider_consent_and_preview_before_the_open_action() {
    let mut st = app();
    let now = Instant::now();
    let click = |st: &mut AppState, id| dialogs::click(st, now, id, 0.0);
    assert!(compact(&st).phase == Phase::Setup);
    assert!(compact(&st).focus.text.starts_with("No code here."));
    assert!(compact(&st).focus_label().contains("non-code work, picked for this session"));
    let mut edited = compact(&st).focus.text.clone();
    edited.push_str(" Keep the glossary.");
    if let Some(Dialog::Compact(dialog)) = &mut st.dialog {
        dialog.focus.set(edited);
    }
    assert_eq!(compact(&st).focus_label(), "Focus: your own", "an edit makes it the user's own");
    click(&mut st, MODEL);
    click(&mut st, OPTION + 1);
    assert_eq!(st.config.compact.model, "demo/summary-thorough");
    assert!(st.config.compact.consents.is_empty());
    click(&mut st, ACCOUNT);
    click(&mut st, OPTION + 1);
    click(&mut st, OK);
    assert!(compact(&st).phase == Phase::Consent);
    assert!(st.config.compact.consents.is_empty());
    assert!(st.tabs.list.is_empty());
    click(&mut st, OK);
    assert!(compact(&st).phase == Phase::Preview);
    assert_eq!(compact(&st).accounts[compact(&st).account].0, "work");
    assert!(st.config.compact.consents.contains("demo"));
    assert!(compact(&st).job.is_none());
    let before = claude_switch_core::compact::size(&compact(&st).preview.text);
    dialogs::text_input(&mut st, "edit ");
    assert_eq!(claude_switch_core::compact::size(&compact(&st).preview.text).0, before.0 + 5);
    assert!(st.tabs.list.is_empty());
    click(&mut st, OK);
    assert!(st.dialog.is_none());
    assert!(st.tabs.list.is_empty(), "demo never starts Claude");
    st.forget_handover_consents();
    assert!(st.config.compact.consents.is_empty());
}

#[test]
fn delete_the_original_is_off_each_time_and_toggles_by_click_or_alt_d() {
    let mut st = app();
    let now = Instant::now();
    let source = st.selected_session().unwrap().clone();
    let mut dialog = Compact::new(&st, source.clone(), None);
    assert!(!dialog.delete_original, "off by default");
    dialog.apply(&mut st, Ok(Update::Preview("## Goal\n\nContinue.".into())));
    assert!(dialog.click(&mut st, now, DELETE_ORIGINAL, 0.0));
    assert!(dialog.delete_original);
    let alt = Mods { alt: true, ..Mods::default() };
    assert_eq!(dialog.key(&KeyName::Char('d'), alt, None), Some(DELETE_ORIGINAL));
    // The demo opens nothing and deletes nothing, and says so.
    assert!(!dialog.click(&mut st, now, OK, 0.0));
    assert!(st.by_id.contains_key(&source.id));
    assert!(st.toasts.find(|t| t.contains("nothing was")).is_some());
    let again = Compact::new(&st, source, None);
    assert!(!again.delete_original, "not remembered between uses");
}

/// The review at `w` x `h` and `scale`: the footer's buttons, by id, as drawn.
fn draw_review(dialog: &mut Compact, w: f32, h: f32, scale: f32, shot: Option<&str>) -> Vec<(Rect, Target)> {
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::{Gfx, Painter};
    let mut gfx = Gfx::new();
    let mut bmp = Bitmap::new((w * scale) as u32, (h * scale) as u32);
    let mut hits = Vec::new();
    {
        let mut p = Painter::new(&mut bmp, &mut gfx, scale);
        p.fill(Rect::new(0.0, 0.0, w, h), mdshared_ui::theme::DARK.page);
        let mut d = dialogs::Draw::for_test(&mut hits, mdshared_ui::theme::DARK);
        dialog.draw(&mut d, &mut p, w, h);
    }
    if let (Some(shot), Ok(dir)) = (shot, std::env::var("CS_SHOT_DIR")) {
        bmp.write_png(&std::path::Path::new(&dir).join(format!("{shot}.png"))).unwrap();
    }
    hits
}

#[test]
fn review_footer_fits_from_the_smallest_window_to_a_wide_one_at_every_scale() {
    use mdshared_ui::overlay::{DIALOG_FOOTER, DIALOG_PAD, dialog_card};
    let mut st = app();
    let mut dialog = Compact::new(&st, st.selected_session().unwrap().clone(), None);
    let text = "## Goal\n\n> Localize Sfera subtitle projects into Turkish, working from the English EMT plus German \
                audio evidence, then verify and write by code. Two jobs completed back to back.\n";
    dialog.apply(&mut st, Ok(Update::Preview(text.repeat(4))));
    let footer_ids = [OK, OPENCODE, CANCEL, BACK, COPY, SAVE, MORE];
    // 620 px is narrower than the window can be made: it stands for a card with less room.
    for (w, h) in [(620.0, 480.0), (760.0, 480.0), (900.0, 600.0), (1400.0, 900.0)] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let shot = (scale == 1.0).then(|| format!("cs-review-{w}x{h}"));
            let hits = draw_review(&mut dialog, w, h, scale, shot.as_deref());
            let card = dialog_card(760.0_f32.min(w - 28.0), 480.0_f32.min(h - 24.0), w, h);
            let buttons: Vec<_> = hits
                .iter()
                .filter_map(|(r, t)| match t {
                    Target::Dialog(id) if footer_ids.contains(id) => Some((*id, *r)),
                    _ => None,
                })
                .collect();
            for (id, r) in &buttons {
                assert!(
                    r.x >= card.x + DIALOG_PAD - 0.5 && r.right() <= card.right() - DIALOG_PAD + 0.5,
                    "{w}x{h}@{scale}: {id} {r:?}"
                );
                assert!(r.y >= card.bottom() - DIALOG_FOOTER && r.bottom() <= card.bottom(), "{w}x{h}@{scale}: {id}");
            }
            for (i, (a, ra)) in buttons.iter().enumerate() {
                for (b, rb) in &buttons[i + 1..] {
                    assert!(ra.right() <= rb.x || rb.right() <= ra.x, "{w}x{h}@{scale}: {a} overlaps {b}");
                }
            }
            let has = |id| buttons.iter().any(|(b, _)| *b == id);
            assert!(has(OK) && has(OPENCODE) && has(CANCEL) && has(BACK), "{w}x{h}");
            // Copy and Save as… are either both shown or both behind More.
            assert!((has(COPY) && has(SAVE) && !has(MORE)) || (!has(COPY) && !has(SAVE) && has(MORE)), "{w}x{h}");
            if w >= 760.0 {
                assert!(has(COPY), "the smallest window shows every action: {w}x{h}");
            }
        }
    }
    // A narrower card folds them; More offers both.
    draw_review(&mut dialog, 620.0, 480.0, 1.0, None);
    assert!(dialog.more_at.is_some());
    assert!(dialog.click(&mut st, Instant::now(), MORE, 0.0));
    let Some((crate::state::MenuKind::HandoverMore, _)) = &st.menu else { panic!("More opens its menu") };
}

#[test]
fn footer_buttons_narrow_to_their_labels_before_they_overflow() {
    let sizes = [(120.0, 80.0), (96.0, 60.0), (96.0, 50.0)];
    assert_eq!(dialogs::fit(&sizes, 400.0, 8.0), vec![120.0, 96.0, 96.0], "room: natural widths");
    let narrowed = dialogs::fit(&sizes, 250.0, 8.0);
    assert!(narrowed.iter().sum::<f32>() + 16.0 <= 250.0, "{narrowed:?}");
    assert!(narrowed.iter().zip(&sizes).all(|(w, s)| *w >= s.1 && *w <= s.0));
    assert_eq!(dialogs::fit(&sizes, 10.0, 8.0), vec![80.0, 60.0, 50.0], "never below a label");
}
