//! Whether a pid a job file names is still that job's process, and ending its tree.
//!
//! The bridge records no start time for the worker it spawns, only `childPid`. A pid is taken for
//! the job's worker when Windows still knows it as a child of the job's server instance (`owner`;
//! Windows keeps a process's parent pid after the parent exits) and it was created while the job
//! ran: not before the job was made, and not after the bridge last saved the state that names it
//! (it saves the state right after each spawn). Anything else is a number Windows gave out again.

use super::{Job, Refused};

/// Clock skew allowed between the bridge's `Date.now()` and Windows' creation times.
pub const SLACK_MS: u64 = 2_000;

/// What Windows says about a pid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// No such process (or it has exited).
    Gone,
    /// It exists, but this user may not look at it.
    Unknown,
    /// It runs, created at this time (ms since 1970).
    Created(u64),
}

/// The job's worker, by what Windows says of `pid`: its parent and when it was created.
pub fn is_jobs(job: &Job, parent: u32, created_ms: u64) -> bool {
    job.owner == Some(parent)
        && created_ms + SLACK_MS >= job.started_ms
        && created_ms <= job.saved_ms.max(job.last_event_ms.unwrap_or(0)) + SLACK_MS
}

/// Whether a job's owner is still the server instance that made the job: the pid runs and was
/// not created after the job. One that cannot be looked at counts as there, as the bridge's own
/// check (`process.kill(pid, 0)` with EPERM) does.
pub fn owner_alive(pid: u32, started_ms: u64) -> bool {
    match lookup(pid) {
        Lookup::Gone => false,
        Lookup::Unknown => true,
        Lookup::Created(ms) => ms <= started_ms + SLACK_MS,
    }
}

const FILETIME_UNIX: u64 = 116_444_736_000_000_000;

/// The processes a round of ending a tree looks at next: those whose parent is one already
/// held (`held`, whose handles keep their pids from being given out again), not yet held, and
/// never `me` (Switch itself): it is skipped, and with it everything under it, since nothing it
/// started is ever held as a parent.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn next_round(all: &[(u32, u32)], held: &[u32], me: u32) -> Vec<(u32, u32)> {
    all.iter().filter(|(pid, parent)| *pid != me && held.contains(parent) && !held.contains(pid)).copied().collect()
}

/// Whether a process found under a parent in a snapshot is really under it: created no later
/// than the snapshot was taken (`snapshot`; a pid given out again after it would be newer) and no
/// earlier than its parent (an older one names a parent pid that has changed hands). All times
/// are FILETIME ticks.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn accept(created: u64, snapshot: u64, parent_created: u64) -> bool {
    created <= snapshot && created >= parent_created
}

/// Whether Switch may end a process: one of its own user's, and, when Switch runs elevated, one
/// that does not. The job file lies in the user's own temporary folder, so an elevated Switch
/// must not end for it a process the user could not end.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn may_end(same_user: bool, me_elevated: bool, target_elevated: bool) -> bool {
    same_user && (!me_elevated || !target_elevated)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn filetime_ms(ticks: u64) -> u64 {
    ticks.saturating_sub(FILETIME_UNIX) / 10_000
}

#[cfg(not(windows))]
pub fn lookup(_pid: u32) -> Lookup {
    Lookup::Unknown
}

#[cfg(not(windows))]
pub(super) fn end_job_tree(job: &Job) -> Result<usize, Refused> {
    Err(Refused::Unknown(job.child_pid.unwrap_or(0)))
}

#[cfg(windows)]
pub(super) use win::end_job_tree;
#[cfg(windows)]
pub use win::lookup;

#[cfg(windows)]
mod win {
    use super::{Job, Lookup, Refused, filetime_ms, is_jobs};
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::{
        EqualSid, GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess, OpenProcessToken,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, TerminateProcess,
    };

    const STILL_ACTIVE: u32 = 259;

    /// An open process handle, closed when dropped.
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle was opened by `open` and is closed exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }

    enum Open {
        Handle(Handle),
        Gone,
        Denied,
    }

    fn open(pid: u32, access: u32) -> Open {
        if pid == 0 {
            return Open::Gone;
        }
        // SAFETY: OpenProcess takes plain values and returns a handle or null; nothing is borrowed.
        let handle = unsafe { OpenProcess(access, 0, pid) };
        if !handle.is_null() {
            return Open::Handle(Handle(handle));
        }
        // SAFETY: reads the calling thread's last error code; no pointers involved.
        if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER { Open::Gone } else { Open::Denied }
    }

    /// The process's creation time (FILETIME ticks) while it runs; `Err(true)` once it has exited,
    /// `Err(false)` when it cannot be told.
    fn created(handle: &Handle) -> Result<u64, bool> {
        let mut code = 0u32;
        // SAFETY: the handle is live and `code` is a writable u32.
        if unsafe { GetExitCodeProcess(handle.0, &mut code) } == 0 {
            return Err(false);
        }
        if code != STILL_ACTIVE {
            return Err(true);
        }
        let (mut creation, mut exit, mut kernel, mut user) =
            (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        // SAFETY: the handle is live and each FILETIME is writable until return.
        let ok = unsafe { GetProcessTimes(handle.0, &mut creation, &mut exit, &mut kernel, &mut user) } != 0;
        let ticks = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        if ok && ticks != 0 { Ok(ticks) } else { Err(false) }
    }

    pub fn lookup(pid: u32) -> Lookup {
        match open(pid, PROCESS_QUERY_LIMITED_INFORMATION) {
            Open::Gone => Lookup::Gone,
            Open::Denied => Lookup::Unknown,
            Open::Handle(h) => match created(&h) {
                Ok(ticks) => Lookup::Created(filetime_ms(ticks)),
                Err(true) => Lookup::Gone,
                Err(false) => Lookup::Unknown,
            },
        }
    }

    /// Every process now: (pid, parent pid).
    fn snapshot() -> Vec<(u32, u32)> {
        // SAFETY: TH32CS_SNAPPROCESS ignores the second argument and returns a snapshot handle.
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let snap = Handle(snap);
        let mut p = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut out = Vec::new();
        // SAFETY: the snapshot is valid; `p` is initialized and its size describes its fields.
        let mut ok = unsafe { Process32FirstW(snap.0, &mut p) };
        while ok != 0 {
            out.push((p.th32ProcessID, p.th32ParentProcessID));
            // SAFETY: as above.
            ok = unsafe { Process32NextW(snap.0, &mut p) };
        }
        out
    }

    fn reason() -> String {
        std::io::Error::last_os_error().to_string()
    }

    /// The current time as FILETIME ticks (the clock process creation times are kept in).
    fn now_ticks() -> u64 {
        let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        super::FILETIME_UNIX + (since.as_nanos() / 100) as u64
    }

    /// A process's user (its `TOKEN_USER` buffer) and whether it runs elevated.
    struct Token {
        user: Vec<u64>,
        elevated: bool,
    }

    impl Token {
        fn of(process: HANDLE) -> Option<Token> {
            let mut token: HANDLE = std::ptr::null_mut();
            // SAFETY: `process` is a live handle with query rights; `token` is writable.
            if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
                return None;
            }
            let token = Handle(token);
            let mut len = 0u32;
            // SAFETY: a size query: no buffer, `len` is writable.
            unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut len) };
            if len == 0 || len > 4096 {
                return None;
            }
            let mut user = vec![0u64; (len as usize).div_ceil(8)];
            // SAFETY: `user` holds at least `len` bytes, 8-aligned, writable until return.
            if unsafe { GetTokenInformation(token.0, TokenUser, user.as_mut_ptr().cast(), len, &mut len) } == 0 {
                return None;
            }
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
            // SAFETY: `elevation` is a writable TOKEN_ELEVATION of `size` bytes.
            let ok =
                unsafe { GetTokenInformation(token.0, TokenElevation, (&raw mut elevation).cast(), size, &mut len) };
            (ok != 0).then_some(Token { user, elevated: elevation.TokenIsElevated != 0 })
        }

        fn same_user(&self, other: &Token) -> bool {
            let sid = |t: &Token| {
                // SAFETY: the buffer was filled by GetTokenInformation(TokenUser): a TOKEN_USER
                // whose SID points into the same buffer, which lives as long as `t`.
                unsafe { (*t.user.as_ptr().cast::<TOKEN_USER>()).User.Sid }
            };
            // SAFETY: both SIDs are valid for the lifetime of their tokens' buffers.
            unsafe { EqualSid(sid(self), sid(other)) != 0 }
        }

        /// Whether a process with the token `target` may be ended by this one (`super::may_end`).
        fn may_end(&self, target: &Token) -> bool {
            super::may_end(self.same_user(target), self.elevated, target.elevated)
        }
    }

    /// Ends the job's worker, then what runs under it, round by round until a round finds
    /// nothing new. The worker is ended first, so it cannot start anything while the rest is
    /// ended; every process ended is held open to the end, so the pids that name parents in later
    /// snapshots cannot change hands. A process under one is taken only when it existed at the
    /// snapshot that found it, is no older than its parent, still names that parent in a snapshot
    /// taken after it was opened, and may be ended (`super::may_end`). Switch itself, and all
    /// under it, is never ended.
    pub(in crate::bridge) fn end_job_tree(job: &Job) -> Result<usize, Refused> {
        let pid = job.child_pid.ok_or(Refused::NoProcess)?;
        let me = std::process::id();
        if pid == me {
            return Err(Refused::NotTheJobs(pid));
        }
        // SAFETY: the pseudo handle of this process needs no closing and is always valid.
        let mine = Token::of(unsafe { GetCurrentProcess() }).ok_or(Refused::Unknown(pid))?;
        let access = PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION;
        let root = match open(pid, access) {
            Open::Handle(h) => h,
            Open::Gone => return Err(Refused::Ended(pid)),
            Open::Denied => {
                return Err(match lookup(pid) {
                    Lookup::Gone => Refused::Ended(pid),
                    Lookup::Unknown => Refused::Unknown(pid),
                    Lookup::Created(_) => Refused::Denied(pid, "access is denied.".into()),
                });
            }
        };
        let root_created = match created(&root) {
            Ok(ticks) => ticks,
            Err(true) => return Err(Refused::Ended(pid)),
            Err(false) => return Err(Refused::Unknown(pid)),
        };
        // Taken while the handle is open, so the snapshot's entry for the pid is this process.
        let all = snapshot();
        let Some(&(_, parent)) = all.iter().find(|(p, _)| *p == pid) else { return Err(Refused::NotTheJobs(pid)) };
        if !is_jobs(job, parent, filetime_ms(root_created)) {
            return Err(Refused::NotTheJobs(pid));
        }
        if !Token::of(root.0).is_some_and(|t| mine.may_end(&t)) {
            return Err(Refused::Protected(pid));
        }
        // SAFETY: the handle is live and was opened with PROCESS_TERMINATE.
        if unsafe { TerminateProcess(root.0, 1) } == 0 {
            return Err(Refused::Denied(pid, reason()));
        }
        let mut held: Vec<(u32, Handle, u64)> = vec![(pid, root, root_created)];
        let mut count = 1;
        for _ in 0..64 {
            let taken = now_ticks();
            let all = snapshot();
            let pids: Vec<u32> = held.iter().map(|(p, _, _)| *p).collect();
            let mut found = Vec::new();
            for (child, parent) in super::next_round(&all, &pids, me) {
                let parent_created = held.iter().find(|(p, _, _)| *p == parent).map_or(u64::MAX, |(_, _, c)| *c);
                let Open::Handle(h) = open(child, access) else { continue };
                let Ok(ticks) = created(&h) else { continue };
                if super::accept(ticks, taken, parent_created) {
                    found.push((child, parent, h, ticks));
                }
            }
            if found.is_empty() {
                break;
            }
            let after = snapshot();
            let mut ended = false;
            for (child, parent, h, ticks) in found {
                if !after.contains(&(child, parent)) || held.len() >= 512 {
                    continue;
                }
                if !Token::of(h.0).is_some_and(|t| mine.may_end(&t)) {
                    continue;
                }
                // SAFETY: the handle is live and was opened with PROCESS_TERMINATE.
                if unsafe { TerminateProcess(h.0, 1) } != 0 {
                    count += 1;
                }
                held.push((child, h, ticks));
                ended = true;
            }
            if !ended {
                break;
            }
        }
        Ok(count)
    }
}
