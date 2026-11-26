# claude-switch: design

A small Windows desktop app that knows the owner's Claude accounts, lists every Claude Code session on
this computer with its working folder, and opens one with a click: in the Claude Code CLI or in the Claude
desktop app, as the chosen account. A session can be carried to the other app or the other account.

Built with the owner's toolkit (`markdown-shared-rs`: `mdshared-ui` with the `host` window loop,
`mdshared-app`, `mdshared-term`, `mdshared-pty`). Two crates:

- `crates/core` (`claude_switch_core`): everything that is not a window. Plain data and functions, tested
  against temporary folders. No toolkit dependency. `serde_json` is its one dependency.
- `bins/claude-switch`: the window.

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
session's folder (or home when none is selected) and the current account. The four most recent
distinct folders are shortcuts to fill the field. Start or Enter checks that the folder exists;
Cancel or Escape closes the dialog without starting anything.

## Workers

The owner's bridge (`claude-flash-bridge`) runs Claude Code workers with `claude -p`. A setting here says
which account they use; it is written to `%USERPROFILE%\.config\opencode-bridge\workers.json` as the
top-level keys `claudeAccounts` (the CLI folders) and `claudeWorkersUse` (`"same"`, `"other"`, or one of
the folders). The bridge reads them.

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
  selected) and Shift+Page Up/Down (scroll back). A middle click on a tab closes it.
- **Mouse**: the wheel scrolls the scrollback 3 lines a notch, is reported to a program that asked for
  the mouse, and is arrow keys on the alternate screen of one that did not. Drag selects (double click a
  word, triple click a line, Alt a block); with mouse reporting on, Shift+drag selects. The drag requests
  a redraw only when the selected cells change. Copying is by key only, not on release.
  The window chooses the mouse pointer from the current frame's hit map, including when a tab or
  overlay changes under a stationary pointer. Leaving the client resets the app pointer to the
  arrow; the host still supplies native resize cursors at window edges. Only the front layer
  receives hover while a menu, dialog or command palette is open.

**The safeguard, for tabs.** A session open in a tab counts as running at once, before any
`sessions/<pid>.json` says so: opening it again shows its running tab ("Already open in a tab"), and the list
shows it as running in this app. Closing a tab whose program runs asks first; closing the window with
running tabs asks once for all of them. Dropping a tab's pseudo-terminal ends its program.

**Not there:** more than one terminal in a tab, moving tabs by drag, search or links in the terminal,
font-family settings, tabs restored after a restart; Shift+Insert and the Insert key (the window host
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
stays unattached.

Conversation, Agents and Tasks are separate views of the incremental transcript reader. Conversation
opens on the newest messages and follows appends until the user scrolls back; Jump to latest resumes
following. Both Chat and Session view support wheel scrolling, thumb drag, and track clicks that page
by a viewport. Earlier history is loaded on request. Agent transcripts are reachable from expanded
tool details and the Agents page. A cursor checkpoint detects a replacement or truncate-and-regrow near the
read position. Transcript lines over 48 MiB are skipped, picture reads respect the same containing-line
limit, and sidecar metadata over 1 MiB is ignored. The reported agent state comes from the sidecar, so a missing
completion flag is shown as unknown rather than guessed.

Chat and Session view use the same conversation projection, painting and input. Prompts sit in
right-aligned tinted bubbles; cardless answers use a palette-accent Claude mark and body-size prose
(16 logical px in Chat, 14 in Session view). Indented, bordered mono tool rows show recorded status,
the tool and its target, and the time between use and result timestamps when both are valid and
ordered. Runs of four or more consecutive tools start collapsed. Expand a row for the bounded
details and any output pictures; a group can be expanded independently of its rows.

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

Only the visible pane requests polling: on terminal output, with a 500 ms fallback. Reader and
picture work run off the window thread; worker results wake the window rather than making it redraw
every poll. Image bytes are decoded with `mdshared-text`'s existing PNG/JPEG decoder and cached for
inline previews and an enlarged overlay. Opening a picture in the system viewer explicitly exports
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
merge in bounded stages. Model replies supply only six Markdown
fields; the application renders the headings and all local evidence independently. Invalid or
missing summary fields say `unknown`. A transcript over 2 GiB (it is read as a stream; only prompts and answers are kept), or a record over 48 MiB,
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

The preview is editable, with characters and approximate tokens (characters / 4). Previewing or
canceling does not save a handover. Only an explicit Save As or Open writes the edited text.
Local Markdown files live in `handover/<session-id>-<time>.md` beside config; the newest 20
are kept per session. An opened edited handover is kept as a new revision. Save As never
overwrites an existing file. Native Claude Code receives `claude --session-id <new-uuid> --name=<title> -- <text>`
in the original folder with the selected account's normal sanitized launch environment. The
interactive CLI documents no stdin/file argument for a first prompt, so short text is one native
argument (no shell); above 15,000 UTF-16 units a short prompt asks it to read the saved file. The app uses the
new UUID for `marks.json` provenance. It never resumes or modifies the source transcript.

The new session name is independent of the OpenCode run title (“Claude Switch handover”) and
of the Markdown, whose first heading stays `## Goal`. The suggested name is the source's
Switch display name (local custom name, then indexed title) plus ` · continued`, then the
lowest free suffix starting at ` 2`. Suggestions compare indexed display names and very recent
unindexed handover launches case-insensitively, ignoring whitespace runs. The preview can edit
the name; an untouched suggestion is recomputed from the current index before opening.
Claude Code's documented `--name` carries the title as one native option before `--`; Switch
uses it as the initial tab label and reads native custom-title records on scan. The two-minute
launch reservation in `marks.json` avoids reusing names before the transcript appears; it is
not a permanent Switch rename. A failed spawn rolls it back with the new UUID's provenance.

The editor's **Agent** picker is CLI-only. **Settings → Handovers → Default CLI agent** edits
`compact.defaultAgent` under the handover settings lock without changing provider consent or
the model. Blank is the default and means no switch-level override. **Default (Claude)** is the
first entry and always adds no flag. A configured name is preselected, including when the
catalog cannot find it; the catalog is a suggestion list, not proof of CLI availability.
Changing the target account resets to the configured default. A selected custom agent adds
`--agent <name>` before the first-prompt separator. Discovery runs read-only from that account's
`agents/` and the launch project's `.claude/agents/`; project definitions take precedence.
The reader keeps bounded frontmatter names and descriptions, not agent prompts. It also offers
agents in standard `agents/` directories of unambiguous, explicitly enabled version-2 plugin
cache installs. It does not guess at managed policy, custom manifest paths or registry formats.
Discovery does not start Claude. The documented Desktop launch path takes neither an agent
flag nor a first prompt, so the handover editor continues to open new sessions in the CLI.

The CLI runs headlessly for `--print` (the default) and writes the handover only to stdout unless
`--save` is supplied. Explicit `--open` keeps the handover and starts a new window with the
new tab because the app has no single-instance request channel. On Windows, the new window is
ShellExecuted through a fixed non-interactive PowerShell bootstrap so it cannot keep the caller's
output pipes open; only the executable path and validated identifiers reach the bootstrap.
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
`--open` waits up to 30 seconds for the new window to acknowledge creating the tab. On timeout
the outcome is unknown: check for the tab before retrying. Tab creation is not
confirmation that the CLI accepted the prompt or remained running. Exit codes are 0 for success,
1 for runtime errors and 2 for usage errors. `--demo` uses synthetic text and model IDs; it sends no
handover request and launches no Claude process.

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
(saved as `darkTheme` and `lightTheme` under `appearance`), each shown as a row of swatches beside
its choice; a click on a swatch chooses it. Dark: Neutral (the toolkit's dark), Near black, Warm,
Blue-grey and High contrast (the toolkit's); light: Paper, Warm, Cool (the toolkit's light, warm and
cool) and High contrast. Every palette has its own terminal colours, which Automatic terminal colours
follow; the table of palettes and their terminal colours is `termtheme/palettes.rs`, the one place
to change them. An unknown saved id (from a newer version) is kept and shown as the first palette.
Windows high contrast takes priority over all of them. The toolkit's appearance model supplies the
accent (Claude's clay by default on every palette, held to 7:1 on the high-contrast ones, or system
or picked), dialog and menu corners, optional soft shadows, and the system-aware motion policy.
Mica and Acrylic are optional on Windows 11 22H2 or later; the title bar is capped at 15%
compositor colour. Terminal opacity is set separately below. The tab strip stays opaque.

Automatic terminal colours use the chosen palette's terminal colours (Neutral is `#101010`).
Match window uses the window bar background. Custom uses the background, text and accent picked on
the Terminal page; the renderer adjusts pairs below 7:1. Colours apply immediately, and 9–24 logical px font-size
changes resize all open screens and pseudo-terminals. Cursor shape and background opacity are
described under [Terminal cursor and backdrop](#terminal-cursor-and-backdrop).

Tab marks, hover, menu entrances, the dialog dimmer, sessions-list selection and settings controls
animate briefly. Reduce motion snaps them to their targets, as does the system setting. Tab content
switches instantly; terminal text never cross-fades. Real-mode running-session scans are unchanged,
but a polling tick no longer redraws the window unless visible state changes.

## Settings pages

The category list at left selects one page at right: Appearance, Terminal, Sessions, Usage,
Handovers, OpenCode, or Notification area. Only the current page scrolls. Up and Down change
categories when the list has focus, Tab enters the page, and Escape dismisses a popup before
closing Settings. Each window remembers the last page while it runs; category selection is not saved.
A category switch cross-fades the old page into the new one in 140 ms (the toolkit's
`PageTransition`; the old page is drawn once more in the first frame and blended away). The app's
Reduce motion, Windows' reduced-motion setting and high contrast make it instant.

Controls still apply live. An ordinary config save merges only the top-level fields edited against
the window's loaded baseline, refusing a conflicting change to the same field. It never republishes
the separately persisted usage opt-in or handover consent; the tab-restore preference still uses
`window.ini`. Switching categories does not write settings.

## Recent sessions and local marks

Recent is the default. It sorts sessions across folders by activity, newest first, with local
calendar headings: Today, Yesterday, This week (Monday onward), Earlier this month and Older.
It uses the current local offset for the comparison; historical daylight-saving transitions
are not resolved separately. See [Session organization](#session-organization) for the alternate modes.
Only the visible rows and sticky heading are drawn. Mode and Show hidden are config preferences.

Rows show the latest `last-prompt` record (`lastPrompt`) or real user message found in the
bounded transcript tail. Search and the quick switcher match the local name, original title,
last prompt and folder; the switcher orders by fuzzy match quality, then recency. Opening from
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
the app leaves a damaged marks file alone. Showing details or a tab clears Unseen for its
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

This supersedes the earlier notes that tabs cannot be moved or restored.

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
an exit with no replacement tabs. Restoration uses the normal resume warning, process scan
and launch reservation, always targets embedded tabs, ignores a stale saved folder in favor of
the indexed one, and reports skipped entries together.

The Sessions tab is fixed. Terminal tabs reorder when dragged past a neighbor’s midpoint.
Right-click actions target the tab under the pointer, not the selected session row. The
account-copy path uses the same fork and cache warning as the Sessions list, and compaction
opens the handover dialog. Tab renames last until close. Close others keeps the clicked tab
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

Archive is independent of Hide and of the Desktop app’s archive flag. An age threshold counts
back whole weeks from now, excluding the boundary itself and unknown ages. The preview shows
the eligible IDs and their indexed transcript bytes. Commit considers only those IDs, rechecks
pins, live sessions, embedded tabs, recent launch reservations and transcript modification
times, then saves the marks as one update. An incomplete running scan refuses the operation.
Archived rows are omitted from ordinary views but found in searches; the Archived chip
appears only while there are archived sessions in the index. Unarchiving preserves Hide marks.
No archive operation relocates, deletes, or edits a transcript.
The session menu explains why a pinned or running row cannot be archived. Folder archive
remains available because its preview can include other eligible rows.

The Markdown report covers all currently indexed archived sessions. It uses bounded
transcript heads/tails for the first dated conversation record and first/last prompt excerpts,
the latest local handover filename if present, and the local stats reader for turns and
context/output tokens (`~` denotes an estimate). Unknown dates or statistics are shown as
`—`. Rendering is pure; Save As warns about conversation text, writes a new file without
replacing an existing destination, and offers to open it explicitly. No AI or network is used.

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

**What counts as ready.** The screen heuristic checks the live cursor's `❯` input row, including
boxed rows, and ignores SGR-faint placeholder characters. It rejects recognized menus, permission
questions and the alternate screen. A successful write only means input was handed to the terminal.
An ordinary automatic send needs a newly observed, explicit transcript `end_turn`, no unresolved
tool in the current turn, at least 750 ms without terminal output, bracketed-paste mode, and no
alternate screen or blocking drawer signal. An old completion from a resumed transcript, missing
stop reasons, an unrecognized title, and terminal quiet alone are not proof. The OSC title has no
verified Claude readiness protocol: explicit busy labels only veto a send. On a transcript reset
or after any composer send or direct terminal input, an old completion cannot rearm sending;
a new prompt must be observed. There is still a race between observing the PTY and writing it.
When uncertain, Chat retains the draft and displays the refusal inside the composer until its cause
changes. **Send anyway** is explicit and requires bracketed-paste mode outside the alternate screen.
During an active turn, **Queue** keeps one draft in memory and locks its text and attachments until
it sends or is cancelled. Automatic delivery waits for a newer completion and a visibly empty prompt,
even after switching tabs. A minimized window, an open dialog or another blocking window overlay
pauses automatic writes in every tab; stage deadlines still run. After the app's **Stop**, a fresh
clean prompt with the observed working
indicator gone (or a new interruption notice) can also release the queue. **Send now** can send
during a turn when that prompt is visibly empty; **Cancel**
unlocks the draft without sending. Neither path overrides a recognized permission question, pending
tool, bell or slash menu. Direct terminal input cancels a queued or staged send, and samples a
transcript byte watermark before writing so a buffered earlier prompt cannot rearm Chat.

**When the drawer opens.** A pending tool first observed at least four seconds ago plus 750 ms
of terminal silence is a hint, not proof of a permission question. A slow tool can look the
same. A BEL, a slash sent by Chat, and process exit also open the drawer. Only an automatically
opened drawer closes on observable turn progress; a manually opened one stays open. No
permission responses or dialog keys are generated. Stop sends Escape at most once per freshly
verified active turn.
**Slash commands.** An initial `/` opens Switch’s list, never writes a bare slash to the PTY.
The list filters a reference catalog of built-ins and read-only names and frontmatter descriptions
from the selected account, the launch folder’s `.claude` directory, and installed plugin caches.
It does not execute plugins or mirror CLI policy, and caches the scan for the tab. Stars persist in
`window.ini`. Use ↑/↓ to choose, Tab to fill, and Enter to run. Model and Effort offer inline choices
and toolbar menus; Model can also open the native picker. Model, effort and context labels use
reported values, not clicked values; unknown effort stays blank. Direct slash commands need a
verified, quiet, visibly empty CLI prompt, no active turn or blocking interaction, and the same
terminal mode checks as Send. Chat writes command text first, and only writes Enter after seeing
the exact text on the live input row, within two seconds. Compact and Clear (including Clear aliases)
ask for confirmation.
Interactive CLI results stay in the drawer. Chat does not generate answers to its menus; without
observable progress or a quiet, empty prompt it cannot verify that the menu closed.

**Local data.** File chips use the existing path quoting. Ctrl+V prefers clipboard text. Use
Ctrl+Shift+V or **Attach… → Paste picture** to choose the picture explicitly. Supported picture
formats get an in-memory thumbnail. A send checks the clipboard sequence, writes Alt+V alone,
and waits up to four seconds for a newly displayed `[Image #n]` on the CLI input row. Only then does
Chat paste the text and write one Enter. Timeout or an intervening input, permission, clipboard or
session change retains the draft. Failed picture or command staging blocks new sends until newer
terminal output shows a quiet, clean prompt. If paste succeeded but Enter did not, **Press Enter**
rechecks the staged input and retries only the Enter key. The composer replaces line feeds and
tabs with spaces for messages, refuses other control characters, and writes the text as bracketed
paste followed by one app-generated Enter. Ordinary
terminal paste refuses multiline clipboard text when bracketed paste is off. Composer Up on the
first displayed line and Down on the last walk real prompts in the main transcript, including
terminal input. Recall skips adjacent repeats, pages back through the transcript worker as needed,
and restores the original editor on Escape or Down past the newest. Editing or attempting to send
ends recall without restoring the old draft.
While the slash picker is open, it keeps the arrow keys. Expanded tool rows show bounded previews:
Edit compares the unmatched middle after common leading and trailing lines, Write shows
additions, and shell output uses only its first text lines. In Chat's Details panel, Task and Agent
rows expand to show full task text or reported agent details. **Changed files** lists successful
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
turn and prefer the answer over a tool result that repeated the same link. Both Chat and Session
view share the Shells and Outputs Details rendering.

Context now uses
reported input and cache tokens; no limit or percentage is guessed. The toolkit’s TextField
is single-line; the composer wraps the app’s multiline editor at words, displays hyphens only in
overlong words, and uses the toolkit context menu for Cut, Copy, Paste and Select all.
Demo shows the waiting state, and its composer writes only to the synthetic terminal screen.

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

**Include topics when suggesting names** is off by default in Settings → Sessions. If enabled,
the existing confirmed naming request adds existing short topic labels and asks for an optional
topic beside each title. Its transcript excerpts and request count are unchanged. Review keeps
title and topic editable before acceptance. **Set topic…** edits or clears a local label without
running a model. Topics have at most three words and 60 characters; blank means no topic. The demo contains synthetic
projects, a worktree, a four-session thread, adjacent activity, Related prompts and topics.

## Terminal cursor and backdrop

Settings → Terminal saves `appearance.cursorShape`, `appearance.cursorBlink`, and
`appearance.terminalOpacity` in `config.json`. Every tab uses the latest preferences when drawn,
including the Chat drawer. The default cursor is a steady Block; Bar and Underline are fallbacks
when the program has not chosen a DECSCUSR style. Blink is off by default. When enabled it still
respects an explicitly steady cursor, the system caret interval, focus, visibility and reduced motion.

Opacity defaults to 100% and steps down to 60% by 5%. It affects normal backgrounds and padding,
not text, explicit terminal backgrounds, or selected cells. The toolkit can raise opacity for text
contrast. Without an active Mica or Acrylic backdrop (including high contrast or loss of window
focus), the setting is disabled and the surface draws opaque. The scrollback thumb has a solid
gutter and the end notice is drawn on solid rows. The Chat drawer header is solid. Window-level
popups and dialogs temporarily use an opaque terminal frame beneath their edges and scrims.

`--demo --demo-terminal bar,80,mica` previews this with a synthetic Chat drawer; a debug build
can add `--shell cmd.exe` to preview it with an isolated command prompt instead.

## Tab identity and interaction checks

The running-session poll retries unattached live tabs even when PID records are unchanged.
An attached pane keeps its original transcript and draft. If a live record identifies a
different session, the tab clears the current identity used by Open, local marks, and
background transcript notices without moving that draft to the other session. A missing
PID record retains the existing duplicate-resume guard, not permission to send or mark seen.
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
