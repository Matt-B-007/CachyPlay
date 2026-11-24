//! OBS game capture: start games through `obs-gamecapture` so OBS's "Game Capture" source records
//! just the game, not the whole desktop.
//!
//! This is the same as putting `obs-gamecapture %command%` in a game's Steam launch options, but
//! as a switch: per game, or for every game CachyPlay is on for. `obs-gamecapture` comes with the
//! obs-vkcapture package (CachyOS: `sudo pacman -S obs-vkcapture lib32-obs-vkcapture`).
//! If it isn't installed, the game simply starts without it.

use crate::manage::{read_json, write_json};
use crate::paths::Paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const TOOL: &str = "obs-gamecapture";
pub const INSTALL_COMMAND: &str = "sudo pacman -S --needed obs-vkcapture lib32-obs-vkcapture";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ObsSettings {
    /// Every game CachyPlay is on for.
    #[serde(default)]
    pub all: bool,
    /// AppIDs with capture turned on one by one.
    #[serde(default)]
    pub games: Vec<String>,
}

fn file(paths: &Paths) -> std::path::PathBuf {
    paths.config.join("obs-capture.json")
}

pub fn load(paths: &Paths) -> ObsSettings {
    read_json(&file(paths)).unwrap_or_default()
}

pub fn save(paths: &Paths, s: &ObsSettings) -> Result<()> {
    write_json(&file(paths), s)
}

pub fn enabled_for(paths: &Paths, app_id: &str) -> bool {
    let s = load(paths);
    s.all || s.games.iter().any(|g| g == app_id)
}

pub fn set_game(paths: &Paths, app_id: &str, on: bool) -> Result<()> {
    let mut s = load(paths);
    s.games.retain(|g| g != app_id);
    if on {
        s.games.push(app_id.to_string());
        s.games.sort();
    }
    save(paths, &s)
}

pub fn set_all(paths: &Paths, on: bool) -> Result<()> {
    let mut s = load(paths);
    s.all = on;
    save(paths, &s)
}

pub fn installed() -> bool {
    crate::sys::which(TOOL).is_some()
}

/// True if the user already starts this game through obs-gamecapture in their own launch options.
pub fn in_launch_options(options: &str) -> bool {
    options
        .split_whitespace()
        .any(|w| w == TOOL || w.ends_with("/obs-gamecapture"))
}

/// What the app shows for one game.
#[derive(Debug, Clone, Serialize)]
pub struct ObsInfo {
    /// Turned on in CachyPlay for this game (one by one, or through "every game").
    pub on: bool,
    /// Turned on for every game.
    pub all: bool,
    /// obs-gamecapture is installed.
    pub installed: bool,
    /// The user's own Steam launch options already use obs-gamecapture.
    pub in_your_options: bool,
    pub install_command: String,
}

pub fn info(paths: &Paths, app_id: &str, user_options: &str) -> ObsInfo {
    let s = load(paths);
    ObsInfo {
        on: s.all || s.games.iter().any(|g| g == app_id),
        all: s.all,
        installed: installed(),
        in_your_options: in_launch_options(user_options),
        install_command: INSTALL_COMMAND.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_game_and_all() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::isolated(tmp.path());
        assert!(!enabled_for(&p, "10"));
        set_game(&p, "10", true).unwrap();
        set_game(&p, "10", true).unwrap();
        assert_eq!(load(&p).games, vec!["10".to_string()], "no duplicates");
        assert!(enabled_for(&p, "10") && !enabled_for(&p, "20"));
        set_all(&p, true).unwrap();
        assert!(enabled_for(&p, "20"));
        set_all(&p, false).unwrap();
        set_game(&p, "10", false).unwrap();
        assert!(!enabled_for(&p, "10"));
    }

    #[test]
    fn spots_it_in_launch_options() {
        assert!(in_launch_options("obs-gamecapture %command%"));
        assert!(in_launch_options(
            "MANGOHUD=1 /usr/bin/obs-gamecapture %command% -dx11"
        ));
        assert!(!in_launch_options("OBS_VKCAPTURE=1 %command%"));
        assert!(!in_launch_options(""));
    }
}
