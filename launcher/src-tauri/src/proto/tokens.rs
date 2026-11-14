//! The login session of one server: access and refresh tokens (`api.md`, "Tokens"). Stored in the launcher's data
//! folder (`online\<server id>\session.dat`, never in a repository or a log), encrypted for the Windows user with DPAPI
//! (`CryptProtectData`): the file is useless on another account or machine. Removing the server deletes it; logging
//! out deletes it. The tokens never reach the page.

use crate::error::{AppError, AppResult};
use crate::util;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"KLMS1";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub access_token: String,
    #[serde(default)]
    pub access_expires_at: Option<String>,
    pub refresh_token: String,
    #[serde(default)]
    pub refresh_expires_at: Option<String>,
    /// The provider used to log in (`guest`, `dev`, `discord`...).
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub display_name: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print a token, not even in a test failure.
        f.debug_struct("Session").field("provider", &self.provider).field("user_id", &self.user_id).field("access_expires_at", &self.access_expires_at).finish_non_exhaustive()
    }
}

impl Session {
    /// From a login or refresh answer (`{"access_token", "access_expires_at", "refresh_token", ..., "me"}`).
    pub fn from_answer(v: &serde_json::Value, provider: &str) -> Option<Session> {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        let token_ok = |t: &str| !t.is_empty() && t.len() <= 512 && t.bytes().all(|b| (0x21..0x7f).contains(&b));
        let access = s("access_token").filter(|t| token_ok(t))?;
        let refresh = s("refresh_token").filter(|t| token_ok(t))?;
        Some(Session {
            access_token: access,
            access_expires_at: s("access_expires_at"),
            refresh_token: refresh,
            refresh_expires_at: s("refresh_expires_at"),
            provider: provider.to_string(),
            user_id: v.pointer("/me/id").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
            display_name: v.pointer("/me/display_name").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        })
    }

    /// The access token expires within `margin_s` seconds (by the server's clock as far as `offset_s` tells).
    pub fn access_expiring(&self, now_unix: i64, margin_s: i64) -> bool {
        self.access_expires_at.as_deref().and_then(parse_time).is_some_and(|t| t - margin_s <= now_unix)
    }
}

/// RFC 3339 → Unix seconds.
pub fn parse_time(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp())
}

pub fn path(server_dir: &Path) -> PathBuf {
    server_dir.join("session.dat")
}

pub fn save(server_dir: &Path, s: &Session) -> AppResult<()> {
    let json = serde_json::to_vec(s).map_err(|e| AppError::Internal(e.to_string()))?;
    let mut out = MAGIC.to_vec();
    out.extend(protect(&json)?);
    util::atomic_write(&path(server_dir), &out)
}

pub fn load(server_dir: &Path) -> Option<Session> {
    let bytes = std::fs::read(path(server_dir)).ok()?;
    let body = bytes.strip_prefix(MAGIC)?;
    serde_json::from_slice(&unprotect(body)?).ok()
}

pub fn clear(server_dir: &Path) {
    let _ = std::fs::remove_file(path(server_dir));
}

#[cfg(windows)]
fn protect(data: &[u8]) -> AppResult<Vec<u8>> {
    dpapi(data, true).ok_or_else(|| AppError::Internal("could not encrypt the session for this Windows user (DPAPI)".into()))
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    dpapi(data, false)
}

/// DPAPI, current user scope, no UI.
#[cfg(windows)]
fn dpapi(data: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData};
    let input = CRYPT_INTEGER_BLOB { cbData: u32::try_from(data.len()).ok()?, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: `input` points at `data`, alive for the call; DPAPI allocates `out` with LocalAlloc, freed below after copying.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        }
    };
    if ok == 0 || out.pbData.is_null() {
        return None;
    }
    // SAFETY: DPAPI returned `cbData` bytes at `pbData`.
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    // SAFETY: allocated by DPAPI with LocalAlloc.
    unsafe { LocalFree(out.pbData as _) };
    Some(v)
}

// Other systems (the launcher targets Windows; this keeps the crate building elsewhere): the file is only as private
// as the data folder.
#[cfg(not(windows))]
fn protect(data: &[u8]) -> AppResult<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(not(windows))]
fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    Some(data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sessions_are_stored_encrypted_and_never_printed() {
        let d = std::env::temp_dir().join(format!("s3l-tok-{}", util::random_hex(4)));
        let answer = json!({"access_token": "kat1.SECRETACCESS", "access_expires_at": "2026-11-11T21:15:03Z", "refresh_token": "krt1.SECRETREFRESH",
                            "refresh_expires_at": "2026-12-11T20:15:03Z", "created": true, "me": {"id": "u_3k9", "display_name": "Camille"}});
        let s = Session::from_answer(&answer, "guest").unwrap();
        assert_eq!((s.user_id.as_str(), s.display_name.as_str(), s.provider.as_str()), ("u_3k9", "Camille", "guest"));
        assert!(!format!("{s:?}").contains("SECRET"));
        save(&d, &s).unwrap();
        let raw = std::fs::read(path(&d)).unwrap();
        assert!(raw.starts_with(MAGIC));
        #[cfg(windows)]
        assert!(!String::from_utf8_lossy(&raw).contains("SECRET"), "the file is encrypted");
        assert_eq!(load(&d), Some(s.clone()));
        // a damaged file is no session
        std::fs::write(path(&d), b"KLMS1garbage").unwrap();
        assert_eq!(load(&d), None);
        clear(&d);
        assert_eq!(load(&d), None);
        std::fs::remove_dir_all(&d).unwrap();
        // answers without usable tokens are refused
        assert!(Session::from_answer(&json!({"access_token": "a b", "refresh_token": "x"}), "g").is_none());
        assert!(Session::from_answer(&json!({"access_token": "a"}), "g").is_none());
    }

    #[test]
    fn expiry() {
        let s = Session { access_token: "a".into(), access_expires_at: Some("2026-11-11T21:15:03Z".into()), refresh_token: "r".into(), refresh_expires_at: None, provider: String::new(), user_id: String::new(), display_name: String::new() };
        let t = parse_time("2026-11-11T21:15:03Z").unwrap();
        assert!(!s.access_expiring(t - 120, 60));
        assert!(s.access_expiring(t - 30, 60));
        assert_eq!(parse_time("2026-11-11T21:15:03.250Z"), Some(t));
        assert_eq!(parse_time("nope"), None);
    }
}
