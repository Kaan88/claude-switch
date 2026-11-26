# Contributing

Use Windows and Rust stable. See the [README](README.md#install) for the toolkit dependency.

```powershell
cargo build -p claude-switch
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Read [the design](docs/DESIGN.md) before changing launch or account behavior. Keep filesystem and launch decisions in `crates/core` and window code in `bins/claude-switch`. Test against temporary folders, not real Claude profiles. Keep credential and running-session files separate, never overwrite conflicting shared settings, and recheck whether a session is running before resuming it. Read only the bounded transcript slices described in the design.
