//! Runtime settings (`kalamar.ini`, else the legacy `totk.ini`; see `install::ini_path`): a data-driven schema, validation, and load/save through [`IniDoc`].
//!
//! The list of editable keys lives in `resources/settings-schema.json` (embedded; a `settings-schema.json` in the
//! launcher data directory replaces it). Adding a key there adds it to the UI and to the validation.

use crate::error::{AppError, AppResult, FieldError};
use crate::ini::{IniDoc, norm_key};
use crate::util;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

const EMBEDDED_SCHEMA: &str = include_str!("../resources/settings-schema.json");
const MAX_INI_BYTES: u64 = 4 << 20;

type Labels = BTreeMap<String, String>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Bool,
    Int,
    Float,
    Enum,
    Size,
    ChoiceInt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    pub key: String,
    pub section: String,
    pub ini_section: String,
    #[serde(rename = "type")]
    pub kind: FieldKind,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub min_size: Option<[u32; 2]>,
    #[serde(default)]
    pub max_size: Option<[u32; 2]>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub presets: Vec<String>,
    /// Show a slider (with `min`, `max`, `step`) next to the number box.
    #[serde(default)]
    pub slider: bool,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub label: Labels,
    #[serde(default)]
    pub help: Labels,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSection {
    pub id: String,
    pub label: Labels,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ButtonSpec {
    pub id: String,
    pub label: Labels,
    /// The default keys, as the documentation lists them (display only).
    #[serde(default)]
    pub default: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingSpec {
    pub ini_section: String,
    pub buttons: Vec<ButtonSpec>,
    pub key_only_buttons: Vec<ButtonSpec>,
    pub pad_names: Vec<String>,
    pub key_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub version: u32,
    pub sections: Vec<UiSection>,
    pub fields: Vec<Field>,
    pub bindings: BindingSpec,
}

impl Schema {
    pub fn embedded() -> Schema {
        serde_json::from_str(EMBEDDED_SCHEMA).expect("embedded settings-schema.json is valid (checked by a test)")
    }

    /// The data-directory override if it parses, else the embedded schema.
    pub fn load(data_dir: &Path) -> Schema {
        let path = data_dir.join("settings-schema.json");
        if let Ok(Some(text)) = util::read_text_limited(&path, 1 << 20)
            && let Ok(s) = serde_json::from_str::<Schema>(&text) {
                return s;
            }
        Schema::embedded()
    }

    fn field(&self, key: &str) -> Option<&Field> {
        let k = norm_key(key);
        self.fields.iter().find(|f| f.key == k)
    }

    fn binding_button(&self, key: &str) -> Option<(String, bool)> {
        let k = norm_key(key);
        let id = k.strip_prefix("bind_")?.to_ascii_uppercase();
        if self.bindings.buttons.iter().any(|b| b.id == id) {
            Some((id, false))
        } else if self.bindings.key_only_buttons.iter().any(|b| b.id == id) {
            Some((id, true))
        } else {
            None
        }
    }

    fn is_known_key(&self, key: &str) -> bool {
        self.field(key).is_some() || self.binding_button(key).is_some()
    }
}

fn fmt_f64(v: f64) -> String {
    format!("{v}")
}

fn parse_size(s: &str) -> Option<[u32; 2]> {
    let (w, h) = s.split_once(['x', 'X'])?;
    let w: u32 = w.trim().parse().ok()?;
    let h: u32 = h.trim().parse().ok()?;
    Some([w, h])
}

/// Validates `raw` for `field` and returns the canonical text to write.
pub fn validate_field(field: &Field, raw: &str) -> Result<String, String> {
    let v = raw.trim();
    if v.is_empty() {
        return Err("empty value (remove the setting to use the default)".into());
    }
    match field.kind {
        FieldKind::Bool => match v.to_ascii_lowercase().as_str() {
            "0" | "false" | "off" | "no" => Ok("0".into()),
            "1" | "true" | "on" | "yes" => Ok("1".into()),
            _ => Err("expected 0 or 1".into()),
        },
        FieldKind::Int => {
            let n: i64 = v.parse().map_err(|_| "expected a whole number".to_string())?;
            check_range(n as f64, field)?;
            Ok(n.to_string())
        }
        FieldKind::Float => {
            let n: f64 = v.replace(',', ".").parse().map_err(|_| "expected a number".to_string())?;
            if !n.is_finite() {
                return Err("expected a finite number".into());
            }
            check_range(n, field)?;
            Ok(fmt_f64(n))
        }
        FieldKind::Enum => field
            .options
            .iter()
            .find(|o| o.eq_ignore_ascii_case(v))
            .cloned()
            .ok_or_else(|| format!("expected one of: {}", field.options.join(", "))),
        FieldKind::ChoiceInt => {
            if let Some(o) = field.options.iter().find(|o| o.eq_ignore_ascii_case(v)) {
                return Ok(o.clone());
            }
            let n: i64 = v.parse().map_err(|_| format!("expected {} or a whole number", field.options.join(", ")))?;
            check_range(n as f64, field)?;
            Ok(n.to_string())
        }
        FieldKind::Size => {
            let [w, h] = parse_size(v).ok_or_else(|| "expected WIDTHxHEIGHT, for example 1920x1080".to_string())?;
            let [minw, minh] = field.min_size.unwrap_or([1, 1]);
            let [maxw, maxh] = field.max_size.unwrap_or([u32::MAX, u32::MAX]);
            if w < minw || h < minh || w > maxw || h > maxh {
                return Err(format!("size must be between {minw}x{minh} and {maxw}x{maxh}"));
            }
            Ok(format!("{w}x{h}"))
        }
    }
}

fn check_range(n: f64, field: &Field) -> Result<(), String> {
    if let Some(min) = field.min
        && n < min {
            return Err(format!("must be at least {}", fmt_f64(min)));
        }
    if let Some(max) = field.max
        && n > max {
            return Err(format!("must be at most {}", fmt_f64(max)));
        }
    Ok(())
}

/// Validates a `bind_*` value: comma-separated `pad:<name>` / `key:<name>` entries, `key:default`, `pad:default`
/// or the single word `none`. Stick directions and SHAKE take keys only. Returns the canonical text.
pub fn validate_binding(spec: &BindingSpec, key_only: bool, raw: &str) -> Result<String, String> {
    let mut out: Vec<String> = Vec::new();
    for part in raw.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let lower = p.to_ascii_lowercase();
        if lower == "none" {
            out.push("none".into());
            continue;
        }
        let (kind, name) = lower.split_once(':').ok_or_else(|| format!("'{p}': expected pad:<name> or key:<name>"))?;
        let name = name.trim();
        match kind.trim() {
            "pad" => {
                if key_only {
                    return Err(format!("'{p}': this action takes keys only"));
                }
                if name != "default" && !spec.pad_names.iter().any(|n| n == name) {
                    return Err(format!("'{p}': unknown pad button"));
                }
                out.push(format!("pad:{name}"));
            }
            "key" => {
                if name != "default" && !spec.key_names.iter().any(|n| n == name) {
                    return Err(format!("'{p}': unknown key"));
                }
                out.push(format!("key:{name}"));
            }
            _ => return Err(format!("'{p}': expected pad:<name> or key:<name>")),
        }
    }
    if out.is_empty() {
        return Err("no entry".into());
    }
    if out.iter().any(|e| e == "none") && out.len() > 1 {
        return Err("'none' cannot be combined with other entries".into());
    }
    Ok(out.join(", "))
}

#[derive(Debug, Clone, Deserialize)]
pub struct Change {
    pub key: String,
    /// `None` (or an empty string) removes the line: the runtime default applies again.
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsState {
    pub path: String,
    pub exists: bool,
    /// Effective value of every known key present in the file (canonical lower-case key names).
    pub values: BTreeMap<String, String>,
    /// Keys in the file that the schema does not describe; they are kept untouched on save.
    pub unknown_keys: Vec<String>,
}

fn state_of(path: &Path, exists: bool, doc: &IniDoc, schema: &Schema) -> SettingsState {
    let mut values = BTreeMap::new();
    let mut unknown_keys = Vec::new();
    for (k, v) in doc.entries() {
        if schema.is_known_key(&k) {
            values.insert(k, v);
        } else {
            unknown_keys.push(k);
        }
    }
    SettingsState { path: path.display().to_string(), exists, values, unknown_keys }
}

fn read_doc(path: &Path) -> AppResult<(IniDoc, bool)> {
    match util::read_text_limited(path, MAX_INI_BYTES)? {
        Some(t) => Ok((IniDoc::parse(&t), true)),
        None => Ok((IniDoc::empty(), false)),
    }
}

pub fn load(path: &Path, schema: &Schema) -> AppResult<SettingsState> {
    let (doc, exists) = read_doc(path)?;
    Ok(state_of(path, exists, &doc, schema))
}

/// The value of one key of the settings file (None: no file, or the key is not there).
pub fn read_value(path: &Path, key: &str) -> Option<String> {
    let (doc, _) = read_doc(path).ok()?;
    doc.get(key).map(str::to_string)
}

/// The parsed settings file (empty when there is none).
pub fn read_ini(path: &Path) -> IniDoc {
    read_doc(path).map(|(d, _)| d).unwrap_or_else(|_| IniDoc::empty())
}

/// Validates every change first (nothing is written if any is invalid), applies them to the freshly read file and
/// writes it atomically.
pub fn save(path: &Path, schema: &Schema, changes: &[Change]) -> AppResult<SettingsState> {
    enum Op {
        Set { key: String, value: String, section: String },
        Remove(String),
    }
    let mut ops = Vec::new();
    let mut errors = Vec::new();
    for c in changes {
        let key = norm_key(&c.key);
        let value = c.value.as_deref().map(str::trim).filter(|v| !v.is_empty());
        if let Some(field) = schema.field(&key) {
            match value {
                None => ops.push(Op::Remove(key)),
                Some(v) => match validate_field(field, v) {
                    Ok(canon) => ops.push(Op::Set { key, value: canon, section: field.ini_section.clone() }),
                    Err(message) => errors.push(FieldError { key, message }),
                },
            }
        } else if let Some((_, key_only)) = schema.binding_button(&key) {
            match value {
                None => ops.push(Op::Remove(key)),
                Some(v) => match validate_binding(&schema.bindings, key_only, v) {
                    Ok(canon) => ops.push(Op::Set { key, value: canon, section: schema.bindings.ini_section.clone() }),
                    Err(message) => errors.push(FieldError { key, message }),
                },
            }
        } else {
            errors.push(FieldError { key, message: "unknown setting".into() });
        }
    }
    if !errors.is_empty() {
        return Err(AppError::Validation(errors));
    }
    let (mut doc, exists) = read_doc(path)?;
    let mut changed = false;
    for op in ops {
        match op {
            Op::Set { key, value, section } => {
                if doc.get(&key) != Some(value.as_str()) {
                    doc.set(&key, &value, &section);
                    changed = true;
                }
            }
            Op::Remove(key) => changed |= doc.remove(&key),
        }
    }
    if changed || !exists {
        util::atomic_write(path, doc.serialize().as_bytes())?;
    }
    Ok(state_of(path, changed || exists, &doc, schema))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Schema {
        Schema::embedded()
    }
    fn f<'a>(s: &'a Schema, k: &str) -> &'a Field {
        s.field(k).unwrap()
    }
    fn tmp_ini(tag: &str, content: Option<&str>) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("s3l-set-{tag}-{}", util::random_hex(4)));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("totk.ini");
        if let Some(c) = content {
            std::fs::write(&p, c).unwrap();
        }
        p
    }
    fn ch(k: &str, v: Option<&str>) -> Change {
        Change { key: k.into(), value: v.map(str::to_string) }
    }

    #[test]
    fn mouse_aim_settings_follow_the_documented_ranges() {
        let s = schema();
        for k in ["mouse_gyro", "mouse_gyro_sens", "mouse_gyro_cm360", "mouse_gyro_dpi", "mouse_gyro_x", "mouse_gyro_y", "mouse_gyro_invert_y", "mouse_gyro_pitch", "mouse_gyro_attitude", "mouse_gyro_recenter_key"] {
            assert_eq!(f(&s, k).ini_section, "input", "{k}");
        }
        assert_eq!(validate_field(f(&s, "mouse_gyro_sens"), "0.04").unwrap(), "0.04");
        assert!(validate_field(f(&s, "mouse_gyro_sens"), "0.001").is_err());
        assert!(validate_field(f(&s, "mouse_gyro_sens"), "1.5").is_err());
        assert_eq!(validate_field(f(&s, "mouse_gyro_cm360"), "11.9").unwrap(), "11.9");
        assert!(validate_field(f(&s, "mouse_gyro_cm360"), "200").is_ok());
        assert!(validate_field(f(&s, "mouse_gyro_cm360"), "201").is_err());
        assert!(validate_field(f(&s, "mouse_gyro_cm360"), "-1").is_err());
        assert!(validate_field(f(&s, "mouse_gyro_dpi"), "100").is_ok());
        assert!(validate_field(f(&s, "mouse_gyro_dpi"), "99").is_err());
        assert!(validate_field(f(&s, "mouse_gyro_dpi"), "32000").is_ok());
        assert!(validate_field(f(&s, "mouse_gyro_dpi"), "32001").is_err());
        assert!(validate_field(f(&s, "mouse_gyro_x"), "4").is_ok());
        assert!(validate_field(f(&s, "mouse_gyro_y"), "0.05").is_err());
        assert!(validate_field(f(&s, "mouse_gyro_pitch"), "90").is_err());
        assert!(validate_field(f(&s, "mouse_gyro"), "auto").is_ok());
        assert!(validate_field(f(&s, "mouse_gyro_attitude"), "upright").is_ok());
        assert!(validate_field(f(&s, "mouse_gyro_recenter_key"), "mouse_x1").is_ok());
        assert!(validate_field(f(&s, "mouse_gyro_recenter_key"), "f1").is_err()); // F1 and Esc cannot be bound
        assert!(f(&s, "mouse_gyro_sens").slider);
    }

    #[test]
    fn splatoon_key_layout_is_listed_with_defaults() {
        let s = schema();
        let all: Vec<_> = s.bindings.buttons.iter().chain(&s.bindings.key_only_buttons).collect();
        let d = |id: &str| all.iter().find(|b| b.id == id).map(|b| b.default.as_str());
        assert_eq!(d("ZR"), Some("Mouse left"));
        assert_eq!(d("ZL"), Some("Left Shift"));
        assert_eq!(d("B"), Some("Space"));
        assert_eq!(d("RCLICK"), Some("E, Mouse middle"));
        assert_eq!(d("PLUS"), Some("Enter"));
        assert_eq!(d("LSTICK_UP"), Some("W"));
        assert!(all.iter().all(|b| !b.default.is_empty()));
    }

    #[test]
    fn embedded_schema_is_consistent() {
        let s = schema();
        let mut seen = std::collections::BTreeSet::new();
        for fld in &s.fields {
            assert!(seen.insert(fld.key.clone()), "duplicate {}", fld.key);
            assert!(s.sections.iter().any(|x| x.id == fld.section), "section of {}", fld.key);
            assert!(fld.label.contains_key("fr") && fld.label.contains_key("en"), "labels of {}", fld.key);
            if let Some(d) = &fld.default {
                assert!(validate_field(fld, d).is_ok(), "default of {}", fld.key);
            }
            for p in &fld.presets {
                assert!(validate_field(fld, p).is_ok(), "preset {p} of {}", fld.key);
            }
        }
        assert_eq!(s.bindings.buttons.len(), 16);
    }

    #[test]
    fn numeric_validation() {
        let s = schema();
        assert_eq!(validate_field(f(&s, "fps_limit"), "60").unwrap(), "60");
        assert_eq!(validate_field(f(&s, "fps_limit"), "DISPLAY").unwrap(), "display");
        assert!(validate_field(f(&s, "fps_limit"), "29").is_err());
        assert!(validate_field(f(&s, "fps_limit"), "241").is_err());
        assert!(validate_field(f(&s, "fps_limit"), "abc").is_err());
        assert_eq!(validate_field(f(&s, "mouse_sens"), "2,5").unwrap(), "2.5");
        assert_eq!(validate_field(f(&s, "mouse_sens"), "1.0").unwrap(), "1");
        assert!(validate_field(f(&s, "mouse_sens"), "11").is_err());
        assert!(validate_field(f(&s, "mouse_sens"), "NaN").is_err());
        assert!(validate_field(f(&s, "mouse_sens"), "-1").is_err());
        assert!(validate_field(f(&s, "stick_deadzone"), "8.5").is_err());
        assert_eq!(validate_field(f(&s, "render_scale"), "game").unwrap(), "game");
        assert!(validate_field(f(&s, "render_scale"), "24").is_err());
    }

    #[test]
    fn size_enum_bool_validation() {
        let s = schema();
        assert_eq!(validate_field(f(&s, "window_size"), "1280X720").unwrap(), "1280x720");
        assert!(validate_field(f(&s, "window_size"), "100x100").is_err());
        assert!(validate_field(f(&s, "window_size"), "8000x4000").is_err());
        assert!(validate_field(f(&s, "window_size"), "1920").is_err());
        assert_eq!(validate_field(f(&s, "vk_present"), "FIFO").unwrap(), "fifo");
        assert!(validate_field(f(&s, "vk_present"), "tripple").is_err());
        assert_eq!(validate_field(f(&s, "fullscreen"), "true").unwrap(), "1");
        assert!(validate_field(f(&s, "fullscreen"), "2").is_err());
        assert!(validate_field(f(&s, "fullscreen"), "").is_err());
    }

    #[test]
    fn binding_validation() {
        let b = &schema().bindings;
        assert_eq!(validate_binding(b, false, "pad:South,  KEY:E").unwrap(), "pad:south, key:e");
        assert_eq!(validate_binding(b, false, "key:default").unwrap(), "key:default");
        assert_eq!(validate_binding(b, false, "none").unwrap(), "none");
        assert!(validate_binding(b, false, "none, key:e").is_err());
        assert!(validate_binding(b, false, "pad:nope").is_err());
        assert!(validate_binding(b, false, "key:f1").is_err()); // F1 cannot be bound
        assert!(validate_binding(b, false, "e").is_err());
        assert!(validate_binding(b, true, "pad:south").is_err());
        assert!(validate_binding(b, true, "key:w").is_ok());
        assert!(validate_binding(b, false, " , ").is_err());
    }

    #[test]
    fn save_preserves_unknown_keys_comments_and_order() {
        let original = "# my notes\r\n[display]\r\nmystery_key = 42\r\nfullscreen = 0\r\n\r\n[diagnostics]\r\nastc_time = 1\r\n; keep\r\n";
        let p = tmp_ini("keep", Some(original));
        let s = schema();
        let st = save(&p, &s, &[ch("fullscreen", Some("1")), ch("fps_limit", Some("90")), ch("bind_a", Some("key:e, pad:south"))]).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("# my notes"));
        assert!(text.contains("mystery_key = 42"));
        assert!(text.contains("astc_time = 1"));
        assert!(text.contains("; keep"));
        assert!(text.contains("fullscreen = 1"));
        assert!(text.contains("[frame rate]\r\nfps_limit = 90"));
        assert!(text.contains("[input]\r\nbind_a = key:e, pad:south"));
        assert_eq!(st.values.get("fullscreen").map(String::as_str), Some("1"));
        assert!(st.unknown_keys.contains(&"mystery_key".to_string()));
        assert!(st.unknown_keys.contains(&"astc_time".to_string()));
        // The text before the first change is byte-identical up to the changed line.
        assert!(text.starts_with("# my notes\r\n[display]\r\nmystery_key = 42\r\nfullscreen = 1\r\n"));
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }

    #[test]
    fn invalid_change_writes_nothing() {
        let p = tmp_ini("invalid", Some("fullscreen = 0\n"));
        let before = std::fs::read(&p).unwrap();
        let err = save(&p, &schema(), &[ch("fullscreen", Some("1")), ch("mouse_sens", Some("99")), ch("nope", Some("1"))]).unwrap_err();
        match err {
            AppError::Validation(v) => {
                assert_eq!(v.len(), 2);
                assert_eq!(v[0].key, "mouse_sens");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(std::fs::read(&p).unwrap(), before);
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }

    #[test]
    fn remove_restores_default_and_creates_file_when_missing() {
        let p = tmp_ini("missing", None);
        let s = schema();
        let st = save(&p, &s, &[ch("mouse_sens", Some("3"))]).unwrap();
        assert!(st.exists);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[input]\r\nmouse_sens = 3\r\n");
        let st = save(&p, &s, &[ch("mouse_sens", None)]).unwrap();
        assert!(st.values.is_empty());
        assert!(!std::fs::read_to_string(&p).unwrap().contains("mouse_sens"));
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }

    #[test]
    fn unchanged_values_do_not_touch_the_file() {
        let p = tmp_ini("noop", Some("# c\nfullscreen = 1\n"));
        let m1 = std::fs::metadata(&p).unwrap().modified().unwrap();
        save(&p, &schema(), &[ch("fullscreen", Some("true"))]).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), m1);
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }

    #[test]
    fn prefix_and_case_are_normalised() {
        let p = tmp_ini("prefix", Some("TOTK_FULLSCREEN = 0\n"));
        let st = load(&p, &schema()).unwrap();
        assert_eq!(st.values.get("fullscreen").map(String::as_str), Some("0"));
        save(&p, &schema(), &[ch("Fullscreen", Some("1"))]).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "TOTK_FULLSCREEN = 1\n");
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
}
