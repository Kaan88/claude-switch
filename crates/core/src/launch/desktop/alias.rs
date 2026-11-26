//! Activate the known package with its package-family-specific execution alias.
//! Never use the global PATH alias and never spawn an unchecked reparse point.
use std::fs::{File, OpenOptions};
use std::os::windows::{ffi::OsStringExt, fs::OpenOptionsExt, io::AsRawHandle};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ};
use windows_sys::Win32::System::IO::DeviceIoControl;

use super::{FAMILY_NAME, lock_dirs};

const ALIAS: &str = "claude-desktop.exe";
const AUMID: &str = "Claude_pzs8sxrjxfjjc!Claude";
const FSCTL_GET_REPARSE_POINT: u32 = 0x0009_00a8;
const IO_REPARSE_TAG_APPEXECLINK: u32 = 0x8000_001b;

pub(super) fn guard(local: &Path, exe: &Path) -> Option<(PathBuf, Vec<File>, File)> {
    let base = local.join("Microsoft/WindowsApps").join(FAMILY_NAME);
    let dirs = lock_dirs(&base)?;
    let path = base.join(ALIAS);
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .ok()?;
    let mut bytes = [0; 4096];
    let mut len = 0;
    // SAFETY: live handle, no input, output buffer matches capacity and len is writable.
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_GET_REPARSE_POINT,
            null_mut(),
            0,
            bytes.as_mut_ptr().cast(),
            bytes.len() as u32,
            &mut len,
            null_mut(),
        )
    } == 0
        || !matches_target(bytes.get(..len as usize)?, exe)
    {
        return None;
    }
    Some((path, dirs, file))
}

fn matches_target(bytes: &[u8], exe: &Path) -> bool {
    let Some(header) = bytes.get(..12) else { return false };
    // The packed AppExecLink version-3 data holds family, AUMID, target and app type as
    // terminated UTF-16 strings. Unknown layouts are not safe to activate.
    if header[..4] != IO_REPARSE_TAG_APPEXECLINK.to_le_bytes()
        || bytes.len() != 8 + usize::from(u16::from_le_bytes([header[4], header[5]]))
        || header[8..12] != 3_u32.to_le_bytes()
    {
        return false;
    }
    let (words, odd) = bytes[12..].as_chunks::<2>();
    let words = words.iter().copied().map(u16::from_le_bytes).collect::<Vec<_>>();
    let mut fields = words.split(|&word| word == 0);
    if !odd.is_empty() {
        return false;
    }
    let mut take = || fields.next().map(std::ffi::OsString::from_wide);
    take().is_some_and(|family| family == FAMILY_NAME)
        && take().is_some_and(|id| id == AUMID)
        && take().is_some_and(|target| Path::new(&target) == exe)
        && take().is_some_and(|kind| kind == "0")
        && fields.next() == Some(&[])
        && fields.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(family: &str, id: &str, exe: &str) -> Vec<u8> {
        let mut payload = 3_u32.to_le_bytes().to_vec();
        for s in [family, id, exe, "0"] {
            payload.extend(s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        }
        [IO_REPARSE_TAG_APPEXECLINK.to_le_bytes().as_slice(), &(payload.len() as u16).to_le_bytes(), &[0, 0], &payload]
            .concat()
    }

    #[test]
    fn requires_the_exact_package_app_and_installed_target() {
        let name = r"C:\Program Files\WindowsApps\Claude_1.0.0.0_x64__pzs8sxrjxfjjc\app\Claude.exe";
        let exe = Path::new(name);
        let data = record(FAMILY_NAME, AUMID, name);
        assert!(matches_target(&data, exe));
        assert!(!matches_target(&data[..data.len() - 1], exe));
        assert!(!matches_target(&record("Claude_fake", AUMID, name), exe));
        assert!(!matches_target(&record(FAMILY_NAME, "Other!Claude", name), exe));
        assert!(!matches_target(&record(FAMILY_NAME, AUMID, r"C:\fake\Claude.exe"), exe));
    }
}
