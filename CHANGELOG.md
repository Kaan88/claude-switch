# Changelog

## Unreleased

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
  terminal's own choices; the working line says "Waiting for your approval".
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
