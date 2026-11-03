//! Swift: `LineSocket.swift`, the blocking NDJSON connection, as the plain-mode calls use it (one per call).

use crate::models::HerdrError;
use std::io::{Read, Write};
use std::time::Duration;

#[cfg(unix)]
type Stream = std::os::unix::net::UnixStream;

pub struct LineSocket {
    #[cfg(unix)]
    stream: Stream,
    buffer: Vec<u8>,
}

fn io_err(what: &str, e: std::io::Error) -> HerdrError {
    match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => HerdrError::Timeout(what.into()),
        _ => HerdrError::Socket(format!("{what}: {e}")),
    }
}

impl LineSocket {
    /// `timeout`: seconds for each read and write.
    #[cfg(unix)]
    pub fn connect(path: &str, timeout: Duration) -> Result<LineSocket, HerdrError> {
        let stream = Stream::connect(path).map_err(|e| HerdrError::Socket(format!("connect {path} failed: {e}")))?;
        stream.set_read_timeout(Some(timeout)).map_err(|e| io_err("timeout", e))?;
        stream.set_write_timeout(Some(timeout)).map_err(|e| io_err("timeout", e))?;
        Ok(LineSocket { stream, buffer: vec![] })
    }

    /// TODO(windows): herdr-core's endpoint on Windows is unknown (see `HcSession::resolve`). Fails closed.
    #[cfg(not(unix))]
    pub fn connect(path: &str, _timeout: Duration) -> Result<LineSocket, HerdrError> {
        Err(HerdrError::Socket(format!("connect {path} failed: herdr-core sockets are not supported on this OS yet")))
    }

    pub fn write_line(&mut self, data: &[u8]) -> Result<(), HerdrError> {
        let mut out = data.to_vec();
        out.push(b'\n');
        #[cfg(unix)]
        {
            // A write timeout is a socket error, as in Swift (only reads report `timeout`).
            self.stream.write_all(&out).map_err(|e| HerdrError::Socket(format!("write: {e}")))?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = out;
            Err(HerdrError::Closed)
        }
    }

    /// The next full line, or `Closed` at EOF, `Timeout` when a read ran past the timeout.
    pub fn read_line(&mut self) -> Result<Vec<u8>, HerdrError> {
        loop {
            if let Some(nl) = self.buffer.iter().position(|&b| b == b'\n') {
                let line = self.buffer[..nl].to_vec();
                self.buffer.drain(..=nl);
                return Ok(line);
            }
            let mut chunk = vec![0u8; 65536];
            #[cfg(unix)]
            let n = loop {
                match self.stream.read(&mut chunk) {
                    Ok(n) => break n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(io_err("read", e)),
                }
            };
            #[cfg(not(unix))]
            let n = 0;
            if n == 0 {
                return Err(HerdrError::Closed);
            }
            self.buffer.extend_from_slice(&chunk[..n]);
        }
    }
}
