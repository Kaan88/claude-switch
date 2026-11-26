//! The window's size and the splitter's place, kept between runs in `window.ini` through the
//! toolkit's settings layer. A missing or damaged file gives the defaults; nothing is written in
//! the demo (it has no file).

use std::path::Path;

use mdshared_app::settings::{self, Writer};

/// What is remembered.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowFile {
    pub w: f32,
    pub h: f32,
    /// [`mdshared_ui::splitter::Splitter::to_setting`].
    pub split: String,
}

impl Default for WindowFile {
    fn default() -> WindowFile {
        WindowFile { w: 1100.0, h: 720.0, split: String::new() }
    }
}

fn size(v: &str, default: f32) -> f32 {
    match v.trim().parse::<f32>() {
        Ok(n) if n.is_finite() && (200.0..=20_000.0).contains(&n) => n,
        _ => default,
    }
}

/// Reads the text of the file.
pub fn parse(text: &str) -> WindowFile {
    let mut out = WindowFile::default();
    for (k, v) in settings::pairs(text) {
        match k {
            "width" => out.w = size(v, out.w),
            "height" => out.h = size(v, out.h),
            "split" => out.split = settings::unescape(v),
            _ => {}
        }
    }
    out
}

pub fn to_text(f: &WindowFile) -> String {
    let mut w = Writer::new("window");
    w.line("width", &format!("{:.0}", f.w));
    w.line("height", &format!("{:.0}", f.h));
    w.line("split", &settings::escape(&f.split));
    w.finish()
}

pub fn load(path: Option<&Path>) -> WindowFile {
    path.and_then(|p| settings::read(p).ok().flatten()).map_or_else(WindowFile::default, |t| parse(&t))
}

pub fn save(path: Option<&Path>, f: &WindowFile) {
    if let Some(p) = path {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = settings::write_atomic(p, &to_text(f));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_reads_back_as_what_was_written() {
        let f = WindowFile { w: 1280.0, h: 800.0, split: "a:420".into() };
        assert_eq!(parse(&to_text(&f)), f);
    }

    #[test]
    fn nonsense_gives_the_defaults() {
        assert_eq!(parse("width=abc\nheight=-5\nnothing"), WindowFile::default());
        assert_eq!(load(None), WindowFile::default());
    }
}
