//! Locate the installed application, not an updater, protocol handler, or PATH alias.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
use windows_sys::Win32::Security::Cryptography::{CERT_NAME_ATTR_TYPE, CertGetNameStringW, szOID_ORGANIZATION_NAME};
use windows_sys::Win32::Security::WinTrust::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::Storage::Packaging::Appx::*;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, SCS_32BIT_BINARY, SCS_64BIT_BINARY};
use windows_sys::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath,
};
use windows_sys::core::{GUID, w};

const EXE: &str = "Claude.exe";
const FAMILY: *const u16 = w!("Claude_pzs8sxrjxfjjc");

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    let mut path = null_mut();
    // SAFETY: Windows writes a pointer to a NUL-terminated string allocated with CoTaskMemAlloc.
    let status = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY as u32, null_mut(), &mut path) };
    if path.is_null() {
        return None;
    }
    // SAFETY: on success the string lives up to CoTaskMemFree, which also releases memory on error.
    unsafe {
        let folder = (status >= 0).then(|| {
            let n = (0..).find(|&i| *path.add(i) == 0).unwrap();
            PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(path, n)))
        });
        CoTaskMemFree(path.cast());
        folder
    }
}

fn fixed_path(path: &Path) -> bool {
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else { return false };
    let Prefix::Disk(letter) = prefix.kind() else { return false };
    if parts.next() != Some(Component::RootDir) || !parts.all(|p| matches!(p, Component::Normal(_))) {
        return false;
    }
    let drive = [u16::from(letter), u16::from(b':'), 0];
    let root = [drive[0], drive[1], u16::from(b'\\'), 0];
    let mut target = [0u16; 512];
    // SAFETY: the strings are NUL-terminated and the output buffer has the supplied capacity.
    let n = unsafe { QueryDosDeviceW(drive.as_ptr(), target.as_mut_ptr(), target.len() as u32) } as usize;
    let Some(end) = target[..n].iter().position(|&c| c == 0) else { return false };
    let target = String::from_utf16_lossy(&target[..end]);
    target
        .strip_prefix(r"\Device\HarddiskVolume")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
        // SAFETY: `root` is the NUL-terminated drive root.
        && unsafe { GetDriveTypeW(root.as_ptr()) } == DRIVE_FIXED
}

fn version(text: &str) -> Option<[u32; 4]> {
    let mut parts = [0; 4];
    for (i, part) in text.split('.').enumerate() {
        if part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        *parts.get_mut(i)? = part.parse().ok()?;
    }
    Some(parts)
}

fn package_version(name: &str) -> Option<[u32; 4]> {
    let (version, arch) = name.strip_prefix("Claude_")?.strip_suffix("__pzs8sxrjxfjjc")?.split_once('_')?;
    matches!(arch, "x64" | "arm64" | "x86").then(|| self::version(version)).flatten()
}

fn packages() -> Vec<PathBuf> {
    let (mut count, mut len) = (0, 0);
    // SAFETY: the first call asks for sizes only; the family is the fixed NUL-terminated identifier.
    if unsafe {
        FindPackagesByPackageFamily(
            FAMILY,
            PACKAGE_FILTER_HEAD,
            &mut count,
            null_mut(),
            &mut len,
            null_mut(),
            null_mut(),
        )
    } != ERROR_INSUFFICIENT_BUFFER
        || !(1..=128).contains(&count)
        || !(1..=32_768).contains(&len)
    {
        return vec![];
    }
    let mut names = vec![null_mut(); count as usize];
    let mut props = vec![0; count as usize];
    let mut buf = vec![0u16; len as usize];
    // SAFETY: arrays have the capacities passed to the OS. The name pointers point into `buf`.
    if unsafe {
        FindPackagesByPackageFamily(
            FAMILY,
            PACKAGE_FILTER_HEAD,
            &mut count,
            names.as_mut_ptr(),
            &mut len,
            buf.as_mut_ptr(),
            props.as_mut_ptr(),
        )
    } != ERROR_SUCCESS
    {
        return vec![];
    }
    let mut found = Vec::new();
    for (name, properties) in names.into_iter().zip(props).take(count as usize) {
        if name.is_null() || properties & PACKAGE_PROPERTY_DEVELOPMENT_MODE != 0 {
            continue;
        }
        // SAFETY: a successful call provides NUL-terminated names residing in the live `buf`.
        let name_len = unsafe { (0..).find(|&i| *name.add(i) == 0).unwrap() };
        let name_text = unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(name, name_len)) };
        if let Some(version) = package_version(&name_text)
            && let Some(dir) = package_path(name)
            && dir.file_name().is_some_and(|s| s.eq_ignore_ascii_case(&name_text))
        {
            found.push((version, dir.join("app").join(EXE)));
        }
    }
    newest_first(found)
}

fn package_path(name: *const u16) -> Option<PathBuf> {
    let mut len = 0;
    // SAFETY: caller supplies a valid package full name. First call queries the required capacity.
    if unsafe { GetPackagePathByFullName(name, &mut len, null_mut()) } != ERROR_INSUFFICIENT_BUFFER
        || !(2..=32_768).contains(&len)
    {
        return None;
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `buf` has the requested capacity; OS writes the NUL-terminated package path.
    if unsafe { GetPackagePathByFullName(name, &mut len, buf.as_mut_ptr()) } != ERROR_SUCCESS {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0)?;
    Some(PathBuf::from(OsString::from_wide(&buf[..end])))
}

fn newest_first(mut found: Vec<([u32; 4], PathBuf)>) -> Vec<PathBuf> {
    found.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().map(|(_, path)| path).collect()
}

fn squirrel(root: &Path) -> Vec<PathBuf> {
    let Some(_dirs) = lock_dirs(root) else { return vec![] };
    let Ok(entries) = fs::read_dir(root) else { return vec![] };
    newest_first(
        entries
            .flatten()
            .filter_map(|entry| {
                let ver = version(entry.file_name().to_str()?.strip_prefix("app-")?)?;
                entry.file_type().ok()?.is_dir().then(|| (ver, entry.path().join(EXE)))
            })
            .collect(),
    )
}

fn candidates(packages: Vec<PathBuf>, local: Option<&Path>, program_files: &[PathBuf]) -> Vec<(PathBuf, bool)> {
    let mut found: Vec<_> = packages.into_iter().map(|path| (path, true)).collect();
    if let Some(local) = local {
        let root = local.join("AnthropicClaude");
        found.extend(squirrel(&root).into_iter().map(|path| (path, false)));
        found.push((root.join(EXE), false));
        found.extend(flat(&local.join("Programs")));
    }
    found.extend(program_files.iter().flat_map(|root| flat(root)));
    found
}

fn flat(root: &Path) -> impl Iterator<Item = (PathBuf, bool)> + '_ {
    ["Claude", "AnthropicClaude"].into_iter().map(move |name| (root.join(name).join(EXE), false))
}

fn installed() -> Vec<(PathBuf, bool)> {
    let local = known_folder(&FOLDERID_LocalAppData);
    let dirs: Vec<_> =
        [&FOLDERID_ProgramFiles, &FOLDERID_ProgramFilesX86].into_iter().filter_map(known_folder).collect();
    candidates(packages(), local.as_deref(), &dirs)
}

fn lock_dirs(path: &Path) -> Option<Vec<File>> {
    if !fixed_path(path) {
        return None;
    }
    let mut parts: Vec<_> = path.ancestors().collect();
    parts.reverse();
    parts
        .into_iter()
        .map(|dir| {
            let file = OpenOptions::new()
                .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .open(dir)
                .ok()?;
            let meta = file.metadata().ok()?;
            (meta.is_dir() && meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0).then_some(file)
        })
        .collect()
}

/// Keeps the verified file open, denying writes and replacement through process creation.
#[must_use]
pub struct DesktopGuard {
    _dirs: Vec<File>,
    _file: File,
}

fn verify(path: &Path, packaged: bool) -> Option<DesktopGuard> {
    if !fixed_path(path) || !path.file_name().is_some_and(|name| name.eq_ignore_ascii_case(EXE)) {
        return None;
    }
    // MSIX directories are controlled by Windows package deployment. Do not try to open/enumerate
    // the protected WindowsApps root; use the registered package's exact executable path.
    let dirs = if packaged { Vec::new() } else { lock_dirs(path.parent()?)? };
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return None;
    }
    let name = wide(path);
    let mut kind = 0;
    // SAFETY: `name` is NUL-terminated, and `kind` is a writable DWORD.
    if unsafe { GetBinaryTypeW(name.as_ptr(), &mut kind) } == 0 || !matches!(kind, SCS_32BIT_BINARY | SCS_64BIT_BINARY)
    {
        return None;
    }
    signed_by_anthropic(&name, &file).then_some(DesktopGuard { _dirs: dirs, _file: file })
}

fn signed_by_anthropic(name: &[u16], file: &File) -> bool {
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let mut info = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: name.as_ptr(),
        hFile: file.as_raw_handle(),
        pgKnownSubject: null_mut(),
    };
    let mut trust = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut info },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_NONE | WTD_DISABLE_MD2_MD4,
        ..Default::default()
    };
    // SAFETY: the handle, path and structures stay live until the state is released below.
    let ok = unsafe {
        WinVerifyTrust(null_mut(), &mut action, (&raw mut trust).cast()) == 0 && anthropic_signer(trust.hWVTStateData)
    };
    trust.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: CLOSE releases the state for the same data and policy, also on verification failure.
    unsafe { WinVerifyTrust(null_mut(), &mut action, (&raw mut trust).cast()) };
    ok
}

unsafe fn anthropic_signer(state: windows_sys::Win32::Foundation::HANDLE) -> bool {
    // SAFETY: the provider owns these pointers for the lifetime of the WinVerifyTrust state.
    unsafe {
        let data = WTHelperProvDataFromStateData(state);
        if data.is_null() {
            return false;
        }
        let signer = WTHelperGetProvSignerFromChain(data, 0, 0, 0);
        if signer.is_null() {
            return false;
        }
        let leaf = WTHelperGetProvCertFromChain(signer, 0);
        if leaf.is_null() || (*leaf).pCert.is_null() {
            return false;
        }
        let mut org = [0u16; 64];
        let n = CertGetNameStringW(
            (*leaf).pCert,
            CERT_NAME_ATTR_TYPE,
            0,
            szOID_ORGANIZATION_NAME.cast(),
            org.as_mut_ptr(),
            org.len() as u32,
        ) as usize;
        n > 1 && n < org.len() && String::from_utf16_lossy(&org[..n - 1]) == "Anthropic, PBC"
    }
}

pub(super) fn resolve() -> Option<PathBuf> {
    installed().into_iter().find_map(|(path, packaged)| verify(&path, packaged).map(|_| path))
}

pub fn guard_desktop_exe(path: &Path) -> io::Result<DesktopGuard> {
    installed()
        .into_iter()
        .find_map(|(candidate, packaged)| (candidate == path).then(|| verify(path, packaged)).flatten())
        .ok_or_else(|| io::Error::other("Claude Desktop is no longer a verified installed executable."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testing::{TempDir, put};

    #[test]
    fn registered_package_name_requires_claudes_exact_publisher_and_native_architecture() {
        assert_eq!(package_version("Claude_6.28899.6.0_x64__pzs8sxrjxfjjc"), Some([6, 28899, 6, 0]));
        assert!(package_version("Claude_2.0.0.0_arm64__pzs8sxrjxfjjc").is_some());
        for rejected in [
            "Claude_6.28899.6.0_x64__another-publisher",
            "Other_6.28899.6.0_x64__pzs8sxrjxfjjc",
            "Claude_6.28899.6.0_neutral__pzs8sxrjxfjjc",
            "Claude_6.28899.6.0_x64_en-us_pzs8sxrjxfjjc",
        ] {
            assert!(package_version(rejected).is_none(), "{rejected}");
        }
    }

    #[test]
    fn known_layouts_include_versioned_squirrel_in_numeric_order_and_fixed_machine_paths() {
        let temp = TempDir::new("desktop-search");
        let squirrel = temp.0.join("AnthropicClaude");
        put(&squirrel.join("Update.exe"), "");
        for folder in ["app-1.9.0", "app-1.10.0", "app-copy"] {
            put(&squirrel.join(folder).join(EXE), "");
        }
        put(&squirrel.join("app-100.1.0"), "not a directory");
        let package = temp.0.join("WindowsApps/Claude_2.0.0.0_x64__pzs8sxrjxfjjc/app").join(EXE);
        let machine = temp.0.join("Program Files");
        let found = candidates(vec![package.clone()], Some(&temp.0), std::slice::from_ref(&machine));
        assert_eq!(
            found,
            [
                (package, true),
                (squirrel.join("app-1.10.0").join(EXE), false),
                (squirrel.join("app-1.9.0").join(EXE), false),
                (squirrel.join(EXE), false),
                (temp.0.join("Programs/Claude").join(EXE), false),
                (temp.0.join("Programs/AnthropicClaude").join(EXE), false),
                (machine.join("Claude").join(EXE), false),
                (machine.join("AnthropicClaude").join(EXE), false),
            ]
        );
    }

    #[test]
    fn native_but_unsigned_and_non_native_exes_are_refused() {
        let t = TempDir::new("desktop-signature");
        let exe = t.0.join(EXE);
        fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
        let mut kind = 0;
        // SAFETY: the path is NUL-terminated; `kind` is a writable DWORD.
        assert_ne!(unsafe { GetBinaryTypeW(wide(&exe).as_ptr(), &mut kind) }, 0);
        assert!(lock_dirs(&t.0).is_some());
        assert!(verify(&exe, false).is_none());
        put(&exe, "not a Windows executable");
        assert!(verify(&exe, false).is_none());
        assert!(!fixed_path(Path::new(r"\\server\share\Claude.exe")));
    }

    #[test]
    #[ignore = "read-only check of this Windows user's installed Claude package metadata"]
    fn current_users_claude_package_resolves_to_the_manifest_layout() {
        let known = packages();
        assert!(!known.is_empty());
        assert!(known.iter().all(|p| p.ends_with(r"app\Claude.exe") && fixed_path(p)));
        println!("{known:?}");
    }
}
