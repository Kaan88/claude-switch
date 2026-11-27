//! Records what Claude Code draws, for Chat's screen fixtures, and checks after an update that
//! Chat still reads it. See README.md.
//!
//! recorder <script> <workdir> <cols> <rows> <outdir> [claude args...]
//! recorder compat [options]
mod compat;
mod record;
mod tui;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "compat") {
        std::process::exit(compat::main(&args[2..]));
    }
    if args.len() < 6 {
        eprintln!(
            "recorder <script> <workdir> <cols> <rows> <outdir> [claude args...]\nrecorder compat [options]   (see README.md)"
        );
        std::process::exit(2);
    }
    record::main(&args);
}
