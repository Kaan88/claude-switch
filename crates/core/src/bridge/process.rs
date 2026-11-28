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
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        TerminateProcess,
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

    /// Ends the job's worker and the processes under it, the deepest first. Each is held open
    /// from the check of its creation time to its end, so a pid cannot change hands in between;
    /// a process older than the one it names as parent is not under it (a stale parent pid).
    pub(in crate::bridge) fn end_job_tree(job: &Job) -> Result<usize, Refused> {
        let pid = job.child_pid.ok_or(Refused::NoProcess)?;
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
        let mut tree: Vec<(Handle, u64, u32)> = Vec::new();
        let mut queue = vec![(pid, root_created)];
        while let Some((at, at_created)) = queue.pop() {
            for &(child, _) in all.iter().filter(|(c, p)| *p == at && *c != at && *c != pid) {
                let Open::Handle(h) = open(child, access) else { continue };
                let Ok(ticks) = created(&h) else { continue };
                if ticks >= at_created && tree.len() < 512 {
                    queue.push((child, ticks));
                    tree.push((h, ticks, child));
                }
            }
        }
        let mut count = 0;
        for (h, _, _) in tree.iter().rev() {
            // SAFETY: the handle is live and was opened with PROCESS_TERMINATE.
            if unsafe { TerminateProcess(h.0, 1) } != 0 {
                count += 1;
            }
        }
        // SAFETY: as above.
        if unsafe { TerminateProcess(root.0, 1) } == 0 {
            return Err(Refused::Denied(pid, reason()));
        }
        Ok(count + 1)
    }
}
