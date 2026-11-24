//! CachyPlay desktop app. A thin Tauri shell: every command calls into `cachyplay-core`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine;
use cachyplay_core::library::{self, GameDetail, Overview};
use cachyplay_core::manage::{touched_accounts, Ctx, Outcome};
use cachyplay_core::paths::Paths;
use cachyplay_core::recipes::Facts;
use cachyplay_core::{install, report};
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

#[tauri::command]
fn set_enabled(app_id: String, enabled: bool) -> CmdResult<String> {
    let p = paths()?;
    let c = ctx(&p)?;
    let g = c
        .steam
        .find_game(&app_id)
        .ok_or_else(|| format!("Game {app_id} is not installed"))?;
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
    let base = ctx(p)?;
    let mut lines = Vec::new();
    for account in touched_accounts(p) {
        let c = base.for_account(&account);
        for app in c.managed_apps() {
            let name = c
                .steam
                .find_game(&app)
                .map(|g| g.name)
                .unwrap_or_else(|| app.clone());
            match c.undo(&app) {
                Ok(Outcome::RemovedKeptUserEdits) => {
                    lines.push(format!("{name}: restored (your edits kept)"))
                }
                Ok(_) => lines.push(format!("{name}: restored")),
                Err(e) => return Err(format!("{name}: {e:#}")),
            }
        }
    }
    Ok(lines)
}

#[tauri::command]
fn restore_all() -> CmdResult<Vec<String>> {
    restore_everything(&paths()?)
}

#[tauri::command]
fn uninstall(remove_backups: bool) -> CmdResult<String> {
    let p = paths()?;
    let restored = restore_everything(&p)?;
    install::remove_user_files(&p, remove_backups).map_err(|e| format!("{e:#}"))?;
    let exe = std::env::current_exe().unwrap_or_default();
    let games = match restored.len() {
        0 => "No games needed restoring".to_string(),
        1 => "Restored 1 game".to_string(),
        n => format!("Restored {n} games"),
    };
    let mut msg = format!("{games} and removed CachyPlay's files.");
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

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            overview,
            game_detail,
            cover,
            set_enabled,
            create_report,
            open_folder,
            restore_all,
            uninstall,
            install_self
        ])
        .run(tauri::generate_context!())
        .expect("error while running CachyPlay");
}
