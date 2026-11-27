# Screen recorder

Records what Claude Code draws on its terminal, for the fixtures Chat's screen readers are tested
against (`bins/claude-switch/src/view/chat/question/fixtures`). Chat reads the CLI's screen before
every key, and none of what it reads is a protocol: when a new Claude Code version draws a prompt,
a menu or its input differently, the fixtures have to be recorded again. This tool starts the real
`claude` in a ConPTY the way Claude Switch starts a Claude tab, follows a script of keys and waits,
and writes snapshots of the screen.

Claude Code updates often. `recorder compat` records every script in `scripts/` from the installed
Claude Code and has Chat's own readers check each snapshot against `expect/`: see "After a Claude
Code update" below. It runs by hand on this desktop and daily in GitHub Actions.

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
- `REC_PROGRAM=<program>`: record this program instead of `claude` (the recorder's own `compat`
  screen was looked at this way).
- `REC_PROJECT=<folder>`: the session's transcript folder, for `tscopy`. It is the account's
  `projects` folder plus the work folder's absolute path with every `:`, `\` and `/` replaced by `-`.

## Scripts

One op per line; `#` starts a comment.

| Op | What it does |
| --- | --- |
| `start` | Waits for the input box; answers the "trust this folder" dialog and the question about an API key from the environment when they show. It never answers a sign-in or first-run screen: the script ends there. |
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

Lines that start with `#:` are for `recorder compat` (comments to a plain run): `#: args …` (more
arguments for `claude`; `{scripts}` is this folder's `scripts`), `#: size COLS ROWS` (100 x 40
without it) and `#: seconds N` (about how long the script takes, for the time left).

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

## After a Claude Code update

`recorder compat` finds out whether Chat still reads the installed Claude Code. For each script in
`scripts/` it starts `claude` in an empty scratch folder, plays the script and takes its snapshots;
then the app's own readers say what each snapshot is (the ignored test `cli_compat` in
`bins/claude-switch/src/view/chat/question/screens/compat.rs`), and that is compared with
`expect/<script>.expect`. Nothing is compared as text or pixels: a screen that moved or was
reworded passes as long as Chat reads the same thing from it.

The scripts cover every state Chat reads: the idle input at the start, after a turn and after
`/clear`; typed, pasted and wrapped input; a running tool and text typed meanwhile; Esc during a
turn and right after a send; a question, a typed answer, a form and its review; permission
prompts (Bash, Write, a subagent's, two calls of one message), a prompt answered No, a call a
deny rule refuses, a call auto mode blocks and the list of blocked calls with a call marked and
unmarked; plan approval; the rewind list; the panels of `/model`, `/config` and `/permissions`.

### On this desktop

    pwsh tools\cli-compat.ps1 -ConfigDir $env:USERPROFILE\.claude-accounts\<name>\cli

It builds the recorder and starts `recorder compat` with that account (`CLAUDE_CONFIG_DIR`; without
`-ConfigDir`, the signed-in account whose `projects` folder was written least recently). Other
switches: `-Only idle,plan` (some scripts), `-OnlyIfNewVersion` (do nothing when this Claude Code
version was already checked in full; the last one is kept in
`%LOCALAPPDATA%\claude-switch-cli-compat\last-checked.txt`), `-Model`, `-Out`, `-Plain`. Run it from
a terminal of at least 100 columns; with `CARGO_TARGET_DIR` set, both builds go there.

The run shows on a screen of its own:

- at the top the Claude Code version and the one checked last, the account (its folder's name), the
  model, the time, the estimated cost so far, and a bar with the scripts done and the time left;
- one row per script: queued, recording or checking (a spinner), `pass 6/6`, `FAIL 2/3` or
  `ERROR`, how long it took, and the first thing that does not match;
- under them the terminal being recorded, as Claude Code draws it;
- at the end a summary: scripts passed, failed and skipped, time, cost, where the report is, and
  for every state that does not match what Chat expected and what it read, with the recording.

Keys: Up and Down choose a script, Enter shows its states and the screen of the first one that
does not match (Up, Down, PgUp, PgDn scroll; Enter or Esc go back), `r` records the chosen script
again, `q` or Ctrl+C stops. However it ends, the scratch folder (`%TEMP%\cs-compat`) is removed,
and from the account's folder the scratch transcripts (`projects\…-cs-compat-work`, only that
folder) and the files named after those sessions (`plans`, `session-env`, `file-history`). What
stays in the account is Claude Code's own bookkeeping: the scratch folder's entry in `.claude.json`,
its lines in the prompt history, and a small file in `shell-snapshots` per session that used Bash
(they carry no session name, so they cannot be told from those of other sessions).

The recordings and `report.md` (the summary again, with a table of the scripts) and `summary.json`
go to a new folder under `%LOCALAPPDATA%\claude-switch-cli-compat\runs` (the newest five are kept);
the path is printed at the end. Exit code 0: every state reads as expected; 1: some do not; 2: the
check could not run. Output that is not a terminal, or `-Plain`, gives plain lines instead of the
screen: `ok        idle/start`, `MISMATCH  two/second  kind: expected "permission", got "…"`.

### In GitHub Actions

`.github/workflows/cli-compat.yml` runs daily and from the Actions tab ("Run workflow"). A first
job asks `downloads.claude.ai` (what the native installer installs; the npm registry as a second
source) for the newest version and looks for that version's marker in the Actions cache; only for
a new version, or when started by hand, a Windows job installs Claude Code with the installer,
builds the recorder, runs `recorder compat --ci --api-key-env ANTHROPIC_API_KEY`, uploads the
recordings as the artifact `cli-compat-<version>` and writes the report into the run's summary.
The version is the one installed (`claude --version`): when the installer installs another one
than the newest found, the run says so in a warning and the artifact, the issue and the marker
name the installed one. When states do not match, it opens the issue "Chat: screens changed in
Claude Code <version>" with the report, or updates it when it is already open; when it was
closed (the version was checked again after the cache let go of its marker), the report goes
there as a comment and the issue stays closed. The run stays green with a warning. A run
that could not check anything (the key is refused, the check does not build, Claude Code does not
start) is red and is tried again the next day.

It needs the repository secret `ANTHROPIC_API_KEY`: on GitHub, Settings > Secrets and variables >
Actions > New repository secret. Until it is there every run ends green with the notice "The
secret ANTHROPIC_API_KEY is not set". With a key there is no sign-in: the runner writes a config
folder of its own that marks the first-run screens as done and the key as accepted (`.claude.json`:
`hasCompletedOnboarding`, `customApiKeyResponses.approved` with the key's last 20 characters), and
`start` answers the folder trust dialog. A snapshot never holds a key: what the CLI shows of one is
starred out. Only the recording step has the key in its environment: the check is built in a step
before it, and the recorder removes the key (`ANTHROPIC_API_KEY` and the `--api-key-env` name)
from what cargo and the check's test program see, so no build script or proc macro can read it.

### What a run costs

The scripts make about 22 model requests (10 of the 12 scripts talk to the model; a failed script
is recorded once more before it counts). A full run with Haiku 5.5 on Claude Code 2.1.332 takes
three to five minutes and comes to about $0.11 at API prices (measured: 24 requests, two scripts
recorded twice): nearly all of it is Claude Code's own system prompt, sent with each request. On a
subscription account it counts against the plan's usage instead. The estimate on the screen is what the transcripts hold, so requests Claude
Code makes on the side (titles, auto mode's checks) are not in it.

### Reading a failure

Each line names a script, a state (the snapshot's name) and a key:

    MISMATCH  two/second  kind: expected "permission", got "permission-unread"

Open `<run>\two\second.txt` (the screen as text; `.ansi` is the same with styles, the fixture
form) and `<run>\two\log.txt` (the timeline). Three things can be behind it:

- The model did not do what the prompt asked (it asked back, called another tool): the screen
  shows it. Run that script again (`r`, or `-Only two`); a script that fails is already recorded
  twice.
- Claude Code draws the screen differently and a reader no longer takes it: fix the reader in
  `view/chat/question/screens.rs` or `view/chat/submission/screen.rs`, sanitise the recording into
  a fixture ("From a recording to a fixture") and add a test for it in `recorded.rs`. Keep the old
  fixture while both versions are in use.
- Chat reads the screen correctly and only the expectation is out of date (a choice was reworded,
  a panel has another row): change `expect/<script>.expect`.

`recording: expected "a snapshot", got "no recording"` means the script ended before that
snapshot: a `wait` ran out (`aborted.txt` is the screen at that moment, and the report names the
wait), usually because the screen before it already went wrong. A script stops there on purpose:
the keys after a wait were written for a screen that is not there.

### Scripts and expectations

A script is a file in `scripts/` (see "Scripts" above for the ops and the `#:` lines). Its
expectations are in `expect/<name>.expect`: a `[state]` section per snapshot to check, with
`key = value` lines; `#` starts a comment. Only the keys a section names are compared. In a value
`*` stands for any text (use it for what depends on the account, the path or the model's wording),
runs of blanks count as one and `\n` is a line break.

| Key | What the readers say |
| --- | --- |
| `kind` | `idle` (the empty input), `input` (it holds text), `working`, `question`, `review`, `permission`, `permission-unread` (looks like a prompt but cannot be answered), `blocked` (the list of calls auto mode blocked), `plan`, `menu`, `panel`, `rewind`, `rewind-actions`, `menu-unread`, `full-screen`, `unknown` |
| `input` | The one-row input's text; `-` when it is not read |
| `send`, `working`, `attention`, `esc` | `yes` or `no`: a message may be sent now; a turn runs; the tab asks for a permission; the panel's footer offers Esc |
| `mode` | The permission mode under the input: `Manual`, `Auto`, `Plan`, …; `-` for none |
| `title` | The question asked, a panel's title, the prompt a rewind quotes |
| `choices`, `count`, `highlight` | The rows joined with ` \| `, how many, the highlighted row's text |
| `body`, `request` | A permission prompt's rows above its question; all its rows from its top rule, as the card for a prompt no pending call covers shows them |
| `marks`, `pending`, `esc-does` | In the list of blocked calls: each call's mark (`none`, `approve`, `retry`), the row saying what Esc will do, the footer's words for it (`close`, `approve 1 blocked`); `title` is the tab's note and `body` the highlighted call's rows |
| `tell`, `keep` | The declining row of a permission prompt and of the plan approval |
| `notes`, `rows`, `answers` | A question's descriptions, a panel's rows, a review's `question = answer` pairs |
| `answer`, `lines`, `wrapped` | This text reads back exactly on the highlighted row; the input shows this text of several lines; this long line |

To write or renew expectations, look at what the readers make of a run's recordings, printed in
this form:

    CS_COMPAT_RECORDINGS=<run folder> cargo test -p claude-switch compat_facts -- --ignored --nocapture

and to check recordings again without recording (after fixing a reader):

    CS_COMPAT_RECORDINGS=<run folder> cargo test -p claude-switch cli_compat -- --ignored --nocapture

(The run folder as an absolute path; `CS_COMPAT_ONLY=two,plan` for some scripts,
`CS_COMPAT_SUMMARY=<file>` for the result as JSON, `CS_COMPAT_EXPECT=<folder>` for other
expectations than this folder's.) A new state Chat reads gets a script or a snapshot in one, a section in its `.expect` file, and,
when a reader needs it, a key in `compat.rs`. Keep prompts tiny and commands harmless: the model
works in the scratch folder, with the permission mode the script's `#: args` line gives.
