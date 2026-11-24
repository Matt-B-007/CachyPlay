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
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    match recipes::find(app_id, &paths.config, &paths.recipes())? {
        Some(r) => Ok(recipes::plan(&r, &Facts::detect())),
        None => Ok(Plan::default()),
    }
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

    let start = Instant::now();
    let mut passthrough = prep_error.is_some() || plan_is_empty(&plan);
    let status = match build(&command, &plan, passthrough).status() {
        Ok(s) => Ok(s),
        Err(e) if !passthrough => {
            log(
                paths,
                &format!(
                    "app {app_id:?}: modified launch failed to start ({e}); retrying unmodified"
                ),
            );
            passthrough = true;
            build(&command, &plan, true).status()
        }
        Err(e) => Err(e),
    };
    let runtime = start.elapsed().as_secs_f64();

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
            };
            let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
            let _ = write_json(&paths.launches().join(format!("{id}-{stamp}.json")), &rec);
            if failed {
                eprintln!(
                    "cachyplay: the game closed after {:.0}s (exit {:?}). To get help, run: cachyplay report {id}",
                    runtime, code
                );
            }
        }
    }));

    match (code, signal) {
        (Some(c), _) => c,
        (None, Some(s)) => 128 + s,
        _ => 1,
    }
}

fn plan_is_empty(p: &Plan) -> bool {
    p.env.is_empty() && p.extra_args.is_empty() && !p.game_performance
}

fn build(command: &[OsString], plan: &Plan, passthrough: bool) -> Command {
    if passthrough {
        let mut c = Command::new(&command[0]);
        c.args(&command[1..]);
        return c;
    }
    let mut argv: Vec<OsString> = Vec::new();
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
    let _ = std::fs::create_dir_all(&paths.state);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.log_file())
    {
        let _ = writeln!(f, "{} {msg}", now());
    }
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
        .filter_map(|p| crate::manage::read_json(p))
        .collect()
}
