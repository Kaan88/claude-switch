//! Read-only clipboard previews; Claude Code, not Switch, pastes the original image.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Picture {
    pub seq: u32,
    pub thumbnail: Option<Thumbnail>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Thumbnail {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

const MAX_DIM: u32 = 8192;
const MAX_BYTES: usize = 48 * 1024 * 1024;

fn thumbnail(w: u32, h: u32, pixel: impl Fn(usize, usize) -> [u8; 4]) -> Option<Thumbnail> {
    if w == 0 || h == 0 || w.max(h) > MAX_DIM {
        return None;
    }
    let longest = w.max(h);
    let scale = 96.min(longest);
    let (tw, th) = ((w * scale / longest).max(1), (h * scale / longest).max(1));
    let mut rgba = Vec::with_capacity((tw * th * 4) as usize);
    for y in 0..th {
        for x in 0..tw {
            rgba.extend(pixel((x * w / tw) as usize, (y * h / th) as usize));
        }
    }
    Some(Thumbnail { rgba, w: tw, h: th })
}

fn dib(data: &[u8]) -> Option<Thumbnail> {
    let read = |at| Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?));
    let header = read(0)? as usize;
    let width = read(4)?;
    let height = read(8)? as i32;
    let bits = read(12)?;
    let layout = read(16)?;
    let (w, h) = (width, height.unsigned_abs());
    if !matches!(header, 40 | 108 | 124)
        || w == 0
        || h == 0
        || w.max(h) > MAX_DIM
        || !matches!(bits, 0x0018_0001 | 0x0020_0001)
        || read(32)? != 0
        || (header == 124 && read(116)? != 0)
    // Embedded colour profile; prefer the PNG fallback.
    {
        return None;
    }
    let bpp = (bits >> 16) as usize / 8;
    let (offset, alpha) = match layout {
        0 => (header, false),
        3 if bpp == 4 && [read(40)?, read(44)?, read(48)?] == [0xff_0000, 0xff00, 0xff] => {
            (header.max(52), header > 40 && read(52)? == 0xff00_0000)
        }
        _ => return None,
    };
    let stride = (w as usize * bpp).next_multiple_of(4);
    let pixels = data.get(offset..offset + stride * h as usize)?;
    thumbnail(w, h, |x, y| {
        let row = if height < 0 { y } else { h as usize - 1 - y };
        let at = row * stride + x * bpp;
        [pixels[at + 2], pixels[at + 1], pixels[at], if alpha { pixels[at + 3] } else { 255 }]
    })
}

fn png(data: &[u8]) -> Option<Thumbnail> {
    let (w, h) = (
        u32::from_be_bytes(data.get(16..20)?.try_into().ok()?),
        u32::from_be_bytes(data.get(20..24)?.try_into().ok()?),
    );
    if w.max(h) > MAX_DIM || u64::from(w) * u64::from(h) * 4 > MAX_BYTES as u64 {
        return None;
    }
    let (pixels, w, h) = mdshared_text::decode_png_rgba(data)?;
    thumbnail(w, h, |x, y| {
        let at = (y * w as usize + x) * 4;
        pixels[at..at + 4].try_into().unwrap()
    })
}

#[cfg(windows)]
pub(super) fn sequence() -> Option<u32> {
    // SAFETY: No pointers; zero is the documented unavailable result.
    let seq = unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() };
    (seq != 0).then_some(seq)
}

#[cfg(windows)]
pub(super) fn picture() -> Option<Picture> {
    use windows_sys::Win32::System::DataExchange::*;
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    struct Open;
    impl Drop for Open {
        fn drop(&mut self) {
            // SAFETY: This guard is only made after a successful OpenClipboard on this thread.
            unsafe { CloseClipboard() };
        }
    }

    // SAFETY: Null owner is permitted for read-only use; format name is static and terminated.
    let png_format = unsafe { RegisterClipboardFormatW(windows_sys::w!("PNG")) };
    let formats = [png_format, 17, 8, 2]; // Registered PNG, CF_DIBV5, CF_DIB, CF_BITMAP.
    if !formats.into_iter().any(|f| f != 0 && unsafe { IsClipboardFormatAvailable(f) } != 0) {
        return None;
    }
    // SAFETY: No window ownership is needed to read the clipboard.
    if unsafe { OpenClipboard(std::ptr::null_mut()) } == 0 {
        return None;
    }
    let open = Open;
    // Keep the clipboard locked only while copying PNG bytes or sampling a DIB. CF_BITMAP is not
    // an HGLOBAL; Windows normally synthesizes CF_DIB for it, otherwise show a fallback label.
    let mut encoded = None;
    let mut preview = None;
    for format in [17, 8, png_format].into_iter().filter(|f| *f != 0) {
        // SAFETY: Clipboard is open, and these three clipboard formats contain movable memory.
        unsafe {
            let handle = GetClipboardData(format);
            if handle.is_null() {
                continue;
            }
            let size = GlobalSize(handle);
            if size == 0 || size > MAX_BYTES {
                continue;
            }
            let ptr = GlobalLock(handle);
            if ptr.is_null() {
                continue;
            }
            let data = std::slice::from_raw_parts(ptr.cast::<u8>(), size);
            if format == png_format {
                encoded = Some(data.to_vec());
            } else {
                preview = dib(data);
            }
            GlobalUnlock(handle);
        }
        if preview.is_some() {
            break;
        }
    }
    // Delayed rendering can advance the sequence when GetClipboardData first materializes it.
    let seq = sequence()?;
    drop(open);
    (sequence() == Some(seq)).then(|| Picture { seq, thumbnail: preview.or_else(|| png(encoded.as_deref()?)) })
}

#[cfg(not(windows))]
pub(super) fn sequence() -> Option<u32> {
    None
}

#[cfg(not(windows))]
pub(super) fn picture() -> Option<Picture> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_thumbnail_uses_bottom_up_rows_and_checks_truncation() {
        let mut bytes = vec![0; 40];
        for (i, n) in [(0, 40_u32), (4, 1), (8, 2), (12, 0x0018_0001)] {
            bytes[i..i + 4].copy_from_slice(&n.to_le_bytes());
        }
        bytes.extend([255, 0, 0, 0, 0, 0, 255, 0]);
        let t = dib(&bytes).unwrap();
        assert_eq!((t.w, t.h), (1, 2));
        assert_eq!(t.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
        assert!(dib(&bytes[..bytes.len() - 1]).is_none());
        assert!(png(&[]).is_none());
    }
}
