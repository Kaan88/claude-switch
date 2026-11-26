//! Embed the Explorer icon and the version read by Hub in the Windows MSVC executable.

use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }
    println!("cargo:rerun-if-env-changed=ProgramFiles(x86)");

    let rc = find_rc().expect("Windows SDK resource compiler (rc.exe) is required");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let icon = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo CARGO_MANIFEST_DIR"))
        .join("../../assets/icon.ico");
    let script = out.join("claude-switch.rc");
    let res = out.join("claude-switch.res");
    let version = env!("CARGO_PKG_VERSION");
    let (major, minor, patch) =
        (env!("CARGO_PKG_VERSION_MAJOR"), env!("CARGO_PKG_VERSION_MINOR"), env!("CARGO_PKG_VERSION_PATCH"));
    let numeric = format!("{major},{minor},{patch},0");
    fs::write(
        &script,
        format!(
            "1 ICON \"{icon}\"\n\
             1 VERSIONINFO\n FILEVERSION {numeric}\n PRODUCTVERSION {numeric}\n\
             FILEOS 0x40004\n FILETYPE 1\n\
             BEGIN\n BLOCK \"StringFileInfo\"\n BEGIN\n BLOCK \"040904B0\"\n BEGIN\n\
             VALUE \"CompanyName\", \"Kaan\\0\"\n VALUE \"FileDescription\", \"Claude Switch\\0\"\n\
             VALUE \"FileVersion\", \"{version}\\0\"\n VALUE \"ProductVersion\", \"{version}\\0\"\n\
             VALUE \"InternalName\", \"claude-switch\\0\"\n\
             VALUE \"OriginalFilename\", \"claude-switch.exe\\0\"\n\
             VALUE \"ProductName\", \"Claude Switch\\0\"\n END\n END\n\
             BLOCK \"VarFileInfo\"\n BEGIN\n VALUE \"Translation\", 0x0409, 1200\n END\n END\n",
            icon = icon.display().to_string().replace('\\', "/"),
        ),
    )
    .expect("write Windows resource script");
    let status = Command::new(rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg(&script)
        .status()
        .expect("run Windows resource compiler");
    assert!(status.success(), "rc.exe failed: {status}");
    println!("cargo:rustc-link-arg-bins={}", res.display());
}

fn find_rc() -> Option<PathBuf> {
    let sdk = PathBuf::from(env::var_os("ProgramFiles(x86)")?).join("Windows Kits/10/bin");
    if !sdk.is_absolute() {
        return None;
    }
    fs::read_dir(sdk)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("x64/rc.exe"))
        .filter(|rc| rc.is_file())
        .max()
}
