//! The local channel of the Discord application installed on this PC, and nothing else: on Windows the named pipe
//! `\\.\pipe\discord-ipc-<n>` (n = 0 to 9, one per running Discord: stable, PTB, Canary...). No network access, no
//! Discord library: the protocol is the documented IPC one (the `discord-rpc` library's `rpc_connection.h`, Discord's
//! RPC documentation): a frame is `<opcode u32 LE><length u32 LE><length bytes of JSON>`, with Handshake 0, Frame 1,
//! Close 2, Ping 3 and Pong 4.
//!
//! What this file does on its own: open a pipe by name (with an identification-only security level, so the program
//! behind the pipe cannot act as this user), read what is there without ever waiting, write a frame, list the pipe
//! names that exist (a directory listing: no connection), and the machine-wide "one presence at a time" lock.

use std::io;

pub const OP_HANDSHAKE: u32 = 0;
pub const OP_FRAME: u32 = 1;
pub const OP_CLOSE: u32 = 2;
pub const OP_PING: u32 = 3;
pub const OP_PONG: u32 = 4;

/// The largest frame accepted from the other side. Discord's answers are a few KiB (the READY event carries the
/// user's public profile and the client's configuration); anything bigger is not Discord and ends the connection.
pub const MAX_FRAME: usize = 64 * 1024;

/// The pipe names Discord listens on, `<prefix>0` to `<prefix>9`.
pub const PIPE_PREFIX: &str = "discord-ipc-";
pub const PIPES: u32 = 10;

/// One frame as written on the wire.
pub fn encode(op: u32, json: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + json.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(json.as_bytes());
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub op: u32,
    /// The JSON payload; `Null` when it does not parse (a Ping's payload is echoed as bytes, see `raw`).
    pub body: serde_json::Value,
    pub raw: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The header announces more than [`MAX_FRAME`] bytes: the stream is not Discord's, or it is out of step.
    TooLong(u32),
}

/// Bytes in, whole frames out, whatever the reads cut them into.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        if self.buf.len() < 8 {
            return Ok(None);
        }
        let op = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
        let len = u32::from_le_bytes([self.buf[4], self.buf[5], self.buf[6], self.buf[7]]);
        if len as usize > MAX_FRAME {
            return Err(FrameError::TooLong(len));
        }
        let end = 8 + len as usize;
        if self.buf.len() < end {
            return Ok(None);
        }
        let raw: Vec<u8> = self.buf[8..end].to_vec();
        self.buf.drain(..end);
        let body = serde_json::from_slice(&raw).unwrap_or(serde_json::Value::Null);
        Ok(Some(Frame { op, body, raw }))
    }
}

/// A connection to one pipe. Reads never wait: the presence thread polls.
pub trait Conn: Send {
    /// What is there now: `Ok(0)` nothing yet, `Ok(n)` bytes read, `Err` the other side closed or the pipe broke.
    fn read_now(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    fn write_all(&mut self, data: &[u8]) -> io::Result<()>;
}

/// Opens `\\.\pipe\<name>`. `None` when nothing listens there or every instance of it is busy.
pub fn connect(name: &str) -> Option<Box<dyn Conn>> {
    sys::connect(name)
}

/// Which of `<prefix>0..9` exist, from a listing of the pipe namespace (nothing is opened, nothing is sent).
pub fn listed(prefix: &str) -> Vec<u32> {
    sys::listed(prefix)
}

/// "One presence at a time on this PC": a named mutex of the Windows session, held by the thread that shows the
/// presence. A second launcher (or the game started alone, `runtime/discord_rpc.c`) finds it taken and stays quiet; a
/// holder that dies leaves it abandoned, which the next one takes. Dropped on the thread that took it.
pub struct OwnerLock {
    #[allow(dead_code)]
    inner: sys::Lock,
    /// A Windows mutex belongs to the thread that took it: the lock never moves to another thread.
    _not_send: std::marker::PhantomData<*const ()>,
}

impl OwnerLock {
    /// `None` when another holder has it. A lock that cannot be created at all lets the presence go ahead (the lock
    /// only avoids two owners; it must not stop the feature).
    pub fn try_take(name: &str) -> Option<OwnerLock> {
        sys::Lock::try_take(name).map(|inner| OwnerLock { inner, _not_send: std::marker::PhantomData })
    }
}

#[cfg(windows)]
mod sys {
    use super::{Conn, PIPES};
    use std::fs::{File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;
    use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

    /// `SECURITY_IDENTIFICATION` (winbase.h): the server of the pipe may learn who connects, never act as them.
    const SECURITY_IDENTIFICATION: u32 = 1 << 16;

    struct Pipe {
        file: File,
    }

    impl Conn for Pipe {
        fn read_now(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let mut avail: u32 = 0;
            // SAFETY: a query of the bytes waiting in a pipe handle this struct owns; no buffer is passed.
            let ok = unsafe {
                PeekNamedPipe(self.file.as_raw_handle() as HANDLE, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut())
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if avail == 0 || buf.is_empty() {
                return Ok(0);
            }
            let want = (avail as usize).min(buf.len());
            // Never blocks: at least `want` bytes are waiting.
            match self.file.read(&mut buf[..want])? {
                0 => Err(io::Error::new(io::ErrorKind::BrokenPipe, "pipe closed")),
                n => Ok(n),
            }
        }

        fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
            self.file.write_all(data)?;
            self.file.flush()
        }
    }

    pub fn connect(name: &str) -> Option<Box<dyn Conn>> {
        if name.is_empty() || name.contains(['\\', '/']) {
            return None;
        }
        let path = format!(r"\\.\pipe\{name}");
        let file = OpenOptions::new().read(true).write(true).security_qos_flags(SECURITY_IDENTIFICATION).open(path).ok()?;
        Some(Box::new(Pipe { file }))
    }

    pub fn listed(prefix: &str) -> Vec<u32> {
        let Ok(dir) = std::fs::read_dir(r"\\.\pipe\") else { return Vec::new() };
        let mut found: Vec<u32> = dir
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_prefix(prefix)).and_then(|n| n.parse::<u32>().ok()))
            .filter(|n| *n < PIPES)
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    pub struct Lock(HANDLE);

    impl Lock {
        pub fn try_take(name: &str) -> Option<Lock> {
            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: a named mutex of the session, not owned at creation; the handle is checked and closed below.
            let h = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
            if h.is_null() {
                return Some(Lock(std::ptr::null_mut()));
            }
            // SAFETY: a zero-timeout wait on the handle just created.
            match unsafe { WaitForSingleObject(h, 0) } {
                WAIT_OBJECT_0 | WAIT_ABANDONED => Some(Lock(h)),
                _ => {
                    // SAFETY: the handle created above, not owned.
                    unsafe { CloseHandle(h) };
                    None
                }
            }
        }
    }

    impl Drop for Lock {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the mutex this thread owns (OwnerLock is not Send), then its handle.
                unsafe {
                    ReleaseMutex(self.0);
                    CloseHandle(self.0);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod sys {
    //! The launcher ships for Windows; elsewhere there is no pipe to talk to and the presence reports Discord absent.
    use super::Conn;

    pub fn connect(_name: &str) -> Option<Box<dyn Conn>> {
        None
    }

    pub fn listed(_prefix: &str) -> Vec<u32> {
        Vec::new()
    }

    pub struct Lock;

    impl Lock {
        pub fn try_take(_name: &str) -> Option<Lock> {
            Some(Lock)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_whatever_the_reads_cut_them_into() {
        let mut wire = encode(OP_FRAME, r#"{"cmd":"DISPATCH","evt":"READY"}"#);
        wire.extend(encode(OP_PING, "{}"));
        let mut d = Decoder::default();
        let mut got = Vec::new();
        for chunk in wire.chunks(3) {
            d.push(chunk);
            while let Some(f) = d.next_frame().unwrap() {
                got.push(f);
            }
        }
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].op, OP_FRAME);
        assert_eq!(got[0].body["evt"], "READY");
        assert_eq!((got[1].op, got[1].raw.as_slice()), (OP_PING, b"{}".as_slice()));
    }

    #[test]
    fn an_oversized_frame_is_refused_before_it_is_read() {
        let mut d = Decoder::default();
        let mut head = OP_FRAME.to_le_bytes().to_vec();
        head.extend(((MAX_FRAME + 1) as u32).to_le_bytes());
        d.push(&head);
        assert_eq!(d.next_frame(), Err(FrameError::TooLong(MAX_FRAME as u32 + 1)));
        // a payload that is not JSON still comes out as a frame (its body Null)
        let mut d = Decoder::default();
        d.push(&encode(OP_CLOSE, "not json"));
        let f = d.next_frame().unwrap().unwrap();
        assert_eq!((f.op, f.body.clone()), (OP_CLOSE, serde_json::Value::Null));
    }

    #[test]
    fn a_pipe_name_with_a_path_in_it_is_never_opened() {
        assert!(connect(r"..\discord-ipc-0").is_none());
        assert!(connect("a/b").is_none());
        assert!(connect("").is_none());
    }
}
