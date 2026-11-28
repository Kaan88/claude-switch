# claude-switch: design

A small Windows desktop app that knows the owner's Claude accounts, lists every Claude Code session on
this computer with its working folder, and opens one with a click: in the Claude Code CLI or in the Claude
desktop app, as the chosen account. A session can be carried to the other app or the other account.

Built with the owner's toolkit (`markdown-shared-rs`: `mdshared-ui` with the `host` window loop,
`mdshared-app`, `mdshared-term`, `mdshared-pty`). Two crates:

- `crates/core` (`claude_switch_core`): everything that is not a window. Plain data and functions, tested
  against temporary folders. No toolkit dependency. `serde_json` is its one dependency.
- `bins/claude-switch`: the window.

The window's command line is read by `cli_args`. An unknown option, a stray word or a missing or
unreadable value is refused with a short usage text and exit code 2, so a typo such as `--dmeo`
cannot start the real app in place of the demo. `--help` (also `-h`, `/?`) lists every option,
the testing, diagnostic and internal ones under "Hidden" headings, and exits 0. The text goes to the
console the app was started from, or to a message box when there is none. `compact` is read first
and keeps its own parser.

## Facts this is built on (checked on this computer, 2026-11-26)

**Transcripts.** Every session, CLI or desktop, is one file
`<config dir>/projects/<encoded folder>/<session id>.jsonl`, one JSON object per line. The session id is the
file stem (a UUID). Lines of use:

- most lines carry `"cwd"` (the working folder), `"sessionId"`, `"timestamp"` (ISO), `"entrypoint"`
  (`"claude-desktop"` for a session run by the desktop app, `"cli"` or another word for the terminal),
  `"version"`, `"gitBranch"`;
- `{"type":"custom-title","customTitle":"…"}` and `{"type":"ai-title","aiTitle":"…"}`: the title; the last
  one in the file wins, custom before ai;
- `{"type":"user","message":{"role":"user","content": "…" | [{"type":"text","text":"…"},…]}}`: a prompt;
  the first one that is not `isMeta` and does not start with `<` is the fallback title;
- lines with `"isSidechain":true` and files under a `subagents` folder are not sessions of their own.

Files can be tens of megabytes: the session index reads at most the first 256 KiB and the last
256 KiB. Entries are internal to Claude Code and may change: a line that does not parse, or lacks a
field, is skipped, never an error.

Refreshes reuse parsed transcripts and Desktop records when the open file's identity, size and
precise modification time match; entries no longer visited are evicted. The full scan and row
activity poll check the stamp on the same handle before and after reading. An unstable read is
discarded rather than assigning its parsed result a newer stamp.

**Desktop records.** The desktop app keeps one JSON file per session of its own:
`<desktop data dir>/claude-code-sessions/<account uuid>/<organisation uuid>/local_<uuid>.json` with
`sessionId` (`local_…`), `cliSessionId` (the transcript's id), `cwd`, `title`, `model`, `isArchived`,
`lastActivityAt` (ms), `createdAt` (ms), `priorCliSessionIds` (earlier transcripts of the same desktop
session). The default desktop data dir is `%APPDATA%\Claude`.

**Running sessions.** `<config dir>/sessions/<pid>.json`:
`{"pid":…, "sessionId":"<transcript id>", "cwd":"…", "kind":"interactive", "entrypoint":"claude-desktop"|…,
"hostSessionId":"local_…"}`. The file can outlive its process: a session counts as running only while
that pid is alive.

**Accounts.**

- CLI: `CLAUDE_CONFIG_DIR=<dir>` gives the CLI its own login (`<dir>/.credentials.json`), its own
  `<dir>/.claude.json` (which holds `oauthAccount` with `accountUuid`, `emailAddress`, `displayName`,
  `organizationName`, `planDisplayName`, and the user-scope `mcpServers`), and its own `projects`,
  `sessions`, `settings.json`, `skills`, `agents`, `plugins`. Without the variable the CLI uses
  `%USERPROFILE%\.claude`, and its `.claude.json` is `%USERPROFILE%\.claude.json` (beside the folder, not
  in it). The Accounts dialog reads these per-account records directly and checks that the credentials
  file exists without opening it. Usage is a separate path described below. `claude auth login` signs in.
- Desktop: Switch checks Windows' registration of `Claude_pzs8sxrjxfjjc` for the current user and
  its exact `app\Claude.exe` in a Store or developer-signed package, never a development-mode
  registration. A present embedded signature must pass offline Authenticode validation for
  Anthropic, PBC; when absent, trust comes from Windows' signed-package registration. Switch
  starts its verified **package-specific** execution alias, not the inner executable or the
  user-selectable alias on `PATH`. That activates the package with native arguments and the
  explicitly scrubbed child environment. Switch also
  checks the per-user Squirrel install (`%LOCALAPPDATA%\AnthropicClaude\app-<version>\Claude.exe`,
  newest version first) and fixed `Claude` or `AnthropicClaude` folders under Local AppData's
  Programs and Program Files. Standalone executables require an embedded, Windows-trusted
  Authenticode signature from Anthropic, PBC. The desktop launch worker verifies the chosen
  candidate once and holds the checked files through process creation. It does not run the
  `claude:` protocol, an updater, or a shell. A further account passes its own
  `--user-data-dir=<dir>` argument, not a different `CLAUDE_CONFIG_DIR`.
  The Code tab of every copy writes transcripts to `%USERPROFILE%\.claude\projects`.
- `claude --resume <id>` continues a session (it finds it from any folder, but run it in the session's
  folder). `claude --desktop --resume <id>` hands a CLI session to the desktop app and exits.
- The same session open in two places at once loses messages. Never start a session that is running.
- Resuming under another account works on the same transcript, but the prompt cache belongs to the
  account: the first request re-reads the whole conversation at full price.

## Accounts in this app

The first account is the one that exists: CLI folder `%USERPROFILE%\.claude`, desktop data dir
`%APPDATA%\Claude`. Each further account gets `%USERPROFILE%\.claude-accounts\<slug>\cli` and
`…\<slug>\desktop`.

A further account **shares** with the first, so nothing has to be kept in step by hand:

- linked into its CLI folder (directory junctions or hard links):
  `projects` (transcripts and memory), `skills`, `agents`, `plugins`, `mcp-servers`, `settings.json`,
  `CLAUDE.md`, `keybindings.json`;
- the first account's `mcpServers` seeds a missing secondary `.claude.json`; existing Claude-owned
  files are never overwritten, since Claude doesn't participate in the app's locks;
- never shared: `.credentials.json`, `oauthAccount`, `sessions`.

A hard-linked file stops being a link when a program saves it by writing a temporary file and renaming
it. Before every launch missing links are provisioned. Existing copies, even identical ones, are left
as conflicts rather than risking loss of a concurrent edit.

Found on this computer and allowed for: the desktop records' folder also holds JSON files that are not
session records (only files with a `cliSessionId` count); `sessions/` also holds `<pid>.<hash>.key`
files (only numeric `<pid>.json` is read); entrypoints seen are `cli`, `sdk-cli`, `sdk-ts` and
`claude-desktop`.

## Usage

**Show usage** (Settings → Usage, on by default) controls both the figures and the credential read.
On demand, a worker reads only `claudeAiOauth.accessToken` and `expiresAt` from each account's
CLI `.credentials.json`. Claude Switch never refreshes, writes or deletes credentials. Expired
tokens are never sent; the window keeps any previous figures and says “Open a session to refresh.”

WinHTTP sends the token as a bearer header only to `https://api.anthropic.com/api/oauth/usage`,
with normal Windows TLS certificate validation, no proxy, no redirects, and a 10-second request
deadline. There is no URL override outside tests. The token lives only for the request; it is not
part of app state, error details, logs, UI, clipboard or storage. Responses are reduced to the
fields described below; errors carry only a kind, never response bodies or headers.

Opening the window, account selector or Accounts page can request a reading if it is at least five
  minutes old. The selected-account usage popup can do the same for that account. The manual Refresh
  button allows a successful reading to be updated after 30 seconds. After a failure both paths wait
  at least five minutes, or 30 minutes for HTTP 429. If the account is not signed in or its local
  login has expired, Refresh and an embedded sign-in tab ending can retry immediately, because
  the previous attempt never sent a request. If a usage read is in flight when the tab ends, Switch
  rechecks after that read finishes; normal HTTP cooldowns still apply. External sign-ins can use
  Refresh. Failure cooldowns are in memory, not shared across
processes or saved on exit. Only [background usage warnings](#background-notices-and-notification-area)
can request a periodic refresh. Turning the setting
off cancels pending work and prevents new credential reads; a request already sent cannot be unsent.
Demo uses synthetic figures and does not read or write the usage cache, open credentials or use the network.

For local debugging, `--usage-shape [account]` makes one request without a window or cache access.
Use the account slug, or omit it for the selected account. It still requires **Show usage** and is
unavailable in demo mode. Stdout contains only member names, JSON types and array lengths; a member
name containing the bearer is redacted. Headers are never printed.
With `--labels`, the diagnostic may also print short simple strings for `kind`, `group`, `severity`,
`key` and `display_name` (up to 40 ASCII letters, digits, spaces, dots, dashes or underscores),
never a string containing the bearer. Without `--labels` no values are printed.

The app uses the nonempty `limits` array in preference to the older named limits. If it has no
usable Session limit, a numeric legacy `five_hour` can fill that gap; if neither has a figure, the
usage panels show Session as not reported. Valid inactive entries are kept: a limit still belongs
in the display even when it is not the one currently binding.
Both 80% usage and non-normal severity trigger the warning colour. “This week by app” shows up
to six named rows, numeric `percent_of_limit` values largest first, followed by named rows without
a figure (`—`, not reported). We interpret the undocumented field as a share of the weekly allowance,
not a share of the usage so far; Switch does not substitute a differently named measure.

`%APPDATA%\claude-switch\usage.json` keeps limit labels, slots, percentages, normalized reset times,
severity, the displayed breakdown and the reading time by account. It is replaced via a separate
temporary file and rename. Old readings still load. A failed write leaves the snapshot unsaved and
retries after 5, 10, 20 and 40 seconds; a new reading can start another attempt after that.
The plan name
comes from the existing account status (`oauthAccount.planDisplayName`); there is no renewal date.
The Anthropic endpoint is undocumented and can change or disappear.

A missing settings file is a first run. An existing damaged settings file turns usage off and blocks
ordinary settings writes. **Reset settings** backs up the exact old bytes beside the replacement and
keeps usage off until explicitly enabled. If the file cannot be backed up, reset refuses to replace it.
To restore a backup, close Switch, keep a copy of the replacement, and put the chosen
`config.broken.<pid>.<timestamp>.json` back at the original `config.json` path.
App-owned JSON saves refuse reparse-point destinations and create temporary files exclusively. Marks
compare and replace under a cross-process lock; batch naming refuses unreadable or changed marks and
rechecks the confirmed session selection before reading excerpts.

## The safeguard

Before resuming or carrying a session, the running sessions of every account are read again, including
those in a removed account's remaining CLI folder. A session that is
running anywhere is not started a second time; the app says where it runs (which app, which pid).
Several different sessions on one account at once are fine.

**Open a copy.** The button under the "Open elsewhere" bar and the "Open a copy in CLI" command palette
entry use `launch::fork` (`claude --resume <id> --fork-session`) to continue in a new session. The
desktop app keeps a live process for sessions it has open, even idle ones, so resuming those sessions
is refused. Opening a copy leaves the original alone and is safe without a Desktop hand-over.

**Desktop warning.** When a session is no longer running but the desktop app still lists it, opening
it in the CLI asks first: do not continue it in Desktop while it is open here. If changing accounts
also triggers the cold-cache warning, both warnings appear in one Continue/Cancel dialog, never two.

A cross-process lock covers the final running scan, a durable reservation in
`<appdata>/claude-switch/history.json`, and the start. Resume refuses an incomplete scan, invalid
history, or a failed reservation write. An unsuccessful start restores the previous account entry;
a successful start blocks a second resume for 20 seconds while the running record catches up.
A reservation left by an interrupted process blocks for 60 seconds.

Child CLI programs (including naming and debug shells) and Desktop use one environment rule:
remove `ANTHROPIC_*`, `CLAUDE_*`, and `CLAUDECODE`, case-insensitively. This includes API/OAuth
tokens, provider flags, base URLs, session/account IDs and the parent's `CLAUDE_CONFIG_DIR`.
The only Claude variable Switch sets back is `CLAUDE_CONFIG_DIR` for a secondary CLI account;
primary and direct Desktop launches do not receive it. `--shell` and `--type` are debug-build
switches; delayed input belongs only to the tab that switch opened.

Desktop resume runs out of sight on a worker. The window shows pending until the CLI returns,
then its diagnostic on failure or a timeout after 30 seconds. Success only reports that the session
was handed to the desktop app. A profile may exist outside the current account list, so Switch
does not claim to know which profile received it.

## What a launch is

One value, `Launch`, says program, arguments, folder and environment. The window decides where it runs:
an embedded terminal tab (`mdshared-pty` + `mdshared-term`), an external terminal window, or a detached
process (the desktop app). Reading the account status does not launch Claude.
Resume and fork require UUID transcript IDs. Embedded, external, Desktop handoff and naming
launches resolve an absolute native `.exe` before starting; batch-only CLI installations are refused.
External terminals use a new console and pass the directory separately. There is no `cmd /c start`
or Windows Terminal delegation. Account directory junctions use a Windows filesystem call, not a
shell. Explorer Reveal uses the same environment scrub as other direct launches. User-requested
document, image and URL opens use OS associations; Switch does not control their environments.

**New session.** The toolbar button asks for a folder and account first. It starts with the selected
session's folder (or home when none is selected) and the current account. Start or Enter checks that
the folder exists; Cancel or Escape closes the dialog without starting anything.

**Recent folders.** A scrolling list under the folder field (8 rows, fewer in a low window) whose rows
fill the field: the folder's last component, the path above it muted, and its last activity; the
tooltip is the full path.

- *Which:* the folders of sessions that are not archived here, not archived in the Desktop app
  (unless that setting shows them) and not hidden, by themselves or by their folder; **Show hidden**
  does not bring them back. Worker sessions count only while **Show sessions started by tools** is
  on, as in the list.
- *Not temporary:* nothing at or under `%TEMP%`, `%TMP%`, `%LOCALAPPDATA%\Temp`, `%SystemRoot%\Temp`
  or the system's temporary folder (Claude Code's scratchpads are under these). Compared by name,
  with Switch's own environment; a relative value or a drive root is ignored.
- *Not a worktree:* the first `.git` at or above the folder is not a file pointing into a
  `worktrees` directory. Main checkouts and submodules stay. Only that file is read, so a worktree
  whose main repository has gone is left out too. As for Projects, network and redirected
  (junction, `subst`) paths are not probed, so a worktree there is listed.
- Temporary folders and worktrees are only left out of the list: typing one or choosing it with
  Browse works as for any folder.
- *One row per folder:* spellings that differ in case, separators (`/`, doubled, trailing), `.`
  components or the `\\?\` prefix are one, shown as its newest session spells it. Nothing is
  resolved on disk (junctions and 8.3 names stay apart). Newest activity first.
- *Existing only:* a worker looks (for existence and worktrees) after every scan and again when the
  dialog opens; the dialog shows what the last look found and updates when the new one ends, so
  opening waits for no disk or network path. Start still checks the folder itself. The demo's folders
  count as existing, plain folders.
- *Filter:* typing in the folder field narrows the list to paths holding every word, newest first,
  or, when none does, to fuzzy matches, best first. The prefilled, picked or browsed folder filters
  nothing. At most 200 rows show; the filter searches all folders.
- *Keys:* ↑, ↓, Page Up and Page Down in the field move into the list, where Home and End work too
  (in the field they move the caret). Enter picks the highlighted folder; the next Enter starts. Text
  with no `\`, `/` or `:` highlights its first match as it is typed; a typed path highlights nothing,
  so Enter starts there. Tab keeps its three stops: folder, account, app.

## Workers

The owner's bridge (`claude-flash-bridge`) runs Claude Code workers with `claude -p`. A setting here says
which account they use; it is written to `%USERPROFILE%\.config\opencode-bridge\workers.json` as the
top-level keys `claudeAccounts` (the CLI folders) and `claudeWorkersUse` (`"same"`, `"other"`, or one of
the folders). The bridge reads them.

### The Workers tab

The bridge's server instances share a state folder, `%TEMP%\opencode-agent` (Node's `os.tmpdir()`:
`TEMP`, then `TMP`). While that folder exists a fixed **Workers** tab sits beside Sessions (the
strip then shows even without a terminal tab); it is checked at start and with the five-second
running-session poll, and the tab goes (Sessions shows) when the folder does. `crates/core`'s
`bridge` module reads it; the window never writes, renames or deletes a file there.

- **The jobs.** Each job is `<id>.json` (`job_<base36>_<n>[-label]`, the bridge's own name check),
  which the bridge replaces whole, and `<id>.jsonl`, the worker's raw events, which only grow. A
  state file is read whole up to 1 MiB and parsed again only when its size or modification time
  changed (`bridge::Scanner`); its fields are listed in the module. The list shows queued and
  running jobs first, then the 150 most recently ended, each newest first: the label (else the id),
  worker and model, folder, running time and last tool (the call in flight while one runs). A job
  whose file says queued or running but whose `owner` pid has exited, or now belongs to a process
  created after the job, is **Owner gone** (the bridge's "lost"); its time stops at its last save.
  The folder and the shown output are read off the window's thread once a second while the tab
  shows, on the same feed timer as the Sessions details' conversation.
- **The output** of the selected job is the tail of its `<id>.jsonl` (`bridge::log::Tail`): the
  first read starts at most 256 KiB before the end, later reads take what was appended, at most
  1 MiB at a time; a line over 4 MiB is skipped, an entry keeps at most 8 KiB of text and at most
  2 000 entries are kept. A file that shrank or was replaced is read again from its tail. OpenCode's
  `run --format json` events (`text`, `reasoning`, `tool_use` with `part.state`, `error`) and Claude
  Code's `stream-json` messages (`system` init and retries, `assistant` text, thinking and tool
  calls, failed `tool_result`s, `result`) are shown as answer text, quiet tool lines and warnings.
  The output follows its end until scrolled back; the details around it scroll when the window is
  too short for both.
- **Cancel** (after a confirm whose Enter is Cancel): reads the job's file again, and refuses
  plainly unless it says running and names a `childPid`. The bridge records no start time for its
  worker, so the pid is taken for the job's only while Windows knows it as a child of the job's
  `owner` (Windows keeps a process's parent pid after the parent exits) created between the job's
  start and its last save (2 s of slack each way); the snapshot is taken while a handle to it is
  held, so the pid cannot change hands between the check and the end. Only a process of Switch's
  own user is ended, and, when Switch runs elevated, only one that does not: the job file lies in
  the user's temporary folder, so an elevated Switch must not end for it what the user could not.
  The worker is ended first, so it starts nothing more, then what ran under it, round by round
  until a round finds nothing new (at most 64 rounds, 512 processes). Every process ended stays
  held open to the end, so the pids that name parents in later snapshots cannot change hands. A
  process under one is taken only when it was created no later than the snapshot that found it
  (a pid given out again after the snapshot is newer) and no earlier than its parent (a stale
  parent pid), and still names that parent in a snapshot taken after it was opened. Switch
  itself, and everything under it (a worker that ran `cargo run` on Switch), is never ended. The
  job file's texts are drawn as one line without control characters (the folder at most 1 024
  characters, the others 200). A queued job has no process: only the bridge
  (`opencode_cancel`) can take it out of its queue, and the tab says so. With its owner there, the
  bridge sees the worker end and marks the job; with the owner gone the file keeps saying running.
  The demo (`--demo --demo-jobs`) shows made-up jobs from a temporary folder of its own and ends
  nothing.

## Terminal tabs

With the setting "Open CLI sessions in a tab of this window" on (the default), a launch that needs a
terminal (`Place::Terminal`: resume or start a CLI session, sign in) opens a tab of the window instead of
an external terminal; detached launches (the desktop app) are unchanged, and the demo starts nothing. A
strip under the top bar shows the tabs; with no terminal tab it is hidden and the window is as before.

- **The first tab** is "Sessions": the list and the details, always there, never closed.
- **A terminal tab** holds the program in a pseudo-terminal (`mdshared-pty`, ConPTY), its screen with
  10 000 lines of scrollback (`mdshared-term`), drawn by the toolkit's `termview` in the mono face at
  13 logical px by default; the view's scroll offset and selection; its title (the launch's label, replaced by the title the
  program sets); the account's colour as a dot; for a session, the session id and account slug. The grid
  fills the client area under the strip; every layout change resizes the screen and the pseudo-terminal.
- **When the program ends** the tab stays, showing its launch label with ` (ended)` in the strip and a
  last line `[ended with code N - press Enter to close]`; Enter or the close button closes it.
- **Keys** go to the program while a terminal tab is shown, as an xterm sends them. The window keeps
  Ctrl+Tab and Ctrl+Shift+Tab (next and previous tab), Ctrl+Shift+W (close the tab), Ctrl+Shift+T (a
  plain shell in the selected session's folder), Ctrl+Shift+K (the command palette; Ctrl+K is the
  program's), Ctrl+Shift+C and Ctrl+V / Ctrl+Shift+V (copy, paste; Ctrl+C copies only while something is
  selected) and Shift+Page Up/Down (scroll back). Shift+Enter alone sends `CSI 13;2u` only when
  Switch launched Claude Code in the tab, identified by the executable's filename, not its title
  or transcript, and only while Claude Code is what the screen shows: on the main screen (its
  classic view), or on the alternate screen while the cursor is in its input box (a `❯` row between
  solid rules, which its full-screen view draws there too). The box's top is the nearest rule with a
  `❯` row under it, so rule lines in a draft do not end it; a draft taller than the screen is known
  by Claude Code's mode line under a rule below the cursor. In doubt inside such a box Shift+Enter
  stays Claude Code's, since a plain Return would submit the unfinished draft. Any other program on the alternate
  screen, such as an editor Claude Code opened, gets the classic Return. Shells and other programs
  keep the classic Return encoding; this is not full kitty keyboard negotiation. A middle click on a tab closes it.
- **Mouse**: the terminal uses the arrow pointer. Drag selects (double click a word, triple click a
  line, Alt a block); the drag requests a redraw only when the selected cells change. Releasing the
  mouse does not copy. Right-click copies and clears the selection, or pastes when there is none.
  A selection with no text clears without a clipboard read or write.
  A program tracking the mouse gets the clicks and drags instead, unless Shift is held at the
  press. Use Shift+drag and Shift+right-click to select and copy or paste in that case. As in
  Windows Terminal, Shift+right-click copies stream selections as one line; keyboard copy keeps
  the line breaks. A right-click paste in the Chat drawer goes to the terminal, not the composer,
  and uses the same safety checks as keyboard paste.
  The wheel scrolls the scrollback 3 lines a notch, is reported to a program that asked for the
  mouse, and is arrow keys on the alternate screen of one that did not. Shift keeps the wheel
  local. The window chooses the pointer from the current frame's hit map, including when a tab or
  overlay changes under a stationary pointer. Leaving the client resets it to an arrow; the host
  still supplies native resize cursors at window edges. Only the front layer receives hover while
  a menu, dialog or command palette is open.

**The safeguard, for tabs.** A session open in a tab counts as running at once, before any
`sessions/<pid>.json` says so: opening it again shows its running tab ("Already open in a tab"), and the list
shows it as running in this app. Closing a tab whose program runs asks first; closing the window with
running tabs asks once for all of them. Dropping a tab's pseudo-terminal ends its program.

**OpenCode tabs.** The palette's **New OpenCode tab…** opens the New session picker with the folder
only (no account or app) and starts the owner's normal OpenCode there: `opencode_cli::find_program`
(native `opencode.exe`), the plain shell's sanitized environment (Claude Code's config and session
variables removed, `PWD` removed because OpenCode prefers it to the process folder) and no OpenCode
overrides, so the owner's own config, plugins and Magic Context apply, unlike the handover and
search jobs. The installed CLI (v2.4.1, `opencode --help`) is `opencode [flags] [<directory>]` with
`--prompt <text>` and `--continue`; options go before the folder. The tab is labelled “OpenCode ·
<folder>” and shows a terminal glyph instead of an account colour. It has no session id, account,
launch folder in the Claude sense or pane, so it is never a list row, gets no Chat or Session view,
live-record matching, send gate, usage, handover or auto-handover, exactly as a shell tab; its folder is
kept in its own field. The handover preview's **In OpenCode** keeps the edited handover as Open does
and opens such a tab in the source session's folder, beside the source's tab, with the text as
`--prompt` (one native argument, no shell). Above 15,000 UTF-16 units, or when the text starts with
`-` (so OpenCode's parser cannot read it as an option), the prompt instead asks to read the saved
handover file; OpenCode may ask permission to read it, since it is outside the project. The Claude
session is left as it is and nothing is recorded in `marks.json`. Whether `--prompt` submits at once or
waits for Enter was not checked (that would need a model request). Saved tabs keep a running OpenCode
tab as `{"kind":"opencode","folder","active"}` beside the Claude rows in `terminal-tabs.json`
(older builds skip such rows). Restoring reopens the first such tab of a folder with `--continue`,
OpenCode's last session there (possibly a later one started elsewhere, or an older one when that tab
was never used), and further tabs of the same folder fresh, so two tabs never show one session; a
first prompt is never repeated. OpenCode's own session id is not kept: `--session <id>` with a new id
does not create the session at startup (checked with `--standalone` in a scratch folder), and
whether a later first message keeps that id could only be seen with a model request, so it is not
relied on.
A folder that is gone is listed in the "Tabs not reopened" notice.

**Find in a terminal.** Ctrl+Shift+F while the terminal has the keyboard (Windows Terminal's key;
Ctrl+F stays the program's), "Find in terminal" in the palette or "Find…" in the tab's menu opens a
bar at the top right of the terminal shown; in Chat it opens the terminal drawer and sits over it.
Each terminal keeps its own bar (`Term::find`, `termfind.rs`). The search reads every line the screen
keeps, from the oldest scrollback line to the bottom row (`Screen::first_line`..`end_line`,
`line_at`), joins rows a wrap continued into one line, and matches characters case-insensitively
unless Aa is on; matches do not overlap and trailing blanks never match. A new query, the case
setting or a resize (which rewraps) searches everything again. Output only searches from the logical
line that held the screen's old top row down: lines already in the scrollback are never rewritten,
so their matches are kept (less any lines dropped or cleared). That happens at the wake that brought
the output, not while painting (`Term::changes` counts output and resizes). A new query picks the last match that starts above the bottom of the view (a
search upwards, as Windows Terminal's default) and scrolls it into the middle when it is not
shown; new output keeps the current match without scrolling. Enter, ↑ and F3 go up (older),
Shift+Enter, ↓ and Shift+F3 down, both round from the ends; the count reads "N of M" with matches
numbered top to bottom. The marks are blended over the drawn grid in the palette's `find_mark` and
`find_current` at the Markdown viewer's alphas, so termview's own colours, selection and cursor are
untouched. While the bar has the keyboard, keys go to its box (the window's shortcuts still work)
and the cursor is drawn unfocused; a press on the grid gives the keyboard back to the program and
leaves the bar open; Escape or its × closes it. A pane that loses the keyboard also takes it from
its bar (still open), so coming back to the pane types into the program.

**Panes.** A tab can be split into up to four panes (`panes.rs`), as a small tree: each split is
side by side ("Split right", a vertical divider) or stacked ("Split down"), half and half at first,
and divides the focused pane. The commands are in the tab's menu and the palette (with "Close pane"
and "Next pane"); no key was added, since Windows Terminal's Alt+Shift+plus/minus and Alt+arrows
would be taken from the programs. Pane 0 is the tab's own terminal (`Tab::term`): Chat, the session
view, attention, the send gate, identity, saved tabs and the safeguard keep reading only it. A new
pane is a plain shell (`launch::shell`, the same sanitized environment as a shell tab) started in
the tab's launch folder (`Tab::folder`), else its session's, else its OpenCode folder, else the
home folder; never a second Claude Code on the session. It starts at about the half it will get
and every pane is then fitted to its own rectangle on each layout, so a split or a divider drag
resizes only the programs it divides. Keys, paste, dropped files, the mouse and find go to the
focused pane; a press on another pane focuses it first (the programs that asked are told of the
focus change), the wheel over an unfocused pane scrolls its scrollback without focusing it. The
focused pane has a 1 px edge between the theme's line and accent colours; unfocused panes draw the
hollow cursor. The 6 px gap between panes holds a 1 px line, accent while hovered or dragged; it
is dragged with a resize cursor and keeps 60 px for either side. An ended pane shows its end line
and closes on Enter; Enter on the ended tab's own pane is "Close" of the tab, which asks while
another pane's program runs. Only keys for pane 0 count as input to Claude's terminal for Chat (its
send gate and card keys); a shell pane's do not. Switching to Chat's layout while a shell pane has
the keyboard moves it to pane 0, and back again, telling the programs that asked. "Close pane" asks first while the pane's program runs; the tab's own pane
closes only last (closing it under shells would leave Chat without its terminal), and with no
other pane it is "Close" of the tab, asking as before. Closing the tab or the window counts the
panes' programs as running. In Chat's layout only pane 0 is shown (in the drawer) and has the
keyboard; the other panes keep running, their size unchanged, and splitting waits for the
terminal layout. Panes are not saved with the tabs.

**Not there:** links in the terminal,
font-family settings; Shift+Insert and the Insert key (the window host
has no name for Insert); input-method composition (the host turns input methods off on Windows).

## Session view

Terminal tabs for Claude sessions (including forks) can show a read-only transcript beside the terminal.
The tab-strip button, Ctrl+Shift+E and the command palette toggle it. The splitter holds the width of the
right pane. Both the explicit visibility choice and the splitter width are stored in `window.ini`;
without an explicit choice the pane opens from 1100 logical pixels up. Shell and sign-in tabs never
have a pane.

Resumed tabs use the transcript file already found by the session scan. New and forked tabs attach
only after the terminal PID, account and working directory identify one session ID. The pane says
when identity is unverified or ambiguous rather than selecting the newest file in a shared project.
For new and forked tabs the verified ID must name a fresh transcript in the launch folder's encoded
directory. For wrapped processes, the live CLI PID must belong to the terminal's process tree; otherwise the pane
stays unattached. Claude Code writes a new session's transcript only with its first prompt: while the
live record names this tab's process and a valid ID whose file does not exist yet, the pane pins
that ID and Chat may send one message (an ordinary Send needs the usual quiet, visibly empty prompt;
with no transcript no turn can be running). Later sends wait until that exact file appears; a record
naming another session first, or a file whose recorded folder differs, keeps the refusal. Such a tab is
not yet routed, marked seen or restored.

Session view has six pages over the incremental transcript reader: Conversation, Tasks, Agents,
Shells, Files and Outputs. Chat shows the same five beside the conversation in its Details panel. Conversation
opens on the newest messages and follows appends until the user scrolls back; Jump to latest resumes
following. In Chat that is a round button at the foot of the conversation while it is scrolled back,
in a strip of its own: no text is drawn (or taken by the pointer) under it, and the rows below the
strip come into view as the conversation reaches its end, where it follows again and the strip goes. Both Chat and Session view support wheel scrolling, thumb drag, and track clicks that page
by a viewport. Earlier history is loaded on request. Agent transcripts are reachable from expanded
tool details and the Agents page. A cursor checkpoint detects a replacement or truncate-and-regrow near the
read position. Transcript lines over 48 MiB are skipped, picture reads respect the same containing-line
limit, and sidecar metadata over 1 MiB is ignored. The reported agent state comes from the sidecar, so a missing
completion flag is shown as unknown rather than guessed.

Chat and Session view use the same conversation projection, painting and input. Prompts sit in
right-aligned tinted bubbles; cardless answers use a palette-accent Claude mark and body-size prose
(16 logical px in Chat, 14 in Session view). Tool rows recede behind the answers: indented, low
one-line notes without a box, in the palette's muted text (the name included, no accent) with the
status and duration fainter, a chevron in front and the duration right after the target. They sit
close together and close under the answer that announced them. A failed call keeps its status in
the warning colour on a soft wash of the warning ground; a running call keeps its pulsing accent
status. Hovering fills the line with the hover colour; an opened row or group sits on the field or
pane colour. Each row shows the recorded status, the tool and its target, and the time between use
and result timestamps when both are valid and ordered. Runs of four or more consecutive tools start
collapsed into one quiet "N tool calls" line. Expand a row for the bounded details and any output
pictures; a group can be expanded independently of its rows.

**Selecting and copying.** A selection runs from an anchor to a focus, each a row (by its stable
identity, so it survives an appended message) and a byte in that row's shown text, so a drag or
Shift+click can cross messages; dragging past the top or bottom scrolls. A double click takes a
word, a triple click the message; a drag after either goes on by whole words or whole rows, from
the first one in either direction (the message box's drag after a double click goes by words too). Ctrl+C copies the selection, the rows' parts joined by a blank
line (in Chat, where keys go to the message box, when the box has no selection of its own); Ctrl+A selects the whole loaded conversation. The right-click menu has Copy and Select all,
and over a message Copy message (its text as shown) and Copy as Markdown (its source). Each
top-level code block of an answer has a **Copy** button for its code alone.
In Chat a row of buttons sits under each of your messages and under each turn's last answer,
shown while the pointer is over the message (or the buttons) and always under the newest answer;
its room is kept whether it shows or not, so pointing moves nothing. **Copy** copies the message
as shown (as Copy message does). **Rewind** is "Rewind to here…" of the menu, through the same
start and checks, under a prompt the CLI's rewind list shows. **Branch** starts a new session,
as the tab's account, that continues from a copy of the conversation cut at the end of that
message's turn, so that the copy ends where Claude Code can go on (`view::model::branch_cut`): a
turn ends right before the next prompt typed at an idle input (a message sent while it ran goes
on in it), the newest one at the record of its end (`end_turn`, no call pending). It is offered
under your prompts and under a turn's last answer once that turn has ended; never mid-turn, right
before a message sent during the turn, or in a subagent's transcript (whose items are not the
session's). Claude Code 2.1.332 cannot resume a session up to a message interactively (its hidden
`--resume-session-at` is for print mode), so `core::branch` writes the copy, off the window's
thread: the transcript Chat reads (its own file, which must be the one the Sessions list names
for the session, compared as resolved paths: a further account reads it through its `projects`
junction) up to the cut, under a temporary name and then linked to `<new id>.jsonl` beside the
original, or on a volume without hard links created new there and filled; neither ever replaces
a file, a temporary name someone else holds is left alone, and temporary copies older than ten
minutes that a failed branch left are swept; each record's own `sessionId` is set to the new id, in
place where it has the usual `"sessionId":"<id>"` form, else by reading the record. A cut that is
not at a line's start is refused. Then `claude --resume <new id>` (`launch::branch`). The original
is not touched; the branch is marked as a copy of it, as Open a copy's is. File backups
(`file-history/<id>`) are not copied, so the branch cannot rewind code changes made before its
cut. Checked by `recorder compat` (script `branch`): cut copies resumed in print mode and in the
terminal show the turns before the cut only. The toolkit's
`richtext` manages no selection, so this is the app's (`view/ui/select.rs`) over the renderer's
selection rectangles and block boxes.
Notices are rows like a tool row (their kind and the first line) that expand to the whole text,
at most 8 KiB. Claude Code's docs call the transcript format internal; the shapes below were
confirmed read-only against real transcripts (tests use synthetic records). Anything else stays
hidden rather than shown wrongly:

| Shown as | Record |
| --- | --- |
| Thought | an assistant `thinking` block `{thinking, signature}` with text; about half have none (omitted thinking) and are not shown; thoughts sit inside a tool group without counting as calls. With `"thinkingDisplay":"updates"` on the record, the terminal shows the block's text as an answer line ("· …"), so Chat shows it as answer text |
| Prompt | a long paste is recorded inside `<pasted_content id="N">` tags, which are taken out (2.1.332); also an `attachment` `queued_command` with `commandMode: "prompt"` and `origin.kind: "human"`: a message typed while a turn ran, which Claude Code records only there (its `prompt` a string, or text and image blocks). It starts no turn and splits no turn's changed files; Claude Code's rewind list shows it where the transcript has it, so Rewind offers it as any prompt (recorded from 2.1.332 by `recorder compat`, script `queued`); the same record from a task notification or another session is not shown |
| Hook error | `system` `stop_hook_summary` with `hookErrors` (strings); an `attachment` `hook_non_blocking_error` (`hookName`, `stderr`) except for Stop hooks, whose summary already lists it |
| Hook | an `attachment` `hook_system_message` (`hookName`, `content`), the hook's `systemMessage` |
| Stop hook feedback | Claude Code's own user record starting "Stop hook feedback:"; a summary with `preventedContinuation` |
| Memory | an `attachment` `nested_memory`: "Loaded" and its `displayPath` |
| Recap | `system` `away_summary` with `content` |
| Notice | `system` `informational` at `level: "warning"` (such as the Stop-hook block cap) |
| Interrupted | Claude Code's own user text for an Esc, `[Request interrupted by user]` or `… for tool use]` (2.1.332): not the user's prompt, so no bubble, no new turn (it does not move the newest prompt), nothing for Rewind, recall or a Branch cut |
| Prompt blocked by a hook | not seen yet; a text with "Original prompt:", as the docs quote |

Not shown: informational records below warning (usage-limit and goal notices), `turn_duration`,
`compact_boundary` (the compact summary keeps its card), `local_command` (item 47), `api_error`
retries, `permission_retry`, `bridge_status`, and the other attachments: context sent to Claude
(`hook_additional_context`, `instructions`, `prompt_snapshot`, `deferred_tools_*`, reminders),
`hook_success`, and `file` / `compact_file_reference` (files read into the context, most of them
again after a compaction; an @-mentioned file is already in the prompt's text). "Saved N memories"
lines are not records of their own and are not shown. Attachment records are parsed only for the
three kinds shown. Compaction and API errors keep their cards.

The embedded Markdown viewer renders assistant answers and named Markdown files. Conversation row
heights are cached and only visible slices are rasterized. Bare or inline-code Markdown paths and
explicit Markdown link destinations are actionable when the reader worker finds a regular file.
Relative and absolute file paths stay inside the session’s working folder, except for exact paths
named by a successful Write/Edit in the currently loaded transcript. A bare file name (no folder)
that is also the name of such a written file outside the folder means the newest of those writes;
the session folder's file of that name, when there is one, is offered beside it in the viewer
("Also in <folder>"). This grants nothing new: only already written paths are candidates. Parent hops (`..`), device
and network paths, redirects (including junctions), and nonexistent files are not actionable. Bash command text
and output do not infer file links. Successful Write/Edit paths to existing Markdown files get one
button per file at the end of the loaded turn. Click a path or button to preview up to 1 MiB of
UTF-8 Markdown on the right in wide Chat or in place of the reading body when narrow. A divider
between the conversation and the docked viewer drags its width (at least 300 px, the conversation
at least 320 px); the viewer's toolbar cycles a reading width (Narrow 560, Medium 720, Wide 900
px or Full, a centred column, after mdview's page widths). Both are one setting for the window,
shared by every tab and kept in `window.ini` (`markdown_split`, `markdown_reading`). The viewer
shows the file name and, under it, its full path. The preview
does not follow links in the document; images and optional math/diagram engine jobs are disabled
in both renderer surfaces. **Open in editor** rechecks the named file and asks the OS to open it
with its default association. Ctrl+click on an inline file link instead reveals it. HTTP(S) links
in answers still open in the browser; transcript attachments still use the picture pipeline.
Chat's conversation has its own Narrow, Medium, Wide or Full reading width in Settings → Sessions,
stored per window in `window.ini` (`chat_reading`); it uses the same column sizes as the viewer.
The lines above and below the conversation (a read error, Load earlier, the working or waiting line
with Stop and Jump to latest, the terminal-answer hint) share the column's left edge and width.
The composer stays centred and capped independently. Conversation and Markdown scrollbars appear
on scrolling or hover, hold briefly, then fade; with reduced motion they disappear without a fade.
Their tracks remain interactive even when hidden.
Named UTF-8 code files in the viewer use extension-based syntax highlighting, with token colours
adjusted for contrast against the current page palette. Line numbers mark source lines; wrapped
continuations prefer whitespace or operator boundaries and hang under the original indentation.
An overlong token must split at a UTF-8 character boundary. Highlighting is cached, and unknown
languages and files over 256 KiB remain plain; file previews still stop at the existing 1 MiB limit.
Rendered Markdown retains the renderer's existing fenced-code syntax highlighting.

Only the visible pane requests polling: on terminal output, with a 500 ms fallback. Reader and
picture work run off the window thread; worker results wake the window rather than making it redraw
every poll. Image bytes are decoded with `mdshared-text`'s existing PNG/JPEG decoder and cached for
inline previews and an enlarged overlay. Pictures (pasted ones, those in an expanded tool row, and
tool-result pictures under the answer, where they were a "Picture" chip before) are thumbnails in a
fixed 240 × 180 slot so a late decode never moves the conversation; the picture keeps its aspect,
is never enlarged past its own pixels, and is captioned (the file it read, or the tool and its
number). A picture that cannot be shown keeps the slot and says why: too large, no longer in the
transcript, not a supported format, or not decodable. A click opens the enlarged overlay.
Opening a picture in the system viewer explicitly exports
a copy to the application's temp folder. No transcript is edited. The demo supplies a synthetic
session, writes only to temp, and appends a few updates without starting Claude Code.

## Handover

**Compact and open** reads a full sequential snapshot through the existing JSONL decoder with no
subagent files. A local reducer collects prompts and timestamps, the latest known task list,
Write/Edit paths, the last 15 shell calls, and six recent turns. A prompt is capped at 4,000 Unicode
characters with a marker. User image blocks become `[picture]`. Only assistant text and fixed tool
category names cross the OpenCode request boundary; the optional focus is data for emphasis only.
Tool arguments, paths and commands are not copied out of tool records. The assistant's own prose
and the focus may still contain sensitive text; review the provider choice accordingly.
Raw prompts, transcription metadata, and the full task list are assembled locally. System/meta
records, embedded reminders, sidechains, tool results, image bytes, and thinking are not sent.

Every serialized request, including a merge, is capped at 60,000 characters. The worker emits
parts one at a time, prefers turn boundaries and splits oversized turns. Adjacent summaries
merge in bounded stages. Model replies supply only seven Markdown
fields (goal, where it stands, in flight and promised, decisions, standing instructions, open
items, not verified); the application renders the headings and all local evidence independently.
Invalid or missing summary fields say `unknown`. The fixed prompt asks for at most 800 characters
a field; a longer one is cut at the last line break before 1,500 characters and ends with a
visible `[cut]`. It tells the summarizer that it sees assistant text and tool categories only,
that the user's prompts are appended verbatim afterwards, and never to infer that a permission
was given or a rule lifted; it defines "instructions" as the user's rules still in force (no
tasks or next steps), and "in flight" as jobs, workers and reviews still running or unread with
id, worktree, branch and state and what was promised for when one reports. When partial
summaries disagree the later wins, and items a later part shows done are dropped. The Files
section leaves out paths in temporary and scratch folders and keeps the 30 most recently edited
(saying how many older ones were left out); the task list is marked possibly stale.

When the user gives no focus of their own, one is picked locally, without a model, from the
session's tool calls: **coordination** when at least three calls hand work to agents or workers
(Claude Code's Agent or Task, SendMessage, Workflow, MCP tools named for workers or agents), they
are most of the calls that act (edits, shell and other tools; reads and searches do not count),
and own edits are at most a third of them; **non-code work** when the session acted but edited no
code file outside temporary folders and ran no build tool (cargo, npm, dotnet and the like); else
the **default**. The dialog's focus box is filled with that kind's text and its label says which
kind was picked; editing it, a `/compact` focus in the transcript, or `--focus-file` with text
makes it the user's own, which replaces the picked one. An empty focus file (the plugin with no
focus) counts as none. A transcript over 2 GiB (it is read as a stream; only prompts and answers are kept), or a record over 48 MiB,
fails the handover rather than silently dropping a possible user turn.

Native OpenCode V2 is discovered on `PATH`. `opencode models` is cached in memory, with manual
refresh. Each request uses an empty temp working folder, disables project and Claude Code config
discovery, and defines a text-only agent in a process-local OpenCode configuration override. The app uses
`opencode run --standalone --archived --agent cs-handover-text-only --model <provider/model>
--title "Claude Switch handover" --format json --log-level none`, with UTF-8 input on stdin. The agent
has a one-step budget and denies tools; additional policies deny tools, MCP, skills, plugins and web
search. The CLI has no no-persistence flag; the run is archived, not deleted. OpenCode's installed
configuration and provider retention policies still apply. The worker kills and reaps its active
process on Cancel or after 120 seconds per request. Provider consent and the selected model are in
`config.json`. A CLI invocation without prior provider consent refuses before reading the transcript.
Both routes recheck persisted consent before sending each part or merge.

The preview is editable, with characters and approximate tokens (characters / 4). Its text wraps at word
boundaries (a word is cut only when it alone is longer than a row); End, Up/Down and a click past a
row's end stay on that row. The review card is up to 760 px wide. Its footer has Back, Cancel, In
OpenCode and Open in CLI (or Continue here) on the right and Copy and Save as… in a group on the left,
which folds into **More ▾** (a menu with both) when the card is too narrow for the two groups. Any
dialog's footer narrows its buttons toward their labels' width rather than run past the card. Previewing or
canceling does not save a handover. Only an explicit Save As or Open writes the edited text.
Local Markdown files live in `handover/<session-id>-<time>.md` beside config; the newest 20
are kept per session. An opened edited handover is kept as a new revision. Save As never
overwrites an existing file. Native Claude Code receives `claude --session-id <new-uuid> --name=<title> -- <text>`
in the original folder with the selected account's normal sanitized launch environment. The
interactive CLI documents no stdin/file argument for a first prompt, so short text is one native
argument (no shell); above 15,000 UTF-16 units a short prompt asks it to read the saved file, and
the session may read exactly that file (`--allowedTools=Read(//c/…/<file>.md)`, the form Claude
Code writes such paths in): the handover folder lies outside the project folder, where Claude Code
2.1.332 asks before a read in manual mode. A path the rule could not name exactly (not a drive
path, or with `*`, brackets or quotes in it) gets no rule; the question then shows as a card in
Chat, which answers it as any permission prompt. The app uses the
new UUID for `marks.json` provenance. It never resumes or modifies the source transcript.

The new session name is independent of the OpenCode run title (“Claude Switch handover”) and
of the Markdown, whose first heading stays `## Goal`. The suggested name is the source's
Switch display name (local custom name, then indexed title), kept as it is: the new session keeps
the session's name, and the old one is marked as handed over, so the list still tells the two
apart. Endings ` · continued` and ` · continued N` that earlier versions added are removed
(`compact_open::names::suggest`), so they no longer pile up; no number is added and no other name
is consulted. The preview can edit the name; an untouched suggestion is recomputed from the
current display name before opening. Claude Code's documented `--name` carries the title as one
native option before `--`; Switch uses it as the initial tab label and reads native custom-title
records on scan. `marks.json` records the launch name under the new UUID with the provenance (a
failed spawn rolls both back); suggestions do not read it, as two sessions may share a name.

**Settings → Handovers** holds, in order: **Where it continues** (This tab, the default, or A new
tab; `compact.continueIn`), **Summary written by**, **Default focus text**, **New chat starts as**
(new tabs only: a session continued in place keeps its process and agent), then **Hand over
automatically** and **When the context reaches**. **Default focus text**, **Focus for
coordination** and **Focus for non-code work** are the three picked focus texts (see above); each
is built in until edited, saved as typed (`compact.focus`, at most 4,000 characters, control
characters dropped), and its **Reset**, an emptied box, or the built-in text itself go back to the
built-in text.

The editor's **Agent** picker is CLI-only. **Settings → Handovers → New chat starts as** is a
choice, not a text box: **None (plain Claude)** first, then the agents every account has in its own
`agents/` and its user-scope plugins (read when the page opens, local files only), each labelled
with the accounts that have it when the accounts differ. A saved name that no account has is shown
as “(missing)” until another is chosen. Saving goes through the handover settings lock and leaves
provider consent alone. **Summary written by** names the OpenCode model (`compact.model`), with
the models this window last listed. The two are separate settings: one is a Claude Code agent,
the other the OpenCode model that writes the summary. **None** adds no flag. A configured name is
preselected but passed only if the target account and launch folder's catalog has it: the dialog
and `compact --open` read that catalog before launching (the dialog even when its picker was
never opened) and otherwise start plain Claude with a notice, instead of a launch that Claude
Code refuses with `--agent '<name>' not found`. An explicit `--agent` or an agent picked in the
dialog is passed as given. Built-in and managed agents are not in the catalog, so they cannot be
the Settings default (they can still be named with `--agent`). Changing the target account
resets to the configured default. A selected custom agent adds
`--agent <name>` before the first-prompt separator. Discovery runs read-only from that account's
`agents/` and the launch project's `.claude/agents/`; project definitions take precedence.
The reader keeps bounded frontmatter names and descriptions, not agent prompts. It also offers
agents in standard `agents/` directories of unambiguous, explicitly enabled version-2 plugin
cache installs. It does not guess at managed policy, custom manifest paths or registry formats.
Discovery does not start Claude. The documented Desktop launch path takes neither an agent
flag nor a first prompt, so the handover editor continues to open new sessions in the CLI.

The CLI runs headlessly for `--print` (the default) and writes the handover only to stdout unless
`--save` is supplied. Explicit `--open` keeps the handover and hands the request to the running
Claude Switch window, which opens the tab beside the source session's tab when one of its tabs
runs it (otherwise at the end) and comes to the front. The channel is one per-user named pipe,
`\\.\pipe\claude-switch-handover-<user SID>`: it is owned by that SID and its DACL grants only
that SID, remote clients are rejected, and only the first instance can be created. A window
claims the name at startup before it restores any tabs, so of two windows starting together only
one restores the saved tabs; a window started for a handover that finds the name taken forwards
the request and exits, and requests sent before the holder serves wait in the pipe. The client
checks that the pipe object's owner is its own SID before writing (another user cannot create
the name with that owner) and allows identification, not impersonation. The server accepts only
a client running as the same user at an integrity level no lower than its own, so a normal
process cannot steer an elevated window; an elevated window also labels the pipe High with
no-write-up. A client has 5 seconds to write its request, repeated connect errors back off, and a
window that cannot create the channel says so. Known limit: another local user can create the
name first (a pipe name is first come). The owner check then stops every client from writing to
it, so no request leaks, but each window starts as a second window: it restores no tabs and
handovers sent with `--open` fail. That is a local denial of service only, needs a second local
account, and is not worked around. It carries two
request kinds. A handover is a versioned list of the same validated words as the window arguments (session
ID, account, saved handover file name, optional agent, optional hex-encoded title), at most 2 KiB,
decoded and revalidated by the window, which then continues in place when one of its tabs runs
the session (see "Continuing in place"; with `--as`, only a tab of that account) and otherwise
opens only an app-owned saved handover in a new tab. The answer is `continuing`, `opened`, `shown:` and
the window's error when the window has put it on screen (its own refusal, or an error notice for a
request it could not decode or load), or `failed:` and a reason nobody has seen yet: the window was
closing, or its own 30-second wait ran out. That wait is settled with a ticket the window must take
before acting on the request: when the window has not taken it, the pipe thread cancels it and
answers that the handover was not opened (the window then drops it); when the window has taken it,
the thread waits 3 seconds more and otherwise answers that the outcome is unknown and the tab may
still open. An answer the client cannot read counts as not shown. A notice is a version line, `claude-switch-handover-failed-1` (shown as an error notice) or
`claude-switch-handover-note-1` (an info notice), then one line of at most 1,500 bytes with no
control characters; it brings the window to the front, acts on nothing, and the answer is `shown`. A client that gets no answer (refused, busy,
closed or timed out) knows the window may have shown nothing. When no window listens, the CLI starts Claude Switch
itself as the normal window (saved tabs, then the handover tab), which first offers the request
to a window that started meanwhile. On Windows, that window is
ShellExecuted through a fixed non-interactive PowerShell bootstrap run without a console window, so it
cannot keep the caller's output pipes open; only the executable path and validated identifiers
reach the bootstrap.
`compact --open --agent <name>` overrides the Settings default. The agent option is rejected
with `--print`. The resolved agent is passed to the new Switch window as a separate
word and revalidated there, then passed to Claude Code before `--`. The CLI, picker, and settings
use the same rule: 1–128 ASCII characters, colon-separated segments starting with a letter or
digit and continuing with letters, digits, hyphens or underscores. If no name is resolved, the
launch is unchanged. The plugin's first-word `agent:<name>` uses the same rule; it passes focus
via a newly created UTF-8 temp file, never by interpolating arguments into shell code.
`compact --open --name <title>` overrides the continuation title, not the agent or first prompt.
The optional title is encoded as bounded UTF-8 hex for the fixed bootstrap and revalidated by
the new window; absent a title, it uses the same index-based suggestion as the dialog. A title
must be nonempty, single-line text of at most 200 UTF-16 units, with no control characters.
The plugin doesn't supply this option and its focus text is never interpreted as a title.
`--open` waits up to 30 seconds for the window to acknowledge creating the tab. On timeout
the outcome is unknown: check for the tab before retrying. Tab creation is not
confirmation that the CLI accepted the prompt or remained running. Exit codes are 0 for success,
1 for runtime errors and 2 for usage errors. `--demo` uses synthetic text and model IDs; it sends no
handover request and launches no Claude process.

`compact --open --background` (what `/switch-compact` runs) returns within seconds so the chat
that asked is not blocked, and nothing about the outcome goes back into that chat as a tool
result its agent could act on. It runs the quick checks first (provider consent, OpenCode, the
session, the account; an error there is the command's one line), reads the focus file at once
(the plugin removes it when the command returns), refuses a job over 256 KiB (focus text full of control
characters can pass 64,000 bytes when escaped) with a clear line, and writes a job, the same options as words
with the session fixed to the ID found, the focus text and the session's name, to a new
app-named file `cs-handover-job-<pid>-<nanos>.json` in the user's temporary folder. It then starts
`claude-switch compact --background-job <that name>` through the same fixed PowerShell
ShellExecute bootstrap as the handover window: a GUI process with no console window and none of
the caller's handles, so the caller's shell call ends and the process is not its child. When
the bootstrap fails, the job is removed and the command fails; when it runs out of its 30 seconds,
Windows may already have started the process, so the job stays and the line says the handover may
have started and to check Switch before retrying. That
process reads and removes the job, parses its words with the same parser and limits as typed
options (a job cannot name a file, a working folder or another job), and runs `compact --open`.
Success shows as the tab or the continuation in place. A failure the window answered with `shown:` is already
on screen; any other failure (a `failed:` answer included) (the transcript, the summary, no answer from the window, the new
window not confirming) goes to the running window as a failure notice, and when no window takes
it, to a message box. When the Settings default agent was dropped for a new tab, the same
sentence the foreground prints (the agent was not found for that account, so the new chat starts
as plain Claude) goes to the window as an info notice, or to an information message box. A job
file stays behind only if that process dies before reading it; each Claude Switch window start
(not `--demo`) removes, off the startup path, plain files in the temporary folder whose names match
the job pattern exactly and that are more than a day old, and nothing else.

Setup's optional **Add Claude Switch to PATH** task is checked by default. It appends the install
folder to `HKCU\Environment\Path` without changing the existing value type or claiming an
equivalent pre-existing entry. It records its own additions separately. Uninstall removes only
an unambiguous exact match for a recorded addition and preserves other entries and edits. A
previously missing value is removed again if Setup's entry is still the only one. Both paths
broadcast `WM_SETTINGCHANGE` after a change; already-open shells keep their inherited environment.
Setup checks for intervening edits before writing; Windows does not provide an atomic
compare-and-swap for this value. An interruption after append but before recording ownership can
leave an untracked entry rather than let uninstall claim an entry it cannot prove it added.
The plugin retains its fallback to the default per-user installation.

### Writing in the background

**Compact and open…** (session menu, palette, details pane, tab menu, the usage pop-up's offer and
Automatic handover's **Ask first**, which all open the same dialog) keeps its setup step: model,
account, focus and provider consent, all checked as before. **Make handover** (or **Allow and make
handover**) starts the worker and closes the dialog at once; the dialog itself, with every choice
made in it, waits while the model writes, so the app can be used meanwhile. Several handovers can
be written at once, one a session: asking again for a session whose handover is being written says
so, and one that is written or failed opens instead.

When it is written, the same **Review handover** window opens by itself with the result, as soon as
nothing modal is open (a dialog, Settings, a confirmation, a menu, the palette, the accounts list or
the usage pop-up) and the keyboard has not been used anywhere in the app for 1.5 seconds, so
someone typing a sentence goes on typing where they are; several open one after another, in the
order they finished. It opens over the Chat composer or a terminal with the dialog focused, not its
editor or name field: typing edits nothing until a click in one of them, or Tab (to the name
field). For its first 300 ms it also ignores keys and typed text, and clicks on Open in CLI, In
OpenCode and the delete box, so a key already on its way (an Enter half-typed into the composer)
cannot act in it. The delete box starts off and where it continues (in place or a
new tab) is checked again, as each time the review opens. When the window is not in front, its
taskbar button flashes (and, when it is hidden in the notification area, a notice says the handover
is ready); it never takes the focus from another program.

The Sessions column shows a strip between its filter chips and the list, taking no room when empty:
**Writing handover · 0:42 · <session>** with a spinner and **Cancel** (which stops the worker), a
failure in the danger colour with its reason on a second line and **Retry** (which starts again with
the same choices, through the consent check; when consent is needed again the dialog opens), and
**Handover ready · <session>** with **Review** and ✕ for a review closed with Cancel or Esc without
acting on it (✕ discards it). More than two fold into one line ("3 handovers · 1 being written ·
1 ready · 1 failed") with **Show**/**Hide**. The session's details pane has the same entry beside
**Handovers…**. A failure while a terminal tab is shown also puts up the error bar with **Retry**.

Closing the window while a handover is being written or a written one has not been opened asks
first (with the terminal tabs' line when tabs run) and says what happens: one being written is
canceled, and a written one that was not opened is lost, since previewing does not save a handover
(only Open, In OpenCode and Save as… write the file). An open review is moved to the strip before
the confirmation, so it is still there when you stay. Nothing about them survives a restart.

### Automatic handover instead of compaction

**Settings → Handovers → Hand over automatically** is Off (the default), Ask first or Automatic.
It applies only to the CLI tabs Claude Switch starts from then on; nothing outside the app
changes (no plugin hook, no global or account setting). While it is on, those tabs start Claude
Code with `DISABLE_AUTO_COMPACT=1` in that process's environment only, which per Claude Code's
docs turns off automatic (and idle) compaction and keeps manual `/compact`. Tabs started while it
was off keep Claude Code's compaction and are not watched. A tab started with compaction off
stays watched for as long as it runs, whatever the setting is later: switching to Off then makes
it ask first, since nothing else keeps its context in bounds. Sign-in tabs never get it.

A worker reads each watched tab's verified transcript tail (512 KiB, at most every 5 seconds and
only when the file changed) for the last request's context size (input plus cache tokens, as the
Chat view's context chip), the model and whether the turn has ended with no tool pending. The
threshold is **When the context reaches**: either a set percentage (50–90 %; a hand-edited value
from 40 to 95 is kept) or, by default, where Claude Code would have compacted, as far as Switch
can read it: the window is `CLAUDE_CODE_AUTO_COMPACT_WINDOW` or `autoCompactWindow` from the
account's `settings.json` (its `env` first, then Switch's own environment), otherwise the
model's context window (1M for `[1m]` models and, per the model docs, Fable, Sonnet 5+, Haiku 5.5+
and Opus 4.7+, unless `CLAUDE_CODE_DISABLE_1M_CONTEXT=1`; 200K otherwise, and 1M once the context
is past 200K), and the point is 95 % of it, lowered by `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` (which,
as in Claude Code, cannot raise it). Claude Code's docs give no exact default percentage (a
native 1M window compacts at about 967K); 95 % is a little earlier, so the next turn cannot
overflow while compaction is off. A per-session `/autocompact` or `--autocompact` is not visible
to Switch.

Once past the threshold and after the turn has ended, **Ask first** opens the handover dialog for
that tab (same account preselected) with a notice, never over another dialog or menu (it asks
again at the next poll). **Automatic** makes the handover with the configured summarizer
(**Summary written by**) and the focus from the transcript or, without one, the picked focus, keeps it like any opened handover, and
opens the new chat in the same account beside the old tab, with the Settings default agent if
that account and folder have it. It needs that provider's consent already recorded; otherwise it
asks first and says so. Either way it happens once per session in a tab. If an automatic
handover fails (OpenCode missing, the summary failing, the handover not opening or not
continuing in place), the notice says why and that tab is watched again, asking with the dialog
from then on instead of trying by itself. By default the handover
continues in place in that tab (below); only with a new tab chosen does the new chat open beside
the old tab. The new chat is linked, not forked: it carries the handover provenance
(`handoverFrom`, `parentSessionId`), so it shows as a continuation in the list and Threads, and
starts from the summary alone (a `--fork-session` would carry the full context). The old session
is marked handed over (`handedOverTo` in marks): its tab title says “· handed over” and its row
in Sessions starts with “Handed over”. Once a session has continued in place, a failed save of
these links (another window's lock or a damaged file) keeps them in memory: the save is tried
again up to three times, 5 seconds apart, and the next save carries them after that. A new tab
that fails to start removes only its own links; an earlier handover's mark on the old session
stays.

### Continuing in place

A handover continues in the tab and CLI process that run the session, by default: from the
dialog (whose **Open in CLI** then reads **Continue here**), automatically, and from
`compact --open` or `/switch-compact` through the pipe. It needs a tab of the running window
whose identity is verified as that session, with a live process and the Chat view's pane (and,
for the dialog and automatic handovers, as the chosen account). Otherwise, when **Where it
continues** is A new tab, or when Switch is not running, it opens a new tab as before.

Switch shows that tab in the Chat view (that tab only; the saved Chat default for new tabs is
left as it was) and goes through the Chat view's gate for every write; it
never writes to the terminal itself. Right before `/clear` it checks that the handover still
covers the session: the transcript's length is recorded when it is read for the handover (the
dialog's, the automatic one's and `compact --open`'s, which passes it in its request), and a
prompt recorded at or past that offset means the user sent something since, so it stops with that
reason before clearing. It also waits while the composer holds a draft or a picture, or anything
is queued or staging, and never clears under them; if they stay past the deadline it stops, the
session uncleared. At an idle prompt it then sends `/clear` with the gate's slash command path (up
to 90 seconds for the prompt to become idle), waits until the tab has followed
the CLI to the new session that `/clear` starts (the same following as for any `/clear`; the
running records are read every second meanwhile, up to 30 seconds; the new session counts as
followed once the pane is pinned to it and awaits its transcript, which the CLI writes only with
the first message), then puts the handover in the empty composer and sends it with the
composer's own send as that new chat's first message, so its checks, refusals and staging apply:
a handover of several lines is entered by the gate once the input line shows it, and a staging
the gate gives up on stops the handover with the draft left for the gate's recovery (up to 60
seconds). A draft in the composer is never replaced. Up to the same 15,000 UTF-16 units as a new
tab's first message the handover goes whole, pasted on the input (script `long-paste` of
`recorder compat` checks that Claude Code shows such a paste within the gate's five-second echo
wait); above it the first message asks the session to read the saved file, as for a new tab, since
a longer paste could be slow to show and the wait would end after `/clear` already ran. The
running session has no rule for that file, which lies outside the project folder: in manual mode
Claude Code asks first, and Chat's card answers. When the message is taken, the
new session gets the handover provenance and continuation name, and the old one is marked handed
over; then, once the new transcript exists and at the next idle prompt, `/rename <continuation
name>` goes through the same slash gate (which re-checks the session right before writing), so Claude Code's own name follows as for a tab rename. A step that does not happen in time,
a process that ends, a tab that changes session before `/clear`, or a Claude Code that draws its
full-screen view (where Chat cannot send) stops it with the reason; nothing else is sent, and the notice names the saved
handover file, which is kept either way.

## Pictures into the CLI

Ctrl+V, Ctrl+Shift+V and the palette's Paste send clipboard text using the terminal's usual paste
mode. With no text they send `ESC v` (Alt+V), so Claude Code reads its clipboard image directly.
The Attach picture button opens the toolkit's file picker. File drops arrive through the host's raw
winit hook (one file at a time, without a drop coordinate); the shown terminal tab receives the batch.
Both routes send space-separated paths, with whitespace-containing paths in double quotes, and use
bracketed paste only when requested by the program. They do not send Enter.

## Appearance

The Appearance page controls the window. The window theme is System, Light or Dark;
`--theme light|dark` overrides that preference. A dark and a light palette are chosen separately
(saved as `darkTheme` and `lightTheme` under `appearance`), each shown as a grid of equal cards,
five across at every Settings width: a small window drawn in the palette (title bar, side pane,
text, accent button, and a terminal with five of its ANSI colours) above the whole name. A custom
name too long for its card is elided, with the full name in a tooltip on hover. The chosen card has
an accent border and a check; hover lightens a card; the keyboard ring is the toolkit's, outside the
card (or round the whole line when its choice is on another line). Each line of cards is one form
choice row plus empty spacer rows (`tiles::line`), so the next setting starts below the cards; a
press between cards does nothing, and keyboard focus scrolls the whole line into view.
A click on the card chooses it; Tab focuses a row and Left/Right or Home/End changes
the choice. On a row without the current selection, Right starts at the first swatch, Left at the
last, and Enter or Space leaves the selection alone. Terminal presets use the same control.
Dark: Neutral (the toolkit's dark), Near black, Warm,
Blue-grey and High contrast (the toolkit's); light: Paper, Warm, Cool (the toolkit's light, warm and
cool) and High contrast. Every palette has its own terminal colours, which Automatic terminal colours
follow; the table of palettes and their terminal colours is `termtheme/palettes.rs`, the one place
to change them. An unknown saved id (from a newer version) is kept and shown as the first palette.
Windows high contrast takes priority over all of them. The toolkit's appearance model supplies the
accent options (Default, System or Custom), dialog and menu corners, optional soft shadows, and
the system-aware motion policy. Default uses Claude's clay on built-ins (held to 7:1 on the
high-contrast ones) and the saved accent on named themes.
Mica and Acrylic are optional on Windows 11 22H2 or later; the title bar is capped at 15%
compositor colour. Terminal opacity is set separately below. The tab strip stays opaque.

Named palettes are stored by stable `user-` id under `appearance.themes`. The existing
`darkTheme` and `lightTheme` keys select them as well as built-ins; no settings rewrite is
needed to upgrade. The name is display text, never a path. Window surfaces and automatic
terminal colours resolve by id, even when two palettes have identical window colours.
Theme files are versioned JSON containing only a name, base palette and `#RRGGBB` colours.
Import validates the whole file including all 16 ANSI colours; export creates a new file.

The Appearance library opens the selected dark or light palette as a draft. A built-in is
always copied; a user theme can be renamed, updated or saved under a new name. Generate shades
rebuilds surfaces, text and terminal colours from the draft's page and accent. Edits stay in
the form until Save. The editor reports the weakest text, dim text, selection, accent or terminal
pair, or the focused ANSI colour against the terminal background. Contrast below 4.5:1 is
flagged, not forbidden. The existing system/custom accent choice and Windows high contrast
still take priority. Settings use switches for booleans and segmented controls for short plain
choices. Session and folder context menus use the same switch drawing for pinned/hidden states.

Automatic terminal colours use the chosen palette's terminal colours (Neutral is `#101010`).
Match window uses the window bar background. Custom uses the background, text and accent picked on
the Terminal page; the renderer adjusts pairs below 7:1. An applied colour takes effect at once, and 9–24 logical px font-size
changes resize all open screens and pseudo-terminals. Cursor shape and background opacity are
described under [Terminal cursor and backdrop](#terminal-cursor-and-backdrop).

The four colour rows (accent, terminal background, text and accent) use the toolkit's confirming
picker (`Form::show_as_colour_with_confirmation`). The picker previews a choice without saving it;
Apply, Enter or a press outside the picker saves it, and that press activates nothing underneath.
Cancel and Escape put the opening colour back. Tab reaches both buttons. Held Enter or Escape
repeats are ignored while the picker is open, so the key that opened it cannot also close it.

Tab marks, hover, menu entrances, the dialog dimmer, sessions-list selection and settings controls
animate briefly. Reduce motion snaps them to their targets, as does the system setting. Tab content
switches instantly; terminal text never cross-fades. Real-mode running-session scans are unchanged,
but a polling tick no longer redraws the window unless visible state changes.

## Settings pages

The category list at left selects one page at right: Appearance, Terminal, Sessions, Chat,
Claude Code (with its sub-pages Advanced and Status, indented under it), Usage, Handovers, OpenCode, or
Notification area. A sub-page's name is "Parent › Name" in the code and its title; the rail shows
"Name", indented. Only the current page scrolls. Up and Down change
categories when the list has focus, Tab enters the page, and Escape dismisses a popup before
closing Settings. Each window remembers the last page while it runs; category selection is not saved.
A category switch cross-fades the old page into the new one in 140 ms (the toolkit's
`PageTransition`; the old page is drawn once more in the first frame and blended away). The app's
Reduce motion, Windows' reduced-motion setting and high contrast make it instant.

Controls still apply live. An ordinary config save, under the settings lock, re-reads the file and
applies only the fields this window edited against its loaded baseline, inside groups such as
`appearance` and `notifications` too, so two windows can change different settings. Only a field
both windows changed differently is refused, with a notice naming it; the other window's value is
kept and this window reloads the current settings, without asking for a restart. It never republishes
the separately persisted usage opt-in or handover consent; the tab-restore preference still uses
`window.ini`. Switching categories does not write settings.

**Peek.** Settings dims the window, so a palette, the backdrop, terminal colours or the Chat
reading width can't be judged under it. The **Peek** button (an eye, at the top right of the page's
card on its title's line; Close stays in the footer; tooltip "Hold to see the window without
Settings") hides the dim and the dialog while the left button is held on it, and Space does the
same while held when Settings has the keyboard and what has focus does nothing with Space: the
category list, no control, or a number. A text box, switch, segmented choice, palette card, button,
Close or an open dropdown or picker keeps its Space, and Space with Ctrl, Alt or Shift does not
peek. Both fade the layer (a snapshot of the window under it, blended) over 120 ms each way, instant
with Reduce motion, Windows' reduced motion or high contrast; when settled nothing of Settings is
drawn, so Mica or Acrylic shows exactly as without it. A pill at the top centre, under the title
bar ("Peeking · let go to return to Settings", an eye, accent border on the popup surface) says
what is happening. Settings keeps its page, scroll and focus, and nothing is saved by a peek.
While peeking the app swallows every press, key, wheel, text and pointer move (`AppState::
peek_event`), so nothing reaches the window under it; only the release of what started the peek
ends it: the left button anywhere (the press captured the mouse, so a release outside the window
arrives too), or Space's key release, which the host does not reduce and the app reads in its raw
winit hook. Losing the keyboard (Alt+Tab, a click in another window) ends any peek.

## Claude Code's /config

Claude Code keeps the choices of its `/config` panel (Settings › Config) in two files of the
account's CLI folder: the user `settings.json` and the global `.claude.json` (for the first account
`%USERPROFILE%\.claude.json`, beside the folder). Settings › Claude Code edits a few of them and
Settings › Claude Code › Advanced the rest (below).

**Where the list comes from.** Taken from Claude Code **2.1.332** (`claude --version`) on
2026-11-28, read only: the `/config` panel's entry list in the bundled JavaScript of
`%USERPROFILE%\.local\bin\claude.exe` (each entry's `id`, `label`, `value` expression, `options`
and the writer its `onChange` calls: the user-settings writer for `settings.json`, the
global-config writer for `.claude.json`), the global config's defaults (the function that builds
a fresh `.claude.json`, which also drops keys that equal their default when it saves) and the
settings schema's descriptions. Many entries only show when a feature flag, the plan or the
terminal allows; "shown when" says so. `tools/screen-recorder/scripts/config-list.txt` (written by `tools/screen-recorder/config-list.py`)
searches `/config` for each label Switch writes, so a recording shows which ones an account sees.
`crates/core/src/cc_config.rs` holds the same table for the page, and `tools/cli-compat.ps1`
checks that an installed Claude Code still has each of its keys and labels.

Store: **S** `settings.json` (user scope), **G** `.claude.json`. Page: **CC** Settings › Claude
Code, **Adv** Settings › Claude Code › Advanced, **—** left out (why, after the table). The default
is in bold, or named.

| # | Label in /config | Key | Store | Values | Shown when | Page |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Auto-compact | `autoCompactEnabled` | G | **true**, false | always | CC |
| 2 | Idle compaction | `idleCompaction` | S | **true**, false | feature flag | Adv |
| 3 | Continue automatically at usage limit | `autoContinueAtUsageLimit` | S | **true**, false | claude.ai plans | Adv |
| 4 | Use this machine's settings in cloud sessions | `remoteHomeSettingsMode` | G | `forward` or not | remote sessions available | — |
| 5 | Unattended commands from cloud sessions on this computer | (Claude Code's consent store) | — | Allowed, Not allowed | remote tools available | — |
| 6 | Switch models when a message is flagged | `switchModelsOnFlag` | S | absent (not asked yet), true (switch automatically), false (ask each time) | flagged-message fallback available | Adv |
| 7 | Show tips | `spinnerTipsEnabled` | S | **true**, false | always | Adv |
| 8 | Claude-drafted feedback | `feedbackDrafts` | S | **notify**, quiet, off | feature flag | Adv |
| 9 | Reduce motion | `prefersReducedMotion` | S | **false**, true | always | Adv |
| 10 | Show Clawd carrying files | `clawdFileProgressEnabled` | S | **false**, true | internal flag, connectors | — |
| 11 | Thinking mode | `alwaysThinkingEnabled` | S | **absent or true** (on), false | always (locked on for some models) | Adv |
| 12 | Fast mode (model) | `fastMode` | S | **absent**, true | fast mode available | — |
| 13 | Prompt suggestions | `promptSuggestionEnabled` | S | **absent or true** (on), false | feature flag | Adv |
| 14 | Session recap | `awaySummaryEnabled` | S | **absent or true** (on), false | always | Adv |
| 15 | Rewind code (checkpoints) | `fileCheckpointingEnabled` | G | **true**, false | checkpointing available | Adv |
| 16 | Synced project memory (this directory; applies next session) | (per-folder memory state) | — | on, off | org memory available | — |
| 17 | Synced project memory writes | (per-folder memory state) | — | on, off | org memory writes allowed | — |
| 18 | Dynamic workflows | `enableWorkflows` (also removes `disableWorkflows`) | S | **absent** (by plan), true, false | workflows available | Adv |
| 19 | Ultracode keyword trigger | `workflowKeywordTriggerEnabled` | S | **true**, false | workflows available | Adv |
| 20 | Dynamic workflow size | `workflowSizeGuideline` | G | **absent** (small on Pro, else medium), small, medium, large, unrestricted | workflows available | Adv |
| 21 | Artifacts | `enableArtifact` (also removes `disableArtifact`) | S | **absent** (on once available), true, false | artifacts available | Adv |
| 22 | Verbose output | `verbose` | G | **false**, true | always | CC |
| 23 | Terminal progress bar | `terminalProgressBarEnabled` | G | **true**, false | always | Adv |
| 24 | Show status in terminal tab | `showStatusInTerminalTab` | G | **false**, true | feature flag (`tengu_terminal_sidebar`) | Adv* |
| 25 | Show turn duration | `showTurnDuration` | G | **true**, false | always | Adv |
| 26 | Precompute compaction | `precomputeCompactionEnabled` | S | **true**, false | feature flag | Adv |
| 27 | Show message timestamps | `showMessageTimestamps` | G | **false**, true | feature flag (`tengu_silk_hinge`) | Adv* |
| 28 | Time format | `timeFormat` | S | **auto**, 12-hour, 24-hour, 24-hour-utc | always | Adv |
| 29 | Default permission mode | `permissions.defaultMode` | S | **absent**, default, plan, acceptEdits, auto, … | always | CC |
| 30 | Worktree base ref | `worktree.baseRef` | S | **fresh**, head | always | Adv |
| 31 | Use auto mode during plan | `useAutoModeDuringPlan` | S | **true**, false | always | Adv |
| 32 | Respect .gitignore in file picker | `respectGitignore` | G | **true**, false | always | Adv |
| 33 | Skip the /copy picker | `copyFullResponse` | G | **false**, true | always | Adv |
| 34 | Copy on select | `copyOnSelect` | G | **true**, false | fullscreen renderer | Adv* |
| 35 | Auto-scroll | `autoScrollEnabled` | G | **true**, false | fullscreen renderer | Adv* |
| 36 | Agents view | `leftArrowOpensAgents`, `defaultToAgentsView` | G | shown, not changed there | agents view available | — |
| 37 | Auto-update channel | `autoUpdatesChannel` | S | shown, not changed there | always | — |
| 38 | Theme | `theme` | S | dark, light, … | always | CC |
| 39 | Local notifications (Notifications) | `preferredNotifChannel` | G | **auto**, terminal_bell, iterm2, iterm2_with_bell, kitty, ghostty, notifications_disabled | always | Adv |
| 40 | Push when actions required | `inputNeededNotifEnabled` | G | **false**, true | push notifications available | Adv |
| 41 | Push when Claude decides | `agentPushNotifEnabled` | G | **false**, true | push notifications available | Adv |
| 42 | Output style | `outputStyle` | the folder's `.claude/settings.local.json` | default, explanatory, learning, … | always | — |
| 43 | Default view | `defaultView` | the folder's `.claude/settings.local.json` | **default**, transcript, chat | feature flag | — |
| 44 | Language | `language` | S | **absent** (English), any language name | always | Adv |
| 45 | Editor mode | `editorMode` | G | **normal**, vim (emacs reads as normal) | always | Adv |
| 46 | Question auto-continue timeout | `askUserQuestionTimeout` | S | **never**, 60s, 5m, 10m | not set by policy | Adv |
| 47 | Claude-proposed goals | `modelProposedGoals` | S | **auto**, alwaysAsk, disabled | feature flag (`tengu_propose_goal`) | Adv* |
| 48 | Show last response in external editor | `externalEditorContext` | G | **false**, true | always | Adv |
| 49 | Show PR status footer | `prStatusFooterEnabled` | G | **true**, false | always | Adv |
| 50 | Model | `model` | S | **absent** (default), opus, sonnet, … | always | CC |
| 51 | Diff tool | `diffTool` | G | **auto**, terminal | an IDE is connected | Adv* |
| 52 | Auto-connect to IDE (external terminal) | `autoConnectIde` | G | **false**, true | outside an IDE terminal | Adv |
| 53 | Auto-install IDE extension | `autoInstallIdeExtension` | G | **true**, false | inside an IDE's own terminal | — |
| 54 | Claude in Chrome enabled by default | `claudeInChromeDefaultEnabled` | G | **false**, true | always | CC |
| 55 | Teammate mode | `teammateMode` | G | auto, tmux, iterm2, **in-process** | agent teams on (`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS` or `--agent-teams`, and a flag) | Adv* |
| 56 | Enable Remote Control for all sessions | `remoteControlAtStartup` | G | **absent** (default), true, false | Remote Control available | Adv |
| 57 | Dialog expiry | `dialogExpiry` | S | **absent** (default), 60s, 5m, 10m, never | not set by policy | Adv |
| 58 | Messages from your other sessions | `crossSessionInbound` | S | **absent** (default), accept, hold, refuse | cross-session messages on | Adv |
| 59 | Always deliver in this session | (this session only) | — | off, mine, anyone | cross-session messages on | — |
| 60 | Delivery notices to and from cloud and Remote Control sessions | `remoteMessageReceipts` | S | **on**, always, off | cross-session messages on and a feature flag (`tengu_toasty_phoenix`) | Adv* |
| 61 | Show external CLAUDE.md includes dialog | (the folder's project entry in `.claude.json`) | G | true, false | a project with external includes | — |
| 62 | Use custom API key | `customApiKeyResponses` | G | approved or not | `ANTHROPIC_API_KEY` set | — |

The 46 entries an account saw in 2.1.332 begin with Auto-compact, Idle compaction, Continue
automatically at usage limit, Switch models when a message is flagged, Show tips, Claude-drafted
feedback, Reduce motion, Thinking mode, Fast mode, Prompt suggestions, Session recap, Rewind code,
Dynamic workflows, Ultracode keyword trigger, Dynamic workflow size, Artifacts, Verbose output,
Terminal progress bar, Show turn duration and Precompute compaction: the order above, less the
entries its flags hide.

**Adv\*** marks an entry `/config` lists only in some contexts. A recording of `/config` on
2026-11-28 (a signed-in account, an ordinary terminal, the recorder's tab) found 34 of the 43
labels; the nine missing were exactly the eight Adv\* rows and Auto-install IDE extension, each hidden by its
condition, none renamed (the keys check finds every label in the bundle). They stay on the page
because a Switch tab can be in that context (fullscreen mode, an IDE connected through Auto-connect
to IDE, agent teams on) or Claude Code turns them on per account. Their help line ends with when
they apply ("In fullscreen mode.", "With an IDE connected.", "With agent teams.", "If Claude
Code enables it." or "If enabled." for a flag), from `cc_config::WHEN`, and the recording's
expectation accepts "No settings match" for them.

Left out of Switch, and why:

- Fast mode: it changes the session's model and price and is only saved when the account may keep
  it; `/fast` in a session decides.
- Remote home settings, unattended commands and synced project memory: consent and identity
  checks Claude Code makes itself, and memory state per folder.
- Show Clawd carrying files: an internal flag.
- Output style, Default view and the external includes dialog: per folder (the folder's
  `.claude/settings.local.json` or its project entry), not per account.
- Agents view and Auto-update channel: `/config` shows them but does not change them there.
- Always deliver in this session: that session only. Custom API key: the approval of a key in the
  environment, which Switch removes from what it starts.
- Auto-install IDE extension: listed only inside an IDE's own terminal (VS Code and the like),
  which a Switch tab never is.
- Model, Effort, Permission mode, Theme, Chrome, Auto-compact, Verbose: on the Claude Code page.

Claude Code 2.1.332 also has a newer storage for user settings behind a flag (`storageV5`); the
page writes `settings.json` as the user-settings writer does without it. Seven of the G keys on
the page are in `settings_sync::PREFS` (Rewind code, Terminal progress bar, Show turn duration,
Local notifications, Editor mode, Diff tool and Auto-connect to IDE): they go to the sharing accounts
at the next sync, as Chrome and Auto-compact do; the other G keys stay with their account.

**Settings › Claude Code › Advanced** shows every **Adv** entry with a switch or a choice (a
segmented control up to four options, else a drop-down), a short line on what it does, and
"· default" after that line while the key is absent (Claude Code's default applies). A set key
gets a row under it, "Claude Code's default: On" with **Reset**, which removes the key; a choice
whose default depends on the plan or on what Claude Code asked has a "Default" option for that
instead. A saved value the page does not know is shown as it is and kept. A few labels are
shorter than `/config`'s so they fit beside the control at the narrowest Settings (the table
keeps both). **Search** at the top filters by label, `/config`'s label, key and help, case aside.

Two sections. "Shared · settings.json of <master>": saved in the master account and brought to the
sharing accounts by the same sync as the Claude Code page (`settings.json` is backed up before it
is replaced). "Per account · .claude.json": an **Account** choice (the master first time); only that
account's file changes. Claude Code holds the whole of `.claude.json` in memory and writes it back
while it runs and when it exits, so while the chosen account may run Claude Code its rows are
read-only: each shows its value as a single option with "Close <account>'s sessions to change
this.", and **Check again** reads again after they were closed. "May run" is any of
(`cc_config::refuse_if_running`):

- a live `sessions/<pid>.json` in the account's own CLI folder, whatever account the session is
  shown under: a desktop app's session of a further account runs in the first account's
  `%USERPROFILE%\.claude` and holds `%USERPROFILE%\.claude.json` (`running::live_folders` keeps
  the folder each record was found in);
- a running-session scan that could not read everything, or found a live process whose record
  names no session yet (one that is starting): then nothing tells that the account is free;
- a program in one of this window's tabs for that account (a Claude tab or a sign-in tab that
  started seconds ago may not have written its record yet).

The write checks all of it again, and also refuses when the account has no `.claude.json` yet:
Claude Code makes it on first use, and the sharing seeds a further account's `mcpServers` only
while there is none (`share::sync_mcp`), so a file of one key would stop that.

Every write (`cc_config::save`) reads the file again and compares the key with the value the page
showed; when another program changed it since, nothing is written, an error says so, and the
page shows the current value. Otherwise only the key changes (and, for Dynamic workflows and
Artifacts, the old `disable…` key `/config` removes too); every other key, known or not, is kept
in its place (serde_json's `preserve_order`; a removed key is taken out with `shift_remove`).
The file as read is kept in the backups folder (`settings-backups`, the newest 30 of each name,
`…-settings.json` and `…-claude.json`). The new file is written beside it, then the file is read
again and compared byte for byte with what was read, and only when equal renamed over it: the
compare is the last step before the rename. A file that is not a JSON object is never replaced.

For `.claude.json` the write holds Claude Code's own lock folder `.claude.json.lock`, which Claude
Code takes with `proper-lockfile` when that lock is on (its call passes no `stale`, so the
library's 10 s apply; a holder touches the folder every 5 s). A lock folder younger than a minute
refuses the write with "try again"; an older one was left behind by a Claude Code that ended, and
is taken over (only an empty folder is removed, and a racing taker makes the create fail), and the
page says so. When a `settings.json` save reaches the master but the sync to the other accounts
fails, the page says that the setting was saved in the master account and not shared yet.

Reads and writes run off the window thread (a background job per page); the page shows the
result when it ends, and a change made while one runs is not taken (the row shows its value
again). The page reads the files when it opens, after every change, and on Check again or
another account.

**Settings › Claude Code › Status** shows, for the account chosen on it (the same choice as
Advanced's), what `/status` and `/stats` show that files hold, so nothing is run
(`core::cc_status`). Read off the window thread when the page opens and when another account is
chosen; values are single-option rows, as the toolkit has no label/value row.

- Claude Code: the installed version, the newest folder of `%USERPROFILE%\.local\share\claude\versions`
  (the native installer's), with the version the Advanced table was taken from.
- Account: the sign-in method (a claude.ai login when `.credentials.json` exists, which is never
  opened, and `.claude.json` has `oauthAccount`; else an `apiKeyHelper` in `settings.json`; else a
  `primaryApiKey` in `.claude.json`) and `oauthAccount`'s email, organization and plan.
- Settings in effect: the user `settings.json` (how many keys, or not valid JSON) and a
  machine-wide `%ProgramFiles%\ClaudeCode\managed-settings.json`; a folder's own settings depend on
  the folder and are named only.
- MCP servers in `.claude.json`'s user-scope `mcpServers`, name and transport (eight by name, then
  a count); user memory, `CLAUDE.md` in the CLI folder, with its lines.
- Stats from the account's `history.jsonl` (one line per prompt: `timestamp` in ms, `project`,
  `sessionId`), only its last 8 MiB (the header says so when it is longer): prompts, sessions,
  local days with a prompt, folders. Models used: the newest 40 of those sessions whose
  transcript the session index knows, the last 64 KiB of each, the `message.model` of assistant
  lines (`<synthetic>` skipped), counted once per session.
- Usage is not repeated: a row opens the Usage page (plan usage shows in the top bar).

Left out because only a running Claude Code knows them: the model and permission mode of a
session, whether MCP servers connect, a session's cost and context use, and `/stats`' token totals
(they would need whole transcripts, not the bounded reads above). See TODO.md, item 81.

## Recent sessions and local marks

Recent is the default. It sorts sessions across folders by activity, newest first, with local
calendar headings: Today, Yesterday, This week (Monday onward), Earlier this month and Older.
It uses the current local offset for the comparison; historical daylight-saving transitions
are not resolved separately. See [Session organization](#session-organization) for the alternate modes.
Only the visible rows and sticky heading are drawn. Mode and Show hidden are config preferences.

Rows show the latest `last-prompt` record (`lastPrompt`) or real user message found in the
bounded transcript tail. Search and the quick switcher match the local name, original title,
last prompt and folder, and a session id: the whole id or a prefix of at least eight hex digits,
alone, or after `--resume` in a pasted line (`claude --resume <id>`), which ranks that session
first; in the Sessions list it goes into a first group, "Session id", also when hidden, archived or
a worker's (with the All or Hidden filter), its row showing that state as usual; the switcher orders by fuzzy match quality, then recency. Opening from
it uses the existing resume checks, and Ctrl+Enter opens the other-account menu.

The Waiting filter is a hint, not a process signal. A live session is waiting if its newest
meaningful record is an assistant end-of-turn message, or assistant text without an explicit
stop reason or tool call. A tool call, a tool result or a newer user prompt is not waiting.
Sidechains, metadata and summaries do not count. The heuristic can mistake streamed assistant
text for a finished turn, cannot detect approvals or stalls before a line is written, and can
miss turns whose meaningful records are beyond the bounded tail. Running sessions and the
selected transcript are checked for changes in the background; Refresh and returning focus
also rescan the index.

Local names, pins, per-session and per-folder hiding, and last-seen transcript timestamps live
in `marks.json` beside `config.json`. The file is replaced atomically, with unknown keys kept;
the app leaves a damaged marks file alone. Two windows may both write it: under the file's lock, a
save that finds the file changed since this window read it applies only this window's own changes,
per session or folder and field, onto the file as it is now, and the window then has the other's
changes too. Last-seen times take the later value, the missing-since stamp the earlier, and the
transcript path the one on disk; a session one window pruned stays pruned when the other only
updated those (its next scan marks it afresh), so an entry is never left empty. An empty (null)
entry an earlier version wrote is dropped on load. Only a field both changed differently (two names for the same
session, say) refuses the save with a notice naming it; the other window's value is kept, nothing
else from that save is written, and the next change saves normally. No restart is needed. Showing details or a tab clears Unseen for its
known transcript version. Remembered session marks expire only after the transcript has been
observed missing for at least 90 days. Hiding is reversible from Undo or Show hidden; it never
deletes a folder or changes a transcript. A blank local name restores the transcript title.

**Suggest names is opt-in.** It sends the first real prompt and up to the last three real prompts
from each chosen session (at most 500 characters per prompt) to Claude and uses a little of
the selected worker account's usage. The batch asks first, then takes at most 20 unnamed,
non-hidden sessions from the last seven days. The CLI runs headlessly with Haiku, in an empty
temporary folder, with session persistence disabled. Projects belonging to those temporary
folders are never indexed, even with tool sessions shown. You can cancel a run; each suggestion
must be accepted or edited before it is stored locally.
This is the additional process the core can start. For naming, the worker-account setting
uses the app's current account for Same, the next account for Other, or the configured account.

## Inside conversations

**In conversations** (Ctrl+Shift+F in Sessions) is separate from title/folder search. After a
220 ms typing pause, the worker scans the indexed transcripts newest first. It searches case-insensitive
literal terms in any order on one decoded text line, excluding metadata, sidechains, thinking, tool
outputs and pictures. It streams up to three highlighted excerpts per session and reports the remaining
count when that file finishes. The list still honors visibility and status filters. Escape, clearing
the field, or switching modes cancels the scan; there is no persistent index. The scanner reads in
64 KiB chunks, checks the top-level type before JSON parsing, and uses the transcript reader's
48 MiB record cap. Incomplete and oversized records are skipped.

**Conversation size** reads only the selected session on a worker. It counts user prompt records,
tool calls (unique by tool ID when present), picture blocks including tool-result pictures, and
subagent transcript files. The latest assistant record's `message.usage` supplies **Context now**:
input plus cache-read and cache-creation input. **Output so far** sums the latest reported output
for each assistant message ID. Missing fields are optional; absent context uses visible conversation
characters / 4, and messages missing output contribute their visible assistant characters / 4.
An estimate is marked **about**. Results are cached in memory by path, file size and modification
time, also invalidated when the subagent directory's modification time changes. Demo uses synthetic
JSONL without touching the listed paths.
`--demo --demo-search "width splitter"` opens the Sessions tab with an example search.

## Window placement and reopening terminal tabs

The window saves the normal placement and maximized bit in `window.ini` via the toolkit’s
Windows placement capture. Restore keeps a rectangle when any one connected work area shows
at least 100 × 100 physical pixels of it. Otherwise it centers the normal rectangle in the
primary work area, shrinking it to fit if needed, then reapplies the maximized bit.

On a confirmed exit, `terminal-tabs.json` is atomically replaced with session UUIDs, account
slugs, launch folders and active flags, in strip order. Shell and sign-in tabs never enter the
snapshot; neither local tab labels nor terminal contents are saved. For a new or forked tab,
identity must match the PTY’s live CLI process (or its immediate npm shim child), account and
folder. Ambiguous or unavailable identities are not guessed. Ask is the default; Always
starts the batch after the first scan, Never clears the snapshot. The pending offer survives
an exit with no replacement tabs. Only the window holding the handover channel restores and saves
the snapshot; a second window started meanwhile neither restores it nor writes it on exit (its
tabs are not saved), so the first window's tabs survive whichever window closes first. The one
exception is a second window that closes after the first has gone: on exit it tries to claim the
channel, and when it gets it (no other window holds it) it is the last window, re-reads the
snapshot the first one left and replaces it with its own tabs. With no tabs of its own it leaves
the first window's.
Restoration uses the normal resume warning, process scan
and launch reservation, always targets embedded tabs, ignores a stale saved folder in favor of
the indexed one, and reports skipped entries together.

The Sessions tab is fixed. Terminal tabs reorder when dragged past a neighbor’s midpoint.
Right-click actions target the tab under the pointer, not the selected session row. The
account-copy path uses the same fork and cache warning as the Sessions list, and compaction
opens the handover dialog. Renaming a tab that runs a Claude session the list knows renames
the session: its Switch mark (so the list, search, the tab label and a restored tab use it;
`terminal-tabs.json` still holds identifiers only) and, when the tab's live identity is verified,
Claude Code's own name through `/rename <name>`. That command is never typed directly: it waits
on the tab until the tab is shown in the Chat view and is handed to the Chat view's slash gate,
which refuses a running turn, a pending tool, a menu, a full-screen view or a prompt that is not
idle and empty, and shows why; its own retry clears the refusal once the prompt is idle, and
the next tick sends. A handover's rename goes only once Chat has read the new session's first prompt: after a
handover's `/clear` the gate cannot tell the new session's first turn from an idle input before
that, and a `/rename` typed then was left on the input when a permission prompt took the screen
before its Enter. It is followed until Claude Code took it, like every command Chat types on its
own (a button's, a handover's `/clear`): after Enter the input must let go of the line within
three seconds (other text read at rest, or an empty input; one reading of a redraw caught half
way is not enough); a line that was not entered (its echo never came, a prompt was drawn over it,
or Enter did not take) comes off again with Ctrl+U, one press at a time and only while the input
shows exactly that line at rest and the gate would let a key go (no running turn, menu, pending
call or other session; once the input can be read again: a prompt over it is answered from its
card meanwhile; time a turn runs, which holds the Ctrl+U back, does not count against the
minute); text of anyone else's is never cleared, and terminal input or another session ends the
following without a key (after Enter, a rename so ended is dropped without a word: most likely
Claude Code took it). An input Chat cannot read for a minute after Enter (a panel the command
opened, as `/config` or a bare `/model` do) means the command ran: following ends without a word,
unless the input, once readable at rest again, shows exactly the line, which the box then says. A
`/rename` that was not entered is then typed again, at most three times, after which the box says to rename
in the terminal. A command that did not run ends its tracking with it, so no line left on the
input holds every later message without a word. A handover's `/rename` waits for the new
session's first prompt at most two minutes; then the box says to rename in the terminal. The demo has no CLI to follow. A tab that starts running another session drops the waiting rename. A blank
name clears the mark and sends nothing (a bare `/rename` would make Claude Code generate a name).
Claude Code's docs say a session answers to the name set with `/rename` or `--name` in
cross-session messaging; its per-process record (`~/.claude/sessions/<pid>.json`, `name`,
`nameSource: "user"`) is not documented. The transcript is never edited. A plain terminal tab,
or a session not yet indexed, keeps a label-only rename until close. Close others keeps the clicked tab
and asks once if any of the others still run. Ctrl+Tab and Ctrl+Shift+Tab keep cycling as
before; Ctrl+1–9 selects a tab only in Sessions or after opening the command palette with
Ctrl+Shift+K, because the terminal encoder can turn Ctrl+digits into meaningful input bytes.
The process-free demo includes a synthetic restore offer; `--demo-offscreen` also exercises
recovery of a maximized window from a disconnected monitor.

## Folder launch defaults and local archive

`marks.json` stores `folders[folder_key].openDefault` (account slug and app) and
`sessions[id].archivedMs` (timestamp). An explicit change or a successful launch through the
Sessions list updates the folder default; cancelled and refused launches do not. The default
Open action uses the session’s last app when there is no folder preference; New session uses
CLI in that case. A removed account falls back to the selected account without changing the
preferred app. Defaults do not change the global account selection.

Archive is independent of Hide and of the Desktop app’s archive flag (`isArchived`). The UI keeps
the two apart by name: "Archived" alone always means Switch's archive (the **Archived here** chip,
the row's Archived mark); the desktop app's flag is **Show sessions archived in the desktop app**
in Settings → Sessions (off by default; such rows are dimmed when shown) and **Desktop archive: yes**
in the details. An age threshold counts
back whole weeks from now, excluding the boundary itself and unknown ages. The preview shows
the eligible IDs and their indexed transcript bytes. Commit considers only those IDs, rechecks
pins, live sessions, embedded tabs, recent launch reservations and transcript modification
times, then saves the marks as one update. An incomplete running scan refuses the operation.
Archived rows are omitted from ordinary views but found in searches; the Archived here chip
appears only while there are archived sessions in the index. Unarchiving preserves Hide marks.
No archive operation relocates, deletes, or edits a transcript (deleting is separate: see
[Deleting a session](#deleting-a-session)).
The session menu explains why a pinned or running row cannot be archived. Folder archive
remains available because its preview can include other eligible rows.

The Markdown report covers all currently indexed archived sessions. It uses bounded
transcript heads/tails for the first dated conversation record and first/last prompt excerpts,
the latest local handover filename if present, and the local stats reader for turns and
context/output tokens (`~` denotes an estimate). Unknown dates or statistics are shown as
`—`. Rendering is pure; Save As warns about conversation text, writes a new file without
replacing an existing destination, and offers to open it explicitly. No AI or network is used.

## Deleting a session

**Delete session…** in a session row's menu (right after Archive session…) moves the session to
the Windows Recycle Bin; nothing is deleted permanently by Switch. It moves the one real
transcript `<config dir>/projects/<encoded folder>/<session id>.jsonl` and, when present, the
`<session id>/` folder beside it (subagents, tool results), which belong only to that session.
The path is followed to the real file first, so a transcript reached from several accounts
through the shared `projects` junction is moved once. The transcript must be a plain file named
after the session inside a `projects/<folder>/` folder; a session folder that is itself a link is
left in place (and said so), never followed. The transcript goes first: if it does not move,
nothing does. A folder that then does not move is reported as a partial deletion with its path
and reason, never silently. The desktop app's records (`claude-code-sessions/…/local_*.json`)
are not touched; when the session has one, the confirmation says the desktop app may still list
it. The move uses the shell (`SHFileOperationW`, `FOF_ALLOWUNDO`, no confirmation and no error
UI, with `FOF_WANTNUKEWARNING`: where the drive has no Recycle Bin or the file is too large for
it, the shell asks before deleting permanently, and declining keeps it). Switch checks that each
path is gone afterwards.

Afterwards the session's own entry in `marks.json` (name, topic, pin, hide, archive, seen time,
handover links) is removed through the marks' merge, so another window's changes elsewhere are
kept; other sessions' entries, including links that name it, are kept. Its summary in
`meaning.json` is removed. (A summarizing run already going when the session is deleted can write
its summary back; it names a session no longer listed and is never shown.)

It is refused, with the item disabled and the reason as its hint (Running, Open in a tab,
Pinned), while the session runs (a live pid in any account's `sessions/`, including the
desktop app's), while a tab of this window shows it, or while it is pinned. The confirmation
names the title, folder and "moved to the Recycle Bin"; **Delete** is not the default button
(Enter and Escape cancel). Pressing it checks again: the window's own state, the saved pin (another
window may have pinned it), then, under the launch reservation lock, the running sessions of every
account and launches reserved a moment ago; an incomplete running check refuses. `--demo` deletes
nothing and says so. The Delete key still toggles **Session hidden**.

**Delete the original afterwards.** The row menu's **Open a copy, then delete the original…**
(after Open a copy) opens a copy as the current account after one confirmation (which carries the
cold-cache warning when it applies) and deletes the original once Claude Code has written the
copy's transcript (a record of the copy's session id at its head; Claude Code writes it with the
copy's first message). Until then the original is kept; if the copy's tab ends first, it is kept
and a notice says why (a copy in an external terminal is waited for while the window runs). The check runs with the running poll. Plain **Open a copy** is unchanged and
deletes nothing. The handover dialog's review step has a checkbox **Delete the original session
afterwards** (Alt+D), off each time the dialog opens. It acts only after the handover fully
succeeded: a new handover tab (or OpenCode tab) started, or, continuing in place, the first
message taken and the tab following the new session; there it also refuses unless the tab follows a
session other than the original. Every case uses the same checks, Recycle Bin path and mark
cleanup as Delete session…; when a check fails then, the original is kept with a notice naming
the reason (a handover opened in a new tab from the session's own tab therefore keeps it until that
tab is closed and it is deleted by hand). `/switch-compact`, `compact --open` and automatic handover
have no such option.

## Background notices and notification area

An embedded Claude tab can ask for attention when it is not the focused window's selected
tab. The watcher reads the same bounded transcript tail and uses the Sessions list's Waiting
classifier, even if the Session view is closed. A distinct real user record re-arms the notice;
terminal bells and a recognized interactive permission prompt share the same per-turn deduplication.
Old completion records do not notify on resume. The mark clears when seen or when a new turn starts.
The permission prompt check recognizes English Claude CLI text; a changed or unrecognized prompt
without a bell is not detected. A very long turn whose user record has fallen outside the bounded
tail cannot reliably be distinguished from the previous turn.

The unpackaged Windows build uses `Shell_NotifyIcon` balloons rather than registered Windows
toast activation. There is no installer or Start menu identity requirement. **Toast and mark**
requests a balloon when the optional tray icon is present and flashes the taskbar once; without
the icon it only marks and flashes. **Mark only** does not flash or request a balloon. Windows
can suppress balloons. Clicking one opens its associated tab if still available. Explorer
restart re-registers the icon. Close to tray hides the window only if the icon successfully
registered; its Quit command uses the usual running-program confirmation.

When usage warnings are on, running embedded Claude sessions can request their accounts'
readings through the existing five-minute automatic throttle and cooldowns, even when hidden.
Successful readings can trigger one 80% and one 95% notice per account and session, week, or
model reset window. With no tray icon, a background warning flashes the taskbar and is shown
in the window when it regains focus. `usage-notices.json` remembers the thresholds across
restarts; it stores only account slugs, limit labels/types, resets and notified thresholds.
Without a reset timestamp the app cannot distinguish subsequent windows for that limit.
The usage pop-up can offer an account with a recent, comparable reading below the warning
threshold. This opens the handover dialog for the visible tab and preselects the account;
it neither closes the running tab nor sends the transcript at the click.

`--demo --demo-notifications` uses two synthetic tabs and usage readings without a CLI launch or
a request. The tray tooltip has a 127 UTF-16 unit Windows limit, so long lists may be truncated.

## Shared OpenCode CLI and Workers controls

`crates/opencode` is the MIT-licensed `opencode-cli` workspace library for Switch and other GUI
clients. It centralizes native executable discovery, bounded no-window process capture and
deadlines, service probes and verified restart, model IDs, and cancellable UTF-8 stdin prompts.
Switch still supplies its sanitized child environment and the handover-only standalone agent
policy. `opencode models` supplies provider/model IDs; richer model catalog data remains available
via the generic runner. The library doesn't use GUI APIs or connect to the service itself.

Settings → OpenCode shows the results of background probes. The probe checks `service status` before
`session.active`, because the direct API command can auto-start the service. The restart
confirmation counts only `running` sessions, warns that jobs will be cut off, and identifies
an unavailable count as unknown. Cancel is the default. The worker asks again if the count
changes after confirmation. Restart tries the native restart, falls back to stop/start, and
checks the active-session response before reporting success. Config Studio is only offered when
its executable is found at the fixed Local AppData install location.

For longer handovers, the preview names the file-based behavior. Open saves the exact preview
under the app's data folder, including edits, before constructing a first prompt that names the
absolute path and asks Claude to read it in full. It does not send the oversized text in a
command-line argument. `--demo --demo-workers` and `--demo --demo-compact-long` show these
behaviors with synthetic data and start no child programs.

## Chat over the terminal

Chat uses the same ConPTY and the same verified, incremental transcript reader as Session view.
It adds no API, launch path, or second Claude client. The layout is saved per restorable tab,
with the last explicitly selected layout as the new-tab default in `window.ini`. Drafts, attachment
chips, and drawer state stay in memory only. The closed drawer keeps its previous PTY grid.
Terminal layout keeps its normal Session view and controls. F6 switches layouts only after the
command palette is open; a focused terminal still receives its ordinary F6. Ctrl+Shift+E remains reserved by the window
and toggles the drawer in Chat.

**What counts as ready.** The screen is what the next key meets, so the screen decides (decided
2026-11-27, after 0.1.2 refused nearly every send; the owner: "zero instances where I have to open
the terminal"). Ready is the CLI's input shown idle: the live cursor's input row (`❯`, or `>` right
under the input box's rule where Claude Code draws without Unicode; boxed rows too) visibly empty,
SGR-faint placeholder and suggestion characters aside; no working line (the interrupt hint in the
rows under the input, or the CLI's own spinner line right above it: never those words in an answer);
no recognized menu, permission question or alternate screen; bracketed-paste mode; and at least
750 ms without terminal output or input. A message Chat just submitted must have shown first (newer
output, or five seconds). The transcript only hints: it shows no completion for a resumed session,
its last record after an Esc reads as a running turn, and a command or a key in the terminal leaves
it stale. It still says a turn runs (a newer prompt with no end, a pending tool) while the screen
does not show an input at rest, and a newly observed `end_turn` still makes a quiet demo or test
screen ready. The OSC title is no protocol: explicit busy labels only veto a send. A successful
write only means input was handed to the terminal, and there is still a race between observing the
PTY and writing it.
While a turn runs and the CLI's input reads visibly empty, Send types the message into it as
**Send now** does (below), and Claude Code hands it to the running turn as when typed in the
terminal; the box is free for the next message once it went. Into a running turn Enter goes only
once the input row shows the message (as for several lines), so a permission prompt or menu drawn
between the paste and Enter never gets it; the input may show the text or Claude Code's paste
marker (`[Pasted text #1]` for a long single line), and a call the turn records meanwhile does not
stop it. Such a send leaves the turn as the transcript shows it (Claude Code records the message in
that turn, never as a new prompt), so Stop stays available. Whether a send goes into a running turn
is read from the gate when its first key is written, not from the button: Send now into an idle
input (no turn, or one that ended while the draft waited) is an ordinary send.
A call the turn runs does not hold the message back once the screen shows the CLI's working line
and the call has been pending for 750 ms (a permission prompt is drawn as soon as its call is
written): Claude Code queues a message typed during a running command as at any other moment of
the turn (decided 2026-11-28 with queued messages, below). Within that moment the draft is held,
and goes into the turn by itself, under Send now's checks, once the call counts as running (or as
an ordinary send if the turn ends first). While a call is pending a message that a prompt drawn in
the same instant would take as its answer (one character, or a number of up to three digits) is
held until no call is pending. A command or the override is then refused as during any running turn, not as an answer
Claude Code may wait for: a permission question has no input box and is read first.
While messages wait in Claude Code's queue its input reads empty ("Press up to edit queued
messages" is faint); Esc then sends them at once as the next turn (recorded from 2.1.332).
**Queued messages.** A message whose Enter went into a running turn waits in Claude Code's queue
(the screen shows it with "⎿ Queued · ctrl+x ctrl+s to send now" under the queue) until Claude
Code hands it to the turn at its next step, recorded as a `queued_command` (a prompt "during a
turn"), or sends it as the next prompt when the turn ends or is stopped. Until the transcript
shows it, Chat shows each such message as a quiet bubble at the end of the conversation, in the
order it was sent, with "Queued — waiting for its turn" under it (`chat/queued.rs`). A bubble goes
with the first prompt after its Enter (the transcript's length then) that reads the same,
spacing aside (or the same after attached paths or a picture's marker), each prompt taken by one
bubble in order; at an idle input Claude Code's queue is empty and what is left goes; another
session's transcript or an ended program forgets them. Under the bubbles,
**Send now** (Send all now for several: Claude Code hands its whole queue in at once) writes
Claude Code's own `chat:sendNow` key, Ctrl+X Ctrl+S, the key the screen names. With its input
empty it hands every queued message to the running turn at once, moving a running command to the
background (its call's result says so) and stopping nothing (recorded 2026-11-28 from 2.1.332,
`recorder compat` script `queued-now`; Ctrl+Enter is bound to the same action but a terminal
cannot always tell it from Enter, and Ctrl+X Enter is `chat:queueSubmit`). Chat writes those two
keys only while the screen names them under the queue, the gate reads the turn running with no
card, prompt or menu up, the input reads empty (with text on it the same key sends that text too)
and no other keys of Chat's are on their way; it never types a queued message again or takes one
off the queue. While the keys' result is not in the transcript the button reads "Sending now…"
and takes no click, for five seconds at most. A draft held in the box (above) is not in Claude
Code's queue and has no bubble. A draft that only has to wait is held, not
refused: a running turn whose input does not read empty, a prompt the CLI is still
drawing, a command's output, a new chat's transcript, a picture while the input is not empty. It is
locked (text and attachments) until it sends or is cancelled, and goes out by itself, also in a
hidden tab, once the input is idle (or a newer completion shows, where no screen can be read).
**Send now** on a held draft pastes into the visibly empty input while a turn runs, and is the
explicit override (bracketed paste, outside the alternate screen, no pending tool, no text on the
input) when no prompt can be read. Refused outright, with the reason in the composer, are only
what waiting cannot fix: text already on the CLI's input, a card that wants an answer first, an
unverified session, an ended program.
Interrupted before any answer, Claude Code 2.1.332 takes the turn out of the conversation and puts
the prompt back on its input, where it would block the next send. While that input shows exactly
the message Chat last submitted, with no working line and no terminal input since, Chat empties it
with Ctrl+U (one line a key, each after the screen showed the one before, two seconds a key, forty
keys at most) and puts the text back in the message box; a picture has to be pasted again. Once
the input reads empty the text comes back when the screen goes quiet. While output keeps coming
(a turn starting by itself, a redraw) one empty reading may be a screen caught half way, so it
takes two in a row, each of newer output than the last Ctrl+U's; without them by the key's
deadline the text still comes back (no Enter went, so it was not sent), with a notice to check
that the CLI input is empty before sending. Every message Chat sent since the last one at an idle
input counts for the match (one sent during the turn does not hide the turn's own prompt), and
other text resting on the input after a Stop is said in a notice rather than left to hold the next
send silently. The take-back starts only when the message box is empty, with no
attachments and no held draft: otherwise the message stays on the CLI input, with a notice to send
or clear it in the terminal (which lets a held draft go, back to the box) and then send the draft.
Text the user typed in the terminal is never cleared, and no text is cleared without a place to go.
A minimized window, an open dialog or another blocking window overlay pauses automatic writes in
every tab; stage deadlines still run. **Cancel** unlocks a held draft without sending. No path
overrides a recognized permission question, a pending tool or a menu. Direct terminal input cancels a queued or staged send, and samples a
transcript byte watermark before writing so a buffered earlier prompt cannot rearm Chat.

**When the drawer opens.** For what Chat cannot read. A pending tool first observed at least four
seconds ago plus 750 ms of terminal silence, or a running turn whose terminal has been silent for
four seconds, with no working line on the screen, no text on the input and no card for what shows:
the CLI may be waiting for an answer ("Claude Code may be waiting for your answer in the
terminal."). Under the working line the same silence is a slow tool. A BEL counts only while the
screen shows neither the working line nor an idle input (an idle input clears the bell). Neither
that wait nor the bell opens the drawer by itself any more: the terminal card shows the screen
instead (below). A permission prompt Chat cannot read whole and a process exit still open it.
When it opens for a permission prompt Chat cannot read whole, or a card's answer could not go, or
"Open terminal" on a card or **Answer in terminal** on the terminal card is clicked, the terminal
takes the keyboard, so the answer can be typed at once; an automatic opening, or a card's answer
that could not go, leaves it with Chat while a key went to the message box within the last second,
the box or a card's field holds text, or the conversation has a selection (Ctrl+C is meant for it
and never reaches the CLI that way). An automatically opened drawer closes on observable turn
progress; a manually opened one stays open, except the one **Answer in terminal** opened (below).
Chat never answers a dialog by itself; the only dialog keys are a card's, after a click (see
Questions and Permission prompts).

**The terminal card** (`view/chat/terminal_card.rs`). While Claude Code may be waiting, the bell
rang, a permission prompt Chat cannot read whole is shown with the drawer closed, or a send is
refused because the screen cannot be read (the check, prompt, input, menu and full-screen
refusals, while the gate is not Ready), and the drawer is closed and no question card shows the
prompt, a card above the message box shows the part of the live screen that matters, with the
cause as its title. Its rows: from the last row within the bottom 14 rows with text whose column 0
is Claude Code's bullet (`●`, or `⏺` on renderers that use it) down to the screen's last row with
text; without such a row, those 14 rows. A running call's bullet blinks, so its row also counts
while the bullet is off: columns 0 and 1 blank, `Name(` from column 2, and under it the call's
`⎿` row or the working line (a 2.1.332 recording caught it off). They are read from the screen's cells each tick (colours
and attributes re-encoded as SGR) and laid out with the same grid renderer as command output,
in the terminal theme's colours at full strength; one terminal row stays one card row, the text
shrinking to fit the width down to 9 px, past that cut on the right. The card takes at most 60%
of the conversation's height and follows the screen while it stays. It only looks: it never
writes to the terminal and takes no key. **Dismiss** hides it until its cause is gone.
**Answer in terminal** opens the drawer with the keyboard; that drawer closes by itself, and the
keyboard comes back, once the cause is gone by the automatic drawer's rule (no reason left, or
one a card in Chat answers: a question, a request, a menu, a slash command; for a refusal, the
screen readable again), unless the user resized, closed or reopened the drawer, or typed in Chat,
since the click. The card goes once Chat can read the screen again or the terminal is in view. Stop sends Escape at most once per freshly
verified active turn.
**Command results and shell mode.** A `<local-command-stdout>` (or `-stderr`) record becomes a finished
row named after the slash command before it, and `<bash-input>` with its `<bash-stdout>`/`<bash-stderr>`
becomes a "Shell" row (`! command`). Both are quiet tool rows (see the conversation look above). A
one-line reply of at most 140 characters ("Set model to …", "Settings dialog dismissed") sits on the
row as its target and opens on a click like any tool. Longer output and shell output are a card
under the row, open at first and folded by a click (no output, one line); a shell
command whose output is only stderr is a failed row. Output is cut at 60 lines or 6,000 characters,
escape sequences and surrounding blank lines removed, the first line's indent kept
(`transcript/local.rs`); beside that plain text the core keeps the output with its escapes, up to
64 KiB (`/context` colours each cell of its grid). The card draws that copy as the terminal draws
it (`view/ui/grid.rs`, cmd-output): fed into an off-screen `mdshared_term` screen as wide as the
box and painted with `termview` in the terminal's font, cell grid and theme colours, so
`/context`'s grid and the legend beside it line up and keep their colours. Text the program gave
no colour is drawn part way to the background, as other tool output is muted. Only text, line
ends, tabs and SGR reach the emulator (other escapes and control characters are dropped, so
output that moves the cursor or clears the screen still reads top to bottom); lines wider than
the box wrap; selection and Copy give the grid's plain text. The Session view draws it the same
way. Without the escaped copy (older records) the plain text is drawn in the same grid. They never count as pending calls. Interactive panels that print their
result the same way (`/context`, `/usage`, `/config`, `/mcp`) get the same row; a command that
printed nothing shows no row and does not lend its name to the next output. `!` typed into the
empty box turns the composer to shell mode, as in Claude Code (a `!` mark before the text; Backspace
in the empty box or a click on the mark leaves it). The command goes as `!command` pasted into an
idle input only, one line and no attachments, and Enter only once the cursor's row reads
`! command`: Claude Code 2.1.332's source switches an empty input to shell mode when text starting
with `!` reaches it, but how the screen shows that is not recorded yet (`recorder compat` script
`shell-paste`). Without that row nothing more goes and the text-on-the-input notice shows
(`chat/shell.rs`). The mode ends with the send. A command that wraps is read over its rows, from
the `!` row down to the cursor's (no rule between), spacing aside. A draft whose text starts with
`!` (pasted, recalled, set by code) turns the box to shell mode with the `!` as its mark, as Claude
Code would take it; one sent that way is first shown so, with a notice, and goes on the next Send.
Shell mode with no command yet counts as a draft: the in-place handover, text a card leaves for
the box and a message given back after a stop wait for it; the suggested next prompt and a start
under a new session's box (clicks of the user's) leave it.
**Slash commands.** An initial `/` opens Switch’s list, never writes a bare slash to the PTY.
The list filters a reference catalog of built-ins and read-only names and frontmatter descriptions
from the selected account, the launch folder’s `.claude` directory, and installed plugin caches.
It does not execute plugins or mirror CLI policy, and caches the scan for the tab. Stars persist in
`window.ini`. Use ↑/↓ to choose, Tab to fill, and Enter to run. Model and Effort offer inline choices
and toolbar menus; Model can also open the native picker. Model, effort and context labels use
reported values, not clicked values. Effort is, in Claude Code's order: `CLAUDE_CODE_EFFORT_LEVEL`
in the tab's environment (it outranks `/effort`), else the newest "Set effort level to …" reply in
the transcript's `<local-command-stdout>` records, whoever typed the command (a newer "Set model to
…" sets it back), else the tab's `--effort` flag, else `effortLevel` in the project's
`settings.local.json` and `settings.json` and the account's `settings.json` (read once when the tab
opens, never written), else the model's documented default, marked "(default)"; unknown stays
blank. Per-model `modelSettings` and managed settings are not read. After a `compact_boundary` or
compact summary record, Context reads "<postTokens> after compact" from the boundary's
`compactMetadata` ("— after compact" without it) until the next answer reports a size; the Session
view's "Context now" does the same. Direct slash commands need a
verified, quiet, visibly empty CLI prompt, no active turn or blocking interaction, and the same
terminal mode checks as Send. Chat writes command text first, and only writes Enter after seeing
the exact text on the live input row, within two seconds. Compact and Clear (including Clear aliases)
ask for confirmation.
**CLI menus.** Menus, pickers and dialogs used to stay in the drawer. The owner gave that up with
item 42's reason ("give that up, otherwise all I see is that chat has stopped. convenience trumps").
Any menu whose footer (an "Esc to …"/"Enter to …" hint, or arrows with navigate/select/choose) is the
last row by the input shows as a card when no question or permission card covers it: its title row
and its numbered choices as buttons, answered like a question (arrows, then Enter once the screen
shows the highlight on that row with the same title and choices); the step ends as soon as that
menu is gone. A menu without numbered rows is a "Claude Code panel", read from its own top: under the
nearest full-width solid rule above its footer, without the settings dialog's tab row ("Settings
Status Config Usage Stats"), whose selected (inverse) tab becomes the card's title. A panel taller
than the terminal has no rule on screen; it is then read from the top of the screen, unless the rows
above its footer show conversation (a `❯`, `●` or `⎿` row), when only its last 14 rows are. The card
grows to fit, up to its usual cap, and names the rows it has no room for. Its rows are read-only (a usage row,
block characters then "N% used", drawn as a bar in the theme's colours; a row starting with a spinner
glyph keeps it in the working line's fixed slot, as every copied CLI row does), with Close (Esc) when
its footer offers Esc and Open terminal otherwise; while it is up the composer says "Close Claude
Code's panel above to send." instead of notices about the terminal's prompt. While a
card shows a menu, sends are refused with "Choose in Claude Code's menu above" and the drawer stays
closed; a slash command no longer opens the drawer by itself. A menu that may be a permission
prompt never shows as a menu card: within 24 rows of its footer a "Do you want to"/"Would you like
to" row, "Tab to amend", "don't ask again" or "tell Claude what to do", or Yes and No choices over
"Esc to cancel". Only the menu's own rows are read for these: those below the nearest rule above
the footer (the CLI draws its prompts and menus under one), so the same words in the conversation
above do not count; with no rule in those 24 rows, all of them do. Unless a question card answers that menu, the gate treats it as a permission
prompt: one Chat recognises gets a permission card (see Permission prompts), one that only looks
like a prompt is left to the terminal (drawer, notice). The list of calls auto mode blocked
(/permissions › Recently blocked, footer "r to retry · Enter to approve") is no menu either: Enter
on a row approves that call, so it has a card of its own (see Denied permissions).

**Questions.** Claude Code 2.1.332 writes an `AskUserQuestion` call to the transcript only once it
is answered or declined (checked with copies of the transcript while a question waited 30 s; a
`Write`, `Bash` or `ExitPlanMode` call is written at once). So while a question waits the screen is
all there is, and it shows through the menu card (see CLI menus): "Claude asks", the question's
text (every row between its tab row, " ☐ Header" or a form's "←  ☐ A  ☐ B  ✔ Submit  →", and its
choices), each choice with the description the screen shows under it, a form page's unnumbered
Next or Submit row where the arrow keys reach it, and the review ("Review your answers", which has
no footer) as its two entries with the answers in the title. A click moves the highlight and
presses Enter under the menu card's checks; a box is ticked the same way. The working line reads
"Waiting for your answer" while that card shows, whatever the transcript says. The user's own answer goes
through the card's field while the question's "Type something" row is as the CLI first draws it
("Type something.", or "[ ] Type something" in a multi-select): arrows to that row, the text only
once the screen shows the highlight there, and Enter only once that row reads exactly the text,
the rows it wraps onto included ("4. my own answer"; recorded 2026-11-27, also without Unicode). In
a multi-select the row's box ticks itself as text is typed and Enter would clear it again, so the
step ends once the row reads "[x] text" with its tick, and the form's Next or Submit row sends it.
A text starting with a digit is refused (digits pick choices), as is one with control characters;
a row that does not show the text within two seconds stops with a notice and no Enter. Text
already typed on that row is a choice like any other and is not edited from Chat.
A pending `AskUserQuestion` call that the main transcript does hold is shown as a card built from
the call's input (header, question, options with descriptions), never from the screen. A lone
single-choice question is answered by one click: before any key it requires the live screen to show a menu footer,
the question text, and every option as `n. label` in order, followed only by "Type something" and
"Chat about this", and not to look like a permission question. It moves the `❯` highlight with arrow
keys, and writes Enter only after newer output shows the highlight on the chosen row. A typed answer
is written on the "Type something" row and Enter follows only once the screen shows that text; an
answer starting with a digit is refused, since that row shows no field of its own and digits pick
choices.
"Chat about this" chooses that row and puts the card's text in the composer: once Enter went, the
text is kept (the composer's notice says so) until the composer is empty and nothing is being sent,
whether the call resolves, the prompt closes or the step stops; it never replaces a draft. Only a
question or permission card has a field: when none shows, the card's field gives the keyboard back. Every step has a two
second deadline; a mismatch, timeout or a question still open eight seconds after Enter stops, says
why, and opens the drawer. While the card can answer, sends are refused with "Answer Claude's question
above" and the drawer does not open by itself; the working line reads "Waiting for your answer".

**Previews** (2026-11-28, `question/preview.rs`). An option of a question may carry a `preview`
(monospace text: a mock-up, code, a layout); the terminal shows the highlighted option's preview in
a box right of the choices. Chat reads previews only from the call's input in the transcript
(`QuestionOption::preview`: plain text, escape sequences and control characters other than tab and
line break dropped, cut at 8,000 characters or 200 lines with a line saying so), never from the
screen. The card built from the call shows the preview of the option under the pointer, else the
one ↑ and ↓ moved to (whether or not the field has the keys; they write nothing), else the first
option that has one; an option without one says so. It is drawn with the command output's grid
renderer (`view/ui/grid.rs`): the terminal's font, cell grid and theme colours at full strength on
the terminal's background, the text smaller to fit the widest preview line down to 9 px, past that
wrapped. Beside the choices when the card is at least 720 px wide inside (56 % of it), under them
otherwise (the narrowest window). It is as tall as the tallest of the question's previews, at most
16 rows, so the card does not jump as the pointer moves; a short window cuts it to 10, then 5 rows
before the question is cut, and the wheel over it scrolls a longer one. A drag selects (the
selection scrolls on past its edges); Ctrl+C copies the selection, Copy the selection or the whole
preview, as plain text (a wrapped line as one line). The pane only looks. When Chat reads a
question from the screen (the menu card, and every check before a key), a box beside the choices
is left out first (`screens::side_box`, `question_view`): a top-left corner and rule at column 12
or further right, its side down to its bottom corner on every row, text left of it, and a menu
footer under it; the cells right of its column on its rows are blanked in a copy of the screen
(cursor and key modes kept), so its rows no longer run into the choices and the question. Not
recorded yet: the layout was described, not seen (fixture `made_preview.txt` is made up), and
Claude Code 2.1.332 wrote a question's call to the transcript only once answered, in which case the
card built from the call, and so the preview, does not show while the question waits.
`recorder compat` scripts `preview` and `preview-form` record both (`side`, `asked`).

**Forms.** Several questions, or a multi-select one, are a form (decided 2026-11-27: it was the main
reason to open the terminal). The card shows one question at a time under a row of tabs, as the
terminal's form has them (2026-11-28): each question's header (or "Question n" without one), then
Submit. A tab turns the card's page; an answered question's tab has a tick, the shown one is
filled, and the one the terminal's form shows now has a dot (the same reading as before every key:
`form_question` for each question, the review for Submit; no dot when the screen shows none of
them). A click picks a choice (single choice) or ticks a box (multi-select), and the field holds
that question's "Type something" text; Back and Next turn the page, and Enter in the field turns
to the next question. After the last question comes the Submit step, in the card: "Review your
answers", each question with the answer the card holds (labels in option order, then the text, as
the terminal's review writes them; "Not answered yet" in the warning colour), Back and **Submit
answers** (Enter there submits too). Nothing reaches the terminal before Submit, and nothing is
picked for the user. Submit first checks the card: every question answered, a single choice by one choice
or one text, texts short, without control characters and not starting with a digit; otherwise it
shows the question and says why. Then Chat drives the CLI's form one key at a time, each written only
while the screen shows exactly what the key before should have led to, with a two second deadline
per key. A form question is read only as its own screen: right under the rule, the tab row ("←  ☐
Size  ☐ Speed  ✔ Submit  →") naming the form's headers in order, then that question's text and
nothing else, then every option in order (as `n. [✔] label` or `n. [ ] label` for a multi-select),
"Type something", for a multi-select an unnumbered Submit row ("Next" while another question
follows), a rule, "Chat about this" and a menu footer. Two questions worded alike stay
terminal-only. For each question in order: a multi-select ticks or clears each box that differs
from the card (arrows to the row, Enter, and the next key only once that box alone changed), types
the text on its "Type something" row (only while that row holds no text; Claude Code ticks it by
itself) and checks that it shows `[✔] text`, then moves to Submit or Next and presses Enter; a single
choice moves to its choice (or types on "Type something") and presses Enter. Each Enter must lead to
the next question's own screen. After the last question the CLI shows a review with no footer:
"Review your answers" under the tab row, a "● question" and "→ answer" pair per question (a
multi-select's answer is its labels in option order, then the text, joined by ", "), then "1. Submit
answers" and "2. Cancel" as the last rows drawn. Enter goes on "Submit answers" only once every pair
reads the question and the answer the card holds; any difference, a box that did not change, text
that did not appear, the wrong question, or text the user had already typed in the terminal stops
the run before the next key, says why and leaves the rest to the terminal. "Chat about this" and
Esc act on the form question the screen shows. Recorded from Claude Code 2.1.332 (the `ask_*`,
`form_*` and `multi_*` fixtures): a lone single-choice question has no tab row and no review; a lone
multi-select question has both. A call whose input cannot be read stays terminal-only.

**Permission prompts.** These used to be answered only in the terminal. The owner gave that rule up
(2026-11-27): "give that up, otherwise all I see is that chat has stopped. convenience trumps". The
safeguards that cost nothing stay. While the main transcript has exactly one pending tool call and
the screen shows a permission prompt (a "Do you want to …" or "Would you like to …" row, then
`1. Yes`/`1. Allow` down to a `No`/`Deny` choice, each choice wrapping onto at most four rows, then
the "Esc to cancel" footer), the card shows
the call as the transcript records it (command, file and diff lines, MCP server and tool), the
prompt's own rows above the question (warnings and countdowns included), the question and the
screen's choices, exactly as written. Clicks are answered only if the prompt names that call: the
whole command as whole rows of the prompt (whitespace and wrapping aside), the file's full path or
its path from the session folder, the MCP server and tool, or the fetched address or host, each as
a whole word, never a part of a longer word or path. A choice moves the highlight with arrow keys and Enter follows once the screen shows the
highlight there with the same choices; nothing is pre-selected and nothing is answered without a
click. A decline with text uses the prompt's Tab comment field when the footer offers "Tab to
amend" (Tab; the text only once the screen shows the field open on that row, the choice followed by
":", since on a menu row digits pick choices; then Enter once the text is visible), or its "No, and tell Claude …" choice,
whose text then goes to the composer. Shift+Tab is never sent at a prompt. The same deadlines and
fail-closed rules as questions apply. While the card can answer, sends are refused with "Answer
Claude's request above", the drawer stays closed and the working line reads "Waiting for your
approval".

**A prompt the pending call does not cover.** A subagent's prompt, the prompts of several calls made
in one message, and any prompt that does not name the pending call used to stay in the terminal.
The owner decided otherwise (2026-11-27): show such a prompt exactly as Claude Code draws it, so
that what is approved is what was seen. Recorded from Claude Code 2.1.332 (`perm_agent_*`,
`perm_two_*`): a subagent's prompt says whose call it is in its first row ("Bash command · from the
general-purpose agent", "Create file · from the general-purpose agent"), its footer adds "ctrl+x
ctrl+k twice to stop background agents" and drops "Tab to amend" while the highlight is on a
choice that takes no comment; the main transcript holds no pending call then, and may show the turn
as ended. Two calls of one message are asked one after the other, each prompt for one call with no
source in its header, and at the first prompt the transcript holds neither call. At the second
prompt Claude Code leaves its hidden cursor on the first call's output row, fifteen rows above the
prompt's footer (`perm_two_after`), so a prompt is known by its place and not by the cursor: its
"Esc to cancel" footer is the last text on the screen, which prompt text left in the conversation
never is (the input box and its status rows are under it). So the transcript
cannot say what such a prompt is for, and the card is built from the screen alone
(`question/prompt.rs`): every row from the prompt's own top rule down to its question (what is asked
for and by whom, the tip, the description, the command or the file's content between its dashed
rules, wrapped at the card's width), the question, the choices as buttons, Esc, and Open terminal.
The top rule is the full-width solid rule the CLI draws over a prompt; a row of the prompt itself
cannot be one, since each starts after a column of margin. The card may take up to 85 % of the
conversation's height; a longer prompt scrolls (wheel, or the "n more" button), and its choices
take a click only once every row has been in view. A choice also needs the card to have shown that
prompt for half a second, so that a second click meant for one prompt cannot answer the next one,
which Claude Code draws in the same place at once. Every key (the arrows, then Enter once the
highlight is on the chosen row) is written only while the screen still shows exactly the same rows,
question and choices; a prompt that changed gets no Enter, the step stops after two seconds with
"The prompt changed in the terminal", and the card shows the new prompt, to be read again. Esc is
written under the same check, without reading to the end. The gate reports such a prompt as it does
the pending call's card: sends are refused with "Answer Claude's request above", the drawer stays
closed and the working line reads "Waiting for your approval", also after the main turn's end. Left
to the terminal: a prompt whose top rule is not on the screen (taller than the terminal: the card
shows the rows it can read and says so), a row that does not fit the card even wrapped, and a
decline with text (how the comment field draws was never recorded, and the transcript cannot
confirm it for a call it does not hold). The pending call's own card stays as it was where it
applies, and the plan approval is no such prompt (its menu has no "No" choice). Two identical
prompts in a row cannot be told apart: the first answer then stops after eight seconds with "the
prompt is still open", and the card answers the second on the next click.

**Denied permissions.** A tool call Claude Code refuses looked like any failed call, and the reason
was in the terminal. The owner asked for it to be said in Chat (2026-11-27): what was denied, by
whom, and a way to approve it. Recorded from Claude Code 2.1.332: the `user` record with the call's
`tool_result` carries `toolDenialKind` at its top level, `"user-rejected"` for the user's own No or
Esc (with `toolDenialBeforeRun: true`), `"permission-rule"` for a deny rule (with
`permissionDecision.ruleSource`, seen: `"flagSettings"`) and `"automode-blocked"` for auto mode's
classifier (`permissionDecision.reasonType: "classifier"`); an allowed call in auto mode has
`permissionDecision.decision: "accept"` and no denial kind. The reader keeps the kind, the
before-run flag, the rule's source and the reason on the tool item (`transcript/denial.rs`: for auto
mode the classifier's words after "Reason: ", else the result's first line); a kind it does not
know is kept as "denied", and a failure without `toolDenialKind` is never taken for a denial,
whatever its words. In the conversation such a row reads "✗ blocked by auto mode", "✗ denied by a
rule" or "✗ you said no" instead of "✗ failed", says why when opened, and a folded group counts
"2 denied · 1 failed" apart.
A card under the conversation lists the current turn's denials (`question/denials.rs`): "✗ 2
permissions denied since your last message", each call with its tool and target and one line on
who refused it and what that means (the user's own No needs no approval; a rule names where it
comes from and stays in force until it changes; auto mode's line is the classifier's reason).
"Current" is the turn of the newest prompt in the main conversation, without the calls that a
later call of the same tool and target ran successfully (approved, retried, or allowed the second
time); the next prompt starts over, and a subagent's own denials are not counted (they are in its
transcript). A denial that happens while the tab is open opens the card; ones already there when
the tab opens show as a one-line strip with the count and Show; Hide folds it back to that strip.
It takes the card's place only when no question, prompt, menu or background card wants it, and it
never blocks a send. Its buttons do the least: **Review blocked calls…** (or **Open /permissions**
for a rule) runs `/permissions` as any slash command, under that command's checks; **Ask Claude to
retry** puts one sentence naming the calls that were not auto mode's in the message box when it is
empty (else once it is) and sends nothing.
What Claude Code offers for a call auto mode blocked, recorded (`blocked_*`): `/permissions` opens
on its "Recently blocked" tab while there are such calls ("Commands auto mode recently blocked.
Approving adds no rule."), newest first as "n. ✘ what  why", with the highlighted call's command
between dashed rules under the list. Enter marks the highlighted call `✔` and Enter again unmarks
it; `r` marks it `✔ … (retry)` and `r` again makes that an approval; a row under the list then
says "Esc will approve 1 blocked call" and the footer's "Esc to close" becomes "Esc to approve 1
blocked" (or "retry"). Esc applies the marks and closes: "Told Claude it may retry <command>. Auto
mode checks it again; this adds no rule." (a `permission_retry` system record, now shown as a
notice), and for a retry the turn starts at once; for an approval nothing runs until the next
message. The classifier does check again: the retried call was blocked a second time in the
recording. A user's No and a rule's denial are not in that list.
Chat drives that list as a card (`question/blocked.rs`), never as a menu of plain choices (there a
click on a row would have approved it): the tab's own note, the calls with their marks, the
highlighted call's command, what Esc will do, and buttons for exactly the recorded changes
(Approve and Retry now on an unmarked call, Unmark on an approved one, Approve only on one marked
to retry), plus the Esc button under the footer's own words ("Close", "Approve 1 blocked"). A
click on another call only moves the highlight (arrow keys, no Enter), so that its command shows
before anything can be marked. A key that marks, and the Esc that applies marks, is written only
for the list the card painted, row for row with its highlight and command, at least half a second
ago, and while the screen still shows that list; Esc that only closes needs no wait. A command
longer than eight rows or wider than the card is shown cut, with no button to mark that call. A
list Chat cannot read (a call's row that wraps, a mark it does not know) shows as rows only, with
Close while Esc just closes and Open terminal otherwise. The dialog's other tabs (Allow, Ask,
Deny, Auto mode, Workspace) are read-only panels in Chat: changing a rule is the terminal's. While another card has the card's place (a question, a permission prompt, a menu), the
count stays in view on a line of that card over what it asks, in the warning colour; the list comes
back with the strip once that card is gone.

A card answers ahead of a queued draft: the queue waits for the turn to end, which waits for the
answer, and the queued send keeps its own checks. Only an answer whose text goes to the composer
("Chat about this", "No, and tell Claude") is refused while a draft is queued, saying to cancel it
first. While a message is still on its way to the terminal, a card click says so and writes
nothing; the notice goes once those keys have gone (a queued draft is not such keys). In turn, the
queue is not released while a card's answer, a mode change or Ctrl+B is still in flight: the draft
stays queued until it has ended.

**Plan approval.** A pending `ExitPlanMode` call shows its plan as Markdown in the conversation, from
the call's input (cut at 20,000 characters), always open. The card under it offers the plan menu's
own choices, and only while the screen shows that menu: the row right above the choices reads
"Would you like to proceed?", one choice contains "keep planning", and the menu footer is the last
row. The same highlight, Enter and deadline rules apply. "Keep planning" with text uses the Tab field
when the footer offers it, else the text goes to the composer for the next message.

**Permission mode.** The toolbar's Mode chip names the mode from the indicator in the four rows under
the input row ("accept edits on", "plan mode on", "auto mode on", "don't ask on", "bypass
permissions on", "manual mode on"); no indicator under a clean prompt reads as Default, anything
else as "—". No transcript field is used. A click writes one Shift+Tab (`CSI Z`, the CLI's own
cycle) only under the same checks as a direct slash command (verified, quiet, visibly empty prompt,
no turn or pending call, no menu); the next click is taken only after the indicator changed, and a
mode that does not change or cannot be read within two seconds stops with a notice. The chip shows
only where the toolbar row has room.

**Toolbar chips never clip.** Every chip is at least as wide as its label. Effort and Context take
their long forms ("medium (default)", "… after compact") where the row has room, else their short
forms, which are measured too. The layout reserves the widest short forms ("Effort medium ▾",
"Context 999.9k") and puts the two chip groups on two lines when one line cannot hold them with
Attach, Terminal, Details and Send. A test lays out every effort level, mode and Context form from
the 760 px minimum window up at several display scales.

**Settings › Chat.** Quick commands: the toolbar's left chips (Model, Effort, Context, Permission
mode, Compact, Clear, "/ …") show in the order of seven slots, each a dropdown of the chips plus
None; a chip picked in another slot swaps with it, None hides it. Saved as `chat.chips` (keys in
order; a key from a newer version is kept and not shown). The one-line toolbar reserves only the
shown chips, so fewer chips keep one line at narrower widths. Three own commands (`chat.commands`,
by slot) are text rows, saved as typed whenever the text is a command line (a CLI-style name, a
"/" added if left out, one printable line of arguments, at most 200 bytes). Each set one is a chip
after the others, labelled with its line (or only "/name" when the line is wider than 150 px), drawn
only when it fits whole before the right group; a click runs it as a direct slash command, with the
same checks and the Compact/Clear confirmation. Typing "/" still opens the list with every chip
hidden. "Compact chip runs" (`chat.compactChip`): "Switch handover" (`"handover"`, the default)
labels the chip "Handover" and a click starts the handover exactly as "Compact and open…" in the
menus and the palette does (the dialog for the shown tab, with its identity and one-at-a-time
checks, continuing in place where it can); "Claude Code /compact" (`"cli"`) labels it "/compact"
and sends the CLI's `/compact` as before, with its confirmation. Both labels fit the chip's one
width, so the choice never moves the toolbar. Restore defaults resets chips and commands, not this
choice or the switch below. "Answer menus from the
chat" (`chat.answerMenus`, on by default) off: no card for the CLI's menus, pickers, plan approval
or permission prompts; the gate leaves them to the terminal and opens the drawer as before those
cards. Claude's questions (`AskUserQuestion`) and the Ctrl+B background offer keep their cards.
A two-window save merges `chat` field by field and keeps keys it does not know.

**Esc.** A card's Esc button writes one Escape only under the same checks as its choices: the screen
still shows that question, the pending call's own prompt, or the same menu (a menu Chat cannot read
is matched by its rows) whose footer offers Esc. It then waits as a choice does; a menu step ends as
soon as the menu is gone. Stop keeps its own rule: Escape at most once per freshly verified turn.

**Background controls.** While the only pending call is a shell command (Bash or PowerShell), no
prompt or menu shows, and the CLI's "(ctrl+b to run in background)" row closes that call's own block
(its `Bash(command)` row above it, the input box a few rows below), a card offers Run in background.
A click re-checks all of that and writes one Ctrl+B; it is done once the call ended or newer output
shows the call's row followed by "Running in the background", and stops with a notice after two
seconds otherwise. Stop agent goes through the CLI's agent list, the rows it draws under the input
box while background agents exist (`● main`, then `◯ type  description  status`). The card offers
it for an agent the transcript knows (its type and description, not known to have finished) that
the list shows once, running (its status starts with how long it has run), while the input is
clean and empty and its hint row offers "↓ to manage". The keys, each only once newer output shows
what the one before led to: ↓ (the list takes the keys, "↑/↓ to select", highlight on main), ↓ one
row at a time to the agent (the hint then reads "enter to view · x to stop"), `x` (the row reads
"stopped", the hint "x to clear"), Esc (the input has the keys again). Esc there leaves a running
turn alone. Left is never sent: at the input it opens "Background this session?". After a stop
the hint row reads "↓ to resume subagent" and other agents are no longer offered until it offers
"↓ to manage" again. Recorded from Claude Code 2.1.332 (`bg_*`, `agents_*`).

**Rewind.** The conversation's right-click menu offers "Rewind to here…" over a prompt the user
typed, in a tab with Chat, never over a subagent's transcript (refused there with a word). It goes through the CLI's own rewind list and never edits the transcript.
The first Esc only at an idle, quiet, empty prompt with no turn, pending call or menu (the checks of
a direct slash command); it shows nothing, so the second follows 150 ms later only while the screen
still shows that same prompt. The list ("Rewind", "Restore the code and/or conversation to the point
before…", each prompt over a row naming its code changes, "(current)", "Enter to continue · Esc to
cancel") must open on "(current)"; then one Up at a time, each only once the highlight moved one
entry up (or the list scrolled by one), until the highlighted row reads the chosen prompt: its first
line, whole, or cut with "…" (a prompt of several lines always is). A later prompt the same row could
read stops it before Enter. Enter must lead to the prompt's choices, under "Restore to the point
before you sent this message:" with the prompt quoted; the card shows that screen's rows and its
numbered choices (for example "Restore code and conversation", "Restore conversation", "Restore
code", "Summarize from here", "Summarize up to here", "Never mind") and chooses nothing by itself. A
click moves the highlight one row at a time and Enter goes on the choice only with the same choices
on screen; it is done when neither rewind screen shows and the input is back (Claude Code then puts
the prompt back on its input after a conversation restore). Cancel writes Esc (back to the list),
then Esc (back to the prompt). Each step has a two second deadline (eight for the last Enter); any
mismatch stops, says why in the card (Open terminal, Close) and opens the drawer. A rewind that
stops with its list on screen closes it with one Esc, checked on the next screen, and says so (not
after Cancel, whose own Esc may still be on its way); when
its choices are on screen or no key can go, the card says the rewind screen is still open, keeps its
place over the panel card while that screen shows, and goes once it is gone. The rewind writes
nothing to the transcript, so the Chat view still shows the turns after the chosen prompt until
the next prompt is written; that prompt names an earlier message as its parent, and from then on
the reader shows the active branch only (`transcript/active.rs`): the chain from the newest record
back through `parentUuid` (through a compaction boundary by its `logicalParentUuid`). A record is
left out only when its own parents reach the chain through a typed prompt written before the
chain's next record there, and that next record is a typed prompt too (the prompt the rewind
replaced and the one typed after it, or two first prompts when the chain is whole back to its
start): a recap or notice parented to the previous turn and written after a new prompt becomes the
newest record without hiding that prompt. Records whose parents lead out of the loaded slice stay until history pages in,
and records without a `uuid`, sidechains, a subagent's transcript, and records hanging off the chain
that do not start with a prompt (tool results recorded against their call, progress, notices) are
never left out. The Tasks, Agents, Shells and Files pages still read every record. Recorded from
Claude Code 2.1.332 (`rewind_*`).

A Shift+Tab or Ctrl+B in flight is chat work like a send: the composer waits for it, a card click
says so instead of writing, and it ends (or fails) in a hidden tab too. A menu that stays open after
Enter (a toggle) is done as soon as newer output shows the chosen row changed; one that shows no
change at all by the deadline still stops with a notice.

Terminal input stops a card's key sequence in flight (an answer, a form, Ctrl+B, Stop agent, a rewind)
before its next key, as do Cancel and switching the tab to the terminal. Keys, pastes, mouse clicks
(with their drags and releases) and wheel notches sent as arrow keys count; a mouse move without a
button counts as no input at all, and a wheel report to a program that asked for the mouse leaves
card sequences going (each still checks the screen before its key) while still counting as input
for a send. Only an answer stopped before its Enter leaves its prompt to the terminal; an answer
whose Enter went ends quietly, a Ctrl+B or Stop agent leaves a notice that goes with its card, and a
rewind says so on its own card.

Every screen pattern lives in `view/chat/question/screens.rs` and `view/chat/submission/screen.rs`.
The fixtures next to them are recorded from the real CLI through a ConPTY (`*.txt`: rows and
cursor; `*.ansi`: also styles, size and the rows the terminal marks as wrapping; `plain_*`: started
without `TERM`). `cargo test probe_recordings -- --ignored --nocapture` with `CS_PROBE_DIR` set to a
folder of such recordings prints what the gates make of each. Recorded on 2026-11-27 for the
readiness rules: the idle input after a turn, at the start, with the next-prompt suggestion, with a
background shell, after Esc, after a declined prompt, under one- and two-row status lines, at 40 to
200 columns and 10 to 50 rows, after `/clear` and `/compact`; a running tool, agent and compaction
(the screen changes at least every 229 ms while a turn runs); typed, pasted (`[Pasted text #1 +13
lines]`), wrapped and queued input; questions, forms, permission prompts (the main session's, a
subagent's with the highlight on each kind of choice and after Esc, and the two prompts of two
calls made in one message), a call blocked by auto mode and the /permissions list of blocked calls
in each of its states, plan approval, the
rewind list and the panels of `/model`, `/effort`, `/config` (which takes two Esc to close),
`/status`, `/help`, `/mcp`, `/permissions`, `/resume` and `/theme`. A pasted picture's marker
is recorded only by `recorder compat` (script `queued`, which puts a picture on the clipboard): the
text pasted after it follows without a space.

The fixtures prove the readers against the version they were recorded from. For each new Claude
Code version `recorder compat` records these states again from the installed CLI and the readers
are asked what each screen is (`screens/compat.rs`, the expectations in
`tools/screen-recorder/expect`): by hand with `tools/cli-compat.ps1`, and daily in
`.github/workflows/cli-compat.yml`, which opens an issue naming the states that no longer read as
expected. `tools/screen-recorder/README.md` ("After a Claude Code update") says how to read a
failure and what to do. Its first run found that the second of two prompts was not recognised,
which the fixtures had not shown. It also checks what Chat reads from Claude Code's files and
flags: messages sent while a turn runs as the transcript records them, what Esc does with messages
waiting in Claude Code's queue and its own Send now for them, thinking shown as updates, and
branches cut at a turn's end resumed under their new ids in print mode and in the terminal
(scripts `queued`, `queued-text`, `queued-esc`, `queued-now` and `branch`), a new handover session
reading its file without a prompt and a command left typed on the input taken off with Ctrl+U
(script `handover`), and a handover-sized paste shown on the input in time (script `long-paste`).

**What still needs the terminal.** Each for a reason a key could not be justified: text the user
typed on the CLI's input (Chat will not delete it; the composer says so); a permission prompt Chat
cannot show whole (its top is above the terminal's screen, or a row does not fit the card), and a
decline with text for a prompt the pending call does not cover; changing a permission rule (the
/permissions tabs are read-only in Chat), and a blocked call whose row or command Chat cannot show
whole; a dialog with no footer Chat knows (after four silent seconds the terminal card shows it
and **Answer in terminal** opens the drawer); Claude Code's full-screen renderer or another full-screen program; a session
whose identity could not be verified; a card step whose screen did not follow within its deadline
(the card says which, and Open terminal).

**Renderer.** Claude tabs start with `TERM=xterm-256color` (a launch's own `TERM` stays). Windows
sets no `TERM` for a program started from the Start menu, and Claude Code 2.1.332 then takes the
terminal for one without Unicode: `>` for `❯`, `[ ]`/`[×]`/`√` for `☐`/`☒`/`✔`. With `TERM` set,
and nothing else, it draws as recorded. The readers take the plain drawing all the same (a tab
kept from an older version, a launch with another `TERM`): the input mark `>` counts only right
under the input box's rule, a ticked box is `[√]` or `[✔]`. 0.1.2 knew only `❯`, which is why the
installed app, unlike one started from a shell, refused every send.
Claude tabs start with `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1` while Settings ›
Sessions › Classic terminal view is on (the default): Claude Code then draws on the main screen,
where the prompt checks above apply. The variable is set for that process only, never in Claude
Code's settings. The alternate-screen veto stays for every tab; in a tab started without the
variable it names the setting and `/tui default`. Recognizing the fullscreen renderer's own input box
needs a capture from a real session and is not attempted.

**Local data.** File chips use the existing path quoting. Ctrl+V prefers clipboard text. Use
Ctrl+Shift+V or **Attach… → Paste picture** to choose the picture explicitly. Supported picture
formats get an in-memory thumbnail. A send checks the clipboard sequence, writes Alt+V alone,
and waits up to twenty seconds for a newly displayed `[Image #n]` on the CLI input row (Claude Code
reads the clipboard through a helper process on Windows; the wait ends at once on any other
change). A picture sent while the input is not visibly empty is held like any draft. Only then does
Chat paste the text and write one Enter. Timeout or an intervening input, permission, clipboard or
session change retains the draft. Failed picture or command staging blocks new sends until newer
terminal output shows a quiet, clean prompt. If paste succeeded but Enter did not, **Press Enter**
rechecks the staged input and retries only the Enter key. The composer keeps line breaks (the
bracketed paste carries them as CR, as a terminal pastes), replaces tabs with spaces, refuses other
control characters, and writes the text as bracketed paste followed by one app-generated Enter. A
message of several lines gets its Enter only once newer output shows it on the input line: the rows
from the `❯` row to the cursor equal the text (whitespace and wrapping aside), or the input is the
CLI's `[Pasted text #n +m lines]` marker (after a picture's marker when there is one). Five seconds
without that keep the draft, press no Enter and ask to check the terminal. A slash command's line
may wrap onto more rows; one refused only because the prompt is not quiet yet waits up to three
seconds for it and then runs by itself. Ordinary
terminal paste refuses multiline clipboard text when bracketed paste is off. Composer Up on the
first displayed line and Down on the last walk real prompts in the main transcript, including
terminal input. Recall skips adjacent repeats, pages back through the transcript worker as needed,
and restores the original editor on Escape or Down past the newest. Editing or attempting to send
ends recall without restoring the old draft.
While the slash picker is open, it keeps the arrow keys. Expanded tool rows show bounded previews:
Edit compares the unmatched middle after common leading and trailing lines, Write shows
additions, and shell output uses only its first text lines. In Chat's Details panel, Task and Agent
rows expand to show full task text or reported agent details. The task list comes from TodoWrite
or the desktop plan tool, or from native TaskCreate/TaskUpdate calls whose results confirmed them
(the same reducer as the handover's; a failed call changes nothing); whichever changed last is
shown, and history paged in later is replayed in transcript order. **Changed files** lists successful
Edit/Write tool paths in the displayed conversation's loaded history, not a filesystem or Git diff.
The disclosure arrow expands a file's edits newest first, even if the file is unavailable. An edit
jumps to its exact tool row and expands its group and detail, pausing follow. Clicking the file name
separately opens the embedded Markdown viewer or asks the system for the default app for a supported
document or media type. It uses only an existing, verified transcript-named file and rechecks at
activation. When a turn is active, both conversation layouts show the CLI’s working line from
visible screen rows, not scrollback. A missing or unreadable timer falls back to time measured
since the app observed the turn. The spinner glyph is drawn centred in a slot as wide as the
widest glyph, the text after it starts at a fixed x, and each digit takes the widest digit's
width, so neither the animation nor a ticking counter moves the line. Stop uses the same
fresh-turn safety checks as the composer and
is inactive when those checks fail.

**Shells** uses only background `Bash` inputs whose successful paired result names a task ID and
output path. Later task notifications or matching successful status/stop tools determine the
finished state and exit code; a missing terminal record does not prove a shell stopped. The
visible expanded rows read at most the final 16 KiB through the reader worker and the fixed-volume,
no-reparse open, showing at most eight lines. Only a result's exact `<task-id>.output` under
Claude Code’s temp project (or that project’s current session) `tasks` directory is eligible.
Following is on by
default; pausing retains the displayed lines. **Full output** opens a literal UTF-8 snapshot from
the start (first 1 MiB). Shell logs are view-only: no external open action or active links. **Stop…**
binds confirmation to the tab, transcript generation and shell. Switch opens the terminal drawer
and can send a single Down key only at a verified, unblocked, empty CLI prompt with the adjacent
“↓ to manage” footer. Otherwise it explains the manual path; it never selects a menu entry or
stops a process.

**MCP tasks** follow the shells in the same tab. An MCP tool call that Claude Code moved to the
background is known only from what the transcript holds: the call's result naming the task
("MCP tool "server/tool" started server-side task JOB. It was moved to the background as task
ID"), a `<task-notification>` record when it ends (`taskKind: mcp-task`, its status, a summary and
the result) and a TaskStop result with `task_type: "mcp_task"`. A row shows the task id, status and
running time (from the call to the end record). The last step the server reports is kept by the
CLI in memory only, so it is said to be unavailable rather than guessed; Claude Code writes no
output file for these tasks. A task's name opens its result (at most 64 KiB, as the notification
held it) in the viewer, or jumps to its call while it runs; Switch does not stop MCP tasks (the
CLI's `/tasks` does). Nothing is fetched or asked of a tool.

Every Details row opens its work from its name, the arrow still expanding it: a task jumps to the
newest plan update, an agent opens its sidechain transcript (not the one being shown), a shell
with a log opens its full output, a file opens as above and an output opens as below.

**Outputs** lists the newest occurrence of each HTTP(S) destination or verified saved path, with
tool-result pictures identified by their transcript references. It sorts by reported timestamps
when available. It reads assistant prose and bounded tool-result text for links, with no fetching;
opening requires a click and uses the default browser. Outputs and conversation links share the
same host checks and reject userinfo. Saved files use the same path authorization
as conversation links; a tool result mentioning an arbitrary filename does not grant access. The
shared viewer handles bounded Markdown, UTF-8 text and supported images. Cards are deduplicated per
turn and prefer the answer over a tool result that repeated the same link. Every chip and Outputs
row says what it is with an icon and a plain kind: PDF, document, spreadsheet, presentation,
Markdown, text file, audio, video, archive or file by extension; link, artifact or pull request
for web destinations, with the host or `owner/repo #N`, and the folder for a file. Sizes, page
counts and document previews are not read. The transcript reader knows no attachment blocks other
than pictures, so nothing else is shown as an attachment. Both Chat and Session
view share the Shells and Outputs Details rendering.

Context now uses
reported input and cache tokens; no limit or percentage is guessed. The toolkit’s TextField
is single-line; the composer wraps the app’s multiline editor at words, displays hyphens only in
overlong words, and uses the toolkit context menu for Undo, Redo, Cut, Copy, Paste and Select all.
The composer has undo (Ctrl+Z) and redo (Ctrl+Y, Ctrl+Shift+Z) in `view/chat/undo.rs`, recorded
inside the composer's editor, so every way the draft changes is covered. Typing is one step per
word or after a one-second pause, a run of Backspace or Delete is one step, and a paste, a cut, a
completion, a recalled prompt (and leaving it) or a filled chip is a step of its own. A step brings
back the text with its caret and selection. At most 100 steps are kept; sending a message clears
them. Neither the toolkit's single-line `TextField` (search box, dialog fields, Settings) nor the
handover dialog's editor has undo.
Caret keys follow a Windows text box. Shift extends the selection. Ctrl+Left and Ctrl+Right go to
the start of the previous or next word, where a run of punctuation is a word of its own and a line
break a stop. Ctrl+Backspace and Ctrl+Delete remove a word, one undo step each. Home and End go to
the wrapped line's ends; with Ctrl, to the draft's ends. Ctrl+A selects all. Up and Down recall
earlier prompts only without modifiers, so Shift+Up and Shift+Down always select. The word rules
live in the editor the composer shares with the handover dialog (`compact_dialog/editor.rs`); the
toolkit's `TextField` has its own word movement.
The composer toolbar separates session values, actions and panels by their surfaces. Fixed-width
value controls keep their neighbours still as values change; effort is a five-dot scale and
context remains a token count, not a filled bar, because the transcript supplies no verified limit.
An unknown model name too long for the control is shown as Model rather than a cut-off name.
**Composer conveniences**, all local (`view/chat/popups.rs`). `@` at the caret opens a list of
the session folder's files and folders, read once per tab on a worker: breadth first, at most 8
deep and 5000 entries, without hidden folders, `node_modules`, `target` and other build folders,
and without following links or junctions. Choosing inserts `@path` (quoted when it holds a space).
Ctrl+R searches the prompts the transcript reader already holds, newest first, each once; it loads
no older history. The CLI's grey suggestion for the next prompt is read from the live, empty
prompt row (`screen::clean_prompt`) only as one unbroken faint run starting at the caret, and never
as the CLI's fixed hints (`Try "…"`, `<…>`, an ellipsis, "for shortcuts"). It shows as a chip in
the empty composer; the chip or Tab fills the draft. Nothing is sent: no Tab reaches the CLI.
The message box shows three lines when empty and grows with the draft's wrapped rows up to 40% of
the chat column, then scrolls. The terminal drawer is sized against the smallest box, so typing
never resizes the terminal; a window too short to keep 80 px of conversation gets the two-line box.
Demo shows the waiting state, and its composer writes only to the synthetic terminal screen.

## Text fields

Every single-line field (the session search, the name dialogs, the new-session folder, the quick
switcher and rename boxes, the handover's new session name) takes the mouse through the toolkit's
`textfield::Pointer`: a click places the caret, a drag or Shift+click selects, a double click takes
a word, a triple click everything, and a drag past either end scrolls the text. Each field records
its text rectangle as drawn (`FieldHit`) every frame; a press in the box's padding goes to the
nearest end. One drag is held at a time (`fieldmouse.rs`). A right click opens the toolkit's Cut,
Copy, Paste, Select all (`TextField::context_menu`), drawn above a dialog; outside the selection it
first moves the caret. Settings text rows, the hex box of a colour row included, use the
toolkit's own mouse editing (`Form::pointer_press`/`pointer_motion`/`pointer_release`, edge scrolling
through `text_scroll_deadline`) and its menu built from the real selection (`Form::context_menu`,
`apply_context_action`); the app only holds the drag and draws the menu. Secret rows refuse Cut
and Copy but take Paste; Settings has none today. An open dropdown or colour picker takes the pointer, so no
text row under it gets the I-beam or the menu (`pointer_cursor_at`). A field under the picture overlay
or the usage popup gets neither the menu nor the I-beam. The
I-beam shows only over a field that takes the mouse now, the composer, the handover editor and
terminal text.

## Session organization

Recent, Projects, Threads and (when at least one indexed session has a label) Topics are remembered in
`config.json`. The old Folders setting migrates to Projects → Group by: Folder. Projects resolves each
working folder once per refresh on the scan worker. It walks up to the first `.git` directory or file,
follows `gitdir:` and `commondir` using paths relative to the files that hold them, and uses the
main checkout's `HEAD` as the branch reference. It does not run Git. On Windows the starting path
must have an ordinary drive letter mapped directly to a `\Device\HarddiskVolumeN` device and
reported as `DRIVE_FIXED`. Git pointers must stay on that drive. Metadata and file reads use the
validated native volume path with `OBJ_DONT_REPARSE`; UNC, mapped drives, junctions, symbolic links
and other reparse points are not followed. A submodule with its own gitdir
and no commondir stays separate. Missing folders, malformed markers and common directories that do
not identify a checkout fall back to the original folder. Within a project the visible members are
newest first, and the header age uses the same draw-time clock and timestamp as the first row.
Groups containing pins are promoted in Projects, Threads and Topics without splitting
the groups. Recent and the Folder option still have a separate Pinned section.

New handovers retain the display-only `handoverFrom` and store the exact source UUID in
`parentSessionId` in `marks.json`. Open a copy preallocates a different UUID, persists its source
before launch and passes it with `--fork-session --session-id`; failed starts remove the parent mark.
An old handover with only a title is not evidence of lineage. The bounded transcript head accepts
optional `forkedFrom.sessionId` and `parentSessionId` UUIDs in the first 32 lines; these two names are
compatibility guesses, not fields documented by the previous reader or verified against live
transcripts. Conflicting transcript candidates, unknown parents and cycles do not create links. The documented
Desktop `priorCliSessionIds` describes Desktop's aliases and still follows the existing scan policy
of keeping the current transcript. `parentUuid` is not a session link. Threads show the newest
visible member, with earlier members collapsed; text search reveals all matches.

Recent’s optional Sittings view merges visible non-pinned activity intervals within 30 minutes in
the same project and local activity day. Unknown starts use the last activity as a point; unknown
end times are never merged. A range that crosses midnight is clipped to its last activity day. The
cluster occupies its newest member’s position, and its members stay in newest-first order.
The scrollable details pane shows visible thread members first, then up to five other sessions
from the same project with nonzero Jaccard overlap of case-folded, stop-word-filtered first/last
prompt words. Each full scan builds one immutable prompt index off the window thread. Scoring
runs on a cancellable selection-scoped worker against that snapshot, then current visibility
and thread membership are applied before showing results. Related sends nothing to a model.

Under Related the details end with the selected session's **Conversation**: the shared conversation
view (`view::ui::Ui` over `view::model::Model`, as Chat and the Session view draw it) in its
embedded form, with no heading, page tabs, Attach or shell button, and read-only: no message
buttons, Stop, Rewind, Branch or right-click menu. It reads the transcript through the same
incremental reader, so the newest records first and earlier ones only on **Load earlier**, and
follows appends on the window's one-second feed timer while the Sessions page shows it. The region
is as tall as the pane less its heading (at least 280 px) and scrolls on its own: the wheel over it
scrolls the conversation, and once that reaches an end, the details around it. A click in it takes
Ctrl+C, Ctrl+A and Escape. Changing the selection drops the reader (its worker ends with it). The
demo reads a made-up conversation written to a temporary folder of its own that goes with it.

**Include topics when suggesting names** is off by default in Settings → Sessions. If enabled,
the existing confirmed naming request adds existing short topic labels and asks for an optional
topic beside each title. Its transcript excerpts and request count are unchanged. Review keeps
title and topic editable before acceptance. **Set topic…** edits or clears a local label without
running a model. Topics have at most three words and 60 characters; blank means no topic. The demo contains synthetic
projects, a worktree, a four-session thread, adjacent activity, Related prompts and topics.

**Search by meaning** (Settings → Sessions, off by default) adds a model-written summary to
literal search. **Model** is Claude Haiku through the CLI on the workers' account (the same
isolated `-p` run as Suggest names: an empty private folder, no tools, MCP, settings, slash
commands or session persistence) or an OpenCode model, run like the handover's summarizer (an
empty temporary folder, no project or Claude Code config, its own one-step agent that may use no
tool, `--standalone --archived`, title “Claude Switch search”). **List models** fills the OpenCode
choices on a worker; the list is shared with the handover dialog. The palette's **Summarize
sessions for search by meaning…** confirms first, naming the model, the count and what is sent:
the payload of a name suggestion (numeric ids, the first and up to three recent prompts, 500
characters each) for visible, not hidden sessions with no summary or activity since it, newest
first, at most 100 a run in requests of 20, plus the subjects already in use among those
visible, not hidden sessions, so groups stay stable. Each answer line gives a summary of at most 200 characters, up to eight keywords and a
subject (a topic-shaped label); invalid, repeated and out-of-range ids are dropped. They are kept
in `meaning.json` beside `config.json` with an FNV-1a fingerprint of the exact payload: a session
whose prompts did not change is not sent again, and the file is written after every request, so a
cancelled or failed run keeps what was answered. A request whose answer gives nothing readable
(prose, a refusal, cut-off output) fails the run and marks none of its sessions. A session with no
readable prompt, or one the model left out while answering others, is kept as a marker
(fingerprint, activity and model, no summary): it is not sent or counted again until its prompts
change or, for a left-out one, another model is chosen (an earlier real summary of it stays), and markers are
never searched, asked over, counted as kept summaries or used for subjects. A missing or damaged file is an empty cache;
**Forget** removes it. While the setting is on, the list's search also matches a session when
every meaningful word of the search (one may miss from three or more; stop words and simple
suffixes ignored) begins a word of its summary, keywords or subject, locally; Details shows the
summary as **About**; Topics appears once any summary has a subject and its **By meaning** button
groups by subject across projects (sessions without one under “Not summarized”). **Ask about past
sessions…** takes a question and confirms before sending it with the kept summaries, keywords and
subjects (no prompts, folders or dates) of up to 200 visible sessions; the model answers with at
most 1,200 characters and up to ten ids, which open from the answer. The answer is kept by
question (case and spacing ignored), model and the fingerprint of the summaries it went over, so
the same question again is answered from the file without a request. All requests and transcript
reads run on one worker job; the dialog's Cancel kills the child process. The demo answers
locally with synthetic text and sends nothing. Each run is confirmed instead of recording a
provider consent, so the handover's consents are neither used nor changed.

## Terminal cursor and backdrop

Settings → Terminal saves `appearance.cursorShape`, `appearance.cursorBlink`, and
`appearance.terminalOpacity` in `config.json`. Every tab uses the latest preferences when drawn,
including the Chat drawer. The default cursor is a steady Block; Bar and Underline are fallbacks
when the program has not chosen a DECSCUSR style. Blink is off by default. When enabled it still
respects an explicitly steady cursor, the system caret interval, focus, visibility and reduced motion.
It starts again, on, after each key and after output that moves the cursor (by absolute line, so
scrolling counts) or changes its look, its screen or the cell under it. Output that repaints
elsewhere and puts the cursor back, as Claude Code's elapsed-time counter does every second while it
works, leaves the rhythm alone.

Opacity defaults to 100% and steps down to 60% by 5%. It affects normal backgrounds and padding,
not text, explicit terminal backgrounds, or selected cells. The toolkit can raise opacity for text
contrast. On a translucent frame the grid requests grayscale glyph masks even for cells the
toolkit promotes to opaque. Edges use source-over on premultiplied BGRA, not a second blend
against black. On an opaque frame the font returns to the system's antialiasing. The pinned
toolkit still raises opacity per ink colour, so dim text can have darker cell backgrounds.
Without an active Mica or Acrylic backdrop (including high contrast or loss of window
focus), the setting is disabled and the surface draws opaque. The scrollback thumb has a solid
gutter and the end notice is drawn on solid rows. The Chat drawer header is solid.

The terminal uses the same opacity and glyph masks whether an overlay is open or not. Over a
translucent terminal, menus and dialogs use solid, square-edged cards without exterior shadows.
Widgets with built-in shadows (such as toasts and tips) draw on a solid backing clipped to the
card; entrance animations fade the finished card over the grid. Only modal dialogs dim the
window: fading a black fill over the saved pixels blends premultiplied colour **and** alpha,
including where marked chrome meets the grid. No separate compositor surface is needed.

`--demo --demo-terminal bar,80,mica` previews this with a synthetic Chat drawer; a debug build
can add `--shell cmd.exe` to preview it with an isolated command prompt instead.

## Tab identity and interaction checks

The running-session poll retries unattached live tabs even when PID records are unchanged.
An attached pane keeps its draft. When the live record of the tab's own process names a
different session (the CLI's `/clear`, compaction or `/resume`), the pane follows it: it reads that
transcript (any age, same encoded folder, recorded folder must match) or awaits it as for a new
chat, cancels anything queued or staged, and requires a prompt written after the change before an
ordinary Send. A followed session the CLI had not written yet is recorded as the old one's
continuation (`parentSessionId` in marks, as for handovers); an existing transcript is not. A change
Switch cannot explain (the new transcript records another folder, or the record moves before a
new chat's first message created its file) clears the current identity used by Open, local marks, and
background transcript notices without moving that draft to the other session. A missing
PID record retains the existing duplicate-resume guard, not permission to send or mark seen.
`/fork` in Claude Code 2.1.332 starts a background session (its own process and live record,
`kind: bg`, its transcript a copy of the conversation that names the tab's session in
`derived-from` with `via: background`) and, as recorded by `recorder compat`'s `fork` script, leaves
the terminal in its session: the live record keeps its id, the tab's transcript gets the command's
records and Claude Code's note about the fork (`isMeta`, no prompt), and the next turn goes on in
it. The background session's process is not the tab's, so it is never taken for the tab. In
transcripts from 2.1.325 and once from 2.1.332 (2026-11-27), the session moved right after a
`/fork` instead: the transcript the fork left ends with a `continued-in` record naming a new id,
whose transcript (the active chain copied, then the new turns) is written only with the next
message. The shown pane follows that record as it follows a new id in the live record (awaiting
the transcript, linked as the old one's continuation), unless the live record names a third
session, and from then on a live record still naming the session left stands for the one it went
on in.
Real Chat panes start with both Send paths blocked until verification. A verified tab missing from
the Sessions index requests a refresh when asked to open a handover and explains how to retry.

Direct terminal input samples only file length at the cached verified transcript path; it
does not rerun transcript discovery. An unchanged tray tooltip does not call Explorer again
on terminal wakes. Explorer restart and a close-to-tray request bypass that cached decision.
Session-view Reveal uses the same sanitized Explorer launcher as Sessions.

A permission dialog must have its question, choices, and cancel footer together near the
terminal input cursor. Either send keeps the draft when this pattern is recognized. The
pattern is an English CLI screen heuristic, not confirmation that no interaction is pending.
`--demo --demo-permission` displays a synthetic dialog with a draft.

Main-user picture blocks count as a new turn for Waiting and background notices, without
contributing naming text. Failed saves of local names, pins, or visibility restore the
previous marks; unsuccessful hides do not change selection or offer Undo.

## Account and usage recovery checks

Adding a secondary account claims its directory exclusively; a collision advances the slug suffix.
Before its first CLI sign-in, Switch seeds a missing MCP list and creates Main's projects directory
and the shared junction. An independent projects tree is never replaced or removed. An existing
transcript can be resumed or forked only when the chosen CLI can reach that same file. A new CLI
launch or sign-in refuses an independent tree rather than silently splitting transcript history.

The Workers bridge is synchronized from committed settings, including after reset. Switch serializes
its updates to the bridge file and preserves unrelated keys. If sync fails, use **Settings → OpenCode
→ Sync bridge account settings** to retry. An unrecognized active-session response gives an unknown
count. If a confirmed restart is in progress, normal exit waits for its completion, including the
stop/start fallback and verification.

Usage retries local settings-lock contention within the request deadline instead of applying network
backoff. The credential reader decodes only the selected token and expiry values; other values remain
borrows of the wiped input buffer. Expiry and the deadline are checked again under the final submission
permit. Windows check the committed usage preference before rendering or accepting results; opt-out
discards unsubmitted usage balloons without dropping waiting-tab alerts. Undelivered warning claims
are retried, and expired claims are pruned under the ledger lock. Hovering leaves keyboard input with
the current control or terminal. `--demo --demo-usage-hover` echoes terminal keys into a process-free
screen; F8 reopens the passive preview for testing without launching Claude.

A verified live CLI identity can establish the owner of a new or copied session. Missing evidence
means **Unknown**, not Main. Windows rejects a running record whose last write predates the live
process's creation. The writer resolves a redirected profile above the app directory but refuses
redirects at or below the app directory. Settings, marks and saved tabs have an 8 MiB read limit;
oversized settings must be moved aside manually rather than reset without an exact backup.
