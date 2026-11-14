//! Line-preserving reader/writer for the runtime's `totk.ini`.
//!
//! The format follows `runtime/cfg.c`: `key = value` lines, `#`/`;` comments, `[sections]` that only group
//! lines, keys case-insensitive with an optional `TOTK_` prefix, an optional UTF-8 BOM, a later line wins over an
//! earlier one, an empty value means "default", and a value may be wrapped in double quotes. Everything the
//! writer does not change (comments, unknown keys, blank lines, order, line endings, BOM) is kept as it was.

#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    /// Comment, blank line or anything we do not interpret.
    Raw(String),
    Section(String),
    Entry { key: String, raw_key: String, value: String },
}

#[derive(Debug, Clone)]
pub struct IniDoc {
    lines: Vec<Line>,
    eol: &'static str,
    bom: bool,
    final_newline: bool,
}

fn trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\r'])
}

/// Canonical key: lower case, without the `TOTK_` or `KLM_` prefix (the runtime accepts both).
pub fn norm_key(k: &str) -> String {
    let l = k.to_ascii_lowercase();
    l.strip_prefix("totk_").or_else(|| l.strip_prefix("klm_")).map(str::to_string).unwrap_or(l)
}

impl IniDoc {
    pub fn empty() -> Self {
        IniDoc { lines: Vec::new(), eol: "\r\n", bom: false, final_newline: true }
    }

    pub fn parse(text: &str) -> Self {
        let (bom, text) = match text.strip_prefix('\u{feff}') {
            Some(t) => (true, t),
            None => (false, text),
        };
        let eol = if text.contains("\r\n") || !text.contains('\n') { "\r\n" } else { "\n" };
        let final_newline = text.is_empty() || text.ends_with('\n');
        let mut lines = Vec::new();
        for raw in text.split('\n') {
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            let s = trim(raw);
            let line = if s.starts_with('[') {
                let name = s.trim_start_matches('[').split(']').next().unwrap_or("");
                Line::Section(name.trim().to_string())
            } else if s.is_empty() || s.starts_with('#') || s.starts_with(';') {
                Line::Raw(raw.to_string())
            } else if let Some((k, v)) = s.split_once('=') {
                let mut v = trim(v).to_string();
                if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
                    v = v[1..v.len() - 1].to_string();
                }
                let raw_key = trim(k).to_string();
                Line::Entry { key: norm_key(&raw_key), raw_key, value: v }
            } else {
                Line::Raw(raw.to_string())
            };
            lines.push(line);
        }
        // `split('\n')` yields one extra empty element after a final newline.
        if final_newline && matches!(lines.last(), Some(Line::Raw(r)) if r.is_empty()) {
            lines.pop();
        }
        IniDoc { lines, eol, bom, final_newline }
    }

    /// Effective value of `key` as the runtime sees it: the last line with a non-empty value.
    pub fn get(&self, key: &str) -> Option<&str> {
        let k = norm_key(key);
        self.lines.iter().rev().find_map(|l| match l {
            Line::Entry { key, value, .. } if *key == k && !value.is_empty() => Some(value.as_str()),
            _ => None,
        })
    }

    /// All effective `(key, value)` pairs, in file order of their first definition.
    pub fn entries(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for l in &self.lines {
            if let Line::Entry { key, value, .. } = l {
                if value.is_empty() {
                    continue;
                }
                if let Some(slot) = out.iter_mut().find(|(k, _)| k == key) {
                    slot.1 = value.clone();
                } else {
                    out.push((key.clone(), value.clone()));
                }
            }
        }
        out
    }

    /// Sets `key`. If the key already has lines, the last one is rewritten in place; otherwise a new line goes to
    /// the end of `section` (created at the end of the file when missing).
    pub fn set(&mut self, key: &str, value: &str, section: &str) {
        let k = norm_key(key);
        let last = self.lines.iter().rposition(|l| matches!(l, Line::Entry { key, .. } if *key == k));
        if let Some(i) = last {
            if let Line::Entry { value: v, .. } = &mut self.lines[i] {
                *v = value.to_string();
            }
            return;
        }
        let entry = Line::Entry { key: k.clone(), raw_key: k, value: value.to_string() };
        let header = self.lines.iter().position(|l| matches!(l, Line::Section(s) if s.eq_ignore_ascii_case(section)));
        match header {
            Some(h) => {
                let end = self.lines[h + 1..].iter().position(|l| matches!(l, Line::Section(_))).map_or(self.lines.len(), |p| h + 1 + p);
                let at = self.lines[h + 1..end].iter().rposition(|l| matches!(l, Line::Entry { .. })).map_or(h + 1, |p| h + 1 + p + 1);
                self.lines.insert(at, entry);
            }
            None => {
                let needs_gap = match self.lines.last() {
                    None => false,
                    Some(Line::Raw(r)) => !r.trim().is_empty(),
                    Some(_) => true,
                };
                if needs_gap {
                    self.lines.push(Line::Raw(String::new()));
                }
                self.lines.push(Line::Section(section.to_string()));
                self.lines.push(entry);
            }
        }
        self.final_newline = true;
    }

    /// Removes every line that defines `key` (the setting goes back to the runtime default).
    pub fn remove(&mut self, key: &str) -> bool {
        let k = norm_key(key);
        let before = self.lines.len();
        self.lines.retain(|l| !matches!(l, Line::Entry { key, .. } if *key == k));
        before != self.lines.len()
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        if self.bom {
            out.push('\u{feff}');
        }
        for (i, l) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push_str(self.eol);
            }
            match l {
                Line::Raw(r) => out.push_str(r),
                Line::Section(s) => {
                    out.push('[');
                    out.push_str(s);
                    out.push(']');
                }
                Line::Entry { raw_key, value, .. } => {
                    out.push_str(raw_key);
                    out.push_str(" = ");
                    out.push_str(value);
                }
            }
        }
        if self.final_newline && !self.lines.is_empty() {
            out.push_str(self.eol);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# totk.ini\r\n\r\n[display]\r\n# window_size = 1920x1080\r\nfullscreen = 0\r\nWINDOW_SIZE = 1280x720\r\nweird_key = keep me\r\n\r\n[input]\r\nbind_A = pad:south, key:E\r\n; note\r\nno equals here\r\n";

    #[test]
    fn reads_like_the_runtime() {
        let d = IniDoc::parse(SAMPLE);
        assert_eq!(d.get("window_size"), Some("1280x720"));
        assert_eq!(d.get("TOTK_FULLSCREEN"), Some("0"));
        assert_eq!(d.get("bind_a"), Some("pad:south, key:E"));
        assert_eq!(d.get("missing"), None);
    }

    #[test]
    fn quotes_are_stripped_empty_is_default_and_later_line_wins() {
        let d = IniDoc::parse("q = \"hello\"\na = 1\na =\nb = 2\nb = 3\n");
        assert_eq!(d.get("q"), Some("hello"));
        assert_eq!(d.get("a"), Some("1"));
        assert_eq!(d.get("b"), Some("3"));
    }

    #[test]
    fn roundtrip_is_byte_identical() {
        assert_eq!(IniDoc::parse(SAMPLE).serialize(), SAMPLE);
        let lf = "[a]\nx = 1\n\n# c\n";
        assert_eq!(IniDoc::parse(lf).serialize(), lf);
        let bom = "\u{feff}[a]\r\nx = 1";
        assert_eq!(IniDoc::parse(bom).serialize(), bom);
        assert_eq!(IniDoc::parse("").serialize(), "");
    }

    #[test]
    fn set_rewrites_last_line_and_keeps_the_rest() {
        let mut d = IniDoc::parse(SAMPLE);
        d.set("window_size", "1600x900", "display");
        let out = d.serialize();
        assert!(out.contains("WINDOW_SIZE = 1600x900\r\n"));
        assert!(out.contains("weird_key = keep me"));
        assert!(out.contains("no equals here"));
        assert!(out.contains("# window_size = 1920x1080"));
        assert!(out.contains("; note"));
    }

    #[test]
    fn set_new_key_goes_into_its_section() {
        let mut d = IniDoc::parse(SAMPLE);
        d.set("fps_limit", "60", "frame rate");
        d.set("vk_present", "fifo", "display");
        let out = d.serialize();
        let display = out.find("[display]").unwrap();
        let input = out.find("[input]").unwrap();
        let vk = out.find("vk_present = fifo").unwrap();
        assert!(display < vk && vk < input, "{out}");
        assert!(out.contains("[frame rate]\r\nfps_limit = 60"));
        assert_eq!(IniDoc::parse(&out).get("fps_limit"), Some("60"));
    }

    #[test]
    fn remove_drops_all_definitions_only() {
        let mut d = IniDoc::parse("x = 1\nX = 2\ny = 3\n");
        assert!(d.remove("x"));
        assert!(!d.remove("x"));
        assert_eq!(d.serialize(), "y = 3\n");
    }

    #[test]
    fn set_on_empty_document_creates_section() {
        let mut d = IniDoc::empty();
        d.set("mouse_sens", "2", "input");
        assert_eq!(d.serialize(), "[input]\r\nmouse_sens = 2\r\n");
    }

    #[test]
    fn entries_lists_effective_pairs() {
        let d = IniDoc::parse("a=1\nb=2\na=5\nc=\n");
        assert_eq!(d.entries(), vec![("a".into(), "5".into()), ("b".into(), "2".into())]);
    }
}
