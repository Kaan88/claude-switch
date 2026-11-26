use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;

use claude_switch_core::grouping::open_local_file;
use mdshared_text::image::Image;

use super::links::{MAX_MARKDOWN, NamedFile};

pub(super) enum Document {
    Markdown(String),
    Text(String),
    Picture(Arc<Image>),
}

pub(super) fn read(file: &NamedFile) -> io::Result<Document> {
    if file.markdown() {
        return file.read_markdown().map(Document::Markdown);
    }
    let mut handle =
        open_local_file(file.path(), true).ok_or_else(|| io::Error::other("The named file is unavailable."))?;
    let mut bytes = Vec::new();
    if file.picture() {
        const MAX_IMAGE: u64 = 16 * 1024 * 1024;
        handle.take(MAX_IMAGE + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_IMAGE {
            return Err(io::Error::other("The named picture exceeds the 16 MiB preview limit."));
        }
        return super::pictures::decode(&bytes, 2048).map(Arc::new).map(Document::Picture);
    }
    // A full-output view starts from the beginning, without following future writes.
    (&mut handle).take(MAX_MARKDOWN + 1).read_to_end(&mut bytes)?;
    let clipped = bytes.len() as u64 > MAX_MARKDOWN;
    bytes.truncate(MAX_MARKDOWN as usize);
    let raw = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
    let raw = match std::str::from_utf8(raw) {
        Ok(raw) => raw,
        Err(e) if clipped && e.error_len().is_none() => {
            std::str::from_utf8(&raw[..e.valid_up_to()]).unwrap_or_default()
        }
        _ => return Err(io::Error::other("This file is not UTF-8 text.")),
    };
    let mut text = clean(raw);
    if clipped {
        text.push_str("\n… preview capped at 1 MiB …");
    }
    Ok(Document::Text(text))
}

pub(super) fn tail(file: &NamedFile) -> io::Result<Vec<String>> {
    const CAP: u64 = 16 * 1024;
    let mut f = open_local_file(file.path(), true).ok_or_else(|| io::Error::other("Output file unavailable"))?;
    let end = f.metadata()?.len();
    let start = end.saturating_sub(CAP);
    f.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    f.take(end - start).read_to_end(&mut bytes)?;
    let raw = String::from_utf8_lossy(&bytes);
    let mut lines = raw
        .lines()
        .rev()
        .take(8)
        .map(|line| {
            let line = &line[..line.floor_char_boundary(line.len().min(720))];
            clean(line)
        })
        .collect::<Vec<_>>();
    lines.reverse();
    if start > 0 && raw.lines().rev().take(9).count() <= 8 && !lines.is_empty() {
        lines[0].insert(0, '…');
    }
    Ok(lines)
}

fn clean(text: &str) -> String {
    // Do not interpret ANSI sequences or allow cursor controls to become UI instructions.
    text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_snapshot_is_literal_and_tail_reads_only_the_end_of_the_named_file() {
        let demo = crate::demo::Transcript::new().unwrap();
        let path = demo.cwd().join("shell.output");
        let mut text = format!("{}\n", "old opening".repeat(2_000));
        for i in 0..12 {
            text.push_str(&format!("line {i} <b>literal</b>\n"));
        }
        std::fs::write(&path, &text).unwrap();
        let file = NamedFile::shell_output(path.to_str().unwrap(), demo.cwd()).unwrap();
        let lines = tail(&file).unwrap();
        assert_eq!(lines.len(), 8);
        assert_eq!(lines[0], "line 4 <b>literal</b>");
        assert_eq!(lines[7], "line 11 <b>literal</b>");
        assert!(matches!(read(&file).unwrap(), Document::Text(s) if s == text));
        assert!(NamedFile::shell_output("../unrelated.output", demo.cwd()).is_none());
    }
}
