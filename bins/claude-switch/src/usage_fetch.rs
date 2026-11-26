//! WinHTTP transport. No configurable destination, proxy, redirects or certificate bypass.

use claude_switch_core::Roots;
use claude_switch_core::usage::{Error, Reading};
#[cfg(windows)]
use claude_switch_core::usage::{Token, now_ms};
use mdshared_app::fmt::Civil;
use std::path::Path;

pub fn local_time(secs: i64) -> Civil {
    #[cfg(windows)]
    if let Some(local) = win::local_time(secs) {
        return local;
    }
    Civil::from_unix(secs, claude_switch_core::clock::local_offset_minutes())
}

#[cfg(windows)]
pub fn fetch(roots: &Roots, dir: &Path, enabled: impl Fn() -> bool) -> Result<Reading, Error> {
    win::production(roots, dir, enabled, reading)
}

#[cfg(windows)]
fn reading(token: &Token, body: &[u8]) -> Result<Reading, Error> {
    token.reading(body, now_ms())
}

#[cfg(windows)]
pub fn fetch_shape(roots: &Roots, dir: &Path, labels: bool) -> Result<serde_json::Value, Error> {
    win::production(roots, dir, || true, |token, body| token.shape(body, labels))
}

#[cfg(not(windows))]
pub fn fetch_shape(_roots: &Roots, _dir: &Path, _labels: bool) -> Result<serde_json::Value, Error> {
    Err(Error::Network)
}

/// The GUI subsystem may start without standard handles. Keep inherited redirects untouched.
pub fn attach_diagnostic_console() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Console::{
            ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
        };
        // SAFETY: these calls take only Win32 constants and do not dereference caller memory.
        unsafe {
            let out = GetStdHandle(STD_OUTPUT_HANDLE);
            if out.is_null() || out == INVALID_HANDLE_VALUE {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }
        }
    }
}

#[cfg(not(windows))]
pub fn fetch(_dir: &Path, _enabled: impl Fn() -> bool) -> Result<Reading, Error> {
    Err(Error::Network)
}

#[cfg(windows)]
mod win {
    use super::*;
    use claude_switch_core::usage::read_token;
    use std::cell::UnsafeCell;
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::Networking::WinHttp::*;
    use windows_sys::Win32::System::Time::SystemTimeToTzSpecificLocalTimeEx;
    use windows_sys::core::w;

    const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
    const AGENT: &str = concat!("claude-switch/", env!("CARGO_PKG_VERSION"));
    const TIMEOUT: Duration = Duration::from_secs(10);
    const CHUNK: usize = 8192;
    const MAX_BODY: usize = 1024 * 1024;
    // SDK macro, not emitted by windows-sys. WinHttpCloseHandle requires the full mask.
    const WINHTTP_CALLBACK_FLAG_HANDLES: u32 =
        WINHTTP_CALLBACK_STATUS_HANDLE_CREATED | WINHTTP_CALLBACK_STATUS_HANDLE_CLOSING;
    const CALLBACK_FLAGS: u32 = WINHTTP_CALLBACK_FLAG_SENDREQUEST_COMPLETE
        | WINHTTP_CALLBACK_FLAG_HEADERS_AVAILABLE
        | WINHTTP_CALLBACK_FLAG_READ_COMPLETE
        | WINHTTP_CALLBACK_FLAG_REQUEST_ERROR
        | WINHTTP_CALLBACK_FLAG_HANDLES;

    pub(super) fn local_time(secs: i64) -> Option<Civil> {
        let utc = Civil::from_unix(secs, 0);
        let input = SYSTEMTIME {
            wYear: u16::try_from(utc.year).ok()?,
            wMonth: utc.month.into(),
            wDay: utc.day.into(),
            wHour: utc.hour.into(),
            wMinute: utc.minute.into(),
            wSecond: utc.second.into(),
            ..Default::default()
        };
        let mut output = SYSTEMTIME::default();
        // SAFETY: null selects the system zone; input and output are distinct live SYSTEMTIME values.
        if unsafe { SystemTimeToTzSpecificLocalTimeEx(null(), &input, &mut output) } == 0 {
            return None;
        }
        Some(Civil {
            year: output.wYear.into(),
            month: output.wMonth as u8,
            day: output.wDay as u8,
            hour: output.wHour as u8,
            minute: output.wMinute as u8,
            second: output.wSecond as u8,
            weekday: ((output.wDayOfWeek + 6) % 7) as u8,
        })
    }

    struct Handle(*mut c_void);

    impl Handle {
        fn new(raw: *mut c_void) -> Result<Self, Error> {
            if raw.is_null() { Err(Error::Network) } else { Ok(Self(raw)) }
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: this handle is owned here and closed exactly once; child handles drop first.
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }

    enum Event {
        Sent,
        Headers,
        Chunk(Vec<u8>),
        Failed,
    }

    struct Context {
        tx: Sender<Event>,
        buffer: UnsafeCell<[u8; CHUNK]>,
        #[cfg(test)]
        dropped: Option<Sender<()>>,
    }

    #[cfg(test)]
    thread_local! {
        static ON_DROP: std::cell::RefCell<Option<Sender<()>>> = const { std::cell::RefCell::new(None) };
        static ON_READ: std::cell::RefCell<Option<Sender<()>>> = const { std::cell::RefCell::new(None) };
    }

    #[cfg(test)]
    impl Drop for Context {
        fn drop(&mut self) {
            if let Some(tx) = &self.dropped {
                let _ = tx.send(());
            }
        }
    }

    unsafe extern "system" fn status(_: *mut c_void, context: usize, status: u32, info: *mut c_void, len: u32) {
        let ptr = context as *mut Context;
        if ptr.is_null() {
            return;
        }
        if status == WINHTTP_CALLBACK_STATUS_HANDLE_CLOSING {
            // SAFETY: WinHTTP promises this is the last callback after all pending operations,
            // even on cancellation. The single registration owns the Box from install().
            drop(unsafe { Box::from_raw(ptr) });
            return;
        }
        // SAFETY: this callback is sequenced before HANDLE_CLOSING. Only the sender is borrowed;
        // the OS may write the separately stored UnsafeCell buffer for the active read.
        let sender = unsafe { &(*ptr).tx };
        let event = match status {
            WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE => Event::Sent,
            WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE => Event::Headers,
            WINHTTP_CALLBACK_STATUS_REQUEST_ERROR => Event::Failed,
            WINHTTP_CALLBACK_STATUS_READ_COMPLETE if len == 0 => Event::Chunk(Vec::new()),
            WINHTTP_CALLBACK_STATUS_READ_COMPLETE if len as usize <= CHUNK && !info.is_null() => {
                // SAFETY: READ_COMPLETE supplies the initialized part of our live buffer, and
                // the worker won't issue the next read until this copy has been sent.
                Event::Chunk(unsafe { std::slice::from_raw_parts(info.cast::<u8>(), len as usize) }.to_vec())
            }
            WINHTTP_CALLBACK_STATUS_READ_COMPLETE => Event::Failed,
            _ => return,
        };
        let _ = sender.send(event);
    }

    struct Call {
        id: usize,
        buffer: *mut c_void,
        rx: Receiver<Event>,
        deadline: Instant,
    }

    impl Call {
        fn install(handle: &Handle, deadline: Instant) -> Result<Self, Error> {
            let (tx, rx) = mpsc::channel();
            let context = Box::new(Context {
                tx,
                buffer: UnsafeCell::new([0; CHUNK]),
                #[cfg(test)]
                dropped: ON_DROP.with(|cell| cell.borrow_mut().take()),
            });
            let buffer = context.buffer.get().cast();
            let ptr = Box::into_raw(context);
            let id = ptr as usize;
            // SAFETY: SetOption copies the usize value; the request is idle and has no callback yet.
            let set = unsafe {
                WinHttpSetOption(
                    handle.0,
                    WINHTTP_OPTION_CONTEXT_VALUE,
                    (&raw const id).cast(),
                    size_of::<usize>() as u32,
                )
            };
            if set == 0 {
                // SAFETY: this Box is still ours and no callback has been installed.
                drop(unsafe { Box::from_raw(ptr) });
                return Err(Error::Network);
            }
            // SAFETY: the callback uses the documented system ABI and the context outlives the request.
            let previous = unsafe { WinHttpSetStatusCallback(handle.0, Some(status), CALLBACK_FLAGS, 0) };
            if previous.is_some_and(|f| f as usize == usize::MAX) {
                // SAFETY: registration failed, so WinHTTP cannot dereference this context.
                drop(unsafe { Box::from_raw(ptr) });
                return Err(Error::Network);
            }
            Ok(Self { id, buffer, rx, deadline })
        }

        fn next(&self, enabled: &impl Fn() -> bool) -> Result<Event, Error> {
            while enabled() {
                let wait = self.deadline.checked_duration_since(Instant::now()).ok_or(Error::Network)?;
                match self.rx.recv_timeout(wait.min(Duration::from_millis(100))) {
                    Ok(event) if Instant::now() < self.deadline && enabled() && !matches!(event, Event::Failed) => {
                        return Ok(event);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => return Err(Error::Network),
                }
            }
            Err(Error::Network)
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    struct Endpoint<'a> {
        host: &'a str,
        port: u16,
        path: &'a str,
        flags: u32,
    }

    fn production_endpoint() -> Option<Endpoint<'static>> {
        let (host, path) = USAGE_URL.strip_prefix("https://")?.split_once('/')?;
        Some(Endpoint { host, port: 443, path, flags: WINHTTP_FLAG_SECURE })
    }

    pub(super) fn production<T>(
        roots: &Roots,
        dir: &Path,
        enabled: impl Fn() -> bool,
        decode: impl FnOnce(&Token, &[u8]) -> Result<T, Error>,
    ) -> Result<T, Error> {
        request(dir, production_endpoint().ok_or(Error::Network)?, TIMEOUT, enabled, decode, Some(roots))
    }

    #[cfg(test)]
    fn loopback(dir: &Path, base: &str, timeout: Duration) -> Result<Reading, Error> {
        loopback_with(dir, base, timeout, reading)
    }

    #[cfg(test)]
    fn loopback_with<T>(
        dir: &Path,
        base: &str,
        timeout: Duration,
        decode: impl FnOnce(&Token, &[u8]) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let port = base.strip_prefix("http://127.0.0.1:").and_then(|p| p.parse::<u16>().ok()).ok_or(Error::Network)?;
        let endpoint = Endpoint { host: "127.0.0.1", port, path: "api/oauth/usage", flags: 0 };
        request(dir, endpoint, timeout, || true, decode, None)
    }

    fn request<T>(
        dir: &Path,
        endpoint: Endpoint<'_>,
        timeout: Duration,
        enabled: impl Fn() -> bool,
        decode: impl FnOnce(&Token, &[u8]) -> Result<T, Error>,
        roots: Option<&Roots>,
    ) -> Result<T, Error> {
        if !enabled() {
            return Err(Error::Network);
        }
        let deadline = Instant::now() + timeout;
        let agent = wide(AGENT);
        let host = wide(endpoint.host);
        let path = wide(&format!("/{}", endpoint.path));
        // SAFETY: the agent string is NUL-terminated and valid for this call; no proxy strings.
        let session = Handle::new(unsafe {
            WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_NO_PROXY, null(), null(), WINHTTP_FLAG_ASYNC)
        })?;
        // SAFETY: a live WinHTTP session and a NUL-terminated host; this does not connect yet.
        let connection = Handle::new(unsafe { WinHttpConnect(session.0, host.as_ptr(), endpoint.port, 0) })?;
        // SAFETY: live connection and NUL-terminated verb/path. Optional strings and arrays are null.
        let request = Handle::new(unsafe {
            WinHttpOpenRequest(connection.0, w!("GET"), path.as_ptr(), null(), null(), null(), endpoint.flags)
        })?;
        let disabled = WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION;
        // SAFETY: `disabled` is a DWORD copied by WinHTTP; fail closed if these options cannot apply.
        if unsafe { WinHttpSetOption(request.0, WINHTTP_OPTION_DISABLE_FEATURE, (&raw const disabled).cast(), 4) } == 0
        {
            return Err(Error::Network);
        }
        let call = Call::install(&request, deadline)?;
        if !enabled() {
            return Err(Error::Network);
        }
        let permit =
            || roots.map(|r| claude_switch_core::config::usage_permit(r, dir).ok_or(Error::Network)).transpose();
        let token = {
            let _permit = permit()?;
            if !enabled() {
                return Err(Error::Network);
            }
            read_token(dir, now_ms())?
        };
        const PREFIX: &str = "anthropic-beta: oauth-2025-04-20\r\nAccept: application/json\r\nAuthorization: Bearer ";
        let bearer = token.bearer(now_ms())?;
        let mut headers = Vec::with_capacity(PREFIX.len() + bearer.len() + 2);
        headers.extend(PREFIX.encode_utf16());
        headers.extend(bearer.encode_utf16());
        headers.extend([13, 10]);
        // SAFETY: supplied UTF-16 units are valid for their explicit length. This call copies the headers.
        let added = unsafe {
            WinHttpAddRequestHeaders(request.0, headers.as_ptr(), headers.len() as u32, WINHTTP_ADDREQ_FLAG_ADD)
        };
        headers.fill(0);
        std::hint::black_box(&headers);
        if added == 0 || !enabled() || Instant::now() >= deadline {
            return Err(Error::Network);
        }
        token.bearer(now_ms())?;
        // SAFETY: live request, no temporary headers or body, and the context belongs to the close callback.
        let sent = {
            let _permit = permit()?;
            if !enabled() {
                return Err(Error::Network);
            }
            unsafe { WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, call.id) }
        };
        if sent == 0 || !matches!(call.next(&enabled)?, Event::Sent) {
            return Err(Error::Network);
        }
        // SAFETY: send has completed; the reserved parameter must be null.
        if unsafe { WinHttpReceiveResponse(request.0, null_mut()) } == 0
            || !matches!(call.next(&enabled)?, Event::Headers)
        {
            return Err(Error::Network);
        }
        let mut code = 0u32;
        let mut size = 4;
        // SAFETY: numeric query writes a DWORD into `code`; input/output length is a live DWORD.
        if unsafe {
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&raw mut code).cast(),
                &mut size,
                null_mut(),
            )
        } == 0
        {
            return Err(Error::Network);
        }
        if !(200..300).contains(&code) {
            return Err(Error::Http(u16::try_from(code).map_err(|_| Error::Network)?));
        }
        let mut body = Vec::new();
        loop {
            // SAFETY: native code owns the use of this CHUNK-sized allocation until READ_COMPLETE
            // or HANDLE_CLOSING. The async bytes-read parameter must be null.
            if unsafe { WinHttpReadData(request.0, call.buffer, CHUNK as u32, null_mut()) } == 0 {
                return Err(Error::Network);
            }
            #[cfg(test)]
            ON_READ.with(|cell| {
                if let Some(tx) = cell.borrow_mut().take() {
                    let _ = tx.send(());
                }
            });
            let Event::Chunk(chunk) = call.next(&enabled)? else { return Err(Error::Network) };
            if chunk.is_empty() {
                break;
            }
            if body.len() + chunk.len() > MAX_BODY {
                return Err(Error::NotUnderstood);
            }
            body.extend(chunk);
        }
        decode(&token, &body)
    }

    #[cfg(test)]
    mod tests;
}
