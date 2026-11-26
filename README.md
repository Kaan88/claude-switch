# Claude Switch

Claude Switch is a small Windows desktop app for people who work across Claude Code sessions, folders, or accounts. It is early software and Windows-only.

## Features

- Find local Claude Code sessions and see their working folders.
- Resume a session as the chosen CLI account. By default, it opens in a terminal tab in the same window.
- Open sessions in the Claude desktop app. When more than one Desktop profile is running, the hand-off may go to the default profile.
- Start a new CLI session in a chosen folder or open a copy of an existing session.
- Keep separate logins while sharing transcripts and settings.

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

## Privacy

Claude Switch reads Claude Code's local files. Claude Switch itself sends nothing anywhere and has no telemetry. The Claude CLI and desktop app still use their own network connections when you run them.

## Licence

[MIT](LICENSE).

Claude Switch is an unofficial tool. It is not made by or affiliated with Anthropic. “Claude” is a trademark of Anthropic.

## Find and manage sessions

**Recent** puts all folders in one newest-first view; **Folders** groups them by project. Pins
appear first in both views. Search includes the last prompt, and the All, Running, Waiting, Pinned
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
