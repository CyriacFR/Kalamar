//! A stand-in for the Discord application, for the tests: one named pipe of the test's own name (never
//! `discord-ipc-*`, so a Discord running on the machine is never reached), served in non-blocking mode by one thread.
//! It answers the handshake (READY, a Close with code 4000, or nothing), records every frame it receives, answers each
//! `SET_ACTIVITY` (or refuses the next one), and can be "closed" (the pipe disappears, as when Discord quits) and
//! "opened" again.

use super::ipc::{self, Decoder, OP_CLOSE, OP_FRAME, OP_HANDSHAKE, OP_PING, OP_PONG};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{PIPE_ACCESS_DUPLEX, ReadFile, WriteFile};
use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_NOWAIT, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    Ready,
    RefuseClientId,
    Silent,
}

#[derive(Default)]
struct Shared {
    frames: Vec<(u32, Vec<u8>)>,
    raw: Vec<u8>,
    connections: usize,
    connected: bool,
    closed: bool,
    stop: bool,
    outgoing: Vec<Vec<u8>>,
    reject_next: bool,
}

pub struct Fake {
    shared: Arc<Mutex<Shared>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Fake {
    pub fn start(name: &str, behaviour: Behaviour) -> Fake {
        assert!(!name.starts_with(ipc::PIPE_PREFIX), "the stand-in never takes Discord's own pipe names");
        let shared = Arc::new(Mutex::new(Shared::default()));
        let s2 = shared.clone();
        let path: Vec<u16> = format!(r"\\.\pipe\{name}").encode_utf16().chain(std::iter::once(0)).collect();
        let thread = std::thread::spawn(move || serve(&path, behaviour, &s2));
        // the pipe exists once the thread created it
        for _ in 0..200 {
            if ipc::listed(name.trim_end_matches(|c: char| c.is_ascii_digit())).iter().any(|n| name.ends_with(&n.to_string())) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Fake { shared, thread: Some(thread) }
    }

    fn with<T>(&self, f: impl FnOnce(&mut Shared) -> T) -> T {
        f(&mut self.shared.lock().unwrap())
    }

    fn bodies(&self, op: u32) -> Vec<Value> {
        self.with(|s| s.frames.iter().filter(|(o, _)| *o == op).map(|(_, b)| serde_json::from_slice(b).unwrap_or(Value::Null)).collect())
    }

    pub fn handshakes(&self) -> Vec<Value> {
        self.bodies(OP_HANDSHAKE)
    }

    /// Every `SET_ACTIVITY` received, clears included.
    pub fn commands(&self) -> Vec<Value> {
        self.bodies(OP_FRAME).into_iter().filter(|b| b["cmd"] == "SET_ACTIVITY").collect()
    }

    /// The activities shown (clears left out).
    pub fn activities(&self) -> Vec<Value> {
        self.commands().into_iter().map(|c| c["args"]["activity"].clone()).filter(|a| !a.is_null()).collect()
    }

    pub fn pongs(&self) -> Vec<Vec<u8>> {
        self.with(|s| s.frames.iter().filter(|(o, _)| *o == OP_PONG).map(|(_, b)| b.clone()).collect())
    }

    /// Every byte the launcher wrote.
    pub fn raw(&self) -> Vec<u8> {
        self.with(|s| s.raw.clone())
    }

    pub fn connections(&self) -> usize {
        self.with(|s| s.connections)
    }

    pub fn connected(&self) -> bool {
        self.with(|s| s.connected)
    }

    /// Discord quits: the pipe goes.
    pub fn close(&self) {
        self.with(|s| s.closed = true);
        std::thread::sleep(Duration::from_millis(50));
    }

    /// Discord starts again.
    pub fn open(&self) {
        self.with(|s| s.closed = false);
    }

    pub fn ping(&self, payload: &[u8]) {
        let mut f = OP_PING.to_le_bytes().to_vec();
        f.extend((payload.len() as u32).to_le_bytes());
        f.extend(payload);
        self.with(|s| s.outgoing.push(f));
    }

    pub fn reject_next(&self) {
        self.with(|s| s.reject_next = true);
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.with(|s| s.stop = true);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn write(h: HANDLE, data: &[u8]) {
    let mut at = 0;
    while at < data.len() {
        let mut put = 0u32;
        // SAFETY: a write from a live buffer to the pipe instance this thread owns.
        let ok = unsafe { WriteFile(h, data[at..].as_ptr(), (data.len() - at) as u32, &mut put, std::ptr::null_mut()) };
        if ok == 0 {
            return;
        }
        if put == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
        at += put as usize;
    }
}

fn serve(path: &[u16], behaviour: Behaviour, shared: &Mutex<Shared>) {
    let mut h: HANDLE = INVALID_HANDLE_VALUE;
    let mut connected = false;
    let mut dec = Decoder::default();
    let close = |h: &mut HANDLE, connected: &mut bool| {
        if *h != INVALID_HANDLE_VALUE {
            // SAFETY: the instance this thread created.
            unsafe {
                DisconnectNamedPipe(*h);
                CloseHandle(*h);
            }
            *h = INVALID_HANDLE_VALUE;
        }
        *connected = false;
        shared.lock().unwrap().connected = false;
    };
    loop {
        let (stop, closed, outgoing) = {
            let mut s = shared.lock().unwrap();
            (s.stop, s.closed, std::mem::take(&mut s.outgoing))
        };
        if stop {
            close(&mut h, &mut connected);
            return;
        }
        if closed {
            close(&mut h, &mut connected);
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        if h == INVALID_HANDLE_VALUE {
            // SAFETY: one byte-mode, non-blocking instance named by the test.
            h = unsafe { CreateNamedPipeW(path.as_ptr(), PIPE_ACCESS_DUPLEX, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT, 1, 65536, 65536, 0, std::ptr::null()) };
            if h == INVALID_HANDLE_VALUE {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
        }
        if !connected {
            // SAFETY: a non-blocking connect on the instance above.
            let ok = unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) };
            let err = unsafe { GetLastError() };
            if ok != 0 || err == ERROR_PIPE_CONNECTED {
                connected = true;
                dec = Decoder::default();
                let mut s = shared.lock().unwrap();
                s.connections += 1;
                s.connected = true;
            } else if err == ERROR_NO_DATA {
                // SAFETY: a client that came and went: the instance is made ready for the next one.
                unsafe { DisconnectNamedPipe(h) };
                continue;
            } else {
                std::thread::sleep(Duration::from_millis(3));
                continue;
            }
        }
        for f in outgoing {
            write(h, &f);
        }
        let mut buf = [0u8; 4096];
        let mut n = 0u32;
        // SAFETY: a non-blocking read into a live buffer.
        let ok = unsafe { ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut()) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err == ERROR_BROKEN_PIPE || err == ERROR_PIPE_NOT_CONNECTED {
                // SAFETY: the client went; the instance waits for the next one.
                unsafe { DisconnectNamedPipe(h) };
                connected = false;
                shared.lock().unwrap().connected = false;
            }
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        if n == 0 {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        shared.lock().unwrap().raw.extend_from_slice(&buf[..n as usize]);
        dec.push(&buf[..n as usize]);
        while let Ok(Some(f)) = dec.next_frame() {
            shared.lock().unwrap().frames.push((f.op, f.raw.clone()));
            match f.op {
                OP_HANDSHAKE => match behaviour {
                    Behaviour::Ready => write(
                        h,
                        &ipc::encode(OP_FRAME, &json!({ "cmd": "DISPATCH", "evt": "READY", "data": { "v": 1, "config": {}, "user": { "id": "1", "username": "test" } } }).to_string()),
                    ),
                    Behaviour::RefuseClientId => write(h, &ipc::encode(OP_CLOSE, &json!({ "code": 4000, "message": "Invalid Client ID" }).to_string())),
                    Behaviour::Silent => {}
                },
                OP_FRAME if f.body["cmd"] == "SET_ACTIVITY" => {
                    let reject = std::mem::take(&mut shared.lock().unwrap().reject_next);
                    let answer = if reject {
                        json!({ "cmd": "SET_ACTIVITY", "evt": "ERROR", "data": { "code": 4002, "message": "child \"activity\" fails" }, "nonce": f.body["nonce"] })
                    } else {
                        json!({ "cmd": "SET_ACTIVITY", "evt": null, "data": f.body["args"]["activity"], "nonce": f.body["nonce"] })
                    };
                    write(h, &ipc::encode(OP_FRAME, &answer.to_string()));
                }
                _ => {}
            }
        }
    }
}
