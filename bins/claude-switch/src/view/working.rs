//! Read the CLI's status from visible terminal cells. Display is advisory and cannot authorize Stop.
use std::time::{Duration, Instant};

use claude_switch_core::transcript::{ChatInfo, ChatPhase};
use mdshared_term::Screen;
use mdshared_ui::gfx::{Face, Painter, Rgb};
use mdshared_ui::host::Cx;

use crate::state::{AppState, Msg};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Footer {
    pub label: String,
    pub stop: Option<Turn>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Turn(pub u64, pub u64);

#[derive(Default)]
pub(super) struct Working {
    turn: Option<(Turn, Instant)>,
}

#[derive(Debug, PartialEq, Eq)]
struct Line<'a> {
    word: &'a str,
    elapsed: Option<&'a str>,
    tokens: Option<&'a str>,
}

impl Working {
    fn update(
        &mut self,
        now: Instant,
        info: &ChatInfo,
        allowed: bool,
        screen: &Screen,
        can_stop: bool,
        waiting: Option<&'static str>,
    ) -> (Option<Footer>, Option<Instant>) {
        let phase = info.latest.map(|(_, phase)| phase);
        let Some(prompt) = info.prompt.filter(|_| allowed && phase != Some(ChatPhase::EndTurn)) else {
            self.turn = None;
            return (None, None);
        };
        if let Some(label) = waiting {
            // Claude is not working: the turn waits for an answer or an approval. The transcript
            // may not hold the call yet (Claude Code 2.1.332 writes an `AskUserQuestion` only once
            // it is answered), so the screen's own menu counts.
            return (Some(Footer { label: label.into(), stop: None }), None);
        }
        let (row, hint) = visible(screen);
        let line = row.as_deref().and_then(parse);
        if phase != Some(ChatPhase::Active) && info.pending.is_none() && line.is_none() && !hint {
            self.turn = None;
            return (None, None);
        }
        let key = Turn(info.generation, prompt);
        if self.turn.is_none_or(|(old, _)| old != key) {
            self.turn = Some((key, now));
        }
        let (_, since) = self.turn.expect("working turn started");
        let age = now.saturating_duration_since(since).as_secs();
        let deadline = line.as_ref().is_none_or(|l| l.elapsed.is_none()).then(|| since + Duration::from_secs(age + 1));
        let local_elapsed = || match (age / 3600, (age / 60) % 60, age % 60) {
            (0, 0, seconds) => format!("{seconds}s"),
            (0, minutes, seconds) => format!("{minutes}m {seconds:02}s"),
            (hours, minutes, seconds) => format!("{hours}h {minutes:02}m {seconds:02}s"),
        };
        let label = if let Some(line) = line {
            let elapsed = line.elapsed.map_or_else(local_elapsed, str::to_owned);
            if let Some(tokens) = line.tokens {
                format!("{} ({elapsed} · {tokens})", line.word)
            } else {
                format!("{} ({elapsed})", line.word)
            }
        } else {
            format!("Working… ({})", local_elapsed())
        };
        (Some(Footer { label, stop: can_stop.then_some(key) }), deadline)
    }
}

impl AppState {
    pub(super) fn update_working(&mut self, cx: &mut Cx<'_, Msg>) {
        let area = self.session_view.pane_area;
        let Some(tab) = self.tabs.shown_mut() else { return };
        let Some(pane) = &mut tab.pane else { return };
        let now = cx.now();
        let term = &tab.term;
        let allowed = !pane.chat.identity_unverified
            && pane.model.error.is_none()
            && term.ended.is_none()
            && (pane.chat.is_demo() || term.running())
            && !term.screen.modes().alt_screen;
        let waiting =
            super::chat::question::waiting_label(&pane.model, &term.screen).or_else(|| pane.chat.menu_waiting());
        // A permission prompt hides the working line unless it is the pending call's own. So does
        // the CLI's idle input: the transcript's last record reads as a running turn after an
        // interruption, and nothing is working then.
        let allowed = allowed
            && (waiting.is_some() || !crate::attention::screen_permission_prompt(&term.screen))
            && !pane.chat.screen_resting(term, now);
        let can_stop = allowed && pane.chat.stop_ready(&pane.model.chat, term, &tab.program_title, now);
        let (footer, wake) = pane.working.update(now, &pane.model.chat, allowed, &term.screen, can_stop, waiting);
        if pane.ui.set_working(footer) {
            cx.request_redraw(area);
        }
        if let Some(at) = wake {
            cx.wake_at(at);
        }
    }
}

/// The spinner glyphs the CLI cycles through; the slot is as wide as the widest of them.
const GLYPHS: [&str; 12] = ["·", "✢", "✦", "✧", "✳", "✶", "✹", "✺", "✻", "✽", "*", "⏳"];

/// `(x, text, slot)`: a piece of a working line; with a slot, it is centred in it.
type Piece<'a> = (f32, &'a str, Option<f32>);

/// Where each piece of a working line goes: the spinner glyph centred in a fixed slot, the text
/// after it from a fixed x, and every digit centred in a slot as wide as the widest digit, so
/// neither the animation nor a ticking counter moves what follows.
fn pieces<'a>(label: &'a str, measure: &mut dyn FnMut(&str) -> f32) -> (Vec<Piece<'a>>, f32) {
    let glyph_slot = GLYPHS.iter().map(|g| measure(g)).fold(0.0, f32::max);
    let digit = ('0'..='9').map(|d| measure(d.encode_utf8(&mut [0; 4]))).fold(0.0, f32::max);
    let gap = measure(" ");
    let mut out = Vec::new();
    // A glyph is the first character when a space follows it; the fallback has none, and its
    // text still starts where a glyph's would.
    let rest = match label.split_once(' ') {
        Some((glyph, rest)) if glyph.chars().count() <= 2 && !glyph.starts_with(char::is_alphanumeric) => {
            out.push((0.0, glyph, Some(glyph_slot)));
            rest
        }
        _ => label,
    };
    let mut x = glyph_slot + gap;
    let mut start = 0;
    for (i, c) in rest.char_indices() {
        if c.is_ascii_digit() {
            if start < i {
                let run = &rest[start..i];
                out.push((x, run, None));
                x += measure(run);
            }
            out.push((x, &rest[i..=i], Some(digit)));
            x += digit;
            start = i + 1;
        }
    }
    if start < rest.len() {
        let run = &rest[start..];
        out.push((x, run, None));
        x += measure(run);
    }
    (out, x)
}

/// A copied CLI row that starts with a spinner glyph and a space ("✻ Scanning local sessions…"):
/// drawn with `draw_line`, so the glyph's animation does not move the text after it.
pub(super) fn spinner_row(row: &str) -> bool {
    let mut chars = row.chars();
    let glyph =
        chars.next().is_some_and(|g| GLYPHS.iter().any(|s| s.starts_with(g)) || ('\u{2801}'..='\u{28ff}').contains(&g));
    glyph && chars.next() == Some(' ')
}

/// Draws the working line from `x`; past `max` it is shown elided as plain text.
pub(super) fn draw_line(p: &mut Painter, size: f32, x: f32, cy: f32, label: &str, color: Rgb, max: f32) {
    let (pieces, width) = pieces(label, &mut |t| p.measure(Face::Ui, size, t));
    if width > max {
        p.text(Face::Ui, size, x, cy, label, color, max.max(0.0));
        return;
    }
    for (at, text, slot) in pieces {
        let offset = slot.map_or(0.0, |w| (w - p.measure(Face::Ui, size, text)) / 2.0);
        p.text(Face::Ui, size, x + at + offset, cy, text, color, f32::INFINITY);
    }
}

/// `row_text` is indexed in the active screen, not in the saved scrollback.
fn visible(screen: &Screen) -> (Option<String>, bool) {
    let cursor = screen.cursor().row;
    let rows = cursor.saturating_sub(12)..(cursor + 5).min(screen.rows());
    let mut hint = false;
    for i in rows.rev() {
        let line = screen.row_text(i);
        if i == cursor && line.trim_start().starts_with(['❯', '>']) {
            continue;
        }
        if parse(&line).is_some() {
            return (Some(line), true);
        }
        hint |= interrupt(line.trim_start_matches([' ', '·', '•', '─']));
    }
    (None, hint)
}

/// `row` reads as the CLI's working line: a spinner glyph, a word, "…" and at most its timer,
/// token count and interrupt hint.
pub(in crate::view) fn working_row(row: &str) -> bool {
    parse(row).is_some()
}

fn parse(row: &str) -> Option<Line<'_>> {
    let row = row.trim();
    if row.len() > 240 || row.chars().any(char::is_control) {
        return None;
    }
    let glyph = row.chars().next()?;
    if !matches!(
        glyph,
        '·' | '✢' | '✦' | '✧' | '✳' | '✶' | '✹' | '✺' | '✻' | '✽' | '⏳' | '*' | '\u{2801}'..='\u{28ff}'
    ) {
        return None;
    }
    let head = row[glyph.len_utf8()..].trim_start_matches('\u{fe0f}').strip_prefix(char::is_whitespace)?;
    let end = head.find('…').map(|i| (i, 3)).or_else(|| head.find("...").map(|i| (i, 3)))?;
    let word = head[..end.0].trim();
    if word.is_empty()
        || word.len() > 72
        || !word.starts_with(char::is_alphabetic)
        || !word.chars().all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '\'' | '’'))
    {
        return None;
    }
    let word = row.get(..row.len() - head.len() + end.0 + end.1)?;
    let tail = head[end.0 + end.1..].trim();
    let (metrics, after) = if let Some(group) = tail.strip_prefix('(') {
        let (fields, after) = group.split_once(')')?;
        (fields, after.trim())
    } else {
        ("", tail)
    };
    let after = after.trim_start_matches([' ', '·', '•', '|', '-', '—', '–']);
    if !after.is_empty() && !interrupt(after) {
        return None;
    }
    let mut elapsed = None;
    let mut tokens = None;
    for part in metrics.split(['·', '•', '|']).map(str::trim) {
        if elapsed.is_none() && time(part) {
            elapsed = Some(part);
        } else if tokens.is_none() && token_count(part) {
            tokens = Some(part);
        }
    }
    if !metrics.is_empty() && elapsed.is_none() && tokens.is_none() && !interrupt(metrics) {
        return None;
    }
    Some(Line { word, elapsed, tokens })
}

fn interrupt(s: &str) -> bool {
    let s = s.trim().trim_matches(['(', ')', '.', ' ']);
    ["esc to interrupt", "escape to interrupt", "ctrl+c to interrupt", "ctrl-c to interrupt"]
        .iter()
        .any(|hint| s.eq_ignore_ascii_case(hint))
}

fn time(s: &str) -> bool {
    let mut last = 4;
    let mut found = false;
    for part in s.split_whitespace() {
        let Some((suffix, n)) = part.as_bytes().split_last() else { return false };
        let rank = match suffix {
            b'h' => 3,
            b'm' => 2,
            b's' => 1,
            _ => return false,
        };
        if rank >= last || n.is_empty() || n.len() > 5 || !n.iter().all(u8::is_ascii_digit) {
            return false;
        }
        last = rank;
        found = true;
    }
    found
}

fn token_count(s: &str) -> bool {
    let Some(n) = s.strip_suffix(" tokens").or_else(|| s.strip_suffix(" token")) else { return false };
    let n = n.trim_start_matches(['↑', '↓', '↕', ' ']);
    let n = n.strip_suffix(['k', 'K', 'm', 'M']).unwrap_or(n);
    n.len() <= 20
        && n.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && n.as_bytes().last().is_some_and(u8::is_ascii_digit)
        && n.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b','))
        && !["..", ",,", ".,", ",."].iter().any(|separator| n.contains(separator))
}

#[cfg(test)]
mod tests {
    use super::*;
    const ASKING: &str = "Waiting for your answer";

    #[test]
    fn keeps_cli_word_time_and_tokens_without_echoing_interrupt_or_thinking_hints() {
        for (text, word, time, tokens) in [
            ("✻ Shimmying… (53m 37s · ↓ 27.1k tokens)", "✻ Shimmying…", Some("53m 37s"), Some("↓ 27.1k tokens")),
            (
                " ✶ Pondering... (esc to interrupt · 4s · ↑ 1,234 tokens) ",
                "✶ Pondering...",
                Some("4s"),
                Some("↑ 1,234 tokens"),
            ),
            ("✳ Calculating… (1h 3m 07s) — Esc to interrupt", "✳ Calculating…", Some("1h 3m 07s"), None),
            ("✽ Working… (↓ 1 token) · Ctrl+C to interrupt", "✽ Working…", None, Some("↓ 1 token")),
            ("✢ Thinking hard…", "✢ Thinking hard…", None, None),
            ("* Brewing... (Escape to interrupt)", "* Brewing...", None, None),
        ] {
            assert_eq!(parse(text), Some(Line { word, elapsed: time, tokens }), "{text}");
        }
        for text in [
            "❯ ✻ Shimmying… (99s · ↓ 27.1k tokens)",
            "● Wrote the report… (4s · 10 tokens)",
            "✻ Opens a file… (C:\\notes.md)",
            "✻ Shimmying… (truncated",
            "✻ Shimmying… (4s) and the rest of the output",
            "esc to interrupt",
        ] {
            assert_eq!(parse(text), None, "{text}");
        }
    }

    #[test]
    fn the_glyph_and_counters_keep_their_slots_so_the_text_after_them_stays_put() {
        // Glyphs and digits of different widths, as in a proportional font.
        let mut measure = |t: &str| {
            t.chars()
                .map(|c| match c {
                    '·' => 3.0,
                    '✻' => 11.0,
                    '⏳' => 12.0,
                    '1' => 4.0,
                    '0'..='9' => 7.0,
                    _ => 6.0,
                })
                .sum::<f32>()
        };
        let at = |label: &str, piece: &str, measure: &mut dyn FnMut(&str) -> f32| {
            let (pieces, _) = pieces(label, measure);
            pieces.iter().find(|(_, t, _)| *t == piece).map(|(x, _, _)| *x).unwrap()
        };
        let word = at("· Billowing… (13s · ↓ 1.1k tokens)", "Billowing… (", &mut measure);
        assert_eq!(word, at("✻ Billowing… (13s · ↓ 1.1k tokens)", "Billowing… (", &mut measure));
        assert_eq!(word, at("Working… (0s)", "Working… (", &mut measure), "the fallback starts there too");
        assert_eq!(word, 12.0 + 6.0, "after the widest glyph and a space");
        let tokens = |l: &str, m: &mut dyn FnMut(&str) -> f32| at(l, "s · ↓ ", m);
        assert_eq!(
            tokens("✻ Billowing… (13s · ↓ 1.1k tokens)", &mut measure),
            tokens("✻ Billowing… (40s · ↓ 1.1k tokens)", &mut measure),
            "a 1 is as wide as a 4"
        );
        let (pieces, _) = pieces("✻ Billowing… (13s)", &mut measure);
        assert_eq!(pieces[0], (0.0, "✻", Some(12.0)), "the glyph is centred in its slot");
        assert!(
            pieces
                .iter()
                .filter(|(_, t, _)| t.len() == 1 && t.as_bytes()[0].is_ascii_digit())
                .all(|p| p.2 == Some(7.0))
        );
    }

    #[test]
    fn reports_only_visible_status_and_resets_the_fallback_when_a_turn_ends() {
        let mut screen = Screen::new(96, 6, 32);
        screen.feed("✻ Old spinner… (2s)\r\n".as_bytes());
        screen.feed(b"\r\n\r\n\r\n\r\n\r\n");
        screen.feed("❯ ✻ Prompt echo… (88s)".as_bytes());
        assert_eq!(visible(&screen), (None, false));
        let now = Instant::now();
        let mut info = ChatInfo::default();
        info.prompt = Some(10);
        info.latest = Some((10, ChatPhase::Active));
        let mut state = Working::default();
        let (first, wake) = state.update(now, &info, true, &screen, true, None);
        assert_eq!(first.unwrap().label, "Working… (0s)");
        assert_eq!(wake, Some(now + Duration::from_secs(1)));
        assert_eq!(
            state.update(now + Duration::from_secs(3), &info, true, &screen, false, None).0.unwrap().label,
            "Working… (3s)"
        );
        screen.feed("\r\n✻ Shimmying… (53m 37s · ↓ 27.1k tokens)".as_bytes());
        let (live, wake) = state.update(now + Duration::from_secs(4), &info, true, &screen, true, None);
        assert_eq!(live.unwrap().label, "✻ Shimmying… (53m 37s · ↓ 27.1k tokens)");
        assert_eq!(wake, None);
        info.latest = Some((30, ChatPhase::EndTurn));
        assert_eq!(state.update(now + Duration::from_secs(5), &info, true, &screen, false, None), (None, None));
        assert_eq!(state.update(now, &info, false, &screen, true, None), (None, None));
    }

    #[test]
    fn a_pending_question_reads_as_waiting_for_the_answer_without_stop() {
        let screen = Screen::new(60, 6, 0);
        let mut info = ChatInfo::default();
        info.prompt = Some(10);
        info.latest = Some((20, ChatPhase::Active));
        info.pending = Some(20);
        let mut state = Working::default();
        let footer = state.update(Instant::now(), &info, true, &screen, true, Some(ASKING)).0.unwrap();
        assert_eq!(footer, Footer { label: ASKING.into(), stop: None });
        // Claude Code writes an `AskUserQuestion` to the transcript only once it is answered:
        // the screen's own menu is enough to say the turn waits.
        info.pending = None;
        let footer = state.update(Instant::now(), &info, true, &screen, true, Some(ASKING)).0.unwrap();
        assert_eq!(footer, Footer { label: ASKING.into(), stop: None });
        assert!(state.update(Instant::now(), &info, true, &screen, true, None).0.unwrap().label.starts_with("Working"));
    }
}
