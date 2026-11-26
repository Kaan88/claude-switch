# Claude Switch

Claude Switch is a small Windows desktop app for people who work across Claude Code sessions, folders, or accounts. It is early software and Windows-only.

## Features

- Find local Claude Code sessions and see their working folders.
- Resume a session as the chosen CLI account. By default, it opens in a terminal tab in the same window.
- Open sessions in the Claude desktop app. When more than one Desktop profile is running, the hand-off may go to the default profile.
- Start a new CLI session in a chosen folder or open a copy of an existing session.
- Make an editable handover with an OpenCode model and open it in a brand-new Claude Code session.
- Keep separate logins while sharing transcripts and settings.
- Check OpenCode’s version and active sessions in **Settings → Workers**, confirm a service restart, or open OpenCode Config Studio when installed.
- Open oversized handovers without trimming them: the new Claude session is asked to read the saved Markdown file before continuing.

## Screenshots

![Session list](docs/screenshots/sessions.png)

![Embedded terminal tab](docs/screenshots/terminal.png)

## Install

Download and run the installer from [Releases](https://github.com/Kaan88/claude-switch/releases). It installs for your Windows user and does not need administrator rights. Install the Claude Code CLI to use terminal sessions and the Claude desktop app to use Desktop sessions.

To build from source on Windows, install Rust stable and run:

```powershell
cargo build --release -p claude-switch
```

The executable is `target\release\claude-switch.exe`. The [markdown-shared-rs](https://github.com/Kaan88/markdown-shared-rs) toolkit is fetched by Cargo.

## Accounts and settings

The first account uses your existing Claude CLI and Desktop profiles. Each extra account gets separate profile folders and sign-ins. The CLI accounts share transcripts, skills, agents, plugins, and settings from the first account. Claude Switch copies the user-level MCP server settings at launch and does not overwrite a conflicting settings file. Credentials and running-session records stay separate.

## Duplicate-session safeguard

Claude Switch checks its tabs and the running-session records across accounts before resuming. If the session is already running, it will not start another instance. If the session is already open in one of its tabs, it switches to that tab instead. A short cooldown also prevents a second launch before the running record appears.

## Terminal shortcuts

| Shortcut | Action |
| --- | --- |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Ctrl+Shift+W | Close the terminal tab |
| Ctrl+Shift+T | Open a plain shell in the selected session's folder |
| Ctrl+Shift+K | Open the command palette |
| Ctrl+Shift+C / Ctrl+Shift+V | Copy / paste |
| Ctrl+C | Copy if text is selected; otherwise send Ctrl+C to the program |
| Shift+Page Up / Shift+Page Down | Scroll back / forward |
| Enter in an ended tab | Close the tab |

You can also middle-click a terminal tab to close it.

The OpenCode controls are in **Settings → Workers**; there is no separate keyboard shortcut.
Use `--demo --demo-workers` or `--demo --demo-compact-long` to preview the new screens without
starting OpenCode or Claude.

## Privacy

Claude Switch has no telemetry. When **Show usage** is on (the default), Claude Switch reads each account’s CLI login token to ask Anthropic for its subscription usage. It sends the token only to Anthropic over HTTPS and caches the figures, not the token. The usage endpoint is undocumented and may stop working. Turn **Show usage** off in Settings → Accounts to disable these reads and requests. Claude Code and Claude Desktop use their own network connections when you run them.

## Compact and open

Right-click a session and choose **Compact and open…**, or use the command palette or the details
pane’s **Open as…** menu. Pick a model from OpenCode and the account to start Claude Code as.
After the first-use consent for that provider, review and edit the handover before starting the new
session. The character/token estimate updates as you edit. You can also copy or export it as Markdown.
The original transcript is read-only. The details pane’s **Handovers** button lists up to 20 local
copies per session; the new session’s row says **From** followed by the old name.

OpenCode receives assistant text, one-line tool summaries and the optional focus. The app appends
verbatim prompts locally. Tool results, pictures, thinking and system reminders are excluded from
the OpenCode request. OpenCode archives its own run; this is not a no-retention option. To remove
remembered permission, use **Settings → Handovers → Forget consents**.

```powershell
claude-switch compact --session latest --cwd . --print
claude-switch compact --session latest --cwd . --as work --open
```

Use a session ID instead of `latest` to target one transcript; `--model provider/model` overrides
the remembered model, and `--focus-file` reads UTF-8 emphasis text. The CLI requires prior provider
consent. `--print` outputs Markdown and opens nothing; `--open` explicitly opts into starting
the new session in a new Claude Switch window. Run it from the GUI instead to review first.
A native `claude.exe` must be on `PATH`. Switch finds native OpenCode on `PATH`, in common
install directories, or via an absolute `OPENCODE_EXE` override. Handovers over 15,000 UTF-16
code units are saved in full; the new session's initial prompt asks it to read the saved
Markdown file. Local process-inspection tools may see the short handover text or the longer
handover's file path in the command line.

### Install the Claude Code plugin

Put `claude-switch.exe` on `PATH`. To load the plugin for one Claude Code session, start Claude Code
with `claude --plugin-dir "<path-to-claude-switch-repo>\plugin"`. To keep it available in a trusted
workspace, copy the whole `plugin` directory (including `.claude-plugin`) to
`.claude/skills/claude-switch` in that workspace, then start a new Claude Code session. Use
`/claude-switch:switch-compact [what to keep]` after granting the provider consent in Claude Switch.

## Licence

[MIT](LICENSE).

Claude Switch is an unofficial tool. It is not made by or affiliated with Anthropic. “Claude” is a trademark of Anthropic.

## Find and manage sessions

**Recent** puts all folders in one newest-first view; see [Grouping sessions](#grouping-sessions)
for other views. Pins take priority. Search includes the last prompt, and the All, Running, Waiting, Pinned
and Unseen filters help you narrow the list. Right-click a row to open it, rename or pin it,
hide it or its folder, copy its ID or folder path, or reveal its transcript in Explorer.
Hidden sessions are not deleted; turn on **Show hidden** in Settings to find and unhide them.

| Shortcut | Action |
| --- | --- |
| Ctrl+P in Sessions / Ctrl+Shift+P in a terminal | Quick switcher |
| ↑ / ↓, Enter, Ctrl+Enter, Esc in the switcher | Move, open, open as another account, close |
| F2 | Rename locally; clear the field to restore the transcript title |
| Ctrl+D | Pin or unpin the selected session |
| Delete in the list | Hide the selected session; the toast offers Undo |

**Suggest names** is optional. After you confirm, Claude receives the first prompt and up to
the last three prompts (500 characters each), using the worker account's usage. Suggestions
are never saved unless you accept them. Rename, pins and hiding never edit transcripts.

**In conversations** (shown as **Chat** in a narrow window) searches prompts and replies locally.
All search terms must appear on the same line, in any order. Results show up to three matching
lines per session, newest sessions first. The details pane shows turns, tool calls, pictures,
subagents, and context/output tokens. Figures marked **about** are character-based estimates.

| Shortcut in Sessions | Action |
| --- | --- |
| Ctrl+Shift+F | Toggle In conversations and focus the search field |
| Esc / Enter | Clear and cancel the search / open the selected result |

### Window and terminal tabs

Claude Switch remembers the normal window size and position, even when you quit maximized.
If the saved position is mostly off-screen, it opens centered in the primary monitor’s work area.
**Settings → Terminal → Reopen terminal tabs on startup** offers **Ask** (default), **Always**,
or **Never**. Only Claude sessions can be reopened; the app saves no shell tabs or terminal text.
Reopening still honors the usual resume warnings and skips sessions it cannot safely open.

Drag a terminal tab past another tab’s midpoint to move it. Middle-click closes it, with the
same confirmation as its close button. Right-click to rename the tab, close it or the other
terminal tabs, or open a fork as an account. **Compact and open as…** uses the handover dialog.

| Shortcut | Action |
| --- | --- |
| Ctrl+Tab / Ctrl+Shift+Tab | Cycle tabs in either direction |
| Ctrl+1–9 from Sessions or the command palette | Select a tab by position; Ctrl+1 is Sessions |

In a focused terminal, Ctrl+digits go to the running program. To jump by number, open the
command palette with Ctrl+Shift+K first.

### Folder defaults and archive

The main **Open** button, Enter, and **New session** use the folder’s last successful account
and app (CLI or Desktop). Use **Change** in the session details to edit or clear it. If that
account is removed, the selected account takes its place.

Choose **Archive older sessions…** from the Sessions list’s **Archive** menu or the command
palette. Pick 2, 4, 8, or 12 weeks and optionally one folder. The preview shows the number of
sessions and their transcript size. Right-click a session or folder for a targeted archive.
Pinned and running sessions are skipped. Archived sessions stay on disk and still appear in
search, labelled **Archived**. Use the **Archived** chip to browse them, and the Archive menu
to unarchive a selection or all. **Hide** remains a separate local mark.

**Summary report…** saves a local Markdown file, including prompt excerpts. Its suggested
location is `Documents\Claude Switch\archive-<date>.md`. The app asks before saving or opening it
and never overwrites an existing file. Treat the report as private conversation text.

| Shortcut in Sessions | Action |
| --- | --- |
| Ctrl+N | New session, with the chosen folder’s account and app |
| Ctrl+K | Find **Archive older sessions…** in the command palette |
| Ctrl+click or Space | Add/remove an archived row from the selection |

## Background notifications

- Background Claude tabs get a Waiting mark. Choose **Toast and mark**, **Mark only**, or
  **Off** in Settings → Sessions. With the tray icon enabled, a Windows notice can open
  the waiting tab; otherwise **Toast and mark** uses the mark and a taskbar flash.
- Settings → Accounts can warn at 80% and 95% usage. If another account has room, the usage
  pop-up offers **Open this session as …** to start a handover with that account selected.
- Settings → Notification area enables the tray icon and optionally **Close to tray**.
  The tooltip shows usage, and the menu opens running sessions, starts a new session,
  restores the window, or quits with the usual confirmation.

| Shortcut in the account usage pop-up | Action |
| --- | --- |
| Shift+Enter | Open the suggested handover if offered |

## Chat in a CLI tab

Pick **Chat** or **Terminal** at the right of a Claude session’s tab strip. The choice stays
with each tab and sets the default for new ones. Chat shows the local conversation, a multiline
composer, tool previews, and a collapsible Tasks, Agents, and Changed files panel on wider windows.
The same Claude Code process keeps running in the terminal drawer. Drag the bar above the drawer
to resize it, and answer permissions and other CLI dialogs there.

| Shortcut in a session tab | Action |
| --- | --- |
| Ctrl+Shift+K, then F6 (or choose **Switch Chat / Terminal**) | Switch layouts without sending a key to the CLI |
| Ctrl+Shift+E | Toggle the terminal drawer in Chat or Session view in Terminal |
| Enter / Shift+Enter in the composer | Send / insert a new line |
| Up from an empty composer, then Up / Down | Recall earlier prompts from the loaded transcript |
| Ctrl+V / file drop | Queue a clipboard image or file as a removable attachment |

Chat keeps drafts when it cannot tell that the CLI is ready. Check the terminal before using
**Send anyway**. A slash opens the CLI’s own menus in the drawer. Chat does not answer for you.

## Grouping sessions

The Sessions switcher now offers **Recent**, **Projects**, **Threads**, and **Topics** when a
session has a topic. **Projects** combines folders and worktrees of the same repository. Use
**Group by: Project / Folder** to switch back to the original folder grouping. **Threads**
folds proven continuations under their newest member; click **N earlier** or use Left/Right
with the thread’s first row selected. In **Recent**, turn on **Sittings** to bracket same-day
activity in the same project within 30 minutes. Scroll the details pane to **Related** to
open thread members and sessions with similar prompts. These features work locally.

Right-click **Set topic…** to edit or clear a topic. To suggest topics alongside titles,
enable **Settings → Sessions → Include topics when suggesting names**. The existing
confirmed **Suggest names** request includes topic labels to reuse, but no extra transcript
excerpts or separate request. In review, Tab moves between the title and topic fields.

## Terminal cursor and opacity

In **Settings → Terminal**, choose Block, Bar, or Underline and optionally enable **Blink**.
The terminal program’s explicit cursor style still takes precedence. **Background opacity**
goes from 60% to 100% (the default) when Mica or Acrylic is active on Windows 11 22H2 or later.
These settings apply to open tabs and to Chat’s terminal drawer. **Ctrl+Shift+E** opens or closes
the drawer in Chat.
