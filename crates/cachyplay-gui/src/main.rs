//! CachyPlay desktop app. A thin Tauri shell: every command calls into `cachyplay-core`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cachyplay_core::library::{self, GameDetail, Overview};
use cachyplay_core::manage::{touched_accounts, Ctx, Outcome};
use cachyplay_core::paths::Paths;
use cachyplay_core::recipes::Facts;
use cachyplay_core::{install, report, update};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

type CmdResult<T> = Result<T, String>;

fn paths() -> CmdResult<Paths> {
    Paths::from_env().map_err(|e| format!("{e:#}"))
}

fn ctx(paths: &Paths) -> CmdResult<Ctx> {
    let mut c = Ctx::new(paths.clone()).map_err(|e| format!("{e:#}"))?;
    if let Some(p) = std::env::var_os("CACHYPLAY_PROC_ROOT") {
        c.proc_root = PathBuf::from(p); // tests only
    }
    Ok(c)
}

fn facts() -> &'static Facts {
    static FACTS: OnceLock<Facts> = OnceLock::new();
    FACTS.get_or_init(Facts::detect)
}

#[tauri::command(async)]
fn overview() -> CmdResult<Overview> {
    let p = paths()?;
    let c = ctx(&p);
    Ok(library::overview(&p, c.as_ref().map_err(Clone::clone)))
}

#[tauri::command(async)]
fn game_detail(app_id: String) -> CmdResult<GameDetail> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = c
        .steam
        .find_game(&app_id)
        .ok_or_else(|| format!("Game {app_id} is not installed"))?;
    Ok(library::detail(&c, &g, facts()))
}

/// OBS game capture for one game. Turning it on also turns CachyPlay on for the game, since
/// CachyPlay's launch helper is what starts the game through obs-gamecapture.
#[tauri::command(async)]
fn set_obs_capture(app_id: String, on: bool) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = c
        .steam
        .find_game(&app_id)
        .ok_or_else(|| format!("Game {app_id} is not installed"))?;
    let mut turned_on = false;
    if on && !c.status(&g).is_ok_and(|s| s.applied) {
        let bin = library::installed_cli(&p)
            .ok_or("CachyPlay isn't installed yet. Use the Install button at the top first.")?;
        c.apply(&g, &bin).map_err(|e| format!("{e:#}"))?;
        turned_on = true;
    }
    cachyplay_core::obs::set_game(&p, &app_id, on).map_err(|e| format!("{e:#}"))?;
    let mut msg = if on {
        format!("OBS game capture is on for {}.", g.name)
    } else if cachyplay_core::obs::load(&p).all {
        format!(
            "Turned off for {} only, but it's still on for every game in Settings.",
            g.name
        )
    } else {
        format!("OBS game capture is off for {}.", g.name)
    };
    if turned_on {
        msg = format!("Turned CachyPlay on for {}. {msg}", g.name);
    }
    if on && !cachyplay_core::obs::installed() {
        msg.push_str(" Install obs-vkcapture first (command in the OBS box).");
    }
    Ok(msg)
}

/// "What happened?" for one start, and/or whether it was online.
#[tauri::command(async)]
fn set_run_note(
    app_id: String,
    start: String,
    kind: String,
    online: Option<bool>,
) -> CmdResult<()> {
    let p = paths()?;
    cachyplay_core::notes::set(&p, &app_id, &start, &kind, online).map_err(|e| format!("{e:#}"))?;
    Ok(())
}

#[tauri::command(async)]
fn set_online_only(app_id: String, on: bool) -> CmdResult<()> {
    let p = paths()?;
    cachyplay_core::notes::set_online_only(&p, &app_id, on).map_err(|e| format!("{e:#}"))
}

/// "Use the graphics card" for one game (PCs with two graphics chips).
#[tauri::command(async)]
fn set_prefer_card(app_id: String, on: bool) -> CmdResult<String> {
    let p = paths()?;
    cachyplay_core::gpus::set_prefer_card(&p, &app_id, on).map_err(|e| format!("{e:#}"))?;
    Ok(if on {
        "The game will use the graphics card the next time it starts.".into()
    } else {
        "The game picks its graphics chip by itself from now on.".into()
    })
}

#[tauri::command(async)]
fn set_obs_all(on: bool) -> CmdResult<String> {
    let p = paths()?;
    cachyplay_core::obs::set_all(&p, on).map_err(|e| format!("{e:#}"))?;
    Ok(if on {
        "OBS game capture is on for every game CachyPlay is on for.".into()
    } else {
        "OBS game capture for every game is off. Games you turned it on for one by one keep it."
            .into()
    })
}

#[tauri::command(async)]
fn obs_settings() -> CmdResult<serde_json::Value> {
    let p = paths()?;
    let s = cachyplay_core::obs::load(&p);
    Ok(serde_json::json!({
        "all": s.all,
        "installed": cachyplay_core::obs::installed(),
        "install_command": cachyplay_core::obs::INSTALL_COMMAND,
    }))
}

/// Force a game to use another compatibility tool (Proton version). Needs Steam closed.
#[tauri::command(async)]
fn switch_compat_tool(app_id: String, name: String) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = c
        .steam
        .find_game(&app_id)
        .ok_or_else(|| format!("Game {app_id} is not installed"))?;
    let tools = cachyplay_core::compat::installed(&c.steam.root, &c.steam.library_folders());
    let t = tools
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| format!("{name} isn't installed any more."))?;
    cachyplay_core::compat::switch(&c, &app_id, t).map_err(|e| format!("{e:#}"))?;
    Ok(format!(
        "{} now uses {}. Start it from Steam to try it.",
        g.name, t.display
    ))
}

#[tauri::command(async)]
fn undo_compat_tool(app_id: String) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    cachyplay_core::compat::undo(&c, &app_id).map_err(|e| format!("{e:#}"))?;
    Ok("Put the game back on the compatibility tool it used before.".into())
}

/// Everything the app polls every few seconds: Steam's state, a fingerprint of the data the
/// pages are built from (so the app refreshes when a game starts, ends or crashes), and the
/// changes waiting for Steam to close.
#[derive(serde::Serialize, Clone, PartialEq)]
struct Live {
    steam: cachyplay_core::live::SteamState,
    token: String,
    waiting: Vec<cachyplay_core::live::Action>,
    /// Results of waiting changes that just ran because Steam closed (shown as messages).
    #[serde(default)]
    ran: Vec<String>,
}

#[tauri::command(async)]
fn live_status() -> CmdResult<Live> {
    let p = paths()?;
    let c = ctx(&p)?;
    Ok(Live {
        steam: cachyplay_core::live::steam_state(&c.proc_root),
        token: cachyplay_core::live::token(&c),
        waiting: cachyplay_core::live::queue(&p),
        ran: vec![],
    })
}

/// Save a change to run once Steam closes.
#[tauri::command(async)]
fn queue_action(action: cachyplay_core::live::Action) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let name = c
        .steam
        .find_game(action.app_id())
        .map(|g| g.name)
        .unwrap_or_else(|| action.app_id().to_string());
    let what = action.describe(&name);
    cachyplay_core::live::enqueue(&p, action).map_err(|e| format!("{e:#}"))?;
    Ok(format!(
        "{what}: waiting for Steam to close. It happens by itself the moment Steam exits."
    ))
}

#[tauri::command(async)]
fn cancel_waiting(app_id: String) -> CmdResult<()> {
    let p = paths()?;
    cachyplay_core::live::cancel(&p, &app_id).map_err(|e| format!("{e:#}"))
}

#[tauri::command(async)]
fn run_waiting() -> CmdResult<Vec<String>> {
    let p = paths()?;
    let c = ctx(&p)?;
    let bin = library::installed_cli(&p);
    cachyplay_core::live::run_queue(&c, bin.as_deref()).map_err(|e| format!("{e:#}"))
}

/// Ask Steam to quit properly (the same as Steam → Exit).
#[tauri::command(async)]
fn close_steam() -> CmdResult<String> {
    std::process::Command::new("steam")
        .arg("-shutdown")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't ask Steam to close ({e}). Use Steam → Exit instead."))?;
    Ok("Asked Steam to close. Waiting changes run as soon as it has.".into())
}

#[tauri::command(async)]
fn open_steam() -> CmdResult<String> {
    let started = std::process::Command::new("setsid")
        .args(["-f", "steam"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .or_else(|_| {
            std::process::Command::new("steam")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        });
    started.map_err(|e| format!("Couldn't start Steam ({e})."))?;
    Ok("Starting Steam.".into())
}

#[tauri::command(async)]
fn reset_shader_cache(app_id: String) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = game_of(&c, &app_id)?;
    cachyplay_core::shadercache::reset(&c, &g).map_err(|e| format!("{e:#}"))?;
    let _ = cachyplay_core::fixes::apply(
        &p,
        &app_id,
        cachyplay_core::fixes::Fix {
            id: "shader_cache".into(),
            label: "Fresh shader cache".into(),
            ..Default::default()
        },
    );
    Ok(format!("Moved {}'s shader cache aside. Steam rebuilds it on the next start (expect a little stutter at first). Undo puts the old one back.", g.name))
}

#[tauri::command(async)]
fn undo_shader_cache(app_id: String) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = game_of(&c, &app_id)?;
    cachyplay_core::shadercache::undo(&c, &g).map_err(|e| format!("{e:#}"))?;
    let _ = cachyplay_core::fixes::remove(&p, &app_id, "shader_cache");
    Ok(format!("Put {}'s old shader cache back.", g.name))
}

/// "Try these settings" for one game.
#[tauri::command(async)]
fn set_your_settings(app_id: String, text: String) -> CmdResult<String> {
    let p = paths()?;
    Ok(match cachyplay_core::fixes::set_your_settings(&p, &app_id, &text).map_err(|e| format!("{e:#}"))? {
        Some(_) => "Saved. They're used the next time the game starts (CachyPlay must be on for the game).".into(),
        None => "Your settings for this game were removed.".into(),
    })
}

#[tauri::command(async)]
fn set_enabled(app_id: String, enabled: bool) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = c
        .steam
        .find_game(&app_id)
        .ok_or_else(|| format!("Game {app_id} is not installed"))?;
    let _ = cachyplay_core::live::cancel(&p, &app_id);
    if enabled {
        let bin = library::installed_cli(&p)
            .ok_or("CachyPlay isn't installed yet. Use the Install button at the top first.")?;
        match c.apply(&g, &bin).map_err(|e| format!("{e:#}"))? {
            Outcome::AlreadyApplied => Ok(format!("CachyPlay was already on for {}.", g.name)),
            _ => Ok(format!(
                "CachyPlay is on for {}. Start it from Steam as usual.",
                g.name
            )),
        }
    } else {
        Ok(match c.undo(&app_id).map_err(|e| format!("{e:#}"))? {
            Outcome::Restored => format!(
                "CachyPlay is off for {}. Original launch options restored.",
                g.name
            ),
            Outcome::RemovedKeptUserEdits => format!(
                "CachyPlay is off for {}. Your own edits in Steam were kept.",
                g.name
            ),
            _ => format!("CachyPlay wasn't on for {}.", g.name),
        })
    }
}

#[derive(Serialize)]
struct ReportResult {
    path: String,
    folder: String,
    name: String,
    files: Vec<String>,
}

#[tauri::command]
async fn create_report(app_id: Option<String>) -> CmdResult<ReportResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = paths()?;
        let c = ctx(&p).ok();
        let g = match (&app_id, &c) {
            (Some(id), Some(c)) => c.steam.find_game(id),
            _ => None,
        };
        let r = report::create(&p, c.as_ref(), g.as_ref()).map_err(|e| format!("{e:#}"))?;
        Ok(ReportResult {
            folder: r
                .path
                .parent()
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
            name: r
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: r.path.display().to_string(),
            files: r.files,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Open a folder in the file manager.
#[tauri::command(async)]
fn open_folder(path: String) -> CmdResult<()> {
    let p = PathBuf::from(&path);
    let dir = if p.is_dir() {
        p
    } else {
        p.parent().map(PathBuf::from).ok_or("no folder")?
    };
    std::process::Command::new("xdg-open")
        .arg(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't open the folder: {e}"))
}

fn restore_everything(p: &Paths) -> CmdResult<Vec<String>> {
    restore_everything_with(p, false)
}

/// Restore every managed game; with `keep`, games with settings get them written into Steam.
fn restore_everything_with(p: &Paths, keep: bool) -> CmdResult<Vec<String>> {
    let accounts = touched_accounts(p);
    if accounts.is_empty() && !has_compat_switches(p) {
        return Ok(vec![]); // nothing was ever changed; Steam isn't needed
    }
    let mut base = Ctx::for_restore(p.clone()).map_err(|e| format!("{e:#}"))?;
    if let Some(pr) = std::env::var_os("CACHYPLAY_PROC_ROOT") {
        base.proc_root = PathBuf::from(pr); // tests only
    }
    let mut lines = Vec::new();
    for account in accounts {
        let c = base.for_account(&account);
        let kept: std::collections::BTreeMap<String, String> = if keep {
            cachyplay_core::keep::all(&c)
                .into_iter()
                .map(|g| (g.app_id, g.launch_options))
                .collect()
        } else {
            Default::default()
        };
        for app in c.managed_apps() {
            let name = c
                .steam
                .find_game(&app)
                .map(|g| g.name)
                .unwrap_or_else(|| app.clone());
            if let Some(lo) = kept.get(&app) {
                c.keep_settings(&app, lo)
                    .map_err(|e| format!("{name}: {e:#}"))?;
                lines.push(format!("{name}: settings kept in Steam"));
                continue;
            }
            match c.undo(&app) {
                Ok(Outcome::RemovedKeptUserEdits) => {
                    lines.push(format!("{name}: restored (your edits kept)"))
                }
                Ok(_) => lines.push(format!("{name}: restored")),
                Err(e) => return Err(format!("{name}: {e:#}")),
            }
        }
    }
    lines.extend(cachyplay_core::compat::revert_all(&base, keep));
    Ok(lines)
}

fn has_compat_switches(p: &Paths) -> bool {
    std::fs::read_dir(p.data.join("compat-tool")).is_ok_and(|mut d| d.next().is_some())
}

#[tauri::command(async)]
fn restore_all() -> CmdResult<Vec<String>> {
    restore_everything(&paths()?)
}

#[tauri::command(async)]
fn game_settings_all() -> CmdResult<Vec<cachyplay_core::keep::GameSettings>> {
    let p = paths()?;
    Ok(cachyplay_core::keep::all(&ctx(&p)?))
}

#[tauri::command(async)]
fn export_settings() -> CmdResult<String> {
    let p = paths()?;
    let games = cachyplay_core::keep::all(&ctx(&p)?);
    let file = p.report_dir().join("CachyPlay game settings.txt");
    cachyplay_core::fsutil::atomic_write(
        &file,
        cachyplay_core::keep::export_text(&games).as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    Ok(file.display().to_string())
}

#[tauri::command(async)]
fn uninstall(remove_backups: bool, keep_settings: bool) -> CmdResult<String> {
    let p = paths()?;
    let mut saved = None;
    if keep_settings {
        if let Ok(c) = Ctx::for_restore(p.clone()) {
            let games = cachyplay_core::keep::all(&c);
            if !games.is_empty() {
                let file = p.report_dir().join("CachyPlay game settings.txt");
                cachyplay_core::fsutil::atomic_write(
                    &file,
                    cachyplay_core::keep::export_text(&games).as_bytes(),
                )
                .map_err(|e| e.to_string())?;
                saved = Some(file);
            }
        }
    }
    let kept_note = Ctx::for_restore(p.clone())
        .ok()
        .and_then(|c| cachyplay_core::compat::kept_copies_note(&c));
    let restored = restore_everything_with(&p, keep_settings)?;
    install::remove_user_files(&p, remove_backups).map_err(|e| format!("{e:#}"))?;
    let exe = std::env::current_exe().unwrap_or_default();
    let kept = restored
        .iter()
        .filter(|l| l.ends_with("settings kept in Steam"))
        .count();
    let back = restored
        .iter()
        .filter(|l| l.ends_with(": restored") || l.ends_with("(your edits kept)"))
        .count();
    let tools: Vec<&String> = restored
        .iter()
        .filter(|l| l.contains("compatibility tool") || l.contains(": kept on "))
        .collect();
    let count = |n: usize| {
        if n == 1 {
            "1 game".to_string()
        } else {
            format!("{n} games")
        }
    };
    let games = match (kept, back) {
        (0, 0) => "No games needed changes".to_string(),
        (0, b) => format!("Restored {}", count(b)),
        (k, 0) => format!("Kept the working settings of {} in Steam", count(k)),
        (k, b) => format!(
            "Kept the working settings of {} in Steam, restored {}",
            count(k),
            count(b)
        ),
    };
    let mut msg = format!("{games}, and removed CachyPlay's files.");
    for t in tools {
        msg.push_str(&format!(" {t}."));
    }
    if let Some(n) = kept_note {
        msg.push_str(&format!(" {n}"));
    }
    if let Some(f) = saved {
        msg.push_str(&format!(
            " A copy of your game settings is at {}.",
            f.display()
        ));
    }
    if exe.starts_with("/usr") {
        msg.push_str(" Finish by removing the package in a terminal: sudo pacman -Rns cachyplay");
    } else {
        msg.push_str(" You can close this window.");
    }
    Ok(msg)
}

/// Install a downloaded copy (this GUI + the `cachyplay` command next to it) into ~/.local/bin.
#[tauri::command(async)]
fn install_self() -> CmdResult<String> {
    let p = paths()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let cli = exe.with_file_name("cachyplay");
    if !cli.is_file() {
        return Err(
            "The `cachyplay` program must be in the same folder as this app to install it.".into(),
        );
    }
    let bin = install::install_user(&p, &cli).map_err(|e| format!("{e:#}"))?;
    Ok(format!(
        "Installed to {}. You can delete the downloaded folder and open CachyPlay from your app menu.",
        bin.parent().map(|d| d.display().to_string()).unwrap_or_default()
    ))
}

/* ---------- suggested fixes ---------- */

#[tauri::command]
async fn game_suggestions(app_id: String) -> CmdResult<Vec<cachyplay_core::advice::Suggestion>> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = paths()?;
        let c = ctx(&p)?;
        let g = c
            .steam
            .find_game(&app_id)
            .ok_or_else(|| format!("Game {app_id} is not installed"))?;
        Ok(cachyplay_core::advice::for_game(&c, &g))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn apply_fix(app_id: String, id: String) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = paths()?;
        let c = ctx(&p)?;
        let g = c
            .steam
            .find_game(&app_id)
            .ok_or_else(|| format!("Game {app_id} is not installed"))?;
        let bin = library::installed_cli(&p);
        cachyplay_core::advice::apply_suggestion(&c, &g, &id, bin.as_deref())
            .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command(async)]
fn undo_fix(app_id: String, id: String) -> CmdResult<String> {
    let p = paths()?;
    match cachyplay_core::fixes::remove(&p, &app_id, &id).map_err(|e| format!("{e:#}"))? {
        true => Ok("Fix undone. The game starts without it next time.".into()),
        false => Ok("That fix wasn't applied.".into()),
    }
}

/// Undo one applied change, saying why: "worse" or "didnt_help" rule it out for this game,
/// "testing" doesn't.
#[tauri::command(async)]
fn undo_change(app_id: String, id: String, reason: String) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let reason = Some(reason.as_str()).filter(|r| ["worse", "didnt_help", "testing"].contains(r));
    let _ = cachyplay_core::tries::record_off(&p, &app_id, &id, reason);
    let msg = if id.starts_with("proton:") {
        if cachyplay_core::live::steam_state(&c.proc_root)
            != cachyplay_core::live::SteamState::Closed
        {
            let g = game_of(&c, &app_id)?;
            let what = cachyplay_core::live::Action::UndoTool {
                app_id: app_id.clone(),
            };
            cachyplay_core::live::enqueue(&p, what).map_err(|e| format!("{e:#}"))?;
            return Ok(format!(
                "{}'s Proton version goes back when Steam closes.",
                g.name
            ));
        }
        cachyplay_core::compat::undo(&c, &app_id).map_err(|e| format!("{e:#}"))?;
        "Proton version put back.".to_string()
    } else if id == "shader_cache" {
        let g = game_of(&c, &app_id)?;
        if cachyplay_core::shadercache::has_aside(&g) {
            cachyplay_core::shadercache::undo(&c, &g).map_err(|e| format!("{e:#}"))?;
        }
        cachyplay_core::fixes::remove(&p, &app_id, &id).map_err(|e| format!("{e:#}"))?;
        "Old shader cache put back.".to_string()
    } else {
        cachyplay_core::fixes::remove(&p, &app_id, &id).map_err(|e| format!("{e:#}"))?;
        "Undone. The game starts without it next time.".to_string()
    };
    Ok(match reason {
        Some("worse") | Some("didnt_help") => format!("{msg} It's crossed off for this game."),
        _ => msg,
    })
}

/// "Try again": offer a ruled-out change once more.
#[tauri::command(async)]
fn try_again(app_id: String, id: String) -> CmdResult<()> {
    let p = paths()?;
    cachyplay_core::tries::try_again(&p, &app_id, &id).map_err(|e| format!("{e:#}"))
}

/* ---------- game page: summary, fixes, history, help ---------- */

fn game_of(c: &Ctx, app_id: &str) -> CmdResult<cachyplay_core::steam::Game> {
    c.steam
        .find_game(app_id)
        .ok_or_else(|| format!("Game {app_id} is not installed"))
}

#[tauri::command]
async fn game_insights(app_id: String) -> CmdResult<cachyplay_core::advice::Insights> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = paths()?;
        let c = ctx(&p)?;
        let g = game_of(&c, &app_id)?;
        Ok(cachyplay_core::advice::insights(&c, &g))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// "Fix it": apply the suggestion the summary picked (a fix or a Proton switch).
#[tauri::command]
async fn fix_it(app_id: String) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = paths()?;
        let c = ctx(&p)?;
        let g = game_of(&c, &app_id)?;
        let ins = cachyplay_core::advice::insights(&c, &g);
        let Some(id) = ins.summary.fix_id.clone() else {
            return Err("There's nothing CachyPlay can change with one click for this game right now.".into());
        };
        let s = ins.suggestions.iter().find(|s| s.id == id).ok_or("That suggestion is gone; refresh and try again.")?;
        if let Some(tool) = &s.switch_tool {
            cachyplay_core::compat::switch(&c, &app_id, tool).map_err(|e| format!("{e:#}"))?;
            return Ok(format!("Switched {} to {}. Start it from Steam to try it; CachyPlay will ask if it worked.", g.name, tool.display));
        }
        let bin = library::installed_cli(&p);
        cachyplay_core::advice::apply_suggestion(&c, &g, &id, bin.as_deref())
            .map(|m| format!("{m} CachyPlay will ask if it worked after you play."))
            .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The answer to "Did it work?".
#[tauri::command(async)]
fn set_verdict(app_id: String, id: String, worked: bool) -> CmdResult<String> {
    let p = paths()?;
    cachyplay_core::fixes::set_verdict(&p, &app_id, &id, worked).map_err(|e| format!("{e:#}"))?;
    Ok(if worked {
        "Great, noted. This fix stays on.".into()
    } else {
        "Noted. Try the next suggestion, or undo this one.".into()
    })
}

#[tauri::command(async)]
fn pc_check() -> CmdResult<Vec<cachyplay_core::pccheck::Check>> {
    let p = paths()?;
    let libraries = ctx(&p)
        .map(|c| c.steam.library_folders())
        .unwrap_or_default();
    Ok(cachyplay_core::pccheck::pc_checks(
        &cachyplay_core::pccheck::Root::system(),
        &libraries,
    ))
}

#[tauri::command(async)]
fn self_test() -> CmdResult<Vec<cachyplay_core::pccheck::Check>> {
    Ok(cachyplay_core::pccheck::self_test(&paths()?))
}

#[tauri::command(async)]
fn reset_prefix(app_id: String) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = game_of(&c, &app_id)?;
    cachyplay_core::prefix::reset(&c, &g).map_err(|e| format!("{e:#}"))?;
    Ok(format!("{}'s Windows files were set aside. Steam makes fresh ones the next time you start it. Undo puts the old ones back.", g.name))
}

#[tauri::command(async)]
fn undo_prefix_reset(app_id: String) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = game_of(&c, &app_id)?;
    cachyplay_core::prefix::undo(&c, &g).map_err(|e| format!("{e:#}"))?;
    Ok(format!("Put {}'s earlier Windows files back.", g.name))
}

#[tauri::command(async)]
fn delete_kept_prefixes(app_id: String) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = game_of(&c, &app_id)?;
    let freed = cachyplay_core::prefix::delete_kept(&c, &g).map_err(|e| format!("{e:#}"))?;
    Ok(format!(
        "Deleted the old copies of {}'s Windows files and freed {}.",
        g.name,
        cachyplay_core::prefix::human_size(freed)
    ))
}

#[tauri::command]
async fn protondb_rating(app_id: String) -> CmdResult<Option<serde_json::Value>> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = paths()?;
        if !update::Settings::load(&p).protondb {
            return Ok(None);
        }
        Ok(cachyplay_core::protondb::rating(&p, &app_id).map(|r| {
            serde_json::json!({"tier": r.tier, "trending": r.trending, "reports": r.reports, "meaning": r.meaning(),
                "url": format!("https://www.protondb.com/app/{app_id}")})
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command(async)]
fn app_settings() -> CmdResult<update::Settings> {
    Ok(update::Settings::load(&paths()?))
}

/// Change one display or privacy setting: "protondb", "big_screen" (true/false) or "theme".
#[tauri::command(async)]
fn set_app_setting(key: String, value: serde_json::Value) -> CmdResult<()> {
    let p = paths()?;
    let mut s = update::Settings::load(&p);
    match key.as_str() {
        "protondb" => s.protondb = value.as_bool().unwrap_or(false),
        "big_screen" => s.big_screen = value.as_bool().unwrap_or(false),
        "theme" => {
            s.theme = value
                .as_str()
                .filter(|t| ["system", "dark", "light"].contains(t))
                .map(str::to_string)
        }
        _ => return Err(format!("unknown setting {key}")),
    }
    s.save(&p).map_err(|e| format!("{e:#}"))
}

/// KDE's accent color (from ~/.config/kdeglobals), as "#rrggbb", to match the desktop.
#[tauri::command(async)]
fn desktop_accent() -> Option<String> {
    let p = paths().ok()?;
    let text = std::fs::read_to_string(p.home.join(".config/kdeglobals")).ok()?;
    let line = text.lines().find(|l| l.starts_with("AccentColor="))?;
    let rgb: Vec<u8> = line["AccentColor=".len()..]
        .split(',')
        .filter_map(|x| x.trim().parse().ok())
        .collect();
    (rgb.len() == 3).then(|| format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))
}

/// "dark" or "light" from the desktop's own settings, or None if it doesn't say.
#[tauri::command(async)]
fn desktop_theme() -> Option<String> {
    let p = paths().ok()?;
    cachyplay_core::sys::desktop_theme(&p.home).map(str::to_string)
}

#[tauri::command(async)]
fn open_url(url: String) -> CmdResult<()> {
    if !(url.starts_with("https://") || url.starts_with("steam://")) {
        return Err("Only web and Steam links can be opened.".into());
    }
    std::process::Command::new("xdg-open")
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't open the link: {e}"))
}

/* ---------- updates ---------- */

#[tauri::command]
async fn update_status(force: Option<bool>) -> CmdResult<update::UpdateStatus> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = paths()?;
        Ok(if force.unwrap_or(true) {
            update::check(&p)
        } else {
            update::check_background(&p)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command(async)]
fn set_channel(channel: update::Channel) -> CmdResult<()> {
    let p = paths()?;
    let mut s = update::Settings::load(&p);
    s.channel = channel;
    s.save(&p).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn install_update(to_stable: Option<bool>) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = paths()?;
        let status = update::check(&p);
        if let Some(e) = status.error {
            return Err(e);
        }
        let release = if to_stable.unwrap_or(false) {
            status
                .stable_now
                .ok_or("There's no older Stable version to switch to.")?
        } else {
            status
                .latest
                .filter(|_| status.available)
                .ok_or("You already have the newest version.")?
        };
        update::install(&p, &release).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command(async)]
fn roll_back() -> CmdResult<String> {
    update::roll_back(&paths()?).map_err(|e| format!("{e:#}"))
}

#[tauri::command(async)]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
}

/* ---------- bug reports ---------- */

#[derive(Serialize)]
struct ReportEntry {
    #[serde(flatten)]
    report: report::SavedReport,
    game: Option<String>,
}

#[tauri::command(async)]
fn saved_reports() -> CmdResult<Vec<ReportEntry>> {
    let p = paths()?;
    let games = ctx(&p).map(|c| c.steam.games()).unwrap_or_default();
    Ok(report::saved_reports(&p)
        .into_iter()
        .map(|r| {
            let game = r
                .app_id
                .as_ref()
                .and_then(|id| games.iter().find(|g| &g.app_id == id))
                .map(|g| g.name.clone());
            ReportEntry { report: r, game }
        })
        .collect())
}

#[tauri::command(async)]
fn open_issue_page() -> CmdResult<()> {
    let url = format!("https://github.com/{}/issues/new/choose", update::REPO);
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't open the browser: {e}"))
}

/* ---------- watching for changes (instead of the window asking every few seconds) ---------- */

/// Whether the window can be seen and has focus. Hidden: nothing is refreshed; unfocused: slower.
static VISIBLE: AtomicBool = AtomicBool::new(true);
static FOCUSED: AtomicBool = AtomicBool::new(true);
/// Set when the window comes back, so the watcher looks at once instead of waiting.
static CHECK_NOW: AtomicBool = AtomicBool::new(true);

/// How long the watcher waits between looks.
fn watch_interval(visible: bool, focused: bool, waiting: bool) -> Option<Duration> {
    match (visible, focused) {
        (true, true) => Some(Duration::from_secs(3)),
        (true, false) => Some(Duration::from_secs(10)),
        // Hidden: only keep an eye on Steam if changes are waiting for it to close.
        (false, _) if waiting => Some(Duration::from_secs(5)),
        (false, _) => None,
    }
}

/// The window tells the watcher when it's hidden (minimized, another workspace) or shown again.
#[tauri::command]
fn set_visible(visible: bool) {
    VISIBLE.store(visible, Ordering::Relaxed);
    if visible {
        CHECK_NOW.store(true, Ordering::Relaxed);
    }
}

/// A background thread that looks at Steam and CachyPlay's data, and tells the window (event
/// "live") only when something changed: Steam opened or closed, a game started, ended or crashed,
/// or a change waiting for Steam ran. Linux has no simple signal for "a game started", so this is
/// a cheap look every few seconds on this thread, never on the window's thread.
fn watch(app: tauri::AppHandle) {
    use cachyplay_core::live::{self, SteamState};
    let mut last_sent: Option<Live> = None;
    let mut last_steam: Option<SteamState> = None;
    let mut last_look = Instant::now() - Duration::from_secs(60);
    let mut pending_ran: Vec<String> = Vec::new();
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let Ok(p) = paths() else { continue };
        let visible = VISIBLE.load(Ordering::Relaxed);
        let waiting_now = !live::queue(&p).is_empty();
        let due = match watch_interval(visible, FOCUSED.load(Ordering::Relaxed), waiting_now) {
            Some(every) => last_look.elapsed() >= every,
            None => false,
        };
        if !(due || (visible && CHECK_NOW.swap(false, Ordering::Relaxed))) {
            continue;
        }
        last_look = Instant::now();
        let Ok(c) = ctx(&p) else { continue };
        let steam = live::steam_state(&c.proc_root);
        if last_steam.is_some_and(|s| s != SteamState::Closed)
            && steam == SteamState::Closed
            && waiting_now
        {
            let bin = library::installed_cli(&p);
            match live::run_queue(&c, bin.as_deref()) {
                Ok(lines) => pending_ran.extend(lines),
                Err(e) => pending_ran.push(format!("{e:#}")),
            }
        }
        last_steam = Some(steam);
        if !visible {
            continue; // nothing is redrawn while hidden; the window catches up when shown
        }
        let now = Live {
            steam,
            token: live::token(&c),
            waiting: live::queue(&p),
            ran: std::mem::take(&mut pending_ran),
        };
        let changed = last_sent.as_ref().is_none_or(|l| {
            l.steam != now.steam || l.token != now.token || l.waiting != now.waiting
        });
        if changed || !now.ran.is_empty() {
            let _ = app.emit("live", &now);
            last_sent = Some(Live { ran: vec![], ..now });
        }
    }
}

/* ---------- cover art through cover://localhost/<appid> ---------- */

/// Serves only the cover images CachyPlay finds for a game in Steam's own cache; nothing else on
/// disk can be reached through this link.
fn cover_response(path: &str) -> tauri::http::Response<Vec<u8>> {
    let not_found = || {
        tauri::http::Response::builder()
            .status(404)
            .body(Vec::new())
            .unwrap_or_default()
    };
    let app_id = path.trim_matches('/');
    if app_id.is_empty() || !app_id.bytes().all(|b| b.is_ascii_digit()) {
        return not_found();
    }
    let Some(file) = paths()
        .ok()
        .and_then(|p| ctx(&p).ok())
        .and_then(|c| library::cover_art(&c.steam.root, app_id))
    else {
        return not_found();
    };
    let Ok(bytes) = std::fs::read(&file) else {
        return not_found();
    };
    if bytes.len() > 4 * 1024 * 1024 {
        return not_found();
    }
    let mime = if file.extension().is_some_and(|e| e == "png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    tauri::http::Response::builder()
        .header("Content-Type", mime)
        .header("Cache-Control", "max-age=3600")
        .body(bytes)
        .unwrap_or_else(|_| not_found())
}

/* ---------- "closed unexpectedly" ---------- */

static LAST_CRASH: Mutex<Option<cachyplay_core::crash::LastCrash>> = Mutex::new(None);

#[tauri::command]
fn last_crash() -> Option<cachyplay_core::crash::LastCrash> {
    LAST_CRASH.lock().ok().and_then(|c| c.clone())
}

#[tauri::command]
fn dismiss_crash() {
    if let Ok(mut c) = LAST_CRASH.lock() {
        *c = None;
    }
}

/// Errors in the window itself (window.onerror) go into CachyPlay's log.
#[tauri::command(async)]
fn log_ui_error(message: String) {
    if let Ok(p) = paths() {
        let msg: String = message.chars().take(2000).collect();
        cachyplay_core::logging::write(&p, &format!("window error: {msg}"));
    }
}

/// `cachyplay-gui --version`, `--self-test [--quick]`: answered without opening a window.
fn answer_flags(context: &tauri::Context) -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("cachyplay-gui {}", cachyplay_core::VERSION);
        return Some(0);
    }
    if !args.iter().any(|a| a == "--self-test") {
        return None;
    }
    let p = match Paths::from_env() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[FAIL] {e:#}");
            return Some(1);
        }
    };
    let mut lines = match cachyplay_core::selftest::quick(&p) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[FAIL] {e:#}");
            return Some(1);
        }
    };
    for page in ["index.html", "app.js", "style.css"] {
        if context
            .assets()
            .get(&tauri::utils::assets::AssetKey::from(page))
            .is_none()
        {
            eprintln!("[FAIL] the window's {page} is missing from this build");
            return Some(1);
        }
    }
    lines.push("Window files built in".into());
    for l in lines {
        println!("[ok  ] {l}");
    }
    if !args.iter().any(|a| a == "--quick") {
        for c in cachyplay_core::pccheck::self_test(&p) {
            println!("[{:?}] {}: {}", c.level, c.title, c.detail);
        }
    }
    println!("Self-test passed.");
    Some(0)
}

/// Logging out, shutting down or `kill` send SIGTERM/SIGHUP/SIGINT: close normally (removing the
/// running marker) instead of being cut off, so the next start doesn't say it closed unexpectedly.
fn close_on_signals(app: tauri::AppHandle) {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGTERM, SIGHUP, SIGINT]) else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            if signals.forever().next().is_some() {
                if let Ok(p) = paths() {
                    cachyplay_core::crash::clean_exit(&p);
                }
                app.exit(0);
            }
        });
}

fn main() {
    let context = tauri::generate_context!();
    if let Some(code) = answer_flags(&context) {
        std::process::exit(code);
    }
    if let Ok(p) = Paths::from_env() {
        cachyplay_core::crash::install_panic_hook(&p, "cachyplay-gui");
        let previous = cachyplay_core::crash::check(&p, std::path::Path::new("/proc"));
        if let Ok(mut c) = LAST_CRASH.lock() {
            *c = previous;
        }
        cachyplay_core::housekeeping::protect_settings(&p);
        cachyplay_core::install::refresh_menu_entries(&p);
    }
    let app = tauri::Builder::default()
        // Must be first: a second launch hands over to this window and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .register_asynchronous_uri_scheme_protocol("cover", |_ctx, request, responder| {
            let path = request.uri().path().to_string();
            std::thread::spawn(move || responder.respond(cover_response(&path)));
        })
        .setup(|app| {
            // By now a second launch has handed over and exited, so this is the copy that stays.
            if let Ok(p) = paths() {
                cachyplay_core::crash::claim(&p, std::path::Path::new("/proc"));
            }
            close_on_signals(app.handle().clone());
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("watcher".into())
                .spawn(move || watch(handle))?;
            Ok(())
        })
        .on_window_event(|_w, event| {
            if let tauri::WindowEvent::Focused(f) = event {
                FOCUSED.store(*f, Ordering::Relaxed);
                if *f {
                    CHECK_NOW.store(true, Ordering::Relaxed);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            set_visible,
            last_crash,
            dismiss_crash,
            log_ui_error,
            overview,
            game_detail,
            set_enabled,
            set_prefer_card,
            live_status,
            queue_action,
            cancel_waiting,
            run_waiting,
            close_steam,
            open_steam,
            set_your_settings,
            reset_shader_cache,
            undo_change,
            try_again,
            undo_shader_cache,
            set_run_note,
            set_online_only,
            switch_compat_tool,
            undo_compat_tool,
            set_obs_capture,
            set_obs_all,
            obs_settings,
            create_report,
            open_folder,
            restore_all,
            uninstall,
            install_self,
            update_status,
            set_channel,
            install_update,
            roll_back,
            restart_app,
            saved_reports,
            open_issue_page,
            game_suggestions,
            game_insights,
            fix_it,
            set_verdict,
            pc_check,
            self_test,
            reset_prefix,
            undo_prefix_reset,
            delete_kept_prefixes,
            protondb_rating,
            app_settings,
            set_app_setting,
            desktop_accent,
            desktop_theme,
            open_url,
            apply_fix,
            undo_fix,
            game_settings_all,
            export_settings
        ])
        .build(context)
        .expect("error while starting CachyPlay");
    app.run(|_app, event| {
        if let tauri::RunEvent::Exit = event {
            if let Ok(p) = Paths::from_env() {
                cachyplay_core::crash::clean_exit(&p);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_rests_while_hidden_and_slows_when_unfocused() {
        assert_eq!(
            watch_interval(true, true, false),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            watch_interval(true, false, false),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            watch_interval(false, false, false),
            None,
            "hidden: nothing runs"
        );
        assert!(
            watch_interval(false, false, true).is_some(),
            "still runs waiting changes when Steam closes"
        );
    }

    #[test]
    fn cover_links_only_take_an_app_id() {
        for bad in ["", "/", "/../../etc/passwd", "/12a", "/%2e%2e"] {
            assert_eq!(cover_response(bad).status(), 404, "{bad}");
        }
    }
}
