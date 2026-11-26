use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Compile a native fake, never use a shell or depend on a real installation.
pub fn fake_program() -> PathBuf {
    static PROGRAM: OnceLock<PathBuf> = OnceLock::new();
    PROGRAM
        .get_or_init(|| {
            let dir = std::env::temp_dir().join(format!(
                "opencode-cli-fake-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            std::fs::create_dir(&dir).unwrap();
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
