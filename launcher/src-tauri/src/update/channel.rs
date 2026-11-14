//! Where this build looks for its updates, fixed when it is compiled (`build.rs`):
//!
//! - `KALAMAR_UPDATE_REPO` (environment variable of the build, `owner/repo`): the GitHub repository whose releases carry
//!   the launcher. Unset or empty (the default, and every development build): the update feature does not exist in
//!   that build: nothing in the interface, no network access. `scripts/make-release.ps1` sets it.
//! - `src-tauri/update-keys.txt`: the minisign public keys that may sign a release manifest, one per line (the base64
//!   line of a `minisign.pub`): the maintainers' current key first, then the backup key kept offline for a rotation. Read
//!   only when the repository is set; the build fails if it is set with no valid key.

use super::manifest;
use super::net::Source;

/// `owner/repo`, or empty.
pub const REPO: &str = env!("KALAMAR_UPDATE_REPO");
/// The keys of `update-keys.txt`, `;`-separated (empty without a repository).
pub const KEYS: &str = env!("KALAMAR_UPDATE_KEYS");

/// The configured source, or `None` (no update feature in this build).
pub fn source() -> Option<Source> {
    if REPO.is_empty() || !manifest::valid_repo(REPO) {
        return None;
    }
    let keys = manifest::parse_keys(KEYS).ok().filter(|k| !k.is_empty())?;
    Some(Source::github(REPO, keys))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key line in `update-keys.txt` is a minisign public key (the build only checks its shape).
    #[test]
    fn the_shipped_key_file_holds_only_valid_keys() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/update-keys.txt")).expect("update-keys.txt");
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        assert!(manifest::parse_keys(&lines.join(";")).is_ok());
        assert!(lines.len() <= 4, "the current key and a backup, a few at most");
    }

    #[test]
    fn a_build_without_a_repository_has_no_update_feature() {
        if REPO.is_empty() {
            assert!(source().is_none());
            assert!(KEYS.is_empty());
        } else {
            assert!(source().is_some_and(|s| !s.keys.is_empty() && s.repo == REPO));
        }
    }
}
