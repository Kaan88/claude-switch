//! Search controls, emphasized excerpts, and the independent session-size block.

use claude_switch_core::fulltext::Hits;
use claude_switch_core::stats::Tokens;
use mdshared_ui::gfx::{Face, Painter, Rect, Wrap};
use mdshared_ui::tokens::legible;

use crate::conversation_search::SNIPPET_LINE_H;
use crate::state::{AppState, Target};

pub fn toggle(st: &mut AppState, p: &mut Painter, r: Rect) {
    let pal = *st.pal();
    let target = Target::SearchConversation;
    let on = st.conversation_search.on;
    let fill = if st.pressed == Some(target) {
        pal.press
    } else if st.hover == Some(target) {
        pal.hover
    } else if on {
        pal.on
    } else {
        pal.field
    };
    p.frame(r, 5.0, fill, if on { pal.accent } else { pal.line });
    let text = if r.w < 100.0 { "Chat" } else { "In conversations" };
    p.text_center(Face::Semibold, 11.5, r, text, if on { pal.text } else { pal.muted });
    st.hits.push((r, target));
}

pub fn search_status(st: &AppState, p: &mut Painter, area: Rect, count: usize) {
    let pal = st.pal();
    let x = area.x + 12.0;
    let y = area.y + 22.0;
    let title = "In conversations";
    let width = p.measure(Face::Semibold, 12.0, title);
    p.text(Face::Semibold, 12.0, x, y, title, pal.text, width + 1.0);
    let status = st.conversation_search.status(st.search.text(), count);
    p.text(Face::Ui, 12.0, x + width + 10.0, y, &status, pal.muted, (area.w - width - 34.0).max(0.0));
}

pub fn snippets(st: &AppState, p: &mut Painter, area: Rect, hits: &Hits, selected: bool) {
    let pal = st.pal();
    let color = legible(pal.accent, if selected { pal.on } else { pal.pane }, 4.5);
    let mut y = area.y;
    for hit in &hits.shown {
        // Keep the first match on-screen when a narrow pane cannot fit the preceding context.
        let first = hit.matches.first().map_or(0, |r| r.start);
        let prefix = &hit.line[..first];
        let clipped = p.measure(Face::Ui, 12.0, prefix) > area.w * 0.4;
        let (mut at, mut x) = if clipped {
            (first, area.x + p.text(Face::Ui, 12.0, area.x, y, "…", pal.muted, area.w))
        } else {
            (0, area.x)
        };
        for span in &hit.matches {
            let context = &hit.line[at..span.start];
            x += p.text(Face::Ui, 12.0, x, y, context, pal.muted, (area.right() - x).max(0.0));
            // The toolkit's drawn width excludes hanging whitespace. Measure it between markers
            // so styled runs don't collapse the spaces before a match.
            let spaces = &context[context.trim_end().len()..];
            if !spaces.is_empty() {
                x += (p.measure(Face::Ui, 12.0, &format!("|{spaces}|")) - p.measure(Face::Ui, 12.0, "||")).max(0.0);
            }
            x += p.text(Face::Semibold, 12.0, x, y, &hit.line[span.clone()], color, (area.right() - x).max(0.0));
            at = span.end;
            if x >= area.right() {
                break;
            }
        }
        p.text(Face::Ui, 12.0, x, y, &hit.line[at..], pal.muted, (area.right() - x).max(0.0));
        y += SNIPPET_LINE_H;
    }
    if hits.more > 0 {
        p.text(Face::Semibold, 11.0, area.x, y, &format!("+{} more", hits.more), pal.muted, area.w);
    }
}

fn token_label(tokens: Tokens) -> String {
    let mut digits = tokens.count.to_string();
    for at in (1..digits.len()).rev().filter(|at| (digits.len() - at).is_multiple_of(3)).collect::<Vec<_>>() {
        digits.insert(at, ',');
    }
    if tokens.about { format!("about {digits}") } else { digits }
}

pub fn size(st: &AppState, p: &mut Painter, x: f32, y: f32, w: f32) -> f32 {
    let pal = st.pal();
    p.fill(Rect::new(x, y - 9.0, w, p.px()), pal.line);
    p.text(Face::Semibold, 13.0, x, y + 8.0, "Conversation size", pal.text, w);
    let Some(stats) = st.session_size.current else {
        let text = if st.session_size.pending { "Counting\u{2026}" } else { "Conversation size unavailable." };
        p.text(Face::Ui, 12.0, x, y + 28.0, text, pal.muted, w);
        return y + 49.0;
    };
    let counts =
        [(stats.turns, "turn"), (stats.tools, "tool call"), (stats.pictures, "picture"), (stats.subagents, "subagent")]
            .map(|(n, label)| format!("{n} {label}{}", if n == 1 { "" } else { "s" }))
            .join("  ·  ");
    let mut next = y + 18.0;
    next += p.paragraph_wrapped(Face::Ui, 12.0, x, next, w, &counts, pal.muted, Wrap::Words);
    let tokens =
        format!("Context now  {}     ·     Output so far  {}", token_label(stats.context), token_label(stats.output));
    next += p.paragraph_wrapped(Face::Semibold, 12.0, x, next + 2.0, w, &tokens, pal.text, Wrap::Words);
    next + 14.0
}
