//! Prints what the install wizard sees on this PC: the tools it finds, the checks of three given files (codes only,
//! never a key value) and the plan for a folder. Developer tool, not part of the app.
//!
//!   cargo run --example wizard_probe -- <install dir> <base.nsp> <update.nsp> <prod.keys>

use kalamar_lib::installer::{self, InstallRequest, nsp, prereq};
use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let t = std::time::Instant::now();
    let det = prereq::detect();
    println!("detection in {:.1} s", t.elapsed().as_secs_f64());
    for tool in &det.tools {
        println!("  {:7} {}", tool.id, tool.path.as_deref().unwrap_or("MISSING"));
    }
    println!("  msys packages missing: {:?}", det.missing_packages);
    println!("  winget: {:?}", det.winget);
    if a.len() >= 4 {
        let show = |n: &str, c: nsp::FileCheck| println!("{n}: {} {} {:?} program {:?}", c.status, c.code, c.params, c.program_size);
        show("base", nsp::check_base(Path::new(&a[1])));
        show("update", nsp::check_update(Path::new(&a[2])));
        let k = nsp::check_keys(Path::new(&a[3]));
        println!("keys: {} {} (names checked: {:?})", k.status, k.code, k.params.iter().map(|p| &p.0).collect::<Vec<_>>());
        let req = InstallRequest { install_dir: a[0].clone(), base_nsp: a[1].clone(), update_nsp: a[2].clone(), prod_keys: a[3].clone(), ..Default::default() };
        let dir = installer::bundle::locate(None);
        let p = installer::plan(&req, dir.as_deref());
        println!("{}", serde_json::to_string_pretty(&p).unwrap());
    }
}
