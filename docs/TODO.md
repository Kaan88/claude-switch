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
