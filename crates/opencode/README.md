# opencode-cli

MIT-licensed, synchronous Rust helpers for the native OpenCode V2 CLI. GUI callers should run them on a background worker. The library has no GUI or HTTP client and depends only on `serde_json`.

```toml
opencode-cli = { git = "https://github.com/Kaan88/claude-switch.git" }
```

Use this dependency once the crate is pushed; for reproducible builds add the published commit as `rev`. Cargo finds the workspace package by name.

```rust
use std::{process::Command, time::Duration};

let program = opencode_cli::find_program();
let status = opencode_cli::status(|| program.as_ref().map(Command::new));
if let Some(program) = program {
    let models = opencode_cli::models(Command::new(program), Duration::from_secs(20), || false);
    // Model has separate `provider` and `model` fields; `id()` returns provider/model.
}
```

- `find_program` checks an absolute `OPENCODE_EXE` override, PATH and common native install directories. Pass your own `Command` to inject an executable or customize the environment/working directory.
- `run(command, args, timeout)` captures output without a console window and returns trimmed stdout or an error. Output is bounded; the direct child is killed and reaped on timeout. This is not a process-tree supervisor.
- `status` checks the path, version and running count. It checks `service status` before calling the API; `active_sessions(command)` probes the API directly, which OpenCode can use to start a missing service. An unrecognized response is `None`, not zero. Counting uses only entries whose `type` is `running`.
- `restart_service` falls back to `stop` then `start` on restart failure, and requires a valid `session.active` response afterward. Confirmation and any preflight recheck belong to the caller.
- `models` uses `opencode models` for IDs, including provider-qualified/nested model names. Clients that need the richer catalog or model variants can use `run` with `["api", "model.list"]`.
- `run_prompt` sends one UTF-8 prompt via stdin and takes a timeout and a cancellation callback (for example, reading an atomic flag). Its stderr is discarded. Caller options control persistence, format, tools and which server to use; private stdin does not imply a sandbox or no persistence.
