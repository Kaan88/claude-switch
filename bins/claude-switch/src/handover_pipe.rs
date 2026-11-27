//! The two requests a second Claude Switch process may hand to the running window: open a saved
//! handover as a tab, or show why a background handover failed (a notice that acts on nothing).
//! A per-user named pipe, never a command channel.
//!
//! - The pipe's name carries the user's SID and its DACL grants only that SID; remote clients
//!   are refused and only one instance exists (`FILE_FLAG_FIRST_PIPE_INSTANCE`).
//! - The window claims the name before it restores anything, so two starting windows cannot both
//!   become the main one; requests queue until it serves.
//! - The pipe object is owned by the user's SID, and the client checks that owner before writing;
//!   the server checks that the client runs as the same user at no lower integrity level (an
//!   elevated window also labels the pipe High). The client allows identification only.
//! - A client gets [`READ_WAIT`] to write its request; repeated connect errors back off.
//! - The payload is a fixed versioned list of validated words (`compact_cli::encode_request`),
//!   at most [`MAX_REQUEST`] bytes, decoded and revalidated by the receiving window; the reply is
//!   `opened`, `continuing`, `shown` (a failure notice) or `failed:` and the app's own message,
//!   which the window shows as well.

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
    /// Showed a background handover's failure notice.
    Shown,
}

/// What the caller learned.
#[derive(Debug, PartialEq, Eq)]
pub enum Sent {
    /// No Claude Switch window of this user is listening.
    NotRunning,
    /// The running window answered; an error is one it shows in its window as well.
    Answered(Result<Outcome, String>),
    /// A window is there but gave no answer (refused, busy, closed or timed out), so it may not
    /// have shown anything.
    NoAnswer(String),
}

pub fn reply(result: &Result<Outcome, String>) -> Vec<u8> {
    match result {
        Ok(Outcome::NewTab) => b"opened".to_vec(),
        Ok(Outcome::InPlace) => b"continuing".to_vec(),
        Ok(Outcome::Shown) => b"shown".to_vec(),
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
        b"shown" => return Ok(Outcome::Shown),
        _ => {}
    }
    match std::str::from_utf8(bytes).ok().and_then(|s| s.strip_prefix("failed:")) {
        Some(e) if !e.trim().is_empty() => Err(e.chars().filter(|c| !c.is_control()).collect()),
        _ => Err("The running Claude Switch did not confirm opening a Claude tab.".into()),
    }
}

/// Whether a client may hand this window a request: never from a lower integrity level (a
/// normal process must not steer an elevated window), and only when both levels are known.
pub fn integrity_allows(client: Option<u32>, own: Option<u32>) -> bool {
    matches!((client, own), (Some(client), Some(own)) if client >= own)
}

/// The pipe's security descriptor: owned by and open only to `sid`; an elevated window also
/// labels it High with no-write-up, so lower-integrity processes cannot even open it to write.
pub fn descriptor(sid: &str, own_integrity: Option<u32>) -> String {
    let label = if own_integrity.is_some_and(|rid| rid >= HIGH_RID) { "S:(ML;;NW;;;HI)" } else { "" };
    format!("O:{sid}D:P(A;;GA;;;{sid}){label}")
}

/// `SECURITY_MANDATORY_HIGH_RID`.
const HIGH_RID: u32 = 0x3000;
/// How long a connected client may take to write its request.
const READ_WAIT: Duration = Duration::from_secs(5);

/// The result of claiming the channel at startup.
pub enum Claim {
    /// This process holds it; serve it once the window runs.
    Mine(Listener),
    /// Another process already holds the name (another Claude Switch window, normally).
    Taken,
    /// It could not be created for another reason.
    Failed(String),
}

#[cfg(windows)]
pub use win::{Listener, claim, send};

#[cfg(not(windows))]
pub struct Listener;

#[cfg(not(windows))]
impl Listener {
    pub fn serve(self, _: impl Fn(Vec<u8>) -> Vec<u8> + Send + 'static) {}
}

#[cfg(not(windows))]
pub fn claim() -> Claim {
    Claim::Mine(Listener)
}

#[cfg(not(windows))]
pub fn send(_: &[u8]) -> Sent {
    Sent::NotRunning
}

#[cfg(windows)]
mod win {
    use super::*;

    use std::ptr::{null, null_mut};
    use std::time::Instant;
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
        SE_KERNEL_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, OWNER_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER,
        TokenIntegrityLevel, TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile,
        SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
    };
    use windows_sys::Win32::System::Pipes::*;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
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

    /// A process's token, for queries only. `None` is this process.
    fn token(pid: Option<u32>) -> Option<Owned> {
        // SAFETY: every handle is checked and owned; the pseudo handle needs no closing.
        unsafe {
            let process = match pid {
                Some(pid) => Owned(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid)),
                None => Owned(null_mut()),
            };
            let handle = if pid.is_some() { process.0 } else { GetCurrentProcess() };
            if handle.is_null() {
                return None;
            }
            let mut token = null_mut();
            (OpenProcessToken(handle, TOKEN_QUERY, &mut token) != 0).then(|| Owned(token))
        }
    }

    /// One token information class, in a buffer aligned for its structure.
    fn info(token: &Owned, class: i32) -> Option<Vec<u64>> {
        // SAFETY: the buffer is sized by the first call and passed with its length.
        unsafe {
            let mut size = 0;
            GetTokenInformation(token.0, class, null_mut(), 0, &mut size);
            if size == 0 || size > 4096 {
                return None;
            }
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            (GetTokenInformation(token.0, class, buffer.as_mut_ptr().cast(), size, &mut size) != 0).then_some(buffer)
        }
    }

    fn sid_string(sid: PSID) -> Option<String> {
        // SAFETY: `sid` is a valid SID; the string is freed with LocalFree.
        unsafe {
            let mut text = null_mut();
            if ConvertSidToStringSidW(sid, &mut text) == 0 || text.is_null() {
                return None;
            }
            let len = (0..256).find(|&i| *text.add(i) == 0);
            let out = len.map(|len| String::from_utf16_lossy(std::slice::from_raw_parts(text, len)));
            LocalFree(text.cast());
            out.filter(|s| s.starts_with("S-1-") && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        }
    }

    /// The string SID of the user a token belongs to.
    fn user(token: &Owned) -> Option<String> {
        let buffer = info(token, TokenUser)?;
        // SAFETY: the buffer holds a TOKEN_USER whose SID points into it.
        sid_string(unsafe { (*(buffer.as_ptr() as *const TOKEN_USER)).User.Sid })
    }

    /// The token's integrity level (the label SID's last sub-authority, 0x2000 for Medium).
    fn integrity(token: &Owned) -> Option<u32> {
        let buffer = info(token, TokenIntegrityLevel)?;
        // SAFETY: the buffer holds a TOKEN_MANDATORY_LABEL whose SID points into it.
        unsafe {
            let sid = (*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL)).Label.Sid;
            let count = *GetSidSubAuthorityCount(sid);
            (count > 0).then(|| *GetSidSubAuthority(sid, u32::from(count) - 1))
        }
    }

    fn me() -> Option<String> {
        user(&token(None)?)
    }

    fn name(sid: &str, suffix: &str) -> Vec<u16> {
        wide(&format!(r"\\.\pipe\claude-switch-handover-{sid}{suffix}"))
    }

    /// The connected client runs as `sid` at no lower integrity than this window.
    fn client_allowed(pipe: HANDLE, sid: &str, own: Option<u32>) -> bool {
        let mut pid = 0;
        // SAFETY: `pipe` is a connected pipe handle owned by the caller.
        if unsafe { GetNamedPipeClientProcessId(pipe, &mut pid) } == 0 || pid == 0 {
            return false;
        }
        let Some(token) = token(Some(pid)) else { return false };
        user(&token).as_deref() == Some(sid) && integrity_allows(integrity(&token), own)
    }

    /// The pipe object itself is owned by `sid` (another user cannot make that so), rather than
    /// trusting the server's process id.
    fn owned_by(pipe: HANDLE, sid: &str) -> bool {
        let mut owner: PSID = null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: valid handle; the descriptor holds the owner and is freed below.
        unsafe {
            let error = GetSecurityInfo(
                pipe,
                SE_KERNEL_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor,
            );
            let ok = error == ERROR_SUCCESS && !owner.is_null() && sid_string(owner).as_deref() == Some(sid);
            if !descriptor.is_null() {
                LocalFree(descriptor);
            }
            ok
        }
    }

    /// Waits until a whole message is there to read, the client went away, or the wait ran out.
    fn message_ready(pipe: HANDLE, wait: Duration) -> bool {
        let start = Instant::now();
        loop {
            let mut avail = 0;
            // SAFETY: valid handle; no buffer is read.
            if unsafe { PeekNamedPipe(pipe, null_mut(), 0, null_mut(), &mut avail, null_mut()) } == 0 {
                return false;
            }
            if avail > 0 {
                return true;
            }
            if start.elapsed() >= wait {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
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
                    return Sent::NoAnswer("Claude Switch is running but refused the handover request.".into());
                }
            }
        }
        let Some(pipe) = pipe else {
            return Sent::NoAnswer("Claude Switch is running but busy. Try again.".into());
        };
        if !owned_by(pipe.0, &sid) {
            return Sent::NoAnswer("The handover channel is not owned by this Windows user. Nothing was sent.".into());
        }
        let mode = PIPE_READMODE_MESSAGE;
        // SAFETY: valid handle and mode pointer.
        if unsafe { SetNamedPipeHandleState(pipe.0, &mode, null(), null()) } == 0 || !write_message(pipe.0, payload) {
            return Sent::NoAnswer("Could not hand the request to Claude Switch.".into());
        }
        // The window bounds its own wait; this bound only guards against a hung window.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let pipe = pipe;
            let _ = tx.send(read_message(pipe.0, MAX_REPLY));
        });
        match rx.recv_timeout(REPLY_WAIT + Duration::from_secs(5)) {
            Ok(Some(bytes)) => Sent::Answered(read_reply(&bytes)),
            Ok(None) => Sent::NoAnswer("Claude Switch closed the handover request without an answer.".into()),
            Err(_) => Sent::NoAnswer(
                "Timed out waiting for Claude Switch. The outcome is unknown; check for the tab before retrying."
                    .into(),
            ),
        }
    }

    /// The channel, claimed before the window restores anything; requests queue in it until
    /// [`Listener::serve`] runs.
    pub struct Listener {
        pipe: Owned,
        sid: String,
        integrity: Option<u32>,
    }

    /// Claims the channel name for this process (atomically: only the first instance is made).
    pub fn claim() -> Claim {
        claim_on("")
    }

    fn claim_on(suffix: &str) -> Claim {
        let Some(own) = token(None) else { return Claim::Failed("Could not read this process's token.".into()) };
        let (Some(sid), integrity) = (user(&own), integrity(&own)) else {
            return Claim::Failed("Could not read this Windows user's SID.".into());
        };
        let path = name(&sid, suffix);
        let sddl = wide(&descriptor(&sid, integrity));
        let mut security: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: NUL-terminated SDDL; the descriptor is freed below.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut security,
                null_mut(),
            )
        } == 0
        {
            return Claim::Failed("Could not build the channel's security descriptor.".into());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security,
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
        // SAFETY: no arguments; read before LocalFree can change it.
        let error = unsafe { GetLastError() };
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe { LocalFree(security) };
        if pipe == INVALID_HANDLE_VALUE {
            return match error {
                ERROR_ACCESS_DENIED | ERROR_PIPE_BUSY => Claim::Taken,
                code => Claim::Failed(format!("Windows error {code} creating the handover channel.")),
            };
        }
        Claim::Mine(Listener { pipe: Owned(pipe), sid, integrity })
    }

    impl Listener {
        /// Serves on a thread for the rest of the process; `answer` gets each allowed request.
        pub fn serve(self, answer: impl Fn(Vec<u8>) -> Vec<u8> + Send + 'static) {
            let Listener { pipe, sid, integrity } = self;
            std::thread::spawn(move || {
                let pipe = pipe;
                let mut failures = 0u32;
                loop {
                    // SAFETY: synchronous connect on the handle this thread owns.
                    let connected =
                        unsafe { ConnectNamedPipe(pipe.0, null_mut()) != 0 || GetLastError() == ERROR_PIPE_CONNECTED };
                    if !connected {
                        // Back off rather than spin while Windows keeps refusing.
                        failures = (failures + 1).min(7);
                        std::thread::sleep(Duration::from_millis(50 << failures));
                    } else {
                        failures = 0;
                        if client_allowed(pipe.0, &sid, integrity)
                            && message_ready(pipe.0, READ_WAIT)
                            && let Some(request) = read_message(pipe.0, MAX_REQUEST)
                        {
                            let _ = write_message(pipe.0, &answer(request));
                            // SAFETY: valid handle.
                            unsafe { FlushFileBuffers(pipe.0) };
                        }
                    }
                    // SAFETY: valid handle; prepares it for the next client.
                    unsafe { DisconnectNamedPipe(pipe.0) };
                }
            });
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn current_user_has_a_plain_sid_and_a_known_integrity_level() {
            let sid = me().unwrap();
            assert!(sid.starts_with("S-1-5-"), "{sid}");
            let path = String::from_utf16(&name(&sid, "")).unwrap();
            assert!(path.starts_with(r"\\.\pipe\claude-switch-handover-S-1-"));
            assert!(!path[9..].contains(['\\', '/']));
            let level = integrity(&token(None).unwrap()).unwrap();
            assert!(level >= 0x1000, "{level:#x}");
            assert_eq!(integrity(&token(Some(std::process::id())).unwrap()), Some(level));
        }

        fn serving(suffix: &str, answer: impl Fn(Vec<u8>) -> Vec<u8> + Send + 'static) {
            let Claim::Mine(listener) = claim_on(suffix) else { panic!("the first claim gets the name") };
            listener.serve(answer);
        }

        #[test]
        fn same_user_request_gets_the_answer_and_a_second_claim_is_refused() {
            let suffix = format!("-test-{}", std::process::id());
            assert_eq!(send_on(&suffix, b"x"), Sent::NotRunning);
            let answer =
                |request: Vec<u8>| reply(&if request == b"open" { Ok(Outcome::NewTab) } else { Err("no".into()) });
            let Claim::Mine(listener) = claim_on(&suffix) else { panic!("first claim") };
            assert!(matches!(claim_on(&suffix), Claim::Taken), "only the first window holds the name");
            // A request sent before the window serves waits in the pipe.
            let early = std::thread::spawn({
                let suffix = suffix.clone();
                move || send_on(&suffix, b"open")
            });
            std::thread::sleep(Duration::from_millis(200));
            listener.serve(answer);
            assert_eq!(early.join().unwrap(), Sent::Answered(Ok(Outcome::NewTab)));
            assert_eq!(send_on(&suffix, b"other"), Sent::Answered(Err("no".into())));
            let long = vec![b'a'; MAX_REQUEST + 1];
            assert!(matches!(send_on(&suffix, &long), Sent::NoAnswer(_)));
            assert_eq!(send_on(&suffix, b"open"), Sent::Answered(Ok(Outcome::NewTab)), "still serves after a refusal");
        }

        #[test]
        fn a_client_that_never_writes_does_not_block_the_next() {
            let suffix = format!("-test-idle-{}", std::process::id());
            serving(&suffix, |_| reply(&Ok(Outcome::InPlace)));
            let sid = me().unwrap();
            let path = name(&sid, &suffix);
            // SAFETY: NUL-terminated path; the handle is owned and closed at the end.
            let idle = Owned(unsafe {
                CreateFileW(path.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, null(), OPEN_EXISTING, 0, null_mut())
            });
            assert_ne!(idle.0, INVALID_HANDLE_VALUE);
            assert!(owned_by(idle.0, &sid), "the pipe object is owned by this user");
            let start = Instant::now();
            assert_eq!(send_on(&suffix, b"x"), Sent::Answered(Ok(Outcome::InPlace)));
            assert!(start.elapsed() < READ_WAIT + Duration::from_secs(4), "{:?}", start.elapsed());
            drop(idle);
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
        assert_eq!(read_reply(&reply(&Ok(Outcome::Shown))), Ok(Outcome::Shown));
        assert_eq!(read_reply(&reply(&Err("Sign in first.".into()))), Err("Sign in first.".into()));
        for bad in [b"".as_slice(), b"opened\n", b"failed:", b"process started", b"\xff\xfe"] {
            assert!(read_reply(bad).is_err());
        }
        assert_eq!(read_reply(b"failed:a\x1b[2Jb"), Err("a[2Jb".into()));
        assert!(reply(&Err("x".repeat(5000))).len() <= MAX_REPLY);
    }

    #[test]
    fn a_lower_or_unknown_integrity_level_is_refused_and_an_elevated_pipe_is_labelled() {
        const MEDIUM: u32 = 0x2000;
        assert!(integrity_allows(Some(MEDIUM), Some(MEDIUM)));
        assert!(integrity_allows(Some(HIGH_RID), Some(MEDIUM)));
        assert!(!integrity_allows(Some(MEDIUM), Some(HIGH_RID)), "a normal process cannot steer an elevated window");
        assert!(!integrity_allows(None, Some(MEDIUM)) && !integrity_allows(Some(MEDIUM), None));
        assert_eq!(descriptor("S-1-5-21-1", Some(MEDIUM)), "O:S-1-5-21-1D:P(A;;GA;;;S-1-5-21-1)");
        assert_eq!(descriptor("S-1-5-21-1", Some(HIGH_RID)), "O:S-1-5-21-1D:P(A;;GA;;;S-1-5-21-1)S:(ML;;NW;;;HI)");
    }
}
