//! Runtime glue: opens the RomFS of the configured game root, loads or builds the catalogue (cached as JSON in the
//! app data directory, keyed by the image size and index hash) and owns the icon store. Building the catalogue also
//! decodes the stage pictures into the icon store (see [`cache_stage_pictures`]). Everything it writes goes under
//! `<data dir>\game-cache\<key>\`.
//!
//! Neither the game data nor the cache can stop the launcher from starting: a cached catalogue that is too large,
//! unreadable, of another key or with malformed icon keys is rebuilt; a build that panics is an error (the neutral
//! catalogue is shown); and a build that ended the process (nothing can catch an out-of-memory abort) leaves a marker
//! behind, so that after [`MAX_BUILD_ATTEMPTS`] such starts the build waits for an explicit rebuild instead of running
//! (and failing) at every start.

use super::catalogue::{self, Catalogue, SCHEMA};
use super::guard;
use super::icons::IconStore;
use super::romfs::RomFs;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub struct Loaded {
    pub rom: Arc<RomFs>,
    pub store: Arc<IconStore>,
    pub catalogue: Arc<Catalogue>,
    pub from_cache: bool,
}

#[derive(Default)]
pub struct GameService {
    pub loaded: Arc<Mutex<Option<Arc<Loaded>>>>,
    pub warming: Arc<AtomicBool>,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IconProgress {
    pub done: usize,
    pub total: usize,
    pub failed: usize,
    pub running: bool,
}

pub fn cache_root(data_dir: &Path) -> PathBuf {
    data_dir.join("game-cache")
}

fn catalogue_path(dir: &Path) -> PathBuf {
    dir.join("catalogue.json")
}

/// Present while a catalogue is being built; holds the number of builds started since the last one that finished.
fn building_path(dir: &Path) -> PathBuf {
    dir.join("building")
}

/// Builds that ended the process in a row before the build waits for an explicit rebuild.
pub const MAX_BUILD_ATTEMPTS: u32 = 2;
/// Largest cached catalogue read (11.2.0's is 2.4 MB).
const MAX_CACHED_CATALOGUE: u64 = 32 << 20;

/// A cache key is one directory name made of lowercase letters, digits and dashes (see `RomFs::open`).
fn safe_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 96 && key.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b == b'-')
}

/// An icon key as the catalogue builds them (they are checked against the RomFS again when used).
fn plausible_icon_key(k: &str) -> bool {
    (k.starts_with("rom/UI/Icon/") || k.starts_with("lay/")) && k.len() <= 200 && !k.contains("..") && !k.contains(['\\', '\0'])
}

fn read_cached(dir: &Path, key: &str) -> Option<Catalogue> {
    let path = catalogue_path(dir);
    if std::fs::metadata(&path).ok()?.len() > MAX_CACHED_CATALOGUE {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let c: Catalogue = serde_json::from_slice(&bytes).ok()?;
    let icons = c.categories.values().flatten().filter_map(|e| e.icon.as_deref());
    let named = [&c.brands, &c.abilities, &c.subs, &c.specials, &c.classes].into_iter().flat_map(|m| m.values()).filter_map(|n| n.icon.as_deref());
    let stages = c.stages.iter().filter_map(|s| s.image.as_deref());
    let keys_ok = icons.chain(named).chain(stages).all(plausible_icon_key);
    (c.schema == SCHEMA && c.cache_key == key && keys_ok).then_some(c)
}

/// The builds started for this cache directory and not finished (0 without a marker; a garbled marker counts as one).
fn unfinished_builds(dir: &Path) -> u32 {
    match std::fs::read_to_string(building_path(dir)) {
        Ok(s) => s.trim().parse().unwrap_or(1),
        Err(_) => 0,
    }
}

/// Part of building the cache: every stage picture, at every served size, decoded into the icon cache (a few dozen
/// small files, four at a time) so that the stage cards show them at once. A picture that cannot be decoded is left to
/// the on-demand path, which reports the error. Returns the number of files written.
pub fn cache_stage_pictures(loaded: &Loaded) -> usize {
    let keys: Vec<&str> = loaded.catalogue.stages.iter().filter_map(|s| s.image.as_deref()).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let written = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..4.min(keys.len()) {
            s.spawn(|| {
                while let Some(k) = keys.get(next.fetch_add(1, Ordering::SeqCst)) {
                    if let Ok(n) = loaded.store.cache_sizes(&loaded.rom, k) {
                        written.fetch_add(n, Ordering::SeqCst);
                    }
                }
            });
        }
    });
    written.into_inner()
}

impl GameService {
    pub fn current(&self) -> Option<Arc<Loaded>> {
        self.loaded.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Opens `root`'s RomFS and loads (or builds, or with `force` rebuilds) the catalogue.
    pub fn load(&self, root: &Path, data_dir: &Path, force: bool) -> Result<Arc<Loaded>, String> {
        let rom = Arc::new(crate::trace::timed("RomFs::open (index.json parse, image opened)", || RomFs::open(root))?);
        if !safe_key(&rom.cache_key) {
            return Err(format!("unusable cache key {:?}", rom.cache_key));
        }
        let dir = cache_root(data_dir).join(&rom.cache_key);
        let cached = if force { None } else { crate::trace::timed("catalogue.json read+parse", || read_cached(&dir, &rom.cache_key)) };
        let (catalogue, from_cache) = match cached {
            Some(c) => (c, true),
            None => {
                let attempts = unfinished_builds(&dir);
                if !force && attempts >= MAX_BUILD_ATTEMPTS {
                    return Err(format!("the last {attempts} builds of the catalogue from this game data did not finish; it is built again on an explicit rebuild"));
                }
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(building_path(&dir), (attempts + 1).to_string());
                // A panic is an error like any other (the marker stays: it counts as an attempt).
                let c = guard::catch("catalogue", || Ok(catalogue::build(&rom)))?;
                // A different game version leaves a stale cache behind: drop sibling directories.
                if let Ok(rd) = std::fs::read_dir(cache_root(data_dir)) {
                    for e in rd.flatten() {
                        if e.path() != dir && e.path().is_dir() {
                            let _ = std::fs::remove_dir_all(e.path());
                        }
                    }
                }
                let _ = std::fs::create_dir_all(&dir);
                if let Ok(text) = serde_json::to_vec(&c) {
                    let _ = crate::util::atomic_write(&catalogue_path(&dir), &text);
                }
                let _ = std::fs::remove_file(building_path(&dir));
                (c, false)
            }
        };
        let loaded = Arc::new(Loaded { store: Arc::new(IconStore::new(dir.join("icons"))), rom, catalogue: Arc::new(catalogue), from_cache });
        if !from_cache {
            crate::trace::timed("stage pictures", || cache_stage_pictures(&loaded));
        }
        *self.loaded.lock().unwrap_or_else(|p| p.into_inner()) = Some(loaded.clone());
        Ok(loaded)
    }

    /// Starts decoding every icon of the catalogue in the background. `progress` is called after each icon (and once
    /// at the end with `running = false`). Returns false when a job is already running.
    pub fn warm_icons(&self, progress: impl Fn(IconProgress) + Send + 'static) -> bool {
        let Some(loaded) = self.current() else { return false };
        if self.warming.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.cancel.store(false, Ordering::SeqCst);
        let (warming, cancel) = (self.warming.clone(), self.cancel.clone());
        std::thread::spawn(move || {
            let keys = catalogue::icon_keys(&loaded.catalogue);
            let total = keys.len();
            let (mut done, mut failed) = (0usize, 0usize);
            for k in keys {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                if !loaded.store.is_cached(&k) && loaded.store.png(&loaded.rom, &k).is_err() {
                    failed += 1;
                }
                done += 1;
                if done % 25 == 0 {
                    progress(IconProgress { done, total, failed, running: true });
                }
            }
            warming.store(false, Ordering::SeqCst);
            progress(IconProgress { done, total, failed, running: false });
        });
        true
    }
}
