//! Small filesystem helpers shared by the modules.

use crate::error::{AppError, AppResult};
use std::io::Write;
use std::path::Path;

/// A Unicode format character (general category Cf) other than the zero-width joiner and non-joiner (U+200D, U+200C,
/// which emoji sequences, Persian and Indic scripts need): the bidirectional embeddings, overrides, isolates and marks
/// (U+202A to U+202E, U+2066 to U+2069, U+200E, U+200F, U+061C), invisible spaces and operators, the byte-order mark, tag
/// characters. Such a character in a name lets a server or another player reorder or hide what the player reads
/// ("Kalamar official" shown for another text).
pub fn is_hidden_format(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x0890..=0x0891 | 0x08E2 | 0x180E | 0x200B | 0x200E..=0x200F
        | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x13430..=0x1343F
        | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}

/// Not allowed in a name or a one-line text from a server: a control character (Cc) or a hidden format one.
pub fn unsafe_char(c: char) -> bool {
    c.is_control() || is_hidden_format(c)
}

/// Writes `bytes` to a temporary file next to `path`, flushes it to disk and renames it over `path`.
/// A crash in the middle leaves either the old file or the new one, never half of each.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, &e))?;
    }
    let mut tmp_name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    tmp_name.push(format!(".tmp{}", std::process::id()));
    let tmp = path.with_file_name(tmp_name);
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::io(path, &e));
    }
    Ok(())
}

/// Reads a text file (lossy UTF-8) of at most `max` bytes; `Ok(None)` when it does not exist.
pub fn read_text_limited(path: &Path, max: u64) -> AppResult<Option<String>> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > max => {
            return Err(AppError::Io { path: path.display().to_string(), message: format!("file is larger than {max} bytes") });
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(AppError::io(path, &e)),
    }
    let bytes = std::fs::read(path).map_err(|e| AppError::io(path, &e))?;
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// Seconds since the Unix epoch of a file's modification time.
pub fn mtime_secs(path: &Path) -> Option<u64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    m.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Random-ish hex from the std hasher's per-process random keys and the clock. Not a CSPRNG: it only needs to
/// make a player identity or room code unlikely to collide with another one.
pub fn random_hex(bytes: usize) -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    let mut counter = 0u64;
    while out.len() < bytes * 2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(counter);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        h.write_u128(nanos);
        out.push_str(&format!("{:016x}", h.finish()));
        counter += 1;
    }
    out.truncate(bytes * 2);
    out
}

/// Last `n` lines of a text file, reading at most the final 256 KiB. `Ok(None)` when the file is missing.
pub fn tail_lines(path: &Path, n: usize) -> AppResult<Option<String>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(AppError::io(path, &e)),
    };
    let len = f.metadata().map_err(|e| AppError::io(path, &e))?.len();
    let start = len.saturating_sub(256 * 1024);
    f.seek(SeekFrom::Start(start)).map_err(|e| AppError::io(path, &e))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| AppError::io(path, &e))?;
    let text = String::from_utf8_lossy(&buf);
    let lines: Vec<&str> = text.lines().collect();
    let skip = lines.len().saturating_sub(n);
    Ok(Some(lines[skip..].join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bidi_and_hidden_format_characters_are_unsafe_in_names() {
        for c in ['\u{202E}', '\u{202A}', '\u{2066}', '\u{2069}', '\u{200E}', '\u{200F}', '\u{061C}', '\u{200B}', '\u{FEFF}', '\u{2060}', '\u{E0041}', '\u{0007}', '\n'] {
            assert!(unsafe_char(c), "U+{:04X}", c as u32);
        }
        for c in ['a', 'é', '\u{0643}', '\u{05D0}', '\u{200D}', '\u{200C}', '\u{1F991}', ' ', '\u{3042}'] {
            assert!(!unsafe_char(c), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("s3l-util-{}", random_hex(4)));
        let p = dir.join("a.txt");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_missing_is_none() {
        assert!(read_text_limited(Path::new("Z:/definitely/not/here.txt"), 10).unwrap().is_none());
    }

    #[test]
    fn random_hex_len() {
        assert_eq!(random_hex(16).len(), 32);
        assert_ne!(random_hex(16), random_hex(16));
    }

    #[test]
    fn tail_returns_last_lines() {
        let dir = std::env::temp_dir().join(format!("s3l-tail-{}", random_hex(4)));
        let p = dir.join("t.log");
        atomic_write(&p, b"1\n2\n3\n4\n5\n").unwrap();
        assert_eq!(tail_lines(&p, 2).unwrap().unwrap(), "4\n5");
        assert!(tail_lines(&dir.join("nope"), 2).unwrap().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
