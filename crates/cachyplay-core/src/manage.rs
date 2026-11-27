// SPDX-License-Identifier: GPL-3.0-or-later
//! Apply, undo and restore: the only code that writes to Steam's files.
//!
//! Rules enforced here (see the design spec, "Safety model"):
//! - snapshot a game's original launch options before the first change, never overwrite it;
//! - record every change; back up and atomically rewrite `localconfig.vdf`;
//! - never write while Steam is running (Steam would overwrite the edit on exit);
//! - on undo, restore the snapshot exactly if nothing changed since, otherwise remove only
//!   CachyPlay's segment and keep whatever the user edited.

use crate::fsutil::{atomic_write, backup};
use crate::launch_options;
use crate::paths::Paths;
use crate::steam::{self, Account, Game, Steam, APPS_PATH};
use crate::vdf::Doc;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub app_id: String,
    pub account_id: String,
    pub game_name: String,
    pub taken_at: String,
    /// `None` when the game had no LaunchOptions key at all.
    pub launch_options: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Change {
    pub id: String,
    pub label: String,
    pub before: String,
    pub after: String,
    pub source: String,
    pub applied_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChangeLog {
    pub app_id: String,
    pub game_name: String,
    pub changes: Vec<Change>,
}

pub struct Ctx {
    pub paths: Paths,
    pub steam: Steam,
    pub account: Account,
    /// Directory scanned for a running Steam process; tests point this at a fake /proc.
    pub proc_root: PathBuf,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Applied,
    AlreadyApplied,
    /// Restored the exact original launch options.
    Restored,
    /// The user edited launch options after CachyPlay did; only CachyPlay's part was removed.
    RemovedKeptUserEdits,
    NothingToUndo,
    /// CachyPlay was removed and its settings written straight into Steam's launch options.
    Kept,
}

#[derive(Debug)]
pub struct Status {
    pub app_id: String,
    pub launch_options: String,
    pub applied: bool,
    /// The user's own launch options, with CachyPlay's segment removed.
    pub user_options: String,
    /// Launch options were changed outside CachyPlay after it last wrote them.
    pub edited_since: bool,
    pub tweaks: Vec<String>,
}

impl Ctx {
    pub fn new(paths: Paths) -> Result<Ctx> {
        let steam = Steam::locate(&paths.home)?;
        let account = steam.current_account()?;
        Ok(Ctx {
            paths,
            steam,
            account,
            proc_root: PathBuf::from("/proc"),
        })
    }

    /// A context for restoring games: works even if the current account can't be determined,
    /// because restore goes through every account CachyPlay recorded (see `for_account`).
    pub fn for_restore(paths: Paths) -> Result<Ctx> {
        let steam = Steam::locate(&paths.home)?;
        let account = steam.current_account().unwrap_or(Account {
            account_id: String::new(),
            persona: String::new(),
            most_recent: false,
            timestamp: 0,
        });
        Ok(Ctx {
            paths,
            steam,
            account,
            proc_root: PathBuf::from("/proc"),
        })
    }

    /// A context for a specific account (used by restore-all for every touched account).
    pub fn for_account(&self, account_id: &str) -> Ctx {
        Ctx {
            paths: self.paths.clone(),
            steam: self.steam.clone(),
            account: Account {
                account_id: account_id.to_string(),
                persona: String::new(),
                most_recent: false,
                timestamp: 0,
            },
            proc_root: self.proc_root.clone(),
        }
    }

    fn localconfig(&self) -> PathBuf {
        self.steam.localconfig_path(&self.account.account_id)
    }

    pub(crate) fn require_editable(&self) -> Result<()> {
        if self.steam.kind == steam::InstallKind::Flatpak {
            bail!("Flatpak Steam isn't supported yet in CachyPlay 0.1.0. Nothing was changed.");
        }
        if steam::is_running_in(&self.proc_root) {
            bail!("Steam is running. Close Steam first (Steam rewrites its settings when it exits and would undo this change). Nothing was changed.");
        }
        Ok(())
    }

    fn read_doc(&self) -> Result<Doc> {
        let p = self.localconfig();
        let text = fs::read_to_string(&p).with_context(|| {
            format!(
                "reading {} (has this account launched Steam at least once?)",
                p.display()
            )
        })?;
        Doc::parse(&text).with_context(|| format!("parsing {}", p.display()))
    }

    pub fn launch_options(&self, app_id: &str) -> Result<Option<String>> {
        let doc = self.read_doc()?;
        Ok(doc.get_str(&lo_path(app_id)).map(str::to_string))
    }

    /// If Steam's settings file isn't laid out the way CachyPlay knows (Valve changed the
    /// format, or the file is damaged), say why. CachyPlay then changes nothing and keeps only its
    /// read-only features (crash detection, bug reports) until an update supports the new format.
    pub fn format_problem(&self) -> Option<String> {
        let p = self.localconfig();
        let text = match fs::read_to_string(&p) {
            Ok(t) => t,
            // A missing file is handled where it's needed (first start of the account).
            Err(_) => return None,
        };
        let unknown = "Steam's settings file looks different from what this version of CachyPlay knows, so it won't change anything until an update supports it. Crash detection and bug reports still work.";
        match Doc::parse(&text) {
            Err(_) => Some(unknown.into()),
            Ok(doc) if doc.lookup(&APPS_PATH[..1]).is_none() => Some(unknown.into()),
            Ok(_) => None,
        }
    }

    fn write_launch_options(&self, app_id: &str, value: &str) -> Result<()> {
        if let Some(why) = self.format_problem() {
            bail!("{why}");
        }
        let doc = self.read_doc()?;
        let new_text = doc.with_str(&lo_path(app_id), value)?;
        backup(
            &self.localconfig(),
            &self.paths.backups(&self.account.account_id),
            5,
        )?;
        let allowed = vec![lo_path(app_id).join("/").to_ascii_lowercase()];
        crate::fsutil::verified_vdf_write(&self.localconfig(), &doc.text, &new_text, &allowed)?;
        // Read back to confirm what Steam will see.
        let check = self.launch_options(app_id)?;
        if check.as_deref() != Some(value) {
            let _ = atomic_write(&self.localconfig(), doc.text.as_bytes());
            bail!("verification failed after writing launch options for {app_id}; the original was restored");
        }
        Ok(())
    }

    fn snapshot_path(&self, app_id: &str) -> PathBuf {
        self.paths
            .snapshots(&self.account.account_id)
            .join(format!("{app_id}.json"))
    }
    fn changes_path(&self, app_id: &str) -> PathBuf {
        self.paths
            .changes(&self.account.account_id)
            .join(format!("{app_id}.json"))
    }

    pub fn snapshot(&self, app_id: &str) -> Option<Snapshot> {
        read_json(&self.snapshot_path(app_id))
    }
    pub fn change_log(&self, app_id: &str) -> Option<ChangeLog> {
        read_json(&self.changes_path(app_id))
    }

    /// Add the CachyPlay launch wrapper to a game.
    pub fn apply(&self, game: &Game, bin: &Path) -> Result<Outcome> {
        self.require_editable()?;
        let current = self.launch_options(&game.app_id)?;
        let current_str = current.clone().unwrap_or_default();
        let new = launch_options::apply(&current_str, bin);
        if new == current_str {
            return Ok(Outcome::AlreadyApplied);
        }
        if self.snapshot(&game.app_id).is_none() {
            let snap = Snapshot {
                app_id: game.app_id.clone(),
                account_id: self.account.account_id.clone(),
                game_name: game.name.clone(),
                taken_at: now(),
                launch_options: current,
            };
            write_json(&self.snapshot_path(&game.app_id), &snap)?;
        }
        self.write_launch_options(&game.app_id, &new)?;
        let mut log = self.change_log(&game.app_id).unwrap_or_else(|| ChangeLog {
            app_id: game.app_id.clone(),
            game_name: game.name.clone(),
            changes: vec![],
        });
        log.changes.retain(|c| c.id != "launch_wrapper");
        log.changes.push(Change {
            id: "launch_wrapper".into(),
            label: "CachyPlay launch helper".into(),
            before: current_str,
            after: new,
            source: "cachyplay".into(),
            applied_at: now(),
        });
        write_json(&self.changes_path(&game.app_id), &log)?;
        Ok(Outcome::Applied)
    }

    /// Remove CachyPlay from a game, restoring the original launch options when possible.
    pub fn undo(&self, app_id: &str) -> Result<Outcome> {
        self.require_editable()?;
        let current = self.launch_options(app_id)?.unwrap_or_default();
        let log = self.change_log(app_id);
        let snap = self.snapshot(app_id);
        let ours = log
            .as_ref()
            .and_then(|l| l.changes.iter().find(|c| c.id == "launch_wrapper"));
        let (target, outcome) = match (ours, &snap) {
            (Some(c), Some(s)) if c.after == current => (
                s.launch_options.clone().unwrap_or_default(),
                Outcome::Restored,
            ),
            _ if launch_options::is_applied(&current) => (
                launch_options::strip(&current),
                Outcome::RemovedKeptUserEdits,
            ),
            _ => {
                self.forget(app_id);
                return Ok(Outcome::NothingToUndo);
            }
        };
        if target != current {
            self.write_launch_options(app_id, &target)?;
        }
        self.forget(app_id);
        Ok(outcome)
    }

    /// Replace CachyPlay's launch helper with the plain launch options that keep the game's
    /// settings, then stop managing the game.
    pub fn keep_settings(&self, app_id: &str, launch_options: &str) -> Result<Outcome> {
        self.require_editable()?;
        if launch_options::is_applied(launch_options) {
            bail!("refusing to keep launch options that still need CachyPlay");
        }
        self.write_launch_options(app_id, launch_options)?;
        self.forget(app_id);
        Ok(Outcome::Kept)
    }

    fn forget(&self, app_id: &str) {
        let _ = fs::remove_file(self.changes_path(app_id));
        let _ = fs::remove_file(self.snapshot_path(app_id));
    }

    /// App IDs CachyPlay has changed for this account.
    pub fn managed_apps(&self) -> Vec<String> {
        let mut ids: Vec<String> = [
            self.paths.changes(&self.account.account_id),
            self.paths.snapshots(&self.account.account_id),
        ]
        .iter()
        .filter_map(|d| fs::read_dir(d).ok())
        .flat_map(|rd| rd.flatten())
        .filter_map(|e| {
            e.file_name()
                .to_string_lossy()
                .strip_suffix(".json")
                .map(str::to_string)
        })
        .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    pub fn status(&self, game: &Game) -> Result<Status> {
        let lo = self.launch_options(&game.app_id)?.unwrap_or_default();
        let ours = self
            .change_log(&game.app_id)
            .and_then(|l| l.changes.into_iter().find(|c| c.id == "launch_wrapper"));
        Ok(Status {
            app_id: game.app_id.clone(),
            applied: launch_options::is_applied(&lo),
            user_options: launch_options::strip(&lo),
            edited_since: ours.is_some_and(|c| c.after != lo),
            tweaks: detect_file_tweaks(&game.install_dir),
            launch_options: lo,
        })
    }
}

/// Every Steam account CachyPlay has ever changed (folders under changes/ and snapshots/).
pub fn touched_accounts(paths: &Paths) -> Vec<String> {
    let mut out: Vec<String> = ["changes", "snapshots"]
        .iter()
        .filter_map(|d| fs::read_dir(paths.data.join(d)).ok())
        .flat_map(|rd| rd.flatten())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out.dedup();
    out
}

fn lo_path(app_id: &str) -> Vec<&str> {
    let mut p = APPS_PATH.to_vec();
    p.push(app_id);
    p.push("LaunchOptions");
    p
}

/// Signs that the user changed game files themselves (intro-skip renames, drop-in DLL mods).
pub fn detect_file_tweaks(install_dir: &Path) -> Vec<String> {
    const MOD_DLLS: [&str; 8] = [
        "dinput8.dll",
        "winmm.dll",
        "version.dll",
        "dxgi.dll",
        "d3d9.dll",
        "d3d11.dll",
        "dsound.dll",
        "xinput1_3.dll",
    ];
    let mut out = Vec::new();
    walk(install_dir, 0, &mut |p: &Path| {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let rel = p
            .strip_prefix(install_dir)
            .unwrap_or(p)
            .display()
            .to_string();
        if name.ends_with(".bak")
            || name.ends_with(".old")
            || name.ends_with(".disabled")
            || name.ends_with("_backup")
        {
            out.push(format!("Renamed file or folder: {rel}"));
        } else if p.is_file() && name.ends_with(".asi") {
            out.push(format!("ASI mod: {rel}"));
        } else if p.is_file() && MOD_DLLS.contains(&name.as_str()) {
            out.push(format!("Drop-in DLL (often a mod or ReShade): {rel}"));
        }
    });
    out.sort();
    out
}

fn walk(dir: &Path, depth: usize, f: &mut dyn FnMut(&Path)) {
    if depth > 3 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        f(&p);
        if p.is_dir() && !p.is_symlink() {
            walk(&p, depth + 1, f);
        }
    }
}

pub fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

pub fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(p).ok()?).ok()
}

pub fn write_json<T: Serialize>(p: &Path, v: &T) -> Result<()> {
    atomic_write(p, serde_json::to_string_pretty(v)?.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::FakeSteam;

    #[test]
    fn apply_then_undo_restores_byte_identical_file() {
        let f = FakeSteam::new();
        let before = fs::read_to_string(f.localconfig()).unwrap();
        let ctx = f.ctx();
        let game = ctx.steam.find_game("1240440").unwrap();
        assert_eq!(
            ctx.apply(&game, Path::new("/usr/bin/cachyplay")).unwrap(),
            Outcome::Applied
        );
        let lo = ctx.launch_options("1240440").unwrap().unwrap();
        assert!(lo.contains("/usr/bin/cachyplay %command% -nosplash"));
        assert_eq!(
            ctx.apply(&game, Path::new("/usr/bin/cachyplay")).unwrap(),
            Outcome::AlreadyApplied
        );
        assert_eq!(ctx.undo("1240440").unwrap(), Outcome::Restored);
        assert_eq!(fs::read_to_string(f.localconfig()).unwrap(), before);
        assert!(ctx.managed_apps().is_empty());
    }

    #[test]
    fn undo_keeps_user_edits_made_after_apply() {
        let f = FakeSteam::new();
        let ctx = f.ctx();
        let game = ctx.steam.find_game("1240440").unwrap();
        ctx.apply(&game, Path::new("/usr/bin/cachyplay")).unwrap();
        // The user adds an argument in Steam afterwards.
        let edited = format!(
            "{} -windowed",
            ctx.launch_options("1240440").unwrap().unwrap()
        );
        ctx.write_launch_options("1240440", &edited).unwrap();
        assert_eq!(ctx.undo("1240440").unwrap(), Outcome::RemovedKeptUserEdits);
        assert_eq!(
            ctx.launch_options("1240440").unwrap().unwrap(),
            "%command% -nosplash -windowed"
        );
    }

    #[test]
    fn game_without_launch_options_key() {
        let f = FakeSteam::new();
        let ctx = f.ctx();
        let game = ctx.steam.find_game("570").unwrap();
        ctx.apply(&game, Path::new("/usr/bin/cachyplay")).unwrap();
        assert_eq!(ctx.snapshot("570").unwrap().launch_options, None);
        assert_eq!(ctx.undo("570").unwrap(), Outcome::Restored);
        assert_eq!(ctx.launch_options("570").unwrap().as_deref(), Some(""));
    }

    #[test]
    fn refuses_while_steam_runs() {
        let f = FakeSteam::new();
        f.start_steam();
        let ctx = f.ctx();
        let before = fs::read_to_string(f.localconfig()).unwrap();
        let game = ctx.steam.find_game("1240440").unwrap();
        let err = ctx
            .apply(&game, Path::new("/usr/bin/cachyplay"))
            .unwrap_err();
        assert!(err.to_string().contains("Steam is running"));
        assert_eq!(fs::read_to_string(f.localconfig()).unwrap(), before);
    }

    #[test]
    fn snapshot_is_never_overwritten() {
        let f = FakeSteam::new();
        let ctx = f.ctx();
        let game = ctx.steam.find_game("1240440").unwrap();
        ctx.apply(&game, Path::new("/usr/bin/cachyplay")).unwrap();
        let first = ctx.snapshot("1240440").unwrap();
        ctx.apply(&game, Path::new("/home/x/.local/bin/cachyplay"))
            .unwrap();
        assert_eq!(ctx.snapshot("1240440").unwrap(), first);
    }

    #[test]
    fn detects_renamed_movies_and_mod_dlls() {
        let f = FakeSteam::new();
        let ctx = f.ctx();
        let game = ctx.steam.find_game("1240440").unwrap();
        fs::create_dir_all(game.install_dir.join("Content/Movies.bak")).unwrap();
        fs::write(game.install_dir.join("dinput8.dll"), "").unwrap();
        let t = ctx.status(&game).unwrap().tweaks;
        assert_eq!(t.len(), 2, "{t:?}");
    }
}
