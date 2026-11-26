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

Files can be tens of megabytes: read at most the first 256 KiB and the last 256 KiB, never the whole file.
Entries are internal to Claude Code and may change: a line that does not parse, or lacks a field, is
skipped, never an error.

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
  in it). `claude auth status` prints JSON with `"loggedIn"`; `claude auth login` signs in.
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

## The safeguard

Before any launch or carry, the running sessions of every account are read again. A session that is
running anywhere is not started a second time; the app says where it runs (which app, which pid).
Several different sessions on one account at once are fine.

## What a launch is

One value, `Launch`, says program, arguments, folder and environment. The window decides where it runs:
an embedded terminal tab (`mdshared-pty` + `mdshared-term`), an external terminal window, or a detached
process (the desktop app). The core never spawns anything itself except `claude auth status`.

## Workers

The owner's bridge (`claude-flash-bridge`) runs Claude Code workers with `claude -p`. A setting here says
which account they use; it is written to `%USERPROFILE%\.config\opencode-bridge\workers.json` as the
top-level keys `claudeAccounts` (the CLI folders) and `claudeWorkersUse` (`"same"`, `"other"`, or one of
the folders). The bridge reads them.
