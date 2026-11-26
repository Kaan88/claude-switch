use super::MAX_LINE;
use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const SCAN_BYTES: u64 = 64 * 1024;

/// Keep at most `limit` bytes of a record, but consume it all up to newline or EOF.
pub(super) fn bounded_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    oversized: &mut bool,
    limit: usize,
) -> io::Result<(u64, bool)> {
    let mut consumed = 0;
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            return Ok((consumed, false));
        }
        let end = newline(bytes);
        let n = end.map_or(bytes.len(), |i| i + 1);
        if !*oversized {
            *oversized = n > limit.saturating_sub(line.len());
            if *oversized {
                line.clear();
            } else {
                line.extend_from_slice(&bytes[..n]);
            }
        }
        reader.consume(n);
        consumed += n as u64;
        if end.is_some() {
            return Ok((consumed, true));
        }
    }
}

/// Byte cursor shared by session views and the lightweight agent statistics reducer.
pub(super) struct Tail {
    pub(super) path: PathBuf,
    file: File,
    pub(super) chunk: usize,
    cursor: u64,
    pub(super) head: Option<u64>,
    pending_start: u64,
    pending: Vec<u8>,
    clipped: bool,
    oversized: bool,
    checkpoint: Vec<u8>,
    modified: Option<std::time::SystemTime>,
}

impl Tail {
    pub(super) fn open(path: &Path, chunk: usize) -> io::Result<Self> {
        let file = File::open(path)?;
        Ok(Self {
            path: path.to_owned(),
            file,
            chunk,
            cursor: 0,
            head: Some(0),
            pending_start: 0,
            pending: Vec::new(),
            clipped: false,
            oversized: false,
            checkpoint: Vec::new(),
            modified: None,
        })
    }

    pub(super) fn read(&mut self, start: u64, len: u64) -> io::Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        self.file.by_ref().take(len).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub(super) fn initial(&mut self, mut visit: impl FnMut(u64, &[u8])) -> io::Result<()> {
        let meta = self.file.metadata()?;
        let size = meta.len();
        let start = size.saturating_sub(self.chunk.min(MAX_LINE) as u64);
        let bytes = self.read(start, size - start)?;
        self.cursor = start + bytes.len() as u64;
        self.remember(meta.modified().ok())?;
        let skip = if start == 0 {
            0
        } else if let Some(i) = newline(&bytes) {
            i + 1
        } else {
            self.head = None;
            self.clipped = true;
            self.pending_start = start;
            self.pending = bytes;
            return Ok(());
        };
        let mut offset = start + skip as u64;
        self.head = Some(offset);
        for line in bytes[skip..].split_inclusive(|&byte| byte == b'\n') {
            if line.ends_with(b"\n") {
                visit(offset, line);
                offset += line.len() as u64;
            } else {
                self.pending.extend_from_slice(line);
            }
        }
        self.pending_start = offset;
        Ok(())
    }

    fn remember(&mut self, modified: Option<std::time::SystemTime>) -> io::Result<()> {
        let start = self.cursor.saturating_sub(256);
        self.checkpoint = self.read(start, self.cursor - start)?;
        self.modified = modified;
        Ok(())
    }

    pub(super) fn replaced(&mut self) -> io::Result<bool> {
        let held = self.file.metadata()?;
        if held.len() < self.cursor {
            return Ok(true);
        }
        if self.cursor != 0 {
            if held.len() == self.cursor && self.modified.is_some() && held.modified().ok() != self.modified {
                return Ok(true);
            }
            let len = self.checkpoint.len() as u64;
            if len != 0 && self.read(self.cursor - len, len)? != self.checkpoint {
                return Ok(true);
            }
        }
        let path_meta = match fs::metadata(&self.path) {
            Ok(meta) => meta,
            // The writer may be between rename and recreate. Keep the old view until a new path exists.
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err),
        };
        Ok(different_identity(&held, &path_meta)
            // On Windows stable std doesn't expose the file index. Comparing against the open handle
            // also catches replacements that inherit the former path's creation time.
            || ((path_meta.len(), path_meta.modified().ok()) != (held.len(), held.modified().ok())
                && self.file.metadata().is_ok_and(|now| {
                    (now.len(), now.modified().ok()) == (held.len(), held.modified().ok())
                })))
    }

    pub(super) fn append(&mut self, mut visit: impl FnMut(u64, &[u8])) -> io::Result<()> {
        let meta = self.file.metadata()?;
        if meta.len() <= self.cursor {
            return Ok(());
        }
        self.file.seek(SeekFrom::Start(self.cursor))?;
        let mut reader = BufReader::new(self.file.by_ref().take(meta.len() - self.cursor));
        loop {
            let (count, complete) = bounded_line(&mut reader, &mut self.pending, &mut self.oversized, MAX_LINE)?;
            self.cursor += count;
            if !complete {
                break;
            }
            if self.clipped {
                // The line began outside the initial budget. Leave it in history rather than
                // seeking backward in a poll or trying to parse a suffix as a whole line.
                self.clipped = false;
                self.head = Some(self.cursor);
            } else if !self.oversized {
                visit(self.pending_start, &self.pending);
            }
            self.pending.clear();
            self.oversized = false;
            self.pending_start = self.cursor;
        }
        drop(reader);
        self.remember(meta.modified().ok())
    }

    // Find the start of the line containing `at`, or return `at` if it is already a boundary.
    fn line_start(&mut self, mut at: u64) -> io::Result<u64> {
        while at > 0 {
            let start = at.saturating_sub(SCAN_BYTES);
            let bytes = self.read(start, at - start)?;
            if let Some(i) = bytes.iter().rposition(|&byte| byte == b'\n') {
                return Ok(start + i as u64 + 1);
            }
            at = start;
        }
        Ok(0)
    }

    pub(super) fn earlier(&mut self, mut visit: impl FnMut(u64, &[u8])) -> io::Result<()> {
        let end = match self.head {
            Some(head) => head,
            None => self.line_start(self.pending_start)?,
        };
        if self.clipped {
            // Keep the partial line's real start even as historical chunks move farther back.
            if self.cursor - end > MAX_LINE as u64 || self.oversized {
                self.pending.clear();
                self.oversized = true;
            } else {
                let mut prefix = self.read(end, self.pending_start - end)?;
                prefix.append(&mut self.pending);
                self.pending = prefix;
            }
            self.pending_start = end;
            self.clipped = false;
        }
        if end == 0 {
            self.head = Some(0);
            return Ok(());
        }
        let probe = end.saturating_sub((self.chunk.min(MAX_LINE) as u64).max(1));
        let candidate = if probe == 0 {
            0
        } else {
            let bytes = self.read(probe - 1, end - probe)?;
            newline(&bytes).map_or(end, |i| probe + i as u64)
        };
        let start = if candidate < end { candidate } else { self.line_start(end - 1)? };
        self.file.seek(SeekFrom::Start(start))?;
        let mut reader = BufReader::new(self.file.by_ref().take(end - start));
        let mut line = Vec::new();
        let mut offset = start;
        loop {
            let mut oversized = false;
            let (count, complete) = bounded_line(&mut reader, &mut line, &mut oversized, MAX_LINE)?;
            if !complete {
                break;
            }
            if !oversized {
                visit(offset, &line);
            }
            offset += count;
            line.clear();
        }
        self.head = Some(start);
        Ok(())
    }
}

#[cfg(unix)]
fn different_identity(a: &Metadata, b: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    (a.dev(), a.ino()) != (b.dev(), b.ino())
}

#[cfg(not(unix))]
fn different_identity(a: &Metadata, b: &Metadata) -> bool {
    a.created().ok() != b.created().ok()
}

fn newline(bytes: &[u8]) -> Option<usize> {
    bytes.iter().position(|&byte| byte == b'\n')
}
