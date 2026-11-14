//! BYML (binary YAML) reader, versions 2 to 7, little endian (`YB`) or big endian (`BY`).
//! Only what the parameter tables need: strings, numbers, booleans, null, arrays, dictionaries, binary blobs.
//!
//! Values point at their containers and strings by offset or index, so a crafted file can share one container many
//! times over (a few hundred bytes whose tree has 2^64 nodes) or one long string from every row. Strings and keys are
//! therefore shared (`Arc<str>`, one copy per table entry), and each parse counts against budgets proportional to the
//! file's size: the nodes it expands (at most [`MAX_NODES`]) and the bytes of its string tables and blobs.

use super::guard::Budget;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Most nodes one document may expand to. The largest parameter table of 11.2.0 (1.9 MB) has 156,644.
pub const MAX_NODES: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Byml {
    Null,
    Bool(bool),
    I32(i32),
    U32(u32),
    F32(f32),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(Arc<str>),
    Bin(Vec<u8>),
    Array(Vec<Byml>),
    Dict(BTreeMap<Arc<str>, Byml>),
}

impl Byml {
    pub fn get(&self, key: &str) -> Option<&Byml> {
        match self {
            Byml::Dict(d) => d.get(key),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Byml::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Byml::I32(v) => Some(v as i64),
            Byml::U32(v) => Some(v as i64),
            Byml::I64(v) => Some(v),
            Byml::U64(v) => i64::try_from(v).ok(),
            Byml::Bool(b) => Some(b as i64),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Byml::F32(v) => Some(v as f64),
            Byml::F64(v) => Some(v),
            _ => self.as_i64().map(|v| v as f64),
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Byml::Bool(b) => Some(b),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Byml]> {
        match self {
            Byml::Array(a) => Some(a),
            _ => None,
        }
    }
    /// Convenience accessors on dictionary rows.
    pub fn str_of(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Byml::as_str)
    }
    pub fn int_of(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Byml::as_i64)
    }
    pub fn bool_of(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Byml::as_bool)
    }
}

struct Reader<'a> {
    b: &'a [u8],
    le: bool,
}

type Res<T> = Result<T, String>;

impl<'a> Reader<'a> {
    fn bytes(&self, off: usize, n: usize) -> Res<&'a [u8]> {
        off.checked_add(n).and_then(|e| self.b.get(off..e)).ok_or_else(|| format!("BYML: read past end at {off:#x}"))
    }
    fn u8(&self, off: usize) -> Res<u8> {
        Ok(self.bytes(off, 1)?[0])
    }
    fn u16(&self, off: usize) -> Res<u16> {
        let b: [u8; 2] = self.bytes(off, 2)?.try_into().unwrap();
        Ok(if self.le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }
    fn u24(&self, off: usize) -> Res<u32> {
        let b = self.bytes(off, 3)?;
        Ok(if self.le { u32::from_le_bytes([b[0], b[1], b[2], 0]) } else { u32::from_be_bytes([0, b[0], b[1], b[2]]) })
    }
    fn u32(&self, off: usize) -> Res<u32> {
        let b: [u8; 4] = self.bytes(off, 4)?.try_into().unwrap();
        Ok(if self.le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }
    fn u64(&self, off: usize) -> Res<u64> {
        let b: [u8; 8] = self.bytes(off, 8)?.try_into().unwrap();
        Ok(if self.le { u64::from_le_bytes(b) } else { u64::from_be_bytes(b) })
    }

    /// A string table: `0xC2`, u24 count, then count+1 u32 offsets relative to the table start. The string ranges are
    /// charged to `bytes` before they are read (each string is stored once in a sound file, the ranges do not overlap).
    fn string_table(&self, off: usize, bytes: &mut Budget) -> Res<Vec<Arc<str>>> {
        if off == 0 {
            return Ok(Vec::new());
        }
        if self.u8(off)? != 0xC2 {
            return Err("BYML: bad string table tag".into());
        }
        let n = self.u24(off + 1)? as usize;
        if n.saturating_mul(4) > self.b.len() {
            return Err("BYML: string table too large".into());
        }
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let s = off + self.u32(off + 4 + 4 * i)? as usize;
            let e = off + self.u32(off + 8 + 4 * i)? as usize;
            let len = e.checked_sub(s).ok_or("BYML: bad string range")?;
            bytes.take(len)?;
            let raw = self.bytes(s, len)?;
            let raw = raw.split(|&c| c == 0).next().unwrap_or(&[]);
            out.push(Arc::from(String::from_utf8_lossy(raw)));
        }
        Ok(out)
    }
}

struct Ctx<'a> {
    r: Reader<'a>,
    keys: Vec<Arc<str>>,
    strs: Vec<Arc<str>>,
    /// Nodes expanded so far (each container charges its elements before they are built).
    nodes: Budget,
    /// Bytes of the string tables and of the binary blobs.
    bytes: Budget,
}

impl Ctx<'_> {
    fn value(&mut self, t: u8, v: u32, depth: u32) -> Res<Byml> {
        Ok(match t {
            0xA0 => Byml::Str(self.strs.get(v as usize).ok_or("BYML: string index out of range")?.clone()),
            0xA1 => {
                let n = self.r.u32(v as usize)? as usize;
                self.bytes.take(n)?;
                Byml::Bin(self.r.bytes(v as usize + 4, n)?.to_vec())
            }
            0xD0 => Byml::Bool(v != 0),
            0xD1 => Byml::I32(v as i32),
            0xD2 => Byml::F32(f32::from_bits(v)),
            0xD3 => Byml::U32(v),
            0xD4 => Byml::I64(self.r.u64(v as usize)? as i64),
            0xD5 => Byml::U64(self.r.u64(v as usize)?),
            0xD6 => Byml::F64(f64::from_bits(self.r.u64(v as usize)?)),
            0xFF => Byml::Null,
            0xC0 | 0xC1 => self.node(v as usize, depth + 1)?,
            other => return Err(format!("BYML: unsupported value type {other:#x}")),
        })
    }

    fn node(&mut self, off: usize, depth: u32) -> Res<Byml> {
        if depth > 64 {
            return Err("BYML: nesting too deep".into());
        }
        let t = self.r.u8(off)?;
        let n = self.r.u24(off + 1)? as usize;
        match t {
            0xC0 => {
                // Guard against absurd counts before allocating: each element needs at least 1 + 4 bytes.
                if n.saturating_mul(5) > self.r.b.len() {
                    return Err("BYML: array count too large".into());
                }
                self.nodes.take(n)?;
                let types = self.r.bytes(off + 4, n)?;
                let base = off + 4 + ((n + 3) & !3);
                let mut out = Vec::with_capacity(n);
                for (i, &ty) in types.iter().enumerate() {
                    let v = self.r.u32(base + 4 * i)?;
                    out.push(self.value(ty, v, depth)?);
                }
                Ok(Byml::Array(out))
            }
            0xC1 => {
                if n.saturating_mul(8) > self.r.b.len() {
                    return Err("BYML: dictionary count too large".into());
                }
                self.nodes.take(n)?;
                let mut d = BTreeMap::new();
                for i in 0..n {
                    let e = off + 4 + 8 * i;
                    let k = self.r.u24(e)? as usize;
                    let ty = self.r.u8(e + 3)?;
                    let v = self.r.u32(e + 4)?;
                    let key = self.keys.get(k).ok_or("BYML: key index out of range")?.clone();
                    d.insert(key, self.value(ty, v, depth)?);
                }
                Ok(Byml::Dict(d))
            }
            other => Err(format!("BYML: unexpected container tag {other:#x}")),
        }
    }
}

pub fn parse(data: &[u8]) -> Result<Byml, String> {
    if data.len() < 16 {
        return Err("BYML: file too short".into());
    }
    let le = match &data[..2] {
        b"YB" => true,
        b"BY" => false,
        _ => return Err("BYML: bad magic".into()),
    };
    let r = Reader { b: data, le };
    let version = r.u16(2)?;
    if !(2..=7).contains(&version) {
        return Err(format!("BYML: unsupported version {version}"));
    }
    let (ko, so, ro) = (r.u32(4)? as usize, r.u32(8)? as usize, r.u32(12)? as usize);
    // A sound file stores each string and blob once, at most its own size in all (twice, should the key and string
    // tables be the same one), and holds at most one node per 5 bytes (the game's tables: one per 6 or more); the
    // budgets allow twice the size and one node per 2 bytes.
    let mut bytes = Budget::new("BYML", data.len(), 2, 0, usize::MAX);
    let keys = r.string_table(ko, &mut bytes)?;
    let strs = r.string_table(so, &mut bytes)?;
    let nodes = Budget::new("BYML", data.len() / 2, 1, 64, MAX_NODES);
    let mut ctx = Ctx { r, keys, strs, nodes, bytes };
    if ro == 0 {
        return Ok(Byml::Null);
    }
    ctx.node(ro, 0)
}

/// Builds a minimal little-endian version-7 BYML document. Used by the tests (and only the tests) to construct
/// synthetic fixtures.
#[cfg(test)]
pub mod build {
    use super::Byml;
    use std::collections::BTreeSet;

    fn align4(v: &mut Vec<u8>) {
        while !v.len().is_multiple_of(4) {
            v.push(0);
        }
    }

    fn table(strings: &[String]) -> Vec<u8> {
        let mut out = vec![0xC2];
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes()[..3]);
        let mut off = 4 + 4 * (strings.len() + 1);
        let mut offs = Vec::new();
        let mut blob = Vec::new();
        for s in strings {
            offs.push(off);
            blob.extend_from_slice(s.as_bytes());
            blob.push(0);
            off += s.len() + 1;
        }
        offs.push(off);
        for o in offs {
            out.extend_from_slice(&(o as u32).to_le_bytes());
        }
        out.extend_from_slice(&blob);
        align4(&mut out);
        out
    }

    fn collect(n: &Byml, k: &mut BTreeSet<String>, s: &mut BTreeSet<String>) {
        match n {
            Byml::Str(x) => {
                s.insert(x.to_string());
            }
            Byml::Array(a) => a.iter().for_each(|x| collect(x, k, s)),
            Byml::Dict(d) => d.iter().for_each(|(kk, v)| {
                k.insert(kk.to_string());
                collect(v, k, s);
            }),
            _ => {}
        }
    }

    fn scalar(n: &Byml, out: &mut Vec<u8>, keys: &[String], strs: &[String]) -> (u8, u32) {
        let wide = |bytes: &[u8], t: u8, out: &mut Vec<u8>| {
            align4(out);
            let o = out.len();
            out.extend_from_slice(bytes);
            (t, o as u32)
        };
        match n {
            Byml::Null => (0xFF, 0),
            Byml::Bool(b) => (0xD0, *b as u32),
            Byml::I32(v) => (0xD1, *v as u32),
            Byml::U32(v) => (0xD3, *v),
            Byml::F32(v) => (0xD2, v.to_bits()),
            Byml::Str(s) => (0xA0, strs.iter().position(|x| **x == **s).unwrap() as u32),
            Byml::I64(v) => wide(&v.to_le_bytes(), 0xD4, out),
            Byml::U64(v) => wide(&v.to_le_bytes(), 0xD5, out),
            Byml::F64(v) => wide(&v.to_le_bytes(), 0xD6, out),
            Byml::Bin(b) => {
                let mut blob = (b.len() as u32).to_le_bytes().to_vec();
                blob.extend_from_slice(b);
                wide(&blob, 0xA1, out)
            }
            Byml::Array(_) => (0xC0, container(n, out, keys, strs) as u32),
            Byml::Dict(_) => (0xC1, container(n, out, keys, strs) as u32),
        }
    }

    fn container(n: &Byml, out: &mut Vec<u8>, keys: &[String], strs: &[String]) -> usize {
        match n {
            Byml::Array(a) => {
                let items: Vec<(u8, u32)> = a.iter().map(|x| scalar(x, out, keys, strs)).collect();
                align4(out);
                let o = out.len();
                out.push(0xC0);
                out.extend_from_slice(&(a.len() as u32).to_le_bytes()[..3]);
                out.extend(items.iter().map(|i| i.0));
                align4(out);
                for i in &items {
                    out.extend_from_slice(&i.1.to_le_bytes());
                }
                o
            }
            Byml::Dict(d) => {
                let items: Vec<(u32, u8, u32)> = d
                    .iter()
                    .map(|(k, v)| {
                        let (t, x) = scalar(v, out, keys, strs);
                        (keys.iter().position(|y| **y == **k).unwrap() as u32, t, x)
                    })
                    .collect();
                align4(out);
                let o = out.len();
                out.push(0xC1);
                out.extend_from_slice(&(d.len() as u32).to_le_bytes()[..3]);
                for (k, t, x) in items {
                    out.extend_from_slice(&k.to_le_bytes()[..3]);
                    out.push(t);
                    out.extend_from_slice(&x.to_le_bytes());
                }
                o
            }
            _ => panic!("root must be a container"),
        }
    }

    pub fn encode(root: &Byml) -> Vec<u8> {
        let (mut keys, mut strs) = (BTreeSet::new(), BTreeSet::new());
        collect(root, &mut keys, &mut strs);
        let keys: Vec<String> = keys.into_iter().collect();
        let strs: Vec<String> = strs.into_iter().collect();
        let mut out = vec![0u8; 16];
        out[0..2].copy_from_slice(b"YB");
        out[2..4].copy_from_slice(&7u16.to_le_bytes());
        let ko = out.len();
        out.extend_from_slice(&table(&keys));
        let so = out.len();
        out.extend_from_slice(&table(&strs));
        let ro = container(root, &mut out, &keys, &strs);
        out[4..8].copy_from_slice(&(ko as u32).to_le_bytes());
        out[8..12].copy_from_slice(&(so as u32).to_le_bytes());
        out[12..16].copy_from_slice(&(ro as u32).to_le_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(items: Vec<(&str, Byml)>) -> Byml {
        Byml::Dict(items.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// A version-7 header: key table, string table and root offsets.
    fn header(ko: u32, so: u32, ro: u32) -> Vec<u8> {
        let mut b = vec![b'Y', b'B', 7, 0];
        for v in [ko, so, ro] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b
    }

    /// `levels` arrays of `k` elements, every element of a level pointing at the one array of the next level; the last
    /// level holds `k` integers. A file of ~ levels * (8 + 5k) bytes whose tree has k^levels leaves.
    fn dag(k: usize, levels: usize) -> Vec<u8> {
        let node_len = 4 + ((k + 3) & !3) + 4 * k;
        let mut out = header(0, 0, 16);
        for lvl in 0..levels {
            let last = lvl + 1 == levels;
            out.push(0xC0);
            out.extend_from_slice(&(k as u32).to_le_bytes()[..3]);
            out.extend(std::iter::repeat_n(if last { 0xD1 } else { 0xC0 }, k));
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
            let child = (16 + (lvl + 1) * node_len) as u32;
            for i in 0..k {
                out.extend_from_slice(&(if last { i as u32 } else { child }).to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn shared_containers_cannot_expand_past_the_node_budget() {
        // 1,040 bytes, 2^64 leaves: an error at once instead of filling the memory.
        let f = dag(2, 64);
        assert_eq!(f.len(), 1040);
        let t = std::time::Instant::now();
        assert!(parse(&f).unwrap_err().contains("expands beyond"));
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        // Wide and shallow: 16^5 leaves out of 436 bytes.
        assert!(parse(&dag(16, 5)).unwrap_err().contains("expands beyond"));
        // A container that holds itself stops at the budget (or the depth limit), never recursing further.
        let mut cyc = header(0, 0, 16);
        cyc.extend_from_slice(&[0xC0, 2, 0, 0, 0xC0, 0xC0, 0, 0]);
        cyc.extend_from_slice(&16u32.to_le_bytes());
        cyc.extend_from_slice(&16u32.to_le_bytes());
        assert!(parse(&cyc).is_err());
        // A small sharing that stays within the budget still reads (8 leaves out of 3 levels of 2).
        let ok = parse(&dag(2, 3)).unwrap();
        let leaves = |b: &Byml| -> usize {
            fn n(b: &Byml) -> usize {
                b.as_array().map_or(1, |a| a.iter().map(n).sum())
            }
            n(b)
        };
        assert_eq!(leaves(&ok), 8);
    }

    /// A root array of `n` elements of type `ty`, all with the value 0, after a string table holding one string of
    /// `len` bytes (or, for blobs, a blob of `len` bytes at offset 16, referenced as value 16).
    fn one_shared(ty: u8, n: usize, len: usize) -> Vec<u8> {
        let mut f = header(0, if ty == 0xA0 { 16 } else { 0 }, 0);
        if ty == 0xA0 {
            f.push(0xC2);
            f.extend_from_slice(&1u32.to_le_bytes()[..3]);
            f.extend_from_slice(&12u32.to_le_bytes());
            f.extend_from_slice(&((12 + len) as u32).to_le_bytes());
        } else {
            f.extend_from_slice(&(len as u32).to_le_bytes());
        }
        f.extend(std::iter::repeat_n(b'A', len));
        f.push(0);
        while !f.len().is_multiple_of(4) {
            f.push(0);
        }
        let root = f.len() as u32;
        f[12..16].copy_from_slice(&root.to_le_bytes());
        f.push(0xC0);
        f.extend_from_slice(&(n as u32).to_le_bytes()[..3]);
        f.extend(std::iter::repeat_n(ty, n));
        while !f.len().is_multiple_of(4) {
            f.push(0);
        }
        let v: u32 = if ty == 0xA0 { 0 } else { 16 };
        for _ in 0..n {
            f.extend_from_slice(&v.to_le_bytes());
        }
        f
    }

    #[test]
    fn a_string_referenced_by_every_row_is_stored_once() {
        // 565 KB whose 100,000 elements all name one 64 KiB string: 6.5 GB as separate copies, one copy here.
        let f = one_shared(0xA0, 100_000, 65_536);
        assert_eq!(f.len(), 565_572);
        let t = std::time::Instant::now();
        let doc = parse(&f).unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        let items = doc.as_array().unwrap();
        assert_eq!(items.len(), 100_000);
        let (Byml::Str(a), Byml::Str(b)) = (&items[0], &items[99_999]) else { panic!("strings expected") };
        assert!(Arc::ptr_eq(a, b), "one shared copy");
        assert_eq!(a.len(), 65_536);
    }

    #[test]
    fn a_blob_referenced_many_times_is_budgeted() {
        // One 16 KiB blob named by 1000 elements: 16 MB of copies out of 21 KB.
        let f = one_shared(0xA1, 1000, 16_384);
        assert!(parse(&f).unwrap_err().contains("expands beyond"));
        // Referenced once, it reads.
        assert_eq!(parse(&one_shared(0xA1, 1, 16_384)).unwrap().as_array().unwrap()[0], Byml::Bin(vec![b'A'; 16_384]));
    }

    #[test]
    fn roundtrip_all_value_kinds() {
        let doc = Byml::Array(vec![
            dict(vec![
                ("Id", Byml::I32(-5)),
                ("Name", Byml::Str("Hed_ACC003".into())),
                ("Rate", Byml::F32(0.5)),
                ("Flag", Byml::Bool(true)),
                ("Big", Byml::I64(-1 << 40)),
                ("UBig", Byml::U64(1 << 50)),
                ("D", Byml::F64(2.25)),
                ("U", Byml::U32(4_000_000_000)),
                ("Nil", Byml::Null),
                ("Blob", Byml::Bin(vec![1, 2, 3])),
                ("Nested", dict(vec![("A", Byml::Array(vec![Byml::I32(1), Byml::Str("x".into())]))])),
            ]),
            dict(vec![("Id", Byml::I32(7))]),
        ]);
        let bytes = build::encode(&doc);
        assert_eq!(parse(&bytes).unwrap(), doc);
    }

    #[test]
    fn accessors() {
        let row = dict(vec![("Id", Byml::I32(3)), ("S", Byml::Str("a".into())), ("F", Byml::F32(1.5)), ("B", Byml::Bool(false))]);
        assert_eq!(row.int_of("Id"), Some(3));
        assert_eq!(row.str_of("S"), Some("a"));
        assert_eq!(row.get("F").and_then(Byml::as_f64), Some(1.5));
        assert_eq!(row.bool_of("B"), Some(false));
        assert_eq!(row.int_of("missing"), None);
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        assert!(parse(b"").is_err());
        assert!(parse(b"XXXXXXXXXXXXXXXXXXXX").is_err());
        let mut good = build::encode(&Byml::Array(vec![Byml::I32(1)]));
        good.truncate(good.len() - 3);
        assert!(parse(&good).is_err());
        // A huge claimed element count must not allocate or panic.
        let mut evil = build::encode(&Byml::Array(vec![Byml::I32(1)]));
        let ro = u32::from_le_bytes(evil[12..16].try_into().unwrap()) as usize;
        evil[ro + 1..ro + 4].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
        assert!(parse(&evil).is_err());
    }

    #[test]
    fn big_endian_header_is_recognised() {
        // Hand-built big-endian document: root array with one int.
        let mut b = vec![b'B', b'Y', 0, 7];
        b.extend_from_slice(&0u32.to_be_bytes()); // no key table
        b.extend_from_slice(&0u32.to_be_bytes()); // no string table
        b.extend_from_slice(&16u32.to_be_bytes()); // root
        b.extend_from_slice(&[0xC0, 0, 0, 1, 0xD1, 0, 0, 0]);
        b.extend_from_slice(&42u32.to_be_bytes());
        assert_eq!(parse(&b).unwrap(), Byml::Array(vec![Byml::I32(42)]));
    }
}
