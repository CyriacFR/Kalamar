//! Makes `resources/runtime-src.tar.gz` + `runtime-src.json`: the runtime source the install wizard compiles.
//!
//!   cargo run --example bundle_runtime -- --repo <runtime repo> --commit <hash> [--kit-tools <folder with s3_extract.py,
//!       default the repository's kit-tools/>] [--subdir <folder>] [--out <folder, default src-tauri/resources>]
//!
//! `git archive` of the commit named by `--commit` (tracked files only, so nothing ignored or local leaks in) — of its `<folder>` only with
//! `--subdir`, for a repository that holds the runtime in a folder (`engine`) next to the launcher — plus `s3_extract.py`,
//! `s3_merge.py`, `profile/s3save.py` and `profile/profile.example.json` under `kit-tools/`. Every file goes through
//! `installer::bundle::scan_entry`; the archive is not written if one fails. Developer tool, not part of the app.

use kalamar_lib::installer::bundle::{self, Manifest};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const KIT_TOOLS: &[&str] = &["s3_extract.py", "s3_merge.py", "profile/s3save.py", "profile/profile.example.json"];
const BINARY: &[&str] = &[".ico", ".png", ".jpg", ".ttf", ".otf", ".bmp", ".gif", ".webp"];

fn arg(name: &str) -> String {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1)).cloned().unwrap_or_else(|| panic!("missing {name}"))
}

fn git(repo: &str, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

fn main() {
    // The helper scripts live in this repository (`kit-tools/`), unless another folder is named.
    let kit = {
        let a: Vec<String> = std::env::args().collect();
        a.iter().position(|x| x == "--kit-tools").and_then(|i| a.get(i + 1)).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("kit-tools"))
    };
    let (repo, commit) = (arg("--repo"), arg("--commit"));
    let subdir = {
        let a: Vec<String> = std::env::args().collect();
        a.iter().position(|x| x == "--subdir").and_then(|i| a.get(i + 1)).cloned()
    };
    let full = String::from_utf8(git(&repo, &["rev-parse", &format!("{commit}^{{commit}}")])).unwrap().trim().to_string();
    // `<commit>:<folder>` names that folder's tree: the archive's paths are then relative to it.
    let tree = match &subdir {
        Some(d) => format!("{full}:{}", d.trim_matches('/')),
        None => full.clone(),
    };
    // The manifest names the snapshot by the id of that tree, not by the commit: the same sources have the same id in
    // every repository, whatever its history (a commit hash means nothing outside the repository it was made in).
    let tree_spec = if subdir.is_some() { tree.clone() } else { format!("{full}^{{tree}}") };
    let snapshot = String::from_utf8(git(&repo, &["rev-parse", "--verify", &tree_spec])).unwrap().trim().to_string();
    let tar_bytes = git(&repo, &["archive", "--format=tar", &tree]);

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut ar = tar::Archive::new(&tar_bytes[..]);
    for e in ar.entries().unwrap() {
        let mut e = e.unwrap();
        if !e.header().entry_type().is_file() {
            continue;
        }
        let p = e.path().unwrap().to_string_lossy().replace('\\', "/");
        let mut d = Vec::new();
        std::io::Read::read_to_end(&mut e, &mut d).unwrap();
        files.push((p, d));
    }
    let mut kit_tools = Vec::new();
    for rel in KIT_TOOLS {
        let d = std::fs::read(kit.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        kit_tools.push((format!("kit-tools/{rel}"), bundle::sha256_hex(&d)));
        files.push((format!("kit-tools/{rel}"), d));
    }

    let mut problems = Vec::new();
    for (p, d) in &files {
        for why in bundle::scan_entry(p, d) {
            problems.push(format!("{p}: {why}"));
        }
    }
    if !problems.is_empty() {
        eprintln!("REFUSED: {} problem(s)", problems.len());
        for p in &problems {
            eprintln!("  {p}");
        }
        std::process::exit(1);
    }

    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    {
        let mut b = tar::Builder::new(&mut gz);
        b.mode(tar::HeaderMode::Deterministic);
        for (p, d) in &files {
            let mut h = tar::Header::new_gnu();
            h.set_size(d.len() as u64);
            h.set_mode(0o644);
            h.set_mtime(0);
            h.set_cksum();
            b.append_data(&mut h, p, &d[..]).unwrap();
        }
        b.finish().unwrap();
    }
    let gz = gz.finish().unwrap();
    let reviewed = files
        .iter()
        .filter(|(p, _)| BINARY.iter().any(|s| p.to_ascii_lowercase().ends_with(s)))
        .map(|(p, d)| (p.clone(), bundle::sha256_hex(d)))
        .collect();
    let man = Manifest {
        commit: snapshot.clone(),
        files: files.len(),
        bytes: files.iter().map(|(_, d)| d.len() as u64).sum(),
        sha256: bundle::sha256_hex(&gz),
        kit_tools,
        reviewed_binaries: reviewed,
    };
    // `--out <folder>`: somewhere else than the launcher's resources (a test bundle of another commit).
    let out = {
        let a: Vec<String> = std::env::args().collect();
        a.iter().position(|x| x == "--out").and_then(|i| a.get(i + 1)).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources"))
    };
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join(bundle::ARCHIVE_NAME), &gz).unwrap();
    let mut f = std::fs::File::create(out.join(bundle::MANIFEST_NAME)).unwrap();
    f.write_all(serde_json::to_string_pretty(&man).unwrap().as_bytes()).unwrap();
    println!(
        "{}: {} files, {:.1} MB of source, archive {:.2} MB, engine snapshot {}, scan: 0 problems",
        bundle::ARCHIVE_NAME,
        man.files,
        man.bytes as f64 / 1e6,
        gz.len() as f64 / 1e6,
        &snapshot[..12]
    );
}
