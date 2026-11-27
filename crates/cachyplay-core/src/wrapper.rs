// SPDX-License-Identifier: GPL-3.0-or-later
//! `cachyplay run -- <steam command>`: the launch wrapper Steam calls.
//!
//! FAIL-OPEN CONTRACT: nothing CachyPlay does may stop a game from launching.
//! Every preparation step runs inside one error boundary (errors *and* panics). If anything
//! goes wrong, the game starts exactly as Steam passed it. If even the modified launch can't
//! be spawned, the unmodified one is tried.

use crate::manage::{now, write_json};
use crate::paths::Paths;
use crate::recipes::{self, Facts, Plan};
use crate::sys;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LaunchRecord {
    pub app_id: String,
    pub started_at: String,
    pub runtime_secs: f64,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub failed: bool,
    pub passthrough: bool,
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
    pub prep_error: Option<String>,
    /// Written when the game starts and cleared when it closes. Still true after hours means the
    /// game (or CachyPlay with it) was killed, e.g. after a GPU hang, without closing normally.
    #[serde(default)]
    pub running: bool,
    /// The graphics chip the game drew on, e.g. "the NVIDIA graphics card". Only checked on PCs
    /// with two chips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    /// True if that was the weaker chip built into the processor or motherboard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_integrated: Option<bool>,
}

/// A launch still marked running after this long didn't close normally.
pub const STALE_RUNNING_SECS: i64 = 6 * 3600;

impl LaunchRecord {
    /// Turn a launch that never recorded its end into a failed one once it's clearly over.
    pub fn settle(mut self, now: chrono::DateTime<chrono::Local>) -> Self {
        if self.running {
            let age = chrono::DateTime::parse_from_rfc3339(&self.started_at)
                .map(|t| (now - t.with_timezone(&chrono::Local)).num_seconds())
                .unwrap_or(i64::MAX);
            if age > STALE_RUNNING_SECS {
                self.running = false;
                self.failed = true;
            }
        }
        self
    }
}

pub fn app_id_from_env() -> Option<String> {
    ["SteamAppId", "SteamGameId", "STEAM_COMPAT_APP_ID"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty() && v != "0" && v.chars().all(|c| c.is_ascii_digit()))
}

/// Resolve what to change for this launch. Any error here means "launch unmodified".
fn prepare(paths: &Paths, app_id: Option<&str>) -> anyhow::Result<Plan> {
    let Some(app_id) = app_id else {
        return Ok(Plan::default());
    };
    if std::env::var_os("CACHYPLAY_TEST_PANIC").is_some() {
        panic!("test panic in prepare");
    }
    let mut plan = match recipes::find(app_id, &paths.config, &paths.recipes())? {
        Some(r) => recipes::plan(&r, &Facts::detect()),
        None => Plan::default(),
    };
    // Fixes you applied from a suggestion win over the fix file.
    for fix in crate::fixes::load(paths, app_id).fixes {
        let yours = fix.id == crate::fixes::YOUR_SETTINGS;
        for (k, v) in &fix.env {
            if !(crate::fixes::fix_env_allowed(k) || yours && crate::fixes::your_setting_allowed(k))
            {
                continue;
            }
            plan.env.retain(|(name, _, _)| name != k);
            plan.env.push((k.clone(), v.clone(), fix.label.clone()));
        }
        plan.extra_args.extend(fix.args.iter().cloned());
        plan.applied.push(format!("Fix: {}", fix.label));
    }
    // The detailed Proton log goes to a folder CachyPlay knows, whichever Proton writes it.
    if plan.env.iter().any(|(k, _, _)| k == "PROTON_LOG")
        && std::env::var_os("PROTON_LOG_DIR").is_none()
    {
        let dir = paths.state.join("proton-logs");
        if std::fs::create_dir_all(&dir).is_ok() {
            plan.env.push((
                "PROTON_LOG_DIR".into(),
                dir.to_string_lossy().into_owned(),
                "Detailed Proton log".into(),
            ));
        }
    }
    // "Record performance": MangoHud logs FPS and frame times without showing its overlay.
    if plan
        .env
        .iter()
        .any(|(k, v, _)| k == "MANGOHUD_CONFIG" && v.contains("autostart_log"))
    {
        let dir = paths.state.join("performance");
        let _ = std::fs::create_dir_all(&dir);
        for e in plan
            .env
            .iter_mut()
            .filter(|(k, _, _)| k == "MANGOHUD_CONFIG")
        {
            e.1 = format!("{},output_folder={}", e.1, dir.display());
        }
    }
    // Two graphics chips: make sure the game draws on the card, not the built-in chip. Settings
    // you set yourself (in launch options or a fix) win.
    if crate::gpus::prefer_card_for(paths, app_id) {
        let gpus = crate::gpus::list(&crate::pccheck::Root::system());
        let env = crate::gpus::prefer_card_env(&gpus);
        let mut added = false;
        for (k, v) in env {
            if std::env::var_os(&k).is_some() || plan.env.iter().any(|(n, _, _)| *n == k) {
                continue;
            }
            plan.env.push((k, v, "Use the graphics card".into()));
            added = true;
        }
        if added {
            plan.applied.push("Use the graphics card".into());
        }
    }
    if crate::obs::enabled_for(paths, app_id) {
        if std::env::var("OBS_VKCAPTURE").is_ok_and(|v| v == "1") {
            // Already started through obs-gamecapture (e.g. in your own launch options).
        } else if crate::obs::installed() {
            plan.obs_capture = true;
            plan.applied.push("OBS game capture".into());
        } else {
            plan.skipped
                .push("OBS game capture (obs-gamecapture isn't installed)".into());
        }
    }
    Ok(plan)
}

/// Run the game. Returns the exit code to hand back to Steam.
pub fn run(paths: &Paths, command: Vec<OsString>) -> i32 {
    if command.is_empty() {
        eprintln!("cachyplay run: no command given (expected `cachyplay run -- %command%`)");
        return 2;
    }
    let app_id = app_id_from_env();
    let started_at = now();
    // Panics are caught below; send their message to our log instead of Steam's console.
    let log_paths = paths.clone();
    std::panic::set_hook(Box::new(move |info| {
        log(&log_paths, &format!("panic: {info}"))
    }));

    let (plan, prep_error) =
        match catch_unwind(AssertUnwindSafe(|| prepare(paths, app_id.as_deref()))) {
            Ok(Ok(p)) => (p, None),
            Ok(Err(e)) => (Plan::default(), Some(format!("{e:#}"))),
            Err(_) => (
                Plan::default(),
                Some("internal panic while preparing the launch".into()),
            ),
        };
    if let Some(e) = &prep_error {
        log(
            paths,
            &format!("app {app_id:?}: preparation failed, launching unmodified: {e}"),
        );
    }

    let mut passthrough = prep_error.is_some() || plan_is_empty(&plan);
    // Note the launch right away, so a game that hangs (and never returns here) still shows up.
    let record_path = app_id.as_ref().map(|id| {
        let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        paths.launches().join(format!("{id}-{stamp}.json"))
    });
    if let (Some(id), Some(path)) = (&app_id, &record_path) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let rec = LaunchRecord {
                app_id: id.clone(),
                started_at: started_at.clone(),
                passthrough,
                applied: if passthrough {
                    vec![]
                } else {
                    plan.applied.clone()
                },
                skipped: plan.skipped.clone(),
                prep_error: prep_error.clone(),
                running: true,
                ..Default::default()
            };
            let _ = write_json(path, &rec);
        }));
    }

    if let Some(id) = &app_id {
        log(
            paths,
            &format!(
                "app {id}: start{}",
                if passthrough {
                    " (unchanged)".to_string()
                } else {
                    format!(" with: {}", plan.applied.join("; "))
                }
            ),
        );
    }
    let start = Instant::now();
    let chip: std::sync::Arc<std::sync::Mutex<Option<crate::gpus::Gpu>>> = Default::default();
    let child = match spawn_retry(&mut build(&command, &plan, passthrough)) {
        Ok(c) => Ok(c),
        Err(e) if !passthrough => {
            log(
                paths,
                &format!(
                    "app {app_id:?}: modified launch failed to start ({e}); retrying unmodified"
                ),
            );
            passthrough = true;
            spawn_retry(&mut build(&command, &plan, true))
        }
        Err(e) => Err(e),
    };
    let mut gpu_log = None;
    if let (Ok(c), Some(id)) = (&child, &app_id) {
        watch_chip(c.id(), chip.clone());
        gpu_log = crate::gpulog::start(paths, id, &started_at);
        // Diagnostic-only fixes (detailed Proton log, performance recording) are for one start:
        // they're in this start's settings already, so take them off now. Waiting for the end
        // meant they stayed on whenever Steam stopped the game before CachyPlay could clean up.
        if !passthrough {
            for f in crate::fixes::load(paths, id)
                .fixes
                .iter()
                .filter(|f| f.temporary)
            {
                let _ = crate::fixes::remove(paths, id, &f.id);
                log(
                    paths,
                    &format!("app {id}: \"{}\" is used for this start only", f.label),
                );
            }
        }
    }
    let status = child.and_then(|mut c| c.wait());
    if let Some(g) = gpu_log {
        g.stop();
    }
    let runtime = start.elapsed().as_secs_f64();
    if let Some(id) = &app_id {
        log(
            paths,
            &format!(
                "app {id}: ended after {runtime:.0}s ({})",
                match &status {
                    Ok(s) => format!("{s}"),
                    Err(e) => format!("couldn't start: {e}"),
                }
            ),
        );
    }

    let (code, signal) = match &status {
        Ok(s) => {
            use std::os::unix::process::ExitStatusExt;
            (s.code(), s.signal())
        }
        Err(e) => {
            eprintln!("cachyplay: could not start the game: {e}");
            (Some(127), None)
        }
    };

    // Recording is best-effort and must never change the exit code.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let min = if plan.min_runtime_secs == 0 {
            30
        } else {
            plan.min_runtime_secs
        };
        let failed = code.is_some_and(|c| c != 0) || runtime < min as f64;
        if let Some(id) = &app_id {
            let rec = LaunchRecord {
                app_id: id.clone(),
                started_at: started_at.clone(),
                runtime_secs: (runtime * 10.0).round() / 10.0,
                exit_code: code,
                signal,
                failed,
                passthrough,
                applied: if passthrough {
                    vec![]
                } else {
                    plan.applied.clone()
                },
                skipped: plan.skipped.clone(),
                prep_error: prep_error.clone(),
                running: false,
                gpu: chip
                    .lock()
                    .ok()
                    .and_then(|g| g.as_ref().map(|g| g.describe())),
                gpu_integrated: chip
                    .lock()
                    .ok()
                    .and_then(|g| g.as_ref().map(|g| g.integrated)),
            };
            if let Some(path) = &record_path {
                let _ = write_json(path, &rec);
            }
            crate::housekeeping::trim(paths);
            if failed {
                eprintln!(
                    "cachyplay: the game closed after {:.0}s (exit {:?}). To get help, run: cachyplay report {id}",
                    runtime, code
                );
                notify_failure(paths, id, runtime);
            }
        }
    }));

    match (code, signal) {
        (Some(c), _) => c,
        (None, Some(s)) => 128 + s,
        _ => 1,
    }
}

/// Start the game. If Linux says the program file is still open for writing ("Text file busy",
/// for example right after an update or install wrote it), wait a moment and try again.
fn spawn_retry(c: &mut Command) -> std::io::Result<std::process::Child> {
    for _ in 0..25 {
        match c.spawn() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            other => return other,
        }
    }
    c.spawn()
}

/// On a PC with two graphics chips, look (in the background) at which one the game opened.
/// Checked after 20 seconds, then every 40 seconds for a few minutes, since games open the chip
/// only once their window appears. Never affects the game.
fn watch_chip(pid: u32, out: std::sync::Arc<std::sync::Mutex<Option<crate::gpus::Gpu>>>) {
    let _ = std::thread::Builder::new()
        .name("chip".into())
        .spawn(move || {
            let root = crate::pccheck::Root::system();
            let gpus = crate::gpus::list(&root);
            if !crate::gpus::has_both(&gpus) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(20));
            for _ in 0..8 {
                if let Some(g) = crate::gpus::chip_in_use(&root, pid, &gpus) {
                    let card = !g.integrated;
                    if let Ok(mut slot) = out.lock() {
                        *slot = Some(g);
                    }
                    if card {
                        return;
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(40));
            }
        });
}

/// Desktop notification so a failed launch doesn't go unnoticed. Best-effort and non-blocking.
fn notify_failure(paths: &Paths, app_id: &str, runtime: f64) {
    if std::env::var_os("CACHYPLAY_NO_NOTIFY").is_some() {
        return;
    }
    let Some(notify) = sys::which("notify-send") else {
        return;
    };
    let name = crate::steam::Steam::locate(&paths.home)
        .ok()
        .and_then(|s| s.find_game(app_id))
        .map(|g| g.name)
        .unwrap_or_else(|| format!("Game {app_id}"));
    let _ = Command::new(notify)
        .args([
            "--app-name=CachyPlay",
            "--icon=cachyplay",
            &format!("{name} closed after {runtime:.0} seconds"),
            "Open CachyPlay and save a bug report to see what went wrong.",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn plan_is_empty(p: &Plan) -> bool {
    p.env.is_empty() && p.extra_args.is_empty() && !p.game_performance && !p.obs_capture
}

fn build(command: &[OsString], plan: &Plan, passthrough: bool) -> Command {
    if passthrough {
        let mut c = Command::new(&command[0]);
        c.args(&command[1..]);
        return c;
    }
    let mut argv: Vec<OsString> = Vec::new();
    if plan.obs_capture {
        if let Some(obs) = sys::which(crate::obs::TOOL) {
            argv.push(obs.into_os_string());
        }
    }
    if plan.game_performance {
        if let Some(gp) = sys::which("game-performance") {
            argv.push(gp.into_os_string());
        }
    }
    argv.extend(command.iter().cloned());
    argv.extend(plan.extra_args.iter().map(OsString::from));
    let mut c = Command::new(&argv[0]);
    c.args(&argv[1..]);
    for (k, v, _) in &plan.env {
        c.env(k, v);
    }
    c
}

pub fn log(paths: &Paths, msg: &str) {
    crate::logging::write(paths, msg);
}

/// The most recent launch records for a game, newest first.
pub fn recent_launches(paths: &Paths, app_id: &str, max: usize) -> Vec<LaunchRecord> {
    let mut files: Vec<_> = std::fs::read_dir(paths.launches())
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p: &std::path::PathBuf| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(&format!("{app_id}-")))
    });
    files.sort();
    files
        .iter()
        .rev()
        .take(max)
        .filter_map(|p| crate::manage::read_json::<LaunchRecord>(p))
        .map(|r| r.settle(chrono::Local::now()))
        .collect()
}
