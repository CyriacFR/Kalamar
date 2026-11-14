//! Integration test against the player's real game data. Ignored by default; run with
//!   KALAMAR_GAME_ROOT=<folder holding romfs\romfs.bin> cargo test --test game_data -- --ignored --nocapture
//! It prints the category counts and checks that every category that has game data is populated.

use kalamar_lib::game::{
    bntx,
    catalogue::{self, CATEGORY_IDS},
    icons::{self, IconStore},
    romfs::RomFs,
};
use std::path::PathBuf;

fn root() -> PathBuf {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
    PathBuf::from(var("KALAMAR_GAME_ROOT").or_else(|| var("SPLAT3_GAME_ROOT")).expect("set KALAMAR_GAME_ROOT"))
}

#[test]
#[ignore = "needs the player's game data (KALAMAR_GAME_ROOT)"]
fn catalogue_and_icons_from_the_real_romfs() {
    let rom = RomFs::open(&root()).expect("open romfs");
    println!("RomFS: {} entries, cache key {}", rom.len(), rom.cache_key);
    let started = std::time::Instant::now();
    let cat = catalogue::build(&rom);
    println!("catalogue built in {:?}", started.elapsed());
    for r in &cat.report {
        println!("{:10} rows {:5}  selectable {:5}  icons {:5}  names {:5}  {}", r.category, r.rows, r.selectable, r.with_icon, r.with_name, r.note);
    }
    for id in CATEGORY_IDS {
        let r = cat.report.iter().find(|r| r.category == id).expect("report row");
        // Every badge is listed but none can be selected (`catalogue::apply_save_constraints`).
        let present = if id == "badge" { r.rows } else { r.selectable };
        assert!(present > 0, "category {id} is empty");
    }
    // Gear, weapons and splashtag items must be fully named and pictured.
    for id in ["head", "clothes", "shoes", "weapon", "badge"] {
        let r = cat.report.iter().find(|r| r.category == id).unwrap();
        assert_eq!(r.with_icon, r.selectable, "{id}: icons");
        assert_eq!(r.with_name, r.selectable, "{id}: names");
    }
    for id in ["adjective", "subject"] {
        let r = cat.report.iter().find(|r| r.category == id).unwrap();
        assert_eq!(r.with_name, r.selectable, "{id}: names");
    }
    assert_eq!(cat.report.iter().find(|r| r.category == "banner").unwrap().with_icon, 254);

    // Decode a sample of icons (one per kind) and check they are real images.
    let dir = std::env::temp_dir().join(format!("s3l-it-icons-{}", std::process::id()));
    let store = IconStore::new(dir.clone());
    let mut sample: Vec<String> = Vec::new();
    for id in ["hair", "eyebrows", "head", "clothes", "shoes", "weapon", "banner", "badge"] {
        sample.extend(cat.categories[id].iter().filter(|e| e.selectable).filter_map(|e| e.icon.clone()).take(3));
    }
    assert!(sample.len() >= 20);
    let t = std::time::Instant::now();
    for k in &sample {
        let png = store.png(&rom, k).unwrap_or_else(|e| panic!("{k}: {e}"));
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }
    println!("decoded {} sample icons in {:?}", sample.len(), t.elapsed());

    // Format survey of every UI icon: all must decode with the supported formats, within the size limits.
    let mut formats = std::collections::BTreeMap::<String, usize>::new();
    for p in rom.paths().filter(|p| p.starts_with("UI/Icon/") && p.ends_with(".bntx.zs")) {
        let data = rom.read(p).unwrap();
        for t in bntx::parse(&data).unwrap() {
            *formats.entry(bntx::format_name(t.format)).or_default() += 1;
            assert!(t.width <= bntx::MAX_DIM && t.height <= bntx::MAX_DIM && t.width as u64 * t.height as u64 <= bntx::MAX_PIXELS, "{p}: {}x{}", t.width, t.height);
        }
    }
    println!("UI/Icon texture formats: {formats:?} ({} textures)", formats.values().sum::<usize>());
    let _ = std::fs::remove_dir_all(dir);
}

/// The versus stages: every one named in the four UI languages and pictured, the pictures decoded and scaled to the
/// served sizes. `KALAMAR_STAGE_DUMP=<folder outside the repository>` keeps the pictures there to look at them.
#[test]
#[ignore = "needs the player's game data (KALAMAR_GAME_ROOT)"]
fn stages_from_the_real_romfs() {
    let rom = RomFs::open(&root()).expect("open romfs");
    let cat = catalogue::build(&rom);
    let listed = cat.stages.iter().filter(|s| s.listed).count();
    println!("{} stages, {listed} in the game's stage lists", cat.stages.len());
    let keep = std::env::var_os("KALAMAR_STAGE_DUMP").or_else(|| std::env::var_os("SPLAT3_STAGE_DUMP")).map(PathBuf::from);
    let dir = keep.clone().unwrap_or_else(|| std::env::temp_dir().join(format!("s3l-it-stages-{}", std::process::id())));
    let store = IconStore::new(dir.clone());
    let started = std::time::Instant::now();
    for s in &cat.stages {
        let langs: Vec<&str> = catalogue::LANGUAGES.iter().map(|(l, _)| *l).filter(|l| s.name.contains_key(*l)).collect();
        println!("{:>3} {:<18} listed {:<5} picture {:<5} names {}", s.id, s.scene, s.listed, s.image.is_some(), langs.join(","));
        assert_eq!(langs.len(), catalogue::LANGUAGES.len(), "{}: a name in every language", s.scene);
        let key = s.image.as_deref().unwrap_or_else(|| panic!("{}: no picture", s.scene));
        store.cache_sizes(&rom, key).unwrap_or_else(|e| panic!("{key}: {e}"));
        for (w, h) in [(480, 270), (160, 90)] {
            let png = store.cached(&icons::sized(key, w)).expect("cached");
            let size = (u32::from_be_bytes(png[16..20].try_into().unwrap()), u32::from_be_bytes(png[20..24].try_into().unwrap()));
            assert_eq!(size, (w, h), "{key}@{w}");
        }
    }
    println!("pictures decoded and scaled in {:?} into {}", started.elapsed(), dir.display());
    assert!(listed >= 20, "the stage lists of the game");
    let report = cat.report.iter().find(|r| r.category == "stage").expect("stage report");
    assert_eq!((report.with_icon, report.with_name), (report.selectable, report.selectable));
    if keep.is_none() {
        let _ = std::fs::remove_dir_all(dir);
    }
}
