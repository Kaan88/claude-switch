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

## Reported after 0.1.0 (owner, 2026-11-26)

Status after the fix rounds on branch `integrate` (2026-11-26; GPT-6.2 Sol workers, two reviews by
Claude Fable): items 1 to 13 and 15 to 21 are built and pass format, lint and the test suite; item 14
is built for the Chat composer only, the text fields belong to the toolkit (asked for in its session).
Nothing below was tried against a live Claude Code session, the real desktop app or a real install:
the owner's checks are what is left. To check live first: the empty-prompt and slash-echo detection
(items 7, 10, 11), the picture send (11), the desktop launch through the package alias (2), Stop via
the CLI's shell manager (17), the new chat's name via `--name` (21), and the installer's PATH task (19).
Item 10 C and D (CLI menus as cards, Quick commands settings) were not picked and are not built.


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
    (artboard C, not picked). Not started.
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
