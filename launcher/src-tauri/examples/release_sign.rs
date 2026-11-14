//! The maintainers' release tool: the release manifest and the same check the launcher makes. Step by step:
//! docs/releasing.md.
//!
//! ```text
//! cargo run --example release_sign -- manifest --repo owner/repo --version 0.2.0 --channel stable --notes notes.txt
//!                                              --portable Kalamar.exe [--installer Kalamar_0.2.0_x64-setup.exe] --out <folder>
//! cargo run --example release_sign -- verify   --dir <release folder> --repo owner/repo [--current 0.1.0] [--keys update-keys.txt]
//! ```
//!
//! The secret signing key is never handled here: keys are made and manifests signed with the official `minisign`
//! command-line tool (`minisign -G`, `minisign -S`; `scripts/make-release.ps1 -Minisign`). This example links the whole
//! launcher library and its dependencies: one compromised dependency in a process that decrypts the key could read it.

use kalamar_lib::update::manifest::{self, Channel, Expect};
use kalamar_lib::update::version::Version;
use std::path::{Path, PathBuf};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn need(args: &[String], name: &str) -> String {
    arg(args, name).unwrap_or_else(|| fail(&format!("missing {name}")))
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2)
}

fn sha256_file(p: &Path) -> String {
    kalamar_lib::update::apply::sha256_file(p).unwrap_or_else(|e| fail(&format!("{}: {e}", p.display())))
}

fn make_manifest(args: &[String]) {
    let repo = need(args, "--repo");
    if !manifest::valid_repo(&repo) {
        fail("--repo must be owner/repo");
    }
    let version = need(args, "--version");
    if Version::parse(&version).is_none() {
        fail("--version must be like 0.2.0 or 0.3.0-beta.1 (no v)");
    }
    let channel = arg(args, "--channel").unwrap_or_else(|| "stable".into());
    if Channel::parse(&channel).is_none() {
        fail("--channel is stable or beta");
    }
    let notes = match arg(args, "--notes") {
        // Windows editors often start a UTF-8 file with a byte-order mark: it is not part of the text
        Some(p) => std::fs::read_to_string(&p).unwrap_or_else(|e| fail(&format!("{p}: {e}"))).trim_start_matches('\u{feff}').to_string(),
        None => String::new(),
    };
    if notes.chars().count() > manifest::MAX_NOTES {
        eprintln!("warning: the notes are longer than {} characters: the launcher shows the beginning only", manifest::MAX_NOTES);
    }
    let mut files = Vec::new();
    for (flag, kind) in [("--portable", "portable"), ("--installer", "installer")] {
        let Some(p) = arg(args, flag) else { continue };
        let path = PathBuf::from(&p);
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if !manifest::valid_file_name(&name) || !name.to_ascii_lowercase().ends_with(".exe") {
            fail(&format!("{name}: a release file name is letters, digits, '.', '_' and '-' only, ending in .exe"));
        }
        let size = std::fs::metadata(&path).unwrap_or_else(|e| fail(&format!("{p}: {e}"))).len();
        files.push(serde_json::json!({"name": name, "kind": kind, "size": size, "sha256": sha256_file(&path)}));
    }
    if files.is_empty() {
        fail("give --portable and/or --installer");
    }
    let published = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let m = serde_json::json!({
        "format": manifest::FORMAT, "product": manifest::PRODUCT, "repo": repo, "version": version, "channel": channel,
        "published": published, "notes": notes.replace("\r\n", "\n"), "files": files,
    });
    let out = PathBuf::from(need(args, "--out")).join(manifest::MANIFEST_NAME);
    std::fs::write(&out, serde_json::to_vec_pretty(&m).unwrap_or_default()).unwrap_or_else(|e| fail(&format!("{}: {e}", out.display())));
    println!("{}", out.display());
}

fn verify(args: &[String]) {
    let dir = PathBuf::from(need(args, "--dir"));
    let repo = need(args, "--repo");
    let keys_file = arg(args, "--keys").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("update-keys.txt"));
    let text = std::fs::read_to_string(&keys_file).unwrap_or_else(|e| fail(&format!("{}: {e}", keys_file.display())));
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    let keys = manifest::parse_keys(&lines.join(";")).unwrap_or_else(|e| fail(&e));
    if keys.is_empty() {
        fail(&format!("{} holds no public key", keys_file.display()));
    }
    let m = std::fs::read(dir.join(manifest::MANIFEST_NAME)).unwrap_or_else(|e| fail(&format!("manifest: {e}")));
    let s = std::fs::read(dir.join(manifest::SIGNATURE_NAME)).unwrap_or_else(|e| fail(&format!("signature: {e}")));
    let current = Version::parse(&arg(args, "--current").unwrap_or_else(|| "0.0.0".into())).unwrap_or_else(|| fail("--current"));
    let channel = Channel::Beta; // the manifest's own channel is accepted; the launcher's stable channel refuses beta ones
    let got = manifest::verify(&m, &s, &keys, &Expect { repo: &repo, tag_version: None, current: &current, channel }).unwrap_or_else(|r| fail(&format!("the launcher would refuse this release: {r}")));
    for f in &got.files {
        let p = dir.join(&f.name);
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or_else(|e| fail(&format!("{}: {e}", p.display())));
        if size != f.size || sha256_file(&p) != f.sha256 {
            fail(&format!("{} does not match the manifest", f.name));
        }
        println!("ok  {}  {} bytes  {:?}", f.name, f.size, f.kind);
    }
    println!("ok  {} {} ({}), signature by a key of {}", manifest::MANIFEST_NAME, got.version, got.channel.as_str(), keys_file.display());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("manifest") => make_manifest(&args),
        Some("keygen" | "sign") => fail("keys are made and manifests signed with the official minisign tool (minisign -G, minisign -S; scripts/make-release.ps1 -Minisign): no program built from this repository reads the secret key (docs/releasing.md)"),
        Some("verify") => verify(&args),
        _ => {
            eprintln!("usage: release_sign manifest|verify ... (see the top of examples/release_sign.rs and docs/releasing.md)");
            std::process::exit(2);
        }
    }
}
