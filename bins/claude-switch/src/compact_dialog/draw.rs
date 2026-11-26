use super::*;
use crate::dialogs::Draw;
use mdshared_ui::gfx::{Face, Painter, Wrap};
use mdshared_ui::overlay::{DIALOG_PAD, dialog_card};

pub(super) const LINE: f32 = 18.0;
const FONT: f32 = 12.5;

pub fn draw_history_row(st: &mut AppState, p: &mut Painter, id: &str, x: f32, y: f32, w: f32, area: Rect) -> f32 {
    let pal = *st.pal();
    let files = st.compact.history.as_ref().filter(|(sid, _)| sid == id).map(|(_, files)| files.as_slice());
    let label =
        files.filter(|f| !f.is_empty()).map_or_else(|| "Handovers…".into(), |f| format!("Handovers ({})…", f.len()));
    let r = Rect::new(x, y - 13.0, 132.0, 27.0);
    let target = Target::Handovers;
    mdshared_ui::overlay::draw_dialog_button(
        p,
        &pal,
        r,
        &label,
        false,
        st.hover == Some(target),
        st.pressed == Some(target),
    );
    if r.bottom() <= area.bottom() {
        st.hits.push((r, target));
    }
    if let Some(latest) = files.and_then(|f| f.first()).and_then(|p| p.file_name()).and_then(|p| p.to_str()) {
        p.text(Face::Mono, 11.0, x, y + 29.0, latest, pal.muted, w);
        y + 55.0
    } else {
        y + 35.0
    }
}

impl Compact {
    pub fn draw(&mut self, d: &mut Draw<'_>, p: &mut Painter, w: f32, h: f32) {
        let card = dialog_card(660.0_f32.min(w - 28.0), 410.0_f32.min(h - 24.0), w, h);
        let footer = d.card(p, card);
        let body = Rect::new(card.x + DIALOG_PAD, card.y + 51.0, card.w - DIALOG_PAD * 2.0, footer.y - card.y - 51.0);
        d.title(
            p,
            card,
            match self.phase {
                Phase::Setup => "Compact and open",
                Phase::Consent => "Allow this provider?",
                Phase::Preview => "Review handover",
                Phase::History => "Handovers",
            },
        );
        p.text(Face::Ui, FONT, body.x, body.y + 8.0, &self.name, d.pal.muted, body.w);
        match self.phase {
            Phase::Setup => self.draw_setup(d, p, body, footer),
            Phase::Consent => self.draw_consent(d, p, body, footer),
            Phase::Preview => self.draw_preview(d, p, body, footer),
            Phase::History => self.draw_history(d, p, body, footer),
        }
        if let Some((message, error)) = &self.message {
            p.text(
                Face::Ui,
                12.0,
                body.x,
                footer.y - 12.0,
                message,
                if *error { d.pal.danger } else { d.pal.muted },
                body.w,
            );
        }
    }

    fn draw_setup(&mut self, d: &mut Draw<'_>, p: &mut Painter, body: Rect, footer: Rect) {
        if self.choice.is_some() {
            p.text(
                Face::Ui,
                FONT,
                body.x,
                body.y + 34.0,
                if self.choice == Some(Choice::Model) { "Choose an OpenCode model" } else { "Choose an account" },
                d.pal.text,
                body.w,
            );
            self.draw_options(d, p, body, body.y + 50.0);
            d.footer(p, footer, &[("‹ Back", false, if self.choice == Some(Choice::Model) { MODEL } else { ACCOUNT })]);
            return;
        }
        let busy = self.job.is_some();
        let ready = !busy && !self.models.is_empty();
        let account = self.accounts.get(self.account).map_or("No account", |a| a.1.as_str());
        let model = self.models.get(self.model).map_or("No model selected", String::as_str);
        let half = (body.w - 16.0) / 2.0;
        let y = body.y + 54.0;
        p.text(Face::Ui, FONT, body.x, y - 13.0, "OpenCode model", d.pal.muted, half);
        p.text(Face::Ui, FONT, body.x + half + 16.0, y - 13.0, "Open as", d.pal.muted, half);
        if ready {
            d.button(p, Rect::new(body.x, y, half, 32.0), &format!("{model}   ▾"), false, MODEL);
            d.button(p, Rect::new(body.x + half + 16.0, y, half, 32.0), &format!("{account}   ▾"), false, ACCOUNT);
        } else {
            p.text(Face::Ui, FONT, body.x, y + 16.0, model, d.pal.muted, half);
            p.text(Face::Ui, FONT, body.x + half + 16.0, y + 16.0, account, d.pal.muted, half);
        }
        p.text(Face::Semibold, 13.0, body.x, body.y + 115.0, "Focus (optional)", d.pal.text, body.w);
        if ready {
            self.draw_editor(d, p, Rect::new(body.x, body.y + 131.0, body.w, (body.h - 183.0).max(36.0)), false);
        }
        let note = if busy {
            if self.data.is_some() && !self.models.is_empty() {
                "Working… You can cancel without opening a new session."
            } else {
                "Reading the transcript and OpenCode model list…"
            }
        } else {
            "Focus changes emphasis only. You’ll review the handover before opening it."
        };
        p.text(Face::Ui, 12.0, body.x, footer.y - 33.0, note, d.pal.muted, body.w);
        let mut buttons = Vec::new();
        if ready {
            buttons.push(("Make handover", true, OK));
        }
        buttons.push(("Cancel", false, CANCEL));
        if !busy && self.program.is_some() {
            buttons.push(("Refresh models", false, REFRESH));
        }
        d.footer(p, footer, &buttons);
    }

    fn draw_consent(&self, d: &mut Draw<'_>, p: &mut Painter, body: Rect, footer: Rect) {
        let provider = self.models.get(self.model).and_then(|m| compact::provider(m)).unwrap_or("the provider");
        let prompt = format!("Assistant text and optional focus are sent to {provider} through OpenCode.");
        let next = p.paragraph_wrapped(Face::Ui, 15.0, body.x, body.y + 43.0, body.w, &prompt, d.pal.text, Wrap::Words);
        p.paragraph_wrapped(
            Face::Ui,
            13.0,
            body.x,
            body.y + 64.0 + next,
            body.w,
            "The request also includes fixed tool categories, not tool arguments, command bodies or file paths from tool records. \
             Assistant text and your focus can themselves contain private details. Verbatim user prompts are added locally. \
             You can forget this consent in Settings.",
            d.pal.muted,
            Wrap::Words,
        );
        d.footer(p, footer, &[("Allow and make handover", true, OK), ("Back", false, BACK), ("Cancel", false, CANCEL)]);
    }

    fn draw_preview(&mut self, d: &mut Draw<'_>, p: &mut Painter, body: Rect, footer: Rect) {
        if let Some(choice) = self.choice {
            let title = match choice {
                Choice::Agent if self.job.is_some() => "Reading CLI agents…",
                Choice::Agent if self.agents.is_empty() => "No custom CLI agents found",
                Choice::Agent => "Choose an agent for the CLI session",
                _ => "Choose an account",
            };
            p.text(Face::Ui, FONT, body.x, body.y + 34.0, title, d.pal.text, body.w);
            self.draw_options(d, p, body, body.y + 50.0);
            d.footer(p, footer, &[("‹ Back", false, BACK)]);
            return;
        }
        let (chars, tokens) = compact::size(&self.preview.text);
        let account = self.accounts.get(self.account).map_or("no account", |a| a.1.as_str());
        p.text(
            Face::Ui,
            FONT,
            body.x,
            body.y + 33.0,
            &format!("{chars} characters  ·  about {tokens} tokens"),
            d.pal.text,
            body.w,
        );
        let half = (body.w - 12.0) / 2.0;
        let y = body.y + 47.0;
        d.button(p, Rect::new(body.x, y, half, 28.0), &format!("Open as: {account}   ▾"), false, ACCOUNT);
        d.button(
            p,
            Rect::new(body.x + half + 12.0, y, half, 28.0),
            &format!("Agent: {}   ▾", self.agent_label()),
            false,
            AGENT,
        );
        let note = p.paragraph_wrapped(
            Face::Ui,
            12.0,
            body.x,
            body.y + 81.0,
            body.w,
            "Agent choice (Alt+A) is CLI-only. There is no supported Desktop launch option.",
            d.pal.muted,
            Wrap::Words,
        );
        let mut editor_y = 85.0 + note;
        if crate::compact_open::uses_file(&self.preview.text) {
            let h = p.paragraph_wrapped(
                Face::Ui,
                12.0,
                body.x,
                body.y + editor_y,
                body.w,
                "Too long to pass directly: the new session will read it from the file",
                d.pal.muted,
                Wrap::Words,
            );
            editor_y += h + 4.0;
        }
        self.draw_editor(d, p, Rect::new(body.x, body.y + editor_y, body.w, (body.h - 34.0 - editor_y).max(0.0)), true);
        d.footer(
            p,
            footer,
            &[
                ("Open in CLI", true, OK),
                ("Cancel", false, CANCEL),
                ("Back", false, BACK),
                ("Save as…", false, SAVE),
                ("Copy", false, COPY),
            ],
        );
    }

    fn draw_history(&self, d: &mut Draw<'_>, p: &mut Painter, body: Rect, footer: Rect) {
        if self.history.is_empty() {
            p.text(Face::Ui, 13.0, body.x, body.y + 44.0, "No handovers saved for this session.", d.pal.muted, body.w);
        } else {
            self.draw_options(d, p, body, body.y + 28.0);
        }
        d.footer(p, footer, &[("Close", true, CANCEL)]);
    }

    fn draw_options(&self, d: &mut Draw<'_>, p: &mut Painter, body: Rect, mut y: f32) {
        for i in self.scroll..(self.scroll + PAGE).min(self.count()) {
            let row = Rect::new(body.x, y, body.w, 27.0);
            if self.choice == Some(Choice::Agent) {
                let info = i.checked_sub(1).and_then(|i| self.agents.get(i));
                let (name, description) =
                    info.map_or((agents::DEFAULT, "No agent override"), |a| (a.name.as_str(), a.description.as_str()));
                d.button(p, row, "", self.agent_focus == i, OPTION + i as u32);
                let color = if self.agent_focus == i { d.pal.on_accent } else { d.pal.text };
                let mark = if self.agent == i { "✓  " } else { "" };
                // Give the name its own line so descriptions never push it out of view.
                p.text(Face::Semibold, 11.5, row.x + 9.0, row.y + 7.0, &format!("{mark}{name}"), color, row.w - 18.0);
                p.text(Face::Ui, 10.5, row.x + 9.0, row.y + 20.0, description, color, row.w - 18.0);
                y += 29.0;
                continue;
            }
            let (label, selected) = match self.choice {
                Some(Choice::Model) => (self.models[i].as_str(), self.model == i),
                Some(Choice::Account) => (self.accounts[i].1.as_str(), self.account == i),
                Some(Choice::Agent) => unreachable!(),
                None => (self.history[i].file_name().and_then(|n| n.to_str()).unwrap_or("Handover"), false),
            };
            d.button(p, row, label, selected, OPTION + i as u32);
            y += 29.0;
        }
        if self.scroll > 0 {
            d.button(p, Rect::new(body.x, y + 4.0, 103.0, 27.0), "‹ Previous", false, PREV);
        }
        if self.scroll + PAGE < self.count() {
            d.button(p, Rect::new(body.x + 111.0, y + 4.0, 103.0, 27.0), "Next ›", false, NEXT);
        }
    }

    fn draw_editor(&mut self, d: &mut Draw<'_>, p: &mut Painter, r: Rect, preview: bool) {
        p.frame(r, 5.0, d.pal.field, d.pal.line);
        let inner = r.inset(9.0);
        let cell = p.measure(Face::Mono, FONT, "M").max(1.0);
        self.edit_area = inner;
        self.cell_width = cell;
        let editor = self.editor();
        editor.visible = ((inner.h / LINE).floor() as usize).max(1);
        editor.wrap(((inner.w / cell) as usize).max(1));
        let clip = p.clip(inner);
        let selection = editor.selection();
        let caret_row = editor.row_at(editor.caret);
        if editor.text.is_empty() && !preview {
            p.text(
                Face::Ui,
                FONT,
                inner.x,
                inner.y + LINE / 2.0,
                "What should the handover keep?",
                d.pal.muted,
                inner.w,
            );
        }
        for (i, row) in editor.rows.iter().enumerate().skip(editor.scroll).take(editor.visible) {
            let y = inner.y + (i - editor.scroll) as f32 * LINE;
            let prefix_w = |p: &mut Painter, end| p.measure(Face::Mono, FONT, &editor.text[row.start..end]);
            let (start, end) = (selection.start.max(row.start), selection.end.min(row.end));
            if start < end {
                let a = prefix_w(p, start);
                let b = prefix_w(p, end);
                p.fill(Rect::new(inner.x + a, y, b - a, LINE), d.pal.selection);
            }
            let text = &editor.text[row.clone()];
            let colour = if preview && text.starts_with("## ") { d.pal.accent } else { d.pal.text };
            p.text(Face::Mono, FONT, inner.x, y + LINE / 2.0, text, colour, inner.w);
            if i == caret_row {
                let x = inner.x + prefix_w(p, editor.caret.min(row.end));
                p.fill(Rect::new(x, y + 2.0, p.px().max(1.0), LINE - 4.0), d.pal.accent);
            }
        }
        p.restore(clip);
        if editor.rows.len() > editor.visible {
            let height = (r.h * editor.visible as f32 / editor.rows.len() as f32).max(12.0);
            let fraction = editor.scroll as f32 / (editor.rows.len() - editor.visible) as f32;
            p.rbox(Rect::new(r.right() - 5.0, r.y + fraction * (r.h - height), 3.0, height), 1.5, d.pal.muted);
        }
        d.hits.push((r, Target::Dialog(EDITOR)));
    }
}
