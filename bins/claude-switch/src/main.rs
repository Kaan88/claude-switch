//! Claude Switch: lists the Claude Code sessions of this computer and opens one in the CLI or
//! the desktop app, as the chosen account. `docs/DESIGN.md` says what and why.
//!
//! `--demo` uses made-up data and never writes personal settings; `--theme light|dark` forces
//! the look instead of following Settings.
//!
//! For testing the terminal tabs only (not shown anywhere): `--shell <program>` opens a tab that
//! runs the program at start, also in the demo, and each `--type <text>` writes text to it, the
//! first two seconds after the start and each further one two seconds later (`\r`, `\n`, `\t`,
//! `\e`, `\\` and `\xHH` are read as those characters); `--sessions-tab` then shows the list.

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod actions;
mod app;
mod app_draw;
mod archive;
mod archive_report;
mod attention;
mod compact_cli;
mod compact_dialog;
mod compact_open;
mod compact_runner;
mod conversation_demo;
mod conversation_draw;
mod conversation_search;
mod demo;
mod details;
mod dialogs;
mod folder_defaults;
mod list;
mod resume_tabs;
mod run;
mod session_marks;
mod session_panel;
mod session_size;
mod source;
mod state;
mod tabcontrols;
mod tabs;
mod term;
mod termapp;
mod termkeys;
mod termtheme;
mod topbar;
mod tray;
mod usage;
mod usage_fetch;
mod usage_shape;
mod usage_ui;
mod usage_warnings;
mod view;
mod windowlook;
mod winfile;

use claude_switch_core::Roots;
use mdshared_ui::host::{self, Config};

use crate::source::Source;
use crate::state::AppState;

fn main() {
    let mut demo = false;
    let mut demo_notifications = false;
    let mut forced_dark = None;
    let mut usage_shape = None;
    let mut usage_labels = false;
    let mut handoff = None;
    let mut demo_compact = None;
    let mut demo_search = None;
    let mut demo_offscreen = false;
    let (mut shell, mut typed, mut list_tab) = (None, Vec::new(), false);
    let mut args = std::env::args().skip(1).peekable();
    if args.next_if(|a| a == "compact").is_some() {
        std::process::exit(compact_cli::entry(args));
    }
    while let Some(a) = args.next() {
        match a.as_str() {
            "--demo" => demo = true,
            "--demo-notifications" => demo_notifications = true,
            "--theme" => forced_dark = args.next().as_deref().and_then(termtheme::window_dark),
            "--shell" => shell = debug_value(cfg!(debug_assertions), args.next()),
            "--type" => typed.extend(debug_value(cfg!(debug_assertions), args.next())),
            "--sessions-tab" => list_tab = true,
            "--demo-search" => demo_search = args.next(),
            "--demo-compact" => demo_compact = Some(false),
            "--demo-compact-preview" => demo_compact = Some(true),
            "--demo-offscreen" => demo_offscreen = true,
            "--compact-open-window" => handoff = compact_cli::window_args(&mut args),
            "--usage-shape" => {
                if args.next_if(|s| s == "--labels").is_some() {
                    usage_labels = true;
                }
                let account = if args.peek().is_some_and(|s| !s.starts_with('-')) { args.next() } else { None };
                usage_shape = Some(account);
            }
            "--labels" => usage_labels = true,
            _ => {}
        }
    }
    if let Some(account) = usage_shape {
        std::process::exit(usage_shape::run(demo, account.as_deref(), usage_labels));
    }
    let source = if demo {
        Source::Demo
    } else {
        match Roots::from_env() {
            Some(roots) => Source::Real(roots),
            None => {
                eprintln!("claude-switch: USERPROFILE or APPDATA is not set");
                std::process::exit(1);
            }
        }
    };
    let mut win_file = winfile::load(source.window_file().as_deref());
    if demo && demo_offscreen {
        win_file.place = Some(winfile::demo_offscreen());
    }
    let startup = if let (Source::Real(roots), Some(args)) = (&source, handoff) {
        match compact_cli::handoff(roots, args) {
            Ok(request) => Some(request),
            Err(e) => {
                eprintln!("claude-switch compact: {e}");
                std::process::exit(1);
            }
        }
    } else {
        None
    };
    let mut state = AppState::new(source, forced_dark, win_file.clone());
    state.compact.standalone_window = startup.is_some();
    state.compact.startup = startup;
    state.compact.demo = demo_compact.filter(|_| demo);
    state.debug_shell = shell;
    state.notifications.demo = demo && demo_notifications;
    state.debug_list = list_tab || state.compact.demo.is_some();
    let start = std::time::Instant::now();
    if let Some(query) = demo_search.filter(|_| demo) {
        state.debug_list = true;
        state.search.insert(&query);
        state.toggle_conversation_search(start);
    }
    state.debug_type = typed
        .iter()
        .enumerate()
        .map(|(k, t)| (start + std::time::Duration::from_secs(2 * (k as u64 + 1)), termkeys::unescape(t)))
        .collect();
    let config = Config::new("Claude Switch")
        .size(win_file.w, win_file.h)
        .min_size(760.0, 480.0)
        .app_id("claude-switch")
        .dark(state.pal().dark);
    if let Err(e) = host::run(config, state) {
        eprintln!("claude-switch: {e}");
        std::process::exit(1);
    }
}

fn debug_value(debug_build: bool, value: Option<String>) -> Option<String> {
    value.filter(|_| debug_build)
}

#[test]
fn shell_and_typed_arguments_are_discarded_in_release() {
    assert_eq!(debug_value(false, Some("ignored".into())), None);
    assert_eq!(debug_value(true, Some("example".into())).as_deref(), Some("example"));
}
