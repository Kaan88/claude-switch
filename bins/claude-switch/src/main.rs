//! Claude Switch: lists the Claude Code sessions of this computer and opens one in the CLI or
//! the desktop app, as the chosen account. `docs/DESIGN.md` says what and why.
//!
//! `--demo` uses made-up data and never writes personal settings; `--theme light|dark` forces
//! the look instead of following Settings. `cli_args` reads the flags and refuses unknown ones;
//! `--help` lists them all.
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
mod auto_handover;
mod cli_args;
mod compact_cli;
mod compact_dialog;
mod compact_open;
mod compact_runner;
mod conversation_demo;
mod conversation_draw;
mod conversation_search;
mod delete;
mod demo;
mod demo_terminal;
mod details;
mod dialogs;
mod fieldmouse;
mod folder_defaults;
mod handover_in_place;
mod handover_jobs;
mod handover_pipe;
mod list;
mod meaning;
mod opencode_tab;
mod opencode_workers;
mod recent_folders;
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
#[cfg(test)]
mod testing;
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
mod workers_tab;

use claude_switch_core::Roots;
use mdshared_ui::host::{self, Config};

use crate::source::Source;
use crate::state::AppState;

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    if args.next_if(|a| a == "compact").is_some() {
        std::process::exit(compact_cli::entry(args));
    }
    let options = match cli_args::parse(args, cfg!(debug_assertions)) {
        Ok(options) => options,
        Err(stop) => {
            let (text, code) = cli_args::message(&stop);
            cli_args::show(&text, code != 0);
            std::process::exit(code);
        }
    };
    let cli_args::Options {
        demo,
        demo_notifications,
        demo_usage_hover,
        demo_permission,
        demo_working,
        demo_terminal,
        demo_terminal_settings,
        forced_dark,
        usage_shape,
        usage_labels,
        handoff,
        demo_compact,
        demo_compact_long,
        demo_search,
        demo_offscreen,
        demo_workers,
        demo_jobs,
        shell,
        typed,
        list_tab,
    } = options;
    let (handoff, handoff_ack) = handoff.map_or((None, None), |(request, ack)| (Some(request), Some(ack)));
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
    // Claim the handover channel before restoring anything: only the window that holds it is the
    // main one. A window started for a handover gives it to the holder instead.
    let mut claim = if demo { handover_pipe::Claim::Failed(String::new()) } else { handover_pipe::claim() };
    if matches!(claim, handover_pipe::Claim::Taken)
        && let (Some(request), Some(ack)) = (&handoff, &handoff_ack)
    {
        if compact_cli::forward_to_running(request, ack) {
            std::process::exit(0);
        }
        claim = handover_pipe::claim(); // The holder went away meanwhile.
    }
    if !demo {
        // Background handover jobs a process never read (it died first); off the startup path.
        std::thread::spawn(compact_cli::clean_old_jobs);
    }
    let mut win_file = winfile::load(source.window_file().as_deref());
    if demo && demo_offscreen {
        win_file.place = Some(winfile::demo_offscreen());
    }
    let startup = if let (Source::Real(roots), Some(args)) = (&source, handoff) {
        match compact_cli::handoff(roots, args) {
            Ok(request) => Some(request),
            Err(e) => {
                if let Some(key) = handoff_ack.as_deref() {
                    compact_cli::acknowledge(key, false);
                }
                eprintln!("claude-switch compact: {e}");
                std::process::exit(1);
            }
        }
    } else {
        None
    };
    let mut state = AppState::new(source, forced_dark, win_file.clone());
    state.usage.demo_echo = demo && demo_usage_hover;
    state.session_view.demo_permission = demo && demo_permission;
    state.session_view.demo_working = demo && demo_working && !demo_permission;
    if let Some(preview) = demo_terminal {
        preview.apply(&mut state);
    }
    if demo_terminal_settings {
        demo_terminal::show_settings(&mut state);
    }
    match claim {
        handover_pipe::Claim::Mine(listener) => state.compact.listener = Some(listener),
        handover_pipe::Claim::Taken => state.restore.second_window = true,
        handover_pipe::Claim::Failed(reason) if !demo => state.compact.listen_failed = Some(reason),
        handover_pipe::Claim::Failed(_) => {}
    }
    state.compact.startup = startup;
    state.compact.startup_ack = handoff_ack;
    state.compact.demo = demo_compact.filter(|_| demo);
    state.compact.demo_long = demo && demo_compact_long;
    if demo && demo_workers {
        state.opencode_settings(true);
    }
    if demo && demo_jobs {
        state.start_demo_workers();
    }
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
