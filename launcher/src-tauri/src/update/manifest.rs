//! The signed release manifest: the only thing that says which files make a release, and the only text of a release the
//! launcher shows (its notes). A binary is never trusted because of where it comes from.
//!
//! A release carries `kalamar-release.json` and `kalamar-release.json.minisig`, a minisign signature (Ed25519 over the
//! BLAKE2b-512 hash of the exact bytes of the manifest, the "prehashed" mode of minisign 0.10+; legacy signatures are
//! refused) made with the maintainers' secret key. The launcher holds the public keys (`channel.rs`, fixed at build time):
//! the signature must come from one of them, and only then is the manifest read, strictly (unknown fields refuse it):
//!
//! ```json
//! { "format": 1, "product": "kalamar", "repo": "owner/repo", "version": "0.2.0", "channel": "stable",
//!   "published": "2026-11-12T10:00:00Z", "notes": "plain text",
//!   "files": [ { "name": "Kalamar.exe", "kind": "portable", "size": 41234567, "sha256": "<64 hex>" },
//!              { "name": "Kalamar_0.2.0_x64-setup.exe", "kind": "installer", "size": 39876543, "sha256": "<64 hex>" } ] }
//! ```
//!
//! Refused: a version that is not above the running one (no going back), a version other than the release tag's, a
//! repository other than the configured one, a beta manifest on the stable channel, file names with a path in them.

use super::version::Version;
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};

pub const MANIFEST_NAME: &str = "kalamar-release.json";
pub const SIGNATURE_NAME: &str = "kalamar-release.json.minisig";
pub const MAX_MANIFEST: usize = 64 * 1024;
pub const MAX_SIGNATURE: usize = 2048;
/// Larger than any build of the launcher (about 40 MB): a bigger file is refused before it is downloaded.
pub const MAX_FILE: u64 = 512 * 1024 * 1024;
/// Characters of release notes kept (the rest is cut).
pub const MAX_NOTES: usize = 8000;
pub const FORMAT: u32 = 1;
pub const PRODUCT: &str = "kalamar";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Stable,
    Beta,
}

impl Channel {
    pub fn parse(s: &str) -> Option<Channel> {
        match s {
            "stable" => Some(Channel::Stable),
            "beta" => Some(Channel::Beta),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    /// The lone `Kalamar.exe`.
    Portable,
    /// The NSIS installer of the same version.
    Installer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub kind: FileKind,
    pub size: u64,
    /// Lower-case hex.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub version: Version,
    pub channel: Channel,
    pub repo: String,
    pub published: Option<String>,
    /// Plain text, control and bidirectional-override characters removed, at most [`MAX_NOTES`] characters.
    pub notes: String,
    pub files: Vec<FileEntry>,
}

impl Manifest {
    pub fn file(&self, kind: FileKind) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.kind == kind)
    }
}

/// Why a manifest is not used. Every case means "do not install"; none is shown with details to the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    TooLarge,
    /// Not a minisign signature, or a legacy (not prehashed) one.
    SignatureFormat,
    /// Signed with a key this launcher does not hold.
    UnknownKey,
    /// The signature does not match the manifest's bytes.
    BadSignature,
    /// Signed, but not a manifest of the expected shape.
    Format(String),
    WrongProduct,
    WrongRepo,
    /// The signed version is not the one of the release it was found in.
    VersionMismatch,
    /// Not above the running version.
    NotNewer,
    /// A beta manifest seen on the stable channel.
    WrongChannel,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Format(m) => write!(f, "manifest format: {m}"),
            other => write!(f, "{other:?}"),
        }
    }
}

/// What the manifest must match.
pub struct Expect<'a> {
    pub repo: &'a str,
    /// The version of the release tag the manifest was downloaded from (`None` when re-reading a stored manifest).
    pub tag_version: Option<&'a Version>,
    pub current: &'a Version,
    pub channel: Channel,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    name: String,
    kind: String,
    size: u64,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    format: u32,
    product: String,
    repo: String,
    version: String,
    channel: String,
    #[serde(default)]
    published: Option<String>,
    #[serde(default)]
    notes: String,
    files: Vec<RawFile>,
}

/// The public keys of a `;`-separated list of minisign public key lines (the base64 line of a `minisign.pub`).
pub fn parse_keys(list: &str) -> Result<Vec<PublicKey>, String> {
    list.split(';').map(str::trim).filter(|k| !k.is_empty()).map(|k| PublicKey::from_base64(k).map_err(|e| format!("public key {k}: {e}"))).collect()
}

/// A release file name: letters, digits, `.`, `_`, `-`, not starting with a dot, at most 100 characters. No path.
pub fn valid_file_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && !s.starts_with('.') && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) && !s.contains("..")
}

/// `owner/repo` as GitHub allows them.
pub fn valid_repo(s: &str) -> bool {
    let Some((owner, repo)) = s.split_once('/') else { return false };
    let ok = |p: &str, max: usize| !p.is_empty() && p.len() <= max && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) && p != "." && p != "..";
    ok(owner, 39) && ok(repo, 100) && !owner.contains('.') && !owner.contains('_')
}

/// Removes what could make plain text misleading on screen: control characters (but line breaks and tabs), the
/// bidirectional overrides and isolates, zero-width characters; caps the length.
pub fn sanitize_notes(s: &str) -> String {
    let s = s.replace("\r\n", "\n");
    let kept: String = s
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
        .filter(|&c| !matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'))
        .take(MAX_NOTES)
        .collect();
    kept.trim().to_string()
}

/// Checks the signature with the launcher's keys, then reads and checks the manifest. Nothing of the manifest is
/// parsed before its signature is known good.
pub fn verify(manifest: &[u8], signature: &[u8], keys: &[PublicKey], expect: &Expect) -> Result<Manifest, Refusal> {
    if manifest.len() > MAX_MANIFEST || signature.len() > MAX_SIGNATURE {
        return Err(Refusal::TooLarge);
    }
    let sig_text = std::str::from_utf8(signature).map_err(|_| Refusal::SignatureFormat)?;
    let sig = Signature::decode(sig_text).map_err(|_| Refusal::SignatureFormat)?;
    let mut known_key = false;
    let mut good = false;
    for k in keys {
        match k.verify(manifest, &sig, false) {
            Ok(()) => {
                good = true;
                break;
            }
            Err(minisign_verify::Error::UnexpectedKeyId) => {}
            Err(minisign_verify::Error::UnexpectedAlgorithm | minisign_verify::Error::UnsupportedAlgorithm) => return Err(Refusal::SignatureFormat),
            Err(_) => known_key = true,
        }
    }
    if !good {
        return Err(if known_key { Refusal::BadSignature } else { Refusal::UnknownKey });
    }
    // Signed by a maintainer: now read it.
    let raw: Raw = serde_json::from_slice(manifest).map_err(|e| Refusal::Format(e.to_string()))?;
    if raw.format != FORMAT {
        return Err(Refusal::Format(format!("format {} (this launcher reads {FORMAT})", raw.format)));
    }
    if raw.product != PRODUCT {
        return Err(Refusal::WrongProduct);
    }
    if !valid_repo(&raw.repo) || !raw.repo.eq_ignore_ascii_case(expect.repo) {
        return Err(Refusal::WrongRepo);
    }
    let version = Version::parse(&raw.version).ok_or_else(|| Refusal::Format("version".into()))?;
    if expect.tag_version.is_some_and(|t| *t != version) {
        return Err(Refusal::VersionMismatch);
    }
    let channel = Channel::parse(&raw.channel).ok_or_else(|| Refusal::Format("channel".into()))?;
    if channel == Channel::Beta && expect.channel == Channel::Stable {
        return Err(Refusal::WrongChannel);
    }
    if version <= *expect.current {
        return Err(Refusal::NotNewer);
    }
    if raw.files.is_empty() || raw.files.len() > 4 {
        return Err(Refusal::Format("files".into()));
    }
    let mut files = Vec::new();
    for f in raw.files {
        let kind = match f.kind.as_str() {
            "portable" => FileKind::Portable,
            "installer" => FileKind::Installer,
            _ => return Err(Refusal::Format(format!("file kind {}", f.kind))),
        };
        if !valid_file_name(&f.name) || !f.name.to_ascii_lowercase().ends_with(".exe") {
            return Err(Refusal::Format("file name".into()));
        }
        if f.size == 0 || f.size > MAX_FILE {
            return Err(Refusal::Format("file size".into()));
        }
        if f.sha256.len() != 64 || !f.sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(Refusal::Format("sha256".into()));
        }
        if files.iter().any(|g: &FileEntry| g.kind == kind || g.name.eq_ignore_ascii_case(&f.name)) {
            return Err(Refusal::Format("duplicate file".into()));
        }
        files.push(FileEntry { name: f.name, kind, size: f.size, sha256: f.sha256 });
    }
    let published = raw.published.filter(|p| p.len() <= 40 && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b':' | b'.' | b'+')));
    Ok(Manifest { version, channel, repo: raw.repo, published, notes: sanitize_notes(&raw.notes), files })
}

#[cfg(test)]
mod tests {
    use super::super::testkit::{TestKey, manifest_json};
    use super::*;

    fn cur() -> Version {
        Version::parse("0.1.0").unwrap()
    }

    fn expect<'a>(cur: &'a Version, tag: Option<&'a Version>, channel: Channel) -> Expect<'a> {
        Expect { repo: "owner/kalamar", tag_version: tag, current: cur, channel }
    }

    fn good() -> (TestKey, Vec<u8>, Vec<u8>) {
        let k = TestKey::generate();
        let m = manifest_json("owner/kalamar", "0.2.0", "stable", &[("Kalamar.exe", "portable", 1234, &"ab".repeat(32))]);
        let s = k.sign(&m);
        (k, m, s)
    }

    #[test]
    fn a_good_manifest_is_read() {
        let (k, m, s) = good();
        let c = cur();
        let tag = Version::parse("0.2.0").unwrap();
        let got = verify(&m, &s, &[k.public()], &expect(&c, Some(&tag), Channel::Stable)).unwrap();
        assert_eq!(got.version.to_string(), "0.2.0");
        assert_eq!(got.file(FileKind::Portable).unwrap().size, 1234);
        assert!(got.file(FileKind::Installer).is_none());
        // the backup key listed second, the signing key first or second: both work
        let other = TestKey::generate();
        assert!(verify(&m, &s, &[other.public(), k.public()], &expect(&c, Some(&tag), Channel::Stable)).is_ok());
    }

    #[test]
    fn an_altered_manifest_is_refused() {
        let (k, m, s) = good();
        let c = cur();
        let mut bad = m.clone();
        let i = bad.iter().position(|&b| b == b'1').unwrap();
        bad[i] = b'9'; // the size 1234 becomes 9234
        assert_eq!(verify(&bad, &s, &[k.public()], &expect(&c, None, Channel::Stable)), Err(Refusal::BadSignature));
        // one byte added at the end
        let mut longer = m.clone();
        longer.push(b' ');
        assert_eq!(verify(&longer, &s, &[k.public()], &expect(&c, None, Channel::Stable)), Err(Refusal::BadSignature));
        // the signature itself altered
        let mut sig = String::from_utf8(s.clone()).unwrap();
        let line = sig.lines().nth(1).unwrap().to_string();
        let mut chars: Vec<char> = line.chars().collect();
        let j = chars.len() / 2;
        chars[j] = if chars[j] == 'A' { 'B' } else { 'A' };
        sig = sig.replacen(&line, &chars.into_iter().collect::<String>(), 1);
        assert!(matches!(verify(&m, sig.as_bytes(), &[k.public()], &expect(&c, None, Channel::Stable)), Err(Refusal::BadSignature | Refusal::SignatureFormat | Refusal::UnknownKey)));
        // the trusted comment changed: the global signature no longer matches
        let tampered = String::from_utf8(s.clone()).unwrap().replace("trusted comment: ", "trusted comment: x");
        assert_eq!(verify(&m, tampered.as_bytes(), &[k.public()], &expect(&c, None, Channel::Stable)), Err(Refusal::BadSignature));
    }

    #[test]
    fn the_wrong_key_is_refused() {
        let (_, m, s) = good();
        let c = cur();
        let stranger = TestKey::generate();
        assert_eq!(verify(&m, &s, &[stranger.public()], &expect(&c, None, Channel::Stable)), Err(Refusal::UnknownKey));
        assert_eq!(verify(&m, &s, &[], &expect(&c, None, Channel::Stable)), Err(Refusal::UnknownKey));
        // a key with the same id but other key bytes (forged id): the signature does not verify
        let forged = stranger.public_with_id_of(&TestKey::id_of(&s));
        assert_eq!(verify(&m, &s, &[forged], &expect(&c, None, Channel::Stable)), Err(Refusal::BadSignature));
    }

    #[test]
    fn garbage_signatures_are_refused() {
        let (k, m, _) = good();
        let c = cur();
        for sig in [&b""[..], b"not a signature", &[0xff, 0xfe, 0x00][..], &vec![b'a'; MAX_SIGNATURE + 1][..]] {
            assert!(matches!(verify(&m, sig, &[k.public()], &expect(&c, None, Channel::Stable)), Err(Refusal::SignatureFormat | Refusal::TooLarge)));
        }
        assert_eq!(verify(&vec![b' '; MAX_MANIFEST + 1], b"x", &[k.public()], &expect(&c, None, Channel::Stable)), Err(Refusal::TooLarge));
    }

    #[test]
    fn older_equal_or_mismatched_versions_are_refused() {
        let k = TestKey::generate();
        let file = [("Kalamar.exe", "portable", 10, "cd".repeat(32))];
        let files: Vec<(&str, &str, u64, &str)> = file.iter().map(|f| (f.0, f.1, f.2, f.3.as_str())).collect();
        let c = Version::parse("0.2.0").unwrap();
        for old in ["0.1.9", "0.2.0", "0.2.0-beta.3"] {
            let m = manifest_json("owner/kalamar", old, "stable", &files);
            assert_eq!(verify(&m, &k.sign(&m), &[k.public()], &expect(&c, None, Channel::Beta)), Err(Refusal::NotNewer), "{old}");
        }
        // a signed 0.3.0 manifest put in the release of tag v0.4.0
        let m = manifest_json("owner/kalamar", "0.3.0", "stable", &files);
        let tag = Version::parse("0.4.0").unwrap();
        assert_eq!(verify(&m, &k.sign(&m), &[k.public()], &expect(&c, Some(&tag), Channel::Stable)), Err(Refusal::VersionMismatch));
        // a manifest of another repository signed with the same key
        let m = manifest_json("someone/else", "0.3.0", "stable", &files);
        assert_eq!(verify(&m, &k.sign(&m), &[k.public()], &expect(&c, None, Channel::Stable)), Err(Refusal::WrongRepo));
        // beta manifests only on the beta channel
        let m = manifest_json("owner/kalamar", "0.3.0-beta.1", "beta", &files);
        assert_eq!(verify(&m, &k.sign(&m), &[k.public()], &expect(&c, None, Channel::Stable)), Err(Refusal::WrongChannel));
        assert!(verify(&m, &k.sign(&m), &[k.public()], &expect(&c, None, Channel::Beta)).is_ok());
    }

    #[test]
    fn signed_but_malformed_manifests_are_refused() {
        let k = TestKey::generate();
        let c = cur();
        let h = "ab".repeat(32);
        let cases: Vec<(String, &str)> = vec![
            (r#"{"format":1}"#.into(), "missing fields"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[("..\\Kalamar.exe", "portable", 5, &h)])).unwrap(), "path in a name"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[("sub/Kalamar.exe", "portable", 5, &h)])).unwrap(), "slash in a name"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[("Kalamar.zip", "portable", 5, &h)])).unwrap(), "not an exe"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[("Kalamar.exe", "portable", 0, &h)])).unwrap(), "empty file"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[("Kalamar.exe", "portable", MAX_FILE + 1, &h)])).unwrap(), "huge file"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[("Kalamar.exe", "portable", 5, "AB")])).unwrap(), "short hash"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[("Kalamar.exe", "portable", 5, &"AB".repeat(32))])).unwrap(), "upper-case hash"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[("Kalamar.exe", "script", 5, &h)])).unwrap(), "unknown kind"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[("A.exe", "portable", 5, &h), ("B.exe", "portable", 5, &h)])).unwrap(), "two portables"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "nightly", &[("A.exe", "portable", 5, &h)])).unwrap(), "unknown channel"),
            (String::from_utf8(manifest_json("owner/kalamar", "v0.2.0", "stable", &[("A.exe", "portable", 5, &h)])).unwrap(), "v in the version"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[])).unwrap(), "no file"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[("A.exe", "portable", 5, &h)])).unwrap().replace("\"format\":1", "\"format\":2"), "newer format"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[("A.exe", "portable", 5, &h)])).unwrap().replace("\"product\":\"kalamar\"", "\"product\":\"other\""), "other product"),
            (String::from_utf8(manifest_json("owner/kalamar", "0.2.0", "stable", &[("A.exe", "portable", 5, &h)])).unwrap().replace("\"notes\"", "\"run\":\"calc.exe\",\"notes\""), "unknown field"),
        ];
        for (text, why) in cases {
            let m = text.into_bytes();
            assert!(verify(&m, &k.sign(&m), &[k.public()], &expect(&c, None, Channel::Stable)).is_err(), "{why} should be refused");
        }
    }

    #[test]
    fn notes_are_plain_text_without_tricks() {
        assert_eq!(sanitize_notes("  Fixes\r\n- one\u{202E}cod.exe\u{2069}\n\u{0007}two\t \u{200B}"), "Fixes\n- onecod.exe\ntwo");
        assert_eq!(sanitize_notes(&"x".repeat(MAX_NOTES + 50)).chars().count(), MAX_NOTES);
        // HTML stays text: shown as typed, never interpreted (the page sets it as text)
        assert_eq!(sanitize_notes("<b>bold</b> [link](https://evil.example)"), "<b>bold</b> [link](https://evil.example)");
    }

    #[test]
    fn names_and_repos() {
        for ok in ["Kalamar.exe", "Kalamar_0.2.0_x64-setup.exe"] {
            assert!(valid_file_name(ok));
        }
        for bad in ["", ".hidden.exe", "a/b.exe", "a\\b.exe", "a..exe", "C:x.exe", "x y.exe", &"a".repeat(101)] {
            assert!(!valid_file_name(bad), "{bad}");
        }
        for ok in ["owner/repo", "Some-Owner/repo.name_2"] {
            assert!(valid_repo(ok));
        }
        for bad in ["", "owner", "owner/", "/repo", "a/b/c", "own.er/repo", "owner/..", "owner/re po", &format!("{}/r", "o".repeat(40))] {
            assert!(!valid_repo(bad), "{bad}");
        }
    }

    #[test]
    fn key_lists() {
        let (a, b) = (TestKey::generate(), TestKey::generate());
        let list = format!("{} ; {}", a.public_b64(), b.public_b64());
        assert_eq!(parse_keys(&list).unwrap().len(), 2);
        assert_eq!(parse_keys("").unwrap().len(), 0);
        assert!(parse_keys("RWQnotakey").is_err());
    }
}
