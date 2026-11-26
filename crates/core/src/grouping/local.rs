//! Reject remote/device/redirected paths before repository discovery probes them.
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

#[cfg(windows)]
fn drive(path: &Path) -> Option<u8> {
    use std::path::Prefix;
    match path.components().next()? {
        Component::Prefix(p) if path.is_absolute() => match p.kind() {
            Prefix::Disk(letter) => Some(letter.to_ascii_uppercase()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(windows)]
fn fixed(letter: u8) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, QueryDosDeviceW};
    let name = [u16::from(letter), u16::from(b':'), 0];
    let root = [name[0], name[1], u16::from(b'\\'), 0];
    let mut device = [0; 512];
    // SAFETY: root and name are terminated UTF-16; device owns its writable capacity.
    if unsafe { QueryDosDeviceW(name.as_ptr(), device.as_mut_ptr(), device.len() as u32) } == 0 {
        return None;
    }
    let target = String::from_utf16_lossy(&device[..device.iter().position(|&c| c == 0).unwrap_or(device.len())]);
    let local = target
        .strip_prefix(r"\Device\HarddiskVolume")
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()));
    // SAFETY: root is a terminated drive-root string.
    (local && unsafe { GetDriveTypeW(root.as_ptr()) } == 3).then_some(target)
}

fn normalize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    let mut depth = 0;
    for component in path.components() {
        match component {
            Component::ParentDir if depth != 0 => {
                out.pop();
                depth -= 1;
            }
            Component::ParentDir => return None,
            Component::Normal(name) => {
                #[cfg(windows)]
                {
                    let name = name.to_str()?;
                    let device = name.split('.').next()?.trim_end_matches(' ').to_ascii_uppercase();
                    let serial_device =
                        device.strip_prefix("COM").or_else(|| device.strip_prefix("LPT")).is_some_and(|suffix| {
                            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
                        });
                    if name.ends_with([' ', '.'])
                        || name.contains([':', '\0'])
                        || matches!(device.as_str(), "CON" | "AUX" | "PRN" | "NUL" | "CONIN$" | "CONOUT$")
                        || serial_device
                    {
                        return None;
                    }
                }
                out.push(name);
                depth += 1;
            }
            Component::CurDir => {}
            part => out.push(part.as_os_str()),
        }
    }
    Some(out)
}

/// A local fixed volume. Absolute file and directory reads do not follow reparse points on Windows.
pub struct Volume {
    #[cfg(windows)]
    letter: u8,
    #[cfg(windows)]
    device: String,
}

impl Volume {
    pub fn at(path: &Path) -> Option<Self> {
        if !path.is_absolute() {
            return None;
        }
        #[cfg(windows)]
        {
            let letter = drive(path)?;
            Some(Self { letter, device: fixed(letter)? })
        }
        #[cfg(not(windows))]
        Some(Self {})
    }

    fn accepts(&self, path: &Path) -> bool {
        #[cfg(windows)]
        return drive(path) == Some(self.letter);
        #[cfg(not(windows))]
        path.is_absolute()
    }

    pub fn folder(&self, path: &Path) -> Option<PathBuf> {
        if !self.accepts(path) {
            return None;
        }
        let path = normalize(path)?;
        self.metadata(&path).ok()?.is_dir().then_some(path)
    }

    pub fn metadata(&self, path: &Path) -> std::io::Result<std::fs::Metadata> {
        self.open(path, false)?.metadata()
    }

    pub(super) fn file(&self, path: &Path, read: bool) -> Option<std::fs::File> {
        let file = self.open(&normalize(path)?, read).ok()?;
        file.metadata().ok()?.is_file().then_some(file)
    }

    #[cfg(not(windows))]
    pub fn open(&self, path: &Path, _: bool) -> std::io::Result<std::fs::File> {
        let path = normalize(path).filter(|p| self.accepts(p)).ok_or(std::io::ErrorKind::InvalidInput)?;
        for at in path.ancestors() {
            if crate::util::reparse(&std::fs::symlink_metadata(at)?) {
                return Err(std::io::ErrorKind::PermissionDenied.into());
            }
        }
        std::fs::File::open(path)
    }

    #[cfg(windows)]
    pub fn open(&self, path: &Path, read: bool) -> std::io::Result<std::fs::File> {
        use std::io::{Error, ErrorKind};
        use std::os::windows::io::FromRawHandle;
        use std::ptr::{null, null_mut};
        use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
        use windows_sys::Wdk::Storage::FileSystem::{FILE_SYNCHRONOUS_IO_NONALERT, NtOpenFile};
        use windows_sys::Win32::Foundation::{
            OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, SYNCHRONIZE,
        };
        use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

        if !self.accepts(path) {
            return Err(ErrorKind::PermissionDenied.into());
        }
        let path = normalize(path).ok_or(ErrorKind::InvalidInput)?;
        let tail = path.to_str().and_then(|s| s.get(2..)).ok_or(ErrorKind::InvalidInput)?;
        let mut wide = self.device.encode_utf16().chain(tail.encode_utf16()).collect::<Vec<_>>();
        let len = u16::try_from(wide.len() * 2).map_err(|_| ErrorKind::InvalidInput)?;
        let name = UNICODE_STRING { Length: len, MaximumLength: len, Buffer: wide.as_mut_ptr() };
        let attributes = OBJECT_ATTRIBUTES {
            Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: null_mut(),
            ObjectName: &name,
            Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
            SecurityDescriptor: null(),
            SecurityQualityOfService: null(),
        };
        let mut status = IO_STATUS_BLOCK::default();
        let mut handle = null_mut();
        let access = FILE_READ_ATTRIBUTES | SYNCHRONIZE | if read { FILE_READ_DATA } else { 0 };
        // SAFETY: native name and counted UTF-16 buffer are live for the call. The synchronous
        // open returns one owned HANDLE on success. OBJ_DONT_REPARSE applies to the entire name.
        let result = unsafe {
            NtOpenFile(
                &mut handle,
                access,
                &attributes,
                &mut status,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_SYNCHRONOUS_IO_NONALERT,
            )
        };
        if result < 0 {
            // SAFETY: conversion accepts any NTSTATUS without a pointer.
            return Err(Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(result) } as i32));
        }
        if handle.is_null() {
            return Err(ErrorKind::Other.into());
        }
        // SAFETY: transfer the unique handle returned by a successful NtOpenFile to File.
        let file = unsafe { std::fs::File::from_raw_handle(handle.cast()) };
        if crate::util::reparse(&file.metadata()?) {
            return Err(ErrorKind::PermissionDenied.into());
        }
        Ok(file)
    }

    /// Bounded directory listing. On Windows the enumeration uses the validated handle, not a
    /// second open by path that could follow a junction swapped in after validation.
    #[cfg(windows)]
    pub fn entries(&self, path: &Path, max: usize) -> std::io::Result<Vec<OsString>> {
        use std::io::{Error, ErrorKind};
        use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
        use windows_sys::Win32::Foundation::ERROR_NO_MORE_FILES;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FULL_DIR_INFO, FileFullDirectoryInfo, GetFileInformationByHandleEx,
        };

        let dir = self.open(path, true)?;
        if !dir.metadata()?.is_dir() {
            return Err(ErrorKind::NotADirectory.into());
        }
        let mut entries = Vec::new();
        let mut buffer = [0_u64; 1024];
        while entries.len() < max {
            // SAFETY: an owned directory handle and an aligned writable buffer of the stated size.
            let ok = unsafe {
                GetFileInformationByHandleEx(
                    dir.as_raw_handle(),
                    FileFullDirectoryInfo,
                    buffer.as_mut_ptr().cast(),
                    size_of_val(&buffer) as u32,
                )
            };
            if ok == 0 {
                let err = Error::last_os_error();
                if err.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                    break;
                }
                return Err(err);
            }
            let mut offset = 0;
            loop {
                let name_at = offset + std::mem::offset_of!(FILE_FULL_DIR_INFO, FileName);
                if !offset.is_multiple_of(align_of::<FILE_FULL_DIR_INFO>())
                    || offset + size_of::<FILE_FULL_DIR_INFO>() > size_of_val(&buffer)
                {
                    return Err(ErrorKind::InvalidData.into());
                }
                // SAFETY: the aligned record prefix fits in the initialized buffer.
                let entry = unsafe { &*buffer.as_ptr().cast::<u8>().add(offset).cast::<FILE_FULL_DIR_INFO>() };
                let bytes = entry.FileNameLength as usize;
                if bytes == 0 || !bytes.is_multiple_of(2) || name_at + bytes > size_of_val(&buffer) {
                    return Err(ErrorKind::InvalidData.into());
                }
                // SAFETY: the UTF-16 name starts at an aligned offset and is within the buffer.
                let name =
                    unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>().add(name_at).cast(), bytes / 2) };
                let name = OsString::from_wide(name);
                let mut components = Path::new(&name).components();
                if matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none() {
                    entries.push(name);
                }
                let next = entry.NextEntryOffset as usize;
                if next == 0 || entries.len() == max {
                    break;
                }
                if offset + next < name_at + bytes {
                    return Err(ErrorKind::InvalidData.into());
                }
                offset += next;
            }
        }
        Ok(entries)
    }

    #[cfg(not(windows))]
    pub fn entries(&self, path: &Path, max: usize) -> std::io::Result<Vec<OsString>> {
        self.open(path, true)?;
        std::fs::read_dir(path)?.take(max).map(|row| row.map(|entry| entry.file_name())).collect()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn entries_cross_native_batches_without_exceeding_the_bound() {
        let test = crate::util::testing::TempDir::new("volume-entries");
        let dir = &test.0;
        let expected: Vec<_> = (0..113).map(|i| format!("{i:03}-{}.md", "x".repeat(70))).collect();
        for name in &expected {
            std::fs::write(dir.join(name), "").unwrap();
        }
        let volume = Volume::at(dir).unwrap();
        assert_eq!(volume.entries(dir, 75).unwrap().len(), 75);
        let mut all: Vec<_> = volume.entries(dir, 200).unwrap().into_iter().map(|s| s.into_string().unwrap()).collect();
        all.sort();
        assert_eq!(all, expected);
    }

    #[test]
    fn network_device_and_wrong_drive_are_refused_without_opening_them() {
        let local = Volume { letter: b'C', device: String::new() };
        for path in
            [r"\\test.invalid\share\repo", r"\\?\UNC\test.invalid\share", r"\\.\C:\repo", r"D:\repo", r"C:relative"]
        {
            assert!(!local.accepts(Path::new(path)), "{path}");
        }
        assert!(normalize(Path::new(r"C:\..\repo")).is_none());
        assert_eq!(normalize(Path::new(r"C:\repo\.git\worktrees\foo\..\..")).unwrap(), Path::new(r"C:\repo\.git"));
    }

    #[test]
    fn only_reserved_serial_and_parallel_device_suffixes_are_refused() {
        for name in ["COM1", "lpt9.txt", "COM¹", "LPT³ .txt", "CON .txt"] {
            assert!(normalize(&Path::new(r"C:\work").join(name)).is_none(), "{name}");
        }
        for name in ["comp", "lptx", "COM10", "COMP.txt"] {
            assert!(normalize(&Path::new(r"C:\work").join(name)).is_some(), "{name}");
        }
    }

    #[test]
    fn named_file_open_refuses_an_intermediate_junction_for_metadata_and_reads() {
        let dir = crate::util::testing::TempDir::new("named-file");
        let real = dir.0.join("real");
        crate::util::testing::put(&real.join("note.md"), "# A note");
        let alias = dir.0.join("redirect");
        crate::share::junction::create(&alias, &real).unwrap();
        for read in [false, true] {
            assert!(crate::grouping::open_local_file(&real.join("note.md"), read).is_some());
            assert!(crate::grouping::open_local_file(&real, read).is_none());
            assert!(crate::grouping::open_local_file(&alias.join("note.md"), read).is_none());
        }
        std::fs::remove_dir(alias).unwrap();
    }
}
