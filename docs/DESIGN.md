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
  file exists without opening it. `claude auth status` prints JSON with `"loggedIn"`; `claude auth login` signs in.
- Desktop: the app is an MSIX package; its executable is in the command registered for the `claude:`
  protocol (`HKCU\Software\Classes\claude\shell\open\command`). Started with
  `--user-data-dir=<dir>` it runs as a second, separate copy with its own login, beside the normal one.
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

- linked into its CLI folder (directory junctions; a hard link or copy-on-launch for single files):
  `projects` (transcripts and memory), `skills`, `agents`, `plugins`, `mcp-servers`, `settings.json`,
  `CLAUDE.md`, `keybindings.json`;
- copied on every launch from the first account's `.claude.json` into its own: the `mcpServers` key only;
- never shared: `.credentials.json`, `oauthAccount`, `sessions`.

A hard-linked file stops being a link when a program saves it by writing a temporary file and renaming
it. Before every launch the links are looked at again: a copy with the same content is linked again; a
copy that differs is the owner's to settle and is never overwritten (it shows as a conflict).

Found on this computer and allowed for: the desktop records' folder also holds JSON files that are not
session records (only files with a `cliSessionId` count); `sessions/` also holds `<pid>.<hash>.key`
files (only numeric `<pid>.json` is read); entrypoints seen are `cli`, `sdk-cli`, `sdk-ts` and
`claude-desktop`.

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

A session that was just started is not in the running list for a few seconds. Every start is therefore
written to `<appdata>/claude-switch/history.json` (session id, account, time) before the program is
started, and a second start of the same session within 20 seconds is refused. The same file says which
account a session last ran as, for the cold-cache warning.

A launch never passes on the identity of the session the app itself was started from: the login and
session variables of Claude Code (`CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_API_KEY`, `CLAUDECODE`, …) are
removed from the environment of everything it starts.

Not settled (needs a second signed-in account to try): which copy of the desktop app receives
`claude --desktop --resume` when two are running.

## What a launch is

One value, `Launch`, says program, arguments, folder and environment. The window decides where it runs:
an embedded terminal tab (`mdshared-pty` + `mdshared-term`), an external terminal window, or a detached
process (the desktop app). The core never spawns anything itself except `claude auth status`.

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

**The safeguard, for tabs.** A session open in a tab counts as running at once, before any
`sessions/<pid>.json` says so: opening it again shows its tab ("Already open in a tab"), and the list
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

Resumed tabs use the transcript file already found by the session scan. New and forked tabs look for
the newest JSONL **created** in the launch folder's encoded project directory after launch, under
that tab's account config folder. This needs a transcript to have been written, and can't disambiguate
concurrent starts in the same project (including accounts sharing `projects`). It doesn't track a
later working-directory change. The reader rejects an explicit mismatched working folder found in
the candidate's first 64 KiB; otherwise it relies on the project directory and creation time. The
pane updates the tab's session ID once discovered.

Conversation, Agents and Tasks are separate views of the incremental transcript reader. Conversation
opens on the newest messages and follows appends until the user scrolls back; Jump to latest resumes
following. Earlier history is loaded on request. Consecutive tools collapse together; an agent row
opens its own transcript. The reported agent state comes from the transcript sidecar, so a missing
completion flag is shown as unknown rather than guessed.

Conversation uses the toolkit's `Painter::rich_text` for selectable, wrapped headings, emphasis,
bullets, quotes, code and links. It is a Markdown subset, not a web renderer: no inline HTML, remote
images or tables. Local file links are resolved on click; only recognized document, source and
media types open in their default applications. Ctrl+click reveals any recognized existing file.

Only the visible pane requests polling: on terminal output, with a 500 ms fallback. Reader and
picture work run off the window thread; worker results wake the window rather than making it redraw
every poll. Image bytes are decoded with `mdshared-text`'s existing PNG/JPEG decoder and cached for
inline previews and an enlarged overlay. Opening a picture in the system viewer explicitly exports
a copy to the application's temp folder. No transcript is edited. The demo supplies a synthetic
session, writes only to temp, and appends a few updates without starting Claude Code.

## Pictures into the CLI

Ctrl+V, Ctrl+Shift+V and the palette's Paste send clipboard text using the terminal's usual paste
mode. With no text they send `ESC v` (Alt+V), so Claude Code reads its clipboard image directly.
The Attach picture button opens the toolkit's file picker. File drops arrive through the host's raw
winit hook (one file at a time, without a drop coordinate); the shown terminal tab receives the batch.
Both routes send space-separated paths, with whitespace-containing paths in double quotes, and use
bracketed paste only when requested by the program. They do not send Enter.

## Appearance

Settings has navigable Appearance, Terminal, and Custom colours sections (as well as Workers and
Sessions). The window theme is System, Light or Dark; `--theme light|dark` overrides that preference.
Windows high contrast takes priority. The toolkit's appearance model supplies the accent (theme,
system or picked), dialog and menu corners, optional soft shadows, and the system-aware motion policy.
Mica and Acrylic are optional on Windows 11 in the title bar, capped at 15% compositor colour.
The terminal grid, its padding, and the strip it joins stay opaque.

Automatic terminal colours use Dark (`#101010`) or Light according to the effective window theme.
Match window uses the window bar background. Custom lets the user pick the background, text and
accent; the renderer adjusts pairs below 7:1. Colours apply immediately, and 9–24 logical px font-size
changes resize all open screens and pseudo-terminals. The cursor is steady by default; enabling blink
also respects the program's request, the system caret setting and reduced motion. The program sets
the cursor shape.

Tab marks, hover, menu entrances, the dialog dimmer, sessions-list selection and settings controls
animate briefly. Reduce motion snaps them to their targets, as does the system setting. Tab content
switches instantly; terminal text never cross-fades. Real-mode running-session scans are unchanged,
but a polling tick no longer redraws the window unless visible state changes.
