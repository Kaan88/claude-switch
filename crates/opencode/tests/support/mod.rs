use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Compile a native fake, never use a shell or depend on a real installation. It is built once per
/// test program and shared by its tests, so it cannot go when one of them ends: it is built under
/// cargo's own temporary folder for tests (`target/tmp`, not the user's %TEMP%), where
/// the fakes of runs more than a day old are removed.
pub fn fake_program() -> PathBuf {
    static PROGRAM: OnceLock<PathBuf> = OnceLock::new();
    PROGRAM
        .get_or_init(|| {
            // Integration tests have cargo's folder; the unit tests that share this module use
            // the same place: `tmp` in the build folder the test program is in (the nearest
            // folder up with cargo's CACHEDIR.TAG).
            let base = option_env!("CARGO_TARGET_TMPDIR").map(PathBuf::from).unwrap_or_else(|| {
                let exe = std::env::current_exe().unwrap();
                let root = exe.ancestors().find(|dir| dir.join("CACHEDIR.TAG").is_file());
                root.unwrap_or_else(|| exe.parent().unwrap()).join("tmp")
            });
            let base = base.as_path();
            let day = std::time::Duration::from_secs(24 * 3600);
            for entry in std::fs::read_dir(base).into_iter().flatten().filter_map(Result::ok) {
                let old = entry.metadata().and_then(|m| m.modified()).is_ok_and(|at| at.elapsed().is_ok_and(|age| age > day));
                if old && entry.file_name().to_string_lossy().starts_with("opencode-cli-fake-") {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
            let dir = base.join(format!(
                "opencode-cli-fake-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let source = dir.join("fake.rs");
            let exe = dir.join(format!("fake{}", std::env::consts::EXE_SUFFIX));
            std::fs::write(&source, include_str!("fake.rs")).unwrap();
            let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg(&source)
                .arg("--edition=2024")
                .arg("-o")
                .arg(&exe)
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            exe
        })
        .clone()
}
