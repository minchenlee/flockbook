//! Blocking NDJSON over a byte transport (port of LineSocket.swift).
//!
//! The transport is a small trait: a Unix socket (behind `cfg(unix)`) or a Windows named pipe (behind
//! `cfg(windows)`, where hc listens).

use crate::error::HerdrError;
use std::io::{self, Read, Write};
use std::time::Duration;

/// A connected byte stream. `shutdown` must unblock a reader on another thread.
pub trait Transport: Read + Write + Send {
    fn shutdown(&mut self);
}

/// Opens a transport to a herdr server. `timeout` applies to each read and write; `None` blocks forever (event streams).
pub trait Connector: Send + Sync {
    fn connect(&self, timeout: Option<Duration>) -> Result<Box<dyn Transport>, HerdrError>;
}

/// Line framing over a transport: `write_line` appends `\n`; `read_line` returns one line without it.
pub struct LineSocket {
    inner: Box<dyn Transport>,
    buf: Vec<u8>,
}

impl LineSocket {
    pub fn new(inner: Box<dyn Transport>) -> Self {
        LineSocket { inner, buf: Vec::new() }
    }

    pub fn connect(c: &dyn Connector, timeout: Option<Duration>) -> Result<Self, HerdrError> {
        Ok(Self::new(c.connect(timeout)?))
    }

    pub fn write_line(&mut self, data: &[u8]) -> Result<(), HerdrError> {
        let mut out = Vec::with_capacity(data.len() + 1);
        out.extend_from_slice(data);
        out.push(b'\n');
        self.inner.write_all(&out).map_err(|e| HerdrError::Socket(format!("write: {e}")))?;
        self.inner.flush().map_err(|e| HerdrError::Socket(format!("write: {e}")))
    }

    /// Next full line, or `Closed` at EOF.
    pub fn read_line(&mut self) -> Result<Vec<u8>, HerdrError> {
        loop {
            if let Some(nl) = self.buf.iter().position(|b| *b == b'\n') {
                let line = self.buf[..nl].to_vec();
                self.buf.drain(..=nl);
                return Ok(line);
            }
            let mut chunk = [0u8; 65536];
            match self.inner.read(&mut chunk) {
                Ok(0) => return Err(HerdrError::Closed),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                // Ran past the read timeout: the request may still take effect.
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                    return Err(HerdrError::Timeout("read".into()))
                }
                Err(e) => return Err(HerdrError::Socket(format!("read: {e}"))),
            }
        }
    }

    pub fn close(&mut self) {
        self.inner.shutdown();
    }
}

#[cfg(unix)]
pub use unix::UnixConnector;

#[cfg(unix)]
mod unix {
    use super::*;
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    /// Connects to a herdr Unix socket by path.
    #[derive(Debug, Clone)]
    pub struct UnixConnector {
        pub path: String,
    }

    struct UnixTransport(UnixStream);

    impl Read for UnixTransport {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.0.read(b)
        }
    }
    impl Write for UnixTransport {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.write(b)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl Transport for UnixTransport {
        fn shutdown(&mut self) {
            let _ = self.0.shutdown(Shutdown::Both);
        }
    }

    impl Connector for UnixConnector {
        fn connect(&self, timeout: Option<Duration>) -> Result<Box<dyn Transport>, HerdrError> {
            let s = UnixStream::connect(&self.path).map_err(|e| HerdrError::Socket(format!("connect {} failed: {e}", self.path)))?;
            s.set_read_timeout(timeout).map_err(|e| HerdrError::Socket(format!("set timeout: {e}")))?;
            s.set_write_timeout(timeout).map_err(|e| HerdrError::Socket(format!("set timeout: {e}")))?;
            Ok(Box::new(UnixTransport(s)))
        }
    }
}

#[cfg(windows)]
pub use windows::{pipe_exists, PipeConnector};

/// A named pipe, opened for overlapped I/O so that each read and write can end at the timeout as on a Unix socket.
#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::time::Instant;
    use windows_sys::Win32::Foundation::{
        GetLastError, BOOL, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_NO_DATA, ERROR_OPERATION_ABORTED,
        ERROR_PIPE_BUSY, ERROR_PIPE_NOT_CONNECTED, ERROR_SEM_TIMEOUT, HANDLE, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileType, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_TYPE_PIPE, SECURITY_IDENTIFICATION,
    };
    use windows_sys::Win32::Security::{EqualSid, GetTokenInformation, IsValidSid, TokenUser, PSID, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Pipes::{GetNamedPipeServerProcessId, WaitNamedPipeW};
    use windows_sys::Win32::System::Threading::{
        CreateEventW, GetCurrentProcess, OpenProcess, OpenProcessToken, WaitForSingleObject, INFINITE,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

    /// How long a connect waits at most while every instance of the pipe is busy.
    const BUSY_WAIT: Duration = Duration::from_secs(2);

    /// Connects to a herdr-core named pipe by name, like `\\.\pipe\herdr-core-<user>-<session>.sock`.
    #[derive(Debug, Clone)]
    pub struct PipeConnector {
        pub path: String,
    }

    /// True when a server has the pipe `path` open, busy or not. Does not connect.
    pub fn pipe_exists(path: &str) -> bool {
        let name = wide(path);
        // SAFETY: `name` is NUL-terminated and outlives the call.
        // No instance at all: false at once. A free instance: true. Every instance busy: false after 1 ms, with
        // ERROR_SEM_TIMEOUT.
        unsafe { WaitNamedPipeW(name.as_ptr(), 1) != 0 || GetLastError() == ERROR_SEM_TIMEOUT }
    }

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain([0]).collect()
    }

    /// Whole milliseconds, at least 1 (0 would not wait at all) and below `INFINITE`.
    fn millis(d: Duration) -> u32 {
        d.as_millis().clamp(1, (INFINITE - 1) as u128) as u32
    }

    fn open(path: &str, timeout: Option<Duration>) -> io::Result<OwnedHandle> {
        let give_up = Instant::now() + timeout.map_or(BUSY_WAIT, |t| t.min(BUSY_WAIT));
        loop {
            // Identification only: the server may learn who connects but cannot act as this user.
            let r = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(FILE_FLAG_OVERLAPPED)
                .security_qos_flags(SECURITY_IDENTIFICATION)
                .open(path);
            match r {
                Ok(f) => return Ok(OwnedHandle::from(f)),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    let now = Instant::now();
                    if now >= give_up {
                        return Err(e);
                    }
                    // SAFETY: a NUL-terminated name that outlives the call. A false return is retried by the open.
                    unsafe { WaitNamedPipeW(wide(path).as_ptr(), millis(give_up - now)) };
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// A process's user: its token's `TOKEN_USER`, in a pointer-aligned buffer (as hc 0.1.22 reads its own,
    /// crates/herdr-core/src/platform/windows.rs:205-211).
    struct ProcessUser(Vec<usize>);

    impl ProcessUser {
        /// The user of `process`, a handle open for `PROCESS_QUERY_LIMITED_INFORMATION` (or the current-process
        /// pseudo handle).
        fn of(process: HANDLE) -> io::Result<ProcessUser> {
            let mut token: HANDLE = std::ptr::null_mut();
            // SAFETY: `token` receives a new handle on success; it is owned below.
            if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: the token handle just opened, owned by nothing else.
            let token = unsafe { OwnedHandle::from_raw_handle(token) };
            let t = token.as_raw_handle() as HANDLE;
            let mut n = 0u32;
            // SAFETY: a size query (no buffer); it fails with the size needed in `n`.
            unsafe { GetTokenInformation(t, TokenUser, std::ptr::null_mut(), 0, &mut n) };
            if n == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut buf = vec![0usize; (n as usize).div_ceil(std::mem::size_of::<usize>())];
            // SAFETY: `buf` holds at least `n` bytes, aligned for TOKEN_USER.
            if unsafe { GetTokenInformation(t, TokenUser, buf.as_mut_ptr().cast(), n, &mut n) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(ProcessUser(buf))
        }

        /// The user's SID; it points into this value.
        fn sid(&self) -> PSID {
            // SAFETY: GetTokenInformation(TokenUser) filled the buffer with a TOKEN_USER first.
            unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
        }
    }

    /// True when `a` and `b` are valid SIDs of the same account.
    fn same_user(a: PSID, b: PSID) -> bool {
        // SAFETY: each pointer is checked by IsValidSid before EqualSid reads it.
        unsafe { IsValidSid(a) != 0 && IsValidSid(b) != 0 && EqualSid(a, b) != 0 }
    }

    /// Ok when the process that serves `pipe` runs as this process's user. Anyone may create a pipe of hc's name
    /// before hc does (hc's DACL guards only the pipes hc made), so a connect checks who serves it before it writes
    /// anything. Err: why not, or why it could not tell.
    fn check_server(pipe: HANDLE) -> Result<(), String> {
        let mut pid = 0u32;
        // SAFETY: `pipe` is a client end of a named pipe this caller owns.
        if unsafe { GetNamedPipeServerProcessId(pipe, &mut pid) } == 0 {
            return Err(format!("cannot tell which process serves it: {}", io::Error::last_os_error()));
        }
        // SAFETY: plain call; a null return is checked.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return Err(format!("cannot open the process {pid} that serves it: {}", io::Error::last_os_error()));
        }
        // SAFETY: the process handle just opened, owned by nothing else.
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        let theirs = ProcessUser::of(process.as_raw_handle() as HANDLE)
            .map_err(|e| format!("cannot read the user of the process {pid} that serves it: {e}"))?;
        // SAFETY: GetCurrentProcess returns a pseudo handle that needs no closing.
        let ours = ProcessUser::of(unsafe { GetCurrentProcess() }).map_err(|e| format!("cannot read this process's user: {e}"))?;
        if same_user(theirs.sid(), ours.sid()) {
            Ok(())
        } else {
            Err(format!("the process {pid} that serves it runs as another Windows user"))
        }
    }

    struct PipeTransport {
        /// `None` once shut down.
        pipe: Option<OwnedHandle>,
        /// Manual-reset event the overlapped calls signal.
        event: OwnedHandle,
        timeout: Option<Duration>,
    }

    impl PipeTransport {
        /// One overlapped `ReadFile` or `WriteFile`, waited for up to the timeout and cancelled past it.
        fn io(&mut self, start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> BOOL) -> io::Result<usize> {
            let Some(pipe) = &self.pipe else { return Err(io::ErrorKind::NotConnected.into()) };
            let h = pipe.as_raw_handle() as HANDLE;
            // SAFETY: all-zero is a valid OVERLAPPED; the call reads from offset 0, which a pipe ignores.
            let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
            ov.hEvent = self.event.as_raw_handle() as HANDLE;
            if start(h, &mut ov) == 0 {
                // SAFETY: plain FFI calls on handles this value owns. `ov` and the caller's buffer stay alive until
                // GetOverlappedResult below has waited for the call to end, cancelled or not.
                unsafe {
                    let e = GetLastError();
                    if e != ERROR_IO_PENDING {
                        return Err(io::Error::from_raw_os_error(e as i32));
                    }
                    if WaitForSingleObject(ov.hEvent, self.timeout.map_or(INFINITE, millis)) != WAIT_OBJECT_0 {
                        CancelIoEx(h, &ov);
                    }
                }
            }
            let mut n = 0u32;
            // SAFETY: waits for the call started above to end; `ov` is the one it was given.
            if unsafe { GetOverlappedResult(h, &ov, &mut n, 1) } == 0 {
                return match unsafe { GetLastError() } {
                    ERROR_OPERATION_ABORTED => Err(io::ErrorKind::TimedOut.into()),
                    // A message-mode pipe with more of the message to come: these bytes are good.
                    ERROR_MORE_DATA => Ok(n as usize),
                    e => Err(io::Error::from_raw_os_error(e as i32)),
                };
            }
            Ok(n as usize)
        }
    }

    impl Read for PipeTransport {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            let len = b.len().min(u32::MAX as usize) as u32;
            // SAFETY: `b` is valid for `len` bytes and outlives the call (see `io`).
            let r = self.io(|h, ov| unsafe { ReadFile(h, b.as_mut_ptr(), len, std::ptr::null_mut(), ov) });
            match r {
                // The server closed its end: the end of the stream, as a Unix socket reads 0.
                Err(e) if matches!(e.raw_os_error().map(|c| c as u32), Some(ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_NO_DATA)) => Ok(0),
                r => r,
            }
        }
    }

    impl Write for PipeTransport {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            let len = b.len().min(u32::MAX as usize) as u32;
            // SAFETY: `b` is valid for `len` bytes and outlives the call (see `io`).
            self.io(|h, ov| unsafe { WriteFile(h, b.as_ptr(), len, std::ptr::null_mut(), ov) })
        }

        /// Nothing to do: a write has ended once its bytes are in the pipe. (FlushFileBuffers would wait for the
        /// server to read them.)
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Transport for PipeTransport {
        fn shutdown(&mut self) {
            self.pipe = None;
        }
    }

    impl Connector for PipeConnector {
        fn connect(&self, timeout: Option<Duration>) -> Result<Box<dyn Transport>, HerdrError> {
            let fail = |e: io::Error| HerdrError::Socket(format!("connect {} failed: {e}", self.path));
            let pipe = open(&self.path, timeout).map_err(fail)?;
            // SAFETY: a handle this function owns.
            if unsafe { GetFileType(pipe.as_raw_handle() as HANDLE) } != FILE_TYPE_PIPE {
                return Err(fail(io::Error::new(io::ErrorKind::InvalidInput, "not a named pipe")));
            }
            // Before the first byte goes out: a pipe of another user's process is not this user's herdr-core.
            if let Err(why) = check_server(pipe.as_raw_handle() as HANDLE) {
                return Err(HerdrError::Socket(format!("refused {}: {why}; only this user's herdr-core is trusted", self.path)));
            }
            // SAFETY: no security attributes and no name; a null return is checked.
            let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
            if event.is_null() {
                return Err(fail(io::Error::last_os_error()));
            }
            // SAFETY: a valid event handle that nothing else owns.
            let event = unsafe { OwnedHandle::from_raw_handle(event) };
            Ok(Box::new(PipeTransport { pipe: Some(pipe), event, timeout }))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::Security::{CreateWellKnownSid, WinWorldSid, SECURITY_MAX_SID_SIZE};

        /// The "Everyone" SID, which no process runs as.
        fn everyone() -> Vec<u32> {
            let mut buf = vec![0u32; (SECURITY_MAX_SID_SIZE as usize).div_ceil(4)];
            let mut n = SECURITY_MAX_SID_SIZE;
            // SAFETY: `buf` holds `n` bytes.
            let ok = unsafe { CreateWellKnownSid(WinWorldSid, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut n) };
            assert!(ok != 0, "CreateWellKnownSid: {}", io::Error::last_os_error());
            buf
        }

        #[test]
        fn same_user_compares_sids() {
            let (mut a, mut b) = (everyone(), everyone());
            let me = ProcessUser::of(unsafe { GetCurrentProcess() }).unwrap();
            let me_again = ProcessUser::of(unsafe { GetCurrentProcess() }).unwrap();
            assert!(same_user(a.as_mut_ptr().cast(), b.as_mut_ptr().cast()));
            assert!(same_user(me.sid(), me_again.sid()));
            assert!(!same_user(me.sid(), a.as_mut_ptr().cast()), "another account's SID");
            let mut zero = vec![0u32; 17];
            assert!(!same_user(me.sid(), zero.as_mut_ptr().cast()), "not a SID");
        }

        #[test]
        fn a_pipe_this_user_serves_passes_the_server_check() {
            let core = crate::hc::client::fake_core::start(|_, _| Some(serde_json::json!({})));
            let pipe = open(&core.path, Some(Duration::from_secs(2))).unwrap();
            assert_eq!(check_server(pipe.as_raw_handle() as HANDLE), Ok(()));
            drop(pipe);
            let mut s = LineSocket::connect(&PipeConnector { path: core.path.clone() }, Some(Duration::from_secs(2))).unwrap();
            s.write_line(br#"{"jsonrpc":"2.0","id":1,"method":"server.status","params":{}}"#).unwrap();
            assert!(!s.read_line().unwrap().is_empty());
        }
    }
}
