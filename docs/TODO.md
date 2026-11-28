# Where this stands and what is next

Written 2026-11-26, when work was stopped for the night. Read `DESIGN.md` first.

## State

- `crates/core`: session index, launch safeguards and incremental transcript reader. Real-data and
  manual timing checks are ignored by the ordinary workspace test run.
- `bins/claude-switch`: terminal tabs, session view and clipboard/file attachment input.
- The toolkit (`markdown-shared-rs`) is a git dependency pinned to a revision of its `main`; raise the
  revision in `bins/claude-switch/Cargo.toml` to take a newer toolkit. A new pin must also be audited
  and added to `$reviewedGit` in `tools/write-third-party-notices.ps1` (licence still MIT, no new
  embedded files in the crates Switch ships), or the release run stops at "Unaudited git source"
  after the build, as 0.1.2's first run did. The script needs PowerShell 7 (`pwsh`).
- Released: 0.1.2 on 2026-11-27 (tag `v0.1.2`, items 22, 47, 49, 53, 56, 59, 61, 63-72 and the Chat
  cards) and 0.1.3 the same day (tag `v0.1.3`, items 73-77 and the Chat send gate rebuilt from the
  screen; CHANGELOG.md). Toolkit work is finished for now (owner, 2026-11-27): item 32 (fonts) and
  item 62's single-opacity option wait for it. OpenCode tabs (59) were tried by the owner and run
  well; nearly everything else in 0.1.2 and 0.1.3 has not been seen in the running app.
- Build: `cargo build -p claude-switch`.
- Hidden flags for trying things: `--demo`, `--theme light|dark`, `--shell <program>`, `--type <text>`,
  `--sessions-tab`.

- In 0.1.3 (2026-11-27): Chat's readiness from the screen, `TERM` for Claude tabs, held drafts, the
  question and permission cards from the screen, typed answers, denied permissions, the take-back
  after an early Stop (DESIGN.md, "What counts as ready", "Questions", "Renderer", "What still needs
  the terminal"). Checked by tests over screens recorded from Claude Code 2.1.332 and two review
  loops, never in the running app: the first things to try there are a plain send in a resumed
  tab, a send while a turn runs, Stop right after a send, a question from Claude, and a pasted
  picture (its marker was not recorded). A new Claude Code version is checked with
  `tools/cli-compat.ps1` (by hand) or the `cli-compat` workflow (needs an `ANTHROPIC_API_KEY` secret).

## Review later: the Chat redesign (released in 0.1.5 unreviewed)

The ten changes of the redesign study (branch `design/chat-redesign`, `docs/chat-redesign.md`) were
built by one agent (merge `d2deea0`) and released at the owner's word before a review (owner,
2026-11-28). Still to do:

- A correctness review of the branch (`main` b71f1a4 to `ee3500a`) for behaviour that changed beyond
  the look: sending, queueing, Send now, card answering, Stop, undo, prompt recall.
- The round "jump to latest" button covers the last line of text while scrolled up.
- Never tried in the running app or in a real session: the banner read on the empty state, the
  patch on real edits (diff line numbers), "Run in background" on the working line.
- Never drawn: a multi-question form, a plain CLI menu, a read-only panel, the rewind and
  blocked-calls cards, a card in a very short window; scales other than 100 % and 150 %; the new
  cards and panel in custom themes.
- Decided by the builder, to confirm: typing while a card is docked goes nowhere until the card's
  field is clicked; with a draft, Stop is an icon-only square beside Send; Chat's side padding grew
  12 to 28 px; code blocks sit about 40 px lower (header drawn in a spacer, the renderer is an
  external crate); the Session view keeps its strip and its tabs.
- Not updated: the README's screenshots, `docs/DESIGN.md`, this file's Chat items.

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
- When the terminal opens for a question (a card's "Open terminal", or Chat handing a question it
  cannot answer to the terminal), give the terminal keyboard focus so the answer can be typed at
  once (owner, 2026-11-28).
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

## Reported after 0.1.0 (owner, 2026-11-26)

Status of items 23 to 60 (2026-11-27, merged into `main` after two reviews by Claude Fable and the fixes
of both): built and passing format, lint and the test suite: 23 to 31, 34 to 48, 50 to 55, 57, 58 and 60.
Partly: 47 (no `!` toggle in the composer). 22, 53 (with Stop agent), 49 (rewind), 56 and 59 were built on
2026-11-27, 49 and 53 against recorded screens and 49 before a live check of 42 and 48 (see the items). Not built:
32 (later; fonts need toolkit work). Item 14 for Settings text rows waits for the toolkit pin move
(toolkit 7da7b22). Nothing was tried against a live Claude Code: the screen shapes the cards read (the
comment field after Tab, permission prompt phrases, full paths in Edit prompts, the /model picker, menu
footers, the Ctrl+B hint), the in-place handover's paste echo within 2 s, two real windows and the pipe
are guesses or unit-tested only; where a guess is wrong the card leaves the prompt to the terminal.
Known and left (owner to decide): a handover made mid-turn misses the rest of that turn; a draft typed
between `/clear` and the handover send makes the handover wait and then stop with the saved file.


Status after the fix rounds on branch `integrate` (2026-11-26; GPT-6.2 Sol workers, two reviews by
Claude Fable): items 1 to 13 and 15 to 21 are built and pass format, lint and the test suite; item 14
is built for the Chat composer only, the text fields belong to the toolkit (asked for in its session).
Nothing below was tried against a live Claude Code session, the real desktop app or a real install:
the owner's checks are what is left. To check live first: the empty-prompt and slash-echo detection
(items 7, 10, 11), the picture send (11), the desktop launch through the package alias (2), Stop via
the CLI's shell manager (17), the new chat's name via `--name` (21), and the installer's PATH task (19).
Item 10 C and D (CLI menus as cards, Quick commands settings) were built later as items 48 and 22.


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
    - Today every item is the same grey card with a small name label (screenshot, owner, same day):
      the user's own messages go right-aligned in a tinted bubble, Claude's answers without a card,
      tool calls as the rows above. The path highlighting also marks words that are not files (seen:
      "enter/leave" inside a Bash command); only paths that exist under the session's folder should
      become links.
    Approved (owner, same day): artboard E as drawn is the target for items 11 and 12.
13. **The Session view (the narrow side panel beside a terminal) gets the same conversation design**
    (owner, 2026-11-26, screenshot). Today it is the same grey cards as the Chat view, with tool calls
    as cards of their own ("Read …", "Bash …", "2 tool calls"). Apply artboard E at panel width: the
    user's messages tinted and right-aligned, Claude's answers without a card, tool calls as compact
    mono rows grouped under the answer, Markdown files opening in the viewer. Share the drawing code
    with the Chat view rather than keeping two looks; the panel keeps its own width and font size.
14. **Text in input boxes cannot be selected with the mouse, and there is no context menu** (owner,
    2026-11-26; seen in the Sessions search box). Expected everywhere text is typed (search, name
    dialogs, new session, settings fields, the Chat composer): click to place the caret, drag and
    Shift+click to select, double-click a word, triple-click all, and a right-click menu with Cut,
    Copy, Paste, Select all. The fields are the toolkit's `mdshared_ui::textfield::TextField` (and the
    composer's own editor), so the fix belongs in the toolkit where possible and then benefits every
    app; tell the toolkit session before building it here.
15. **The Details panel's Files list (and its Tasks and Agents rows) cannot be clicked** (owner,
    2026-11-26, screenshot): the rows have a ▸ but nothing expands or opens. A file row should open
    the file (Markdown in the viewer panel of item 12, other files with the system's default app,
    only files the transcript names), ▸ should expand its edits, and Tasks and Agents rows should
    expand to their detail.
16. **Show the CLI's live working line in the Chat view** (owner, 2026-11-26, screenshot of
    "✻ Shimmying… (53m 37s · ↓ 27.1k tokens)"). Today the Chat view reads that line only to tell
    that a turn is running (`view/chat/submission.rs`, "esc to interrupt"); it never shows it. Show it
    live at the foot of the conversation while a turn runs: the CLI's word, the elapsed time and the
    token count as the CLI prints them, updated as the terminal screen changes, with Stop beside it;
    gone when the turn ends. Read only from the visible screen rows; if the line cannot be parsed,
    show "Working…" with the elapsed time the app measures itself.
17. **Background shells stay hidden at the foot of the terminal** (owner, 2026-11-26, screenshot of
    "3 shells still running · ↓ to manage"). Proposed in artboard F of the mock-ups: a "Shells" tab in
    the Details panel with a count, one row per shell Claude started in the background (command,
    running time, status), a running row expands to its last output lines with Full output, Follow and
    Stop…; finished shells below with their exit code; a "3 shells running" chip under the message box
    opens the tab; tool rows of background commands carry a "background" mark. Source: the
    transcript's background Bash calls and the output files their results name (read-only, only files
    the transcript names). Stop asks first and goes through the CLI's own shell manager (the "↓ to
    manage" menu); the app never kills processes itself. Not approved yet.
    Approved (owner, same day): build F as drawn.
18. **Artifacts and other outputs reachable from the chat or the Details panel** (owner, 2026-11-26).
    Things a session produced outside the working folder should be one click away: claude.ai
    artifact links published in the session (from tool results and Claude's text), pull requests and
    other http(s) links Claude gave, pictures and files it saved. Proposed: an "Outputs" tab in the
    Details panel (beside Files) listing them newest first with their kind, title and time, each
    opening in the browser or the viewer; the same items as link cards under the answer that made
    them. Links open only as http(s) in the default browser, files under the same rules as item 12.
19. **Choose the new session's agent from the plugin and the command line** (owner, 2026-11-26).
    The plugin goes straight from the command to the new session, so the handover dialog's agent
    choice (item 8) is never shown, and `claude-switch compact` has no agent option. Add
    `--agent <name>` to `claude-switch compact` (same name rules and `--` placement as item 8), let the
    plugin take `agent:<name>` as the first word of its arguments (the rest stays literal focus text),
    and add a default agent in Settings › Handover used when none is named (default: none, plain
    Claude). Also found: the plugin called `claude-switch` from PATH, which the installer never sets;
    the plugin now falls back to the installed exe (commit 915ef71 on `integrate`); decide whether the
    installer should also offer to add itself to PATH.
20. **Up in the Chat composer should step back through earlier prompts, as in the CLI and Desktop**
    (owner, 2026-11-26: it does not). Code exists (`view/chat/input.rs`, `recall`) but only when the box
    is empty and only over prompts loaded from the transcript; find why it does not work for the
    owner (key routed elsewhere, the box not empty, history empty). Expected: Up on the first line
    (any text) steps to the previous prompt and keeps the unsent draft; Down past the newest brings
    the draft back; Escape leaves recall with the draft; prompts typed in the terminal count too;
    no duplicates in a row.
21. **A chat opened from a handover is named "Claude Switch handover"** (owner, 2026-11-26), every one
    of them. It should carry a name related to the session it came from, e.g. the source session's
    name (or its first prompt's summary) with " · continued" or a running number, editable in the
    handover dialog before opening. To find out: where the name comes from (the handover text's first
    heading becoming the session title, or the CLI's own naming), and whether the CLI takes a name on
    launch or the app should set it afterwards through Claude Switch's own session names.
22. **Later: Settings › Chat › Quick commands** (owner, 2026-11-26: "note D"), artboard D of the
    mock-ups: which chips show under the message box and in what order, the user's own commands with
    arguments, back to the defaults, and the switch for answering the CLI's menus from the chat
    (artboard C, not picked). **Done (2026-11-27):** a Chat page in Settings (after Sessions) with
    seven chip slots (dropdowns: a picked chip swaps, None hides), three own command lines as chips
    after the others (shown when they fit whole; a click runs the line with the direct-slash checks),
    Restore defaults, and "Answer menus from the chat" (off: no menu, picker, plan or permission card;
    questions keep theirs). Stored under `chat`; older files load the defaults, unknown chip keys and
    `chat` fields from newer versions are kept through merged saves. The mock-ups (artboards C and D)
    are not in the repository, so the page follows the existing Settings conventions instead of the
    drawing. Checked: unit tests for reading, merging, slot swaps, validation, the page, the toolbar
    order and fit, and the switch's gate. Not checked: a running app, a live Claude Code session, the
    owner's mock-up.
23. **Fresh chats refuse every send: "The terminal is showing a full-screen view"** (owner,
    2026-11-26, after 0.1.1). Likely cause, not yet confirmed on the owner's machine: Claude Code now
    starts new sessions in its fullscreen renderer (the `tui` setting, research preview; default for
    first use since 2026-05-06 / v2.1.239, per https://code.claude.com/docs/en/fullscreen.md), which
    draws in the terminal's alternate screen. The Chat view treats any alternate screen as a
    foreign full-screen program and vetoes sending (`view/chat/submission.rs`, `ALTERNATE`). Check
    first: `/tui` in such a session names the renderer. Options, not decided: (a) accept the
    alternate screen when the CLI's own input box and prompt are recognised there (the box is fixed
    at the bottom; footer and prompt rows to be captured from a real fullscreen session), keeping
    the veto for other full-screen programs; (b) start CLI tabs with
    `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1` (classic renderer) as a setting; (c) both, with (b) as
    a fallback. On Windows the docs also suggest `CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT=1` against
    stale text under ConPTY. Not started (owner: record first).
24. **A fresh chat refuses its first message: "The current session could not be verified"** (owner,
    2026-11-26, after switching that session to the classic renderer). Cause found in code, not yet
    seen live: a new session's tab is verified by matching the CLI's live session record to its
    transcript file (`view/mod.rs` `update_identity` → `view/model.rs` `discover`), and `discover`
    needs `<session-id>.jsonl` to exist; Claude Code writes that file only with the first prompt. So
    the Chat view refuses exactly the message that would create it: the first message of every new
    chat must be typed in the terminal. In fullscreen mode item 23's veto hid this. Fix direction, not
    decided: treat "live record names this tab's process and a valid new session ID, no transcript
    yet" as verified-for-the-first-send (pin that ID, then require the file to appear with it), keeping
    the refusal for a mismatching or missing record.
    Items 23 and 24: fix after the required toolkit pull requests are merged (owner, 2026-11-26).
25. **Automatic handover instead of Claude Code's own compaction, in Claude Switch's tabs only**
    (owner, 2026-11-26; decided):
    - Only for CLI tabs Claude Switch starts; nothing outside the app changes (no plugin hook, no
      global setting). Those tabs start the CLI with `DISABLE_AUTO_COMPACT=1` (per process; manual
      `/compact` still works) while the feature is on.
    - Threshold: by default the point where Claude Code itself would have compacted, read from the
      session's auto-compact window and percentage (`/autocompact`, `--autocompact`,
      `autoCompactWindow`, `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`, as far as they can be read); or a
      percentage the owner sets. Context size from the transcript, as "Context now" already does.
    - At the threshold, once the current turn has ended: make the handover with the configured
      summarizer and open the new chat in the SAME account only (Off / Ask first / Automatic).
    - The new chat must show as linked to the old one (a continuation in the session list and
      Threads, through the existing handover provenance), not as an unrelated session. A real
      `--fork-session` would carry the full context and defeat the purpose, so link, don't fork,
      unless a fork can start from the handover text alone. The old chat stays open, marked handed
      over.
    Docs checked 2026-11-26: env-vars, hooks (PreCompact can block but not replace), statusline.
    Changed (owner, 2026-11-27): the handover continues IN PLACE by default, in the same tab and CLI process: at an
    idle prompt, through the Chat view's gate, Switch sends `/clear`, waits for the CLI's new session id (item 36's
    following), then sends the handover text as the first message; the new transcript is linked to the old one as
    its continuation (thread), and the old one is marked handed over. The same applies to the manual handover
    (dialog, `/switch-compact`). Settings › Handover makes Switch's compaction configurable: where it continues
    (this tab / a new tab), the summarizer, the focus text default, the agent for a new tab, and the automatic
    trigger (Off / Ask first / Automatic, threshold). Option kept as an idea only, not to build: write Switch's
    summary into the transcript as a compaction record so the session id stays; undocumented format, may be
    rejected or misread by the CLI, and against the rule that the app never edits a transcript.
26. **The conversation cannot be copied: no right-click menu, Ctrl+C does nothing** (owner,
    2026-11-26), in the Chat view's conversation (and the Session view). Expected: select text with
    the mouse across messages, Ctrl+C copies, right-click gives Copy, Select all and, on a message,
    Copy message / Copy as Markdown; code blocks get a copy button. Partly depends on the toolkit
    text work (item 14's pull requests); do after those are merged.
27. **The message box is too small** (owner, 2026-11-26): make it the size Claude Desktop uses: about
    three lines tall when empty, growing with the text up to roughly 40% of the view, then
    scrolling; a drag handle to resize is optional.
28. **More dark and light themes** (owner, 2026-11-26), inspired by editors and IDEs: e.g. a few dark
    (neutral dark, deeper near-black, a warm dark, a cool blue-grey, high contrast) and light (paper
    white, warm light, cool light, high contrast) palettes, each with matching terminal colours,
    previewed as swatches in Settings › Appearance; the Claude accent stays available on every one.
    Check whether the toolkit already offers themes so other apps share them.
29. **Smooth, fast transitions in Settings when switching category pages** (owner, 2026-11-26),
    short (about 120-160 ms), respecting the system's reduced-motion setting and the app's own
    animation switch; ideally the toolkit's existing page animation.
30. **Settings › Handover › Default agent is a plain text box** (owner, 2026-11-26): make it a picker
    over the same agent list the handover dialog already shows ("Default (Claude)" first, then the
    account's, project's and plugins' agents), per account where the lists differ. Same work as item 37.
31. **The colour picker does not apply the chosen colour** (owner, 2026-11-26; same in another app,
    so probably the toolkit's picker): the selected colour's code is shown but not applied when
    clicking outside, and there is no confirm button. Expected: a clear Apply/OK and Cancel, Enter
    applies and Escape cancels, and clicking outside either applies (with the value visible) or is
    clearly a cancel; whichever, consistent with the toolkit's other popups. Report to the toolkit
    session; the app takes the fix through the pin.
    Toolkit status (toolkit session, 2026-11-26): TextField mouse selection and its menu are PR #55
    (`textfield::Pointer`, `FieldHit::cursor`, `TextField::context_menu`/`apply_context_action`,
    `selection_for_export`, `clicks::ClickCounter`); the colour picker fix (item 31) is in work on
    fix/colour-picker-confirm. Already on the toolkit's main at 137121c: `theme::Palette`, `tokens`
    (high contrast), `appearance` schemes (dark, light, warm, cool) for item 28, and
    `anim::PageTransition` plus `settings::pages::Form`'s `draw_animated` for item 29. No terminal
    colour schemes yet (toolkit roadmap T7). CS1's review fixes are fbfc5ee (safe to move the pin,
    page fades over the translucent terminal included).
32. **Font choices** (owner, 2026-11-26) in Settings › Appearance: the interface font, the
    conversation font, and the terminal and code font, each with a size, picked from the fonts
    installed on the system (monospaced ones only for the terminal and code), with a live preview
    and a "Default" entry. Check first what the toolkit's text stack can load (system fonts by
    family name, fallback for emoji and missing glyphs, the terminal's cell metrics after a change);
    whatever the toolkit lacks goes to the toolkit session.
    Toolkit status (2026-11-26): the terminal font (`termview::CellFont::with_family`, metrics
    re-measured from the font) and the conversation's family list (`gfx` rich layout, `richtext`)
    and emoji/glyph fallback exist at 137121c. New toolkit work, waiting on the owner's yes: a list
    of installed families with a monospace flag (`fonts::families()`), and a setter for the
    interface font (today private consts `UI_FAMILY`/`MONO_FAMILY` in gfx.rs; touches every app).
33. **"Open Desktop as <account>" shows the sign-in screen for the further accounts** (owner,
    2026-11-26); only the first account opens signed in. By design so far: a further account's
    Desktop runs as its own profile (`--user-data-dir=<desktop_dir>`, `launch.rs desktop_launch`),
    whose sign-in is separate from that account's CLI sign-in and is never copied. To find out: does
    a sign-in in that window complete and stay (then: say so in the app, "Sign in once in this
    window"), or does the browser's return to the desktop app land in the first account's running
    instance instead (then the profile can never be signed in while the first one runs, and Switch
    must say so or close and reopen in order)? Fonts (item 32): the toolkit queued FN1 (font list
    with monospace flag), FN2 (settable interface and mono font), FN3 (font picker row) with the
    owner's yes; build item 32 after they land.
    Tried by the owner (2026-11-26): the browser's return opens another desktop instance (the
    first profile), as suspected. With an email sign-in that instance shows a code, which can be
    pasted into the window Switch opened, and the further account then works. With Google sign-in
    the return signs in the other instance instead, so the further profile cannot be signed in that
    way. To do: in the further account's Desktop window, Switch says once: "Sign in with your email
    address (the same address as the Google account works), then paste the code shown in the other
    Claude window here". Check whether "Continue with email" for a Google-made account reaches the
    same account; and whether `claude://` can be routed to the right profile at all (likely not:
    the protocol handler always starts the default profile).
    Closed (owner, 2026-11-26): "Continue with email" with the Google account's address gives the
    code and reaches the same account; no further work needed.
34. **The Markdown side panel is too narrow** (owner, 2026-11-26): let it be wider: a draggable
    divider between conversation and panel (width remembered), and toolbar options for reading width
    as in the mdview app (narrow / medium / wide / full, or its exact set; reuse the toolkit's or
    mdview's control if it is shared). Also seen in the same screenshot: a bare `CHANGELOG.md` in an
    answer opened the session folder's file (kweb-mail's), while the answer meant another
    repository's; a relative name is resolved against the session's folder only. Consider showing
    the full path in the panel's title and, when the answer's recent tool calls name a file with that
    name elsewhere, preferring that one or offering both.
35. **Details panel: MCP tasks are missing, and every item should show its work** (owner,
    2026-11-26). The CLI's Background view lists MCP tasks (e.g. `opencode/opencode_run`, working,
    job id, running time, last tool); the Details panel shows none. Add them (a tab of their own or
    under Tasks/Agents), live, with status, running time and the last step. For every item in the
    panel (tasks, agents, shells, MCP tasks, outputs) the owner wants to open its work: an agent's
    conversation and tool calls (its sidechain transcript), a shell's output, an MCP task's progress
    and final result. First find where the CLI records MCP tasks (transcript records, the task
    output files, or only in memory; for the OpenCode bridge `opencode_result` could be read only
    through the CLI) and read what is on disk; never call a tool to get it.
36. **"Session changed in the terminal. Reopen the session to attach its transcript"** leaves the Chat
    view refusing to send ("could not be verified") until the session is reopened (owner's
    screenshot, 2026-11-26). When the change is the CLI's own (`/clear`, compaction, `/resume` in
    that tab), the live record names the new ID for the same process: follow it automatically and
    attach the new transcript (linked to the old one), and keep the refusal only for a change Switch
    cannot explain. Related to item 24.
37. **The default agent in Settings › Handover accepts any name** (owner, 2026-11-26). The owner set
    it to `deepseek-flash` meaning the model that writes the handover; the new chat then failed with
    `--agent 'deepseek-flash' not found. Available agents: claude, claude-code-guide, Explore, …`.
    Two settings read alike: the agent the new Claude Code chat starts as, and the OpenCode model
    that writes the summary (`model` in config, here `deepseek/deepseek-flash`). Make the default agent
    a list of the agents the target account really has (as the dialog's picker), with "None (plain
    Claude)" first; label the two rows plainly ("New chat starts as" / "Summary written by"); a saved
    name that no longer exists is shown as missing and not passed, with a notice, rather than failing
    the launch.
    Item 14 update (toolkit session, 2026-11-26): toolkit main `7fd559f` has it (PR #55, 906d7c7):
    `textfield::Pointer` (click, drag, Shift+click, double/triple click, edge scroll), `FieldHit::cursor`,
    `TextField::context_menu` / `apply_context_action`, `selection_for_export`, `MAX_PASTE_BYTES`
    (64 KiB), `clicks::ClickCounter` for the composer, and Windows clipboard writes through a real
    owner window (may explain clipboard failures here). Also PR #56 (54a7e84) glyph bounds. To finish
    item 14: move the pin from 137121c to 7fd559f and wire the search box, name dialogs, new session
    and settings fields to it. Waiting for the owner's go.

38. **Answer Claude's questions in the Chat view** (owner, 2026-11-26, screenshot: an AskUserQuestion
    menu "Who builds" with four options, "Type something" and "Chat about this", shown only in the
    terminal drawer). This is mock-up C, now wanted for questions. Today the drawer sometimes opens
    for a question and sometimes not, the conversation shows "Working… (3m 11s)" while Claude is
    waiting for the answer, and the composer says "Open the terminal to check". Wanted: the question
    as a card in the conversation, built from the transcript's AskUserQuestion tool call (header,
    question, options with descriptions, multi-select where set), not scraped from the screen; a
    click answers it by sending the option's keys only after the screen shows that same menu (fail
    closed, say why); "Type something" and "Chat about this" as a text field in the card; the working
    line says "Waiting for your answer" instead of Working; the drawer opens by itself only when the
    card cannot be answered from the chat. Permission questions stay terminal-only unless decided
    otherwise (DESIGN.md: no generated permission answers). Also in the screenshot: the conversation
    above was empty in a fresh handover session (check that the Chat view attaches to the new
    transcript).
    Forms done (2026-11-27): several questions and multi-select ones are answered from the card too
    (DESIGN.md, "Forms"): picks in the card, then one Submit that drives the CLI's form key by key
    and presses "Submit answers" only once the review matches. Checked: unit tests over screens
    recorded from Claude Code 2.1.332 (a two-question form with a multi-select first, its typed
    "Type something", the Next row, the second question and the review; one multi-select question
    with its Submit row and review; two single-choice questions and their review) and over a scripted
    terminal for the whole run and each way it stops. Not checked: the card has not been seen or
    clicked in the running app; a form has not been driven end to end by the app against a live CLI
    (the recorded screens were reached by hand-written key scripts); a typed answer on a
    single-choice question inside a form (its row with text is not recorded, so only that the text
    is on that row is checked); a multi-select as the last of several questions (its row is assumed
    to read "Submit", "Next" is accepted too); a typed answer long enough to wrap.
39. **A tab rename is a label only** (owner, 2026-11-26: renamed the toolkit session's tab to "toolkit"; search
    does not find it and other sessions cannot reach it by that name). "Rename tab…" (`tabcontrols.rs`, dialog in
    `dialogs.rs:549`) sets `Tab::rename` in memory only: it is not in `terminal-tabs.json` (lost on restart), not in
    the session marks (so the list and search do not know it), and Claude Code's live name for the session stays the
    derived one (`~/.claude/sessions/<pid>.json`: `"name":"anim-03","nameSource":"derived"`), which is what
    ListAgents and SendMessage use. Wanted: renaming a tab that runs a Claude session renames the session: the
    session mark (list and search), the saved tab, and Claude Code's own name through the CLI's `/rename <name>`,
    sent only through the Chat view's usual gate at an idle prompt (never into a running turn or a menu), so the
    transcript is still never edited by the app. Check first whether `/rename` updates the live name other sessions
    see (not verified); a plain terminal tab keeps a label-only rename.
40. **`/switch-handover:switch-compact` opened a new window when it finished** (owner, 2026-11-26). The skill runs
    `claude-switch compact --session <id> --open`; the new chat came up in a window of its own instead of a tab of the
    running Claude Switch. Wanted: when Claude Switch is running, the new chat opens as a tab in that window (the same
    account and, if the source session runs in one of its tabs, beside it), as the app's own handover does
    (`compact_open.rs` `open_handover_tab`); a new window only when Switch is not running, and then the Switch window,
    not a bare console. Find what `--open` launches today (`compact_cli.rs`, `compact_runner.rs`, `run.rs`
    `CREATE_NEW_CONSOLE`) and how a second process can hand the request to the running one.
41. **"Could not save session marks: marks.json changed on disk. Restart before saving marks."** (owner, 2026-11-27,
    screenshot). Cause seen: two Claude Switch processes ran at once, the owner's window and the one
    `compact --open` started (`--compact-open-window`, item 40); each holds its own copy of the marks and the second
    save is refused (`crates/core/src/marks.rs` `save`). Item 40 removes the usual cause; also make the save hold
    with two writers: under the existing lock, re-read the file and apply only this process's own changes (per
    session and field) onto it, refusing only a true conflict (both changed the same field) with a plain message; never
    ask for a restart. Same check for settings (`config.rs`: "Settings changed on disk").
42. **Answer permission prompts in the Chat view** (owner, 2026-11-27, decided: "give that up, otherwise all I see is
    that chat has stopped. convenience trumps"). This replaces DESIGN.md's rule that permission prompts are answered
    only in the terminal. Wanted: when the CLI asks to allow a tool (edit, bash, MCP, web fetch, plan approval and
    the like), the Chat view shows a card with the tool call from the transcript (command, file and diff, MCP tool
    and arguments) and the exact choices the CLI offers (Yes / Yes and don't ask again for ... / No, with the text
    field for telling Claude what to do instead), and the working line says "Waiting for your approval" instead of
    Working. Safety kept where it costs nothing: the card's choices come from the screen's own menu; a click sends
    keys only while the screen shows that same prompt for that same tool call (fail closed and say why, never a
    blind keystroke); the app never answers by itself, and nothing is pre-selected. Built on item 38's question
    card. Update DESIGN.md's rule with the owner's reason.
43. **The composer's Context and Effort chips are stale or empty** (owner, 2026-11-27, screenshot: "Effort —",
    "Context 275k" after a /compact, with "could not be verified" beside them, item 36).
    - Context: the figure comes from the last assistant turn's usage, so after a compaction it keeps the
      pre-compaction size until the next answer. On the transcript's compaction record (`compact_boundary`, with
      `compactMetadata.preTokens`; check the real shape in a transcript, read-only) show the size as unknown
      ("Context — after compact") or an estimate from the summary, and take the real figure from the next turn.
      Same in the Session view and wherever "Context now" shows. Also check that /compact in a tab does not change
      the session id (if it does, item 36 covers following it).
    - Effort: the chip only learns the level from an `/effort` reply in this session (`submission/report.rs`
      `reported_effort`), so a fresh session shows "—". Show the level the session started with, from the same
      sources the CLI uses, in its order (the `--effort` flag the tab was started with, `CLAUDE_CODE_EFFORT_LEVEL`
      or the like in the tab's environment, `effortLevel` in the account's and project's settings, else the model's
      default, marked as default); check the names in Claude Code's docs. Read-only, never written.
    - Seen 2026-11-27: `/effort` typed in the terminal ("Set effort level to medium (saved as your default for new
      sessions)") left the chip at "—". Cause in code: `observe_cli` reads reports only from the screen and only
      while `tracking`, which `CliReport::begin` turns on just for a command the Chat view itself sent; a command
      typed in the terminal is never read. The transcript holds the reply as a `<local-command-stdout>` record: read
      the newest "Set effort level to …" and "Set model to …" from the transcript, whoever typed the command, and
      keep the screen reading only for the chip's own confirmation.

Audit 2026-11-27 (read-only, cc-opus): what still sends the user to the terminal. Owner's rule for all of these: the Chat view must cover everything the CLI does; convenience over the old terminal-only rules (item 42). Each keeps the one safeguard: keys are sent only while the screen shows the matching prompt or menu, the choice is picked by its row text, and anything unexpected fails closed with a reason and an Open terminal button.

Additions to item 42 from the audit: fail closed for two pending tools at once, a background-subagent prompt, or rows
    that differ from the pending `tool_use` input; keep the dangerous-`rm` 2-minute countdown visible; the Tab comment
    field; find out whether number keys or arrows+Enter select. `attention.rs` already recognises the prompt layout.
44. **Plan approval card**: the ExitPlanMode plan from the transcript as Markdown with the CLI's options as buttons,
    under item 42's gate; an option is offered only when the screen shows it.
45. **Permission mode chip**: show the mode (footer `⏸ plan mode on`, `⏵⏵ accept edits on`, or a transcript field if
    one exists); a click sends one Shift+Tab (`CSI Z`) at a verified idle empty prompt, the next only after the
    indicator changed; unreadable indicator stops.
46. **Keep line breaks in messages**: today the send turns line feeds and tabs into spaces. Paste with line feeds in
    the bracketed paste and press Enter only after the input row shows the text or its `[Pasted text #n +N lines]`
    marker, as the picture send does.
47. **Command results and shell mode in the conversation**: `<local-command-stdout>` and `<bash-input>`/`<bash-stdout>`
    records as mono cards under their command (filtered out today, `prompt_clean.rs`); interactive panels get a
    card; a `!` toggle in the composer once shell mode by paste is verified.
    **Done except the `!` toggle** (2026-11-27, branch `fix/items-47-tools`). The records were already read into
    finished rows (`transcript/local.rs`); this pass gave them item 71's quiet look. One-line replies (at most 140
    characters, which covers the shapes found read-only in this machine's transcripts: "Set model to …", "Goal
    set: …", "Settings dialog dismissed", "Reconnected to …") sit on their command's row. Longer output
    (`/context`) and shell output are muted mono cards under the row. They now get their real height (before,
    an always-open output or plan card kept a one-line height and was drawn over the rows below) and fold on a
    click. A shell output's first-line indent is kept (`git status --short`). An empty `<local-command-stdout>`
    no longer leaves its command's name to label the next output. Real transcripts also hold shell mode as
    one record with two text blocks (`<bash-input>` then `<bash-stdout>`). The reader stopped at the first
    block and lost the output; it now reads every block. Checked: parser, reader and row unit tests (the
    shapes copied from real records), plus a bitmap test with `/model`, `/context` and two shell commands
    (one failing) in light and dark, whose pictures were looked at. Not built: the `!` toggle. Shell mode
    by paste was not checked against a real Claude Code: that needs an interactive CLI session driven
    through a terminal, which this pass did not start. Nothing was seen in the running app.
48. **CLI menus and dialogs as cards**: any menu, picker or startup dialog the CLI shows (the native /model and
    /effort pickers, /config, /permissions, /mcp, folder trust, the fullscreen offer, settings errors, MCP sign-in
    and elicitation, the rate-limit menu) is copied from the screen into a card with its rows as buttons, answered
    under item 42's gate; a menu Switch cannot read shows its rows read-only with Open terminal. Replaces the
    DESIGN.md rule that menus stay in the drawer.
49. **Rewind from a prompt bubble**: "Rewind to here…" on user bubbles; Esc Esc at a verified empty prompt, Up until
    the highlighted row equals the chosen prompt, the action chosen by its row text, every step re-verified; build
    after 42 and 48 have proven the gate.
    Built (2026-11-27) before 42 and 48 were proven live: both are built and unit-tested, but neither card has been
    driven against a live CLI by the app, so this one uses the same verified-gate rules without that proof. "Rewind
    to here…" is in the conversation's right-click menu over a typed prompt (DESIGN.md, "Rewind"). Checked: unit
    tests over screens recorded from Claude Code 2.1.332 in a scratch folder (Haiku, 100 x 40): the idle prompt, the
    list opening on "(current)", each Up, the choices without and with code changes (and the highlight moved down),
    the input holding the prompt after "Restore conversation", Esc from the choices back to the list and from the
    list back to the prompt, a prompt of several lines and a long one as the list cuts them; Up at the top changes
    nothing (recorded). Not checked: the menu item and the card have not been seen or clicked in the running app; a
    list long enough to scroll (the one-row scroll rule is untested); what "Summarize from here", "Summarize up to
    here" and "Never mind" lead to (each is done once the rewind screens are gone); prompts with pictures or pasted
    text markers in the list. Left: the Chat view keeps showing the rewound turns, since the CLI appends nothing on
    a rewind and the reader does not follow `parentUuid` branches.
    Review fixes (2026-11-27): every key on the choices needs them to quote the chosen prompt (the CLI could have
    been moved to another prompt whose choices read the same), and Enter on the list needs the rows under the chosen
    prompt to be the later prompts one for one (slash commands, which the list does not show, left out), so a list
    that leaves a prompt out stops instead of taking an earlier prompt worded alike.
    A key held back (a dialog over the window) no longer waits forever: the step's deadline still holds, and a second
    Esc that cannot go within about 0.75 s of the first stops the rewind rather than going late.
    "Rewind to here…" is offered only on prompts the list shows (not on slash commands such as `/context`, which
    Claude Code 2.1.332 leaves out of it). A rewind that stops with its list on screen closes it with one checked Esc
    and says so, or says the rewind screen is still open (its choices are on screen, or no key could go). A question,
    permission prompt or CLI menu takes the card from a stopped rewind, whose notice comes back afterwards.
    Re-review (2026-11-27): a rewind that stopped with its screen still open keeps its card (why it stopped, and
    that the screen is open) over the generic panel card while that screen shows, and goes once it is gone.
    A Cancel that times out with the list still on screen sends no closing Esc on top of its own; it says the
    screen is still open.
50. **Thinking, hook and memory notices**: thinking blocks collapsed as "Thought" where readable; hook errors,
    blocked prompts, stop-hook feedback, memory notices and session recaps as small notice rows from their system
    records (catalogue the subtypes the current CLI writes first).
51. **TaskCreate/TaskUpdate in the Tasks tab** beside TodoWrite, reusing `handover/native_tasks.rs`.
52. **Composer conveniences**: `@` file autocomplete over the session folder; Ctrl+R search over earlier prompts; the
    CLI's grey next-prompt suggestion as a chip that fills the composer (never sends Tab).
53. **Background controls**: "Run in background" on a running Bash row (Ctrl+B, only while the CLI shows its hint and
    that tool alone is pending); "Stop agent" through the CLI's agent panel under the same rules.
    Done (2026-11-27): Run in background now needs the CLI's "(ctrl+b to run in background)" row under that call's own
    `Bash(command)` row and ends on "Running in the background" or the call's end; Stop agent is a card row per
    running background agent, driven ↓, ↓ to the agent, `x`, Esc (DESIGN.md, "Background controls"). Checked: unit
    tests over screens recorded from Claude Code 2.1.332 in a scratch folder (Haiku, 100 x 40): the Ctrl+B hint and the
    backgrounded row; two background agents in one run through idle, list open, each highlight, stopped and closed;
    the list mid-turn beside a running shell command; Esc in the list does not interrupt a running turn (recorded,
    not a test). Not checked: the card has not been seen or clicked in the running app; PowerShell's row (assumed
    `PowerShell(command)`); an agent whose description the list cuts with "…" (not offered); how a finished agent's
    row reads (not offered); offering a second stop while the hint row reads "↓ to resume subagent" (not offered);
    the sidecar `.meta.json` of 2.1.332 has no `finished` field (it has `stoppedByUser`), so the transcript never
    knows an agent finished and the screen decides.
    Review fix (2026-11-27): terminal input, Cancel, or switching the tab to the terminal stops any card key sequence
    in flight (an answer, a form, Ctrl+B, Stop agent, a rewind) before its next key, with a notice, instead of letting
    it resume later from a screen it no longer knows.
    Re-review (2026-11-27): only an answer stopped before its Enter marks its prompt as one for the terminal; an
    interrupted Ctrl+B or Stop agent leaves a notice only (it goes when the card shows something else, so a later
    permission prompt is still answered from the card), an answer whose Enter went ends quietly, and an interrupted
    rewind keeps its notice on its own card. Mouse moves without a button no longer count as terminal input (they stopped a
    rewind or form when the pointer crossed the drawer with mouse tracking on), and wheel reports no longer stop card
    key sequences; clicks still do, since they can pick a row in the CLI.
54. **Esc for more than Stop**: Stop's once-per-turn Esc rule is kept for Stop; Esc for leaving a menu or declining a
    prompt goes through item 42/48's gate.
55. **The working line jumps sideways as its spinner animates** (owner, 2026-11-27, screenshot "✢ Billowing… (13s ·
    ↓ 1.1k tokens)"). The CLI's spinner glyphs (· ✢ ✳ ✶ ✻ ✽) have different widths in the interface font, so the
    word after them moves with each frame; the counters (seconds, tokens) shift the rest too. Draw the glyph in a
    fixed-width slot (the widest glyph of the set, centred) and start the text at a fixed x; use tabular figures, or a
    fixed slot per counter, so changing digits do not move what follows (`view/working.rs`, and the Session view's
    copy of the line).
56. **Done (branch `fix/items-56-59`): search and group sessions by meaning.** Settings › Sessions › Search by
    meaning (off by default): the model is Claude Haiku via the CLI on the workers' account (Suggest names' isolated
    run) or an OpenCode model (the handover summarizer's isolated run, its own tool-less agent); List models fills
    the choice. Palette: "Summarize sessions for search by meaning…" (confirmed; sends Suggest names' payload for up
    to 100 visible, not hidden, stale sessions in requests of 20; keeps summary, keywords and subject per session
    in `meaning.json`, re-sent only when the prompts change) and "Ask about past sessions…" (confirmed; sends the
    question and the kept summaries only; the answer is kept and repeated without a request). With it on, the
    list's search also matches summaries locally, Details shows "About", and Topics gets a "By meaning" grouping
    by subject across projects. Everything runs on a worker job with Cancel; DESIGN.md, Session organization.
    Checked: core tests with a fake model (payload identical to Suggest names', batching, unchanged sessions not
    re-sent, partial results kept on failure and cancel, answer parsing and cache, local matching of "where we
    fixed push"), window tests on the demo source (off by default, confirmation text names model and data, index
    → search → Topics by subject, question → kept answer without a second request, Cancel), the OpenCode search
    agent's config, fmt, clippy and the workspace tests. Not checked: any real Claude or OpenCode request (none was
    made during development), answer quality of a real model, the dialogs and the Settings rows by eye in the
    running app, and timing with a large `meaning.json`.
    Review fixes (`fix/review-meaning`): the subjects sent with a Summarize request now come only from visible, not
    hidden sessions (they came from every kept summary); sessions with no readable prompt or no answer get a
    marker, so they are no longer re-sent and offered on every run and "Nothing to summarize" is reached.
    Second review fix: a request whose answer parses to nothing (prose, a refusal, cut-off output) now fails the
    run and marks none of its sessions, and a "not answered" marker is retried when another model is chosen.
    Original request: **Later: search and group sessions by meaning** (owner, 2026-11-27: "agent selectable like compact in settings.
    later item. not now."). Today search is literal text and Related is word overlap (DESIGN.md, Session
    organization). Wanted: a search that finds a session by what it was about ("where we fixed push"), questions
    about past sessions, and grouping by meaning across projects. Done by a model the owner picks in Settings, the
    same way as the handover's summarizer (an OpenCode worker/model or the Claude CLI), opt-in, sending only the
    excerpts "Suggest names" already sends, with a confirmation before a run that sends anything. An on-device
    embedding model was the alternative offered; not chosen.
57. **Better handovers: a fitting focus chosen by itself, and a sharper fixed format** (owner, 2026-11-27, from a
    cc-fable review of six real handovers: sections cut mid-word at 800 characters, "Standing instructions" turned
    into a to-do list, a permission guessed ("presumably approved"), job and session ids missing, space spent on
    finished history, the focus echoed back, rarely who decided).
    - Focus chosen by itself, not presets (owner): Switch picks the focus that fits the session from the transcript,
      locally and without a model: a coordination session (most tool calls hand work to workers or agents, few
      edits of its own), non-code work (no edits of code files; e.g. subtitle or CAD tools and files), else the
      default. The dialog shows which kind was picked and the text, editable; the user's own focus text replaces it;
      Settings › Handover lets each of the three texts be edited and reset. The three texts, verbatim:
      - Default: "The reader continues this work unattended and remembers nothing. Put the newest first, so anything cut off is the oldest; leave out finished history. stands: each background job, worker or review still running or unread, with its job/session id, worktree, branch and last commit; uncommitted or unpushed work; where a merge stopped. decisions: who decided (user or assistant) and why; mark them settled. instructions: only rules still in force, in their latest form; no next steps; never assume a permission was given. open: promised follow-ups with their trigger; recorded requests not yet done, by number. Do not restate this focus."
      - Coordination: "This session coordinates workers and writes no code itself. stands: one line per worker or job: id, model, worktree, branch, brief file, state (running, finished unread, reviewed, merged) and what was promised when it reports; the merge order and where it stopped. instructions: which model does which kind of work, as last set, and other rules still in force. Leave out what the workers changed in the code. Newest first. Do not restate this focus."
      - Non-code: "No code here. stands: the file or job in hand, how far it got (episode, line, part), what was applied and what was only proposed. decisions: terminology, style and guideline choices with their reason, so they stay consistent. instructions: rules still in force, such as what may be edited and what is read-only. open: checks and corrections still owed. Newest first. Do not restate this focus."
    - Fixed prompt and sections (`crates/core/src/compact.rs`): a seventh field "In flight and promised" (jobs,
      workers, reviews with id, worktree, branch, state; "when X reports, do Y"); "instructions" defined as rules
      only; the cut at about 1,500 characters on a line boundary with a visible `[cut]` (still asking for 800); tell
      the summarizer it sees assistant text only and the user's prompts are appended, and never to infer that a
      permission was given or a rule lifted; when partial summaries conflict the later wins and items a later part
      shows done are dropped; Files leaves out temp and scratchpad paths and keeps about the 30 most recent; the
      task list is marked as possibly stale.
58. **Pictures in the conversation as thumbnails, as in Claude Desktop** (owner, 2026-11-27, screenshot: a tool
    result's picture shows only as a bar "Picture · Tool result picture 1"). Show every picture in the conversation
    (the user's pasted pictures, pictures in tool results such as screenshots and Read of an image) as an inline
    thumbnail at a bounded size (about 240 px on the long side, aspect kept), decoded off the window thread and
    cached, with the label as its caption; a click opens it enlarged (the existing viewer). Same in the Session
    view. Keep a placeholder with the reason when a picture cannot be decoded or is over the size limit.
    Other embeds (owner: "make it clear what other embeds are"): every non-text item in the conversation says what
    it is at a glance, with an icon and a plain kind label instead of a generic bar: PDF or document (name, pages
    if known, Open), file attachment (name, size, Open / Show in folder), link or artifact (title and host, Open),
    pull request (repo and number), saved output file, Markdown file (opens the side panel), audio or other binary
    (kind and size). Where a cheap preview exists (a picture, the first page of a PDF if the toolkit can render it),
    show it as the thumbnail; otherwise the icon. The Details panel's Outputs uses the same labels.
59. **Done (branch `fix/items-56-59`, owner: "full 59"): OpenCode as a tab type and as a handover target.** Palette
    "New OpenCode tab…" (folder-only picker) starts the owner's normal OpenCode (own config, plugins, Magic Context;
    Claude Code variables stripped) as a terminal tab with its own label and glyph and no account, transcript, live
    record, send gate or Chat view. The handover preview's "In OpenCode" opens one in the source folder beside the
    source tab with the handover as `--prompt` (checked: `opencode --help`, v2.4.1, `opencode [flags] [<directory>]`,
    `--prompt`, `--continue`); long or option-like text becomes a request to read the saved handover file. Restore:
    running OpenCode tabs are saved by folder and reopen with `--continue` (the folder's last OpenCode session; the
    first prompt is never repeated). DESIGN.md, Terminal tabs. Checked: unit tests for the launch (args order, no
    OpenCode overrides, Claude variables and PWD removed), first-prompt rules, the folder-only dialog opening a
    non-Claude tab, placement beside the source tab with no marks written, and save/decode/restore of OpenCode
    rows beside Claude rows; fmt, clippy, workspace tests. Not checked: starting the real OpenCode TUI in a tab
    (only `--help` was run, in a scratch folder), whether `--prompt` submits at once or waits for Enter, how
    `--continue` behaves in a folder with no OpenCode session, the button and dialog by eye, and the `compact
    --open` CLI and auto-handover, which still continue only in Claude.
    Review fix (`fix/review-meaning`): two OpenCode tabs of one folder both reopened `--continue` onto the same
    session; now only the first per folder does, the others start fresh (OpenCode's session id could not be learned
    without a model request; see DESIGN.md). A never-used tab that is first in its folder still reopens the
    folder's last session.
    Original idea: **Maybe, idea only: OpenCode as a tab type, and as a handover target** (owner, 2026-11-27: "opencode makes more
    sense as a tab ... leave this as a maybe idea"). A "New OpenCode tab" entry with the folder picker, running the
    owner's normal OpenCode (Magic Context enabled, unlike the bridge's jobs), and "Continue in OpenCode" sending the
    handover text as its first prompt, e.g. when a Claude account hits its limit. Codex CLI support was considered
    and not chosen: every layer of the app is Claude Code specific (accounts, transcripts, live records, the send
    gate's screen rules). Decided 2026-11-27: build it all.
    **Design proposal (2026-11-27; built as above, except restore, which reopens with `--continue`).**
    - *New OpenCode tab*: a palette and New-menu entry that opens the existing New session folder picker without
      the account choice, then starts a tab like "New shell tab" does: `launch::shell`'s sanitized environment
      (Claude Code variables stripped) with `opencode_cli::find_program()` as the program and the folder as cwd.
      No `OPENCODE_CONFIG_CONTENT`, `OPENCODE_DISABLE_CLAUDE_CODE` or project-config override, so the owner's normal
      config, plugins and Magic Context apply; label "OpenCode · <folder>"; ended tabs show "(ended)" as shells do.
    - *What a non-Claude tab must skip* (most of it already does for shell tabs, which have no session id):
      accounts (no account picker, colour, usage or "Open as"; OpenCode has its own providers); transcripts and
      the session index (OpenCode keeps sessions in its own store, so the tab is never a row in Recent, Threads,
      Related or search, and cannot be resumed, copied or archived); live records and `running.rs` (no
      already-running safeguard; closing the tab is the only stop); the send gate and the Chat view (both read
      Claude Code's screen and transcript; the tab is terminal-only, the Chat/Session view toggle disabled);
      handover, auto-handover and the Details panel's Tasks/Files/Agents. Tab restore would reopen only "OpenCode in
      <folder>" (a fresh start, not the conversation) or nothing; the latter is simpler. The work is one tab-kind
      flag checked where tabs with a session id are assumed today, not a new abstraction layer.
    - *Continue in OpenCode*: a third choice beside This tab / A new tab in the handover preview. It needs the TUI's
      first-prompt flag (`opencode <folder> --prompt <text>`, to be confirmed with `opencode --help` on this
      computer before building), the same long-text rule as Claude (above ~15,000 UTF-16 units pass a short prompt
      that names the saved handover file), and optionally `--model` from a picker. No provenance in `marks.json`
      (there is no Claude session to link), and the Claude source session is left as is.
    - *Effort*: the tab about a day (launch, menu entry, guarding Chat view/Details/restore against the new kind,
      tests); Continue in OpenCode another half day once the prompt flag is verified. Risk is low because shell
      tabs already prove the non-Claude path; the main cost is auditing every `session`-assuming code path.
    - *Recommendation*: build the plain OpenCode tab first, terminal-only, no restore, as the owner's fallback when a
      Claude account hits its limit; add Continue in OpenCode only after the prompt flag is confirmed, and do not
      try to index or chat-view OpenCode sessions (that would be the Codex-sized rewrite already rejected).
60. **Clearer names for the two archives** (owner, 2026-11-27: yes). Settings › Sessions "Show archived sessions"
    becomes "Show sessions archived in the desktop app" (help: "Claude Desktop's own archive. Switch's Archive is
    separate."); the Archived chip becomes "Archived here"; the details row "Archived: yes, in the desktop app"
    becomes "Desktop archive: yes", so "Archived" alone always means Switch's archive. Update README, DESIGN and
    the wiki's Sessions and Settings pages.
61. **`/switch-compact` blocks the chat, and its report sets the agent going again** (owner, 2026-11-27). Compact and
    open should run in the background: the skill starts `compact --open` and ends its turn at once, and the result
    (the new tab, or the failure) is shown by Switch, not reported back into the old chat as a tool result the agent
    then acts on. The old session must not resume its tasks because a handover finished. Seen with it: "claude-switch
    compact: Could not read the transcript." on a 112 MB transcript. Cause: the `claude-switch` on PATH was built
    before `a6d6b84` (64 MiB limit, now 2 GiB); current `main` reads that transcript. The CLI and the automatic
    handover now say why a read failed instead of dropping the reason.
    **Done** (2026-11-27, branch `fix/item-61`): `compact --open --background`, which the skill now runs, does the
    quick checks (consent, OpenCode, session, account), reads the focus file, writes an app-named job file to the
    temp folder and starts `compact --background-job <name>` through the window's fixed ShellExecute bootstrap (no
    console, no inherited handles), then prints one line and exits. The detached process revalidates the job with
    the same parser and runs `compact --open`; the result is the tab or the continuation in place, a failure is a
    notice in the running window (a new pipe request kind that only shows text) or a message box when no window
    takes it. The window now also shows a handover request it could not load. The skill tells the agent to report
    the one line and end its turn, not wait, poll, retry or resume. Owner's decisions (2026-11-27), built: a
    dropped Settings default agent is an info notice in the window (same pipe request kind, a note version); no
    fallback to the blocking call; each window start removes job files over a day old (job name pattern, plain
    files only). Checked: format, lint, tests (job and notice encoding and revalidation, both notice kinds, job
    file read once and removed, cleanup keeps other names, folders and new files, option rules). Not checked: a real `/switch-compact`
    run; that the detached process survives the end of Claude Code's Bash call on Windows (the same bootstrap
    already starts the window from the skill); the message box; the window's notices on screen; the cleanup on a real temp folder. Left: an old
    `claude-switch` on `PATH` refuses `--background` (no fallback, by decision).
    Review fixes (2026-11-27): the window answers `shown:` only for errors it put on screen and `failed:` for the
    rest (its own wait ran out, closing, unreadable answer), which the background process then reports; a queued
    request carries a ticket, so after the 30-second wait it is cancelled when the window has not taken it (the
    answer says it was not opened) and otherwise the answer says the outcome is unknown and the tab may still
    open; a launcher that runs out of time keeps the job and says the handover may have started; a job over
    256 KiB is refused before starting. Tested: reply kinds, ticket, the wait's four outcomes, launcher result,
    job size.
62. **Dark text in a transparent terminal gets a dark outline** (owner, 2026-11-27, two screenshots: the grey
    tool-result rows such as "Shell cwd was reset to ..." and the dim session-name rule "═══ Claude-switch handover
    file · continued ═══"). Selection is fine. With terminal opacity below 100 % dim and dark glyphs show a dark
    fringe around each letter, as if their edges were blended against black (or against an opaque cell background)
    and then laid over the backdrop. Glyph edges must blend with the real background colour (premultiplied alpha,
    grayscale rather than ClearType antialiasing over transparency), and a cell with the default background must not
    paint an opaque box behind its text.
    **App-side fix done; toolkit change outstanding.** Translucent frames now request grayscale font
    masks, including on cells promoted to opaque. Bitmap tests composite grey glyphs over black
    and white. Not checked against the owner's desktop. At e951766, `mdshared-text` already uses
    premultiplied source-over. `mdshared-ui::termview::draw::Background::alpha` calls `for_ink`
    for each distinct foreground, sometimes returning 255 beside translucent spaces. Expose a
    uniform-default-background option on `BackgroundOpacity`: use `normal` on every default cell,
    retaining 255 for selection, block cursor, inverse and explicit backgrounds. Use that option
    here when available. No toolkit or pin change.
    **Later (owner, 2026-11-27: "put 62 on for later"):** the toolkit option above is parked; ask the
    toolkit session for it when this comes back, then move the pin and use it here.
63. **Settings › Appearance: flexible, customizable colours and theming, and one kind of control per choice**
    (owner, 2026-11-27, screenshot of Appearance; "more flexible, customizable color and theming"). Today the dark
    palette is chosen with swatches and a dropdown ("Neutral") side by side, the light palette with swatches and a
    segmented row (Paper, Warm, Cool, High contrast), other rows use segmented rows or switches. Pick one control per
    kind of choice across all Settings pages and menus (swatches with a name under them for palettes, segmented rows
    for two to four plain options, switches for on/off) and make toggles and selections look the same in menus.
    Customizing: own themes the user can make, name, save and switch between, with the colours that matter editable
    (window background, surfaces, text, dim text, accent, selection, and the terminal's 16 ANSI colours and its
    background and foreground), starting from any built-in palette or generated from a base colour and an accent;
    contrast checked as the user edits; import and export of a theme as a file.
    **Done.** Named palette swatches on Appearance and Terminal, a draft editor for both colour
    modes, generation from background and accent, contrast feedback, save/rename/copy, and
    versioned theme files. Short plain choices use segments; boolean menu marks for pinned/hidden
    use the Settings switch. Tests cover draft isolation, window and ANSI colours, bitmap rendering,
    pointer and keyboard selection, file round trips and migration with stale writers and unknown
    settings. Native file picker interaction and the owner's live desktop were not checked.
    **Known mixed-version limit:** If two Switch versions share a settings file, a version from
    before named themes replaces the whole `appearance` object when it saves appearance changes.
    This drops `appearance.themes`. A `darkTheme` such as `user-1` survives, but without its
    palette definition Switch falls back to a built-in.
64. **A scrollbar that fades, and page width options** (owner, 2026-11-27). Scrollbars show while scrolling or
    under the pointer and fade out after a moment (Windows 11 style). A page width setting for the Chat view and
    the Markdown panel: full width, or a readable column (e.g. narrow, medium, wide). **Done:** the Chat
    conversation has its own width in Settings → Sessions; the Markdown viewer keeps its existing width
    control. Both scrollbars show on movement or hover and fade, or snap away with reduced motion.
    Rendering and timing tests cover the layouts; visual inspection in a running app has not been checked.
65. **Done: Windows Terminal mouse gestures for terminal tabs and the Chat drawer.** Arrow pointer, drag to
    select, and right-click to copy and clear the selection or paste safely. A blank selection clears without
    reading or writing the clipboard (covered by a test). Unmodified clicks and drags go to
    a program that asks for mouse reports; Shift keeps the gestures local. Checked against Windows Terminal 1.25
    source and with tests for pointer choice, selection, reporting and paste safety. Native clipboard gestures
    and Claude Code's own fullscreen auto-copy were not checked interactively. Switch does not handle clipboard
    writes via OSC 52. If application-driven OSC 52 copy is wanted, the toolkit needs a bounded parser event
    for validated, decoded clipboard writes (no clipboard reads); the owner should decide when Switch may honor it.
66. **A better design for the composer's chips** (owner, 2026-11-27): Model, Effort, Context, Compact, Clear, the
    slash menu, Attach, Terminal and Details look like the same flat button and their labels are cut ("Effort
    mediu…"). Group them by kind (state the session is in: model, effort, context; actions: compact, clear;
    panels: terminal, details), show a value as a value (an effort level as a small meter or dots, context as a
    filling bar), never cut a label, and keep the row from jumping when a value changes. Item 43 covers the values
    being stale; this is the look. **Done:** distinct state, action and panel surfaces, fixed-width values,
    effort dots and two-row layout at narrow widths. Context remains a token count: no verified limit is
    reported, so a filling bar would mislead. Owner question: should a trustworthy limit become available
    from Claude Code, should it be used for a context bar? Unknown model names that do not fit show
    "Model" rather than a cut label. Composer rendering and layout tests, workspace Clippy and tests
    passed; visual inspection in a running app has not been checked.
67. **Done: Shift+Enter starts a line in the Claude Code terminal without sending.** Switch sends `CSI 13;2u`
    for Shift+Enter alone in a tab launched as Claude Code. Shells and other programs retain classic Return
    (covered by tests). A scratch ConPTY run with Claude Code 2.1.332 showed two prompt rows with this
    sequence even without kitty negotiation. The same run showed line breaks from the Chat composer's bracketed
    paste encoding (item 46); the composer test checks that Enter waits for the echo. No prompt was submitted.
    The desktop key path, other CLI versions and other programs were not checked interactively.
    Review fix (2026-11-27): the tab's launch name alone sent `CSI 13;2u` to any program in it, such as an editor
    Claude Code opened. Now it goes on the main screen (the classic view) and on the alternate screen only while the
    cursor is in Claude Code's input box. Checked by recording Claude Code 2.1.332 with its full-screen view turned on
    for that session only (`--settings '{"tui":"fullscreen"}'`): it draws on the alternate screen, with the same
    input box, and `CSI 13;2u` started a second input line there. Not checked: a terminal editor opened with Ctrl+G
    (Claude Code offers Notepad on this computer, a window of its own, so none was opened); a program that draws such a box on the alternate screen would get
    Claude Code's Shift+Enter.
    Re-review (2026-11-27): a draft holding a rule line, the cursor on one, or a draft taller than the screen turned
    Shift+Enter into a plain Return that would submit the draft. The box's top is now the nearest rule with a `❯` row
    under it, and a draft whose top scrolled away is known by the mode line under the box ("shift+tab to cycle",
    "? for shortcuts" or "esc to interrupt"; only the first was seen in a recording).
68. **The `/usage` card loses the terminal's bars, and its spinner line jumps** (owner, 2026-11-27, screenshot of the
    read-only menu card for /usage). The card copies the panel's rows as plain text, so the usage bars the terminal
    draws (coloured cells or block characters) are missing; copy each cell's colours and draw the rows as the
    terminal does (or recognise the bar rows and draw a real bar with the percentage). The "* Scanning local
    sessions…" row animates in the card and jumps sideways: use item 55's fixed-width spinner slot there too, and in
    every other place that shows a copied CLI row. The card says "Choose in the terminal's menu" and "Answer this in
    the terminal" for a panel that only shows information; say "Claude Code panel" and offer Close (Esc) only. With
    the card up, the composer's notice "No quiet, empty Claude Code prompt is visible" is noise: the card already
    says what is on screen.
    Done (2026-11-27): a usage row (block characters, none at 0%, then "N% used") is drawn as a real bar in the
    theme's colours with its percentage; rows starting with a spinner glyph (in the panel card, the permission
    card's copied rows and a menu's title) keep the glyph in item 55's fixed slot; a menu without choices is a
    "Claude Code panel" card with Close (Esc), or Open terminal when its footer offers no Esc; while a menu or panel
    card is up, the composer's prompt notices are replaced by "Close Claude Code's panel above to send." (or the
    menu's own notice). Checked: unit tests over the recorded /usage rows and a scripted panel. Not checked: the
    card has not been seen in the running app; the "Scanning local sessions…" row is not recorded (its glyph is
    assumed to be one of the working line's). The card reads a panel from its own top (see DESIGN.md), so the bars
    at the top of a tall /usage panel are in it; the selected settings tab as the card's title is tested only on a
    scripted screen, since the recorded screens keep no styles.
69. **Syntax highlighting, wrapping and line numbers in the code viewer** (owner, 2026-11-27).
    **Done:** literal code files use extension-based `mdshared-highlight` colours adjusted for palette
    contrast, source line numbers, and boundary-aware wrapping with hanging indentation. Unknown
    languages and files above 256 KiB stay plain; a token wider than the panel splits at a character.
    Markdown stays rendered, with the existing fenced-code highlighting. Rendering, wrapping,
    contrast and language tests cover the viewer; visual inspection in a running app was not checked.
70. **Any popup turns the terminal's transparency off** (owner, 2026-11-27; screenshot: the ⋯ menu
    over a transparent terminal). **Done.** The grid and margins keep their set opacity under
    menus, dialogs, tooltips, toasts, pictures and the undo bar. The overlay has its own solid
    card; only modal dialogs dim the rest of the window with a translucent scrim. A bitmap test
    opens the ⋯ menu over normal and dim glyphs, checks the entrance and settled card, and checks
    tooltip bounds and modal dimming over black and white. The separate per-cell contrast issue
    in item 62 still needs its toolkit change. Not checked on the owner's live desktop.
71. **Tool use pulls the eye away from Claude's text** (owner, 2026-11-27, screenshot of bordered "✓ done Read
    C:\…  53 ms" rows and "5 tool calls" boxes). **Done.** Tool rows and groups lost their box: one low line
    (22 px, 1 px apart) in the theme's muted text with the tool name no longer in the accent colour and in the UI
    face rather than mono, a faint "✓" and duration (set right after the target), a leading chevron, and a little
    air before the next answer. A failure keeps "✗ failed" in the warning colour on a soft wash of the theme's
    warning ground; a running call keeps its accent pulse. Hover, open and jump fills come from palette tokens.
    A bitmap test draws a synthetic conversation in the light and dark palettes, every built-in theme and two
    user-made themes (through the app's own theme code) and checks: no accent, no border, quieter than the answer
    text yet readable, a failure in the warning colour on its wash, and that opened rows and groups stay
    clickable. The pictures were looked at for light, dark and both user themes; nothing was seen in the
    running app.
72. **Delete a session from its menu, and optionally the original after a copy or a handover** (owner,
    2026-11-27: "can a delete be added next to archive on context menu for sessions, and for copy and compact as an
    option?"). **Done.** **Delete session…** after Archive session… moves the transcript and its own
    `<session id>/` folder to the Recycle Bin (shell API with undo), removes the session's own marks and kept summary,
    and leaves the desktop app's records alone (the confirmation says when the desktop app may still list it).
    Disabled with the reason while running, open in a tab or pinned, and checked again when Delete is pressed
    (running scan of every account under the launch lock, the saved pin). A partial move is reported. The handover
    dialog's review step has "Delete the original session afterwards" (off each time); the copy case is a separate
    row item, **Open a copy, then delete the original…**, rather than a checkbox, because Open a copy has no dialog
    to hold one and a new dialog would slow every copy. The copy's original goes once the copy's transcript is
    written. `/switch-compact` and automatic handover have no option. Checked: core tests over temporary folders
    (what moves, the shared-junction case, linked folders, guards, partial failure, nothing else touched) with a
    folder stand-in for the Recycle Bin; window tests for the menu items, disabled states, wording, demo,
    re-checks, mark and summary cleanup, copy and in-place deletion. Not checked: the real Recycle Bin call
    (`SHFileOperationW`) has not been run by a test or by hand, and the copy-written condition assumes Claude Code
    writes the copied conversation into the new transcript with its first message (not verified on a live fork).
73. **Compact and open… still blocks the app while the model writes** (owner, 2026-11-27: "It still blocks the
    usage of the app. Was it not done?"; then "put it behind like I asked and have it pop up the same window when
    it finishes"). Item 61 covered `/switch-compact` and automatic handover, not the dialog. **Done.** Make
    handover closes the dialog and the worker runs behind; the same Review handover window opens by itself when
    it is written, after any modal thing closes, one at a time, ignoring keys and acting clicks for 300 ms; the
    taskbar flashes when the window is not in front. A strip between the Sessions filter chips and the list
    shows ones being written (elapsed time, spinner, Cancel), failures (reason, Retry) and reviews closed without
    acting (Review, ✕), folding above two; the details pane has the same entry, and a failure with a terminal
    shown also gets the error bar with Retry. Closing the window asks first and says a running one is canceled
    and an unopened written one is lost (previewing never saved a handover, so none is kept across a restart).
    Consent, in-place rules, the delete box (off each time the review opens) and the name are unchanged.
    Checked: tests for start, finish and auto-open, waiting for a dialog and a menu, two at once opening in turn,
    keys in flight, Cancel/Esc parking and Review, Dismiss, Cancel stopping the worker, failure and Retry through
    consent, a second ask for the same session, the close confirmation and exit cancelling, the strip's folding;
    the Sessions column drawn with the strip in each state (light and dark) and looked at. Not checked: the
    real OpenCode run, the taskbar flash and tray notice, and the timing of a real keystroke against the 300 ms
    guard, since nothing was run in the live app.
74. **The Review handover dialog's footer buttons overflow** (owner, 2026-11-27, screenshot: Copy sticks
    out past the dialog's left edge). **Done.** The review card is up to 760 px wide (660 before), with
    Copy and Save as… in their own group on the left and Back, Cancel, In OpenCode and Open in CLI (or
    Continue here) on the right; when a card has no room for both groups, the left group folds into
    **More ▾**, a menu with Copy and Save as…. Every dialog footer now narrows its buttons toward their
    labels' width instead of running past the card's left edge (no other dialog has more than three
    buttons, which already fit). A test draws the review at 620 (narrower than the window can be),
    760, 900 and 1400 px wide at 100, 125, 150 and 200 % and checks every button is inside the card's
    padding and none overlap; another checks the narrowing. Looked at the 620, 760 and 1400 px pictures.
75. **The handover text editor wraps in the middle of words** (owner, 2026-11-27: "plu s", "back t o").
    **Done.** Rows break after the spaces before a word that does not fit (the spaces hang past the edge)
    and inside a word only when the word alone is longer than a row, as the Chat composer wraps. End,
    Up/Down and a click past a row's end stay on that row (before the space it broke at); Home, Right
    and selection cross soft breaks as before. Tests cover wrapping, long words, multi-byte characters,
    caret movement, selection across a soft break and reflow while typing.
76. **Permission prompts Chat could not match to the one pending call** (owner, 2026-11-27: a card that
    shows the prompt exactly as Claude Code draws it, so the user sees what they approve; never approve
    something the card did not show). **Done.** A subagent's prompt, the prompts of several calls of one
    message, and a prompt that does not name the pending call show as a card read from the screen alone
    (DESIGN.md, "A prompt the pending call does not cover"; `view/chat/question/prompt.rs`): every row
    from the prompt's top rule to its question, the question, the choices, Esc. Keys go only while the
    screen shows exactly those rows, question and choices; a choice needs every row to have been in view
    (a long prompt scrolls) and the card to have shown that prompt for half a second. The gate treats it
    like the pending call's own card (no drawer, "Answer Claude's request above", "Waiting for your
    approval"). Checked: recordings of Claude Code 2.1.332 (a subagent's Bash and Write prompts, the
    highlight on each kind of choice, Esc, two parallel Bash calls) as fixtures; reader tests; state
    tests on a tab gated like a real one, with the card painted (a click before the card showed the
    prompt, rows that change between the click and the key, the next prompt taking the first one's
    place, Esc, the setting off, a prompt longer than the card with the wheel and the "more" button);
    the card's picture looked at in two window sizes. Not checked: nothing was tried in the running
    app; a prompt with a diff (Edit) was not recorded, only Bash and Write; a prompt taller than the
    terminal was built by hand, not recorded. Still the terminal's: a prompt whose top is above the
    terminal's screen, a row wider than the card, and a decline with text (the comment field's drawing
    is not recorded). Fixed afterwards: the second of two prompts was not recognised when Claude Code
    left its hidden cursor on the first call's output row (found by the update check's recording, now
    the `perm_two_after` fixture); a prompt is now known by its footer being the last text on the
    screen, not by the cursor.
77. **A warning about denied permissions, with a way to approve** (owner, 2026-11-27: when a permission
    was denied by auto mode, by a rule or by the user's own No, show it prominently, say what and why,
    and offer what Claude Code offers). **Done.** The transcript reader carries the denial on the tool
    item (`crates/core/src/transcript/denial.rs`: kind, before-run flag, reason, the rule's source);
    tool rows say who refused; a card counts and lists the current turn's denials
    (`view/chat/question/denials.rs`); `/permissions` › Recently blocked is driven as a card of its own
    (`question/blocked.rs`); DESIGN.md, "Denied permissions", has the recorded facts and the rules.
    Checked: recordings of Claude Code 2.1.332 in scratch folders (a classifier block made with one
    added `autoMode.soft_deny` rule passed by `--settings`; the dialog with one and two blocked calls,
    Enter, Enter again, `r`, `r` again, Esc after an approval and after a retry; the transcript records
    of all three kinds); core tests on those records; reader tests on the fixtures; state tests with the
    cards painted (approve, choose another call, retry, a list that changed since it was painted, a
    mark that never shows, the setting off, Review through the slash-command path, Ask Claude to
    retry); the cards' pictures looked at. Not checked: nothing was tried in the running app; a block
    with a reason from the classifier (the recorded one gave none; the "Reason: " reading is from the
    record's fixed wording); Enter on a call marked to retry and `r` on an approved one (not offered);
    a list longer than the terminal, a call's row that wraps, and marks of two kinds at once (the
    footer's words are shown as they are); `ruleSource` values other than `flagSettings` (the names
    of the other settings files are Claude Code's documented ones); a No given with text; a denial
    inside a subagent (not counted: it is in the subagent's transcript, and only the CLI's passing
    notice under the input says so). Open: the count is hidden while another card has the card's
    place; a rule can only be changed in the terminal.

80. **Later: `/fork` breaks Chat's continuation** (owner, 2026-11-27: "fork broke chat continuation, I
    think. not a big deal right now. note it."; screenshot). After `/fork` typed in a Claude tab,
    Claude Code goes on in the forked session (Chat shows the `/fork` row, "session waiting for a
    prompt · <title> · continued · <id>"), while Chat keeps showing the old conversation and does not
    pick up the new turns the terminal shows. Chat follows a tab to a new session after `/clear`,
    compaction and `/resume`; `/fork` probably needs the same (find the forked session's transcript
    and switch the tab to it, as for `/resume`). Not looked into yet.
