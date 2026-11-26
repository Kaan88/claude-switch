//! Locate the installed application, not an updater, protocol handler, or PATH alias.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
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
use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;
use windows_sys::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath,
};
use windows_sys::core::{GUID, w};

const EXE: &str = "Claude.exe";
const FAMILY_NAME: &str = "Claude_pzs8sxrjxfjjc";
const FAMILY: *const u16 = w!("Claude_pzs8sxrjxfjjc");
pub(super) const REQUEST: &str = EXE;
mod alias;

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

fn packages() -> Vec<Package> {
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
        if name.is_null() {
            continue;
        }
        // SAFETY: a successful call provides NUL-terminated names residing in the live `buf`.
        let name_len = unsafe { (0..).find(|&i| *name.add(i) == 0).unwrap() };
        let name_text = unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(name, name_len)) };
        if let Some(version) = package_version(&name_text)
            && let Some(dir) = package_path(name)
            && dir.file_name().is_some_and(|s| s.eq_ignore_ascii_case(&name_text))
        {
            let mut origin = PackageOrigin_Unknown;
            // SAFETY: registered name is NUL-terminated and origin is writable.
            let known = unsafe { GetStagedPackageOrigin(name, &mut origin) } == ERROR_SUCCESS;
            let signed = known && signed_origin(origin) && properties & PACKAGE_PROPERTY_DEVELOPMENT_MODE == 0;
            found.push((version, Package { exe: dir.join("app").join(EXE), signed }));
        }
    }
    found.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.exe.cmp(&b.1.exe)));
    found.into_iter().map(|(_, pkg)| pkg).collect()
}

struct Package {
    exe: PathBuf,
    signed: bool,
}

fn signed_origin(origin: PackageOrigin) -> bool {
    [PackageOrigin_Store, PackageOrigin_DeveloperSigned].contains(&origin)
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

fn standalone(local: Option<&Path>, program_files: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Some(local) = local {
        let root = local.join("AnthropicClaude");
        found.extend(squirrel(&root));
        found.push(root.join(EXE));
        found.extend(flat(&local.join("Programs")));
    }
    found.extend(program_files.iter().flat_map(|root| flat(root)));
    found
}

fn flat(root: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    ["Claude", "AnthropicClaude"].into_iter().map(move |name| root.join(name).join(EXE))
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
    program: PathBuf,
    _alias: Option<File>,
    _dirs: Vec<File>,
    _file: File,
}

impl DesktopGuard {
    pub fn program(&self) -> &Path {
        &self.program
    }
}

fn verify(path: &Path, package: Option<&Package>, local: Option<&Path>) -> Option<DesktopGuard> {
    if !fixed_path(path) || !path.file_name().is_some_and(|name| name.eq_ignore_ascii_case(EXE)) {
        return None;
    }
    // MSIX directories are controlled by Windows package deployment. Do not try to open/enumerate
    // the protected WindowsApps root; use the registered package's exact executable path.
    let (program, dirs, alias) = if let Some(p) = package {
        if !p.signed || p.exe != path {
            return None;
        }
        let (alias_path, parents, alias) = alias::guard(local?, path)?;
        (alias_path, parents, Some(alias))
    } else {
        (path.to_owned(), lock_dirs(path.parent()?)?, None)
    };
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
    let embedded = native_pe(&file)?;
    let trusted = if embedded { signed_by_anthropic(&wide(path), &file) } else { package.is_some() };
    trusted.then_some(DesktopGuard { program, _alias: alias, _dirs: dirs, _file: file })
}

/// GetBinaryTypeW can return access denied for a protected WindowsApps path even though
/// we have a readable, locked file. Read the bounded PE headers from that same handle.
fn native_pe(mut file: &File) -> Option<bool> {
    let mut dos = [0; 64];
    file.read_exact(&mut dos).ok()?;
    if &dos[..2] != b"MZ" {
        return None;
    }
    file.seek(SeekFrom::Start(u32::from_le_bytes(dos[60..64].try_into().ok()?) as u64)).ok()?;
    let mut pe = [0; 26];
    file.read_exact(&mut pe).ok()?;
    let word = |i| pe.get(i..i + 2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes);
    let machine = word(4)?;
    let characteristics = word(22)?;
    let directory = match (machine, word(24)?) {
        (0x014c, 0x10b) => 96,
        (0x8664 | 0xaa64, 0x20b) => 112,
        _ => return None,
    };
    if &pe[..4] != b"PE\0\0" || characteristics & 0x2002 != 2 || usize::from(word(20)?) < directory + 40 {
        return None;
    }
    file.seek(SeekFrom::Current(directory as i64 - 6)).ok()?; // NumberOfRvaAndSizes
    let mut directories = [0; 44];
    file.read_exact(&mut directories).ok()?;
    let dword = |i| directories.get(i..i + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    if dword(0)? < 5 {
        return None;
    }
    match (dword(36)?, dword(40)?) {
        (0, 0) => Some(false),
        (offset, size)
            if offset > 0 && size > 0 && u64::from(offset) + u64::from(size) <= file.metadata().ok()?.len() =>
        {
            Some(true)
        }
        _ => None,
    }
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
    guard_desktop_exe().ok().map(|g| g.program)
}

pub fn guard_desktop_exe() -> Result<DesktopGuard, crate::Refusal> {
    let local = known_folder(&FOLDERID_LocalAppData);
    let packages = packages();
    let mut found = !packages.is_empty();
    for package in &packages {
        if let Some(guard) = verify(&package.exe, Some(package), local.as_deref()) {
            return Ok(guard);
        }
    }
    let dirs: Vec<_> =
        [&FOLDERID_ProgramFiles, &FOLDERID_ProgramFilesX86].into_iter().filter_map(known_folder).collect();
    for path in standalone(local.as_deref(), &dirs) {
        if let Some(guard) = verify(&path, None, None) {
            return Ok(guard);
        }
        // Attribute query only, rejecting redirects before touching the next component.
        found |= lock_dirs(path.parent().expect("candidate has parent"))
            .is_some_and(|_parents| fs::symlink_metadata(&path).is_ok_and(|meta| !meta.is_dir()));
    }
    Err(if found { crate::Refusal::UnverifiedDesktopApp } else { crate::Refusal::NoDesktopApp })
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
        let machine = temp.0.join("Program Files");
        let found = standalone(Some(&temp.0), std::slice::from_ref(&machine));
        assert_eq!(
            found,
            [
                squirrel.join("app-1.10.0").join(EXE),
                squirrel.join("app-1.9.0").join(EXE),
                squirrel.join(EXE),
                temp.0.join("Programs/Claude").join(EXE),
                temp.0.join("Programs/AnthropicClaude").join(EXE),
                machine.join("Claude").join(EXE),
                machine.join("AnthropicClaude").join(EXE),
            ]
        );
    }

    #[test]
    fn native_but_unsigned_and_non_native_exes_are_refused() {
        let t = TempDir::new("desktop-signature");
        let exe = t.0.join(EXE);
        fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
        assert_eq!(native_pe(&File::open(&exe).unwrap()), Some(false));
        assert!(lock_dirs(&t.0).is_some());
        assert!(verify(&exe, None, None).is_none());
        put(&exe, "not a Windows executable");
        assert_eq!(native_pe(&File::open(&exe).unwrap()), None);
        assert!(verify(&exe, None, None).is_none());
        assert!(!fixed_path(Path::new(r"\\server\share\Claude.exe")));
    }

    #[test]
    #[ignore = "read-only check of this Windows user's installed Claude package metadata"]
    fn current_users_claude_package_resolves_to_the_manifest_layout() {
        let known = packages();
        assert!(!known.is_empty());
        assert!(known.iter().all(|p| p.exe.ends_with(r"app\Claude.exe") && fixed_path(&p.exe)));
        for p in known {
            println!("{} signed-package-origin={}", p.exe.display(), p.signed);
        }
    }

    #[test]
    #[ignore = "read-only verification of this Windows user's installed Claude executable"]
    fn current_users_claude_desktop_resolves_as_verified() {
        for p in packages() {
            let file = File::open(&p.exe).unwrap();
            println!(
                "native PE (embedded cert): {:?}, Anthropic trust: {}, package alias matched: {}",
                native_pe(&file),
                signed_by_anthropic(&wide(&p.exe), &file),
                known_folder(&FOLDERID_LocalAppData).is_some_and(|local| alias::guard(&local, &p.exe).is_some())
            );
        }
        let desktop = resolve();
        println!("{desktop:?}");
        assert!(desktop.is_some(), "no verified Claude Desktop installation");
    }

    #[test]
    fn only_store_or_developer_signed_install_origin_can_attest_an_unsigned_inner_exe() {
        assert!(signed_origin(PackageOrigin_Store));
        assert!(signed_origin(PackageOrigin_DeveloperSigned));
        for origin in [
            PackageOrigin_Unknown,
            PackageOrigin_DeveloperUnsigned,
            PackageOrigin_Unsigned,
            PackageOrigin_LineOfBusiness,
        ] {
            assert!(!signed_origin(origin));
        }
        assert_ne!(crate::Refusal::NoDesktopApp.to_string(), crate::Refusal::UnverifiedDesktopApp.to_string());
    }
}
