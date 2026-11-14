//! The tools a first install needs, kept inside the install folder: nothing is installed on the system, nothing asks
//! for an administrator, no winget, no PATH or registry change. Every file comes from its project's official download
//! (python.org, PyPI, the GitHub releases of llvm-mingw, Ninja and SDL, the MSYS2 package repository) over https, and is
//! checked against the SHA-256 pinned here before anything is unpacked. The toolchain pins are the engine's own
//! (`tools/make_release.py`, `PINS`) except SDL and VMA (below); the wheels are the versions of
//! `tools/requirements-installer.txt`.
//!
//! ```text
//! <I>\tools\python\        python.org embeddable Python 3.14.9 + the five wheels in Lib\site-packages
//! <I>\tools\llvm\          llvm-mingw 20261020, the x86_64 files clang and lld need (make_release.py's selection)
//! <I>\tools\deps\          include\ lib\ bin\ of SDL3, Opus, libvpx, Vulkan, VMA and zstd (build_game.py --deps)
//! <I>\tools\bin\           ninja.exe; hactool.exe (shipped with the launcher, `prebuilt`)
//! <I>\tools\downloads\     the archives while they download (a cut download resumes); each is deleted once unpacked
//! ```
//!
//! What is downloaded is run as separate programs or linked into the game the player builds on their own PC; none of it
//! is redistributed by the launcher. The embeddable Python runs without its `._pth` file, which makes it behave like an
//! installed Python whose home is that folder (the script's folder and `Lib\site-packages` on `sys.path`).

use super::sys;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// One pinned download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Pin {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

macro_rules! msys2 {
    ($file:literal) => {
        concat!("https://repo.msys2.org/mingw/ucrt64/", $file)
    };
}

pub const PYTHON_EMBED: Pin = Pin {
    name: "python-3.14.9-embed-amd64.zip",
    url: "https://www.python.org/ftp/python/3.14.9/python-3.14.9-embed-amd64.zip",
    sha256: "3c01d7cfc666f746a6f064f60fb2156ab0ec464d3d738ab796371a46a9bb24a2",
    size: 12_609_248,
};

/// What the install steps import: numpy (recomp/gen.py), lz4 (recomp/nso.py), cryptography with cffi and pycparser (the
/// extraction scripts and the save keys).
pub const WHEELS: [Pin; 5] = [
    Pin {
        name: "numpy-2.5.4-cp314-cp314-win_amd64.whl",
        url: "https://files.pythonhosted.org/packages/52/9a/e1e512ebc948d5b9dd33b08736760f0ebbed2848fd4eda1f553088a6dcee/numpy-2.5.4-cp314-cp314-win_amd64.whl",
        sha256: "2ca144f15135b6212a5c47b1e2aeca6e412f102f95a2d5d88d8aec77eb255de3",
        size: 12_703_552,
    },
    Pin {
        name: "lz4-4.4.5-cp314-cp314-win_amd64.whl",
        url: "https://files.pythonhosted.org/packages/18/2c/8332080fd293f8337779a440b3a143f85e374311705d243439a3349b81ad/lz4-4.4.5-cp314-cp314-win_amd64.whl",
        sha256: "92159782a4502858a21e0079d77cdcaade23e8a5d252ddf46b0652604300d7be",
        size: 101_497,
    },
    Pin {
        name: "cryptography-50.0.2-cp311-abi3-win_amd64.whl",
        url: "https://files.pythonhosted.org/packages/9a/4f/adfc442765721292fff86d314ce385d3249d22db42295c0dd057727b60f3/cryptography-50.0.2-cp311-abi3-win_amd64.whl",
        sha256: "7afa5a6602a9f29af1f3a2965f831bae7c9d5d597b7cbb716d41ab3b7d89879c",
        size: 3_824_692,
    },
    Pin {
        name: "cffi-2.1.1-cp314-cp314-win_amd64.whl",
        url: "https://files.pythonhosted.org/packages/a7/06/1c3e01e3ba14c39f6d10bfbac52753b7e22259e38088e5cfe1d704918690/cffi-2.1.1-cp314-cp314-win_amd64.whl",
        sha256: "3222ba5d678f80a030e6afbcc33dc1ae5cb45facabb61cee2c7016b8432fde48",
        size: 187_949,
    },
    Pin {
        name: "pycparser-3.11-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/90/11/0e6f11117525ff0eec40ebac3d313376f102df93ca44ad9e893ee85e4f89/pycparser-3.11-py3-none-any.whl",
        sha256: "51d5a8ba2be0bbe440b99d2112604c95bbbc3c2748a64260186c541e1729cd80",
        size: 51_178,
    },
];

pub const LLVM_VERSION: &str = "20261020";
pub const LLVM_MINGW: Pin = Pin {
    name: "llvm-mingw-20261020-ucrt-x86_64.zip",
    url: "https://github.com/mstorsjo/llvm-mingw/releases/download/20261020/llvm-mingw-20261020-ucrt-x86_64.zip",
    sha256: "fd5462b7f5d93b2ff981fcd3f9c7e427e47f3203c64ea91c27bcc444eebd74cd",
    size: 190_612_575,
};
pub const NINJA: Pin = Pin {
    name: "ninja-win.zip",
    url: "https://github.com/ninja-build/ninja/releases/download/v1.13.2/ninja-win.zip",
    sha256: "07fc8261b42b20e71d1720b39068c2e14ffcee6396b76fb7a795fb460b78dc65",
    size: 291_570,
};
/// SDL and VMA are the versions the MSYS2 builds of the game used until now (3.4.20, 3.5.0), not tools/make_release.py's,
/// so the game a first install builds is the one the existing installs run; their digests are the ones GitHub publishes
/// for the release assets and the MSYS2 repository database gives for the package.
pub const SDL3_VERSION: &str = "3.4.20";
pub const SDL3_DLL: Pin = Pin {
    name: "SDL3-3.4.20-win32-x64.zip",
    url: "https://github.com/libsdl-org/SDL/releases/download/release-3.4.20/SDL3-3.4.20-win32-x64.zip",
    sha256: "368bed8b24e44ad9ea372990814f2343fcfb9e468abb7f8f4a3bc146c7304752",
    size: 1_166_032,
};
/// SDL3.dll inside the SDL zip.
pub const SDL3_DLL_INNER: &str = "66844f10251cc1c10619ed5d8e4e3224f564900ee6d69ba87d63de782addae40";
pub const SDL3_DEVEL: Pin = Pin {
    name: "SDL3-devel-3.4.20-mingw.tar.gz",
    url: "https://github.com/libsdl-org/SDL/releases/download/release-3.4.20/SDL3-devel-3.4.20-mingw.tar.gz",
    sha256: "d2d3bf3fb97a90b22e908a306d7362631ef1bab229d988fe6669595dc29cd9ed",
    size: 5_288_546,
};
pub const OPUS: Pin = Pin {
    name: "mingw-w64-ucrt-x86_64-opus-1.6.1-2-any.pkg.tar.zst",
    url: msys2!("mingw-w64-ucrt-x86_64-opus-1.6.1-2-any.pkg.tar.zst"),
    sha256: "bb114bcf26225f50f874497ebfd6b8224fff2a20bda3f8ee45cd07ce5f02657b",
    size: 344_930,
};
pub const LIBVPX: Pin = Pin {
    name: "mingw-w64-ucrt-x86_64-libvpx-1.17.0-1-any.pkg.tar.zst",
    url: msys2!("mingw-w64-ucrt-x86_64-libvpx-1.17.0-1-any.pkg.tar.zst"),
    sha256: "8ff6919dbb88ea26ce28d22fd81d6f0d6e200cf56a4dd81ffc2d33124fe98024",
    size: 1_625_301,
};
pub const VULKAN_HEADERS: Pin = Pin {
    name: "mingw-w64-ucrt-x86_64-vulkan-headers-1~1.4.363.0-1-any.pkg.tar.zst",
    url: msys2!("mingw-w64-ucrt-x86_64-vulkan-headers-1~1.4.363.0-1-any.pkg.tar.zst"),
    sha256: "be602b2657cace531316e78b1eda4087e8f7f84ebb877f08fba198a2db06406a",
    size: 1_799_185,
};
pub const VULKAN_LOADER: Pin = Pin {
    name: "mingw-w64-ucrt-x86_64-vulkan-loader-1~1.4.363.0-1-any.pkg.tar.zst",
    url: msys2!("mingw-w64-ucrt-x86_64-vulkan-loader-1~1.4.363.0-1-any.pkg.tar.zst"),
    sha256: "065076fe13d39ab97eea4740596426d5ae741b5044e48e1c9495d04835c92dbe",
    size: 222_857,
};
pub const VMA: Pin = Pin {
    name: "mingw-w64-ucrt-x86_64-vulkan-memory-allocator-3.5.0-1-any.pkg.tar.zst",
    url: msys2!("mingw-w64-ucrt-x86_64-vulkan-memory-allocator-3.5.0-1-any.pkg.tar.zst"),
    sha256: "bc6dd2250bf198a9ab5a0a77767f0a6401cb9ef8dd8a2d640fb23f70d052c026",
    size: 122_306,
};
pub const ZSTD: Pin = Pin {
    name: "mingw-w64-ucrt-x86_64-zstd-1.5.7-2-any.pkg.tar.zst",
    url: msys2!("mingw-w64-ucrt-x86_64-zstd-1.5.7-2-any.pkg.tar.zst"),
    sha256: "dbdb8427280046a2b41697780aa4c52983b708082b0da4755951dc3bea96ca89",
    size: 668_277,
};

/// The compiler, Ninja and the libraries, in download order (the large one last, so the small ones are unpacked while
/// it downloads... in practice one after the other; the order only fixes what the progress shows).
pub const TOOLCHAIN: [Pin; 10] = [NINJA, SDL3_DLL, SDL3_DEVEL, OPUS, LIBVPX, VULKAN_HEADERS, VULKAN_LOADER, VMA, ZSTD, LLVM_MINGW];

/// Every download of a first install: Python, its wheels, then the toolchain.
pub fn all_pins() -> Vec<Pin> {
    let mut v = vec![PYTHON_EMBED];
    v.extend(WHEELS);
    v.extend(TOOLCHAIN);
    v
}

pub fn python_pins() -> Vec<Pin> {
    let mut v = vec![PYTHON_EMBED];
    v.extend(WHEELS);
    v
}

pub fn bytes(pins: &[Pin]) -> u64 {
    pins.iter().map(|p| p.size).sum()
}

fn pins_id(prefix: &str, pins: &[Pin]) -> String {
    let text: String = pins.iter().map(|p| format!("{}\0{}\n", p.name, p.sha256)).collect();
    format!("{prefix}-{}", &super::bundle::sha256_hex(text.as_bytes())[..16])
}

/// Identity of the portable toolchain: recorded with every compile made with it, so a launcher that pins another one
/// rebuilds the game (Ninja would anyway, the compiler's path being in every command; this tells the update screen).
pub fn toolchain_id() -> String {
    pins_id(&format!("llvm-mingw-{LLVM_VERSION}"), &TOOLCHAIN)
}

pub fn python_id() -> String {
    pins_id("python-3.14.9", &python_pins())
}

/// Disk the toolchain takes once unpacked (llvm-mingw's selection 320 MB, the libraries 39 MB, Ninja), measured.
pub const TOOLCHAIN_DISK: u64 = 360_000_000;
/// Disk the portable Python takes with its wheels (80 MB measured).
pub const PYTHON_DISK: u64 = 85_000_000;

// ---- paths -------------------------------------------------------------------------------------------------------

pub fn tools(root: &Path) -> PathBuf {
    root.join("tools")
}
pub fn python_dir(root: &Path) -> PathBuf {
    tools(root).join("python")
}
pub fn python_exe(root: &Path) -> PathBuf {
    python_dir(root).join("python.exe")
}
pub fn llvm_dir(root: &Path) -> PathBuf {
    tools(root).join("llvm")
}
pub fn deps_dir(root: &Path) -> PathBuf {
    tools(root).join("deps")
}
pub fn bin_dir(root: &Path) -> PathBuf {
    tools(root).join("bin")
}
pub fn ninja_exe(root: &Path) -> PathBuf {
    bin_dir(root).join("ninja.exe")
}
pub fn downloads(root: &Path) -> PathBuf {
    tools(root).join("downloads")
}
fn toolchain_marker(root: &Path) -> PathBuf {
    tools(root).join("toolchain.json")
}
fn python_marker(root: &Path) -> PathBuf {
    python_dir(root).join("kalamar-python.json")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
struct Marker {
    id: String,
    pins: Vec<(String, String)>,
    at: u64,
}

fn write_marker(path: &Path, id: String, pins: &[Pin]) -> std::io::Result<()> {
    let m = Marker { id, pins: pins.iter().map(|p| (p.name.to_string(), p.sha256.to_string())).collect(), at: crate::util::now_secs() };
    std::fs::write(path, serde_json::to_vec_pretty(&m).unwrap_or_default())
}

fn marker_id(path: &Path) -> Option<String> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Marker>(&b).ok()).map(|m| m.id)
}

/// The portable toolchain of `root` is complete and is the one this launcher pins.
pub fn toolchain_ready(root: &Path) -> bool {
    marker_id(&toolchain_marker(root)).is_some_and(|id| id == toolchain_id())
        && llvm_dir(root).join("bin").join("clang.exe").is_file()
        && ninja_exe(root).is_file()
        && deps_dir(root).join("lib").join("libSDL3.dll.a").is_file()
}

/// The portable Python of `root` is unpacked with its wheels (its imports are checked by the Python step).
pub fn python_ready(root: &Path) -> bool {
    marker_id(&python_marker(root)).is_some_and(|id| id == python_id()) && python_exe(root).is_file()
}

// ---- errors ------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Cancelled,
    /// No answer, a refused redirect, an HTTP error, a cut connection after the retries.
    Network { url: String, message: String },
    /// The file downloaded is not the pinned one (deleted).
    Mismatch { name: String, got: String },
    Io { what: String, message: String },
    /// An archive that does not hold what it should.
    Archive { name: String, message: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Cancelled => write!(f, "cancelled"),
            Error::Network { url, message } => write!(f, "{url}: {message}"),
            Error::Mismatch { name, got } => write!(f, "{name}: SHA-256 {got} is not the pinned one"),
            Error::Io { what, message } => write!(f, "{what}: {message}"),
            Error::Archive { name, message } => write!(f, "{name}: {message}"),
        }
    }
}

fn io(what: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |e| Error::Io { what: what.display().to_string(), message: e.to_string() }
}

// ---- download ----------------------------------------------------------------------------------------------------

pub fn file_sha256(path: &Path) -> std::io::Result<String> {
    super::file_sha256(path)
}

/// Downloads `pins` into `dir` (skipping the ones already there and intact). `progress(done, total)` is called as bytes
/// arrive. A cut download leaves `<name>.part`, which the next call resumes with an HTTP range request.
pub fn download_all(pins: &[Pin], dir: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64)) -> Result<Vec<PathBuf>, Error> {
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    let total = bytes(pins);
    let mut done_before = 0u64;
    let mut out = Vec::new();
    let mut fetcher: Option<Fetcher> = None;
    for pin in pins {
        let file = dir.join(pin.name);
        if file.is_file() && file_sha256(&file).is_ok_and(|h| h == pin.sha256) {
            done_before += pin.size;
            progress(done_before, total);
            out.push(file);
            continue;
        }
        if fetcher.is_none() {
            fetcher = Some(Fetcher::new()?);
        }
        let f = fetcher.as_ref().expect("fetcher");
        let base = done_before;
        f.fetch(pin, &file, cancel, &mut |n| progress(base + n, total))?;
        done_before += pin.size;
        out.push(file);
    }
    Ok(out)
}

struct Fetcher {
    rt: tokio::runtime::Runtime,
    client: reqwest::Client,
}

impl Fetcher {
    fn new() -> Result<Fetcher, Error> {
        crate::proto::http::install_crypto();
        let net = |e: String| Error::Network { url: String::new(), message: e };
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| net(e.to_string()))?;
        // Release assets on GitHub answer with a redirect to their storage host; any redirect must stay on https.
        let policy = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > 5 {
                attempt.error("too many redirects")
            } else if attempt.url().scheme() != "https" {
                attempt.error("redirect to a non-https URL refused")
            } else {
                attempt.follow()
            }
        });
        let client = reqwest::Client::builder()
            .user_agent(crate::proto::http::USER_AGENT)
            .redirect(policy)
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| net(e.to_string()))?;
        Ok(Fetcher { rt, client })
    }

    /// One file, with up to four attempts (each resumes what the previous one wrote), then the size and SHA-256 check.
    fn fetch(&self, pin: &Pin, file: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<(), Error> {
        if !pin.url.starts_with("https://") {
            return Err(Error::Network { url: pin.url.into(), message: "not an https URL".into() });
        }
        let part = file.with_file_name(format!("{}.part", pin.name));
        let mut last = None;
        for attempt in 0..4 {
            if cancel.load(Ordering::SeqCst) {
                return Err(Error::Cancelled);
            }
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs(2 * attempt as u64));
            }
            match self.rt.block_on(self.fetch_once(pin, &part, cancel, progress)) {
                Ok(()) => {
                    last = None;
                    break;
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e @ Error::Io { .. }) => return Err(e),
                Err(e) => last = Some(e),
            }
        }
        if let Some(e) = last {
            return Err(e);
        }
        let got = file_sha256(&part).map_err(io(&part))?;
        if got != pin.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err(Error::Mismatch { name: pin.name.into(), got });
        }
        std::fs::rename(&part, file).map_err(io(file))
    }

    async fn fetch_once(&self, pin: &Pin, part: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<(), Error> {
        let net = |m: String| Error::Network { url: pin.url.into(), message: m };
        let mut have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        if have > pin.size {
            let _ = std::fs::remove_file(part);
            have = 0;
        }
        if have == pin.size {
            return Ok(());
        }
        let mut req = self.client.get(pin.url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let mut resp = req.send().await.map_err(|e| net(e.to_string()))?;
        let status = resp.status();
        if have > 0 && status.as_u16() != 206 {
            // The server sent the whole file: start over.
            have = 0;
        }
        if !status.is_success() {
            return Err(net(format!("HTTP {status}")));
        }
        let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(have == 0).append(have > 0).open(part).map_err(io(part))?;
        let mut got = have;
        progress(got);
        while let Some(chunk) = resp.chunk().await.map_err(|e| net(e.to_string()))? {
            if cancel.load(Ordering::SeqCst) {
                return Err(Error::Cancelled);
            }
            f.write_all(&chunk).map_err(io(part))?;
            got += chunk.len() as u64;
            if got > pin.size {
                drop(f);
                let _ = std::fs::remove_file(part);
                return Err(Error::Mismatch { name: pin.name.into(), got: format!("more than {} bytes", pin.size) });
            }
            progress(got);
        }
        f.flush().map_err(io(part))?;
        if got != pin.size {
            return Err(net(format!("connection closed after {got} of {} bytes", pin.size)));
        }
        Ok(())
    }
}

// ---- archives ----------------------------------------------------------------------------------------------------

/// A relative archive path made safe (no root, no `..`), or `None`.
pub fn safe_rel(path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in Path::new(&path.replace('\\', "/")).components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// A minimal ZIP reader (stored and deflated members, ZIP64), enough for the python.org, PyPI and GitHub archives.
pub mod zip {
    use std::io::{Read, Seek, SeekFrom, Write};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Entry {
        pub name: String,
        pub method: u16,
        pub flags: u16,
        pub crc: u32,
        pub compressed: u64,
        pub size: u64,
        pub local_offset: u64,
    }

    impl Entry {
        pub fn is_dir(&self) -> bool {
            self.name.ends_with('/')
        }
    }

    fn u16le(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([b[at], b[at + 1]])
    }
    fn u32le(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap_or([0; 4]))
    }
    fn u64le(b: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]))
    }
    fn bad(m: &str) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_string())
    }

    /// The members listed in the central directory.
    pub fn entries<R: Read + Seek>(f: &mut R) -> std::io::Result<Vec<Entry>> {
        let len = f.seek(SeekFrom::End(0))?;
        let tail = len.min(65_557);
        f.seek(SeekFrom::Start(len - tail))?;
        let mut buf = vec![0u8; tail as usize];
        f.read_exact(&mut buf)?;
        let eocd = (0..buf.len().saturating_sub(21)).rev().find(|&i| u32le(&buf, i) == 0x0605_4b50).ok_or_else(|| bad("no end of central directory"))?;
        let mut count = u16le(&buf, eocd + 10) as u64;
        let mut cd_size = u32le(&buf, eocd + 12) as u64;
        let mut cd_off = u32le(&buf, eocd + 16) as u64;
        if count == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_off == 0xFFFF_FFFF {
            // ZIP64: the locator sits just before the end record.
            let loc = eocd.checked_sub(20).ok_or_else(|| bad("no ZIP64 locator"))?;
            if u32le(&buf, loc) != 0x0706_4b50 {
                return Err(bad("no ZIP64 locator"));
            }
            let rec_off = u64le(&buf, loc + 8);
            let mut rec = [0u8; 56];
            f.seek(SeekFrom::Start(rec_off))?;
            f.read_exact(&mut rec)?;
            if u32le(&rec, 0) != 0x0606_4b50 {
                return Err(bad("bad ZIP64 end record"));
            }
            count = u64le(&rec, 32);
            cd_size = u64le(&rec, 40);
            cd_off = u64le(&rec, 48);
        }
        if cd_size > 512 << 20 {
            return Err(bad("central directory too large"));
        }
        let mut cd = vec![0u8; cd_size as usize];
        f.seek(SeekFrom::Start(cd_off))?;
        f.read_exact(&mut cd)?;
        let mut out = Vec::with_capacity(count as usize);
        let mut at = 0usize;
        while at + 46 <= cd.len() && u32le(&cd, at) == 0x0201_4b50 {
            let flags = u16le(&cd, at + 8);
            let method = u16le(&cd, at + 10);
            let crc = u32le(&cd, at + 16);
            let mut compressed = u32le(&cd, at + 20) as u64;
            let mut size = u32le(&cd, at + 24) as u64;
            let (nl, xl, cl) = (u16le(&cd, at + 28) as usize, u16le(&cd, at + 30) as usize, u16le(&cd, at + 32) as usize);
            let mut local_offset = u32le(&cd, at + 42) as u64;
            let name_end = at + 46 + nl;
            if name_end + xl + cl > cd.len() {
                return Err(bad("truncated central directory"));
            }
            let name = String::from_utf8_lossy(&cd[at + 46..name_end]).replace('\\', "/");
            let extra = &cd[name_end..name_end + xl];
            let mut x = 0;
            while x + 4 <= extra.len() {
                let (id, n) = (u16le(extra, x), u16le(extra, x + 2) as usize);
                let body = &extra[(x + 4).min(extra.len())..(x + 4 + n).min(extra.len())];
                if id == 1 {
                    let mut p = 0;
                    for field in [&mut size, &mut compressed, &mut local_offset] {
                        if *field == 0xFFFF_FFFF && p + 8 <= body.len() {
                            *field = u64le(body, p);
                            p += 8;
                        }
                    }
                }
                x += 4 + n;
            }
            out.push(Entry { name, method, flags, crc, compressed, size, local_offset });
            at = name_end + xl + cl;
        }
        if out.len() as u64 != count {
            return Err(bad("central directory does not list every member"));
        }
        Ok(out)
    }

    /// Writes one member's content to `out`, checking its size and CRC-32.
    pub fn read_entry<R: Read + Seek>(f: &mut R, e: &Entry, out: &mut dyn Write) -> std::io::Result<()> {
        if e.flags & 1 != 0 {
            return Err(bad("encrypted member"));
        }
        let mut head = [0u8; 30];
        f.seek(SeekFrom::Start(e.local_offset))?;
        f.read_exact(&mut head)?;
        if u32le(&head, 0) != 0x0403_4b50 {
            return Err(bad("bad local header"));
        }
        let skip = u16le(&head, 26) as i64 + u16le(&head, 28) as i64;
        f.seek(SeekFrom::Current(skip))?;
        let raw = f.take(e.compressed);
        let mut reader: Box<dyn Read + '_> = match e.method {
            0 => Box::new(raw),
            8 => Box::new(flate2::read::DeflateDecoder::new(raw)),
            m => return Err(bad(&format!("compression method {m} not supported"))),
        };
        let mut crc = flate2::Crc::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut n_total = 0u64;
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            crc.update(&buf[..n]);
            out.write_all(&buf[..n])?;
            n_total += n as u64;
        }
        if n_total != e.size || crc.sum() != e.crc {
            return Err(bad(&format!("{}: size or CRC-32 does not match", e.name)));
        }
        Ok(())
    }
}

/// Unpacks the members of a ZIP that `map` keeps (archive path -> path below `dest`). Returns the files written.
pub fn unzip(archive: &Path, dest: &Path, map: &dyn Fn(&str) -> Option<PathBuf>, cancel: &AtomicBool) -> Result<usize, Error> {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let arch = |m: String| Error::Archive { name: name.clone(), message: m };
    let mut f = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(archive).map_err(io(archive))?);
    let entries = zip::entries(&mut f).map_err(|e| arch(e.to_string()))?;
    let mut n = 0;
    for e in entries.iter().filter(|e| !e.is_dir()) {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::Cancelled);
        }
        let Some(rel) = map(&e.name) else { continue };
        let Some(rel) = safe_rel(&rel.to_string_lossy()) else { return Err(arch(format!("unsafe path {}", e.name))) };
        let target = dest.join(rel);
        if let Some(d) = target.parent() {
            std::fs::create_dir_all(d).map_err(io(d))?;
        }
        let mut out = std::io::BufWriter::new(std::fs::File::create(&target).map_err(io(&target))?);
        zip::read_entry(&mut f, e, &mut out).map_err(|x| arch(x.to_string()))?;
        out.flush().map_err(io(&target))?;
        n += 1;
    }
    Ok(n)
}

/// Unpacks the regular files of a tar stream that `map` keeps.
pub fn untar(reader: impl Read, name: &str, dest: &Path, map: &dyn Fn(&str) -> Option<PathBuf>) -> Result<usize, Error> {
    let arch = |m: String| Error::Archive { name: name.into(), message: m };
    let mut ar = tar::Archive::new(reader);
    let mut n = 0;
    for e in ar.entries().map_err(|e| arch(e.to_string()))? {
        let mut e = e.map_err(|e| arch(e.to_string()))?;
        if !e.header().entry_type().is_file() {
            continue;
        }
        let path = e.path().map_err(|e| arch(e.to_string()))?.to_string_lossy().replace('\\', "/");
        let Some(rel) = map(&path) else { continue };
        let Some(rel) = safe_rel(&rel.to_string_lossy()) else { return Err(arch(format!("unsafe path {path}"))) };
        let target = dest.join(rel);
        if let Some(d) = target.parent() {
            std::fs::create_dir_all(d).map_err(io(d))?;
        }
        let mut out = std::fs::File::create(&target).map_err(io(&target))?;
        std::io::copy(&mut e, &mut out).map_err(io(&target))?;
        n += 1;
    }
    Ok(n)
}

fn untar_zst(archive: &Path, dest: &Path, map: &dyn Fn(&str) -> Option<PathBuf>) -> Result<usize, Error> {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let f = std::io::BufReader::new(std::fs::File::open(archive).map_err(io(archive))?);
    let dec = ruzstd::decoding::StreamingDecoder::new(f).map_err(|e| Error::Archive { name: name.clone(), message: e.to_string() })?;
    untar(dec, &name, dest, map)
}

fn untar_gz(archive: &Path, dest: &Path, map: &dyn Fn(&str) -> Option<PathBuf>) -> Result<usize, Error> {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let f = std::io::BufReader::new(std::fs::File::open(archive).map_err(io(archive))?);
    untar(flate2::read::GzDecoder::new(f), &name, dest, map)
}

// ---- what is kept of each archive -------------------------------------------------------------------------------

/// llvm-mingw's `bin\` files clang and lld need to compile and link the game (make_release.py `LLVM_BIN`).
pub const LLVM_BIN: [&str; 27] = [
    "clang.exe", "clang++.exe", "clang-23.exe", "ld.lld.exe", "llvm-ar.exe", "llvm-ranlib.exe", "llvm-nm.exe", "llvm-objdump.exe",
    "llvm-objcopy.exe", "llvm-strip.exe", "llvm-readobj.exe", "llvm-dlltool.exe", "llvm-rc.exe", "llvm-cvtres.exe", "llvm-windres.exe",
    "llvm-wrapper.exe", "mingw32-common.cfg", "x86_64-w64-windows-gnu.cfg", "x86_64-pc-windows-gnu.cfg", "x86_64-w64-mingw32-clang.exe",
    "x86_64-w64-mingw32-clang++.exe", "libLLVM-23.dll", "libclang-cpp.dll", "libc++.dll", "libunwind.dll", "libwinpthread-1.dll", "libffi-8.dll",
];

/// make_release.py `build_llvm`'s selection: x86_64 only, the compiler's own headers and builtins, no sanitizers, no
/// other targets, no Python, no driver kit headers. `rel` is relative to the archive's top folder.
pub fn llvm_keep(rel: &str) -> bool {
    let top = rel.split('/').next().unwrap_or("");
    if ["aarch64-w64-mingw32", "armv7-w64-mingw32", "i686-w64-mingw32", "busybox", "python", "share"].contains(&top) {
        return false;
    }
    if top == "bin" {
        return LLVM_BIN.iter().any(|b| rel == format!("bin/{b}"));
    }
    if top == "lib" {
        return (rel.starts_with("lib/clang/") && rel.contains("/include/")) || rel.ends_with("/lib/windows/libclang_rt.builtins-x86_64.a");
    }
    let lower = rel.to_ascii_lowercase();
    !(lower.ends_with(".idl") || lower.ends_with(".tlb") || rel.starts_with("include/ddk/"))
}

/// The members of a wheel that go to `site-packages` (a `.data/purelib` or `.data/platlib` member moves up; scripts and
/// headers are left out).
pub fn wheel_member(rel: &str) -> Option<PathBuf> {
    let first = rel.split('/').next().unwrap_or("");
    if first.ends_with(".data") {
        let rest = &rel[first.len() + 1..];
        let (kind, inner) = rest.split_once('/')?;
        return matches!(kind, "purelib" | "platlib").then(|| PathBuf::from(inner));
    }
    Some(PathBuf::from(rel))
}

/// Where each member of the MSYS2 packages goes in `deps\` (make_release.py `build_deps`): Opus, libvpx, Vulkan, VMA
/// and zstd headers to `include\`, their libraries to `lib\`, libopus-0.dll to `bin\`.
pub fn deps_member(pkg: &str, rel: &str) -> Option<PathBuf> {
    let rel = rel.strip_prefix("ucrt64/")?;
    let inc = |dirs: &[&str]| dirs.iter().any(|d| rel.starts_with(&format!("include/{d}/"))).then(|| PathBuf::from(rel));
    let lib = |names: &[&str]| names.iter().any(|n| rel == format!("lib/{n}")).then(|| PathBuf::from(rel));
    match pkg {
        "opus" => inc(&["opus"]).or_else(|| lib(&["libopus.dll.a"])).or_else(|| (rel == "bin/libopus-0.dll").then(|| PathBuf::from(rel))),
        "libvpx" => inc(&["vpx"]).or_else(|| lib(&["libvpx.a"])),
        "vulkan-headers" => inc(&["vulkan", "vk_video"]),
        "vulkan-loader" => lib(&["libvulkan-1.dll.a"]),
        "vma" => inc(&["vma"]),
        "zstd" => ["include/zstd.h", "include/zstd_errors.h", "lib/libzstd.a"].contains(&rel).then(|| PathBuf::from(rel)),
        _ => None,
    }
}

// ---- the two installs --------------------------------------------------------------------------------------------

/// Progress of [`ensure_python`] and [`ensure_toolchain`]: `(stage, fraction)`; stages `download` (with bytes) and
/// `unpack`.
pub type Report<'a> = &'a mut dyn FnMut(&str, f64, Option<(u64, u64)>);

fn fresh_dir(p: &Path) -> Result<(), Error> {
    sys::remove_all(p).map_err(io(p))?;
    std::fs::create_dir_all(p).map_err(io(p))
}

/// Replaces `dst` with `tmp` (a folder made beside it).
fn swap_in(tmp: &Path, dst: &Path) -> Result<(), Error> {
    sys::remove_all(dst).map_err(io(dst))?;
    std::fs::rename(tmp, dst).map_err(io(dst))
}

/// Python 3.14.9 (embeddable) with the wheels, in `<root>\tools\python`. Returns false when it was already there.
pub fn ensure_python(root: &Path, cancel: &AtomicBool, report: Report) -> Result<bool, Error> {
    if python_ready(root) {
        return Ok(false);
    }
    let dl = downloads(root);
    let pins = python_pins();
    let files = download_all(&pins, &dl, cancel, &mut |d, t| report("download", 0.8 * d as f64 / t.max(1) as f64, Some((d, t))))?;
    report("unpack", 0.85, None);
    let tmp = tools(root).join("python.new");
    fresh_dir(&tmp)?;
    unzip(&files[0], &tmp, &|n| Some(PathBuf::from(n)), cancel)?;
    // Without its ._pth file the embeddable Python finds its standard library zip, then Lib\site-packages, and puts the
    // script's folder first on sys.path, like an installed Python (the install scripts import their neighbours).
    for e in std::fs::read_dir(&tmp).map_err(io(&tmp))?.flatten() {
        if e.file_name().to_string_lossy().ends_with("._pth") {
            std::fs::remove_file(e.path()).map_err(io(&e.path()))?;
        }
    }
    let site = tmp.join("Lib").join("site-packages");
    std::fs::create_dir_all(&site).map_err(io(&site))?;
    for w in &files[1..] {
        unzip(w, &site, &|n| wheel_member(n), cancel)?;
    }
    write_marker(&tmp.join("kalamar-python.json"), python_id(), &pins).map_err(io(&tmp))?;
    swap_in(&tmp, &python_dir(root))?;
    for f in files {
        let _ = std::fs::remove_file(f);
    }
    report("unpack", 1.0, None);
    Ok(true)
}

/// llvm-mingw, Ninja and the libraries, in `<root>\tools`. Returns false when they were already there. `hactool`: the
/// launcher's own hactool.exe to put in `tools\bin` too (it is not downloaded).
pub fn ensure_toolchain(root: &Path, cancel: &AtomicBool, report: Report) -> Result<bool, Error> {
    if toolchain_ready(root) {
        return Ok(false);
    }
    let dl = downloads(root);
    let files = download_all(&TOOLCHAIN, &dl, cancel, &mut |d, t| report("download", 0.85 * d as f64 / t.max(1) as f64, Some((d, t))))?;
    let file = |p: &Pin| files[TOOLCHAIN.iter().position(|q| q == p).expect("pinned")].clone();
    report("unpack", 0.86, None);
    let tools = tools(root);
    // deps\: SDL3 (upstream build: its DLL and its MinGW import library and headers), then the MSYS2 packages.
    let deps = tools.join("deps.new");
    fresh_dir(&deps)?;
    let sdl_dir = format!("SDL3-{SDL3_VERSION}/x86_64-w64-mingw32/");
    untar_gz(&file(&SDL3_DEVEL), &deps, &|p| {
        let rel = p.strip_prefix(&sdl_dir)?;
        (rel.starts_with("include/SDL3/") || rel == "lib/libSDL3.dll.a").then(|| PathBuf::from(rel))
    })?;
    unzip(&file(&SDL3_DLL), &deps.join("bin"), &|n| (n == "SDL3.dll").then(|| PathBuf::from(n)), cancel)?;
    let dll = deps.join("bin").join("SDL3.dll");
    if file_sha256(&dll).map_err(io(&dll))? != SDL3_DLL_INNER {
        return Err(Error::Archive { name: SDL3_DLL.name.into(), message: "SDL3.dll is not the pinned one".into() });
    }
    for (pkg, pin) in [("opus", OPUS), ("libvpx", LIBVPX), ("vulkan-headers", VULKAN_HEADERS), ("vulkan-loader", VULKAN_LOADER), ("vma", VMA), ("zstd", ZSTD)] {
        untar_zst(&file(&pin), &deps, &|p| deps_member(pkg, p))?;
    }
    for need in ["include/SDL3/SDL.h", "lib/libSDL3.dll.a", "lib/libvpx.a", "lib/libzstd.a", "lib/libvulkan-1.dll.a", "include/vma/vk_mem_alloc.h", "bin/libopus-0.dll"] {
        if !deps.join(need).is_file() {
            return Err(Error::Archive { name: "deps".into(), message: format!("{need} missing") });
        }
    }
    swap_in(&deps, &deps_dir(root))?;
    report("unpack", 0.88, None);
    let bin = bin_dir(root);
    std::fs::create_dir_all(&bin).map_err(io(&bin))?;
    unzip(&file(&NINJA), &bin, &|n| (n == "ninja.exe").then(|| PathBuf::from(n)), cancel)?;
    // llvm\: the large one.
    let llvm = tools.join("llvm.new");
    fresh_dir(&llvm)?;
    let top = format!("llvm-mingw-{LLVM_VERSION}-ucrt-x86_64/");
    let n = unzip(&file(&LLVM_MINGW), &llvm, &|p| p.strip_prefix(&top).filter(|r| llvm_keep(r)).map(PathBuf::from), cancel)?;
    if !llvm.join("bin").join("clang.exe").is_file() || n < 1000 {
        return Err(Error::Archive { name: LLVM_MINGW.name.into(), message: format!("only {n} files unpacked") });
    }
    swap_in(&llvm, &llvm_dir(root))?;
    write_marker(&toolchain_marker(root), toolchain_id(), &TOOLCHAIN).map_err(io(&tools))?;
    for f in files {
        let _ = std::fs::remove_file(f);
    }
    let _ = std::fs::remove_dir(&dl);
    report("unpack", 1.0, None);
    Ok(true)
}

/// Extra arguments of `tools/build_game.py` that build with the portable toolchain.
pub fn build_args(root: &Path) -> Vec<String> {
    let s = |p: PathBuf| p.display().to_string();
    vec!["--toolchain".into(), s(llvm_dir(root)), "--deps".into(), s(deps_dir(root)), "--ninja".into(), s(ninja_exe(root))]
}

/// `[(group, source, bytes)]` of what a first install downloads, for the plan screen.
pub fn download_groups() -> Vec<(&'static str, String, u64)> {
    vec![
        ("python", "python.org (Python 3.14.9, embeddable)".into(), PYTHON_EMBED.size),
        ("pypi", "pypi.org (numpy, lz4, cryptography, cffi, pycparser)".into(), bytes(&WHEELS)),
        ("llvm", format!("github.com/mstorsjo/llvm-mingw ({LLVM_VERSION})"), LLVM_MINGW.size),
        ("ninja", "github.com/ninja-build/ninja (1.13.2)".into(), NINJA.size),
        ("sdl3", format!("github.com/libsdl-org/SDL ({SDL3_VERSION})"), SDL3_DLL.size + SDL3_DEVEL.size),
        ("msys2", "repo.msys2.org (Opus, libvpx, Vulkan, VMA, zstd)".into(), bytes(&[OPUS, LIBVPX, VULKAN_HEADERS, VULKAN_LOADER, VMA, ZSTD])),
    ]
}

/// Reads the whole file once so a later run of it starts from a warm cache (the clang DLLs are large).
pub fn prefetch(path: &Path) {
    if let Ok(mut f) = std::fs::File::open(path) {
        let _ = f.seek(SeekFrom::Start(0));
        let mut buf = vec![0u8; 1 << 20];
        while f.read(&mut buf).is_ok_and(|n| n > 0) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ZIP written the plain way (stored or deflated members, central directory, end record).
    fn make_zip(members: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, data, deflate) in members {
            let mut crc = flate2::Crc::new();
            crc.update(data);
            let body = if *deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let method: u16 = if *deflate { 8 } else { 0 };
            let off = out.len() as u32;
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend([20u8, 0, 0, 0]);
            out.extend(method.to_le_bytes());
            out.extend([0u8; 4]);
            out.extend(crc.sum().to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(&body);
            cd.extend(0x0201_4b50u32.to_le_bytes());
            cd.extend([20u8, 0, 20, 0, 0, 0]);
            cd.extend(method.to_le_bytes());
            cd.extend([0u8; 4]);
            cd.extend(crc.sum().to_le_bytes());
            cd.extend((body.len() as u32).to_le_bytes());
            cd.extend((data.len() as u32).to_le_bytes());
            cd.extend((name.len() as u16).to_le_bytes());
            cd.extend([0u8; 12]);
            cd.extend(off.to_le_bytes());
            cd.extend(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        out.extend(&cd);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend((members.len() as u16).to_le_bytes());
        out.extend((members.len() as u16).to_le_bytes());
        out.extend((cd.len() as u32).to_le_bytes());
        out.extend(cd_off.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("s3l-portable-{tag}-{}", crate::util::random_hex(4)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn zip_members_are_read_and_checked() {
        let big = vec![7u8; 100_000];
        let z = make_zip(&[("a/stored.txt", b"hello", false), ("a/big.bin", &big, true), ("dir/", b"", false)]);
        let mut c = std::io::Cursor::new(z.clone());
        let e = zip::entries(&mut c).unwrap();
        assert_eq!(e.len(), 3);
        assert!(e[2].is_dir());
        let mut out = Vec::new();
        zip::read_entry(&mut c, &e[1], &mut out).unwrap();
        assert_eq!(out, big);
        // a flipped byte in the stored data fails the CRC
        let mut broken = z;
        let at = broken.windows(5).position(|w| w == b"hello").unwrap();
        broken[at] = b'j';
        let mut c = std::io::Cursor::new(broken);
        let e = zip::entries(&mut c).unwrap();
        assert!(zip::read_entry(&mut c, &e[0], &mut Vec::new()).is_err());
        assert!(zip::entries(&mut std::io::Cursor::new(b"not a zip at all, really not".to_vec())).is_err());
    }

    #[test]
    fn unzip_keeps_what_the_map_keeps_and_refuses_unsafe_paths() {
        let d = temp("unzip");
        let z = d.join("t.zip");
        std::fs::write(&z, make_zip(&[("top/bin/clang.exe", b"c", true), ("top/bin/clangd.exe", b"d", false), ("top/lib/x.a", b"x", false)])).unwrap();
        let no = AtomicBool::new(false);
        let n = unzip(&z, &d.join("out"), &|p| p.strip_prefix("top/").filter(|r| llvm_keep(r)).map(PathBuf::from), &no).unwrap();
        assert_eq!(n, 1);
        assert!(d.join("out").join("bin").join("clang.exe").is_file() && !d.join("out").join("bin").join("clangd.exe").exists());
        std::fs::write(&z, make_zip(&[("../evil.txt", b"e", false)])).unwrap();
        assert!(matches!(unzip(&z, &d.join("out2"), &|p| Some(PathBuf::from(p)), &no), Err(Error::Archive { .. })));
        assert!(!d.join("evil.txt").exists());
        let yes = AtomicBool::new(true);
        std::fs::write(&z, make_zip(&[("a.txt", b"a", false)])).unwrap();
        assert_eq!(unzip(&z, &d.join("out3"), &|p| Some(PathBuf::from(p)), &yes), Err(Error::Cancelled));
        sys::remove_all(&d).unwrap();
    }

    #[test]
    fn the_llvm_selection_is_make_releases() {
        assert!(llvm_keep("bin/clang.exe") && llvm_keep("bin/ld.lld.exe") && llvm_keep("bin/libLLVM-23.dll"));
        assert!(!llvm_keep("bin/clangd.exe") && !llvm_keep("bin/clang-tidy.exe"));
        assert!(llvm_keep("lib/clang/23/include/immintrin.h"));
        assert!(llvm_keep("lib/clang/23/lib/windows/libclang_rt.builtins-x86_64.a"));
        assert!(!llvm_keep("lib/clang/23/lib/windows/libclang_rt.asan-x86_64.a"));
        assert!(llvm_keep("x86_64-w64-mingw32/lib/libc++.a") && llvm_keep("include/windows.h") && llvm_keep("LICENSE.TXT"));
        assert!(!llvm_keep("aarch64-w64-mingw32/lib/libc++.a") && !llvm_keep("python/bin/python3.exe") && !llvm_keep("include/ddk/ntddk.h"));
        assert!(!llvm_keep("include/x.idl"));
    }

    #[test]
    fn wheel_and_package_members() {
        assert_eq!(wheel_member("numpy/__init__.py"), Some(PathBuf::from("numpy/__init__.py")));
        assert_eq!(wheel_member("numpy.libs/x.dll"), Some(PathBuf::from("numpy.libs/x.dll")));
        assert_eq!(wheel_member("cffi-2.1.1.data/platlib/_cffi.pyd"), Some(PathBuf::from("_cffi.pyd")));
        assert_eq!(wheel_member("numpy-2.5.4.data/scripts/f2py.exe"), None);
        assert_eq!(deps_member("opus", "ucrt64/include/opus/opus.h"), Some(PathBuf::from("include/opus/opus.h")));
        assert_eq!(deps_member("opus", "ucrt64/bin/libopus-0.dll"), Some(PathBuf::from("bin/libopus-0.dll")));
        assert_eq!(deps_member("opus", "ucrt64/lib/libopus.a"), None);
        assert_eq!(deps_member("libvpx", "ucrt64/lib/libvpx.a"), Some(PathBuf::from("lib/libvpx.a")));
        assert_eq!(deps_member("libvpx", "ucrt64/bin/vpxenc.exe"), None);
        assert_eq!(deps_member("vulkan-headers", "ucrt64/include/vk_video/x.h"), Some(PathBuf::from("include/vk_video/x.h")));
        assert_eq!(deps_member("zstd", "ucrt64/include/zstd.h"), Some(PathBuf::from("include/zstd.h")));
        assert_eq!(deps_member("zstd", "ucrt64/include/zdict.h"), None);
        assert_eq!(deps_member("vma", ".PKGINFO"), None);
    }

    #[test]
    fn pins_are_https_and_well_formed() {
        for p in all_pins() {
            assert!(p.url.starts_with("https://"), "{}", p.url);
            assert!(p.url.ends_with(p.name), "{} / {}", p.url, p.name);
            assert_eq!(p.sha256.len(), 64);
            assert!(p.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            assert!(p.size > 10_000);
        }
        assert!(toolchain_id().starts_with("llvm-mingw-20261020-") && toolchain_id() != python_id());
        let groups: u64 = download_groups().iter().map(|g| g.2).sum();
        assert_eq!(groups, bytes(&all_pins()));
        assert!((200_000_000..250_000_000).contains(&groups), "{groups}");
    }

    #[test]
    fn a_download_of_a_non_https_url_is_refused() {
        let d = temp("http");
        let no = AtomicBool::new(false);
        let pin = Pin { name: "x.zip", url: "http://example.invalid/x.zip", sha256: "00", size: 1 };
        let f = Fetcher::new().unwrap();
        assert!(matches!(f.fetch(&pin, &d.join("x.zip"), &no, &mut |_| {}), Err(Error::Network { .. })));
        sys::remove_all(&d).unwrap();
    }

    #[test]
    fn readiness_follows_the_markers() {
        let root = temp("ready");
        assert!(!toolchain_ready(&root) && !python_ready(&root));
        for p in [llvm_dir(&root).join("bin").join("clang.exe"), ninja_exe(&root), deps_dir(&root).join("lib").join("libSDL3.dll.a"), python_exe(&root)] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
        assert!(!toolchain_ready(&root), "no marker yet");
        write_marker(&toolchain_marker(&root), toolchain_id(), &TOOLCHAIN).unwrap();
        write_marker(&python_marker(&root), python_id(), &python_pins()).unwrap();
        assert!(toolchain_ready(&root) && python_ready(&root));
        write_marker(&toolchain_marker(&root), "llvm-mingw-old".into(), &TOOLCHAIN).unwrap();
        assert!(!toolchain_ready(&root), "another pin set");
        let a = build_args(&root);
        assert_eq!(a[0], "--toolchain");
        assert!(a.iter().any(|x| x.ends_with("ninja.exe")));
        sys::remove_all(&root).unwrap();
    }
}
