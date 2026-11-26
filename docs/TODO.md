# Where this stands and what is next

Written 2026-11-26, when work was stopped for the night. Read `DESIGN.md` first.

## State

- `crates/core`: session index, launch safeguards and incremental transcript reader. Real-data and
  manual timing checks are ignored by the ordinary workspace test run.
- `bins/claude-switch`: terminal tabs, session view and clipboard/file attachment input.
- The toolkit (`markdown-shared-rs`) is a git dependency pinned to a revision of its `main`; raise the
  revision in `bins/claude-switch/Cargo.toml` to take a newer toolkit.
- Build: `cargo build -p claude-switch`.
- Hidden flags for trying things: `--demo`, `--theme light|dark`, `--shell <program>`, `--type <text>`,
  `--sessions-tab`.

## Completed follow-ups

- Open a copy is in the palette and under "Open elsewhere"; the Desktop and cold-cache warnings
  share one confirmation. Both are documented in `DESIGN.md`.
- Shell and Desktop launches strip the CLI config, login and session variables. Confirm dialogs use
  Enter for the primary action and Escape to cancel.
- Ended tabs show their launch label with " (ended)"; terminal drag requests a redraw only when
  the selection changes. `app.rs` is under 600 lines again.
- New session asks for a folder and account. The running-session scan includes the last removed
  account's folder.
- The two CLI login records have the same account UUID. The Accounts dialog reads them separately
  and does not invoke `claude auth status`; no status-routing change was needed.

## Tried by hand on 2026-11-26 (second pass)

Driven by window messages posted to the app and screenshots (the drive script is not in the repository).

Works: typing letters and digits in a tab; Enter, Tab, Escape, Backspace, the arrows, Home, End, Delete,
Page Up, F1 and F12 arrive as the right sequences; bold, dim, italic, underline, inverse, the 16 colours,
256 colours and true colour; drag selection; the Sessions tab and back; choosing a session; search and
its clear button; the menu; Settings; Accounts (two accounts listed); New session (before the picker
was added); typing into Claude Code;
closing a running tab asks, Enter takes the primary button (Cancel), Close ends it.

The "all text is bold, dim is not dimmed" defect was not one: the test shell had `NO_COLOR=1`, so Claude
Code printed without colours.

Never tried: every shortcut with Ctrl, Shift or Alt (posted messages cannot hold a modifier), copy and
paste, the wheel with scrollback, opening an existing session, Open a copy, the Desktop warning, anything
as the second account, Windows Terminal, the desktop hand-over.

## Defects noticed

- Recheck the historical Claude Code white selection bar in a real session. The app now sets both
  selection colours. If Claude Code enables mouse reporting, use Shift+drag to test the terminal's own
  selection; a plain drag goes to the program. In the 760×480 `cmd.exe` demo, the same selection tint
  covered both SGR 7 inverse spaces and ordinary blank cells.
- Insert and Shift+Insert cannot be typed (`keys::key_name` has no Insert); input-method composition
  does not work (the toolkit's host turns it off on Windows); F13 and above; Ctrl+Alt+letter on
  AltGr layouts.
- `claude --desktop --resume` as a further account probably opens the first account's copy of the
  desktop app (review finding 5, not verified).

## Later (the owner's list)

- **Workers tab**, after a usable first version: the bridge's jobs live (worker, model, folder, running
  time, last tool) from `%TEMP%\opencode-agent\job_*.json`, the output streaming from `job_*.jsonl`, and
  Cancel. Read-only on the bridge's files.
- A real hand-over out of Desktop (make it let go of a session). No supported way is known.
- Multiple terminal splits, search in scrollback, font-family settings, tab reordering, restoring tabs.
- Reuse Conversation in Sessions details later. That view is currently a short read-only preview
  and has no independently scrollable transcript region.
- Verify actual Claude Code image attachment via Alt+V and pasted paths in a real CLI session.

## For the toolkit (what the app's authors had to work around)

1. Drop-down and menu items cannot carry a colour swatch.
2. No plain push button or icon button outside dialogs.
3. No search box (a field with a leading icon and a clear button).
4. No badge; no label/value grid.
5. `fmt` has no relative time ("3 h ago") and no local time-zone offset.
6. No model for a simple confirm or text-prompt dialog.
7. `InfoBar::hit` and `parts` need a `Painter`.
8. `chrome::draw_tab`: its y is fixed to the title row; the icon cannot be changed; no tab without a
   close button; its fill is `pal.bar`, so the app supplies a per-tab palette for the active terminal.
9. `mdshared_pty::Command` has no getters besides `program()`.
10. `keys::KeyName` has no `Insert` and nothing beyond F12, though `mdshared_term::input::Key` has.
11. `host::App::raw` takes a winit type that is not re-exported.
12. `mdshared_term::Screen` has no way to add a local line (the end-of-program notice).
13. `Event::Wheel` gives pixels, not lines; Shift+wheel arrives as `dx`.
14. `termview::draw::look` replaces a selected cell's background after `Scheme::cell_colors` resolves
    defaults and swaps inverse colours. Inverse/default blanks therefore cannot produce a white app
    selection by themselves. However, with `selection_text: None`, explicit backgrounds and inverse
    glyphs can lose contrast against the replacement fill; a full-block glyph can even cover that fill.
    The app supplies a contrasting selection foreground as well as a background in every scheme.
    The focused block cursor still paints above the selection in its own cell.
17. `settings::pages::Form` has no colour rows or configurable card style. Use the grouped, scrolling
    `settings::Form` with its category rail, native colour rows and animated controls instead.
18. The grouped form's style controls its card, not its internal dropdown menus or colour-picker
    flyouts. The app styles its own menus through `menu::Look`; those internal popups keep the
    toolkit's look.
19. `richtext` draws externally shaped text but does not parse Markdown or manage selection.
    The session pane supplies a small Markdown parser and uses Parley's selection geometry for
    mixed-font text.
20. No safe clipboard-image getter in the toolkit or std. Paste picture as file is not offered;
    Claude Code can read the clipboard via its own Alt+V key.
21. `host::Event` has no file-drop event with positions or file batches. The raw winit hook supplies
    file paths to the active terminal tab, with a short coalescing delay for multiple files.

### Done in the toolkit

- 15: `termview::BackgroundOpacity` composites the default terminal background and padding
  independently of the chrome veil, with contrast protection and opaque selection/explicit backgrounds.
- 16: `View::cursor_preference` and `Screen::cursor_style_is_explicit` support a host default
  without overriding DECSCUSR, cursor visibility, or wide-character coverage.

## The bridge (`claude-flash-bridge`)

The worker-account setting (`claudeAccounts`, `claudeWorkersUse` in `workers.json`) is written, tested
and in the README, not committed, and not copied to the installed clone
(`~/.claude/mcp-servers/opencode-agent`). The app's Settings dialog writes the two keys.

## Recent sessions

- Implemented: Recent/Folders, prompt search and quick switcher, local names, pins, hiding/Undo,
  running/waiting/unseen filters and opt-in name suggestions with review and cancellation.
- Name suggestion tests use an injected fake program. Live Claude authentication, CLI flag
  compatibility and returned titles still need an explicitly authorized smoke test.
- Waiting is deliberately heuristic; see `DESIGN.md` for its limits.

## Reported after 0.1.0 (owner, 2026-11-26): recorded, nothing changed yet

1. **A newly added third account keeps saying "Not signed in" under usage** although its sign-in tab
   ended with "Login successful" and the account's header line says "Signed in to the CLI". The text
   comes from `usage_ui.rs` `state` (`Error::NoFile | Error::NotLoggedIn`, or no reading with
   `signed_in == Some(false)`). To find out: whether the usage attempt made before the sign-in is kept
   and never retried when the sign-in tab ends, whether Refresh is disabled in that state (it looks
   greyed in the owner's screenshot), and whether the new selective credential reader (W06) refuses
   what a fresh login writes.
2. **"Open Desktop as this account" answers "The Claude desktop app was not found on this computer"**
   (`Refusal::NoDesktopApp` from `core/launch.rs` `desktop_exe()`), on a PC where the desktop app is
   installed and the first account shows "Desktop app used". To find out: where the installed app
   really is (per-user installer, Store/MSIX package) against what `desktop_exe()` looks for, and
   whether the native-executable rule of the review rounds narrowed the search.
3. **Settings should be split into proper categories** (a category list with one page each) instead
   of one long scrolling list.
4. Seen in the same screenshot, not reported: the "This week by app" rows (Claude Code, Chats,
   Cowork, Other) show no figures; the Refresh buttons are cut off at the dialog's right edge; the
   first account shows no Session row while the second does.
5. **Hovering over an open context menu also highlights the list row underneath it** (owner,
   2026-11-26; seen with the session list's menu over the Recent list). The pointer position should
   not reach the list's hover state while a menu, dialog or palette is open above it. Also in that
   screenshot: "Archive session…" is greyed while "Archive folder…" is not; check that this is the
   pinned/running rule and not a mistake.
   Update to 1 (owner, same day): after restarting the app the third account shows as signed in.
   So the credentials were fine and the reader accepts them: the fault is that the usage state from
   before the sign-in is kept, and is not read again when the sign-in tab ends (or on Refresh).
6. **The Chat view's scrollbar cannot be clicked or dragged** (owner, 2026-11-26): the conversation
   scrolls by wheel only.
7. **The Chat view's "/model" button only opens the terminal drawer; nothing happens there** (owner,
   2026-11-26). It should type the command into the terminal so the CLI shows its model menu. To find
   out: whether the send is now refused by one of the review rounds' vetoes (unverified pane identity,
   no fresh completed turn, bracketed paste state) and fails silently, where a typed "/" from the
   composer goes through the same path. A refusal must at least say why.
8. **Compact and handover should offer the agent for the new chat** (owner, 2026-11-26). When a
   handover is opened in a new chat (CLI or Desktop, this account or another), show a choice of
   agent to start it with: the default (plain Claude) and the agents the target account has
   (`~/.claude/agents/*.md`, the project's `.claude/agents/`, plugin agents), passed as
   `claude --agent <name>` for the CLI. To find out: what the Desktop app accepts for this (it may take
   none, in which case the choice is CLI-only and says so), and whether the choice belongs in the
   handover editor before opening or in the "Open as…" menu. The default stays plain Claude.
9. **After a new terminal opens, the mouse pointer sometimes stays a text caret everywhere** (owner,
   2026-11-26), also outside the terminal. Not reproduced yet. To find out: whether the terminal
   widget sets the text cursor on hover and the reset on leave is skipped when the tab is created
   under the pointer (no enter/leave pair), and whether the app or the toolkit owns the cursor
   choice each frame. Possibly related to the toolkit's cursor-shape item already noted there.
10. **Quick access to common slash commands in the Chat view, and CLI menus answered from the chat**
    (owner, 2026-11-26). Mock-ups: https://claude.ai/artifact/CVdwHaufYDmg86uvapfgYN (private to the
    owner). A: the hint line under the message box becomes chips (Model, Effort, Context, Compact,
    Clear, "/ …"); a chip with choices applies the choice directly (`/model sonnet`) and updates when
    the CLI confirms. B: typing "/" opens a list of built-in commands, the account's skills and plugins
    and the user's own, with pinned ones first and argument choices inline. C: a selection menu or
    permission question the CLI draws in the terminal is shown as a card of buttons; a tap sends the
    option's keys, and nothing is sent if the screen changed meanwhile. D: Settings > Chat > Quick
    commands (which chips, order, own commands, the switch for C). Replaces the single "/model" button
    (item 7). To find out: which commands take their argument on the line (`/model <name>` does;
    `/effort` to check), and how reliably the menus can be read from the terminal screen.
11. **A clipboard picture can never be sent from the Chat view, and the reason disappears** (owner,
    2026-11-26, with screenshots). Cause found: the second review (`706be9a`) made `chat_send` refuse
    every draft holding a `Chip::Clipboard` ("Open the terminal to attach and preview…") and open the
    drawer, so the picture paste the Chat view was built for cannot work. The refusal is in
    `chat.error`, which every keystroke clears (`chat_key`, `chat_text`), so the message and "Send
    anyway" vanish as soon as the user types; Send looks disabled with no reason shown. Also against
    the usual chat conventions: the picture is a text chip above the box instead of a thumbnail inside
    it, and a draft sent during a running turn is refused instead of queued (the CLI queues it
    itself). Proposed: send the picture as Alt+V, wait until the CLI's input line shows the
    `[Image #n]` marker, then paste the text and Enter (never both at once, as DESIGN.md asks); keep a
    refusal on screen until its cause changes; a thumbnail inside the box; queue during a turn.
    Picked (owner, same day): A (chips) and B (the "/" list). C and D are not decided.
12. **Chat view text and reading** (owner, 2026-11-26), drawn in artboard E of the mock-ups above:
    - The composer breaks lines inside words ("fo|rce"): wrap at word boundaries, and hyphenate only a
      word longer than the line.
    - Markdown files named in Claude's answers (links and paths such as `docs/TODO.md`) open with one
      click in a side panel through the toolkit's Markdown viewer, with "Open in editor"; files Claude
      wrote or edited in the turn also get a file chip under the answer.
    - Claude's prose and its tool calls look too alike: prose in the body font at reading size with the
      Claude mark, tool calls as compact one-line mono rows on a darker ground (status, tool name,
      target, duration), expandable, indented under the answer they belong to.
    - The composer per item 11: picture thumbnail inside the box, the waiting notice kept as a bar with
      "Send now" and "Cancel", and Send becoming "Queue" during a turn.
