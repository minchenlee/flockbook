//! Blocking NDJSON over a byte transport (port of LineSocket.swift).
//!
//! The transport is a small trait so a Windows named pipe can plug in later; only the Unix socket
//! connector exists today (behind `cfg(unix)`).

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
