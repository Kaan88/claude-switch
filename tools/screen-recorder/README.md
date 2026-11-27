# Screen recorder

Records what Claude Code draws on its terminal, for the fixtures Chat's screen readers are tested
against (`bins/claude-switch/src/view/chat/question/fixtures`). Chat reads the CLI's screen before
every key, and none of what it reads is a protocol: when a new Claude Code version draws a prompt,
a menu or its input differently, the fixtures have to be recorded again. This tool starts the real
`claude` in a ConPTY the way Claude Switch starts a Claude tab, follows a script of keys and waits,
and writes snapshots of the screen.

It is a crate of its own, outside the workspace (it has its own `[workspace]` table), so the
release build, `cargo test --workspace` and the notices script never see it. It uses the same
toolkit revision as the app: raise both together.

## Build and run

From this folder, in Git Bash:

    export CARGO_TARGET_DIR=E:/cs-target/recorder CARGO_BUILD_JOBS=2
    cargo build
    REC=$CARGO_TARGET_DIR/debug/recorder.exe
    $REC scripts/idle.txt WORKDIR 100 40 out/idle --model haiku

`WORKDIR` is a scratch folder (never a real project: the model may write files there). Pass
`--model haiku` and keep prompts tiny; each run is a real, paid session of the account the shell is
signed in to (`CLAUDE_CONFIG_DIR` is passed on; every other `CLAUDE_*` and `ANTHROPIC_*` variable is
removed, as the app does). Other arguments go to `claude` as given, for example
`--permission-mode default`, `--permission-mode plan`, `--settings FILE`.

Environment:

- `REC_PLAIN=1`: no `TERM` and none of a shell's other terminal variables, as in a tab of a program
  started from the Start menu before 0.1.3. Claude Code then draws without Unicode (`>`, `[ ]`,
  `√`). Without it the recorder sets `TERM=xterm-256color`, as the app now does.
- `REC_ENV="NAME=value;NAME=value"`: more variables for `claude`.
- `REC_PROJECT=<folder>`: the session's transcript folder, for `tscopy`. It is the account's
  `projects` folder plus the work folder's absolute path with every `:`, `\` and `/` replaced by `-`.

## Scripts

One op per line; `#` starts a comment.

| Op | What it does |
| --- | --- |
| `start` | Waits for the input box; answers the "trust this folder" dialog when it shows. |
| `sleep MS` | |
| `send TEXT` | Bytes to the terminal. Escapes: `\e` Esc, `\r` Enter, `\n`, `\t` Tab, `\s` space, `\xHH`, `\\`. Arrows `\e[A` `\e[B` `\e[C` `\e[D`, Shift+Tab `\e[Z`, Ctrl+C `\x03`, Ctrl+U `\x15`, a paste `\e[200~…\e[201~` with `\r` between its lines. |
| `wait TEXT MS` | Until some row contains TEXT, at most MS. |
| `gone TEXT MS` | Until no row contains TEXT. |
| `quiet MS MAX` | Until the terminal sent nothing for MS. |
| `snap NAME` | Writes `NAME.txt` (rows, first line `cursor ROW COL`) and `NAME.ansi` (the fixture form). |
| `idle MS NAME` | Logs every burst of output for MS (is the screen quiet?), then snaps. |
| `watch MS NAME` | Snaps `NAME-001`, `NAME-002`, … once a second while the screen changes. |
| `resize COLS ROWS` | |
| `tscopy NAME` | Copies the newest transcript of `REC_PROJECT` as it is on disk now. |

Type a prompt and its Enter as separate sends with a pause between them (`send …`, `sleep 800`,
`send \r`); the CLI takes text and Enter in one write for a paste. The model does not always do
what a prompt asks: look at the snapshot (`cat out/x/NAME.txt`) before trusting it. `log.txt` has
the timeline, with every bell and title.

## From a recording to a fixture

1. Replace personal text in the `.ansi` file: the user name, the work folder's path, the account's
   plan and folder names, anything from MCP servers or skills.
2. Copy it to the fixtures folder with LF line endings and add its constant in
   `view/chat/question/screens/recorded.rs`. The loader there (`recorded`) reads the header lines
   (`@size`, `@wrapped`, `@modes`).
3. To see what the gates make of a whole folder of recordings without making fixtures:
   `CS_PROBE_DIR=<folder of .ansi files> cargo test -p claude-switch probe_recordings -- --ignored --nocapture`.
4. Delete the scratch sessions the runs left in the account's `projects` folder.

`docs/DESIGN.md` ("Chat over the terminal") lists which states were recorded and when.
