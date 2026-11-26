//! A Windows npm shim runs through cmd.exe. Its immediate Node/Claude child owns the session.

#[derive(Default)]
pub struct Processes(Vec<Process>);

struct Process {
    pid: u32,
    parent: u32,
    image: String,
}

impl Processes {
    pub fn wraps(&self, parent: u32, pid: u32) -> bool {
        self.0.iter().any(|p| p.pid == parent && p.image.eq_ignore_ascii_case("cmd.exe"))
            && self.0.iter().any(|p| {
                p.pid == pid
                    && p.parent == parent
                    && (p.image.eq_ignore_ascii_case("node.exe") || p.image.eq_ignore_ascii_case("claude.exe"))
            })
    }

    #[cfg(not(windows))]
    pub fn current() -> Self {
        Self::default()
    }

    #[cfg(windows)]
    pub fn current() -> Self {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
        };

        // SAFETY: TH32CS_SNAPPROCESS ignores the second argument and returns a snapshot handle.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Self::default();
        }
        let mut p = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        // SAFETY: snapshot is valid; p is initialized and the size describes its writable fields.
        let mut ok = unsafe { Process32FirstW(snapshot, &mut p) };
        let mut entries = Vec::new();
        while ok != 0 {
            let end = p.szExeFile.iter().position(|&c| c == 0).unwrap_or(p.szExeFile.len());
            entries.push(Process {
                pid: p.th32ProcessID,
                parent: p.th32ParentProcessID,
                image: String::from_utf16_lossy(&p.szExeFile[..end]),
            });
            // SAFETY: the snapshot is still open and p remains initialized and writable.
            ok = unsafe { Process32NextW(snapshot, &mut p) };
        }
        // SAFETY: this handle is owned by this call and is closed exactly once.
        unsafe { CloseHandle(snapshot) };
        Self(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_immediate_claude_or_node_child_of_the_shim_can_match() {
        let p = |pid, parent, image: &str| Process { pid, parent, image: image.into() };
        let all =
            Processes(vec![p(10, 1, "cmd.exe"), p(11, 10, "node.exe"), p(12, 11, "claude.exe"), p(13, 10, "foo.exe")]);
        assert!(all.wraps(10, 11));
        assert!(!all.wraps(10, 12));
        assert!(!all.wraps(10, 13));
        assert!(!all.wraps(11, 12));
    }
}
