# Changelog

## Unreleased

- Text boxes (search, name and folder dialogs, Settings) select with the mouse: drag, Shift+click,
  double-click a word, triple-click all, and a right-click menu with Cut, Copy, Paste, Select all.
- The conversation in Chat and Session view can be selected across messages and copied with
  Ctrl+C or the right-click menu (Copy, Select all, Copy message, Copy as Markdown); code blocks
  have a Copy button.
- Chat composer: `@` completes paths in the session folder, Ctrl+R searches earlier prompts, and
  Claude Code's grey next-prompt suggestion appears as a chip that fills the draft.
- The Chat message box is three lines tall and grows with the draft up to about 40% of the view.
- The two archives have distinct names: the **Archived here** chip is Switch's own archive;
  **Show sessions archived in the desktop app** and the details row **Desktop archive: yes** are
  Claude Desktop's.
- Colour pickers in Settings have Apply and Cancel: Enter or a click outside applies the colour,
  Escape restores the old one. Before, the chosen colour was not applied.
- Unknown command-line options are refused with a short usage text instead of being ignored (an
  ignored typo started the real app); `--help` lists every option, hidden ones marked.
- Handover: long sessions no longer fail with "Handover transcript exceeds 64 MiB"; the limit is
  now 2 GiB.

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
