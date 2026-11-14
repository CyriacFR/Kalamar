//! Guards for reading game data. The RomFS the launcher reads is the player's own, but a "ready-made dump" or a mod can
//! carry any bytes, and the launcher must start whatever they are:
//!
//! * third-party decoders (texture2ddecoder, tegra_swizzle, png, ruzstd) run under [`catch`], so a panic inside one of
//!   them (texture2ddecoder 0.1.2 panics on some malformed ASTC blocks) becomes an ordinary error. This needs the
//!   release profile's `panic = "unwind"` (Cargo.toml): with `abort`, nothing can catch it;
//! * the readers charge what a file makes them build (nodes, bytes) to a [`Budget`] proportional to the file's own
//!   size, so offsets shared on purpose cannot expand a few kilobytes into gigabytes;
//! * large buffers whose size comes from the data are reserved with [`filled`], which fails instead of aborting.

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

thread_local! {
    /// Set while a guarded call runs on this thread: its panic is reported as an error, not printed.
    static GUARDED: Cell<bool> = const { Cell::new(false) };
}

/// Installs, once, a panic hook that stays silent for the panics [`catch`] turns into errors and hands every other
/// panic to the hook that was there before.
fn install_quiet_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !GUARDED.with(Cell::get) {
                previous(info);
            }
        }));
    });
}

/// Runs `f`, a call into a decoder working on game data. A panic inside it becomes `Err`, naming `what` and the
/// panic's message.
pub fn catch<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    install_quiet_hook();
    let was = GUARDED.with(|g| g.replace(true));
    let result = panic::catch_unwind(AssertUnwindSafe(f));
    GUARDED.with(|g| g.set(was));
    result.unwrap_or_else(|payload| {
        let msg = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_default();
        let msg: String = msg.chars().take(160).collect();
        Err(format!("{what}: invalid data ({msg})"))
    })
}

/// A per-parse allowance: a reader charges each node or byte it builds, and stops with an error once the allowance is
/// spent. A file that stores each structure once (as the game's own files do) stays within it.
#[derive(Debug)]
pub struct Budget {
    left: usize,
    what: &'static str,
}

impl Budget {
    /// `per_byte` units for each byte of the input, plus `floor`, at most `max`.
    pub fn new(what: &'static str, input_len: usize, per_byte: usize, floor: usize, max: usize) -> Budget {
        Budget { left: input_len.saturating_mul(per_byte).saturating_add(floor).min(max), what }
    }

    pub fn take(&mut self, n: usize) -> Result<(), String> {
        match self.left.checked_sub(n) {
            Some(left) => {
                self.left = left;
                Ok(())
            }
            None => {
                self.left = 0;
                Err(format!("{}: the data expands beyond what its size allows (corrupt or crafted file)", self.what))
            }
        }
    }
}

/// A vector of `n` copies of `v`, or an error when the memory cannot be reserved (where `vec!` would abort).
pub fn filled<T: Clone>(n: usize, v: T) -> Result<Vec<T>, String> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| format!("cannot reserve memory for {n} elements"))?;
    out.resize(n, v);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_becomes_an_error_and_the_thread_goes_on() {
        let r: Result<u8, String> = catch("decoder", || {
            let v: Vec<u8> = Vec::new();
            let i = v.len() + 3;
            Ok(v[i])
        });
        let e = r.unwrap_err();
        assert!(e.starts_with("decoder: invalid data (index out of bounds"), "{e}");
        assert_eq!(catch("decoder", || Ok(7)), Ok(7));
        assert_eq!(catch::<u8>("decoder", || Err("plain error".into())), Err("plain error".to_string()));
        // Nested guards restore the outer state.
        let outer: Result<(), String> = catch("outer", || {
            let inner: Result<(), String> = catch("inner", || panic!("boom"));
            assert!(inner.unwrap_err().contains("boom"));
            assert!(GUARDED.with(Cell::get));
            Ok(())
        });
        assert!(outer.is_ok());
        assert!(!GUARDED.with(Cell::get));
    }

    #[test]
    fn budgets_stop_at_their_allowance() {
        let mut b = Budget::new("T", 10, 2, 5, usize::MAX);
        assert!(b.take(20).is_ok());
        assert!(b.take(5).is_ok());
        assert!(b.take(1).unwrap_err().starts_with("T: the data expands"));
        assert!(b.take(0).is_ok());
        let mut capped = Budget::new("T", usize::MAX, usize::MAX, 1, 100);
        assert!(capped.take(100).is_ok() && capped.take(1).is_err());
        assert_eq!(filled(3, 9u8).unwrap(), vec![9, 9, 9]);
        assert!(filled(usize::MAX / 2, 0u32).is_err());
    }
}
