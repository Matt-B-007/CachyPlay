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
    restore_everything_with(p, false)
}

/// Restore every managed game; with `keep`, games with settings get them written into Steam.
fn restore_everything_with(p: &Paths, keep: bool) -> CmdResult<Vec<String>> {
    let accounts = touched_accounts(p);
    if accounts.is_empty() {
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
    Ok(lines)
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
    let restored = restore_everything_with(&p, keep_settings)?;
    install::remove_user_files(&p, remove_backups).map_err(|e| format!("{e:#}"))?;
    let exe = std::env::current_exe().unwrap_or_default();
    let kept = restored
        .iter()
        .filter(|l| l.ends_with("settings kept in Steam"))
        .count();
    let back = restored.len() - kept;
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

/* ---------- updates ---------- */

#[tauri::command]
async fn update_status() -> CmdResult<update::UpdateStatus> {
    tauri::async_runtime::spawn_blocking(|| Ok(update::check(&paths()?)))
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
async fn install_update() -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(|| {
        let p = paths()?;
        let status = update::check(&p);
        if let Some(e) = status.error {
            return Err(e);
        }
        let release = status
            .latest
            .filter(|_| status.available)
            .ok_or("You already have the newest version.")?;
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
            install_self,
            update_status,
            set_channel,
            install_update,
            roll_back,
            restart_app,
            saved_reports,
            open_issue_page,
            game_suggestions,
            apply_fix,
            undo_fix,
            game_settings_all,
            export_settings
        ])
        .run(tauri::generate_context!())
        .expect("error while running CachyPlay");
}
