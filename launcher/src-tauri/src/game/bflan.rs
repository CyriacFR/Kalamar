//! BFLAN layout animations (`anim/*.bflan` inside a layout archive): just enough to read keyframe tracks.
//!
//! Layout: `FLAN` header, then sections; `pai1` holds the animation contents (one per pane or material), each with
//! tags (`FLMC` material color, `FLTS` texture SRT, `FLVI` visibility, ...), each tag with targets carrying keys.
//! Keys are Hermite triples (frame, value, slope) or step pairs (frame, u16 value).
//!
//! Contents, tags and targets are reached through offset tables, so a crafted file can point every entry of every table
//! at the same target. Each track and key read is charged to a budget of twice the file's size (a sound file stores each
//! of them once: 16 bytes or more per track, 8 or 12 per key).

use super::guard::Budget;

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Pane or material name the content animates.
    pub name: String,
    pub tag: [u8; 4],
    /// Index inside the tag (texture SRT slot for `FLTS`, 0 otherwise).
    pub index: u8,
    /// Animated component (`FLMC`: 0..3 black color RGBA, 4..7 white color RGBA).
    pub target: u8,
    /// (frame, value), in file order.
    pub keys: Vec<(f32, f32)>,
}

fn rd<const N: usize>(b: &[u8], o: usize) -> Result<[u8; N], String> {
    o.checked_add(N).and_then(|e| b.get(o..e)).map(|s| s.try_into().unwrap()).ok_or_else(|| "BFLAN: truncated".to_string())
}
fn u16at(b: &[u8], o: usize) -> Result<usize, String> {
    Ok(u16::from_le_bytes(rd(b, o)?) as usize)
}
fn u32at(b: &[u8], o: usize) -> Result<usize, String> {
    Ok(u32::from_le_bytes(rd(b, o)?) as usize)
}
fn f32at(b: &[u8], o: usize) -> Result<f32, String> {
    Ok(f32::from_le_bytes(rd(b, o)?))
}

/// Every keyframe track of the `pai1` section (little-endian files only, as on the Switch).
pub fn parse(b: &[u8]) -> Result<Vec<Track>, String> {
    if b.get(..4) != Some(b"FLAN") || b.get(4..6) != Some(&[0xFF, 0xFE]) {
        return Err("BFLAN: bad magic or byte order".into());
    }
    let mut o = u16at(b, 6)?;
    let sections = u16at(b, 0x10)?;
    let mut out = Vec::new();
    // The 3,512 animations of 11.2.0's layouts stay under 1.1 times their size this way.
    let mut budget = Budget::new("BFLAN", b.len(), 2, 0, usize::MAX);
    for _ in 0..sections {
        let size = u32at(b, o + 4)?;
        if size < 8 {
            return Err("BFLAN: bad section size".into());
        }
        if rd::<4>(b, o)? == *b"pai1" {
            let contents = u16at(b, o + 0xE)?;
            let table = o + u32at(b, o + 0x10)?;
            for i in 0..contents.min(4096) {
                let c = o + u32at(b, table + 4 * i)?;
                let raw = b.get(c..c + 28).ok_or("BFLAN: truncated name")?;
                let name = String::from_utf8_lossy(&raw[..raw.iter().position(|&x| x == 0).unwrap_or(28)]).into_owned();
                let tags = *b.get(c + 28).ok_or("BFLAN: truncated")? as usize;
                for j in 0..tags {
                    let t = c + u32at(b, c + 32 + 4 * j)?;
                    let tag: [u8; 4] = rd(b, t)?;
                    let entries = *b.get(t + 4).ok_or("BFLAN: truncated")? as usize;
                    for k in 0..entries {
                        let e = t + u32at(b, t + 8 + 4 * k)?;
                        let [index, target, curve, _] = rd::<4>(b, e)?;
                        let n = u16at(b, e + 4)?.min(4096);
                        budget.take(16 + n * if curve == 2 { 12 } else { 8 })?;
                        let first = e + u32at(b, e + 8)?;
                        let mut keys = Vec::with_capacity(n);
                        for q in 0..n {
                            keys.push(match curve {
                                2 => (f32at(b, first + 12 * q)?, f32at(b, first + 12 * q + 4)?),
                                _ => (f32at(b, first + 8 * q)?, u16at(b, first + 8 * q + 4)? as f32),
                            });
                        }
                        out.push(Track { name: name.clone(), tag, index, target, keys });
                    }
                }
            }
        }
        o += size;
    }
    Ok(out)
}

/// The value a track holds at `frame`, for animations made of held values (the color tables these layouts encode
/// as one key pair per frame): the last key at or before the frame, else the first key.
pub fn value_at(keys: &[(f32, f32)], frame: f32) -> Option<f32> {
    keys.iter().rev().find(|(f, _)| *f <= frame).or(keys.first()).map(|&(_, v)| v)
}

#[cfg(test)]
pub mod build {
    /// One animated target: its tag, index, target and keys.
    pub type Track = ([u8; 4], u8, u8, Vec<(f32, f32)>);

    /// Writes a minimal little-endian BFLAN with one `pai1` section: (content name, [(tag, index, target, keys)]).
    /// Keys are written as Hermite triples with a zero slope.
    pub fn encode(contents: &[(&str, Vec<Track>)]) -> Vec<u8> {
        let mut blocks: Vec<Vec<u8>> = Vec::new();
        for (name, tracks) in contents {
            // group targets by tag, keeping order
            type Target = ([u8; 4], u8, u8, Vec<(f32, f32)>);
            let mut tags: Vec<([u8; 4], Vec<&Target>)> = Vec::new();
            for t in tracks {
                match tags.iter_mut().find(|(m, _)| *m == t.0) {
                    Some((_, v)) => v.push(t),
                    None => tags.push((t.0, vec![t])),
                }
            }
            let mut c = vec![0u8; 28];
            c[..name.len()].copy_from_slice(name.as_bytes());
            c.extend_from_slice(&[tags.len() as u8, 1, 0, 0]);
            let table_at = c.len();
            c.resize(table_at + 4 * tags.len(), 0);
            for (j, (magic, targets)) in tags.iter().enumerate() {
                let t = c.len();
                c[table_at + 4 * j..table_at + 4 * j + 4].copy_from_slice(&(t as u32).to_le_bytes());
                c.extend_from_slice(magic);
                c.extend_from_slice(&[targets.len() as u8, 0, 0, 0]);
                let offs_at = c.len();
                c.resize(offs_at + 4 * targets.len(), 0);
                for (k, (_, index, target, keys)) in targets.iter().enumerate() {
                    let e = c.len();
                    c[offs_at + 4 * k..offs_at + 4 * k + 4].copy_from_slice(&((e - t) as u32).to_le_bytes());
                    c.extend_from_slice(&[*index, *target, 2, 0]);
                    c.extend_from_slice(&(keys.len() as u16).to_le_bytes());
                    c.extend_from_slice(&[0, 0]);
                    c.extend_from_slice(&12u32.to_le_bytes());
                    for (f, v) in keys {
                        c.extend_from_slice(&f.to_le_bytes());
                        c.extend_from_slice(&v.to_le_bytes());
                        c.extend_from_slice(&0f32.to_le_bytes());
                    }
                }
            }
            blocks.push(c);
        }
        let mut pai = Vec::new();
        pai.extend_from_slice(b"pai1");
        pai.extend_from_slice(&[0; 4]); // size, patched below
        pai.extend_from_slice(&100u16.to_le_bytes());
        pai.extend_from_slice(&[0, 0, 0, 0]);
        pai.extend_from_slice(&(blocks.len() as u16).to_le_bytes());
        pai.extend_from_slice(&0x14u32.to_le_bytes());
        let mut at = 0x14 + 4 * blocks.len();
        for blk in &blocks {
            pai.extend_from_slice(&(at as u32).to_le_bytes());
            at += blk.len();
        }
        for blk in &blocks {
            pai.extend_from_slice(blk);
        }
        let len = pai.len() as u32;
        pai[4..8].copy_from_slice(&len.to_le_bytes());
        let mut f = Vec::new();
        f.extend_from_slice(b"FLAN");
        f.extend_from_slice(&[0xFF, 0xFE]);
        f.extend_from_slice(&0x14u16.to_le_bytes());
        f.extend_from_slice(&0x0900_0000u32.to_le_bytes());
        f.extend_from_slice(&((0x14 + pai.len()) as u32).to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(&pai);
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tracks_and_held_values() {
        let keys = vec![(0.0, 10.0), (1.0, 10.0), (1.0, 20.0), (2.0, 20.0), (2.0, 30.0)];
        let file = build::encode(&[
            ("P_A", vec![(*b"FLMC", 0, 4, keys.clone()), (*b"FLMC", 0, 5, vec![(0.0, 7.0)]), (*b"FLTS", 1, 0, vec![(0.0, 1.5), (5.0, 0.5)])]),
            ("P_B", vec![(*b"FLVI", 0, 0, vec![(0.0, 1.0)])]),
        ]);
        let t = parse(&file).unwrap();
        assert_eq!(t.len(), 4);
        assert_eq!((t[0].name.as_str(), &t[0].tag, t[0].target), ("P_A", b"FLMC", 4));
        assert_eq!(t[0].keys, keys);
        assert_eq!((t[2].tag, t[2].index), (*b"FLTS", 1));
        assert_eq!(t[3].name, "P_B");
        assert_eq!(value_at(&keys, 0.0), Some(10.0));
        assert_eq!(value_at(&keys, 1.0), Some(20.0));
        assert_eq!(value_at(&keys, 9.0), Some(30.0));
        assert_eq!(value_at(&[(3.0, 4.0)], 0.0), Some(4.0));
        assert_eq!(value_at(&[], 0.0), None);
        assert!(parse(b"FLAN").is_err());
        assert!(parse(&file[..40]).is_err());
    }

    /// One `pai1` whose `contents` table entries all point at one content, whose `tags` all point at one tag, whose
    /// `entries` all point at one target of `keys` step keys.
    fn shared_offsets(contents: usize, tags: usize, entries: usize, keys: usize) -> Vec<u8> {
        let table = 0x14usize;
        let c = table + 4 * contents;
        let t = c + 32 + 4 * tags;
        let e = t + 8 + 4 * entries;
        let mut s = vec![0u8; e + 12 + 8 * keys];
        s[0..4].copy_from_slice(b"pai1");
        let len = s.len() as u32;
        s[4..8].copy_from_slice(&len.to_le_bytes());
        s[0xE..0x10].copy_from_slice(&(contents as u16).to_le_bytes());
        s[0x10..0x14].copy_from_slice(&(table as u32).to_le_bytes());
        for i in 0..contents {
            s[table + 4 * i..table + 4 * i + 4].copy_from_slice(&(c as u32).to_le_bytes());
        }
        s[c..c + 6].copy_from_slice(b"P_Btn\0");
        s[c + 28] = tags as u8;
        for j in 0..tags {
            s[c + 32 + 4 * j..c + 36 + 4 * j].copy_from_slice(&((t - c) as u32).to_le_bytes());
        }
        s[t..t + 4].copy_from_slice(b"FLMC");
        s[t + 4] = entries as u8;
        for k in 0..entries {
            s[t + 8 + 4 * k..t + 12 + 4 * k].copy_from_slice(&((e - t) as u32).to_le_bytes());
        }
        s[e..e + 4].copy_from_slice(&[0, 4, 1, 0]); // step curve
        s[e + 4..e + 6].copy_from_slice(&(keys as u16).to_le_bytes());
        s[e + 8..e + 12].copy_from_slice(&12u32.to_le_bytes());
        let mut f = Vec::new();
        f.extend_from_slice(b"FLAN");
        f.extend_from_slice(&[0xFF, 0xFE]);
        f.extend_from_slice(&0x14u16.to_le_bytes());
        f.extend_from_slice(&0x0900_0000u32.to_le_bytes());
        f.extend_from_slice(&((0x14 + s.len()) as u32).to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&[0, 0]);
        f.extend(s);
        f
    }

    #[test]
    fn shared_targets_cannot_multiply_the_keys() {
        // 35 KB: 65,025 tracks of 4096 keys each (2 GB) when every table entry names the same target.
        let f = shared_offsets(1, 255, 255, 4096);
        assert_eq!(f.len(), 34_904);
        let t = std::time::Instant::now();
        assert!(parse(&f).unwrap_err().contains("expands beyond"));
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        assert!(parse(&shared_offsets(4096, 255, 255, 4096)).is_err());
        // Named once, the same target reads.
        let one = parse(&shared_offsets(1, 1, 1, 4096)).unwrap();
        assert_eq!((one.len(), one[0].keys.len()), (1, 4096));
    }
}
