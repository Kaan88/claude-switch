//! Branch: a new session that continues from a copy of a conversation cut at one message.
//!
//! Claude Code 2.1.332 has no interactive way to resume a session up to a message (its hidden
//! `--resume-session-at` works in print mode only), so the copy is written here: the transcript's
//! lines up to the cut, under a new id in the same folder, each record's own `sessionId` set to
//! that id. `claude --resume <new id>` then continues it as any session, and the original is not
//! touched. Checked against Claude Code by `recorder compat` (script `branch`): a copy cut after a
//! turn resumes, in print mode and in the terminal, with the turns before the cut only.
//!
//! Claude Code's file backups (`file-history/<id>`) are not copied: a branch cannot rewind code
//! changes made before its cut.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Where the copy ends, as byte offsets of transcript lines (`transcript::ItemId::offset`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// Up to the line that starts here, without it. The offset must be a line's start (or the end
    /// of the last whole line).
    Before(u64),
    /// Up to and with the line that starts here, which must be there.
    After(u64),
}

fn invalid(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_owned())
}

/// How old a temporary copy (`<id>.jsonl.part`) must be before a later branch takes it for one
/// left by a branch that failed and removes it.
const STALE_PART: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Writes the copy of `source` (the transcript of session `old_id`) as `<new_id>.jsonl` beside it
/// and returns its path. It is written under a temporary name (`<new_id>.jsonl.part`) and then
/// linked to its own name, or on a volume without hard links created new there and filled; either
/// way no file is ever replaced, and on any failure nothing of this branch is left (a temporary
/// name someone else holds is not touched). A partial last line (one Claude Code is still writing)
/// is never copied. Temporary copies of earlier branches that failed are swept first.
pub fn write(source: &Path, old_id: &str, new_id: &str, cut: Cut) -> io::Result<PathBuf> {
    for id in [old_id, new_id] {
        crate::launch::validate_session_id(id)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    }
    if old_id.eq_ignore_ascii_case(new_id) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "A branch needs a new id."));
    }
    let target = source.with_file_name(format!("{new_id}.jsonl"));
    if target.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "A transcript with the new id exists."));
    }
    if let Some(dir) = source.parent() {
        sweep(dir, STALE_PART);
    }
    let temporary = source.with_file_name(format!("{new_id}.jsonl.part"));
    let mut reader = BufReader::new(File::open(source)?);
    // Not ours until created here: a name someone else holds is left alone.
    let file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let written = (|| {
        let mut out = BufWriter::new(file);
        copy(&mut reader, &mut out, old_id, new_id, cut)?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        place(&temporary, &target, |from, to| std::fs::hard_link(from, to))
    })();
    let _ = std::fs::remove_file(&temporary);
    written.map(|()| target)
}

/// Gives the written temporary copy its own name without replacing anything: a hard link (which
/// fails when the name exists), or where the volume has none, a file created new and filled.
fn place(temporary: &Path, target: &Path, link: impl Fn(&Path, &Path) -> io::Result<()>) -> io::Result<()> {
    match link(temporary, target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(_) => {
            let mut out = OpenOptions::new().write(true).create_new(true).open(target)?;
            let filled = (|| {
                io::copy(&mut File::open(temporary)?, &mut out)?;
                out.sync_all()
            })();
            if filled.is_err() {
                // Created here a moment ago: ours to remove.
                drop(out);
                let _ = std::fs::remove_file(target);
            }
            filled
        }
    }
}

/// Removes temporary copies (`<uuid>.jsonl.part`, the names only a branch writes) older than `age`
/// in `dir`: left by a branch that failed half way (the app ended, the disk filled).
fn sweep(dir: &Path, age: std::time::Duration) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ours = name.strip_suffix(".jsonl.part").is_some_and(|id| crate::launch::validate_session_id(id).is_ok());
        let old =
            entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|e| e >= age);
        if ours && old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn copy(reader: &mut impl BufRead, out: &mut impl Write, old_id: &str, new_id: &str, cut: Cut) -> io::Result<()> {
    let needle = old_id.as_bytes();
    let own = format!("\"sessionId\":\"{old_id}\"").into_bytes();
    let new_own = format!("\"sessionId\":\"{new_id}\"").into_bytes();
    let mut offset = 0u64;
    let mut line = Vec::new();
    let mut lines = 0usize;
    let mut reached = false;
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 || line.last() != Some(&b'\n') {
            break;
        }
        let start = offset;
        offset += n as u64;
        if let Cut::Before(at) = cut {
            if start == at {
                reached = true;
                break;
            }
            if start > at {
                return Err(invalid("The cut is not at the start of a transcript line."));
            }
        }
        if count(&line, needle) == 0 {
            out.write_all(&line)?;
        } else if let Some(at) = find(&line, &own).filter(|_| count(&line, needle) == 1) {
            // The record's own id in its usual form, and nowhere else: the bytes are replaced in
            // place, the rest of the line is kept as it is.
            out.write_all(&line[..at])?;
            out.write_all(&new_own)?;
            out.write_all(&line[at + own.len()..])?;
        } else {
            out.write_all(rename(&line, old_id, new_id).as_deref().unwrap_or(&line))?;
        }
        lines += 1;
        if cut == Cut::After(start) {
            reached = true;
            break;
        }
    }
    match cut {
        Cut::Before(at) if !reached && at != offset => {
            return Err(invalid("The cut is not at the start of a transcript line."));
        }
        Cut::After(_) if !reached => return Err(invalid("The cut's line is not in the transcript.")),
        _ => {}
    }
    if lines == 0 {
        return Err(invalid("Nothing to copy before the cut."));
    }
    out.flush()
}

/// Where `needle` first occurs in `haystack` (ids in any case, as `count` counts them).
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle))
}

/// How often `needle` occurs in `haystack` (ids in any case).
fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack.windows(needle.len()).filter(|w| w.eq_ignore_ascii_case(needle)).count()
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
            json!({"type":"derived-from","sessionId":OLD,"sessions":[{"sessionId":"parent-session"},{"sessionId":OLD}]}),
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

    fn cut_raw(bytes: &[u8], cut: Cut) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        copy(&mut &bytes[..], &mut out, OLD, NEW, cut)?;
        Ok(out)
    }

    fn cut(bytes: &[u8], cut: Cut) -> io::Result<Vec<Value>> {
        let out = cut_raw(bytes, cut)?;
        Ok(out.split(|b| *b == b'\n').filter(|l| !l.is_empty()).map(|l| serde_json::from_slice(l).unwrap()).collect())
    }

    #[test]
    fn a_branch_keeps_the_lines_up_to_its_cut_under_the_new_id() {
        let (bytes, at) = transcript();
        let kept = cut(&bytes, Cut::Before(at[3])).unwrap();
        assert_eq!(kept.len(), 3);
        assert!(kept.iter().all(|v| v["sessionId"] == NEW));
        assert_eq!(kept[2]["sessions"][0]["sessionId"], "parent-session", "only a record's own id changes");
        assert_eq!(kept[2]["sessions"][1]["sessionId"], OLD, "a mention of the original stays");
        let kept = cut(&bytes, Cut::After(at[1])).unwrap();
        assert_eq!(kept.iter().map(|v| v["uuid"].as_str().unwrap()).collect::<Vec<_>>(), ["u1", "a1"]);
        assert_eq!(cut(&bytes, Cut::Before(bytes.len() as u64)).unwrap().len(), 5, "the end of the last line");
        assert!(cut(&bytes, Cut::Before(0)).is_err(), "nothing before the first line");
        // A line still being written is not copied.
        let mut partial = bytes.clone();
        partial.extend(br#"{"type":"user","sessionId":"#);
        assert_eq!(cut(&partial, Cut::Before(bytes.len() as u64)).unwrap().len(), 5);
    }

    #[test]
    fn a_cut_that_is_not_at_a_line_is_refused_rather_than_copying_too_much() {
        let (bytes, at) = transcript();
        assert!(cut(&bytes, Cut::After(at[1] + 3)).is_err(), "After a byte that starts no line");
        assert!(cut(&bytes, Cut::After(bytes.len() as u64 + 10)).is_err(), "After past the end");
        assert!(cut(&bytes, Cut::Before(at[3] - 1)).is_err(), "Before the middle of a line");
        assert!(cut(&bytes, Cut::Before(bytes.len() as u64 + 10)).is_err(), "Before past the end");
    }

    #[test]
    fn the_usual_id_is_replaced_in_place_and_the_rest_of_the_line_kept_byte_for_byte() {
        // Spacing and key order serde would change: the in-place replacement keeps them.
        let line = format!("{{\"z\":1,  \"sessionId\":\"{OLD}\",\"a\":\"x\"}}\n");
        let out = cut_raw(line.as_bytes(), Cut::Before(line.len() as u64)).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), line.replace(OLD, NEW));
        // Not the usual form: parsed, and only the record's own id changes.
        let line = format!("{{\"sessionId\" : \"{OLD}\",\"note\":\"{OLD}\"}}\n");
        let out = cut(line.as_bytes(), Cut::Before(line.len() as u64)).unwrap();
        assert_eq!((out[0]["sessionId"].as_str(), out[0]["note"].as_str()), (Some(NEW), Some(OLD)));
    }

    #[test]
    fn an_id_in_another_case_is_replaced_without_a_panic() {
        let line = format!("{{\"sessionId\":\"{}\",\"a\":1}}\n", OLD.to_uppercase());
        let out = cut(line.as_bytes(), Cut::Before(line.len() as u64)).unwrap();
        assert_eq!(out[0]["sessionId"], NEW);
    }

    #[test]
    fn without_hard_links_the_copy_is_created_new_and_still_never_replaces_a_file() {
        let dir = std::env::temp_dir().join(format!("cs-branch-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (temporary, target) = (dir.join("t.part"), dir.join("t.jsonl"));
        std::fs::write(&temporary, b"copied\n").unwrap();
        let unsupported = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::Unsupported));
        place(&temporary, &target, unsupported).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"copied\n");
        assert!(place(&temporary, &target, unsupported).is_err(), "never over a file");
        assert_eq!(std::fs::read(&target).unwrap(), b"copied\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stale_temporary_copies_of_branches_are_swept_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("cs-branch-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ours = dir.join(format!("{NEW}.jsonl.part"));
        let other = dir.join("notes.jsonl.part");
        let transcript = dir.join(format!("{OLD}.jsonl"));
        for path in [&ours, &other, &transcript] {
            std::fs::write(path, b"x").unwrap();
        }
        sweep(&dir, std::time::Duration::from_secs(3600));
        assert!(ours.exists(), "a fresh one may be another branch's, under way");
        sweep(&dir, std::time::Duration::ZERO);
        assert!(!ours.exists() && other.exists() && transcript.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_temporary_name_someone_else_holds_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("cs-branch-held-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join(format!("{OLD}.jsonl"));
        let (bytes, at) = transcript();
        std::fs::write(&source, &bytes).unwrap();
        let held = dir.join(format!("{NEW}.jsonl.part"));
        std::fs::write(&held, b"another writer's").unwrap();
        assert!(write(&source, OLD, NEW, Cut::Before(at[3])).is_err());
        assert_eq!(std::fs::read(&held).unwrap(), b"another writer's");
        assert!(!dir.join(format!("{NEW}.jsonl")).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_copy_is_written_beside_the_original_and_never_over_a_file() {
        let dir = std::env::temp_dir().join(format!("cs-branch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join(format!("{OLD}.jsonl"));
        let (bytes, at) = transcript();
        std::fs::write(&source, &bytes).unwrap();
        let path = write(&source, OLD, NEW, Cut::Before(at[3])).unwrap();
        assert_eq!(path, dir.join(format!("{NEW}.jsonl")));
        assert_eq!(std::fs::read(&source).unwrap(), bytes, "the original is not touched");
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 3);
        let kept = std::fs::read(&path).unwrap();
        assert!(write(&source, OLD, NEW, Cut::Before(at[3])).is_err(), "never over a file");
        assert_eq!(std::fs::read(&path).unwrap(), kept);
        assert!(write(&source, OLD, "not-an-id", Cut::After(at[0])).is_err());
        assert!(write(&source, OLD, OLD, Cut::After(at[0])).is_err());
        // A failed copy leaves nothing, not even its temporary file.
        let other = "11111111-2222-4333-8444-555566667777";
        assert!(write(&source, OLD, other, Cut::After(at[0] + 1)).is_err());
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
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
