//! CachyPlay desktop app. A thin Tauri shell: every command calls into `cachyplay-core`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine;
use cachyplay_core::library::{self, GameDetail, Overview};
use cachyplay_core::manage::{touched_accounts, Ctx, Outcome};
use cachyplay_core::paths::Paths;
use cachyplay_core::recipes::Facts;
use cachyplay_core::{install, report, update};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::OnceLock;

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

#[tauri::command]
fn overview() -> CmdResult<Overview> {
    let p = paths()?;
    let c = ctx(&p);
    Ok(library::overview(&p, c.as_ref().map_err(Clone::clone)))
}

#[tauri::command]
fn game_detail(app_id: String) -> CmdResult<GameDetail> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = c
        .steam
        .find_game(&app_id)
        .ok_or_else(|| format!("Game {app_id} is not installed"))?;
    Ok(library::detail(&c, &g, facts()))
}

/// Cover art from Steam's local cache as a data URL (no network, no file-access permission).
#[tauri::command]
fn cover(app_id: String) -> Option<String> {
    let p = paths().ok()?;
    let c = ctx(&p).ok()?;
    let file = library::cover_art(&c.steam.root, &app_id)?;
    let bytes = std::fs::read(&file).ok()?;
    if bytes.len() > 4 * 1024 * 1024 {
        return None;
    }
    let mime = if file.extension().is_some_and(|e| e == "png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// OBS game capture for one game. Turning it on also turns CachyPlay on for the game, since
/// CachyPlay's launch helper is what starts the game through obs-gamecapture.
#[tauri::command]
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
#[tauri::command]
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

#[tauri::command]
fn set_online_only(app_id: String, on: bool) -> CmdResult<()> {
    let p = paths()?;
    cachyplay_core::notes::set_online_only(&p, &app_id, on).map_err(|e| format!("{e:#}"))
}

/// "Use the graphics card" for one game (PCs with two graphics chips).
#[tauri::command]
fn set_prefer_card(app_id: String, on: bool) -> CmdResult<String> {
    let p = paths()?;
    cachyplay_core::gpus::set_prefer_card(&p, &app_id, on).map_err(|e| format!("{e:#}"))?;
    Ok(if on {
        "The game will use the graphics card the next time it starts.".into()
    } else {
        "The game picks its graphics chip by itself from now on.".into()
    })
}

#[tauri::command]
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

#[tauri::command]
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
#[tauri::command]
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

#[tauri::command]
fn undo_compat_tool(app_id: String) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    cachyplay_core::compat::undo(&c, &app_id).map_err(|e| format!("{e:#}"))?;
    Ok("Put the game back on the compatibility tool it used before.".into())
}

/// Everything the app polls every few seconds: Steam's state, a fingerprint of the data the
/// pages are built from (so the app refreshes when a game starts, ends or crashes), and the
/// changes waiting for Steam to close.
#[derive(serde::Serialize)]
struct Live {
    steam: cachyplay_core::live::SteamState,
    token: String,
    waiting: Vec<cachyplay_core::live::Action>,
}

#[tauri::command]
fn live_status() -> CmdResult<Live> {
    let p = paths()?;
    let c = ctx(&p)?;
    Ok(Live {
        steam: cachyplay_core::live::steam_state(&c.proc_root),
        token: cachyplay_core::live::token(&c),
        waiting: cachyplay_core::live::queue(&p),
    })
}

/// Save a change to run once Steam closes.
#[tauri::command]
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

#[tauri::command]
fn cancel_waiting(app_id: String) -> CmdResult<()> {
    let p = paths()?;
    cachyplay_core::live::cancel(&p, &app_id).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn run_waiting() -> CmdResult<Vec<String>> {
    let p = paths()?;
    let c = ctx(&p)?;
    let bin = library::installed_cli(&p);
    cachyplay_core::live::run_queue(&c, bin.as_deref()).map_err(|e| format!("{e:#}"))
}

/// Ask Steam to quit properly (the same as Steam → Exit).
#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
fn undo_shader_cache(app_id: String) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = game_of(&c, &app_id)?;
    cachyplay_core::shadercache::undo(&c, &g).map_err(|e| format!("{e:#}"))?;
    let _ = cachyplay_core::fixes::remove(&p, &app_id, "shader_cache");
    Ok(format!("Put {}'s old shader cache back.", g.name))
}

/// "Try these settings" for one game.
#[tauri::command]
fn set_your_settings(app_id: String, text: String) -> CmdResult<String> {
    let p = paths()?;
    Ok(match cachyplay_core::fixes::set_your_settings(&p, &app_id, &text).map_err(|e| format!("{e:#}"))? {
        Some(_) => "Saved. They're used the next time the game starts (CachyPlay must be on for the game).".into(),
        None => "Your settings for this game were removed.".into(),
    })
}

#[tauri::command]
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
#[tauri::command]
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

#[tauri::command]
fn restore_all() -> CmdResult<Vec<String>> {
    restore_everything(&paths()?)
}

#[tauri::command]
fn game_settings_all() -> CmdResult<Vec<cachyplay_core::keep::GameSettings>> {
    let p = paths()?;
    Ok(cachyplay_core::keep::all(&ctx(&p)?))
}

#[tauri::command]
fn export_settings() -> CmdResult<String> {
    let p = paths()?;
    let games = cachyplay_core::keep::all(&ctx(&p)?);
    let file = p.report_dir().join("CachyPlay game settings.txt");
    std::fs::write(&file, cachyplay_core::keep::export_text(&games)).map_err(|e| e.to_string())?;
    Ok(file.display().to_string())
}

#[tauri::command]
fn uninstall(remove_backups: bool, keep_settings: bool) -> CmdResult<String> {
    let p = paths()?;
    let mut saved = None;
    if keep_settings {
        if let Ok(c) = Ctx::for_restore(p.clone()) {
            let games = cachyplay_core::keep::all(&c);
            if !games.is_empty() {
                let file = p.report_dir().join("CachyPlay game settings.txt");
                std::fs::write(&file, cachyplay_core::keep::export_text(&games))
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
#[tauri::command]
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

#[tauri::command]
fn undo_fix(app_id: String, id: String) -> CmdResult<String> {
    let p = paths()?;
    match cachyplay_core::fixes::remove(&p, &app_id, &id).map_err(|e| format!("{e:#}"))? {
        true => Ok("Fix undone. The game starts without it next time.".into()),
        false => Ok("That fix wasn't applied.".into()),
    }
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
#[tauri::command]
fn set_verdict(app_id: String, id: String, worked: bool) -> CmdResult<String> {
    let p = paths()?;
    cachyplay_core::fixes::set_verdict(&p, &app_id, &id, worked).map_err(|e| format!("{e:#}"))?;
    Ok(if worked {
        "Great, noted. This fix stays on.".into()
    } else {
        "Noted. Try the next suggestion, or undo this one.".into()
    })
}

#[tauri::command]
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

#[tauri::command]
fn self_test() -> CmdResult<Vec<cachyplay_core::pccheck::Check>> {
    Ok(cachyplay_core::pccheck::self_test(&paths()?))
}

#[tauri::command]
fn reset_prefix(app_id: String) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = game_of(&c, &app_id)?;
    cachyplay_core::prefix::reset(&c, &g).map_err(|e| format!("{e:#}"))?;
    Ok(format!("{}'s Windows files were set aside. Steam makes fresh ones the next time you start it. Undo puts the old ones back.", g.name))
}

#[tauri::command]
fn undo_prefix_reset(app_id: String) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = game_of(&c, &app_id)?;
    cachyplay_core::prefix::undo(&c, &g).map_err(|e| format!("{e:#}"))?;
    Ok(format!("Put {}'s earlier Windows files back.", g.name))
}

#[tauri::command]
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

#[tauri::command]
fn app_settings() -> CmdResult<update::Settings> {
    Ok(update::Settings::load(&paths()?))
}

/// Change one display or privacy setting: "protondb", "big_screen" (true/false) or "theme".
#[tauri::command]
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
#[tauri::command]
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
#[tauri::command]
fn desktop_theme() -> Option<String> {
    let p = paths().ok()?;
    cachyplay_core::sys::desktop_theme(&p.home).map(str::to_string)
}

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
fn roll_back() -> CmdResult<String> {
    update::roll_back(&paths()?).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
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

#[tauri::command]
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

#[tauri::command]
fn open_issue_page() -> CmdResult<()> {
    let url = format!("https://github.com/{}/issues/new/choose", update::REPO);
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't open the browser: {e}"))
}

fn main() {
    if let Ok(p) = Paths::from_env() {
        cachyplay_core::housekeeping::protect_settings(&p);
        cachyplay_core::install::refresh_menu_entries(&p);
    }
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            overview,
            game_detail,
            cover,
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
        .run(tauri::generate_context!())
        .expect("error while running CachyPlay");
}
