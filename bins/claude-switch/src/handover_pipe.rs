//! The one request a second Claude Switch process may hand to the running window: open a saved
//! handover as a tab. A per-user named pipe, never a command channel.
//!
//! - The pipe's name carries the user's SID and its DACL grants only that SID; remote clients
//!   are refused and only one instance exists (`FILE_FLAG_FIRST_PIPE_INSTANCE`).
//! - Both ends check that the other process runs as the same Windows user before reading or
//!   writing anything; the client allows identification only, never impersonation.
//! - The payload is a fixed versioned list of validated words (`compact_cli::encode_request`),
//!   at most [`MAX_REQUEST`] bytes, decoded and revalidated by the receiving window; the reply is
//!   `opened` or `failed:` and the app's own message.

use std::time::Duration;

pub const MAX_REQUEST: usize = 2048;
const MAX_REPLY: usize = 1024;
/// How long the window may take to open the tab before the caller reports an unknown outcome.
pub const REPLY_WAIT: Duration = Duration::from_secs(30);

/// What the running window did with the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Opened the handover in a new tab.
    NewTab,
    /// Started continuing it in place, in the tab that runs the session.
    InPlace,
}

/// What the caller learned.
#[derive(Debug, PartialEq, Eq)]
pub enum Sent {
    /// No Claude Switch window of this user is listening.
    NotRunning,
    /// The running window answered.
    Answered(Result<Outcome, String>),
}

pub fn reply(result: &Result<Outcome, String>) -> Vec<u8> {
    match result {
        Ok(Outcome::NewTab) => b"opened".to_vec(),
        Ok(Outcome::InPlace) => b"continuing".to_vec(),
        Err(e) => {
            let mut text = format!("failed:{e}");
            while text.len() > MAX_REPLY {
                text.pop();
            }
            text.into_bytes()
        }
    }
}

pub fn read_reply(bytes: &[u8]) -> Result<Outcome, String> {
    match bytes {
        b"opened" => return Ok(Outcome::NewTab),
        b"continuing" => return Ok(Outcome::InPlace),
        _ => {}
    }
    match std::str::from_utf8(bytes).ok().and_then(|s| s.strip_prefix("failed:")) {
        Some(e) if !e.trim().is_empty() => Err(e.chars().filter(|c| !c.is_control()).collect()),
        _ => Err("The running Claude Switch did not confirm opening a Claude tab.".into()),
    }
}

#[cfg(windows)]
pub use win::{send, serve};

#[cfg(not(windows))]
pub fn send(_: &[u8]) -> Sent {
    Sent::NotRunning
}

#[cfg(not(windows))]
pub fn serve(_: impl Fn(Vec<u8>) -> Vec<u8> + Send + 'static) {}

#[cfg(windows)]
mod win {
    use super::*;

    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile,
        SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
    };
    use windows_sys::Win32::System::Pipes::*;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    struct Owned(HANDLE);

    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                // SAFETY: the handle was opened by this module and is closed once.
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    // SAFETY: a pipe handle may be used from any thread; each is used by one thread at a time.
    unsafe impl Send for Owned {}

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    /// The string SID of the user a process runs as.
    fn process_user(pid: u32) -> Option<String> {
        // SAFETY: every handle is checked and closed; the token buffer is sized by the first call
        // and aligned for TOKEN_USER; the SID string is freed with LocalFree.
        unsafe {
            let process = Owned(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid));
            if process.0.is_null() {
                return None;
            }
            let mut token = null_mut();
            if OpenProcessToken(process.0, TOKEN_QUERY, &mut token) == 0 {
                return None;
            }
            let token = Owned(token);
            let mut size = 0;
            GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut size);
            if size == 0 || size > 4096 {
                return None;
            }
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            if GetTokenInformation(token.0, TokenUser, buffer.as_mut_ptr().cast(), size, &mut size) == 0 {
                return None;
            }
            let user = &*(buffer.as_ptr() as *const TOKEN_USER);
            let mut text = null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 || text.is_null() {
                return None;
            }
            let len = (0..256).find(|&i| *text.add(i) == 0);
            let sid = len.map(|len| String::from_utf16_lossy(std::slice::from_raw_parts(text, len)));
            LocalFree(text.cast());
            sid.filter(|s| s.starts_with("S-1-") && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        }
    }

    fn me() -> Option<String> {
        // SAFETY: no arguments.
        process_user(unsafe { GetCurrentProcessId() })
    }

    fn name(sid: &str, suffix: &str) -> Vec<u16> {
        wide(&format!(r"\\.\pipe\claude-switch-handover-{sid}{suffix}"))
    }

    /// Only the same user, on the far end of `pipe`.
    fn same_user(pipe: HANDLE, server: bool, sid: &str) -> bool {
        let mut pid = 0;
        // SAFETY: `pipe` is a connected pipe handle owned by the caller.
        let ok = unsafe {
            if server {
                GetNamedPipeClientProcessId(pipe, &mut pid)
            } else {
                GetNamedPipeServerProcessId(pipe, &mut pid)
            }
        };
        ok != 0 && pid != 0 && process_user(pid).as_deref() == Some(sid)
    }

    fn read_message(pipe: HANDLE, limit: usize) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            let mut got = 0;
            // SAFETY: the buffer is valid for its length; the pipe is synchronous.
            let ok = unsafe { ReadFile(pipe, chunk.as_mut_ptr(), chunk.len() as u32, &mut got, null_mut()) };
            out.extend_from_slice(&chunk[..got as usize]);
            if out.len() > limit {
                return None;
            }
            if ok != 0 {
                return Some(out);
            }
            // SAFETY: no arguments.
            if unsafe { GetLastError() } != ERROR_MORE_DATA {
                return None;
            }
        }
    }

    fn write_message(pipe: HANDLE, bytes: &[u8]) -> bool {
        let mut wrote = 0;
        // SAFETY: the buffer is valid for its length; the pipe is synchronous.
        let ok = unsafe { WriteFile(pipe, bytes.as_ptr(), bytes.len() as u32, &mut wrote, null_mut()) };
        ok != 0 && wrote as usize == bytes.len()
    }

    /// Hands `payload` to the running window and waits for its answer.
    pub fn send(payload: &[u8]) -> Sent {
        send_on("", payload)
    }

    fn send_on(suffix: &str, payload: &[u8]) -> Sent {
        let Some(sid) = me() else { return Sent::NotRunning };
        let path = name(&sid, suffix);
        let mut pipe = None;
        for _ in 0..20 {
            // SAFETY: `path` is NUL-terminated; identification only, so the server cannot act as us.
            let handle = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    null(),
                    OPEN_EXISTING,
                    SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                    null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                pipe = Some(Owned(handle));
                break;
            }
            // SAFETY: no arguments; `path` is NUL-terminated.
            match unsafe { GetLastError() } {
                ERROR_FILE_NOT_FOUND => return Sent::NotRunning,
                ERROR_PIPE_BUSY => unsafe {
                    WaitNamedPipeW(path.as_ptr(), 2000);
                },
                _ => {
                    return Sent::Answered(Err("Claude Switch is running but refused the handover request.".into()));
                }
            }
        }
        let Some(pipe) = pipe else {
            return Sent::Answered(Err("Claude Switch is running but busy. Try again.".into()));
        };
        if !same_user(pipe.0, false, &sid) {
            return Sent::Answered(Err(
                "The handover channel is not owned by this Windows user. Nothing was sent.".into()
            ));
        }
        let mode = PIPE_READMODE_MESSAGE;
        // SAFETY: valid handle and mode pointer.
        if unsafe { SetNamedPipeHandleState(pipe.0, &mode, null(), null()) } == 0 || !write_message(pipe.0, payload) {
            return Sent::Answered(Err("Could not hand the request to Claude Switch.".into()));
        }
        // The window bounds its own wait; this bound only guards against a hung window.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let pipe = pipe;
            let _ = tx.send(read_message(pipe.0, MAX_REPLY));
        });
        match rx.recv_timeout(REPLY_WAIT + Duration::from_secs(5)) {
            Ok(Some(bytes)) => Sent::Answered(read_reply(&bytes)),
            Ok(None) => Sent::Answered(Err("Claude Switch closed the handover request without an answer.".into())),
            Err(_) => Sent::Answered(Err(
                "Timed out waiting for Claude Switch. The outcome is unknown; check for the tab before retrying."
                    .into(),
            )),
        }
    }

    /// Listens on a thread for the rest of the process; `answer` gets each same-user request.
    /// Does nothing if another window of this user already listens.
    pub fn serve(answer: impl Fn(Vec<u8>) -> Vec<u8> + Send + 'static) {
        serve_on("", answer);
    }

    fn serve_on(suffix: &str, answer: impl Fn(Vec<u8>) -> Vec<u8> + Send + 'static) -> bool {
        let Some(sid) = me() else { return false };
        let path = name(&sid, suffix);
        let sddl = wide(&format!("D:P(A;;GA;;;{sid})"));
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: NUL-terminated SDDL; the descriptor is freed below.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return false;
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        // SAFETY: NUL-terminated name and valid attributes; one instance, local clients only.
        let pipe = unsafe {
            CreateNamedPipeW(
                path.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                MAX_REPLY as u32,
                MAX_REQUEST as u32,
                0,
                &attributes,
            )
        };
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe { LocalFree(descriptor) };
        if pipe == INVALID_HANDLE_VALUE {
            return false;
        }
        let pipe = Owned(pipe);
        std::thread::spawn(move || {
            let pipe = pipe;
            loop {
                // SAFETY: synchronous connect on the handle this thread owns.
                let connected =
                    unsafe { ConnectNamedPipe(pipe.0, null_mut()) != 0 || GetLastError() == ERROR_PIPE_CONNECTED };
                if connected
                    && same_user(pipe.0, true, &sid)
                    && let Some(request) = read_message(pipe.0, MAX_REQUEST)
                {
                    let _ = write_message(pipe.0, &answer(request));
                    // SAFETY: valid handle.
                    unsafe { FlushFileBuffers(pipe.0) };
                }
                // SAFETY: valid handle; prepares it for the next client.
                unsafe { DisconnectNamedPipe(pipe.0) };
            }
        });
        true
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn current_user_has_a_plain_sid_for_the_pipe_name() {
            let sid = me().unwrap();
            assert!(sid.starts_with("S-1-5-"), "{sid}");
            let path = String::from_utf16(&name(&sid, "")).unwrap();
            assert!(path.starts_with(r"\\.\pipe\claude-switch-handover-S-1-"));
            assert!(!path[9..].contains(['\\', '/']));
        }

        #[test]
        fn same_user_request_gets_the_answer_and_a_second_server_is_refused() {
            let suffix = format!("-test-{}", std::process::id());
            assert_eq!(send_on(&suffix, b"x"), Sent::NotRunning);
            let answer =
                |request: Vec<u8>| reply(&if request == b"open" { Ok(Outcome::NewTab) } else { Err("no".into()) });
            assert!(serve_on(&suffix, answer));
            assert!(!serve_on(&suffix, |_| Vec::new()), "only the first window listens");
            assert_eq!(send_on(&suffix, b"open"), Sent::Answered(Ok(Outcome::NewTab)));
            assert_eq!(send_on(&suffix, b"other"), Sent::Answered(Err("no".into())));
            let long = vec![b'a'; MAX_REQUEST + 1];
            assert!(matches!(send_on(&suffix, &long), Sent::Answered(Err(_))));
            assert_eq!(
                send_on(&suffix, b"open"),
                Sent::Answered(Ok(Outcome::NewTab)),
                "still serves after a refused request"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_round_trip_and_anything_else_is_not_success() {
        assert_eq!(read_reply(&reply(&Ok(Outcome::NewTab))), Ok(Outcome::NewTab));
        assert_eq!(read_reply(&reply(&Ok(Outcome::InPlace))), Ok(Outcome::InPlace));
        assert_eq!(read_reply(&reply(&Err("Sign in first.".into()))), Err("Sign in first.".into()));
        for bad in [b"".as_slice(), b"opened\n", b"failed:", b"process started", b"\xff\xfe"] {
            assert!(read_reply(bad).is_err());
        }
        assert_eq!(read_reply(b"failed:a\x1b[2Jb"), Err("a[2Jb".into()));
        assert!(reply(&Err("x".repeat(5000))).len() <= MAX_REPLY);
    }
}
