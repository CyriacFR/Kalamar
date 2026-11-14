//! SARC archive reader (the container used for message packs and layouts).
//!
//! Every entry names its file by an offset into the name table, so a crafted archive can give its 65,535 entries the
//! same long name. Names are at most [`MAX_NAME`] bytes, and all of them together at most the archive's size (each name
//! is stored once in a sound archive).

use super::guard::Budget;
use std::ops::Range;

/// Longest entry name read. The archives of 11.2.0 the launcher reads use at most 56 bytes.
pub const MAX_NAME: usize = 1024;

#[derive(Debug, Clone)]
pub struct Sarc {
    pub entries: Vec<(String, Range<usize>)>,
}

fn rd16(b: &[u8], o: usize, le: bool) -> Result<u16, String> {
    let s: [u8; 2] = b.get(o..o + 2).ok_or("SARC: truncated")?.try_into().unwrap();
    Ok(if le { u16::from_le_bytes(s) } else { u16::from_be_bytes(s) })
}
fn rd32(b: &[u8], o: usize, le: bool) -> Result<u32, String> {
    let s: [u8; 4] = b.get(o..o + 4).ok_or("SARC: truncated")?.try_into().unwrap();
    Ok(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
}

impl Sarc {
    pub fn parse(b: &[u8]) -> Result<Sarc, String> {
        if b.get(..4) != Some(b"SARC") {
            return Err("SARC: bad magic".into());
        }
        let le = match b.get(6..8) {
            Some([0xFF, 0xFE]) => true,
            Some([0xFE, 0xFF]) => false,
            _ => return Err("SARC: bad byte order mark".into()),
        };
        let hdr = rd16(b, 4, le)? as usize;
        let data_off = rd32(b, 0xC, le)? as usize;
        if b.get(hdr..hdr + 4) != Some(b"SFAT") {
            return Err("SARC: SFAT missing".into());
        }
        let count = rd16(b, hdr + 6, le)? as usize;
        let nodes = hdr + 12;
        let names = nodes + 16 * count + 8; // the SFNT header is 8 bytes
        if b.get(nodes + 16 * count..nodes + 16 * count + 4) != Some(b"SFNT") {
            return Err("SARC: SFNT missing".into());
        }
        let mut entries = Vec::with_capacity(count);
        let mut name_bytes = Budget::new("SARC", b.len(), 1, 0, usize::MAX);
        for i in 0..count {
            let n = nodes + 16 * i;
            let attr = rd32(b, n + 4, le)?;
            let (s, e) = (rd32(b, n + 8, le)? as usize, rd32(b, n + 12, le)? as usize);
            let no = names + ((attr & 0x00FF_FFFF) as usize) * 4;
            let tail = b.get(no..).ok_or("SARC: bad name offset")?;
            let len = tail.iter().take(MAX_NAME + 1).position(|&c| c == 0).ok_or("SARC: unterminated or overlong name")?;
            name_bytes.take(len)?;
            let name = String::from_utf8_lossy(&tail[..len]).into_owned();
            match (data_off.checked_add(s), data_off.checked_add(e)) {
                (Some(a), Some(z)) if a <= z && z <= b.len() => entries.push((name, a..z)),
                _ => return Err(format!("SARC: entry {name} out of range")),
            }
        }
        Ok(Sarc { entries })
    }

    pub fn find<'a>(&self, data: &'a [u8], name: &str) -> Option<&'a [u8]> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, r)| &data[r.clone()])
    }
}

#[cfg(test)]
pub mod build {
    /// Writes a little-endian SARC with the given (name, bytes) files. Hashes are not computed (the reader does not
    /// use them); names are stored in the given order.
    pub fn encode(files: &[(&str, &[u8])]) -> Vec<u8> {
        let count = files.len();
        let mut sfnt = Vec::new();
        let mut name_offs = Vec::new();
        for (n, _) in files {
            name_offs.push(sfnt.len() / 4);
            sfnt.extend_from_slice(n.as_bytes());
            sfnt.push(0);
            while !sfnt.len().is_multiple_of(4) {
                sfnt.push(0);
            }
        }
        let mut data = Vec::new();
        let mut ranges = Vec::new();
        for (_, d) in files {
            while !data.len().is_multiple_of(4) {
                data.push(0);
            }
            ranges.push((data.len(), data.len() + d.len()));
            data.extend_from_slice(d);
        }
        let hdr = 0x14usize;
        let mut data_off = hdr + 12 + 16 * count + 8 + sfnt.len();
        while !data_off.is_multiple_of(8) {
            data_off += 1;
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"SARC");
        out.extend_from_slice(&(hdr as u16).to_le_bytes());
        out.extend_from_slice(&[0xFF, 0xFE]);
        out.extend_from_slice(&((data_off + data.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(data_off as u32).to_le_bytes());
        out.extend_from_slice(&0x100u16.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(b"SFAT");
        out.extend_from_slice(&12u16.to_le_bytes());
        out.extend_from_slice(&(count as u16).to_le_bytes());
        out.extend_from_slice(&0x65u32.to_le_bytes());
        for i in 0..count {
            out.extend_from_slice(&(i as u32).to_le_bytes()); // fake hash
            out.extend_from_slice(&(0x0100_0000 | name_offs[i] as u32).to_le_bytes());
            out.extend_from_slice(&(ranges[i].0 as u32).to_le_bytes());
            out.extend_from_slice(&(ranges[i].1 as u32).to_le_bytes());
        }
        out.extend_from_slice(b"SFNT");
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&sfnt);
        while out.len() < data_off {
            out.push(0);
        }
        out.extend_from_slice(&data);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_files_by_name() {
        let a = build::encode(&[("CommonMsg/A.msbt", b"hello"), ("blyt/B.bflyt", b"0123456789"), ("empty", b"")]);
        let s = Sarc::parse(&a).unwrap();
        assert_eq!(s.entries.len(), 3);
        assert_eq!(s.find(&a, "CommonMsg/A.msbt"), Some(&b"hello"[..]));
        assert_eq!(s.find(&a, "blyt/B.bflyt"), Some(&b"0123456789"[..]));
        assert_eq!(s.find(&a, "empty"), Some(&b""[..]));
        assert_eq!(s.find(&a, "nope"), None);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Sarc::parse(b"").is_err());
        assert!(Sarc::parse(b"SARC").is_err());
        let a = build::encode(&[("x", b"abc")]);
        assert!(Sarc::parse(&a[..30]).is_err());
        let mut bad = a.clone();
        bad[0] = b'X';
        assert!(Sarc::parse(&bad).is_err());
        // Entry range beyond the file.
        let mut evil = a.clone();
        let node = 0x14 + 12;
        evil[node + 12..node + 16].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        assert!(Sarc::parse(&evil).is_err());
    }

    /// `count` entries that all name the same `len`-byte name (and the same empty file).
    fn same_name(count: usize, len: usize) -> Vec<u8> {
        let hdr = 0x14usize;
        let data_off = hdr + 12 + 16 * count + 8 + len + 1;
        let mut out = Vec::new();
        out.extend_from_slice(b"SARC");
        out.extend_from_slice(&(hdr as u16).to_le_bytes());
        out.extend_from_slice(&[0xFF, 0xFE]);
        out.extend_from_slice(&(data_off as u32).to_le_bytes());
        out.extend_from_slice(&(data_off as u32).to_le_bytes());
        out.extend_from_slice(&[0, 1, 0, 0]);
        out.extend_from_slice(b"SFAT");
        out.extend_from_slice(&12u16.to_le_bytes());
        out.extend_from_slice(&(count as u16).to_le_bytes());
        out.extend_from_slice(&0x65u32.to_le_bytes());
        for _ in 0..count {
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0x0100_0000u32.to_le_bytes()); // name offset 0
            out.extend_from_slice(&[0; 8]);
        }
        out.extend_from_slice(b"SFNT");
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend(std::iter::repeat_n(b'N', len));
        out.push(0);
        out
    }

    #[test]
    fn a_shared_name_cannot_multiply_the_archive() {
        // 1 MB whose 65,535 entries share one 1000-byte name: 65 MB of names, refused.
        let f = same_name(65_535, 1000);
        assert!(f.len() < 1_100_000);
        assert!(Sarc::parse(&f).unwrap_err().contains("expands beyond"));
        // A name over the length limit is refused without scanning further.
        assert!(Sarc::parse(&same_name(1, MAX_NAME + 1)).unwrap_err().contains("overlong"));
        // Within both limits, the same layout reads.
        let ok = Sarc::parse(&same_name(1, MAX_NAME)).unwrap();
        assert_eq!(ok.entries.len(), 1);
        assert_eq!(ok.entries[0].0.len(), MAX_NAME);
    }
}
