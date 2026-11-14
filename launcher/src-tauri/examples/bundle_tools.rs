//! Makes `resources/kalamar-tools.tar.gz` + `kalamar-tools.json`: hactool and the shader translator, built from the
//! engine repository's sources, with their license texts (`installer::prebuilt`).
//!
//!   cargo run --example bundle_tools -- --engine <engine checkout> --hactool <dir: hactool.exe, LICENSE>
//!       --hactool-src <hactool source tree (its mbedtls/apache-2.0.txt)> --build-xlate <empty work folder>
//!       --dotnet-licenses <dir: LICENSE.TXT, THIRD-PARTY-NOTICES.TXT of microsoft.netcore.app.runtime.nativeaot.win-x64>
//!       [--out <folder, default src-tauri/resources>]
//!
//! `--hactool`: the output of `tools/build_hactool.py`, which applies `range.patch` (checked: the exe names the
//! variables). `--build-xlate`: the translator is built here, by the engine's `tools/build_xlate.py --out <folder>` run
//! with the MSBuild properties of a release (`RELEASE_PROPERTIES`: no debug information, so the NativeAOT link writes no
//! PDB path into the exe; deterministic; source paths mapped), passed through the environment so the script and the
//! translator's identity stay the same. `--xlate <dir>` takes an exe already built instead. Whatever the source, the
//! archive is not written when any of its files carries a trace of this machine (`prebuilt::machine_traces`: a
//! `Users\<name>` path, the user's name, the repositories' paths, in ASCII and UTF-16LE). The translator's identity is computed again by
//! the engine's own `tools/build_xlate.py --id-only` run over the files of the runtime source in `--out` (the archive the
//! launcher ships), and must equal the `.id` beside the exe: the translator shipped is the one of the source shipped.
//! Developer tool, not part of the app.

use kalamar_lib::installer::{bundle, prebuilt};
use std::path::{Path, PathBuf};
use std::process::Command;

fn arg(name: &str) -> String {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1)).cloned().unwrap_or_else(|| panic!("missing {name}"))
}

fn opt(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1)).cloned()
}

/// MSBuild properties of a translator that ships: no debug information (NativeAOT then links without `/DEBUG`: no
/// CodeView entry, no PDB path in the exe), a deterministic compile, and the build folders mapped to neutral names in
/// anything the C# compiler records. Environment variables are MSBuild properties, and the SDK only sets these when
/// they are empty.
const RELEASE_PROPERTIES: &[(&str, &str)] = &[("DebugType", "none"), ("DebugSymbols", "false"), ("Deterministic", "true")];

/// Builds the translator with the engine's own script into `work` and returns that folder.
fn build_xlate(engine: &Path, work: &Path) -> PathBuf {
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(work).unwrap();
    // Absolute, without the `\\?\` prefix `canonicalize` adds (dotnet does not take it).
    let work = std::path::absolute(work).unwrap();
    let strip = |p: &Path| p.display().to_string().trim_start_matches(r"\\?\").to_string();
    let mut cmd = Command::new("python");
    cmd.arg(engine.join("tools").join("build_xlate.py")).arg("--out").arg(&work);
    for (k, v) in RELEASE_PROPERTIES {
        cmd.env(k, v);
    }
    // MSVC's link.exe reads switches from LINK too: /Brepro stamps the exe and its debug directory entries with a hash of
    // the content instead of the time, so the same sources give the same exe (NativeAOT takes no extra switch otherwise).
    cmd.env("LINK", "/Brepro");
    // The NuGet packages go to the folder NUGET_PACKAGES names, else beside the build (not into the user's profile).
    if std::env::var_os("NUGET_PACKAGES").is_none() {
        cmd.env("NUGET_PACKAGES", work.join("nuget"));
    }
    cmd.env("PathMap", format!("{}=/_/build/,{}=/_/engine/", strip(&work), strip(&std::path::absolute(engine).unwrap())))
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("MSBUILDDISABLENODEREUSE", "1")
        .env("DOTNET_CLI_USE_MSBUILD_SERVER", "0")
        .env("UseSharedCompilation", "false");
    let st = cmd.status().expect("python");
    assert!(st.success(), "tools/build_xlate.py failed");
    work
}

fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn main() {
    let engine = PathBuf::from(arg("--engine"));
    let hactool = PathBuf::from(arg("--hactool"));
    let hactool_src = PathBuf::from(arg("--hactool-src"));
    let xlate = match (opt("--build-xlate"), opt("--xlate")) {
        (Some(work), _) => build_xlate(&engine, Path::new(&work)),
        (None, Some(dir)) => PathBuf::from(dir),
        (None, None) => panic!("missing --build-xlate <work folder> (or --xlate <dir>)"),
    };
    let dotnet = PathBuf::from(arg("--dotnet-licences"));
    let out = {
        let a: Vec<String> = std::env::args().collect();
        a.iter().position(|x| x == "--out").and_then(|i| a.get(i + 1)).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources"))
    };

    // The runtime source this pair goes with.
    let src = read(&out.join(bundle::ARCHIVE_NAME));
    let man = bundle::load_manifest(&out).expect("runtime-src.json");
    assert_eq!(bundle::sha256_hex(&src), man.sha256, "runtime-src.json does not match its archive");
    let files = bundle::read_archive(&src).expect("archive");
    let xlate_inputs = bundle::inputs_hash_archive(&files, bundle::XLATE_INPUTS);

    // The translator identity of that source, computed by the engine's own script over the archive's files.
    let tmp = std::env::temp_dir().join(format!("kalamar-tools-id-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    for (p, d) in files.iter().filter(|(p, _)| bundle::XLATE_INPUTS.iter().any(|w| if w.ends_with('/') { p.starts_with(w) } else { p == w })) {
        let t = tmp.join("src").join(p);
        std::fs::create_dir_all(t.parent().unwrap()).unwrap();
        std::fs::write(t, d).unwrap();
    }
    let id_out = tmp.join("id");
    std::fs::create_dir_all(&id_out).unwrap();
    let st =Command::new("python")
        .arg("-I")
        .arg(tmp.join("src").join("tools").join("build_xlate.py"))
        .args(["--id-only", "--out"])
        .arg(&id_out)
        .status()
        .expect("python");
    assert!(st.success(), "build_xlate.py --id-only failed");
    let want = String::from_utf8(read(&id_out.join("totk_xlate.id"))).unwrap().trim().to_string();
    let have = String::from_utf8(read(&xlate.join("totk_xlate.id"))).unwrap().trim().to_string();
    assert_eq!(have, want, "the translator in --xlate was not built from the shipped runtime source");
    let _ = std::fs::remove_dir_all(&tmp);

    let exe = read(&hactool.join("hactool.exe"));
    assert!(exe.windows(b"HACTOOL_INPUT_RANGE".len()).any(|w| w == b"HACTOOL_INPUT_RANGE"), "this hactool has no range.patch");
    let engine_commit = String::from_utf8(Command::new("git").arg("-C").arg(&engine).args(["rev-parse", "HEAD"]).output().expect("git").stdout).unwrap().trim().to_string();
    let patches: Vec<String> = ["portable.patch", "unicode.patch", "mbedtls-asm-clobbers.patch", "range.patch"]
        .iter()
        .map(|p| {
            let d = read(&engine.join("tools").join("hactool").join(p));
            format!("{p} {}", &bundle::sha256_hex(&d)[..12])
        })
        .collect();

    let entries: Vec<(String, Vec<u8>)> = vec![
        (prebuilt::HACTOOL.into(), exe),
        (prebuilt::XLATE_EXE.into(), read(&xlate.join("totk_xlate.exe"))),
        (prebuilt::XLATE_ID.into(), read(&xlate.join("totk_xlate.id"))),
        ("licenses/hactool-LICENSE.txt".into(), read(&hactool.join("LICENSE"))),
        ("licenses/mbedtls-LICENSE.txt".into(), read(&hactool_src.join("mbedtls").join("apache-2.0.txt"))),
        ("licenses/MacNativeGames-LICENSE.txt".into(), read(&engine.join("tools").join("vendor").join("macnativegames").join("LICENSE"))),
        ("licenses/Ryujinx-LICENSE.txt".into(), read(&engine.join("tools").join("vendor").join("ryujinx").join("LICENSE.txt"))),
        ("licenses/totk_xlate-LICENSE.txt".into(), read(&engine.join("tools").join("xlate").join("LICENSE"))),
        ("licenses/engine-LICENSE.txt".into(), read(&engine.join("LICENSE"))),
        ("licenses/dotnet-runtime-LICENSE.txt".into(), read(&dotnet.join("LICENSE.TXT"))),
        ("licenses/dotnet-runtime-THIRD-PARTY-NOTICES.txt".into(), read(&dotnet.join("THIRD-PARTY-NOTICES.TXT"))),
    ];
    let manifest = prebuilt::Manifest {
        engine_commit: engine_commit.clone(),
        xlate_inputs,
        xlate_id: want,
        hactool: format!("SciresM/hactool 1d64a83 + {}", patches.join(", ")),
        hactool_ranges: true,
        ..Default::default()
    };
    // Nothing of this machine may ship: no profile path, user name or repository path, in ASCII or UTF-16.
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").canonicalize().unwrap();
    let mut folders: Vec<PathBuf> = vec![repo.clone(), engine.canonicalize().unwrap_or(engine.clone())];
    folders.extend(repo.parent().map(Path::to_path_buf));
    let folder_refs: Vec<&Path> = folders.iter().map(PathBuf::as_path).collect();
    let mut refused = Vec::new();
    for (p, d) in &entries {
        for t in prebuilt::machine_traces(d, &prebuilt::current_user(), &folder_refs) {
            refused.push(format!("{p}: {t}"));
        }
    }
    if !refused.is_empty() {
        eprintln!("REFUSED: {} trace(s) of this machine in the files to ship", refused.len());
        for r in &refused {
            eprintln!("  {r}");
        }
        std::process::exit(1);
    }
    let (gz, m) = prebuilt::build(&entries, manifest);
    assert!(prebuilt::parse(&gz, &m).is_some());
    std::fs::write(out.join(prebuilt::ARCHIVE_NAME), &gz).unwrap();
    std::fs::write(out.join(prebuilt::MANIFEST_NAME), &m).unwrap();
    println!("{}: {} files, {:.2} MB, engine {}, translator {}", prebuilt::ARCHIVE_NAME, entries.len(), gz.len() as f64 / 1e6, &engine_commit[..12], &have[..16]);
}
