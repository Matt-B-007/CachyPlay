//! Keeping the app up to date while it's open, and changes that wait for Steam to close.
//!
//! - [`steam_state`]: running, closing (helper processes still finishing), or closed.
//! - [`token`]: changes whenever something the app shows may have changed (a game started or
//!   ended, a crash, Steam saved its settings), so the app can refresh by itself.
//! - The queue: turning CachyPlay on or off, switching Proton and resetting Windows files need
//!   Steam closed. With Steam open they wait here and run the moment Steam closes.

use crate::manage::{read_json, write_json, Ctx};
use crate::paths::Paths;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SteamState {
    Closed,
    Running,
    /// Steam's main process is gone but its helpers are still finishing.
    Closing,
}

pub fn steam_state(proc_root: &Path) -> SteamState {
    let mut main = false;
    let mut helper = false;
    for e in fs::read_dir(proc_root).into_iter().flatten().flatten() {
        if !e
            .file_name()
            .to_string_lossy()
            .chars()
            .all(|c| c.is_ascii_digit())
        {
            continue;
        }
        match fs::read_to_string(e.path().join("comm"))
            .unwrap_or_default()
            .trim()
        {
            "steam" => main = true,
            "steamwebhelper" | "steam-runtime-l" | "srt-logger" => helper = true,
            _ => {}
        }
    }
    match (main, helper) {
        (true, _) => SteamState::Running,
        (false, true) => SteamState::Closing,
        _ => SteamState::Closed,
    }
}

fn stamp(p: &Path) -> u128 {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn newest_in(dir: &Path) -> (u128, usize) {
    let mut n = 0;
    let mut best = stamp(dir);
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        n += 1;
        best = best.max(stamp(&e.path()));
    }
    (best, n)
}

/// A fingerprint of everything the game pages are built from. When it changes, refresh.
pub fn token(ctx: &Ctx) -> String {
    let p = &ctx.paths;
    let (launches, n) = newest_in(&p.state.join("launches"));
    let parts = [
        launches,
        n as u128,
        stamp(&ctx.steam.root.join("logs/console_log.txt")),
        stamp(&ctx.steam.localconfig_path(&ctx.account.account_id)),
        stamp(&p.state.join("gpu-crash-lines.log")),
        newest_in(&p.data.join("fixes")).0,
        newest_in(&p.data.join("run-notes")).0,
        stamp(&queue_file(p)),
    ];
    parts
        .iter()
        .map(|x| format!("{x:x}"))
        .collect::<Vec<_>>()
        .join("-")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Turn CachyPlay on (keeps the player's own launch options).
    Enable {
        app_id: String,
    },
    Disable {
        app_id: String,
    },
    SwitchTool {
        app_id: String,
        tool: String,
    },
    UndoTool {
        app_id: String,
    },
    ResetPrefix {
        app_id: String,
    },
}

impl Action {
    pub fn app_id(&self) -> &str {
        match self {
            Action::Enable { app_id }
            | Action::Disable { app_id }
            | Action::SwitchTool { app_id, .. }
            | Action::UndoTool { app_id }
            | Action::ResetPrefix { app_id } => app_id,
        }
    }
    pub fn describe(&self, name: &str) -> String {
        match self {
            Action::Enable { .. } => format!("Turn CachyPlay on for {name}"),
            Action::Disable { .. } => format!("Turn CachyPlay off for {name}"),
            Action::SwitchTool { tool, .. } => format!("Switch {name} to {tool}"),
            Action::UndoTool { .. } => format!("Put {name}'s Proton version back"),
            Action::ResetPrefix { .. } => format!("Reset {name}'s Windows files"),
        }
    }
    /// Two actions that cancel or replace each other for the same game.
    fn replaces(&self, other: &Action) -> bool {
        use Action::*;
        self.app_id() == other.app_id()
            && matches!(
                (self, other),
                (
                    Enable { .. } | Disable { .. },
                    Enable { .. } | Disable { .. }
                ) | (
                    SwitchTool { .. } | UndoTool { .. },
                    SwitchTool { .. } | UndoTool { .. }
                ) | (ResetPrefix { .. }, ResetPrefix { .. })
            )
    }
}

fn queue_file(p: &Paths) -> PathBuf {
    p.data.join("waiting-for-steam.json")
}

pub fn queue(p: &Paths) -> Vec<Action> {
    read_json(&queue_file(p)).unwrap_or_default()
}

/// Add an action to run once Steam closes. A newer choice for the same thing replaces the older.
pub fn enqueue(p: &Paths, a: Action) -> Result<Vec<Action>> {
    let mut q = queue(p);
    q.retain(|x| !a.replaces(x));
    q.push(a);
    write_json(&queue_file(p), &q)?;
    Ok(q)
}

pub fn cancel(p: &Paths, app_id: &str) -> Result<()> {
    let mut q = queue(p);
    q.retain(|x| x.app_id() != app_id);
    write_json(&queue_file(p), &q)
}

/// Run the waiting actions (Steam must be closed). Returns one line per action; failed actions are
/// dropped from the queue too, with the reason in their line, so they don't retry forever.
pub fn run_queue(ctx: &Ctx, bin: Option<&Path>) -> Result<Vec<String>> {
    if steam_state(&ctx.proc_root) == SteamState::Running {
        return Err(anyhow!("Steam is still open."));
    }
    let q = queue(&ctx.paths);
    let mut lines = Vec::new();
    for a in &q {
        let game = ctx.steam.find_game(a.app_id());
        let name = game
            .as_ref()
            .map(|g| g.name.clone())
            .unwrap_or_else(|| a.app_id().to_string());
        let result: Result<()> = (|| {
            let g = game.as_ref().ok_or_else(|| anyhow!("game not found"))?;
            match a {
                Action::Enable { .. } => {
                    let bin = bin.ok_or_else(|| anyhow!("CachyPlay isn't installed"))?;
                    ctx.apply(g, bin)?;
                }
                Action::Disable { .. } => {
                    ctx.undo(&g.app_id)?;
                }
                Action::SwitchTool { tool, .. } => {
                    let tools =
                        crate::compat::installed(&ctx.steam.root, &ctx.steam.library_folders());
                    let t = tools
                        .iter()
                        .find(|t| &t.name == tool || &t.display == tool)
                        .ok_or_else(|| anyhow!("{tool} isn't installed any more"))?;
                    crate::compat::switch(ctx, &g.app_id, t)?;
                }
                Action::UndoTool { .. } => crate::compat::undo(ctx, &g.app_id)?,
                Action::ResetPrefix { .. } => {
                    crate::prefix::reset(ctx, g)?;
                }
            }
            Ok(())
        })();
        lines.push(match result {
            Ok(()) => format!("Done: {}", a.describe(&name)),
            Err(e) => format!("Couldn't {}: {e:#}", a.describe(&name).to_lowercase()),
        });
    }
    write_json(&queue_file(&ctx.paths), &Vec::<Action>::new())?;
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::FakeSteam;

    fn proc_with(names: &[&str]) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        for (i, n) in names.iter().enumerate() {
            let d = t.path().join(format!("{}", 100 + i));
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("comm"), format!("{n}\n")).unwrap();
        }
        t
    }

    #[test]
    fn tells_running_closing_and_closed_apart() {
        assert_eq!(
            steam_state(proc_with(&["steam", "steamwebhelper"]).path()),
            SteamState::Running
        );
        assert_eq!(
            steam_state(proc_with(&["steamwebhelper"]).path()),
            SteamState::Closing
        );
        assert_eq!(steam_state(proc_with(&["bash"]).path()), SteamState::Closed);
    }

    #[test]
    fn queue_keeps_the_newest_choice_and_runs_when_steam_closes() {
        let f = FakeSteam::new();
        let ctx = f.ctx();
        let p = &ctx.paths;
        enqueue(
            p,
            Action::Enable {
                app_id: "1240440".into(),
            },
        )
        .unwrap();
        enqueue(
            p,
            Action::Disable {
                app_id: "1240440".into(),
            },
        )
        .unwrap();
        let q = enqueue(
            p,
            Action::Enable {
                app_id: "1240440".into(),
            },
        )
        .unwrap();
        assert_eq!(q.len(), 1, "{q:?}");
        let bin = f.home().join("cachyplay");
        fs::write(&bin, "").unwrap();
        let lines = run_queue(&ctx, Some(&bin)).unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("Done: Turn CachyPlay on"), "{lines:?}");
        let g = ctx.steam.find_game("1240440").unwrap();
        assert!(ctx.status(&g).unwrap().applied);
        assert!(queue(p).is_empty());
    }

    #[test]
    fn token_changes_when_a_game_starts() {
        let f = FakeSteam::new();
        let ctx = f.ctx();
        let before = token(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::create_dir_all(ctx.paths.state.join("launches")).unwrap();
        fs::write(ctx.paths.state.join("launches/1240440-x.json"), "{}").unwrap();
        assert_ne!(before, token(&ctx));
    }
}
