//! The tools the build needs, found the way `Install-Splat3Recomp.ps1` finds them (Find-Git, Find-Dotnet, Find-Python,
//! Find-VsCpp, Missing-MsysPackages), and what installs them: winget's official packages and MSYS2's pacman.

use super::sys;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const MSYS_ROOT: &str = r"C:\msys64";
pub const WINGET_MSYS2: &str = "MSYS2.MSYS2";
pub const WINGET_DOTNET: &str = "Microsoft.DotNet.SDK.10";
pub const WINGET_PYTHON: &str = "Python.Python.3.13";
pub const WINGET_GIT: &str = "Git.Git";
pub const WINGET_VS: &str = "Microsoft.VisualStudio.BuildTools";
/// winget `--override` for the Visual Studio Build Tools (the C++ workload NativeAOT links with).
pub const VS_OVERRIDE: &str = "--passive --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended";

/// MSYS2 UCRT64 packages, then the base ones (the script's `$MsysPackages`).
pub fn msys_packages() -> Vec<String> {
    let ucrt = [
        "clang", "lld", "ninja", "gcc", "make", "pkgconf", "sdl3", "opus", "libvpx", "zstd", "vulkan-devel", "vulkan-memory-allocator",
        "vulkan-validation-layers", "shaderc", "spirv-tools",
    ];
    let mut v: Vec<String> = ucrt.iter().map(|p| format!("mingw-w64-ucrt-x86_64-{p}")).collect();
    v.extend(["base-devel", "git", "unzip"].iter().map(|s| s.to_string()));
    v
}

/// Runs a detection command without a window; `None` on failure to start or after the timeout.
pub fn run_capture(exe: &Path, args: &[&str], timeout: Duration) -> Option<(i32, String)> {
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    sys::no_window(&mut cmd);
    let mut child = cmd.spawn().ok()?;
    let out = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut o) = out {
            let _ = std::io::Read::read_to_string(&mut o, &mut s);
        }
        s
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return Some((st.code().unwrap_or(-1), reader.join().unwrap_or_default())),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

const T: Duration = Duration::from_secs(30);

/// `exe` names found on PATH, in order. Only absolute entries: an empty or relative one would mean the current folder.
pub fn on_path(exe: &str) -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).filter(|d| d.is_absolute()).map(|d| d.join(exe)).filter(|p| p.is_file()).collect())
        .unwrap_or_default()
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

pub fn find_git() -> Option<PathBuf> {
    let mut c = on_path("git.exe");
    c.push(PathBuf::from(r"C:\Program Files\Git\cmd\git.exe"));
    if let Some(l) = env_dir("LOCALAPPDATA") {
        c.push(l.join(r"Programs\Git\cmd\git.exe"));
    }
    c.into_iter().find(|p| p.is_file())
}

pub fn find_dotnet() -> Option<PathBuf> {
    let mut c = on_path("dotnet.exe");
    c.push(PathBuf::from(r"C:\Program Files\dotnet\dotnet.exe"));
    c.into_iter()
        .filter(|p| p.is_file())
        .find(|d| run_capture(d, &["--list-sdks"], T).is_some_and(|(_, out)| out.lines().any(|l| l.trim_start().starts_with("10."))))
}

fn is_py313(p: &Path) -> bool {
    run_capture(p, &["-c", "import sys;print(sys.version_info[:2]==(3,13))"], T).is_some_and(|(_, o)| o.trim() == "True")
}

pub fn find_python() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    for py in on_path("py.exe") {
        if let Some((0, out)) = run_capture(&py, &["-3.13", "-c", "import sys;print(sys.executable)"], T) {
            let p = PathBuf::from(out.trim());
            if !out.trim().is_empty() {
                c.push(p);
            }
        }
    }
    if let Some(l) = env_dir("LOCALAPPDATA") {
        c.push(l.join(r"Programs\Python\Python313\python.exe"));
    }
    c.push(PathBuf::from(r"C:\Program Files\Python313\python.exe"));
    c.extend(on_path("python.exe"));
    c.into_iter().filter(|p| p.is_file()).find(|p| is_py313(p))
}

pub fn vswhere_path() -> PathBuf {
    let pf86 = env_dir("ProgramFiles(x86)").unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    pf86.join(r"Microsoft Visual Studio\Installer\vswhere.exe")
}

pub fn find_vs_cpp() -> Option<PathBuf> {
    let vw = vswhere_path();
    if !vw.is_file() {
        return None;
    }
    let (_, out) = run_capture(
        &vw,
        &["-products", "*", "-latest", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"],
        T,
    )?;
    let p = out.lines().next().map(str::trim).filter(|s| !s.is_empty())?;
    Some(PathBuf::from(p))
}

pub fn find_winget() -> Option<PathBuf> {
    let mut c = on_path("winget.exe");
    if let Some(l) = env_dir("LOCALAPPDATA") {
        c.push(l.join(r"Microsoft\WindowsApps\winget.exe"));
    }
    c.into_iter().find(|p| p.exists())
}

pub fn msys_bash() -> PathBuf {
    Path::new(MSYS_ROOT).join(r"usr\bin\bash.exe")
}

/// Packages of [`msys_packages`] that pacman does not report as installed (a group counts when pacman lists its
/// members).
pub fn missing_msys_packages() -> Vec<String> {
    let pm = Path::new(MSYS_ROOT).join(r"usr\bin\pacman.exe");
    let all = msys_packages();
    if !pm.is_file() {
        return all;
    }
    let have: Vec<String> = run_capture(&pm, &["-Qq"], T).map(|(_, o)| o.lines().map(|l| l.trim().to_string()).collect()).unwrap_or_default();
    all.into_iter()
        .filter(|p| !have.contains(p))
        .filter(|p| !run_capture(&pm, &["-Qgq", p], T).is_some_and(|(_, o)| o.lines().any(|l| !l.trim().is_empty())))
        .collect()
}

/// MSYS2 UCRT64 has what `tools/build_game.py` compiles and links the game with (installs whose objects were
/// compiled with it keep using it; a first install never needs it).
pub fn msys2_builds() -> bool {
    // Asked once per launcher session (pacman takes seconds; the Play page asks at each visit).
    static ANSWER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ANSWER.get_or_init(msys2_builds_now)
}

fn msys2_builds_now() -> bool {
    let bin = Path::new(MSYS_ROOT).join("ucrt64").join("bin");
    if !["clang.exe", "ld.lld.exe", "ninja.exe", "SDL3.dll", "libopus-0.dll"].iter().all(|f| bin.join(f).is_file()) {
        return false;
    }
    let need = ["clang", "lld", "ninja", "sdl3", "opus", "libvpx", "zstd", "vulkan-devel", "vulkan-memory-allocator"];
    let missing = missing_msys_packages();
    !need.iter().any(|p| missing.contains(&format!("mingw-w64-ucrt-x86_64-{p}")))
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Tool {
    /// `msys2`, `dotnet`, `python`, `git`, `vs_cpp`.
    pub id: &'static str,
    pub winget: &'static str,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Detection {
    pub tools: Vec<Tool>,
    pub missing_packages: Vec<String>,
    pub winget: Option<String>,
}

impl Detection {
    pub fn missing_tools(&self) -> Vec<&Tool> {
        self.tools.iter().filter(|t| t.path.is_none()).collect()
    }
    pub fn complete(&self) -> bool {
        self.missing_tools().is_empty() && self.missing_packages.is_empty()
    }
    pub fn path_of(&self, id: &str) -> Option<PathBuf> {
        self.tools.iter().find(|t| t.id == id).and_then(|t| t.path.clone()).map(PathBuf::from)
    }
}

pub fn detect() -> Detection {
    let s = |p: Option<PathBuf>| p.map(|p| p.display().to_string());
    let bash = msys_bash();
    let tools = vec![
        Tool { id: "msys2", winget: WINGET_MSYS2, path: bash.is_file().then(|| bash.display().to_string()) },
        Tool { id: "dotnet", winget: WINGET_DOTNET, path: s(find_dotnet()) },
        Tool { id: "python", winget: WINGET_PYTHON, path: s(find_python()) },
        Tool { id: "git", winget: WINGET_GIT, path: s(find_git()) },
        Tool { id: "vs_cpp", winget: WINGET_VS, path: s(find_vs_cpp()) },
    ];
    Detection { tools, missing_packages: missing_msys_packages(), winget: s(find_winget()) }
}

/// The winget command line for one package (the script's `winget install` arguments, VS override included).
pub fn winget_args(id: &str) -> Vec<String> {
    let mut a: Vec<String> = ["install", "--id", id, "-e", "--source", "winget", "--accept-package-agreements", "--accept-source-agreements"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if id == WINGET_VS {
        a.push("--override".into());
        a.push(VS_OVERRIDE.into());
    }
    a
}

/// winget exit codes: 0 installed; APPINSTALLER_CLI_ERROR_UPDATE_NOT_APPLICABLE (already there, accepted by the
/// script); "restart required to finish" (the Build Tools with `--norestart`).
pub const WINGET_ALREADY: i32 = -1_978_335_189; // 0x8A15002B
pub const WINGET_REBOOT: i32 = -1_978_334_967; // 0x8A150109
pub const MSI_REBOOT: i32 = 3010;

#[derive(Debug, PartialEq, Eq)]
pub enum WingetOutcome {
    Ok,
    Reboot,
    Failed,
}

pub fn winget_outcome(code: i32) -> WingetOutcome {
    match code {
        0 | WINGET_ALREADY => WingetOutcome::Ok,
        WINGET_REBOOT | MSI_REBOOT => WingetOutcome::Reboot,
        _ => WingetOutcome::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_list_matches_the_script() {
        let p = msys_packages();
        assert_eq!(p.len(), 18);
        assert_eq!(p[0], "mingw-w64-ucrt-x86_64-clang");
        assert!(p.contains(&"mingw-w64-ucrt-x86_64-vulkan-memory-allocator".to_string()));
        assert_eq!(&p[15..], ["base-devel", "git", "unzip"]);
    }

    #[test]
    fn winget_arguments_and_codes() {
        let a = winget_args(WINGET_GIT);
        assert_eq!(a[..4], ["install", "--id", "Git.Git", "-e"]);
        assert!(a.contains(&"--source".to_string()) && a.contains(&"winget".to_string()));
        assert!(!a.contains(&"--override".to_string()));
        let v = winget_args(WINGET_VS);
        assert_eq!(v[v.len() - 1], VS_OVERRIDE);
        assert_eq!(winget_outcome(0), WingetOutcome::Ok);
        assert_eq!(winget_outcome(-1978335189), WingetOutcome::Ok);
        assert_eq!(winget_outcome(-1978334967), WingetOutcome::Reboot);
        assert_eq!(winget_outcome(1), WingetOutcome::Failed);
    }

    #[test]
    fn capture_times_out_and_reads_output() {
        let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let cmd = sys.join(r"System32\cmd.exe");
        let (code, out) = run_capture(&cmd, &["/C", "echo hi"], T).unwrap();
        assert_eq!(code, 0);
        assert_eq!(out.trim(), "hi");
        assert!(run_capture(&cmd, &["/C", "ping -n 5 127.0.0.1 >NUL"], Duration::from_millis(200)).is_none());
        assert!(run_capture(Path::new(r"Z:\none.exe"), &[], T).is_none());
    }
}
