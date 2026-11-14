//! Icon extraction: game textures decoded to PNG on demand into the per-user cache.
//!
//! Icon keys, as stored in the catalogue:
//! * `rom/UI/Icon/<dir>/<name>`: the first texture of `UI/Icon/<dir>/<name>.bntx.zs`;
//! * `lay/<Layout>/<texture>`: a texture of `Layout/<Layout>.Nin_NX_NVN.blarc.zs` (`timg/__Combined.bntx`).
//!
//! Either kind can end in `@<width>` (one of [`SIZES`]): the picture scaled down to that width, proportions kept. The
//! stage pictures are served that way (a card's picture and its miniature) rather than at the game's own size.
//!
//! Keys are validated against the RomFS index before anything is read, so a key can only ever select a file that is
//! part of the player's game data; nothing in a key reaches the real filesystem except a sanitised cache file name.
//!
//! Decoding runs under [`guard::catch`]: whatever the game data holds, a request ends in a picture or an error (the
//! icon workers, the background warm-up and the stage pictures never stop on a panic). A cached file that is not a
//! complete PNG of a plausible size is discarded and decoded again.

use super::bntx;
use super::guard;
use super::romfs::RomFs;
use super::sarc::Sarc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconSource {
    Rom { path: String },
    Layout { archive: String, texture: String },
}

/// The widths a picture can be asked at (`<key>@<width>`): a stage card's picture and its miniature.
pub const SIZES: [u32; 2] = [480, 160];

/// Splits the size suffix off a key: `rom/UI/Icon/StageL/X@480` -> (`rom/UI/Icon/StageL/X`, Some(480)). A suffix that
/// is not one of [`SIZES`] is refused, so a page cannot fill the cache with arbitrary sizes.
pub fn split_size(key: &str) -> Result<(&str, Option<u32>), String> {
    let Some((base, w)) = key.rsplit_once('@') else { return Ok((key, None)) };
    match w.parse::<u32>() {
        Ok(w) if SIZES.contains(&w) => Ok((base, Some(w))),
        _ => Err("icon size not offered".into()),
    }
}

/// The key of a picture at one of [`SIZES`].
pub fn sized(key: &str, width: u32) -> String {
    format!("{key}@{width}")
}

/// Parses and validates an icon key against the RomFS index (a size suffix is checked and left out).
pub fn resolve(rom: &RomFs, key: &str) -> Result<IconSource, String> {
    if key.len() > 200 || key.contains('\0') || key.contains("..") || key.contains('\\') {
        return Err("bad icon key".into());
    }
    let (key, _) = split_size(key)?;
    if let Some(rest) = key.strip_prefix("rom/") {
        if !rest.starts_with("UI/Icon/") {
            return Err("icon outside UI/Icon".into());
        }
        let path = format!("{rest}.bntx.zs");
        return if rom.contains(&path) { Ok(IconSource::Rom { path }) } else { Err("icon not in the RomFS".into()) };
    }
    if let Some(rest) = key.strip_prefix("lay/") {
        let (layout, texture) = rest.split_once('/').ok_or("bad layout icon key")?;
        if layout.is_empty() || texture.is_empty() || layout.contains('/') {
            return Err("bad layout icon key".into());
        }
        let archive = format!("Layout/{layout}.Nin_NX_NVN.blarc.zs");
        return if rom.contains(&archive) { Ok(IconSource::Layout { archive, texture: texture.to_string() }) } else { Err("layout not in the RomFS".into()) };
    }
    Err("unknown icon key kind".into())
}

/// Largest cached PNG served (a full-size icon of the largest texture decoded, fast compression, stays well under it).
const MAX_CACHED_PNG: u64 = 16 << 20;

/// A complete PNG (signature, header first, end marker last) of a size the decoder can produce.
fn plausible_png(b: &[u8]) -> bool {
    const IEND: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82];
    if b.len() < 8 + 25 + 12 || &b[..8] != b"\x89PNG\r\n\x1a\n" || &b[12..16] != b"IHDR" || !b.ends_with(&IEND) {
        return false;
    }
    let dim = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let (w, h) = (dim(16), dim(20));
    (1..=bntx::MAX_DIM).contains(&w) && (1..=bntx::MAX_DIM).contains(&h)
}

/// A file name that is safe on every filesystem and unique per key.
pub fn cache_name(key: &str) -> String {
    let mut s: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else if c == '/' { '~' } else { '_' }).collect();
    // `_` replaces several characters, so add a short hash to keep distinct keys distinct.
    let mut h = 0xcbf29ce484222325u64;
    for b in key.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    s.push_str(&format!("-{:08x}.png", h as u32));
    s
}

pub struct IconStore {
    dir: PathBuf,
    /// Decompressed `__Combined.bntx` of the layouts already opened (they are several MB each).
    layouts: Mutex<HashMap<String, Arc<Vec<u8>>>>,
}

impl IconStore {
    pub fn new(dir: PathBuf) -> IconStore {
        IconStore { dir, layouts: Mutex::new(HashMap::new()) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The cached PNG of a key. A file that is not a plausible PNG (truncated, overwritten, too large) is removed, so the
    /// icon is decoded again.
    pub fn cached(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.dir.join(cache_name(key));
        let small = std::fs::metadata(&path).ok()?.len() <= MAX_CACHED_PNG;
        let bytes = if small { std::fs::read(&path).ok() } else { None };
        match bytes {
            Some(b) if plausible_png(&b) => Some(b),
            _ => {
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    pub fn is_cached(&self, key: &str) -> bool {
        self.dir.join(cache_name(key)).is_file()
    }

    /// PNG bytes of an icon, decoding and caching it on first use.
    pub fn png(&self, rom: &RomFs, key: &str) -> Result<Vec<u8>, String> {
        if let Some(b) = self.cached(key) {
            return Ok(b);
        }
        let png = guard::catch("icon", || self.decode(rom, key))?;
        self.keep(key, &png);
        Ok(png)
    }

    /// Decodes a picture once and caches it at every size of [`SIZES`] not cached yet; returns how many were written.
    pub fn cache_sizes(&self, rom: &RomFs, key: &str) -> Result<usize, String> {
        let missing: Vec<u32> = SIZES.iter().copied().filter(|w| !self.is_cached(&sized(key, *w))).collect();
        if missing.is_empty() {
            return Ok(0);
        }
        let pngs = guard::catch("icon", || {
            let img = self.decode_rgba(rom, key)?;
            missing.iter().map(|w| bntx::encode_png(&bntx::scale_to_width(&img, *w))).collect::<Result<Vec<_>, _>>()
        })?;
        for (w, png) in missing.iter().zip(&pngs) {
            self.keep(&sized(key, *w), png);
        }
        Ok(missing.len())
    }

    /// Writes a decoded icon into the cache (through a temporary file, so a reader never sees half a file).
    fn keep(&self, key: &str, png: &[u8]) {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let _ = std::fs::create_dir_all(&self.dir);
        let target = self.dir.join(cache_name(key));
        let tmp = target.with_extension(format!("tmp{}-{}", std::process::id(), SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        if std::fs::write(&tmp, png).is_ok() && std::fs::rename(&tmp, &target).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    fn decode(&self, rom: &RomFs, key: &str) -> Result<Vec<u8>, String> {
        let img = self.decode_rgba(rom, key)?;
        match split_size(key)?.1 {
            Some(w) => bntx::encode_png(&bntx::scale_to_width(&img, w)),
            None => bntx::encode_png(&img),
        }
    }

    /// The texture of a key at the game's own size (a size suffix is ignored here).
    fn decode_rgba(&self, rom: &RomFs, key: &str) -> Result<bntx::Rgba, String> {
        Ok(match resolve(rom, key)? {
            IconSource::Rom { path } => {
                let data = rom.read(&path)?;
                let texs = bntx::parse(&data)?;
                let t = texs.first().ok_or("empty texture container")?;
                bntx::decode(&data, t)?
            }
            IconSource::Layout { archive, texture } => {
                let combined = self.layout_textures(rom, &archive)?;
                let texs = bntx::parse(&combined)?;
                let t = texs.iter().find(|t| t.name == texture).ok_or_else(|| format!("texture {texture} not in {archive}"))?;
                bntx::decode(&combined, t)?
            }
        })
    }

    fn layout_textures(&self, rom: &RomFs, archive: &str) -> Result<Arc<Vec<u8>>, String> {
        if let Some(v) = self.layouts.lock().unwrap_or_else(|p| p.into_inner()).get(archive) {
            return Ok(v.clone());
        }
        let data = rom.read(archive)?;
        let sarc = Sarc::parse(&data)?;
        let inner = sarc.find(&data, "timg/__Combined.bntx").ok_or("layout has no texture bundle")?.to_vec();
        let arc = Arc::new(inner);
        let mut g = self.layouts.lock().unwrap_or_else(|p| p.into_inner());
        if g.len() > 8 {
            g.clear();
        }
        g.insert(archive.to_string(), arc.clone());
        Ok(arc)
    }

    /// (file count, total bytes) of the cached icons.
    pub fn stats(&self) -> (usize, u64) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return (0, 0) };
        rd.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).fold((0, 0), |(n, b), m| (n + 1, b + m.len()))
    }
}

/// Decodes the percent-encoding of a URL path segment sequence (`%2F` and friends).
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_names_are_safe_and_distinct() {
        let a = cache_name("lay/BtnSelectHead_00/HeadBtn_00^t");
        let b = cache_name("lay/BtnSelectHead_00/HeadBtn_00_t");
        assert_ne!(a, b);
        assert!(a.ends_with(".png"));
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c)), "{a}");
    }

    #[test]
    fn a_damaged_cached_icon_is_discarded() {
        let dir = std::env::temp_dir().join(format!("s3l-iconcache-{}", crate::util::random_hex(4)));
        let store = IconStore::new(dir.clone());
        let key = "rom/UI/Icon/Gear/X";
        let good = bntx::encode_png(&bntx::Rgba { width: 2, height: 2, pixels: vec![200; 16] }).unwrap();
        let path = dir.join(cache_name(key));
        std::fs::create_dir_all(&dir).unwrap();
        for bad in [b"not a png".to_vec(), good[..good.len() - 1].to_vec(), Vec::new()] {
            std::fs::write(&path, &bad).unwrap();
            assert_eq!(store.cached(key), None);
            assert!(!path.exists(), "removed, to be decoded again");
        }
        // A header claiming a size no texture decoded here can have.
        let mut huge = good.clone();
        huge[16..20].copy_from_slice(&(bntx::MAX_DIM + 1).to_be_bytes());
        std::fs::write(&path, &huge).unwrap();
        assert_eq!(store.cached(key), None);
        std::fs::write(&path, &good).unwrap();
        assert_eq!(store.cached(key), Some(good));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("lay%2FBtn%5Et"), "lay/Btn^t");
        assert_eq!(percent_decode("a%zzb%4"), "a%zzb%4");
        assert_eq!(percent_decode("plain"), "plain");
    }

    #[test]
    fn keys_are_validated_against_the_index() {
        let dir = std::env::temp_dir().join(format!("s3l-icons-{}", crate::util::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("romfs.bin"), vec![0u8; 64]).unwrap();
        std::fs::write(
            dir.join("index.json"),
            r#"{"files":[{"path":"UI/Icon/Gear/X.bntx.zs","offset":0,"size":8},{"path":"Layout/L_00.Nin_NX_NVN.blarc.zs","offset":8,"size":8},{"path":"Model/Secret.bfres","offset":16,"size":8}]}"#,
        )
        .unwrap();
        let rom = RomFs::open(&dir).unwrap();
        assert_eq!(resolve(&rom, "rom/UI/Icon/Gear/X").unwrap(), IconSource::Rom { path: "UI/Icon/Gear/X.bntx.zs".into() });
        assert!(resolve(&rom, "rom/UI/Icon/Gear/Y").is_err());
        assert!(resolve(&rom, "rom/Model/Secret.bfres").is_err());
        assert!(resolve(&rom, "rom/UI/Icon/../../Model/Secret").is_err());
        assert!(resolve(&rom, "rom/UI/Icon/Gear/X\\..").is_err());
        assert_eq!(resolve(&rom, "lay/L_00/Tex^s").unwrap(), IconSource::Layout { archive: "Layout/L_00.Nin_NX_NVN.blarc.zs".into(), texture: "Tex^s".into() });
        assert!(resolve(&rom, "lay/L_99/Tex").is_err());
        assert!(resolve(&rom, "lay/L_00").is_err());
        assert!(resolve(&rom, "file/C:/x").is_err());
        // Sizes: only the offered widths, on either kind of key.
        assert_eq!(resolve(&rom, "rom/UI/Icon/Gear/X@480").unwrap(), IconSource::Rom { path: "UI/Icon/Gear/X.bntx.zs".into() });
        assert!(resolve(&rom, "lay/L_00/Tex^s@160").is_ok());
        assert!(resolve(&rom, "rom/UI/Icon/Gear/X@4096").is_err());
        assert!(resolve(&rom, "rom/UI/Icon/Gear/X@").is_err());
        assert_eq!(split_size("rom/a@160").unwrap(), ("rom/a", Some(160)));
        assert_eq!(split_size("rom/a").unwrap(), ("rom/a", None));
        assert_eq!(sized("rom/a", 480), "rom/a@480");
        assert_ne!(cache_name("rom/a@480"), cache_name("rom/a@160"));
        drop(rom);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
