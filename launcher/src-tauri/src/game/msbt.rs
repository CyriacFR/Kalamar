//! MSBT message files (`MsgStdBn`): label -> text, with the inline control tags kept apart from the text.
//!
//! Labels name their text by index and label groups are offsets, so a crafted file can point every label at one long
//! text, or every group at the same labels. Labels share their message (`Arc`), and the labels read from a section are
//! charged to a budget of the section's size (a sound file stores each label once).

use super::guard::Budget;
use std::collections::HashMap;
use std::sync::Arc;

/// A message: text runs interleaved with control tags.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Message {
    pub pieces: Vec<Piece>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    Text(String),
    /// `0x0E group type size params` (the closing form `0x0F group type` is dropped).
    Tag { group: u16, kind: u16, params: Vec<u8> },
}

/// Grammar tag that carries the gender/number variants of a word suffix (French titles).
pub const TAG_VARIANTS: (u16, u16) = (201, 5);

impl Message {
    /// Plain text: control tags are dropped, except the variant tag, which contributes its first variant (the
    /// default form). Trailing NULs and surrounding whitespace are trimmed.
    pub fn plain(&self) -> String {
        self.render(&mut |_, _| None)
    }

    /// Like [`Message::plain`], but `subst(group, kind)` may supply text for a tag (a name placeholder).
    pub fn render(&self, subst: &mut dyn FnMut(u16, u16) -> Option<String>) -> String {
        let mut out = String::new();
        for p in &self.pieces {
            match p {
                Piece::Text(t) => out.push_str(t),
                Piece::Tag { group, kind, params } => {
                    if let Some(s) = subst(*group, *kind) {
                        out.push_str(&s);
                    } else if (*group, *kind) == TAG_VARIANTS {
                        out.push_str(&first_variant(params));
                    }
                }
            }
        }
        out.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string()
    }

    pub fn has_tag(&self, group: u16, kind: u16) -> bool {
        self.pieces.iter().any(|p| matches!(p, Piece::Tag { group: g, kind: k, .. } if *g == group && *k == kind))
    }
}

/// Variants are a sequence of `(byte length u16, UTF-16 text)`; the first one is the default form.
fn first_variant(params: &[u8]) -> String {
    if params.len() < 2 {
        return String::new();
    }
    let n = u16::from_le_bytes([params[0], params[1]]) as usize;
    let Some(raw) = params.get(2..2 + n) else { return String::new() };
    let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

#[derive(Debug, Clone, Default)]
pub struct Msbt {
    /// Label -> message; labels naming the same text share it.
    pub messages: HashMap<String, Arc<Message>>,
}

impl Msbt {
    pub fn get(&self, label: &str) -> Option<&Message> {
        self.messages.get(label).map(Arc::as_ref)
    }
    pub fn text(&self, label: &str) -> Option<String> {
        self.get(label).map(Message::plain)
    }
}

struct R<'a> {
    b: &'a [u8],
    le: bool,
}

impl R<'_> {
    fn u16(&self, o: usize) -> Result<u16, String> {
        let s: [u8; 2] = self.b.get(o..o + 2).ok_or("MSBT: truncated")?.try_into().unwrap();
        Ok(if self.le { u16::from_le_bytes(s) } else { u16::from_be_bytes(s) })
    }
    fn u32(&self, o: usize) -> Result<u32, String> {
        let s: [u8; 4] = self.b.get(o..o + 4).ok_or("MSBT: truncated")?.try_into().unwrap();
        Ok(if self.le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    }
}

fn decode_utf16(r: &R, raw: &[u8]) -> Message {
    let units: Vec<u16> = raw.chunks_exact(2).map(|c| if r.le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
    let mut pieces = Vec::new();
    let mut text: Vec<u16> = Vec::new();
    let flush = |text: &mut Vec<u16>, pieces: &mut Vec<Piece>| {
        if !text.is_empty() {
            pieces.push(Piece::Text(String::from_utf16_lossy(text)));
            text.clear();
        }
    };
    let mut i = 0;
    while i < units.len() {
        match units[i] {
            0x0E if i + 3 < units.len() => {
                flush(&mut text, &mut pieces);
                let (group, kind, size) = (units[i + 1], units[i + 2], units[i + 3] as usize);
                let end = (i + 4 + size.div_ceil(2)).min(units.len());
                // Sized by the units actually present, not by the size the tag claims (up to 64 KiB per 8-byte tag).
                let mut params = Vec::with_capacity(2 * (end - (i + 4)));
                for u in &units[i + 4..end] {
                    params.extend_from_slice(&u.to_le_bytes());
                }
                params.truncate(size);
                pieces.push(Piece::Tag { group, kind, params });
                i = end;
            }
            0x0F => {
                flush(&mut text, &mut pieces);
                i += 3; // closing tag: group, type
            }
            0 => i += 1,
            u => {
                text.push(u);
                i += 1;
            }
        }
    }
    flush(&mut text, &mut pieces);
    Message { pieces }
}

pub fn parse(b: &[u8]) -> Result<Msbt, String> {
    if b.get(..8) != Some(b"MsgStdBn") {
        return Err("MSBT: bad magic".into());
    }
    let le = match b.get(8..10) {
        Some([0xFF, 0xFE]) => true,
        Some([0xFE, 0xFF]) => false,
        _ => return Err("MSBT: bad byte order mark".into()),
    };
    let r = R { b, le };
    let enc = *b.get(0xC).ok_or("MSBT: truncated")?;
    let sections = r.u16(0xE)? as usize;
    let mut o = 0x20usize;
    let mut labels: Vec<(String, u32)> = Vec::new();
    let mut texts: Vec<Arc<Message>> = Vec::new();
    for _ in 0..sections {
        let tag = b.get(o..o + 4).ok_or("MSBT: truncated section")?;
        let size = r.u32(o + 4)? as usize;
        let d = b.get(o + 16..o + 16 + size).ok_or("MSBT: section out of range")?;
        let dr = R { b: d, le };
        match tag {
            b"LBL1" => {
                let groups = dr.u32(0)? as usize;
                if groups.saturating_mul(8) > d.len() {
                    return Err("MSBT: LBL1 too large".into());
                }
                // Each label (length byte, name, index) is stored once: their bytes cannot exceed the section.
                let mut budget = Budget::new("MSBT labels", d.len(), 1, 0, usize::MAX);
                for g in 0..groups {
                    let count = dr.u32(4 + 8 * g)? as usize;
                    let mut p = dr.u32(8 + 8 * g)? as usize;
                    for _ in 0..count {
                        let len = *d.get(p).ok_or("MSBT: label out of range")? as usize;
                        budget.take(5 + len)?;
                        let name = d.get(p + 1..p + 1 + len).ok_or("MSBT: label out of range")?;
                        let idx = dr.u32(p + 1 + len)?;
                        labels.push((String::from_utf8_lossy(name).into_owned(), idx));
                        p += 5 + len;
                    }
                }
            }
            b"TXT2" => {
                let count = dr.u32(0)? as usize;
                if count.saturating_mul(4) > d.len() {
                    return Err("MSBT: TXT2 too large".into());
                }
                let mut offs = Vec::with_capacity(count + 1);
                for i in 0..count {
                    offs.push(dr.u32(4 + 4 * i)? as usize);
                }
                offs.push(d.len());
                // Text i runs from offset i to offset i + 1 (a decreasing pair is an error): the texts never overlap.
                for i in 0..count {
                    let (s, e) = (offs[i], offs[i + 1]);
                    let raw = d.get(s..e).ok_or("MSBT: text out of range")?;
                    texts.push(Arc::new(if enc == 1 {
                        decode_utf16(&r, raw)
                    } else {
                        Message { pieces: vec![Piece::Text(String::from_utf8_lossy(raw).trim_end_matches('\0').to_string())] }
                    }));
                }
            }
            _ => {}
        }
        o += 16 + ((size + 15) & !15);
    }
    let mut messages = HashMap::with_capacity(labels.len());
    for (name, idx) in labels {
        if let Some(m) = texts.get(idx as usize) {
            messages.insert(name, m.clone());
        }
    }
    Ok(Msbt { messages })
}

#[cfg(test)]
pub mod build {
    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    pub fn tag(group: u16, kind: u16, params: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&0x0Eu16.to_le_bytes());
        v.extend_from_slice(&group.to_le_bytes());
        v.extend_from_slice(&kind.to_le_bytes());
        v.extend_from_slice(&(params.len() as u16).to_le_bytes());
        v.extend_from_slice(params);
        v
    }

    pub fn text(s: &str) -> Vec<u8> {
        utf16(s)
    }

    /// `messages`: label -> raw UTF-16LE body (without the terminating NUL, added here).
    pub fn encode(messages: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut lbl = Vec::new();
        lbl.extend_from_slice(&1u32.to_le_bytes()); // one hash group
        lbl.extend_from_slice(&(messages.len() as u32).to_le_bytes());
        lbl.extend_from_slice(&12u32.to_le_bytes()); // group data starts after the 4 + 8 header bytes
        for (i, (name, _)) in messages.iter().enumerate() {
            lbl.push(name.len() as u8);
            lbl.extend_from_slice(name.as_bytes());
            lbl.extend_from_slice(&(i as u32).to_le_bytes());
        }
        let mut txt = Vec::new();
        txt.extend_from_slice(&(messages.len() as u32).to_le_bytes());
        let mut off = 4 + 4 * messages.len();
        let mut bodies = Vec::new();
        for (_, body) in messages {
            txt.extend_from_slice(&(off as u32).to_le_bytes());
            let mut b = body.clone();
            b.extend_from_slice(&[0, 0]);
            off += b.len();
            bodies.push(b);
        }
        for b in bodies {
            txt.extend_from_slice(&b);
        }
        sections(&lbl, &txt)
    }

    /// A UTF-16 file made of the given raw LBL1 and TXT2 section contents.
    pub fn sections(lbl: &[u8], txt: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"MsgStdBn");
        out.extend_from_slice(&[0xFF, 0xFE, 0, 0, 1, 3]);
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&[0; 4]); // file size, patched below
        out.extend_from_slice(&[0; 10]);
        for (tag, data) in [(&b"LBL1"[..], lbl), (&b"TXT2"[..], txt)] {
            out.extend_from_slice(tag);
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&[0; 8]);
            out.extend_from_slice(data);
            while !out.len().is_multiple_of(16) {
                out.push(0xAB);
            }
        }
        let n = out.len() as u32;
        out[0x12..0x16].copy_from_slice(&n.to_le_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn reads_labels_and_plain_text() {
        let b = encode(&[("NCP010", text("Bonnet zigzags")), ("B02", text("Krakers")), ("empty", vec![])]);
        let m = parse(&b).unwrap();
        assert_eq!(m.text("NCP010").as_deref(), Some("Bonnet zigzags"));
        assert_eq!(m.text("B02").as_deref(), Some("Krakers"));
        assert_eq!(m.text("empty").as_deref(), Some(""));
        assert!(m.get("zzz").is_none());
    }

    #[test]
    fn variant_tag_gives_default_form() {
        // "champion" + {masculine: "", feminine: "ne"}, as in the French title words.
        let mut params = Vec::new();
        params.extend_from_slice(&0u16.to_le_bytes());
        params.extend_from_slice(&4u16.to_le_bytes());
        params.extend_from_slice(&text("ne"));
        params.extend_from_slice(&[0; 4]);
        let mut body = text("champion");
        body.extend_from_slice(&tag(201, 5, &params));
        body.extend_from_slice(&text(" de C&T"));
        // "travailleu" + {"r", "se"}
        let mut p2 = Vec::new();
        p2.extend_from_slice(&2u16.to_le_bytes());
        p2.extend_from_slice(&text("r"));
        p2.extend_from_slice(&4u16.to_le_bytes());
        p2.extend_from_slice(&text("se"));
        p2.extend_from_slice(&[0; 4]);
        let mut body2 = text("travailleu");
        body2.extend_from_slice(&tag(201, 5, &p2));
        let m = parse(&encode(&[("a", body), ("b", body2)])).unwrap();
        assert_eq!(m.text("a").as_deref(), Some("champion de C&T"));
        assert_eq!(m.text("b").as_deref(), Some("travailleur"));
    }

    #[test]
    fn other_tags_are_dropped_and_placeholders_substituted() {
        let mut body = tag(4, 1, &[0, 0]);
        body.extend_from_slice(&text(" : niveau 4"));
        body.extend_from_slice(&tag(1, 2, &[]));
        body.extend_from_slice(&0x0Fu16.to_le_bytes());
        body.extend_from_slice(&[4, 0, 1, 0]);
        let m = parse(&encode(&[("w", body)])).unwrap();
        assert_eq!(m.text("w").as_deref(), Some(": niveau 4"));
        let msg = m.get("w").unwrap();
        assert!(msg.has_tag(4, 1));
        assert_eq!(msg.render(&mut |g, k| (g == 4 && k == 1).then(|| "Calibre 3000".to_string())), "Calibre 3000 : niveau 4");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"").is_err());
        assert!(parse(b"MsgStdBn").is_err());
        let mut b = encode(&[("a", text("x"))]);
        b.truncate(0x30);
        assert!(parse(&b).is_err());
    }

    /// One hash group of `labels` distinct labels, all naming text 0.
    fn labels_to_text0(labels: usize) -> Vec<u8> {
        let mut lbl = Vec::new();
        lbl.extend_from_slice(&1u32.to_le_bytes());
        lbl.extend_from_slice(&(labels as u32).to_le_bytes());
        lbl.extend_from_slice(&12u32.to_le_bytes());
        for i in 0..labels {
            let name = format!("{i:06x}");
            lbl.push(name.len() as u8);
            lbl.extend_from_slice(name.as_bytes());
            lbl.extend_from_slice(&0u32.to_le_bytes());
        }
        lbl
    }

    #[test]
    fn labels_naming_one_text_share_it() {
        // 1.2 MB: 60,000 labels on one 512 KiB text, 30 GB as separate copies.
        let mut txt = Vec::new();
        txt.extend_from_slice(&1u32.to_le_bytes());
        txt.extend_from_slice(&8u32.to_le_bytes());
        txt.extend(std::iter::repeat_n([0x41, 0], 262_144).flatten());
        let f = sections(&labels_to_text0(60_000), &txt);
        let t = std::time::Instant::now();
        let m = parse(&f).unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(m.messages.len(), 60_000);
        assert!(Arc::ptr_eq(&m.messages["000000"], &m.messages["00ea5f"]));
        assert_eq!(m.text("000001").unwrap().len(), 262_144);
    }

    #[test]
    fn label_groups_cannot_repeat_the_same_labels() {
        // 1000 hash groups that all list the same 100 labels: 100,000 labels out of a 9.5 KB section.
        let (groups, count) = (1000usize, 100usize);
        let mut lbl = Vec::new();
        lbl.extend_from_slice(&(groups as u32).to_le_bytes());
        let first = 4 + 8 * groups;
        for _ in 0..groups {
            lbl.extend_from_slice(&(count as u32).to_le_bytes());
            lbl.extend_from_slice(&(first as u32).to_le_bytes());
        }
        lbl.extend_from_slice(&labels_to_text0(count)[12..]);
        let mut txt = 1u32.to_le_bytes().to_vec();
        txt.extend_from_slice(&8u32.to_le_bytes());
        txt.extend_from_slice(&text("x"));
        assert!(parse(&sections(&lbl, &txt)).unwrap_err().contains("expands beyond"));
        // One group listing them once reads.
        let mut one = lbl.clone();
        one[..4].copy_from_slice(&1u32.to_le_bytes());
        one.drain(12..first);
        one[8..12].copy_from_slice(&12u32.to_le_bytes());
        assert_eq!(parse(&sections(&one, &txt)).unwrap().messages.len(), count);
    }

    #[test]
    fn tag_parameters_take_only_the_room_they_have() {
        // A tag claiming 65,535 bytes of parameters at the very end of its text: nothing reserved for them.
        let body = [0x0E, 4, 1, 0xFFFF].iter().flat_map(|u: &u16| u.to_le_bytes()).collect::<Vec<u8>>();
        let m = parse(&encode(&[("a", body)])).unwrap();
        let Some(Piece::Tag { params, .. }) = m.get("a").unwrap().pieces.first() else { panic!("a tag expected") };
        assert!(params.capacity() <= 4, "{}", params.capacity());
    }
}
