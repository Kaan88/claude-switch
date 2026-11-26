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
mod demo;
mod details;
mod dialogs;
mod list;
mod run;
mod session_marks;
mod session_panel;
mod source;
mod state;
mod tabs;
mod term;
mod termapp;
mod termkeys;
mod termtheme;
mod topbar;
mod usage;
mod usage_fetch;
mod usage_shape;
mod usage_ui;
mod view;
mod windowlook;
mod winfile;

use claude_switch_core::Roots;
use mdshared_ui::host::{self, Config};

use crate::source::Source;
use crate::state::AppState;

fn main() {
    let mut demo = false;
    let mut forced_dark = None;
    let mut usage_shape = None;
    let (mut shell, mut typed, mut list_tab) = (None, Vec::new(), false);
    let mut args = std::env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--demo" => demo = true,
            "--theme" => forced_dark = args.next().as_deref().and_then(termtheme::window_dark),
            "--shell" => shell = args.next(),
            "--type" => typed.extend(args.next()),
            "--sessions-tab" => list_tab = true,
            "--usage-shape" => {
                let account = if args.peek().is_some_and(|s| !s.starts_with('-')) { args.next() } else { None };
                usage_shape = Some(account);
            }
            _ => {}
        }
    }
    if let Some(account) = usage_shape {
        std::process::exit(usage_shape::run(demo, account.as_deref()));
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
    let win_file = winfile::load(source.window_file().as_deref());
    let mut state = AppState::new(source, forced_dark, win_file.clone());
    state.debug_shell = shell;
    state.debug_list = list_tab;
    let start = std::time::Instant::now();
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
