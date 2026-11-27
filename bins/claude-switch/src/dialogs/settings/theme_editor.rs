//! A draft is private to this Settings form. A file import also starts a draft, so it
//! can't overwrite a palette just because its display name matches.
use super::{AppState, Dialog, Form, Row};
use crate::state::Msg;
use crate::termtheme::{named, palettes};
use claude_switch_core::config::theme::{self, Colour, NamedTheme};
use mdshared_ui::settings::Value;
use mdshared_ui::{cover::contrast, gfx::rgb};

fn from_base(base: &palettes::Theme) -> NamedTheme {
    let p = crate::windowlook::palette(&base.palette, &Default::default(), None);
    let t = base.terminal;
    NamedTheme {
        name: format!("My {}", base.name),
        base: base.id.into(),
        page: Colour(p.page),
        title: Colour(p.title),
        bar: Colour(p.bar),
        pane: Colour(p.pane),
        field: Colour(p.field),
        popup: Colour(p.popup),
        text: Colour(p.text),
        muted: Colour(p.muted),
        accent: Colour(p.accent),
        selection: Colour(p.selection),
        terminal_background: Colour(rgb(t.background)),
        terminal_foreground: Colour(rgb(t.foreground)),
        terminal_selection: Colour(rgb(t.selection)),
        ansi: t.ansi.map(|n| Colour(rgb(n))),
    }
}

fn contrast_note(t: &NamedTheme) -> String {
    let surfaces = [t.page, t.title, t.bar, t.pane, t.field, t.popup];
    let worst = |ink: Colour| surfaces.iter().map(|s| contrast(s.0, ink.0)).fold(f32::INFINITY, f32::min);
    let pairs = [
        ("Text", worst(t.text)),
        ("Dim", worst(t.muted)),
        ("Accent", contrast(t.page.0, t.accent.0)),
        ("Selection", contrast(t.selection.0, t.text.0)),
        ("Terminal", contrast(t.terminal_background.0, t.terminal_foreground.0)),
        ("Terminal selection", contrast(t.terminal_selection.0, t.terminal_foreground.0)),
    ];
    let (label, ratio) = pairs.into_iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    format!("{} · {label} {ratio:.1}:1", if ratio < 4.5 { "Low contrast" } else { "Contrast OK" })
}

pub(super) const EDIT_DARK: u32 = 320;
pub(super) const EDIT_LIGHT: u32 = 321;
pub(super) const IMPORT: u32 = 322;
const SAVE: u32 = 323;
const COPY: u32 = 324;
const EXPORT: u32 = 325;
const BACK: u32 = 326;
const NAME: u32 = 327;
const GENERATE: u32 = 328;
const FIRST: u32 = 340;

const LABELS: [&str; 29] = [
    "Window background",
    "Title bar",
    "Tab bar",
    "Pane",
    "Input fields",
    "Popups",
    "Text",
    "Dim text",
    "Accent",
    "Selection",
    "Terminal background",
    "Terminal foreground",
    "Terminal selection",
    "Black",
    "Red",
    "Green",
    "Yellow",
    "Blue",
    "Magenta",
    "Cyan",
    "White",
    "Bright black",
    "Bright red",
    "Bright green",
    "Bright yellow",
    "Bright blue",
    "Bright magenta",
    "Bright cyan",
    "Bright white",
];
fn colour(t: &mut NamedTheme, i: usize) -> &mut Colour {
    match i {
        0 => &mut t.page,
        1 => &mut t.title,
        2 => &mut t.bar,
        3 => &mut t.pane,
        4 => &mut t.field,
        5 => &mut t.popup,
        6 => &mut t.text,
        7 => &mut t.muted,
        8 => &mut t.accent,
        9 => &mut t.selection,
        10 => &mut t.terminal_background,
        11 => &mut t.terminal_foreground,
        12 => &mut t.terminal_selection,
        n => &mut t.ansi[n - 13],
    }
}

/// Work on the unsaved draft. Keep the base's ANSI hues, shifting them slightly
/// towards the accent, so programs still have recognisable reds, blues and greens.
fn generate(t: &mut NamedTheme) {
    use mdshared_ui::{gfx::mix, tokens::legible};
    let bg = t.page.0;
    let fg = if contrast(bg, [0; 3]) > contrast(bg, [255; 3]) { [0; 3] } else { [255; 3] };
    let accent = t.accent.0;
    let surface = |amount| Colour(mix(bg, accent, amount));
    t.title = surface(0.10);
    t.bar = surface(0.06);
    t.pane = surface(0.04);
    t.field = surface(0.02);
    t.popup = surface(0.08);
    t.text = Colour(fg);
    t.muted = Colour(legible(mix(fg, bg, 0.28), bg, 4.5));
    t.selection = Colour(legible(mix(bg, accent, 0.45), fg, 4.5));
    t.terminal_background = t.page;
    t.terminal_foreground = t.text;
    t.terminal_selection = t.selection;
    t.ansi = palettes::get(t.dark(), &t.base).terminal.ansi.map(|n| Colour(mix(rgb(n), accent, 0.08)));
}

pub(super) struct Editor {
    original: Option<String>,
    draft: NamedTheme,
}

impl Editor {
    pub(super) fn note(&self, form: &Form) -> String {
        match self.read(form) {
            Ok(theme) => {
                if let Some(id) = form.colour_open().or_else(|| form.focus_id())
                    && let Some(n) = id.checked_sub(FIRST + 13).filter(|n| *n < 16)
                {
                    let ratio = contrast(theme.ansi[n as usize].0, theme.terminal_background.0);
                    return format!(
                        "{} · {} {ratio:.1}:1",
                        if ratio < 4.5 { "Low ANSI contrast" } else { "ANSI contrast OK" },
                        LABELS[13 + n as usize]
                    );
                }
                contrast_note(&theme)
            }
            Err(_) => "Check name or hex colour".into(),
        }
    }
    fn read(&self, form: &Form) -> Result<NamedTheme, String> {
        let mut t = self.draft.clone();
        if let Some(Value::Text(s)) = form.value(NAME) {
            t.name = s.trim().into();
        }
        for (i, label) in LABELS.iter().enumerate() {
            *colour(&mut t, i) = Colour(form.colour(FIRST + i as u32).ok_or_else(|| format!("{label}: use #RRGGBB."))?);
        }
        t.validate().map_err(|e| e.to_string())?;
        Ok(t)
    }

    fn form(&self) -> Form {
        let mut t = self.draft.clone();
        let mut rows =
            vec![Row::text(NAME, "Theme name", &t.name), Row::action(SAVE, "Keep this palette", "Save and use")];
        if self.original.is_some() {
            rows.push(Row::action(COPY, "Keep original too", "Save as new"))
        }
        rows.extend([
            Row::action(GENERATE, "Rebuild from background and accent", "Generate shades"),
            Row::action(EXPORT, "Share this draft", "Export file…"),
            Row::action(BACK, "Discard unsaved edits", "Back"),
            Row::header("Window colours"),
        ]);
        for (i, label) in LABELS.iter().enumerate() {
            if i == 10 {
                rows.push(Row::header("Terminal · Automatic"))
            } else if i == 13 {
                rows.push(Row::header("ANSI colours"))
            }
            rows.push(Row::text(FIRST + i as u32, *label, String::from(*colour(&mut t, i))));
        }
        let mut form = Form::new("Edit theme", rows).single_page();
        for i in 0..LABELS.len() {
            form.show_as_colour(FIRST + i as u32);
        }
        form
    }
}

fn show(st: &mut AppState, editor: Editor) {
    if let Some(Dialog::Settings(s)) = &mut st.dialog
        && s.category() == "Appearance"
    {
        s.form = editor.form();
        s.editor = Some(editor);
        s.form.focus_row(NAME);
    }
}

fn back(st: &mut AppState) {
    let f = super::page_form(st, 0);
    if let Some(Dialog::Settings(s)) = &mut st.dialog {
        s.editor = None;
        s.form = f;
    }
}

pub(super) fn action(st: &mut AppState, id: u32) -> bool {
    if !(EDIT_DARK..=BACK).contains(&id) && id != GENERATE {
        return false;
    }
    if st.config_damaged {
        return true;
    }
    match id {
        EDIT_DARK | EDIT_LIGHT => {
            let dark = id == EDIT_DARK;
            let a = &st.config.appearance;
            let key = if dark { &a.dark_theme } else { &a.light_theme };
            let editor = if let Some(t) = named::selected(a, dark) {
                Editor { original: Some(key.clone()), draft: t.clone() }
            } else {
                Editor { original: None, draft: from_base(palettes::get(dark, key)) }
            };
            show(st, editor);
        }
        IMPORT => {
            if let Some(proxy) = st.proxy.clone() {
                mdshared_ui::platform::pick_files_async(None, "Claude Switch theme", &["json"], move |files| {
                    if let [path] = files.as_slice() {
                        let result = theme::import(path).map(|t| Some(Box::new(t))).map_err(|e| e.to_string());
                        if let Ok(p) = proxy.lock() {
                            p.send(Msg::ThemeFile(result));
                        }
                    } else if files.len() > 1
                        && let Ok(p) = proxy.lock()
                    {
                        p.send(Msg::ThemeFile(Err("Choose one theme file.".into())));
                    }
                });
            }
        }
        BACK => back(st),
        GENERATE => {
            if let Some(Dialog::Settings(s)) = &mut st.dialog
                && let Some(editor) = &mut s.editor
            {
                match editor.read(&s.form) {
                    Ok(mut t) => {
                        generate(&mut t);
                        for i in 0..LABELS.len() {
                            s.form.set_value(FIRST + i as u32, Value::Text(String::from(*colour(&mut t, i))));
                        }
                    }
                    Err(e) => st.toast(e, std::time::Instant::now()),
                }
            }
        }
        SAVE | COPY | EXPORT => {
            let Some(Dialog::Settings(s)) = &st.dialog else { return true };
            let Some(editor) = &s.editor else { return true };
            let theme = match editor.read(&s.form) {
                Ok(t) => t,
                Err(e) => {
                    st.toast(e, std::time::Instant::now());
                    return true;
                }
            };
            if id == EXPORT {
                if let Some(proxy) = st.proxy.clone() {
                    mdshared_ui::platform::pick_save_async(std::path::Path::new("palette.theme.json"), move |file| {
                        if let Some(mut path) = file {
                            if path.extension().is_none() {
                                path.set_extension("json");
                            }
                            let result = theme::export(&path, &theme).map(|()| None).map_err(|e| e.to_string());
                            if let Ok(p) = proxy.lock() {
                                p.send(Msg::ThemeFile(result));
                            }
                        }
                    });
                }
            } else {
                let key = if id == SAVE { editor.original.clone() } else { None };
                let a = &mut st.config.appearance;
                if a.themes.iter().any(|(id, t)| Some(id) != key.as_ref() && t.name.eq_ignore_ascii_case(&theme.name)) {
                    st.toast("Give the new theme a different name.".into(), std::time::Instant::now());
                    return true;
                }
                let key = match key {
                    Some(id) => id,
                    None => {
                        let mut bytes = [0u8; 8];
                        if getrandom::fill(&mut bytes).is_err() {
                            st.toast("Could not allocate a theme id. Try again.".into(), std::time::Instant::now());
                            return true;
                        }
                        format!("user-{:016x}", u64::from_le_bytes(bytes))
                    }
                };
                *if theme.dark() { &mut a.dark_theme } else { &mut a.light_theme } = key.clone();
                a.themes.insert(key.clone(), theme.clone());
                st.save_config();
                st.retheme();
                if st.config.appearance.themes.get(&key) == Some(&theme) {
                    back(st)
                } else {
                    st.toast("Theme was not saved. The draft is still open.".into(), std::time::Instant::now());
                }
            }
        }
        _ => unreachable!(),
    }
    true
}

impl AppState {
    pub(crate) fn theme_file(&mut self, result: Result<Option<Box<NamedTheme>>, String>) {
        match result {
            Ok(Some(t)) => show(self, Editor { original: None, draft: *t }),
            Ok(None) => self.toast("Theme exported.".into(), std::time::Instant::now()),
            Err(e) => self.toast(e, std::time::Instant::now()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use mdshared_text::Bitmap;
    use mdshared_ui::{
        anim::Animator,
        gfx::{Gfx, Painter, Rect},
    };

    #[test]
    fn editing_a_copy_checks_contrast_without_changing_the_original_and_renders_a_scrollable_page() {
        let mut st = AppState::new(Source::Demo, Some(true), Default::default());
        st.dialog = Some(super::super::settings(&mut st, Some("Appearance")));
        action(&mut st, EDIT_DARK);
        let Some(Dialog::Settings(mut page)) = st.dialog.take() else { panic!() };
        page.form.set_value(NAME, Value::Text("After hours".into()));
        page.form.set_value(FIRST + 6, Value::Text("#212121".into()));
        page.form.set_value(FIRST + 28, Value::Text("#45ab87".into()));
        assert!(page.editor.as_ref().unwrap().note(&page.form).contains("Low contrast"));
        let mut bitmap = Bitmap::new(760, 480);
        let mut gfx = Gfx::new();
        let mut p = Painter::new(&mut bitmap, &mut gfx, 1.0);
        let mut anim = Animator::new();
        anim.begin_frame(std::time::Instant::now());
        page.draw(&mut p, &st, Rect::new(0.0, 0.0, 760.0, 480.0), 1.0, &mut anim);
        if let Some(dir) = std::env::var_os("CS_THEME_SHOTS") {
            p.bmp.write_png(&std::path::Path::new(&dir).join("theme-editor.png")).unwrap();
        }
        assert!(page.form.focus_row(FIRST + 28));
        assert!(page.form.scroll() > 0.0);
        st.dialog = Some(Dialog::Settings(page));
        assert!(st.config.appearance.themes.is_empty());
        action(&mut st, SAVE);
        assert_eq!(st.pal().text, [0x21; 3]);
        assert_eq!(st.term_colours.0.ansi[15], [0x45, 0xab, 0x87]);
        assert_eq!(st.config.appearance.themes.len(), 1);
        action(&mut st, EDIT_DARK);
        let Some(Dialog::Settings(page)) = &mut st.dialog else { panic!() };
        page.form.set_value(NAME, Value::Text("Second draft".into()));
        action(&mut st, COPY);
        assert_eq!(st.config.appearance.themes.len(), 2);
        assert!(st.config.appearance.themes.values().any(|t| t.name == "After hours"));
    }
}
