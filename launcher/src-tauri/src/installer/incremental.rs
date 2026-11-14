//! What an update of an installed game has to redo, decided from contents (never from file dates or commit names), and
//! how long that takes, said before it starts.
//!
//! An install keeps what its build made: the generated C (`work\<game>\lift`, about 1.5 GB) and the objects
//! (`build\obj`, about 0.4 GB), with Ninja's records of them. An update then redoes only what its inputs changed:
//!
//! * the generated C, when the code generator's sources differ (`bundle::LIFT_INPUTS`). `recomp/gen.py` rewrites only
//!   the chunks whose text changed; the others keep their dates, so Ninja leaves their objects alone;
//! * the compile, when a file that reaches the executable differs ([`compile_relevant`]). Ninja then recompiles the
//!   objects whose source, headers or command changed, and relinks;
//! * the save keys and the shader translator, from their own input digests.
//!
//! [`assess`] says beforehand what the compile will do, from the build's own records: the sources and headers each
//! object read (`build\.ninja_deps`) and how long it took (`build\.ninja_log`), against the files of the bundled source
//! that differ from the installed one ([`source_changes`]).

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

/// Folders of the runtime source whose files never reach the game executable: documentation and tests, the old Python
/// installer, the extraction and save scripts, the shader translator's sources and the vendored extraction tools.
const COMPILE_IGNORED_DIRS: &[&str] = &["docs/", "tests/", ".github/", ".githooks/", "installer/", "kit-tools/", "tools/xlate/", "tools/vendor/"];
/// Files that never reach it either (the translator's and hactool's build scripts have their own steps).
const COMPILE_IGNORED_FILES: &[&str] = &["license", "notice", ".gitignore", ".gitattributes", "config.example.toml", "tools/build_xlate.py", "tools/build_hactool.py"];

/// Whether a file of the runtime source (path relative to its root, `/`-separated) can change the game executable.
/// Anything not known to be unrelated counts: a wrong "yes" only costs a Ninja run that finds nothing to do.
pub fn compile_relevant(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    !(COMPILE_IGNORED_DIRS.iter().any(|d| p.starts_with(d)) || COMPILE_IGNORED_FILES.contains(&p.as_str()) || p.ends_with(".md") || p.ends_with(".example"))
}

/// Paths of an installed `src\` that never come from the archive: tools built or fetched there, Python caches, the
/// wizard's marker and the save keys the keys step writes.
fn local_only(rel: &str) -> bool {
    let p = rel.to_ascii_lowercase();
    ["toolchain/", "refs/", "build/", "generated/", ".venv/"].iter().any(|d| p.starts_with(d))
        || p.contains("__pycache__/")
        || p == super::bundle::MARKER_NAME.to_ascii_lowercase()
        || p == KEYS_JSON
}

/// The save keys file the keys step writes inside the source folder (`Layout::keys_json`).
pub const KEYS_JSON: &str = "kit-tools/profile/s3save.keys.json";

/// SHA-256 of every file of an archive, by path.
pub fn archive_hashes(files: &[(String, Vec<u8>)]) -> BTreeMap<String, String> {
    files.iter().map(|(p, d)| (p.clone(), super::bundle::sha256_hex(d))).collect()
}

/// SHA-256 of every file of an installed source folder that can come from the archive, by `/`-separated relative path.
pub fn installed_hashes(src: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![src.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(rel) = p.strip_prefix(src) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if !local_only(&format!("{rel}/")) {
                    stack.push(p);
                }
            } else if !local_only(&rel)
                && let Ok(data) = std::fs::read(&p)
            {
                out.insert(rel, super::bundle::sha256_hex(&data));
            }
        }
    }
    out
}

/// Paths added, removed or changed between two hash maps.
pub fn diff(old: &BTreeMap<String, String>, new: &BTreeMap<String, String>) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = new.iter().filter(|(p, h)| old.get(*p) != Some(*h)).map(|(p, _)| p.clone()).collect();
    out.extend(old.keys().filter(|p| !new.contains_key(*p)).cloned());
    out
}

/// Files of the bundled source (`archive`: path -> SHA-256) that differ from the source installed in `src`.
pub fn source_changes(archive: &BTreeMap<String, String>, src: &Path) -> BTreeSet<String> {
    diff(&installed_hashes(src), archive)
}

/// Digest of the files of an archive that can change the executable (path and content hash, sorted), plus the other
/// things the compile reads that are not in the source: the target and the identity of the RomFS index.
pub fn compile_key(archive: &BTreeMap<String, String>, target: &str, romfs_identity: &str) -> String {
    let mut text = String::new();
    for (p, h) in archive.iter().filter(|(p, _)| compile_relevant(p)) {
        text.push_str(&format!("{p}\0{h}\n"));
    }
    text.push_str(&format!("target\0{target}\nromfs\0{romfs_identity}\n"));
    super::bundle::sha256_hex(text.as_bytes())
}

/// The lines of `tools/build_game.py` that set a compile flag list (`GEN_FLAGS = [...]` up to its closing bracket): a
/// change there recompiles every object of that kind whatever its sources.
pub fn flag_block(text: &str, name: &str) -> Option<String> {
    let start = text.find(&format!("\n{name} = ["))? + 1;
    let rest = &text[start..];
    let end = rest.find(']')? + 1;
    Some(rest[..end].to_string())
}

// ---- the build's records ----------------------------------------------------------------------------------------

/// What Ninja recorded about the last build of an install.
#[derive(Debug, Default, Clone)]
pub struct NinjaRecords {
    /// Inputs (the source and every header it read) of each output, normalized with [`norm`].
    pub deps: HashMap<String, Vec<String>>,
    /// Seconds each output took the last time it was built, by normalized path.
    pub secs: HashMap<String, f64>,
}

/// A path as the records are compared: `/`-separated, lower case (Windows paths are not case-sensitive).
pub fn norm(p: &str) -> String {
    p.replace('\\', "/").to_ascii_lowercase()
}

/// `.ninja_log` (format v5 to v7): `start end mtime output hash` per line, times in milliseconds. The last line of an
/// output wins (a later build of the same object).
pub fn parse_ninja_log(text: &str) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for l in text.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 4 {
            continue;
        }
        if let (Ok(a), Ok(b)) = (f[0].parse::<u64>(), f[1].parse::<u64>())
            && b >= a
        {
            out.insert(norm(f[3]), (b - a) as f64 / 1000.0);
        }
    }
    out
}

/// `.ninja_deps` (format v3 and v4): path records (the path, padded to 4 bytes, then the one's complement of its id)
/// and dependency records (output id, its mtime, the input ids). The last record of an output wins; a truncated tail
/// (an interrupted build) ends the reading.
pub fn parse_ninja_deps(data: &[u8]) -> Option<HashMap<String, Vec<String>>> {
    let sig = b"# ninjadeps\n";
    if data.len() < sig.len() + 4 || !data.starts_with(sig) {
        return None;
    }
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let version = u32_at(data, sig.len());
    let mtime_len = match version {
        3 => 4,
        4 => 8,
        _ => return None,
    };
    let mut paths: Vec<String> = Vec::new();
    let mut deps: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut at = sig.len() + 4;
    while at + 4 <= data.len() {
        let head = u32_at(data, at);
        at += 4;
        let size = (head & 0x7FFF_FFFF) as usize;
        if !size.is_multiple_of(4) || at + size > data.len() {
            break;
        }
        let rec = &data[at..at + size];
        at += size;
        if head & 0x8000_0000 != 0 {
            if size < 4 + mtime_len {
                break;
            }
            let out = u32_at(rec, 0) as usize;
            let ins = rec[4 + mtime_len..].chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as usize).collect();
            deps.insert(out, ins);
        } else {
            if size < 4 {
                break;
            }
            let (name, check) = rec.split_at(size - 4);
            if !u32::from_le_bytes([check[0], check[1], check[2], check[3]]) as usize != paths.len() {
                break;
            }
            let end = name.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
            paths.push(norm(&String::from_utf8_lossy(&name[..end])));
        }
    }
    let name = |i: usize| paths.get(i).cloned();
    Some(deps.into_iter().filter_map(|(o, ins)| Some((name(o)?, ins.into_iter().filter_map(name).collect()))).collect())
}

/// The records of the build folder `build`, when both files are there.
pub fn load_ninja(build: &Path) -> Option<NinjaRecords> {
    let deps = parse_ninja_deps(&std::fs::read(build.join(".ninja_deps")).ok()?)?;
    let secs = parse_ninja_log(&std::fs::read_to_string(build.join(".ninja_log")).ok()?);
    (!deps.is_empty() && !secs.is_empty()).then_some(NinjaRecords { deps, secs })
}

/// Objects Ninja would rebuild by date right now: missing, or older than one of their recorded inputs (an update or a
/// compile that stopped half-way already replaced some sources). Relative inputs are below `src`.
pub fn stale_objects(n: &NinjaRecords, src: &Path) -> BTreeSet<String> {
    let mut seen: HashMap<&str, Option<std::time::SystemTime>> = HashMap::new();
    let mut out = BTreeSet::new();
    for (o, deps) in &n.deps {
        if !o.ends_with(".o") {
            continue;
        }
        let Some(t) = std::fs::metadata(o).and_then(|m| m.modified()).ok() else {
            out.insert(o.clone());
            continue;
        };
        let newer = deps.iter().any(|d| {
            let m = *seen.entry(d.as_str()).or_insert_with(|| {
                let p = Path::new(d);
                let p = if p.is_absolute() { p.to_path_buf() } else { src.join(p) };
                std::fs::metadata(p).and_then(|m| m.modified()).ok()
            });
            m.is_some_and(|m| m > t)
        });
        if newer {
            out.insert(o.clone());
        }
    }
    out
}

// ---- the decision -----------------------------------------------------------------------------------------------

/// What is known about an install and the bundled source before an update.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// Files of the bundled source that differ from the installed one (`/`-separated, relative to the source root).
    pub changed: BTreeSet<String>,
    /// Objects already out of date by Ninja's rule ([`stale_objects`]), normalized.
    pub stale: BTreeSet<String>,
    /// The generated C is complete and was made by the bundled generator.
    pub lift_valid: bool,
    /// The generated C is complete (whatever made it): a new lift then rewrites only the chunks that differ.
    pub lift_complete: bool,
    /// The executable was built from the bundled source (the compile step has nothing to do).
    pub compile_valid: bool,
    /// The objects and Ninja's records of them are there.
    pub objects: bool,
    /// "Rebuild everything" was asked.
    pub force: bool,
    /// The compile flags of the generated code / of the runtime differ (`tools/build_game.py`).
    pub gen_flags_changed: bool,
    pub rt_flags_changed: bool,
    /// The objects in place were compiled with another compiler (MSYS2 before the portable toolchain): every object
    /// is compiled again.
    pub toolchain_changed: bool,
    pub jobs: u32,
}

/// What the update will do to the generated C and the objects, and how long that takes.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UpdatePreview {
    /// `none` (nothing to rebuild), `runtime` (only the engine's own objects and the link), `generated` (some
    /// generated objects too, e.g. after a compile that stopped half-way), `generator` (the generated C is made again:
    /// only the chunks whose text changes are recompiled, unknown beforehand) or `full` (every object).
    pub kind: &'static str,
    /// Why every generated object is recompiled: `force`, `objects_missing`, `lift_missing`, `headers`, `flags`,
    /// `toolchain`.
    pub reasons: Vec<&'static str>,
    /// Files of the bundled source that differ from the installed one; those under `runtime/` and `recomp/`.
    pub changed_files: usize,
    pub changed_runtime: usize,
    pub changed_generator: usize,
    /// The generated C is made again.
    pub lift: bool,
    /// The compile step runs (Ninja, then the link when an object changed).
    pub compile: bool,
    /// Objects expected to be recompiled (of them, generated ones), and how many the build has.
    pub objects: usize,
    pub generated_objects: usize,
    pub objects_total: usize,
    /// Estimated seconds of the lift, of the compile, and of the compile when every generated chunk changes.
    pub lift_secs: f64,
    pub compile_secs: f64,
    pub compile_secs_max: f64,
}

/// Seconds the compile step spends besides compiling: Python and Ninja start, the RomFS index, checking the objects'
/// hashes (`build_game.py verify_objects`).
pub const COMPILE_OVERHEAD_SECS: f64 = 12.0;
/// The link when Ninja has no record of it.
pub const LINK_SECS: f64 = 3.0;
/// Generated objects of a complete build (main and sdk), for the estimate without records.
pub const GENERATED_OBJECTS: usize = 597;

/// Kind of an object of the build: generated (`obj/<module>/`) or the runtime's (`obj/runtime/`), from its normalized
/// path and the normalized `obj/` prefix of the build.
fn object_kind(path: &str, obj_prefix: &str) -> Option<bool> {
    let rest = path.strip_prefix(obj_prefix)?;
    if !rest.ends_with(".o") {
        return None;
    }
    Some(!rest.starts_with("runtime/"))
}

/// Wall time of compiling objects that took `secs` each, `jobs` at a time: the work shared out, never less than the
/// longest one.
pub fn wall(secs: &[f64], jobs: u32) -> f64 {
    let sum: f64 = secs.iter().sum();
    let longest = secs.iter().copied().fold(0.0, f64::max);
    (sum / jobs.max(1) as f64).max(longest)
}

/// The update's effect on the build. `src_prefix` and `obj_prefix` are the normalized source root and `build\obj` of
/// the install, each ending in `/` (Ninja records sources below the root relative to it, everything else absolute).
/// `lift_ref` and `full_ref` are the reference durations of a lift and of a full compile on this PC, used where the
/// records say nothing.
pub fn assess(f: &Facts, ninja: Option<&NinjaRecords>, src_prefix: &str, obj_prefix: &str, lift_ref: f64, full_ref: f64) -> UpdatePreview {
    let changed_runtime = f.changed.iter().filter(|p| p.starts_with("runtime/")).count();
    let changed_generator = f.changed.iter().filter(|p| p.starts_with("recomp/")).count();
    let mut p = UpdatePreview {
        kind: "none",
        reasons: Vec::new(),
        changed_files: f.changed.len(),
        changed_runtime,
        changed_generator,
        lift: false,
        compile: false,
        objects: 0,
        generated_objects: 0,
        objects_total: 0,
        lift_secs: 0.0,
        compile_secs: 0.0,
        compile_secs_max: 0.0,
    };
    if f.compile_valid && !f.force {
        return p;
    }
    p.compile = true;
    p.lift = f.force || !f.lift_valid;
    if p.lift {
        p.lift_secs = lift_ref;
    }
    let changed: BTreeSet<String> = f.changed.iter().map(|c| norm(c)).collect();
    // A recorded input, as a path relative to the source root when it is below it.
    let rel = |dep: &str| dep.strip_prefix(src_prefix).unwrap_or(dep).to_string();
    let objects: Vec<(&String, bool)> =
        ninja.map(|n| n.deps.keys().filter_map(|o| object_kind(o, obj_prefix).map(|g| (o, g))).collect()).unwrap_or_default();
    let have_records = f.objects && !objects.is_empty();

    if f.force {
        p.reasons.push("force");
    }
    if !f.objects {
        p.reasons.push("objects_missing");
    }
    if p.lift && !f.lift_complete {
        p.reasons.push("lift_missing");
    }
    if f.gen_flags_changed {
        p.reasons.push("flags");
    }
    if f.toolchain_changed {
        p.reasons.push("toolchain");
    }
    // Objects whose source or headers changed, by kind, with the seconds each took last time.
    let (mut gen_secs, mut gen_all, mut rt_secs, mut rt_known) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut headers = false;
    let secs_of = |o: &str| ninja.and_then(|n| n.secs.get(o).copied());
    let mut sources = BTreeSet::new();
    for (o, generated) in &objects {
        let deps: &[String] = ninja.and_then(|n| n.deps.get(*o)).map(Vec::as_slice).unwrap_or(&[]);
        if let Some(s) = deps.first() {
            sources.insert(rel(s));
        }
        let s = secs_of(o).unwrap_or(0.0);
        let by_content = deps.iter().any(|d| changed.contains(&rel(d)));
        let hit = by_content || f.stale.contains(*o);
        if *generated {
            gen_all.push(s);
            if hit {
                gen_secs.push(s);
                headers |= by_content;
            }
        } else {
            rt_known.push(s);
            if hit || f.rt_flags_changed || f.toolchain_changed {
                rt_secs.push(s);
            }
        }
    }
    if headers {
        p.reasons.push("headers");
    }
    // Runtime sources the records do not know (new files) compile too.
    let median = {
        let mut v = rt_known.clone();
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(2.0)
    };
    let is_source = |c: &String| c.starts_with("runtime/") && [".c", ".cpp", ".rc"].iter().any(|x| c.ends_with(x));
    let new_sources = changed.iter().filter(|c| is_source(c) && !sources.contains(*c)).count();
    rt_secs.extend(std::iter::repeat_n(median, new_sources));
    let link = ninja.and_then(|n| n.secs.iter().filter(|(o, _)| o.ends_with(".exe")).map(|(_, s)| *s).reduce(f64::max)).unwrap_or(LINK_SECS);

    let every_generated = !have_records || f.force || (p.lift && !f.lift_complete) || f.gen_flags_changed || f.toolchain_changed;
    let full_gen = if have_records && !gen_all.is_empty() { wall(&gen_all, f.jobs) } else { full_ref };
    let full_objects = if have_records && !gen_all.is_empty() { gen_all.len() } else { GENERATED_OBJECTS };
    p.objects_total = if have_records { objects.len() } else { 0 };
    let (gen_count, gen_wall) = if every_generated {
        (full_objects, full_gen)
    } else {
        (gen_secs.len(), wall(&gen_secs, f.jobs))
    };
    // Generated and runtime objects compile in the same Ninja run, in parallel.
    let together = |g: f64, rt: &[f64]| {
        let rt_wall = if rt.is_empty() { 0.0 } else { wall(rt, f.jobs) };
        if g > 0.0 { g + rt.iter().sum::<f64>() / f.jobs.max(1) as f64 } else { rt_wall }
    };
    p.generated_objects = gen_count;
    p.objects = gen_count + if every_generated && !have_records { 0 } else { rt_secs.len() };
    p.compile_secs = together(gen_wall, &rt_secs) + link + COMPILE_OVERHEAD_SECS;
    p.compile_secs_max = if p.lift { together(full_gen, &rt_secs) + link + COMPILE_OVERHEAD_SECS } else { p.compile_secs };
    p.kind = if every_generated || (gen_count > 0 && gen_count == gen_all.len()) {
        "full"
    } else if p.lift {
        "generator"
    } else if gen_count > 0 {
        "generated"
    } else {
        "runtime"
    };
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    const SRC: &str = "c:/i/src/";
    const OBJ: &str = "c:/i/work/g/build/obj/";

    /// A build with two generated chunks (reading cpu.h) and two runtime objects (vk.c reads vk.h; main.c reads cpu.h).
    fn records() -> NinjaRecords {
        let mut n = NinjaRecords::default();
        let g = "c:/i/work/g/lift/main/";
        n.deps.insert(format!("{OBJ}main/main_0000.o"), vec![format!("{g}main_0000.c"), "runtime/cpu.h".into(), "runtime/simd.h".into()]);
        n.deps.insert(format!("{OBJ}main/main_0001.o"), vec![format!("{g}main_0001.c"), "runtime/cpu.h".into(), "runtime/simd.h".into()]);
        n.deps.insert(format!("{OBJ}runtime/vk.o"), vec!["runtime/vk.c".into(), "runtime/vk.h".into(), "c:/msys64/ucrt64/include/stdio.h".into()]);
        n.deps.insert(format!("{OBJ}runtime/main.o"), vec!["runtime/main.c".into(), "runtime/cpu.h".into()]);
        for (o, s) in [("main/main_0000.o", 600.0), ("main/main_0001.o", 400.0), ("runtime/vk.o", 20.0), ("runtime/main.o", 4.0)] {
            n.secs.insert(format!("{OBJ}{o}"), s);
        }
        n.secs.insert("c:/i/work/g/build/splat3.exe".into(), 2.5);
        n
    }

    fn facts(changed: &[&str]) -> Facts {
        Facts { changed: set(changed), lift_valid: true, lift_complete: true, objects: true, jobs: 2, ..Default::default() }
    }

    #[test]
    fn nothing_to_rebuild_when_the_executable_matches() {
        let p = assess(&Facts { compile_valid: true, ..facts(&["docs/a.md"]) }, Some(&records()), SRC, OBJ, 200.0, 1500.0);
        assert_eq!((p.kind, p.compile, p.lift, p.objects), ("none", false, false, 0));
        assert_eq!(p.compile_secs, 0.0);
    }

    #[test]
    fn a_runtime_source_recompiles_its_object_and_links() {
        let p = assess(&facts(&["runtime/vk.c"]), Some(&records()), SRC, OBJ, 200.0, 1500.0);
        assert_eq!((p.kind, p.lift, p.objects, p.generated_objects, p.objects_total), ("runtime", false, 1, 0, 4));
        assert!((p.compile_secs - (20.0 + 2.5 + COMPILE_OVERHEAD_SECS)).abs() < 1e-9, "{}", p.compile_secs);
        // A header only runtime objects read: the same.
        let p = assess(&facts(&["runtime/vk.h"]), Some(&records()), SRC, OBJ, 200.0, 1500.0);
        assert_eq!((p.kind, p.objects), ("runtime", 1));
        // A new runtime file compiles too (median of the known runtime objects).
        let p = assess(&facts(&["runtime/new.c"]), Some(&records()), SRC, OBJ, 200.0, 1500.0);
        assert_eq!((p.kind, p.objects), ("runtime", 1));
    }

    #[test]
    fn a_header_of_the_generated_code_recompiles_it_all() {
        let p = assess(&facts(&["runtime/cpu.h"]), Some(&records()), SRC, OBJ, 200.0, 1500.0);
        assert_eq!((p.kind, p.reasons.as_slice(), p.objects, p.generated_objects), ("full", &["headers"][..], 3, 2));
        // 1000 s of generated objects on 2 jobs (the longest is 600), plus main.o shared out, the link and the overhead.
        assert!((p.compile_secs - (600.0 + 2.0 + 2.5 + COMPILE_OVERHEAD_SECS)).abs() < 1e-9, "{}", p.compile_secs);
    }

    #[test]
    fn a_generator_change_regenerates_the_c_and_gives_a_range() {
        let f = Facts { lift_valid: false, ..facts(&["recomp/lifter.py", "runtime/vk.c"]) };
        let p = assess(&f, Some(&records()), SRC, OBJ, 200.0, 1500.0);
        assert_eq!((p.kind, p.lift, p.changed_generator, p.changed_runtime), ("generator", true, 1, 1));
        assert_eq!(p.lift_secs, 200.0);
        assert!(p.compile_secs < 60.0 && p.compile_secs_max > 600.0, "{p:?}");
    }

    #[test]
    fn objects_already_out_of_date_count_too() {
        // A compile stopped half-way: the source is already the new one (nothing differs) but one chunk is stale.
        let f = Facts { stale: set(&[&format!("{OBJ}main/main_0001.o")]), ..facts(&[]) };
        let p = assess(&f, Some(&records()), SRC, OBJ, 200.0, 1500.0);
        assert_eq!((p.kind, p.objects, p.generated_objects), ("generated", 1, 1));
        assert!(p.reasons.is_empty());
    }

    #[test]
    fn missing_objects_or_c_mean_a_full_build() {
        let p = assess(&Facts { objects: false, ..facts(&["runtime/vk.c"]) }, None, SRC, OBJ, 200.0, 1500.0);
        assert_eq!((p.kind, p.reasons.as_slice()), ("full", &["objects_missing"][..]));
        assert_eq!(p.generated_objects, GENERATED_OBJECTS);
        assert!(p.compile_secs >= 1500.0);
        let f = Facts { lift_valid: false, lift_complete: false, ..facts(&[]) };
        let p = assess(&f, Some(&records()), SRC, OBJ, 200.0, 1500.0);
        assert_eq!((p.kind, p.lift, p.reasons.as_slice()), ("full", true, &["lift_missing"][..]));
        let p = assess(&Facts { force: true, compile_valid: true, ..facts(&[]) }, Some(&records()), SRC, OBJ, 200.0, 1500.0);
        assert_eq!((p.kind, p.lift), ("full", true));
        assert!(p.reasons.contains(&"force"));
        let p = assess(&Facts { gen_flags_changed: true, ..facts(&["tools/build_game.py"]) }, Some(&records()), SRC, OBJ, 200.0, 1500.0);
        assert_eq!((p.kind, p.reasons.as_slice()), ("full", &["flags"][..]));
        // Objects of another compiler: everything, runtime included, with nothing else changed.
        let p = assess(&Facts { toolchain_changed: true, ..facts(&[]) }, Some(&records()), SRC, OBJ, 200.0, 1500.0);
        assert_eq!((p.kind, p.compile, p.lift, p.reasons.as_slice()), ("full", true, false, &["toolchain"][..]));
        assert!(p.objects > p.generated_objects, "runtime objects too");
    }

    #[test]
    fn only_files_that_reach_the_executable_count() {
        for p in ["runtime/vk.c", "recomp/gen.py", "tools/build_game.py", "tools/romfs_index.py", "VERSION", "tools/data/expected-v1.2.1.json"] {
            assert!(compile_relevant(p), "{p}");
        }
        for p in ["docs/building.md", "README.md", "tests/x.py", "kit-tools/profile/s3save.py", "tools/xlate/a.cs", "tools/vendor/ryujinx/x.cs", "installer/gui.py", "LICENSE", "totk.ini.example"] {
            assert!(!compile_relevant(p), "{p}");
        }
        let a: BTreeMap<String, String> = [("runtime/a.c", "1"), ("docs/x.md", "2")].iter().map(|(p, h)| (p.to_string(), h.to_string())).collect();
        let mut b = a.clone();
        b.insert("docs/x.md".into(), "3".into());
        assert_eq!(compile_key(&a, "t", "r"), compile_key(&b, "t", "r"), "a documentation change keeps the key");
        b.insert("runtime/a.c".into(), "4".into());
        assert_ne!(compile_key(&a, "t", "r"), compile_key(&b, "t", "r"));
        assert_ne!(compile_key(&a, "t", "r"), compile_key(&a, "t", "r2"), "another RomFS index rebuilds");
    }

    #[test]
    fn changes_between_the_archive_and_the_installed_source() {
        let root = std::env::temp_dir().join(format!("s3l-incr-{}", crate::util::random_hex(4)));
        let w = |rel: &str, data: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, data).unwrap();
        };
        w("runtime/a.c", "same");
        w("runtime/b.c", "old");
        w("runtime/gone.c", "x");
        w("toolchain/hactool/hactool.exe", "tool");
        w("recomp/__pycache__/gen.cpython-313.pyc", "cache");
        w(KEYS_JSON, "{}");
        w(super::super::bundle::MARKER_NAME, "{}");
        let files = vec![
            ("runtime/a.c".to_string(), b"same".to_vec()),
            ("runtime/b.c".to_string(), b"new".to_vec()),
            ("runtime/added.h".to_string(), b"h".to_vec()),
        ];
        let got = source_changes(&archive_hashes(&files), &root);
        assert_eq!(got, set(&["runtime/added.h", "runtime/b.c", "runtime/gone.c"]));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reads_ninja_records() {
        let log = "# ninja log v7\n53\t14438\t816\tC:/X/build/obj/main/main_0002.o\tab\n10\t20\t1\tC:/X/build/obj/main/main_0002.o\tcd\n";
        let secs = parse_ninja_log(log);
        assert_eq!(secs.get("c:/x/build/obj/main/main_0002.o"), Some(&0.01), "the last line of an output wins");
        // A v4 deps log: two paths, one deps record, then a truncated record.
        let mut d = b"# ninjadeps\n".to_vec();
        d.extend(4u32.to_le_bytes());
        let path = |d: &mut Vec<u8>, s: &str, id: u32| {
            let mut b = s.as_bytes().to_vec();
            while !b.len().is_multiple_of(4) {
                b.push(0);
            }
            d.extend(((b.len() + 4) as u32).to_le_bytes());
            d.extend(&b);
            d.extend((!id).to_le_bytes());
        };
        path(&mut d, "C:/X/build/obj/runtime/vk.o", 0);
        path(&mut d, "runtime/vk.c", 1);
        path(&mut d, "runtime/vk.h", 2);
        d.extend((0x8000_0000u32 | 20).to_le_bytes());
        d.extend(0u32.to_le_bytes());
        d.extend(7u64.to_le_bytes());
        d.extend(1u32.to_le_bytes());
        d.extend(2u32.to_le_bytes());
        d.extend(12u32.to_le_bytes());
        d.extend(b"trun");
        let deps = parse_ninja_deps(&d).unwrap();
        assert_eq!(deps["c:/x/build/obj/runtime/vk.o"], vec!["runtime/vk.c".to_string(), "runtime/vk.h".to_string()]);
        assert!(parse_ninja_deps(b"# ninjalog\n\0\0\0\0").is_none());
    }

    #[test]
    fn stale_objects_follow_ninjas_rule() {
        let root = std::env::temp_dir().join(format!("s3l-stale-{}", crate::util::random_hex(4)));
        let src = root.join("src");
        std::fs::create_dir_all(src.join("runtime")).unwrap();
        std::fs::create_dir_all(root.join("obj")).unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let at = |p: &Path, t: std::time::SystemTime| std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
        for f in ["runtime/a.c", "runtime/b.c", "runtime/h.h"] {
            std::fs::write(src.join(f), "x").unwrap();
            at(&src.join(f), old);
        }
        let obj = |n: &str| norm(&root.join("obj").join(n).display().to_string());
        std::fs::write(root.join("obj").join("a.o"), "o").unwrap();
        std::fs::write(root.join("obj").join("b.o"), "o").unwrap();
        let mut n = NinjaRecords::default();
        n.deps.insert(obj("a.o"), vec!["runtime/a.c".into(), "runtime/h.h".into()]);
        n.deps.insert(obj("b.o"), vec!["runtime/b.c".into()]);
        n.deps.insert(obj("gone.o"), vec!["runtime/b.c".into()]);
        // Everything older than its object, except an object that is missing.
        assert_eq!(stale_objects(&n, &src), set(&[&obj("gone.o")]));
        // A header written after the object (an extraction that stopped half-way, a cancelled compile).
        std::fs::write(src.join("runtime/h.h"), "y").unwrap();
        at(&src.join("runtime/h.h"), std::time::SystemTime::now() + std::time::Duration::from_secs(5));
        assert_eq!(stale_objects(&n, &src), set(&[&obj("a.o"), &obj("gone.o")]));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn flag_blocks_of_build_game() {
        let t = "x = 1\nGEN_FLAGS = ['-O1',\n  '-march=x86-64-v3']\nRT_FLAGS = ['-O2']\n";
        assert_eq!(flag_block(t, "GEN_FLAGS").unwrap(), "GEN_FLAGS = ['-O1',\n  '-march=x86-64-v3']");
        assert_eq!(flag_block(t, "RT_FLAGS").unwrap(), "RT_FLAGS = ['-O2']");
        assert!(flag_block(t, "LINK_FLAGS").is_none());
    }

    #[test]
    fn wall_time_is_shared_out_but_never_below_the_longest() {
        assert_eq!(wall(&[10.0, 10.0, 10.0, 10.0], 2), 20.0);
        assert_eq!(wall(&[100.0, 1.0], 8), 100.0);
        assert_eq!(wall(&[], 4), 0.0);
    }
}
