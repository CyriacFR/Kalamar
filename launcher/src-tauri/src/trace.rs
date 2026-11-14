//! Start-up timing log. Off unless the `KALAMAR_TRACE` environment variable is set (to a file path, or to `1` for
//! `kalamar-startup-trace.log` in the temp directory); then each [`mark`] appends one line `<epoch ms>\t<label>` to it.
//! Epoch milliseconds, so the lines line up with the page's `performance.timeOrigin` and a measuring script's clock.
//! Nothing is written, and nothing but one atomic load is spent, when the variable is absent.

use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

struct Sink {
    file: Mutex<std::fs::File>,
}

static SINK: OnceLock<Option<Sink>> = OnceLock::new();
static ORIGIN: OnceLock<Instant> = OnceLock::new();

fn epoch_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// Reads `KALAMAR_TRACE`; call once, first thing in `run`.
pub fn init() {
    ORIGIN.get_or_init(Instant::now);
    SINK.get_or_init(|| {
        let v = std::env::var_os("KALAMAR_TRACE")?;
        let path = if v == "1" { std::env::temp_dir().join("kalamar-startup-trace.log") } else { v.into() };
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
        Some(Sink { file: Mutex::new(file) })
    });
    mark("rust:init");
}

pub fn enabled() -> bool {
    matches!(SINK.get(), Some(Some(_)))
}

pub fn mark(label: &str) {
    if let Some(Some(s)) = SINK.get() {
        let since = ORIGIN.get().map(|o| o.elapsed().as_micros()).unwrap_or(0);
        if let Ok(mut f) = s.file.lock() {
            let _ = writeln!(f, "{}\t{}\t+{}us", epoch_ms(), label, since);
        }
    }
}

/// Logs `label` with the time `f` took (microseconds) when tracing is on.
pub fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let t = Instant::now();
    let out = f();
    mark(&format!("{label} took {}us", t.elapsed().as_micros()));
    out
}
