//! Release versions: `MAJOR.MINOR.PATCH` with an optional pre-release (`0.3.0-beta.1`), compared by Semantic Versioning
//! 2.0 precedence. Strict: no build metadata, no leading zeros, at most 64 characters, so that a version has exactly one
//! spelling and the version of a signed manifest can be compared with a tag as text.

use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ident {
    Num(u64),
    Alpha(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pre: Vec<Ident>,
}

fn number(s: &str) -> Option<u64> {
    if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0')) {
        return None;
    }
    s.parse().ok()
}

impl Version {
    /// `1.2.3` or `1.2.3-beta.1`; anything else (a `v`, spaces, `+build`, `01`) is refused.
    pub fn parse(s: &str) -> Option<Version> {
        if s.is_empty() || s.len() > 64 {
            return None;
        }
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let mut parts = core.split('.');
        let (major, minor, patch) = (number(parts.next()?)?, number(parts.next()?)?, number(parts.next()?)?);
        if parts.next().is_some() {
            return None;
        }
        let mut idents = Vec::new();
        if let Some(p) = pre {
            for id in p.split('.') {
                if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                    return None;
                }
                idents.push(if id.bytes().all(|b| b.is_ascii_digit()) { Ident::Num(number(id)?) } else { Ident::Alpha(id.to_string()) });
            }
        }
        Some(Version { major, minor, patch, pre: idents })
    }

    /// A release tag: the version, with or without a leading `v` (`v0.2.0`).
    pub fn parse_tag(tag: &str) -> Option<Version> {
        Version::parse(tag.strip_prefix('v').unwrap_or(tag))
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        for (i, id) in self.pre.iter().enumerate() {
            f.write_str(if i == 0 { "-" } else { "." })?;
            match id {
                Ident::Num(n) => write!(f, "{n}")?,
                Ident::Alpha(s) => f.write_str(s)?,
            }
        }
        Ok(())
    }
}

impl Ord for Ident {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Ident::Num(a), Ident::Num(b)) => a.cmp(b),
            (Ident::Num(_), Ident::Alpha(_)) => Ordering::Less,
            (Ident::Alpha(_), Ident::Num(_)) => Ordering::Greater,
            (Ident::Alpha(a), Ident::Alpha(b)) => a.as_bytes().cmp(b.as_bytes()),
        }
    }
}

impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)).then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            // a pre-release comes before its release
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self.pre.cmp(&other.pre),
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| panic!("{s} should parse"))
    }

    #[test]
    fn spellings() {
        for s in ["0.1.0", "1.20.300", "0.2.0-beta.1", "1.0.0-rc.2", "1.0.0-alpha-x.7", "0.0.0"] {
            assert_eq!(v(s).to_string(), s, "one spelling only");
        }
        assert_eq!(Version::parse_tag("v0.2.0"), Some(v("0.2.0")));
        assert_eq!(Version::parse_tag("0.2.0"), Some(v("0.2.0")));
        for bad in ["", "1", "1.2", "1.2.3.4", "01.2.3", "1.02.3", "1.2.3-", "1.2.3-beta..1", "1.2.3-01", "1.2.3+build", " 1.2.3", "1.2.3 ", "v1.2.3",
            "1.2.3-é", "1.2.-3", "1.2.3-beta_1", "9999999999.0.0", &format!("1.2.3-{}", "a".repeat(64))] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Version::parse_tag("vv1.2.3"), None);
        assert_eq!(Version::parse_tag("release-1.2.3"), None);
    }

    #[test]
    fn precedence() {
        // the order of semver.org, item 11
        let order = ["0.1.0", "0.1.1", "0.2.0-alpha", "0.2.0-alpha.1", "0.2.0-alpha.beta", "0.2.0-beta", "0.2.0-beta.2", "0.2.0-beta.11", "0.2.0-rc.1", "0.2.0", "0.10.0", "1.0.0", "2.0.0"];
        for w in order.windows(2) {
            assert!(v(w[0]) < v(w[1]), "{} < {}", w[0], w[1]);
            assert!(v(w[1]) > v(w[0]));
        }
        assert_eq!(v("1.2.3").cmp(&v("1.2.3")), Ordering::Equal);
        assert!(v("0.10.0") > v("0.9.9"), "numbers, not text");
        assert!(v("0.2.0-beta.1").is_prerelease() && !v("0.2.0").is_prerelease());
    }

    #[test]
    fn the_running_version_parses() {
        assert!(Version::parse(env!("CARGO_PKG_VERSION")).is_some());
    }
}
