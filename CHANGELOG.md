# Changelog

## Unreleased

- Chat and the Session view: after a rewind, the Tasks, Agents, Shells and Files pages leave out
  what the replaced turns did once the next message is written, as the conversation does; an agent
  or background shell started there that is still running stays listed.
- Chat: the message box is quieter, as in the desktop app: no shadow in a dark theme (a faint one in a
  light theme), an edge only just lighter than the box, and focus firms it only a little.
- Chat: the Compact chip runs Switch's handover by default (labelled "Handover"; the same dialog
  as "Compact and open…", continuing in place where it can). Settings › Chat › "Compact chip runs"
  switches it back to Claude Code's `/compact` (labelled "/compact").
- Chat and the Session view: the picture viewer goes to the conversation's previous and next
  picture with buttons on its sides (shown where there is one, quiet until pointed at) or the Left
  and Right arrow keys, and says where you are ("2 of 5"; left out in a window too narrow for it
  beside the buttons). Each picture still loads only when shown, and one that cannot be shown
  says why while the arrows go on.
- Chat: when Claude Code may be waiting in the terminal, the bell rang, a permission prompt Chat
  cannot read whole is left with its drawer closed, or a send is refused because Chat cannot read
  the screen, the part of the terminal screen that matters shows as a card above the message box:
  from the call that waits down to the input, in the terminal's font and colours, following the
  screen while it stays. **Answer in terminal** opens the drawer with the keyboard, and the
  drawer closes again by itself once the wait is over, unless you resized it or typed in Chat
  meanwhile; **Dismiss** hides it until the next time. It goes once Chat can read the screen again. Chat still types
  nothing into such a screen, and the drawer no longer opens by itself for these waits.
- Chat and the Session view: what a command printed (`/context`, `/mcp`, `/cost`, shell mode) is
  drawn as the terminal draws it, from the colour codes Claude Code recorded: the terminal's font,
  cell grid and theme colours, so `/context`'s grid and the columns beside it line up and keep
  their colours. Lines wider than the box wrap; selection and Copy give plain text. Other escape
  sequences in the output are dropped, so it cannot move or clear what is shown.
- Chat and the Session view: after a rewind, the turns it replaced are no longer shown once the
  next message is written; the conversation follows the active branch of the transcript.
- Chat: after a handover, `/rename` could stay typed on Claude Code's input without Enter (typed
  just before a permission prompt took the screen) and every later message waited behind it
  without a word. The rename now goes only once Chat has read the new session's first prompt, and
  every command Chat types on its own (a rename, a handover's `/clear`, the Compact, Clear and
  model buttons) is followed until Claude Code took it: one that was not entered is taken off the
  input again (only when the input shows exactly that line, never your own text), a rename is
  typed again up to three times, and the box says what happened.
- Chat: a slash command after `/model` is no longer refused with "A turn is running (the screen
  reads idle)" when the turn before it ended without an `end_turn` record.
- Chat: background agents listed under the input (their timers redraw every second) no longer
  keep the screen from reading quiet, so commands and messages go instead of being refused.
- Chat: the "Review your answers" step of a question shows as a card when blank rows and the
  input's rule are drawn under it, instead of sending you to the terminal. A menu's highlighted
  choice (`❯ 1. Yes`) is no longer taken for the input's mark.
- Chat: a message that is a numbered list ("1. …", "2. …") no longer shows as a question card
  that blocks sending. What is in Claude Code's input box, under it or in the conversation above
  it is never read as a menu, question or dialog.
- Chat: when Claude Code moves a tab's conversation to a new session after `/fork` and says so in
  the transcript it leaves, Chat follows it instead of staying on the old conversation; the
  conversation shows there once its transcript is written with the next message.
- Chat: shell mode turned on with no command yet counts as a draft, so a handover continued in
  place waits for it instead of being sent as a shell command after `/clear`; a draft that starts
  with `!` shows as shell mode before it goes; a long shell command that wraps is entered.
- Chat: a one-character message (or a short number) sent while a call is pending waits for the
  call, as a prompt drawn the same instant could take it as its answer; a message held in the
  first moment of a call joins Claude Code's queue once the call runs; queued bubbles go when the
  program ends, and only a prompt with the same text takes one off.

- Chat: a multi-select question read from the screen shows Claude Code's ticked boxes as the
  card's own ticks instead of "[√]" in the label, and no longer repeats each label under it; the
  list of blocked calls marks the call its buttons act on with a bar, not a tick; a rewind on its
  way says "Rewinding in Claude Code…"; a card too tall for a short window says "Scroll for the
  choices" while they are below.
- Chat: the count of permissions denied in the current turn stays in view, on a line over another
  card that holds the card's place (a question or a permission prompt), instead of disappearing
  until that card is gone.
- Chat: `!` typed into the empty message box turns it to Claude Code's shell mode, as in the
  terminal: the command goes as `!command` and runs in Claude Code's shell (Backspace in the empty
  box leaves the mode). Enter goes only once Claude Code's input shows the command in shell mode,
  which is not recorded from a real session yet.
- Chat: while a command runs, a slash command or "Send anyway" is refused with "a turn is
  running" instead of "Claude Code may be waiting for your answer in the terminal", which it was
  not: the screen shows the turn working over an empty input.
- Chat: messages sent while Claude works wait in Claude Code's queue, and Chat now shows each as a
  quiet bubble at the end of the conversation ("Queued — waiting for its turn"), in the order sent,
  until Claude Code hands it in. **Send now** under them uses Claude Code's own key for its queue
  (Ctrl+X Ctrl+S): every queued message goes to the running turn at once, without stopping it. A
  message sent while a command runs now goes into that queue too, instead of waiting in the box
  for the turn to end.
- Chat: a card taller than its room (a long command in a short window, or with the terminal
  drawer open) no longer cuts off its choices and foot: it lowers its choices, shortens the
  command (saying how many lines are left out) and the question, and scrolls what still does not
  fit, with Open terminal and Esc kept in view. Nothing outside a card takes a click meant for
  it.
- Chat: a question on a new session shows at the foot of the window, not in the greeting's place.
- Chat: a question longer than the card shows says so ("The rest of the question is in the
  terminal") instead of being cut silently.
- Chat: "Run in background" shows on the card when the command's own line is scrolled out of view.
- Chat: Stop keeps its own place beside Send while Claude works, so a click on Send never lands
  on Stop; Stop only stops the turn it was drawn for.
- Chat: Escape under a card does what its "Esc to …" button says; a key typed while a card waits
  says where typing goes.
- Chat: the task progress on the working line ("2 of 3") is not shown where the window is too
  narrow for the panel it opens.
- Chat: copied text no longer carries the invisible character kept above code blocks for their
  header, and the header no longer covers a selection at the code's top.
- Chat: the rewind, blocked-calls, background and denied-permissions cards are laid out like the
  other cards; Chat's side margin narrows to 12 px in a narrow window.

- Handover: Claude Code no longer asks in the terminal whether it may read the handover file. A
  new handover tab may read exactly that one file. A handover that continues in the same tab is
  pasted whole up to 15,000 characters, as before; a longer one is still read from its file, and
  Claude Code's question about it shows in Chat as a card with its choices, which answers it.
- Handover: the new session keeps the old one's name instead of adding " · continued" each time
  (which piled up as "… · continued · continued"); such endings from earlier handovers are
  removed. The old session is marked as handed over, so the list still tells the two apart.
- Terminal tabs: split a tab into up to four panes, side by side or stacked, with "Split right",
  "Split down", "Close pane" and "Next pane" in the tab's menu and the palette. A new pane starts a
  shell in the tab's folder; a click gives a pane the keyboard (it has a thin edge), the divider
  between two panes can be dragged, and each pane has its own size. Chat and the session view stay
  with the tab's own Claude pane, which closes last; closing the last pane closes the tab.
- Terminal tabs: find in the screen and the scrollback. Ctrl+Shift+F (while the terminal has the
  keyboard, as in Windows Terminal), "Find in terminal" in the palette or "Find…" in the tab's menu
  opens a bar at the top right of the terminal, also over Chat's terminal drawer. Matches are marked
  as you type, the current one in its own colour, with a count ("3 of 12"); Enter or ↑ goes to the
  match above, Shift+Enter or ↓ to the one below, Aa (Alt+C) matches case (off by default), Escape
  closes it. A match across a wrapped line is found.
- Sessions: the details show the selected session's conversation as Chat draws it (answers in
  Markdown, code blocks, tool rows that open), read-only, in a region that scrolls on its own and
  follows new messages; it replaces the short "Last messages" list. Earlier history loads on
  request.
- Workers tab: beside Sessions while the OpenCode bridge's job folder (`%TEMP%\opencode-agent`)
  exists, the bridge's jobs live: worker, model, folder, state, running time and last tool, with
  jobs whose server has exited shown as "Owner gone". The selected job's output streams from its
  event log. **Cancel job…** ends the job's worker and the processes it started, after a confirm,
  and only while that process is still the job's; otherwise it says why it did not.
- Settings › Claude Code › Advanced: the rest of Claude Code's `/config` (taken from 2.1.332), with a
  search box, a short line on what each does, a "default" mark and Reset to Claude Code's default.
  `settings.json` entries are saved in the master account and shared; `.claude.json` entries belong
  to the chosen account and are read-only while it runs Claude Code. A value changed elsewhere since
  the page read it is never overwritten. Entries Claude Code lists only in some contexts
  (fullscreen mode, an IDE connected, agent teams, a feature flag) say so on their row.
  `.claude.json` is changed only when no Claude Code can hold it (a desktop session of another
  account in the same folder, a session still starting, a program in one of the window's tabs,
  or a scan that could not read everything all count), never created, backed up first, and
  replaced only if its bytes are still those read; a lock Claude Code left behind is taken over.
  Saves and reads run in the background, and files keep the order of their keys.
- Settings › Claude Code › Status: for a chosen account, the installed Claude Code version, the
  sign-in method and account, the settings files in effect, user MCP servers and memory file, and
  stats from its prompt history (prompts, sessions, days active, folders) and the models of its
  newest sessions. Read from files only; plan usage links to the Usage page.
- Tools: `recorder compat` has a `fork` script that checks what `/fork` does to the live session
  record and the transcripts (a `live` op records both), and whether Chat reads the screen after
  it as an idle input.
- Tools: `recorder compat` has a `rewind-send` script: a rewind, then a prompt, checked against
  what Chat shows of the transcript.

## 0.1.6 (2026-11-28)

- Chat: a long message (pasted or typed) is shown as its text; Claude Code records it inside
  `<pasted_content>` tags, and Chat used to leave it out.
- Chat: a message into a running turn is no longer stopped when the turn records a tool call
  while Chat waits to see it on the input, and a long one-line message (which Claude Code shows
  as "[Pasted text #1]") goes too instead of waiting five seconds and staying on the input. Send
  now into an idle input (the turn ended while the message waited) is an ordinary send.
- Chat: "Rewind to here…" in the right-click menu is not offered over a subagent's transcript, and
  is refused there with a word; it would have rewound to a prompt of the agent's.
- Chat and the Session view: Claude Code's "[Request interrupted by user]" after an Esc is shown
  as a quiet "Interrupted" line, no longer as a message of yours; it starts no new turn, so Stop
  is not offered again for the turn you stopped, and it is not offered for Rewind or prompt recall.
- Chat: Branch is not offered where the copy would hold a tool call without its result.
- Chat: a message with a picture that Claude Code put back on its input after an early Stop is
  recognised (and Press Enter's check finds it): Claude Code shows the text right after the
  picture's marker, without the space Chat looked for.
- Tools: the Claude Code compatibility check (`tools/cli-compat.ps1`) also checks what Chat now
  relies on in Claude Code's files: messages sent while a turn runs (with a picture, one line, two
  lines, two in a row) and how the transcript and the rewind list show them, what Esc does with
  messages waiting in Claude Code's queue, thinking shown as updates, and that a branch's cut copy
  resumes under its new id, in print mode and in the terminal. Its picture step refuses to run
  when the clipboard holds more than plain text, and leaves the clipboard alone when something was
  copied during the run.
- Chat: the round "jump to latest" button no longer covers the last line of text while you are
  scrolled back; the text ends above it.
- Chat: buttons under your messages and under Claude's answer to each turn, shown while you point
  at the message and always under the newest answer: Copy (the message as shown), Rewind (the
  same as the right-click "Rewind to here…") under your own messages, and Branch, which starts a
  new session that continues from a copy of the conversation up to the end of that message's turn
  (with Claude's whole answer). Branch is offered once that turn has ended, and not in a
  subagent's transcript. The original session is not changed; a branch has no file backups, so
  it cannot rewind code changes made before its cut.
- Sessions: the search finds a session by its id: the whole id, its first eight characters or
  more, or a pasted line such as `claude --resume <id>` from a hand-over. It is listed first, under
  "Session id", also when it is hidden, archived or a worker's session (its row says Hidden or
  Archived, or its title is grey when the desktop app archived it). Other searches are as before.
- Chat: when the terminal opens for a question Chat cannot answer itself (Claude Code may be
  waiting, a permission prompt Chat cannot read whole, or a card's answer that could not go), the
  terminal takes the keyboard, so the answer can be typed at once; not while you are typing in the
  message box, the box or a card's field holds text, or text is selected in the conversation (so
  Ctrl+C copies it and never reaches Claude Code). "Open terminal" on a card already did so.
- Chat and the Session view: a drag after a double click selects on by whole words, and after
  a triple click by whole paragraphs, in the conversation; in the message box a drag after a
  double click goes by whole words. A drag after a double or triple click used to do nothing.
- Chat: Ctrl+C copies the text selected in the conversation when the message box has no
  selection of its own; it used to do nothing there.
- Chat: a message typed while Claude works (in Chat or in the terminal) is shown as your message
  where Claude Code handed it in; it used to be missing, with its picture.
- Chat: with thinking shown as updates, the text Claude writes there is shown as its answer, as
  the terminal shows it; it used to be folded away among the steps as a thought.
- Chat: Send while a turn runs types the message into Claude Code's input at once, as Send now
  does, when that input reads empty; Claude Code hands it to the running turn, and the box is free
  for the next message. When the input holds something, the message waits for the turn as before.
  The empty box then reads "Message Claude while it works…". Into a running turn (also with Send
  now) Enter goes only once Claude Code's input shows the message, so a prompt drawn meanwhile
  never gets it, and Stop stays available after such a message.
- Chat: after a Stop before any answer, the message Claude Code puts back on its input is taken
  back into the box also when other messages went in during that turn; other text left on the
  input after a Stop is said in the box's notice instead of holding the next message silently.

## 0.1.5 (2026-11-28)

- Chat: a new session before its first message shows a greeting in place of the grey "Waiting
  for messages" card: the mark, "What should Claude do in <folder>?", the folder and, when
  Claude Code's banner is on the terminal's screen, the model and plan it names. The message box
  is in the middle of the window with a few starts under it: your starred slash commands, else
  /init, /review and /resume, and "Paste a picture". A start fills the box; it sends nothing.
  The Details panel stays closed until there is something to show. With the first message the
  box goes to the foot of the window.
- Chat: a conversation that is still being read shows one quiet line, "Reading the
  conversation…", instead of the card.

- Chat: the Details panel is one scrolling list of sections in place of its five tabs.
  **Progress** is the task list with "2 of 3", a thin bar and a mark per task (done, current,
  waiting). **Background** has the agents, the background shells with their output and the MCP
  tasks, running ones first. **Changed files** lists each file with its lines added and removed;
  the name opens the file and the arrow lists its edits. **Outputs** is as before. A section with
  nothing in it is not shown, and each folds from its heading. Everything the tabs did is still
  there: full output, Stop…, jump to an edit, open an agent's transcript.
- Chat: the Details panel opens by itself when it has a first thing to show and is closed when
  it has nothing, until you open or close it yourself with Details; then it stays as you left it.
  While it is closed and Claude works, the task list's progress ("2 of 3") stands at the right
  end of the working line and opens the panel.

- Chat: a question of Claude's, a permission prompt and a Claude Code menu now stand where the
  message box is, in the same box, instead of in a card over it. The message box is not shown
  while such a card waits (nothing can be sent then anyway); a draft is kept and comes back with
  the box, and typing meanwhile no longer changes it. The line "Waiting for your answer" is gone:
  the card says it.
- Chat: these cards are laid out anew. A question has its own header as a small chip, the
  question in larger type, and the choices as one list, each with its number and its
  description. "Type something" is only the field ("Something else…", with a send arrow) and
  "Chat about this" only a button at the foot; both used to be listed as choices too. A form
  shows "1 of 2" with Back and Next, and Submit on the last question.
- Chat: a permission prompt's title is the prompt's own ("Bash command"), and the command is
  shown once, in a box; it used to be shown twice under two titles. The choices are the CLI's
  exact words with their numbers, with the field "No, and tell Claude what to do instead…" where
  the prompt offers it. An edit's lines are in the diff's colours.
- Chat: every such card has a × at the top (Esc, where the CLI offers it) and "Open terminal" at
  the foot. In a short window a card drops the descriptions, then lowers its rows, before it is
  cut. What a click does, and every check before a key is written, is as before.

- Chat and the Session view: a code block in an answer has a header with its language on the
  left and Copy on the right. The Copy button used to cover the language, so no block showed it.

- Chat and the Session view: a run of tool calls between two answers is one quiet line in words
  ("Ran a command, read 3 files, searched the code"), at the column's edge. A single call reads as
  itself ("Ran cargo test -p tiles"). The line opens into a timeline: an icon per kind of call,
  what it did, its target in the monospace font, the time it took, and the output of an opened
  step in a box. A failure is counted in the line in the warning colour ("· 1 failed"), without
  the coloured wash; a refused call still says who refused it. While a call runs the line names
  it ("Running cargo bench …"), and "Waiting to run …" while a card asks about it. A thought is a
  step of the run. Results of slash commands and of shell mode keep their open output.
- Chat and the Session view: a file edit is a row of its own with the path and the lines added
  (green) and removed (red). It opens into a diff with tinted lines; with line numbers and the
  lines around the change when Claude Code recorded them with the edit, else the changed lines
  alone. The green and red are the terminal theme's. A long line is cut at the row's edge.

- Chat: the working line ("✻ Shimmying… 12s · ↓ 1.2k tokens") is the last line of the
  conversation, where the next text will appear, with the mark in the accent colour. The strip
  between the conversation and the message box is gone.
- Chat: Stop is the round Send button, as a square, while a turn runs and the box is empty. With
  a draft the button sends (the draft is queued, as before) and Stop is a small square left of it.
  Stop is dimmed while it cannot be used. The box reads "Queue the next message…" then, and no
  longer shows a notice bar to say so.
- Chat: "Run in background" stands at the right of the running command's line instead of in a
  card of its own. The card still shows when there are agents to stop or something to report.

- Chat: one column. Claude's answers start at the column's left edge, without the ✳ mark or an
  indent; your messages are in a neutral grey bubble on the right; the message box and the cards
  are as wide as the column, so the draft, a card's text and the answers share a left edge. The
  reading width setting still sets the column; at Full the box stops at 900 px. The Session view
  beside a terminal drops the mark and the indent too.
- Chat: the message box is a raised, rounded box with a soft shadow, and its border no longer
  turns to the accent colour with the keyboard. The draft is in the interface font at 15 px, not
  the monospace one. Send is a round button with an up arrow: in the accent colour when there is
  something to send, grey when not.
- Chat: "Jump to latest" is a round button with a down arrow over the foot of the conversation,
  shown only after you scroll up, with a dot once newer messages arrived. "Following latest" is
  gone.

## 0.1.4 (2026-11-27)

- Chat: in a named session (every session this app names) Claude Code writes the name into the
  rule over its input, and Chat took the row for earlier text: it never saw an empty, idle input.
  Messages stayed queued, Send now said the input was not visibly empty, and Chat could say Claude
  Code might be waiting for an answer. Chat now reads that rule as the input's border.
- Chat: a new session in a folder Claude Code has not seen asks whether you trust it. Chat
  showed that dialog without its choices (they carry no numbers), so Yes needed the terminal;
  its card now offers both.
- Accounts: Claude Code's settings are the same in every account again. They were shared by a
  hard link to the first account's `settings.json`, which Claude Code breaks whenever it saves the
  file (`/model` does), so every further account had drifted to its own copy (one had lost the
  handover plugin). Now they are copied: before every launch and when the app starts, a change
  made in one account goes to the others; when several changed, the master account's wins.
  Accounts shows the master and lets an account keep its own settings. A few preferences of
  `/config` kept in `.claude.json` (such as Chrome) are shared the same way. Replaced files are
  backed up in `%APPDATA%\claude-switch\settings-backups`.
- Settings › Claude Code: the model, effort, permission mode and theme, and Chrome by default,
  auto-compact and verbose output, saved in the master account and shared with the others.
- Sessions and tabs: "Open working folder" in their menus.
- Sessions: "Continue in another folder…" resumes a session in a folder you pick. Claude Code
  writes on into the same transcript; the app remembers the folder, so the list, its group and
  the next Open follow it.
- New session: a folder that does not exist is offered to be created (Start again creates it).
- Chat: after `/model` (or another command that only prints, such as `/effort`), the session no
  longer reads as a running turn. Chat refused the next command, so the effort could not be changed
  after the model, and said Claude Code might be waiting for an answer.
- Chat: Send now while Claude Code's input does not read empty is kept instead of refused: the
  message goes by itself as soon as the input reads empty, still during the turn. A message sent
  while the input holds text is held the same way instead of being refused, and Chat never clears
  that text. Typing in the terminal still cancels a held message (the draft is kept).
- Chat: when it cannot send because of what it read on the screen, or says Claude Code may be
  waiting for an answer, it keeps the screen and a note of why in
  `%APPDATA%\claude-switch\screen-reports` (on this computer only; the newest 20, at most one a
  minute per tab), so a screen it misreads can become a test.

## 0.1.3 (2026-11-27)

- Development: a check against new Claude Code versions (`tools/screen-recorder`, `tools/cli-compat.ps1`,
  `.github/workflows/cli-compat.yml`) records what the newest Claude Code draws and runs Chat's screen
  readers over it, so a version that breaks Chat is caught before it is met in use. On GitHub it runs
  once per new version and waits for an `ANTHROPIC_API_KEY` secret; on this PC it runs by hand.
- Chat no longer refuses nearly every message in the installed app. Started from the Start menu,
  Claude Code got no `TERM` and drew its input mark as `>`, which Chat did not read as the prompt:
  sends, pictures ("No empty CLI input line for the picture"), slash commands and the Mode chip all
  pointed at the terminal. Claude tabs now start with `TERM=xterm-256color`, and Chat reads both
  drawings.
- Chat takes "ready" from the screen: a quiet, empty input with no working line. A resumed session,
  a turn stopped with Esc, a slash command or a key in the terminal no longer leave "Open the
  terminal to check" or a turn that seems to run forever. An answer that mentions "esc to
  interrupt" is no longer taken for a running turn, and a bell no longer blocks an idle prompt.
- A message sent while Claude Code is busy or still drawing its prompt is held and goes out by
  itself; **Send now** and **Cancel** stay. A slash command waits up to three seconds for the
  prompt.
- A question Claude asks now shows in Chat while it waits (Claude Code writes it to the transcript
  only after the answer): its text, choices and their descriptions, a form's pages and its review,
  with "Waiting for your answer" instead of "Working…". When the terminal is silent for seconds
  with nothing Chat can read, the terminal drawer opens by itself.
- Your own answer to a question: the question card has a field for it, also in forms with
  several questions. Chat types it on the CLI's "Type something" row and confirms only once the
  screen shows exactly that text.
- A permission prompt from a subagent, or one of several tool calls made at once, no longer sends
  you to the terminal. Chat shows the prompt as Claude Code draws it: the tool, whose call it is
  ("from the general-purpose agent"), the command or file, the question and its choices. A long
  one scrolls, and its choices can be clicked once you have seen all of it. Nothing is approved
  that the card did not show: the keys go only while the terminal still shows that same prompt.
  To decline with a comment, use Open terminal.
- Denied permissions are said out loud. A tool call that auto mode blocked, a rule denied or you
  declined now reads "blocked by auto mode", "denied by a rule" or "you said no" in the
  conversation instead of "failed", and a card under the conversation counts the current turn's
  denials and lists what was denied and why. **Review blocked calls…** opens Claude Code's own
  list of calls auto mode blocked (`/permissions`), which Chat shows as a card: choose a call, see
  its command, **Approve** or **Retry now**, then apply. **Ask Claude to retry** puts a sentence
  in the message box and sends nothing.
- Stop before Claude answers: Claude Code puts the message back on its own input, which blocked
  the next send. Chat now clears it there and puts the text back in the message box. When the
  box holds a draft (a second message held behind the first, or one you were typing), Chat leaves
  the message on Claude Code's input and says so, instead of clearing it with nowhere to go.
- More time for a pasted picture to show (20 s) and for a pasted message to appear (5 s).
- Chat: the working line ("✶ Sprouting… (40s · ↓ 3.2k tokens)"), "Waiting for your answer/approval",
  the terminal-answer hint, a read error and Load earlier line up with the conversation's reading
  column instead of the window's left edge.
- Chat: Cut in the message box (right-click › Cut and Ctrl+X) removes the selected text after copying
  it; before, it only copied.
- Chat: undo and redo in the message box: Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z, and Undo and Redo in its
  right-click menu. Typing undoes a word at a time; a paste, a cut, a run of deletions, a completion,
  a recalled prompt or a filled chip is one step each. Sending a message clears the history.
- Chat and the handover editor: Ctrl+Left and Ctrl+Right move by word, and Ctrl+Shift+Left/Right
  select by word, as in Windows text boxes. Before, they moved one character. Ctrl+Backspace and
  Ctrl+Delete remove a word, one undo step each in the message box.
- Settings: **Peek** at the top right of every page. Hold it with the mouse, or hold Space when
  nothing focused uses Space, and the dim and the dialog fade away (instantly with Reduce motion)
  to show the window as it is with the changes already applied, Mica or Acrylic included; a pill at
  the top says how to come back. Let go and Settings returns on the same page, scroll and focus.
  Clicks and keys do nothing while peeking, and switching away from the window ends it.
- Settings › Appearance: the dark and light palettes (and the Terminal page's colour presets) are a
  grid of equal cards, five across, each a small window in that palette: title bar, side pane, text,
  accent button and a terminal with its colours. Names sit whole inside the card; a long custom name
  is shortened, with the full name on hover. The chosen card has an accent border and a check, the
  hovered one a lighter fill, and the keyboard's ring sits outside the card. Dark and light share
  one column grid, custom themes wrap to a further line, and nothing runs into the next setting.
- Handover: **Compact and open…** writes the handover in the background. The dialog closes when you
  press Make handover, and the Review handover window opens by itself when it is ready (after any
  dialog or menu you have open, once you have stopped typing for a moment, and ignoring keys
  already on their way; click in its text or press Tab to edit). The Sessions column shows
  handovers being written (with Cancel), failed ones (with the reason and Retry) and reviews closed
  without acting on them (Review). Closing the window asks first while one is being written or
  waiting to be opened.

- Handover: the review's text wraps at word boundaries instead of in the middle of words.
- Handover: the Review handover dialog's buttons always fit; Copy and Save as… sit on the left, or
  under **More ▾** when the dialog is narrow.
- New session: **Recent folders** is now a scrolling list of up to 200 folders instead of four, newest
  activity first, with the last-active time. It leaves out archived and hidden sessions, sessions
  started by tools while the list hides them, folders that no longer exist, temporary folders and
  Git worktrees (you can still type or browse to those), and counts different spellings of a folder
  once. Type in the folder field to filter it; the arrows, Page Up/Down, Home, End and Enter work in
  it.
- Terminal: a blinking cursor (Settings › Terminal › Blink) keeps an even rhythm in Claude tabs and
  the Chat drawer. Output now restarts the blink only when it moves or changes the cursor, so
  Claude Code's once-a-second counter elsewhere on the screen no longer cuts the off phase short.

## 0.1.2 (2026-11-27)

- Sessions: **Delete session…** in a session's menu, after Archive session…, moves its transcript and
  its own folder (subagents, tool results) to the Recycle Bin and removes its Claude Switch marks and
  summary. It is unavailable, with the reason, while the session runs, is open in a tab or is pinned,
  and checks again when you press Delete. The desktop app's own record is left alone. **Open a copy,
  then delete the original…** deletes the original once the copy is written, and the handover dialog
  has **Delete the original session afterwards** (off by default). The Delete key still hides.
- `/switch-compact` runs the handover in the background and returns at once; Switch shows the new
  tab or continuation, or a notice (a message box when no window is open) when it fails. Leftover
  background job files older than a day are removed at start.
- Chat: questions with several choices (checkboxes) and forms of several questions are answered from
  the chat, one question at a time with Back, Next and Submit; every key is checked against the
  screen first. The `/usage` panel shows as a card with real usage bars.
- Chat: tool calls are quiet one-line rows in the muted colour, so Claude's text stands out; failed
  calls still stand out. Command results and `!` shell output show as foldable cards.
- Chat: **Rewind to here…** on a prompt you typed (right-click), driving Claude Code's rewind list
  with a screen check before every key; you pick what to restore. **Stop agent** for a running
  background agent, and **Run in background** only under the running command's own hint.
- Settings › Chat: the order of the chips under the message box, up to three of your own commands
  as chips, and **Answer menus from the chat**.
- Search by meaning (off by default, Settings › Sessions): a model you pick summarizes sessions from
  the excerpts Suggest names already sends, after a confirmation; search then finds sessions by what
  they were about, **Ask about past sessions…** answers questions, and Topics can group by subject.
- OpenCode: **New OpenCode tab…** runs your normal OpenCode in a folder, and **In OpenCode** in the
  handover preview continues a session there with the handover as the first prompt.
- Themes: your own themes, imported and exported as files, and one consistent control per setting in
  Appearance. Text on a transparent terminal is cleaner, and popups no longer turn transparency off.
- Terminal: Windows Terminal mouse behaviour (arrow pointer, drag to select, right-click copies or
  pastes; Shift when a program uses the mouse). Shift+Enter starts a new line in Claude tabs.
- Chat: a reading width, scrollbars that fade, and a grouped toolbar with an effort meter.
- Side panel: code files get syntax colours, line numbers and indented wrapping.
- Handover: continues in place by default. When a Claude Switch tab runs the session (dialog,
  automatic handover, `compact --open` and `/switch-compact`), Switch sends `/clear` there, follows
  the tab to the new session and sends the handover as its first message, through the Chat view's
  checks; the dialog's button reads **Continue here**. The old session is marked handed over in
  its tab and in Sessions. A step that does not happen in time stops it with the reason and the
  saved file's path. Otherwise, or with a new tab chosen, it opens a new tab as before. Only that
  tab switches to Chat (the default for new tabs is kept), and Claude Code's own name follows with
  `/rename` once the first message has gone.
- Handover: when you give no focus, one is picked locally from the session (coordination, non-code
  work, or the default); the dialog says which and lets you edit it, and Settings › Handovers can
  edit and reset each of the three texts. The summary gains an **In flight and promised** section
  (jobs, workers, reviews and what was promised when they report); "instructions" holds only rules
  still in force; long fields are cut at a line with a visible `[cut]`; Files leaves out temporary
  paths and keeps the 30 most recent; the task list is marked possibly stale.
- Handover in place: never clears the session when you sent a message after the handover was
  made (it stops and says to make it again), and never clears under a draft in the message box.
- Handover: when saving the new session's links fails after continuing in place, they are kept
  and saved again instead of being dropped; a new handover tab that fails to start no longer
  removes an earlier handover's mark from the old session.
- Marks: a session forgotten in one window while the other saw it no longer turns into an empty
  entry that refused pins and names for it; such entries in existing files are repaired on load.
- Saved terminal tabs: a second window no longer replaces the first window's saved tabs with its
  own when it closes while the first still runs. A second window that closes last, after the
  first has gone, saves its own tabs (when it has any) so they are the ones restored next time.
- Handover channel: an elevated window refuses requests from lower-integrity processes; the
  window claims the channel before restoring tabs, so two windows starting together never both
  restore them; a client that connects and never writes no longer blocks later requests; the
  client checks the channel's owner, and a window that cannot create it says so.
- Handover tabs now get the classic renderer too (they name Claude Code by its full path), so
  Chat can send in them; continuing in place in a tab that shows Claude Code's full-screen view
  stops at once and says how to fix it, instead of waiting for the deadline.
- Handover: a tab started with Claude Code's compaction off stays watched after **Hand over
  automatically** is set to Off (it then asks first), and a failed automatic handover asks with
  the dialog next time instead of leaving the session without compaction or handover.
- Two Claude Switch windows no longer stop each other from saving: session marks (names, pins,
  hiding, handover links) and settings apply only each window's own changes onto the file as it is
  now. Only the same field changed differently in both is refused, with a notice naming it, and
  nothing asks for a restart.
- Settings › Handovers: **Where it continues** (This tab, A new tab), **Summary written by**,
  the focus texts (used when you give no focus; editable, with Reset), **New chat starts as** (new
  tabs only), **Hand over automatically** and **When the context reaches**, in that order.
- Chat: new Claude tabs use Claude Code's classic renderer (Settings › Sessions › Classic terminal
  view for Claude tabs, on by default), so Chat no longer refuses every send in fresh chats; the
  refusal now says how to switch the renderer.
- Chat: the first message of a new chat can be sent from Chat, and a tab follows its process to a
  new session after `/clear`, compaction or `/resume` instead of asking to reopen it. A new session
  is shown in Threads as the continuation of the old one.
- Chat: Claude's questions (AskUserQuestion) appear as a card you can answer from the chat; the
  working line says "Waiting for your answer".
- Text boxes (search, name and folder dialogs, Settings) select with the mouse: drag, Shift+click,
  double-click a word, triple-click all, and a right-click menu with Cut, Copy, Paste, Select all.
- The conversation in Chat and Session view can be selected across messages and copied with
  Ctrl+C or the right-click menu (Copy, Select all, Copy message, Copy as Markdown); code blocks
  have a Copy button.
- Chat composer: `@` completes paths in the session folder, Ctrl+R searches earlier prompts, and
  Claude Code's grey next-prompt suggestion appears as a chip that fills the draft.
- The Chat message box is three lines tall and grows with the draft up to about 40% of the view.
- Chat: permission prompts can be answered from a card in the chat, with the tool call and the
  terminal's own choices; the working line says "Waiting for your approval". A card answers ahead
  of a queued draft, which stays queued until the card's keys have gone. A menu that only looks
  like a permission prompt is left to the terminal; words in the conversation above a menu do not
  count for that.
- Chat: a finished plan shows as Markdown in the conversation and is approved from a card with the
  terminal's own choices.
- Chat: the Effort chip shows the level the session started with (environment, `--effort`,
  settings, or the model's default) and follows `/effort` and `/model` typed in the terminal; after a
  compaction Context shows the size the compaction left until the next answer.
- Chat: a Mode chip shows the permission mode and cycles it like Shift+Tab at an idle prompt.
- Chat: messages keep their line breaks; a message of several lines is submitted once Claude Code
  shows it on its input line.
- The two archives have distinct names: the **Archived here** chip is Switch's own archive;
  **Show sessions archived in the desktop app** and the details row **Desktop archive: yes** are
  Claude Desktop's.
- Colour pickers in Settings have Apply and Cancel: Enter or a click outside applies the colour,
  Escape restores the old one. Before, the chosen colour was not applied.
- Unknown command-line options are refused with a short usage text instead of being ignored (an
  ignored typo started the real app); `--help` lists every option, hidden ones marked.
- Appearance: more palettes. Dark: Neutral, Near black, Warm, Blue-grey, High contrast; light:
  Paper, Warm, Cool, High contrast; chosen from swatches in Settings › Appearance, each with
  matching terminal colours, and Claude's accent on every one.
- Markdown viewer: drag the divider to make it wider (remembered), choose a reading width
  (Narrow, Medium, Wide, Full) in its toolbar, and see the file's full path. A bare file name
  such as `CHANGELOG.md` now opens the file of that name Claude wrote elsewhere in the session,
  with the session folder's file offered beside it.
- Settings: switching category pages cross-fades quickly (140 ms); instant with Reduce motion or
  Windows' reduced-motion setting.
- Details panel: background MCP tasks appear in the Shells tab with their status, running time
  and result (the last step is not saved by Claude Code, so it is not shown). Every row's name now
  opens its work: a task its plan update, an agent its transcript, a shell its output.
- Chat: the working line no longer jumps sideways as its spinner animates or its counters tick.
- Chat: pictures, including screenshots and other tool-result pictures, show as captioned thumbnails
  (click to enlarge) instead of a "Picture" bar; a picture that cannot be shown says why. Other
  outputs say what they are with an icon and a kind (PDF, document, link, pull request with its
  repository and number, …), in the conversation and in Details › Outputs.
- Details panel: the Tasks tab follows Claude Code's TaskCreate/TaskUpdate tasks, not only TodoWrite.
- Chat: Claude's readable thinking shows as a collapsed "Thought"; hook errors and hook messages,
  Stop hook feedback, CLAUDE.md files loaded on demand, session recaps and Claude Code's warnings
  show as small rows that expand.
- Chat: slash command output and shell-mode commands (`!`) show in the conversation as rows with
  their output.
- Chat: Claude Code's menus, pickers and dialogs show as cards in the chat and are answered there;
  slash commands no longer open the terminal drawer.
- Chat: Run in background for a running shell command, while Claude Code offers it.
- Chat: an Esc button on cards leaves a menu or declines a question or permission prompt.
- Handover: long sessions no longer fail with "Handover transcript exceeds 64 MiB"; the limit is
  now 2 GiB.
- Handover: `/switch-handover:switch-compact` and `claude-switch compact --open` open the new chat as
  a tab in the running Claude Switch window, beside the source session's tab, instead of a window of
  its own; only when Switch is not running do they start it (with its saved tabs and the new one).
- Settings › Handovers: **New chat starts as** is a list of the agents your accounts really have
  (**None (plain Claude)** first) and **Summary written by** names the OpenCode model. A saved
  agent the target account lacks is shown as missing and skipped with a notice instead of failing
  the launch with `--agent '…' not found`.
- Tabs: renaming a tab that runs a Claude session renames the session (list, search, restored
  tabs) and, at an idle prompt and through the Chat view's usual checks, Claude Code's own name
  with `/rename`, so other sessions can reach it by that name. Plain terminal tabs keep a
  label-only rename.
- Handover: **Hand over automatically** (Off, Ask first, Automatic) in Settings › Handovers replaces
  Claude Code's compaction in Claude Switch's own tabs: at the threshold, after the turn, the new
  chat opens from a handover in the same account, linked to the old one, which stays open marked
  handed over.

## 0.1.1 (2026-11-26)

- Chat: a clearer conversation (your messages in tinted bubbles, Claude's answers without a card,
  tool calls as compact rows grouped under their answer) in the Chat and Session views, the CLI's live
  working line with Stop, and Markdown files that open in a side panel.
- Chat message box: chips for model, effort, context, compact and clear; a "/" list of built-in,
  skill, plugin and own commands with pins; clipboard pictures that really send (the CLI confirms the
  picture before Enter); a queue while a turn runs, with Send now and Cancel; notices that stay until
  their cause changes; word wrap; mouse selection and a context menu; Up and Down through earlier
  prompts, keeping the draft.
- Details panel: clickable Tasks, Agents and Files rows; a Shells tab for background shells with
  their output and Stop through the CLI's own manager; an Outputs tab for artifacts, pull requests,
  links, pictures and files; scrollbars that can be dragged.
- Handover: choose the new chat's agent (dialog, `--agent`, the plugin's `agent:<name>`, a default in
  Settings), and the new chat is named after its source (`--name`). The plugin finds the installed
  app when it is not on PATH, and Setup can add Claude Switch to PATH.
- Accounts: usage is read again after a sign-in; Open Desktop finds the Store-installed app; usage
  rows show figures or say why not; Refresh is no longer cut off.
- Settings in category pages; menus no longer highlight the row underneath; the pointer no longer
  stays a text caret; "Archive session…" says why it is unavailable.
- Not yet tried against a live Claude Code session or the real desktop app; see docs/TODO.md.

## 0.1.0 (2026-11-26)

- First release: a session list, terminal tabs and a Chat layout for Claude Code on Windows, with several accounts, usage at a glance and handovers through OpenCode.
- Known: a translucent terminal background (off by default) has rendering defects with colour emoji and page transitions; fixes are pending in the shared toolkit.
- Prefer scoped subscription limits and show the weekly model breakdown when available.
- Block Chat sends during recognized CLI interactions, require a terminal preview for clipboard
  pictures, and make persisted usage opt-out and restore snapshots safe across cooperating windows.
- Keep project discovery on local fixed drives and open Projects at the top instead of scrolling
  to the old selection. Require native Claude executables and verify the session before handing it over.
- Harden session launch and Desktop handoff, recover damaged settings without re-enabling usage,
  and protect marks, transcripts, profile files and the usage cache from unsafe updates.
- Add Compact and open: an opt-in OpenCode handover with an editable preview, local history, new-session provenance, CLI, and a Claude Code plugin.
- Add a Windows session list with working folders and launch controls for Claude Code.
- Add embedded CLI terminal tabs and separate account logins with shared settings.
- Refuse duplicate session launches and offer to open a separate copy in the CLI.
- Add Recent and Folders views, last-prompt search, status filters and a fuzzy quick switcher.
- Add local names, pins, reversible hiding, and opt-in Claude name suggestions with review.
- Show subscription usage and reset times in the account selector and Accounts page, with a local figures-only cache and a **Show usage** opt-out.
- Use Claude clay as the default accent and show the Claude Switch icon beside the window title.
- Add cancellable conversation search with highlighted excerpts and a selected-session size summary with reported or estimated token counts.
- Restore the normal window rectangle and maximized state; center on the primary work area if no monitor shows at least 100 × 100 px of it.
- Offer to reopen Claude terminal tabs in order, with Ask/Always/Never and the normal resume safeguards.
- Add drag-to-reorder and terminal-tab context menus for account forks, handovers, rename and close; Ctrl+1–9 selects a tab from Sessions.
- Remember the last successful account and app per folder, with editable defaults for Open and New session.
- Add reversible single-session, folder and age-based archiving, an Archived filter, and local Markdown summaries with prompt excerpts and statistics.
- Mark background tabs that need attention and optionally notify through the Windows tray.
- Warn at 80% and 95% usage and offer handover to another account with recent headroom.
- Add an optional tray menu with per-account usage and close-to-tray behavior.
- Share the OpenCode CLI client as the MIT-licensed `opencode-cli` crate; show OpenCode health, running sessions, a guarded restart, and the optional Config Studio shortcut in Settings → OpenCode.
- Open oversized handovers by absolute Markdown path rather than refusing the launch at the Windows command-line safety threshold.
- Add an optional Chat layout over the same embedded Claude Code terminal, with draft recall, tool previews, a details panel and an automatic terminal drawer.
- Group sessions by repository and worktree, proven thread, or topic; add optional Recent sittings and local Related suggestions in details. Keep the original Folder option and add opt-in topics to the existing name review.
- Add live terminal cursor shape and blink preferences, and adjustable background opacity with Mica or Acrylic, including in Chat’s drawer.
- Retry delayed transcript attachment, keep stale tab identities out of routing, and keep Chat drafts blocked until real panes verify their session.
- Keep local name, pin and hide changes out of the UI when saving fails; use the sanitized Explorer path for Session-view Reveal.
- Recognize permission prompts near the input cursor, count picture-only turns for Waiting, and avoid transcript discovery per key and unchanged tray updates per wake.
- Reuse unchanged session and Desktop records on refresh, discard transcript reads that race an append, and reuse one Related index per scan.
- Recheck project metadata on refresh and recognize repository folders such as `comp`.
- Make secondary first sign-in prepare shared projects and MCP settings, claim account folders exclusively,
  and retry worker-account bridge sync from committed settings.
- Keep terminal input active during usage hover; honor cross-window opt-out before displaying figures or
  accepting readings, and retry local usage-lock contention and unsaved warning claims.
- Show unknown session ownership or OpenCode counts when evidence is missing, wait for a committed restart
  on exit, and report window-setting save failures and oversized metadata files.
