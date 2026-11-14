//! Random access to the player's RomFS image (`romfs.bin`) through its `index.json`.
//! Nothing is extracted to disk; entries are read at their offset and, for `.zs` files, zstd-decompressed.
//!
//! The image may come from anywhere (a "ready-made dump", a mod), so every size is bounded: an entry is read only when
//! it lies inside the image and is at most [`MAX_ENTRY`] bytes, and a `.zs` entry decompresses to at most
//! [`MAX_ENTRY`] bytes and [`MAX_RATIO`] times its stored size (beyond [`RATIO_FLOOR`]); past either limit it is an
//! error, never a silently truncated file.

use super::guard;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct IndexFile {
    #[serde(default)]
    romfs_size: u64,
    #[serde(default)]
    romfs_sha256: String,
    files: Vec<IndexEntry>,
}

#[derive(Deserialize)]
struct IndexEntry {
    path: String,
    offset: u64,
    size: u64,
}

pub struct RomFs {
    file: File,
    entries: HashMap<String, (u64, u64)>,
    /// Identifies this exact game data: image size, the checksum recorded in the index, and a hash of the index file.
    pub cache_key: String,
    pub image_path: PathBuf,
}

/// Where the RomFS image of a game root lives: `<root>\romfs\romfs.bin`, or `<root>\romfs.bin`.
pub fn locate(root: &Path) -> Option<(PathBuf, PathBuf)> {
    for dir in [root.join("romfs"), root.to_path_buf()] {
        let (bin, idx) = (dir.join("romfs.bin"), dir.join("index.json"));
        if bin.is_file() && idx.is_file() {
            return Some((bin, idx));
        }
    }
    None
}

/// Largest entry read, stored or decompressed. The largest file of 11.2.0's merged RomFS among those the launcher could
/// read (layout archives, message archives, parameter tables, `UI/Icon`) is a layout archive of 49.5 MB decompressed.
pub const MAX_ENTRY: usize = 64 << 20;
/// Largest expansion of a `.zs` entry decompressing to more than [`RATIO_FLOOR`] bytes. In 11.2.0 those expand at most
/// 47 times (smaller files, up to 236 times: uniform icons).
pub const MAX_RATIO: usize = 128;
pub const RATIO_FLOOR: usize = 4 << 20;

/// The part of the image's checksum that names its cache directory: the first 16 characters of `romfs_sha256` when it is
/// a SHA-256 in lowercase hex, else a fixed placeholder (the rest of the cache key still tells images apart). Nothing
/// read from `index.json` can put a path separator or `..` into the key.
fn sha_part(sha: &str) -> &str {
    if sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) { &sha[..16] } else { "nosha" }
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(windows)]
fn read_at(f: &File, buf: &mut [u8], off: u64) -> std::io::Result<usize> {
    use std::os::windows::fs::FileExt;
    f.seek_read(buf, off)
}
#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], off: u64) -> std::io::Result<usize> {
    use std::os::unix::fs::FileExt;
    f.read_at(buf, off)
}

impl RomFs {
    pub fn open(root: &Path) -> Result<RomFs, String> {
        let (bin, idx) = locate(root).ok_or_else(|| format!("no romfs.bin + index.json under {}", root.display()))?;
        Self::open_files(&bin, &idx)
    }

    pub fn open_files(bin: &Path, idx: &Path) -> Result<RomFs, String> {
        let text = std::fs::read(idx).map_err(|e| format!("{}: {e}", idx.display()))?;
        let index: IndexFile = serde_json::from_slice(&text).map_err(|e| format!("{}: {e}", idx.display()))?;
        let file = File::open(bin).map_err(|e| format!("{}: {e}", bin.display()))?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        let mut entries = HashMap::with_capacity(index.files.len());
        for e in index.files {
            if e.offset.checked_add(e.size).is_none_or(|end| end > len) {
                continue; // truncated image: the entry simply cannot be read
            }
            entries.insert(e.path, (e.offset, e.size));
        }
        let cache_key = format!("{}-{}-{:016x}", len, sha_part(&index.romfs_sha256), fnv1a(&text));
        let _ = index.romfs_size;
        Ok(RomFs { file, entries, cache_key, image_path: bin.to_path_buf() })
    }

    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(path)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }

    /// The stored bytes of an entry (still compressed for `.zs`), at most [`MAX_ENTRY`] bytes.
    pub fn read_raw(&self, path: &str) -> Result<Vec<u8>, String> {
        self.read_raw_capped(path, MAX_ENTRY)
    }

    fn read_raw_capped(&self, path: &str, cap: usize) -> Result<Vec<u8>, String> {
        let &(off, size) = self.entries.get(path).ok_or_else(|| format!("{path}: not in the RomFS index"))?;
        // The index gives the size; `open` kept only entries inside the image, and none is read past the cap.
        let size = usize::try_from(size).ok().filter(|s| *s <= cap).ok_or_else(|| format!("{path}: entry of {size} bytes, more than this launcher reads"))?;
        let mut buf = guard::filled(size, 0u8).map_err(|e| format!("{path}: {e}"))?;
        let mut done = 0usize;
        while done < buf.len() {
            let n = read_at(&self.file, &mut buf[done..], off + done as u64).map_err(|e| format!("{path}: {e}"))?;
            if n == 0 {
                return Err(format!("{path}: unexpected end of image"));
            }
            done += n;
        }
        Ok(buf)
    }

    /// The file contents, decompressed when the name ends in `.zs`.
    pub fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let raw = self.read_raw(path)?;
        if path.ends_with(".zs") { zstd_decode(&raw).map_err(|e| format!("{path}: {e}")) } else { Ok(raw) }
    }
}

/// Decompresses one zstd stream (the game's assets are plain frames, no dictionary). Output past [`MAX_ENTRY`] bytes,
/// or past [`MAX_RATIO`] times the input once over [`RATIO_FLOOR`], is an error.
pub fn zstd_decode(raw: &[u8]) -> Result<Vec<u8>, String> {
    zstd_decode_capped(raw, raw.len().saturating_mul(MAX_RATIO).clamp(RATIO_FLOOR, MAX_ENTRY))
}

fn zstd_decode_capped(raw: &[u8], cap: usize) -> Result<Vec<u8>, String> {
    guard::catch("zstd", || {
        // The window is the history the decoder keeps (it grows with the output, nothing is reserved for it up front):
        // at most MAX_ENTRY, half ruzstd's default (11.2.0 uses at most 2 MiB).
        let mut dec = ruzstd::decoding::StreamingDecoder::new_with_max_window_size(raw, MAX_ENTRY as u64).map_err(|e| e.to_string())?;
        let declared = dec.decoder.content_size();
        if declared > cap as u64 {
            return Err(format!("declares {declared} bytes once decompressed, more than the {cap} allowed"));
        }
        let mut out = Vec::new();
        out.try_reserve_exact(declared as usize).map_err(|_| "cannot reserve memory for the decompressed data".to_string())?;
        // One byte more than allowed tells a stream over the limit from one exactly at it.
        (&mut dec).take(cap as u64 + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
        if out.len() > cap {
            return Err(format!("decompresses to more than the {cap} bytes allowed"));
        }
        Ok(out)
    })
}

/// Test helper: a valid zstd frame made of raw (uncompressed) blocks, so fixtures can be `.zs` files.
#[cfg(test)]
pub fn raw_frame(payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0x28, 0xB5, 0x2F, 0xFD];
    if payload.len() < 256 {
        f.push(0x20); // single segment, 1-byte content size
        f.push(payload.len() as u8);
    } else if payload.len() < 65_536 + 256 {
        f.push(0x60); // single segment, 2-byte content size (stored minus 256)
        f.extend_from_slice(&((payload.len() - 256) as u16).to_le_bytes());
    } else {
        f.push(0xA0); // single segment, 4-byte content size
        f.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    }
    // Blocks of at most 128 KiB, type 0 = raw, the last one flagged.
    let blocks: Vec<&[u8]> = if payload.is_empty() { vec![payload] } else { payload.chunks(128 << 10).collect() };
    for (i, b) in blocks.iter().enumerate() {
        let hdr = ((b.len() as u32) << 3) | (i + 1 == blocks.len()) as u32;
        f.extend_from_slice(&hdr.to_le_bytes()[..3]);
        f.extend_from_slice(b);
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zstd_raw_block_roundtrip() {
        assert_eq!(zstd_decode(&raw_frame(b"hello")).unwrap(), b"hello");
        let big = vec![7u8; 1000];
        assert_eq!(zstd_decode(&raw_frame(&big)).unwrap(), big);
        let bigger: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(zstd_decode(&raw_frame(&bigger)).unwrap(), bigger, "several blocks, 4-byte size");
        assert!(zstd_decode(b"not zstd").is_err());
    }

    /// A frame of `blocks` RLE blocks of 128 KiB each (4 bytes in, 128 KiB out), no declared size.
    fn rle_bomb(blocks: usize) -> Vec<u8> {
        let mut f = vec![0x28, 0xB5, 0x2F, 0xFD, 0x00, 0x58]; // no content size, 2 MiB window
        for i in 0..blocks {
            let hdr = ((128u32 << 10) << 3) | (1 << 1) | (i + 1 == blocks) as u32;
            f.extend_from_slice(&hdr.to_le_bytes()[..3]);
            f.push(0x41);
        }
        f
    }

    #[test]
    fn zstd_output_is_capped_with_an_error_not_truncated() {
        // 16 KiB that expand to 512 MiB: stopped at the ratio limit (the floor, 4 MiB here), with an error.
        let bomb = rle_bomb(4096);
        assert_eq!(bomb.len(), 16_390);
        let t = std::time::Instant::now();
        let e = zstd_decode(&bomb).unwrap_err();
        assert!(e.contains("more than the 4194304 bytes allowed"), "{e}");
        assert!(t.elapsed() < std::time::Duration::from_secs(10));
        // Within the floor: decoded in full.
        assert_eq!(zstd_decode(&rle_bomb(32)).unwrap().len(), 32 << 17);
        // Past the cap by one byte is an error; exactly at it is the whole file (nothing silently cut).
        let payload: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        let frame = raw_frame(&payload);
        assert_eq!(zstd_decode_capped(&frame, 1000).unwrap(), payload);
        assert!(zstd_decode_capped(&frame, 999).is_err());
        let mut undeclared = rle_bomb(1);
        assert_eq!(zstd_decode_capped(&undeclared, 128 << 10).unwrap().len(), 128 << 10);
        assert!(zstd_decode_capped(&undeclared, (128 << 10) - 1).unwrap_err().contains("more than the"));
        // A declared size beyond the cap is refused before decoding; so is a window larger than MAX_ENTRY.
        undeclared[4] = 0xC0; // 8-byte content size follows the window descriptor
        undeclared.splice(6..6, (1u64 << 40).to_le_bytes());
        assert!(zstd_decode(&undeclared).unwrap_err().contains("declares 1099511627776 bytes"));
        let mut wide = vec![0x28, 0xB5, 0x2F, 0xFD, 0x00, 0x88]; // 128 MiB window, one 1-byte RLE block
        wide.extend_from_slice(&((1u32 << 3) | (1 << 1) | 1).to_le_bytes()[..3]);
        wide.push(0x41);
        assert!(zstd_decode(&wide).is_err());
    }

    #[test]
    fn cache_keys_only_take_a_lowercase_hex_checksum() {
        let sha = "0123456789abcdef".repeat(4);
        assert_eq!(sha_part(&sha), "0123456789abcdef");
        for bad in ["/../../../../../", "..\\..\\..\\x", "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF", "abcdef0123456789ff", ""] {
            assert_eq!(sha_part(bad), "nosha", "{bad}");
        }
        assert_eq!(sha_part(&"é".repeat(32)), "nosha");
        assert_eq!(sha_part(&format!("{}/", &sha[..63])), "nosha");
    }

    #[test]
    fn reads_entries_through_the_index() {
        let dir = std::env::temp_dir().join(format!("s3l-romfs-{}", crate::util::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let comp = raw_frame(b"zstd payload");
        let mut image = vec![0u8; 512];
        image.extend_from_slice(b"plain file");
        let off2 = image.len();
        image.extend_from_slice(&comp);
        std::fs::write(dir.join("romfs.bin"), &image).unwrap();
        let index = |sha: &str| {
            format!(
                r#"{{"status":"pass","romfs_size":{},"romfs_sha256":"{sha}","files":[
                {{"path":"A/plain.txt","offset":512,"size":10}},
                {{"path":"A/packed.bin.zs","offset":{},"size":{}}},
                {{"path":"A/outside.bin","offset":99999,"size":10}},
                {{"path":"A/huge.bin","offset":0,"size":{}}}]}}"#,
                image.len(),
                off2,
                comp.len(),
                image.len()
            )
        };
        std::fs::write(dir.join("index.json"), index(&"abcdef0123456789".repeat(4))).unwrap();
        let r = RomFs::open(&dir).unwrap();
        assert_eq!(r.read("A/plain.txt").unwrap(), b"plain file");
        assert_eq!(r.read("A/packed.bin.zs").unwrap(), b"zstd payload");
        assert!(r.read("A/outside.bin").is_err());
        assert!(r.read("nope").is_err());
        assert!(r.cache_key.starts_with(&format!("{}-abcdef0123456789-", image.len())));
        // An entry larger than what the launcher reads is refused before anything is allocated for it.
        assert_eq!(r.read_raw("A/huge.bin").unwrap().len(), image.len());
        assert!(r.read_raw_capped("A/huge.bin", image.len() - 1).unwrap_err().contains("more than this launcher reads"));
        drop(r);
        // A checksum crafted to climb out of the cache directory leaves no trace in the key.
        std::fs::write(dir.join("index.json"), index("/../../../../../")).unwrap();
        let r = RomFs::open(&dir).unwrap();
        assert!(r.cache_key.starts_with(&format!("{}-nosha-", image.len())), "{}", r.cache_key);
        assert!(r.cache_key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'), "{}", r.cache_key);
        assert!(RomFs::open(&dir.join("missing")).is_err());
        drop(r);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
