//! Branch: a new session that continues from a copy of a conversation cut at one message.
//!
//! Claude Code 2.1.332 has no interactive way to resume a session up to a message (its hidden
//! `--resume-session-at` works in print mode only), so the copy is written here: the transcript's
//! lines up to the cut, under a new id in the same folder, each record's own `sessionId` set to
//! that id. `claude --resume <new id>` then continues it as any session, and the original is not
//! touched. Tried against Claude Code 2.1.332 (print mode): a copy cut after the second of three
//! answers resumes with the first two turns only, under the new id.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Where the copy ends, as byte offsets of transcript lines (`transcript::ItemId::offset`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// Up to the line that starts here, without it.
    Before(u64),
    /// Up to and with the line that starts here.
    After(u64),
    /// The whole transcript as it is now.
    Whole,
}

/// Writes the copy of `source` (the transcript of session `old_id`) as `<new_id>.jsonl` beside it
/// and returns its path. Never replaces a file. A partial last line (one Claude Code is still
/// writing) is left out.
pub fn write(source: &Path, old_id: &str, new_id: &str, cut: Cut) -> io::Result<PathBuf> {
    crate::launch::validate_session_id(old_id)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    crate::launch::validate_session_id(new_id)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    let target = source.with_file_name(format!("{new_id}.jsonl"));
    let mut reader = BufReader::new(File::open(source)?);
    let file = OpenOptions::new().write(true).create_new(true).open(&target)?;
    let written = copy(&mut reader, &mut BufWriter::new(file), old_id, new_id, cut);
    if written.is_err() {
        let _ = std::fs::remove_file(&target);
    }
    written.map(|()| target)
}

fn copy(reader: &mut impl BufRead, out: &mut impl Write, old_id: &str, new_id: &str, cut: Cut) -> io::Result<()> {
    let needle = old_id.as_bytes();
    let mut offset = 0u64;
    let mut line = Vec::new();
    let mut lines = 0usize;
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 || line.last() != Some(&b'\n') {
            break;
        }
        let start = offset;
        offset += n as u64;
        if let Cut::Before(at) = cut
            && start >= at
        {
            break;
        }
        let renamed = if line.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle)) {
            rename(&line, old_id, new_id)
        } else {
            None
        };
        out.write_all(renamed.as_deref().unwrap_or(&line))?;
        lines += 1;
        if cut == Cut::After(start) {
            break;
        }
    }
    if lines == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Nothing to copy before the cut."));
    }
    out.flush()
}

/// The line with its own `sessionId` set to `new_id`, when it is the original's; `None` to keep
/// it as it is (ids named elsewhere in a record, such as a parent session, stay).
fn rename(line: &[u8], old_id: &str, new_id: &str) -> Option<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(line).ok()?;
    let own = value.get("sessionId")?.as_str()?.eq_ignore_ascii_case(old_id);
    if !own {
        return None;
    }
    value["sessionId"] = Value::String(new_id.to_owned());
    let mut out = serde_json::to_vec(&value).ok()?;
    out.push(b'\n');
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OLD: &str = "0a1b2c3d-1111-4222-8333-444455556666";
    const NEW: &str = "7f00e2aa-9999-4222-8333-444455556666";

    fn transcript() -> (Vec<u8>, Vec<u64>) {
        let records = [
            json!({"type":"user","sessionId":OLD,"uuid":"u1","message":{"role":"user","content":"alpha"}}),
            json!({"type":"assistant","sessionId":OLD,"uuid":"a1","parentUuid":"u1","message":{"content":[{"type":"text","text":"alpha"}]}}),
            json!({"type":"derived-from","sessionId":OLD,"sessions":[{"sessionId":"parent-session"}]}),
            json!({"type":"user","sessionId":OLD,"uuid":"u2","parentUuid":"a1","message":{"role":"user","content":"bravo"}}),
            json!({"type":"assistant","sessionId":OLD,"uuid":"a2","parentUuid":"u2","message":{"content":[{"type":"text","text":"bravo"}]}}),
        ];
        let mut bytes = Vec::new();
        let mut offsets = Vec::new();
        for record in records {
            offsets.push(bytes.len() as u64);
            bytes.extend(serde_json::to_vec(&record).unwrap());
            bytes.push(b'\n');
        }
        (bytes, offsets)
    }

    fn cut(bytes: &[u8], cut: Cut) -> io::Result<Vec<Value>> {
        let mut out = Vec::new();
        copy(&mut &bytes[..], &mut out, OLD, NEW, cut)?;
        Ok(out.split(|b| *b == b'\n').filter(|l| !l.is_empty()).map(|l| serde_json::from_slice(l).unwrap()).collect())
    }

    #[test]
    fn a_branch_keeps_the_lines_up_to_its_cut_under_the_new_id() {
        let (bytes, at) = transcript();
        let kept = cut(&bytes, Cut::Before(at[3])).unwrap();
        assert_eq!(kept.len(), 3);
        assert!(kept.iter().all(|v| v["sessionId"] == NEW));
        assert_eq!(kept[2]["sessions"][0]["sessionId"], "parent-session", "only a record's own id changes");
        let kept = cut(&bytes, Cut::After(at[1])).unwrap();
        assert_eq!(kept.iter().map(|v| v["uuid"].as_str().unwrap()).collect::<Vec<_>>(), ["u1", "a1"]);
        assert_eq!(cut(&bytes, Cut::Whole).unwrap().len(), 5);
        assert!(cut(&bytes, Cut::Before(0)).is_err(), "nothing before the first line");
        // A line still being written is not copied.
        let mut partial = bytes.clone();
        partial.extend(br#"{"type":"user","sessionId":"#);
        assert_eq!(cut(&partial, Cut::Whole).unwrap().len(), 5);
    }

    #[test]
    fn the_copy_is_written_beside_the_original_and_never_over_a_file() {
        let dir = std::env::temp_dir().join(format!("cs-branch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join(format!("{OLD}.jsonl"));
        let (bytes, at) = transcript();
        std::fs::write(&source, &bytes).unwrap();
        let path = write(&source, OLD, NEW, Cut::Before(at[3])).unwrap();
        assert_eq!(path, dir.join(format!("{NEW}.jsonl")));
        assert_eq!(std::fs::read(&source).unwrap(), bytes, "the original is not touched");
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 3);
        assert!(write(&source, OLD, NEW, Cut::Whole).is_err(), "never over a file");
        assert!(write(&source, OLD, "not-an-id", Cut::Whole).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Writes a branch of a real transcript for a check by hand against Claude Code:
/// `CS_BRANCH_SOURCE=<file> CS_BRANCH_AFTER=<line's byte offset> CS_BRANCH_ID=<new id>`.
#[cfg(test)]
#[test]
#[ignore = "writes a transcript copy; run by hand in a scratch folder"]
fn branch_by_hand() {
    let source = PathBuf::from(std::env::var("CS_BRANCH_SOURCE").unwrap());
    let after: u64 = std::env::var("CS_BRANCH_AFTER").unwrap().parse().unwrap();
    let new_id = std::env::var("CS_BRANCH_ID").unwrap();
    let old_id = source.file_stem().unwrap().to_str().unwrap().to_owned();
    println!("{}", write(&source, &old_id, &new_id, Cut::After(after)).unwrap().display());
}
