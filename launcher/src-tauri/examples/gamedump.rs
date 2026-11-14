//! Developer tool (not part of the app): inspect the user's own game data.
//!   gamedump <game root> survey                         texture formats of every UI/Icon file
//!   gamedump <game root> png <rom path> <tex|-> <out>   decode one texture to PNG
//!   gamedump <game root> layout <Layout name> <outdir>  decode every texture of a layout archive
use kalamar_lib::game::{bntx, romfs::RomFs, sarc::Sarc};
use std::collections::BTreeMap;
use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let rom = RomFs::open(Path::new(&a[1])).expect("romfs");
    match a[2].as_str() {
        "survey" => {
            let mut counts: BTreeMap<String, (usize, String)> = BTreeMap::new();
            let mut paths: Vec<&String> = rom.paths().filter(|p| p.starts_with("UI/Icon/") && p.ends_with(".bntx.zs")).collect();
            paths.sort();
            for p in &paths {
                let data = rom.read(p).unwrap();
                for t in bntx::parse(&data).unwrap() {
                    let k = format!("{} {}x{} mips{} layers{} bh{} chan{:?}", bntx::format_name(t.format), t.width, t.height, t.mips, t.layers, t.block_height_log2, t.channels);
                    let e = counts.entry(k).or_insert((0, p.to_string()));
                    e.0 += 1;
                }
            }
            for (k, (n, ex)) in &counts {
                println!("{n:5}  {k}   e.g. {ex}");
            }
            println!("{} files", paths.len());
        }
        "png" => {
            let data = rom.read(&a[3]).unwrap();
            let texs = bntx::parse(&data).unwrap();
            let t = if a[4] == "-" { &texs[0] } else { texs.iter().find(|t| t.name == a[4]).expect("texture") };
            let img = bntx::decode(&data, t).unwrap();
            std::fs::write(&a[5], bntx::encode_png(&img).unwrap()).unwrap();
        }
        "layout" => {
            let blarc = rom.read(&format!("Layout/{}.Nin_NX_NVN.blarc.zs", a[3])).unwrap();
            let s = Sarc::parse(&blarc).unwrap();
            let comb = s.find(&blarc, "timg/__Combined.bntx").expect("combined");
            std::fs::create_dir_all(&a[4]).unwrap();
            for t in bntx::parse(comb).unwrap() {
                match bntx::decode(comb, &t) {
                    Ok(img) => std::fs::write(Path::new(&a[4]).join(format!("{}.png", t.name.replace('^', "_"))), bntx::encode_png(&img).unwrap()).unwrap(),
                    Err(e) => eprintln!("{}: {e}", t.name),
                }
            }
        }
        "catalogue" => {
            let t = std::time::Instant::now();
            let c = kalamar_lib::game::catalogue::build(&rom);
            println!("built in {:?}", t.elapsed());
            for r in &c.report {
                println!("{:10} rows {:5} selectable {:5} icons {:5} names {:5}  {}", r.category, r.rows, r.selectable, r.with_icon, r.with_name, r.note);
            }
            if a.len() > 3 {
                std::fs::write(&a[3], serde_json::to_vec_pretty(&c).unwrap()).unwrap();
            }
        }
        "sheet" => {
            // gamedump <root> sheet <Layout name> <out.png> <name prefix filter|->  : contact sheet, 8 per row, 128px cells
            let blarc = rom.read(&format!("Layout/{}.Nin_NX_NVN.blarc.zs", a[3])).unwrap();
            let s = Sarc::parse(&blarc).unwrap();
            let comb = s.find(&blarc, "timg/__Combined.bntx").expect("combined");
            let mut texs = bntx::parse(comb).unwrap();
            if a[5] != "-" {
                texs.retain(|t| t.name.starts_with(a[5].as_str()));
            }
            let cell: usize = std::env::var("CELL").ok().and_then(|v| v.parse().ok()).unwrap_or(128);
            let cols: usize = std::env::var("COLS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
            let rows = texs.len().div_ceil(cols);
            let (w, h) = (cols * cell, rows * cell);
            let mut px = vec![90u8; w * h * 4];
            for p in px.chunks_exact_mut(4) {
                p[3] = 255;
            }
            for (i, t) in texs.iter().enumerate() {
                let Ok(img) = bntx::decode(comb, t) else { continue };
                let (cx, cy) = ((i % cols) * cell, (i / cols) * cell);
                let sc = (img.width.max(img.height) as f32 / cell as f32).max(1.0);
                for y in 0..cell {
                    for x in 0..cell {
                        let (sx, sy) = ((x as f32 * sc) as u32, (y as f32 * sc) as u32);
                        if sx >= img.width || sy >= img.height {
                            continue;
                        }
                        let sp = &img.pixels[((sy * img.width + sx) * 4) as usize..][..4];
                        let dp = &mut px[((cy + y) * w + cx + x) * 4..][..4];
                        let al = sp[3] as u32;
                        for k in 0..3 {
                            dp[k] = ((sp[k] as u32 * al + dp[k] as u32 * (255 - al)) / 255) as u8;
                        }
                    }
                }
                println!("{i}: {}", t.name);
            }
            let out = bntx::Rgba { width: w as u32, height: h as u32, pixels: px };
            std::fs::write(&a[4], bntx::encode_png(&out).unwrap()).unwrap();
        }
        "table" => {
            // gamedump <root> table <RSDB table name>  : every row of a parameter table
            let path = rom.paths().find(|p| p.starts_with(&format!("RSDB/{}.Product.", a[3])) && p.ends_with(".rstbl.byml.zs")).expect("table").clone();
            let doc = kalamar_lib::game::byml::parse(&rom.read(&path).unwrap()).unwrap();
            for row in doc.as_array().unwrap_or(&[]) {
                println!("{row:?}");
            }
        }
        "msg" => {
            // gamedump <root> msg <region, e.g. EUfr> <message file, e.g. CommonMsg/VS/VSStageName.msbt> : label = text
            let path = rom.paths().find(|p| p.starts_with(&format!("Mals/{}.Product.", a[3])) && p.ends_with(".sarc.zs")).expect("Mals").clone();
            let data = rom.read(&path).unwrap();
            let s = Sarc::parse(&data).unwrap();
            let m = kalamar_lib::game::msbt::parse(s.find(&data, &a[4]).expect("message file")).unwrap();
            let mut labels: Vec<&String> = m.messages.keys().collect();
            labels.sort();
            for label in labels {
                println!("{label} = {}", m.text(label).unwrap_or_default());
            }
        }
        "file" => {
            // gamedump <root> file <Layout name> <member path in the archive> <out file>
            let blarc = rom.read(&format!("Layout/{}.Nin_NX_NVN.blarc.zs", a[3])).unwrap();
            let s = Sarc::parse(&blarc).unwrap();
            std::fs::write(&a[5], s.find(&blarc, &a[4]).expect("member")).unwrap();
        }
        _ => eprintln!("unknown command"),
    }
}
