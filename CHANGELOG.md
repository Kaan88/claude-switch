# Changelog

## 0.1.0 (Unreleased)

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
- Share the OpenCode CLI client as the MIT-licensed `opencode-cli` crate; show OpenCode health, running sessions, a guarded restart, and the optional Config Studio shortcut in Settings → Workers.
- Open oversized handovers by absolute Markdown path rather than refusing the launch at the Windows command-line safety threshold.
- Add an optional Chat layout over the same embedded Claude Code terminal, with draft recall, tool previews, a details panel and an automatic terminal drawer.
- Group sessions by repository and worktree, proven thread, or topic; add optional Recent sittings and local Related suggestions in details. Keep the original Folder option and add opt-in topics to the existing name review.
- Add live terminal cursor shape and blink preferences, and adjustable background opacity with Mica or Acrylic, including in Chat’s drawer.
