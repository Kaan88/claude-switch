use std::fs::{self, OpenOptions};
use std::io;
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
use std::path::{Component, Path, Prefix};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT};

const SET_REPARSE_POINT: u32 = 0x0009_00a4;
const MOUNT_POINT: u32 = 0xa000_0003;

fn buffer(target: &Path) -> io::Result<Vec<u8>> {
    let text = target.to_str().ok_or(io::ErrorKind::InvalidInput)?;
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    let absolute = Path::new(text);
    if !absolute.is_absolute()
        || !matches!(absolute.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)))
        || text.contains('\0')
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let substitute: Vec<_> = format!(r"\??\{text}").encode_utf16().collect();
    let print: Vec<_> = text.encode_utf16().collect();
    let data_len = 8 + (substitute.len() + print.len() + 2) * 2;
    if data_len + 8 > 16 * 1024 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut bytes = MOUNT_POINT.to_le_bytes().to_vec();
    for n in [data_len, 0, 0, substitute.len() * 2, (substitute.len() + 1) * 2, print.len() * 2] {
        bytes.extend((n as u16).to_le_bytes());
    }
    for unit in substitute.into_iter().chain([0]).chain(print).chain([0]) {
        bytes.extend(unit.to_le_bytes());
    }
    Ok(bytes)
}

pub(crate) fn create(link: &Path, target: &Path) -> io::Result<()> {
    let bytes = buffer(&fs::canonicalize(target)?)?;
    fs::create_dir(link)?;
    let result = (|| {
        let dir = OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .share_mode(0)
            .open(link)?;
        let mut returned = 0;
        // SAFETY: the open directory handle and exact input buffer live through the synchronous
        // call; no output buffer is requested and returned points to a live DWORD.
        if unsafe {
            windows_sys::Win32::System::IO::DeviceIoControl(
                dir.as_raw_handle().cast(),
                SET_REPARSE_POINT,
                bytes.as_ptr().cast(),
                bytes.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    // Leave an empty directory on failure; a later invocation can retry without deleting data.
    result
}
