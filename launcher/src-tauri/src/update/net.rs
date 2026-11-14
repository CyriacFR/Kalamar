//! The update check's network side: GitHub's public release API and its download hosts, nothing else.
//!
//! - **Hosts, a closed list.** API requests go to `api.github.com` only; downloads start at `github.com` and may be
//!   redirected only to GitHub's release-asset hosts ([`DOWNLOAD_HOSTS`]); https only, default port, no user name in
//!   the URL, at most 5 redirects, each one checked against the same list.
//! - **Minimal requests.** No token, no cookie, no identifier: `User-Agent: Kalamar-Launcher`, `Accept`, the API version
//!   header, `If-None-Match` for the API (GitHub's ETag, so an unchanged answer is a bodyless 304) and `Range` to resume a
//!   download.
//! - **Bounded.** Each answer has a size limit and a time limit; a download stops as soon as it passes the size the
//!   signed manifest gives.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const API_HOST: &str = "api.github.com";
pub const WEB_HOST: &str = "github.com";
/// Where `github.com/<repo>/releases/download/...` redirects (GitHub's documented release-asset hosts).
pub const DOWNLOAD_HOSTS: &[&str] = &["github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com", "github-releases.githubusercontent.com"];
pub const USER_AGENT: &str = "Kalamar-Launcher";
pub const MAX_API_ANSWER: usize = 1 << 20;
const MAX_REDIRECTS: usize = 5;
const API_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Api,
    Download,
}

/// Which hosts each phase may reach.
#[derive(Debug, Clone)]
pub struct HostPolicy {
    pub api: Vec<String>,
    pub download: Vec<String>,
    /// Plain http to 127.0.0.1 with any port: only the tests' local release server (never set outside tests).
    pub loopback_http: bool,
}

impl HostPolicy {
    pub fn github() -> HostPolicy {
        HostPolicy { api: vec![API_HOST.into()], download: DOWNLOAD_HOSTS.iter().map(|h| h.to_string()).collect(), loopback_http: false }
    }
}

/// Where the releases are and who may sign them.
#[derive(Clone)]
pub struct Source {
    /// `owner/repo`.
    pub repo: String,
    pub api_base: String,
    pub web_base: String,
    pub keys: Vec<minisign_verify::PublicKey>,
    pub policy: HostPolicy,
}

impl Source {
    pub fn github(repo: &str, keys: Vec<minisign_verify::PublicKey>) -> Source {
        Source { repo: repo.to_string(), api_base: format!("https://{API_HOST}"), web_base: format!("https://{WEB_HOST}"), keys, policy: HostPolicy::github() }
    }

    /// The tests' local release server: plain http on the loopback, the same paths as GitHub.
    #[cfg(test)]
    pub fn local(base: &str, repo: &str, keys: Vec<minisign_verify::PublicKey>) -> Source {
        Source {
            repo: repo.to_string(),
            api_base: format!("{base}/api"),
            web_base: format!("{base}/web"),
            keys,
            policy: HostPolicy { api: vec!["127.0.0.1".into()], download: vec!["127.0.0.1".into()], loopback_http: true },
        }
    }

    pub fn latest_url(&self) -> String {
        format!("{}/repos/{}/releases/latest", self.api_base, self.repo)
    }
    pub fn list_url(&self) -> String {
        format!("{}/repos/{}/releases?per_page=10", self.api_base, self.repo)
    }
    /// A file of a release, by its tag and name (both checked by the caller: no `/`, no `..`).
    pub fn asset_url(&self, tag: &str, name: &str) -> String {
        format!("{}/{}/releases/download/{tag}/{name}", self.web_base, self.repo)
    }
}

/// Whether a URL may be fetched (or redirected to) in this phase.
pub fn check_url(policy: &HostPolicy, phase: Phase, url: &reqwest::Url) -> Result<(), String> {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let list = match phase {
        Phase::Api => &policy.api,
        Phase::Download => &policy.download,
    };
    if !url.username().is_empty() || url.password().is_some() {
        return Err("a URL with a user name is refused".into());
    }
    let loopback = policy.loopback_http && host == "127.0.0.1" && url.scheme() == "http";
    if !loopback && (url.scheme() != "https" || url.port().is_some()) {
        return Err(format!("{}: only https on the default port", url.scheme()));
    }
    if !list.contains(&host) {
        return Err(format!("host {host} is not one of the expected GitHub hosts"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// No answer: DNS, connection, TLS, timeout, a cut connection.
    Transport(String),
    /// A URL or a redirect outside the allowed hosts.
    Refused(String),
    /// GitHub's rate limit; `reset` is when it lifts (Unix seconds), if it said.
    RateLimited(Option<u64>),
    Status(u16),
    TooLarge,
    Cancelled,
    Disk(String),
}

impl NetError {
    pub fn code(&self) -> &'static str {
        match self {
            NetError::Transport(_) | NetError::Status(_) => "network",
            NetError::Refused(_) => "untrusted",
            NetError::RateLimited(_) => "rate_limited",
            NetError::TooLarge => "integrity",
            NetError::Cancelled => "cancelled",
            NetError::Disk(_) => "disk",
        }
    }
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

fn transport(e: reqwest::Error) -> NetError {
    let text = crate::proto::http::describe(&e);
    if e.is_redirect() || text.contains("not one of the expected") || text.contains("only https") {
        NetError::Refused(text)
    } else {
        NetError::Transport(text)
    }
}

/// A client for one phase: the redirect rule checks every hop against the phase's hosts.
pub fn client(policy: &HostPolicy, phase: Phase) -> Result<reqwest::Client, NetError> {
    crate::proto::http::install_crypto();
    let pol = policy.clone();
    let redirects = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        match check_url(&pol, phase, attempt.url()) {
            Ok(()) => attempt.follow(),
            Err(e) => attempt.error(e),
        }
    });
    let mut b = reqwest::Client::builder().user_agent(USER_AGENT).redirect(redirects).connect_timeout(CONNECT_TIMEOUT).read_timeout(READ_TIMEOUT).referer(false);
    if phase == Phase::Api {
        b = b.timeout(API_TIMEOUT);
    }
    if policy.loopback_http {
        b = b.no_proxy();
    }
    b.build().map_err(|e| NetError::Transport(e.to_string()))
}

pub enum ApiReply {
    /// 304: the answer did not change since the ETag.
    NotModified,
    Fresh { body: serde_json::Value, etag: Option<String> },
}

fn rate_limit(resp: &reqwest::Response) -> Option<NetError> {
    let h = |k: &str| resp.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let status = resp.status().as_u16();
    let exhausted = h("x-ratelimit-remaining").as_deref() == Some("0");
    if status == 429 || (status == 403 && (exhausted || h("retry-after").is_some())) {
        let now = crate::util::now_secs();
        let reset = h("x-ratelimit-reset").and_then(|v| v.parse::<u64>().ok()).or_else(|| h("retry-after").and_then(|v| v.parse::<u64>().ok()).map(|s| now + s));
        // never wait more than a day because of a header
        return Some(NetError::RateLimited(reset.map(|r| r.min(now + 86_400))));
    }
    None
}

async fn read_bounded(mut resp: reqwest::Response, max: usize) -> Result<Vec<u8>, NetError> {
    if resp.content_length().is_some_and(|n| n > max as u64) {
        return Err(NetError::TooLarge);
    }
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(transport)? {
        out.extend_from_slice(&chunk);
        if out.len() > max {
            return Err(NetError::TooLarge);
        }
    }
    Ok(out)
}

/// One GET of the release API, with the ETag of the previous answer to the same URL.
pub async fn get_api(src: &Source, url: &str, etag: Option<&str>) -> Result<ApiReply, NetError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| NetError::Refused(e.to_string()))?;
    check_url(&src.policy, Phase::Api, &parsed).map_err(NetError::Refused)?;
    let c = client(&src.policy, Phase::Api)?;
    let mut rb = c.get(parsed).header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(e) = etag.filter(|e| e.len() <= 200 && e.is_ascii()) {
        rb = rb.header("If-None-Match", e);
    }
    let resp = rb.send().await.map_err(transport)?;
    if let Some(e) = rate_limit(&resp) {
        return Err(e);
    }
    let status = resp.status().as_u16();
    if status == 304 {
        return Ok(ApiReply::NotModified);
    }
    if status != 200 {
        return Err(NetError::Status(status));
    }
    let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).filter(|v| v.len() <= 200).map(str::to_string);
    let bytes = read_bounded(resp, MAX_API_ANSWER).await?;
    let body = serde_json::from_slice(&bytes).map_err(|e| NetError::Transport(format!("not JSON: {e}")))?;
    Ok(ApiReply::Fresh { body, etag })
}

/// A small release file (the manifest, its signature), at most `max` bytes.
pub async fn get_small(src: &Source, url: &str, max: usize) -> Result<Vec<u8>, NetError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| NetError::Refused(e.to_string()))?;
    check_url(&src.policy, Phase::Download, &parsed).map_err(NetError::Refused)?;
    let c = client(&src.policy, Phase::Download)?;
    let resp = c.get(parsed).header("Accept", "application/octet-stream").timeout(API_TIMEOUT).send().await.map_err(transport)?;
    if let Some(e) = rate_limit(&resp) {
        return Err(e);
    }
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(NetError::Status(status));
    }
    read_bounded(resp, max).await
}

/// Downloads `url` into `part`, resuming from what `part` already holds (`Range`), never past `size` bytes. The caller
/// checks the SHA-256 afterwards; this only guarantees the length. `progress(done, total)` is called as bytes arrive.
pub async fn download(src: &Source, url: &str, part: &Path, size: u64, progress: &(dyn Fn(u64, u64) + Send + Sync), cancel: &AtomicBool) -> Result<(), NetError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| NetError::Refused(e.to_string()))?;
    check_url(&src.policy, Phase::Download, &parsed).map_err(NetError::Refused)?;
    let disk = |e: std::io::Error| NetError::Disk(e.to_string());
    let mut have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    if have > size {
        std::fs::remove_file(part).map_err(disk)?;
        have = 0;
    }
    if have == size {
        progress(size, size);
        return Ok(());
    }
    let c = client(&src.policy, Phase::Download)?;
    let mut rb = c.get(parsed).header("Accept", "application/octet-stream");
    if have > 0 {
        rb = rb.header("Range", format!("bytes={have}-"));
    }
    let mut resp = rb.send().await.map_err(transport)?;
    if let Some(e) = rate_limit(&resp) {
        return Err(e);
    }
    let status = resp.status().as_u16();
    let resumed = match status {
        206 if have > 0 => {
            // `bytes <start>-<end>/<total>` must continue exactly where the part file ends
            let cr = resp.headers().get("content-range").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
            let ok = cr.strip_prefix("bytes ").and_then(|r| r.split_once('/')).is_some_and(|(range, total)| {
                total.parse::<u64>().ok() == Some(size) && range.split_once('-').and_then(|(a, _)| a.parse::<u64>().ok()) == Some(have)
            });
            if !ok {
                let _ = std::fs::remove_file(part);
                return Err(NetError::Transport(format!("unexpected range answer '{cr}'")));
            }
            true
        }
        200 => false,
        s => return Err(NetError::Status(s)),
    };
    let start = if resumed { have } else { 0 };
    if resp.content_length().is_some_and(|n| start + n > size) {
        return Err(NetError::TooLarge);
    }
    if let Some(dir) = part.parent() {
        std::fs::create_dir_all(dir).map_err(disk)?;
    }
    let mut f = if resumed { std::fs::OpenOptions::new().append(true).open(part) } else { std::fs::File::create(part) }.map_err(disk)?;
    let mut done = start;
    progress(done, size);
    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = f.flush();
            return Err(NetError::Cancelled);
        }
        let Some(chunk) = resp.chunk().await.map_err(transport)? else { break };
        done += chunk.len() as u64;
        if done > size {
            drop(f);
            let _ = std::fs::remove_file(part);
            return Err(NetError::TooLarge);
        }
        f.write_all(&chunk).map_err(disk)?;
        progress(done, size);
    }
    f.sync_all().map_err(disk)?;
    if done != size {
        return Err(NetError::Transport(format!("the download stopped at {done} of {size} bytes")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> reqwest::Url {
        reqwest::Url::parse(s).unwrap()
    }

    #[test]
    fn only_the_expected_github_hosts_over_https() {
        let p = HostPolicy::github();
        assert!(check_url(&p, Phase::Api, &u("https://api.github.com/repos/o/r/releases/latest")).is_ok());
        assert!(check_url(&p, Phase::Download, &u("https://github.com/o/r/releases/download/v1/Kalamar.exe")).is_ok());
        assert!(check_url(&p, Phase::Download, &u("https://release-assets.githubusercontent.com/github-production-release-asset/1")).is_ok());
        assert!(check_url(&p, Phase::Download, &u("https://objects.githubusercontent.com/x")).is_ok());
        assert!(check_url(&p, Phase::Download, &u("https://GITHUB.com/x")).is_ok(), "host names are not case sensitive");
        for (phase, bad) in [
            (Phase::Api, "http://api.github.com/x"),
            (Phase::Api, "https://github.com/x"),
            (Phase::Api, "https://api.github.com:8443/x"),
            (Phase::Api, "https://user:pw@api.github.com/x"),
            (Phase::Download, "https://api.github.com/x"),
            (Phase::Download, "https://evil.example/Kalamar.exe"),
            (Phase::Download, "https://github.com.evil.example/x"),
            (Phase::Download, "https://raw.githubusercontent.com/o/r/main/Kalamar.exe"),
            (Phase::Download, "https://gist.githubusercontent.com/x"),
            (Phase::Download, "http://objects.githubusercontent.com/x"),
            (Phase::Download, "https://140.82.112.3/x"),
            (Phase::Download, "http://127.0.0.1:8080/x"),
            (Phase::Download, "ftp://github.com/x"),
            (Phase::Download, "file:///C:/Windows/notepad.exe"),
        ] {
            assert!(check_url(&p, phase, &u(bad)).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn urls_are_built_from_the_configured_repo() {
        let s = Source::github("owner/kalamar", vec![]);
        assert_eq!(s.latest_url(), "https://api.github.com/repos/owner/kalamar/releases/latest");
        assert_eq!(s.list_url(), "https://api.github.com/repos/owner/kalamar/releases?per_page=10");
        assert_eq!(s.asset_url("v0.2.0", "Kalamar.exe"), "https://github.com/owner/kalamar/releases/download/v0.2.0/Kalamar.exe");
    }
}
