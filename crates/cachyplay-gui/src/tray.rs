//! The optional tray icon (off unless turned on in Settings › General) and, when it's off, desktop
//! notifications for the same news. House Standard states: Idle (plain) · Active (lit) · Muted on
//! purpose (red badge) · Needs attention (amber dot) · Problem (red dot) · Disconnected (grey).
//! A state is held at least 400 ms and the icon is only redrawn when the state changes.

use cachyplay_core::library::{self, Overview};
use cachyplay_core::live::{self, SteamState};
use cachyplay_core::manage::Ctx;
use cachyplay_core::paths::Paths;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::Manager;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Steam is there, nothing going on.
    Idle,
    /// A game is running with CachyPlay on.
    Active,
    /// A game is running with CachyPlay turned off for it (on purpose).
    Muted,
    /// A fix is being tested, a change waits for Steam to close, or an update is ready.
    Attention,
    /// The last game start crashed or didn't start.
    Problem,
    /// Steam wasn't found.
    Disconnected,
}

impl State {
    fn png(self) -> &'static [u8] {
        match self {
            State::Idle => include_bytes!("../../../assets/tray/idle.png"),
            State::Active => include_bytes!("../../../assets/tray/active.png"),
            State::Muted => include_bytes!("../../../assets/tray/muted.png"),
            State::Attention => include_bytes!("../../../assets/tray/attention.png"),
            State::Problem => include_bytes!("../../../assets/tray/problem.png"),
            State::Disconnected => include_bytes!("../../../assets/tray/disconnected.png"),
        }
    }
}

/// What the tray (or a notification) says.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub state: State,
    pub tooltip: String,
}

/// An update is ready (set when the app checks for updates).
pub static UPDATE_READY: AtomicBool = AtomicBool::new(false);

/// Work out the status from Steam and the game list. Only called when something changed.
pub fn compute(p: &Paths, c: Result<&Ctx, String>) -> Status {
    let ctx = match c {
        Ok(c) => c,
        Err(_) => {
            return Status {
                state: State::Disconnected,
                tooltip: "CachyPlay: Steam wasn't found".into(),
            }
        }
    };
    let ov: Overview = library::overview(p, Ok(ctx));
    let sessions = cachyplay_core::crashmatch::parse_steam_sessions(&ctx.steam.console_logs());
    let steam_open = live::steam_state(&ctx.proc_root) != SteamState::Closed;
    let name = |id: &str| {
        ov.games
            .iter()
            .find(|g| g.app_id == id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| format!("Game {id}"))
    };
    let running = sessions
        .iter()
        .rev()
        .find(|s| s.end.is_none())
        .filter(|_| steam_open);
    let last_done = sessions.iter().rev().find(|s| s.end.is_some());
    let failed = last_done.and_then(|s| {
        ov.games
            .iter()
            .find(|g| g.app_id == s.app_id && g.last_failed)
    });
    let testing = ov.games.iter().find(|g| {
        matches!(
            g.stage.as_deref(),
            Some("Testing the fix") | Some("Seems to work")
        )
    });
    let waiting = ov.games.iter().find(|g| g.waiting.is_some());
    let (state, tooltip) = if let (Some(g), None) = (failed, running) {
        (
            State::Problem,
            format!("CachyPlay: {} crashed or didn't start last time", g.name),
        )
    } else if let Some(s) = running {
        let on = ov.games.iter().any(|g| g.app_id == s.app_id && g.enabled);
        if on {
            (
                State::Active,
                format!("CachyPlay: playing {}", name(&s.app_id)),
            )
        } else {
            (
                State::Muted,
                format!("CachyPlay: off for {} (you turned it off)", name(&s.app_id)),
            )
        }
    } else if UPDATE_READY.load(Ordering::Relaxed) {
        (State::Attention, "CachyPlay: an update is ready".into())
    } else if let Some(g) = waiting {
        (
            State::Attention,
            format!(
                "CachyPlay: a change for {} waits for Steam to close",
                g.name
            ),
        )
    } else if let Some(g) = testing {
        (
            State::Attention,
            format!("CachyPlay: testing a fix for {}", g.name),
        )
    } else {
        (State::Idle, "CachyPlay".into())
    };
    Status { state, tooltip }
}

struct Shown {
    status: Status,
    since: Instant,
}

static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);
const HOLD: Duration = Duration::from_millis(400);

/// Whether the tray library this desktop needs is installed (it's loaded when the tray is made;
/// without it, creating a tray would fail).
pub fn supported() -> bool {
    [
        "/usr/lib/libayatana-appindicator3.so.1",
        "/usr/lib64/libayatana-appindicator3.so.1",
        "/usr/lib/x86_64-linux-gnu/libayatana-appindicator3.so.1",
        "/usr/lib/libappindicator3.so.1",
        "/usr/lib/x86_64-linux-gnu/libappindicator3.so.1",
    ]
    .iter()
    .any(|p| std::path::Path::new(p).exists())
}

/// Show or remove the tray icon to match the setting. Runs on the main thread.
pub fn apply(app: &tauri::AppHandle, on: bool) -> Result<(), String> {
    let existing = app.tray_by_id("main");
    if !on {
        if existing.is_some() {
            let _ = app.remove_tray_by_id("main");
        }
        if let Ok(mut s) = SHOWN.lock() {
            *s = None;
        }
        return Ok(());
    }
    if existing.is_some() {
        return Ok(());
    }
    if !supported() {
        return Err("The tray icon needs libayatana-appindicator. Install it with: sudo pacman -S libayatana-appindicator. Until then, CachyPlay uses desktop notifications.".into());
    }
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
    let open = MenuItem::with_id(app, "open", "Open CachyPlay", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let quit =
        MenuItem::with_id(app, "quit", "Quit", true, None::<&str>).map_err(|e| e.to_string())?;
    let menu = Menu::with_items(app, &[&open, &quit]).map_err(|e| e.to_string())?;
    let icon = tauri::image::Image::from_bytes(State::Idle.png()).map_err(|e| e.to_string())?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("CachyPlay")
        .menu(&menu)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "open" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = e
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn show_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Redraw the tray if the status changed (and the last state has been shown for 400 ms).
/// Returns true when the status changed.
pub fn update(app: &tauri::AppHandle, status: &Status) -> bool {
    let Ok(mut shown) = SHOWN.lock() else {
        return false;
    };
    if let Some(s) = shown.as_ref() {
        if s.status == *status || s.since.elapsed() < HOLD {
            return false;
        }
    }
    *shown = Some(Shown {
        status: status.clone(),
        since: Instant::now(),
    });
    if let Some(tray) = app.tray_by_id("main") {
        let png = status.state.png();
        let tip = status.tooltip.clone();
        let _ = app.run_on_main_thread(move || {
            if let Ok(img) = tauri::image::Image::from_bytes(png) {
                let _ = tray.set_icon(Some(img));
            }
            let _ = tray.set_tooltip(Some(tip));
        });
    }
    true
}

/// A desktop notification (used when the tray is off). Best-effort: needs notify-send.
pub fn notify(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args([
            "--app-name=CachyPlay",
            "--icon=io.github.matt-b-007.cachyplay",
            title,
            body,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_an_icon() {
        for s in [
            State::Idle,
            State::Active,
            State::Muted,
            State::Attention,
            State::Problem,
            State::Disconnected,
        ] {
            tauri::image::Image::from_bytes(s.png()).expect("valid PNG");
        }
    }

    #[test]
    fn no_steam_means_disconnected() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        let s = compute(&p, Err("no Steam".into()));
        assert_eq!(s.state, State::Disconnected);
    }
}
