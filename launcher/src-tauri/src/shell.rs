//! The launcher window as a native-game launcher would behave: created by the backend (not from the configuration) so
//! that it opens where the player left it, with the theme's color from its very first frame (no white flash while
//! the WebView starts), minimized on request, and its size and position remembered (`window.json` next to
//! `config.json`).

use crate::cli::Cli;
use crate::util;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{Manager, PhysicalPosition};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub maximized: bool,
}

pub fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join("window.json")
}

pub fn load_state(data_dir: &Path) -> Option<WindowState> {
    let text = util::read_text_limited(&state_path(data_dir), 4096).ok().flatten()?;
    serde_json::from_str(&text).ok()
}

pub fn save_state(data_dir: &Path, s: &WindowState) {
    if let Ok(text) = serde_json::to_string(s) {
        let _ = util::atomic_write(&state_path(data_dir), text.as_bytes());
    }
}

/// A saved window only comes back if enough of its title bar is on a screen that still exists (a monitor unplugged
/// since, or a different layout, must not leave the window out of reach); the size is capped to the biggest screen.
/// Screens are `(x, y, width, height)`. `None`: leave the window where the system puts it.
pub fn fit(s: &WindowState, screens: &[(i32, i32, u32, u32)]) -> Option<WindowState> {
    if s.w < 200 || s.h < 150 || screens.is_empty() {
        return None;
    }
    let (maxw, maxh) = screens.iter().fold((0, 0), |a, m| (a.0.max(m.2), a.1.max(m.3)));
    let (w, h) = (s.w.min(maxw), s.h.min(maxh));
    // the strip of the title bar that must be visible: 160 x 40 px at the top left of the window
    let (sx0, sy0, sx1, sy1) = (i64::from(s.x), i64::from(s.y), i64::from(s.x) + 160.min(i64::from(w)), i64::from(s.y) + 40);
    let seen = screens.iter().any(|&(mx, my, mw, mh)| {
        let (mx0, my0, mx1, my1) = (i64::from(mx), i64::from(my), i64::from(mx) + i64::from(mw), i64::from(my) + i64::from(mh));
        sx1.min(mx1) - sx0.max(mx0) >= 80 && sy1.min(my1) - sy0.max(my0) >= 20
    });
    seen.then_some(WindowState { w, h, ..*s })
}

static LAST_NORMAL: Mutex<Option<WindowState>> = Mutex::new(None);
static CLI: OnceLock<Cli> = OnceLock::new();
static CLI_TAKEN: AtomicBool = AtomicBool::new(false);

pub fn set_cli(c: Cli) {
    let _ = CLI.set(c);
}

pub fn minimized_start() -> bool {
    CLI.get().is_some_and(|c| c.minimized)
}

/// The command line, once: a page that reloads does not start the game a second time.
pub fn take_cli() -> Cli {
    let c = CLI.get().cloned().unwrap_or_default();
    if CLI_TAKEN.swap(true, Ordering::SeqCst) { Cli { play: None, invalid: Vec::new(), ..c } } else { c }
}

/// Creates the main window from its configuration entry (`"create": false` in `tauri.conf.json`, so that nothing
/// exists before this), at the saved place and size when they still fit a screen, then minimizes it for `--minimized`.
pub fn create_main_window(app: &tauri::App, data_dir: &Path) -> tauri::Result<()> {
    let Some(mut cfg) = app.config().app.windows.iter().find(|w| w.label == "main").cloned() else { return Ok(()) };
    let screens: Vec<(i32, i32, u32, u32)> =
        app.available_monitors().unwrap_or_default().iter().map(|m| (m.position().x, m.position().y, m.size().width, m.size().height)).collect();
    let saved = load_state(data_dir).and_then(|s| fit(&s, &screens));
    if saved.is_some() {
        cfg.center = false;
    }
    let mut builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?;
    // Tests that point the profile folders elsewhere (`KALAMAR_APPDATA`) keep the WebView's profile there too.
    if let Some(dir) = &crate::datadir::current().webview {
        builder = builder.data_directory(dir.clone());
    }
    if let Some(s) = saved {
        let scale = app.monitor_from_point(f64::from(s.x), f64::from(s.y)).ok().flatten().map(|m| m.scale_factor()).unwrap_or(1.0);
        builder = builder.inner_size(f64::from(s.w) / scale, f64::from(s.h) / scale).position(f64::from(s.x) / scale, f64::from(s.y) / scale);
        if s.maximized {
            builder = builder.maximized(true);
        }
        *LAST_NORMAL.lock().unwrap_or_else(|p| p.into_inner()) = Some(WindowState { maximized: false, ..s });
    }
    // The first screen's data travels with the page (a script run before the page's own): no round trip to wait for.
    if let Some(state) = app.try_state::<crate::commands::AppState>() {
        let data = crate::commands::boot_data(&state);
        if let Ok(json) = serde_json::to_string(&serde_json::json!({ "at": crate::util::now_secs() * 1000, "data": data })) {
            builder = builder.initialization_script(format!("window.__KALAMAR_BOOT__ = {json};"));
        }
    }
    let w = builder.build()?;
    if let Some(s) = saved
        && !s.maximized
        && w.outer_position().is_ok_and(|p| (p.x, p.y) != (s.x, s.y))
    {
        let _ = w.set_position(PhysicalPosition::new(s.x, s.y)); // mixed-DPI screens: put it exactly back
    }
    if minimized_start() {
        let _ = w.minimize();
    }
    Ok(())
}

/// Window events: keep the last normal geometry, write it when the window closes.
pub fn on_event(window: &tauri::Window, event: &tauri::WindowEvent, data_dir: &Path) {
    use tauri::WindowEvent::*;
    match event {
        Moved(_) | Resized(_) => {
            if window.is_maximized().unwrap_or(false) || window.is_minimized().unwrap_or(false) {
                return;
            }
            if let (Ok(p), Ok(sz)) = (window.outer_position(), window.inner_size()) {
                *LAST_NORMAL.lock().unwrap_or_else(|p| p.into_inner()) = Some(WindowState { x: p.x, y: p.y, w: sz.width, h: sz.height, maximized: false });
            }
        }
        CloseRequested { .. } => {
            let last = *LAST_NORMAL.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(mut s) = last {
                s.maximized = window.is_maximized().unwrap_or(false);
                save_state(data_dir, &s);
            }
        }
        _ => {}
    }
}

/// Brings the launcher back after the game ended (or to the front on request).
pub fn bring_back(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn minimize(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.minimize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: (i32, i32, u32, u32) = (0, 0, 1920, 1080);
    const SECOND: (i32, i32, u32, u32) = (-1920, 0, 1920, 1080);

    fn ws(x: i32, y: i32, w: u32, h: u32) -> WindowState {
        WindowState { x, y, w, h, maximized: false }
    }

    #[test]
    fn a_window_on_screen_comes_back_as_it_was() {
        assert_eq!(fit(&ws(100, 80, 1280, 800), &[SCREEN]), Some(ws(100, 80, 1280, 800)));
        // on a second screen to the left
        assert_eq!(fit(&ws(-1500, 50, 1280, 800), &[SCREEN, SECOND]), Some(ws(-1500, 50, 1280, 800)));
    }

    #[test]
    fn a_window_of_an_unplugged_screen_is_left_to_the_system() {
        assert_eq!(fit(&ws(-1500, 50, 1280, 800), &[SCREEN]), None);
        assert_eq!(fit(&ws(5000, 50, 1280, 800), &[SCREEN]), None);
        // the title bar strip only peeks in by a few pixels: not enough to grab
        assert_eq!(fit(&ws(1900, 50, 1280, 800), &[SCREEN]), None);
        assert_eq!(fit(&ws(100, 80, 1280, 800), &[]), None);
    }

    #[test]
    fn nonsense_sizes_are_refused_and_huge_ones_capped() {
        assert_eq!(fit(&ws(0, 0, 10, 10), &[SCREEN]), None);
        assert_eq!(fit(&ws(0, 0, 9000, 9000), &[SCREEN]), Some(ws(0, 0, 1920, 1080)));
    }

    #[test]
    fn the_state_file_round_trips_and_a_bad_one_is_ignored() {
        let d = std::env::temp_dir().join(format!("s3l-win-{}", util::random_hex(4)));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(load_state(&d), None);
        save_state(&d, &WindowState { maximized: true, ..ws(10, 20, 1100, 700) });
        assert_eq!(load_state(&d), Some(WindowState { maximized: true, ..ws(10, 20, 1100, 700) }));
        std::fs::write(state_path(&d), "{ nope").unwrap();
        assert_eq!(load_state(&d), None);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
