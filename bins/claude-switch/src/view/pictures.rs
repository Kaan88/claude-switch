//! Worker-side previews and explicit export for the system picture viewer.
use std::fs::{self, File};
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use claude_switch_core::transcript::{Picture, Reader};
use mdshared_text::image::{self, Image, Limits};
use mdshared_ui::{platform, thumbs};

pub(super) fn decode(bytes: &[u8], side: u32) -> io::Result<Image> {
    let image = image::decode(bytes, Limits::DEFAULT).map_err(io::Error::other)?;
    if image.width.max(image.height) <= side {
        return Ok(image);
    }
    image.scaled_to_fit(side, side).ok_or_else(|| io::Error::other("Could not scale picture"))
}

/// A plain reason for a picture that cannot be shown.
pub(super) fn reason(error: &io::Error) -> &'static str {
    let text = error.to_string();
    if text.contains("exceeds") || text.contains("too large") || text.contains("limit") {
        "Too large to preview"
    } else if text.contains("stale") || text.contains("no longer matches") || text.contains("missing picture") {
        "No longer in the transcript"
    } else if text.contains("nsupported") || text.contains("format") {
        "Not a picture format Switch shows"
    } else {
        "Could not be decoded"
    }
}

pub fn preview(reader: &mut Reader, picture: &Picture, large: bool) -> io::Result<Arc<Image>> {
    let bytes = reader.load_picture(picture)?;
    decode(&bytes, if large { 2048 } else { thumbs::bucket(240) }).map(Arc::new)
}

pub fn open(reader: &mut Reader, picture: &Picture) -> io::Result<()> {
    let bytes = reader.load_picture(picture)?;
    let ext = match image::sniff(&bytes) {
        image::Format::Png => "png",
        image::Format::Jpeg => "jpg",
        _ => return Err(io::Error::other("Unsupported picture format")),
    };
    let dir = std::env::temp_dir().join("claude-switch").join("pictures");
    fs::create_dir_all(&dir)?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("picture-{}-{time}-{seq}.{ext}", std::process::id()));
    let mut file = File::create_new(&path)?;
    let written = file.write_all(&bytes);
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    if platform::open_file(&path, &[ext]) {
        Ok(())
    } else {
        let _ = fs::remove_file(&path);
        Err(io::Error::other("Could not open the exported picture"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_preserve_small_pngs_and_shrink_large_ones() {
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, 2, 1);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&[255, 0, 0, 0, 255, 0]).unwrap();
        }
        let image = decode(&png, 256).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.pixel(1, 0), Some([0, 255, 0, 255]));
        assert_eq!(decode(&png, 1).unwrap().width, 1);
    }

    #[test]
    fn damaged_and_unsupported_images_fail() {
        for bad in [b"\x89PNG\r\n\x1a\nbad".as_slice(), b"GIF89a", b""] {
            assert!(decode(bad, 256).is_err());
        }
    }

    #[test]
    fn a_failed_picture_says_why_in_plain_words() {
        let reason = |text: &str| reason(&io::Error::other(text.to_owned()));
        assert_eq!(reason("picture exceeds 32 MiB"), "Too large to preview");
        assert_eq!(reason("stale picture reference"), "No longer in the transcript");
        assert_eq!(reason("Unsupported picture format"), "Not a picture format Switch shows");
        assert_eq!(reason("invalid base64"), "Could not be decoded");
    }
}
