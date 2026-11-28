// SPDX-License-Identifier: GPL-3.0-or-later
//! Extra facts the advice rules use: what a Unity game's own log says about its graphics, whether
//! a game ships anti-cheat that blocks Linux, and how the games that *do* run fine are set up.

use crate::crashmatch::{CrashEvent, Session};
use serde::Serialize;
use std::path::Path;

/// What a Unity game's Player.log says.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct UnityInfo {
    /// "Direct3D 11", "Direct3D 12", "Vulkan".
    pub graphics: Option<String>,
    /// The game picked its graphics mode with a start option (e.g. -force-d3d11).
    pub forced: bool,
    /// The game turns on its own frame generation (FSR/XeSS/DLSS frame gen).
    pub frame_generation: bool,
    /// The game's version, if it logs one ("Moon Sweepers 1.0.6 (…)" → "1.0.6").
    pub version: Option<String>,
    /// The forced graphics mode broke the game's renderer: missing shader pieces, or the same
    /// rendering error over and over (SHRED A BILLION in Direct3D 12: black screen).
    pub mode_broken: Option<String>,
    /// Unity engine version ("6000.0.74f1").
    pub engine: Option<String>,
}

pub fn unity_info(log: &str, game_name: &str) -> UnityInfo {
    let mut info = UnityInfo::default();
    let mut missing: Option<String> = None;
    let mut render_errors = 0usize;
    for line in log.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("Initialize engine version:") {
            info.engine = v.split_whitespace().next().map(str::to_string);
        }
        if missing.is_none() && l.starts_with("Kernel '") && l.ends_with("not found") {
            missing = Some(l.trim_end_matches('.').to_string());
        }
        if l.starts_with("Exception:") || l.starts_with("ArgumentException:") {
            render_errors += 1;
        }
        if let Some(rest) = l.strip_prefix("Forcing GfxDevice:") {
            info.graphics = Some(rest.trim().to_string());
            info.forced = true;
        } else if info.graphics.is_none() {
            if let Some(rest) = l.strip_prefix("Version:") {
                // "Version:  Direct3D 11.0 [level 11.1]"
                let v = rest.trim();
                if v.starts_with("Direct3D 11") {
                    info.graphics = Some("Direct3D 11".into());
                } else if v.starts_with("Direct3D 12") {
                    info.graphics = Some("Direct3D 12".into());
                } else if v.starts_with("Vulkan") {
                    info.graphics = Some("Vulkan".into());
                }
            }
        }
        let lower = l.to_ascii_lowercase();
        if (lower.starts_with("frame generation:") && !lower.contains("off"))
            || lower.contains("framegen")
        {
            info.frame_generation = true;
        }
        if info.version.is_none() && !game_name.is_empty() {
            if let Some(rest) = l.strip_prefix(game_name) {
                let v = rest.split_whitespace().next().unwrap_or("");
                if v.chars().next().is_some_and(|c| c.is_ascii_digit())
                    && v.contains('.')
                    && v.len() <= 20
                {
                    info.version = Some(v.to_string());
                }
            }
        }
    }
    if info.forced {
        let mode = info.graphics.clone().unwrap_or_default();
        info.mode_broken = match (missing, render_errors) {
            (Some(m), _) => Some(format!("The game logged \"{m}\" right after starting in {mode}: its build doesn't include everything {mode} needs, so it can't draw (black screen).")),
            (None, n) if n >= 50 => Some(format!("The game logged the same rendering error {n} times in {mode}, so it can't draw properly in that mode.")),
            _ => None,
        };
    }
    info
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AntiCheat {
    /// "Easy Anti-Cheat", "BattlEye".
    pub kind: String,
    /// Known to block Linux players (at least online). Otherwise it's just present and may or may
    /// not be enabled for Linux by the developer.
    pub blocks_linux: bool,
}

/// Games whose anti-cheat is known to block Linux/Proton players online. Kept short on purpose:
/// only long-standing, well-known cases. Check areweanticheatyet.com for the current state.
const BLOCKED: &[(&str, &str)] = &[
    ("578080", "PUBG: BATTLEGROUNDS"),
    ("359550", "Tom Clancy's Rainbow Six Siege"),
    ("1085660", "Destiny 2"),
    ("271590", "Grand Theft Auto V"),
    ("1517290", "Battlefield 2042"),
];

pub fn anti_cheat(app_id: &str, install_dir: &Path) -> Option<AntiCheat> {
    let blocked = BLOCKED.iter().any(|(id, _)| *id == app_id);
    let has = |names: &[&str]| -> bool {
        std::fs::read_dir(install_dir)
            .map(|rd| {
                rd.flatten().any(|e| {
                    let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                    names.iter().any(|x| n.starts_with(x))
                })
            })
            .unwrap_or(false)
    };
    let kind = if has(&["easyanticheat", "eac_"]) {
        Some("Easy Anti-Cheat")
    } else if has(&["battleye", "be_"]) {
        Some("BattlEye")
    } else {
        None
    };
    match (kind, blocked) {
        (Some(k), b) => Some(AntiCheat {
            kind: k.into(),
            blocks_linux: b,
        }),
        (None, true) => Some(AntiCheat {
            kind: "anti-cheat".into(),
            blocks_linux: true,
        }),
        (None, false) => None,
    }
}

/// How a game that runs fine is set up, to compare with one that doesn't.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkingGame {
    pub app_id: String,
    pub name: String,
    /// How long its longest recent clean session lasted, in minutes.
    pub minutes: i64,
    /// Compatibility tool folder from Steam's log; None for native Linux games.
    pub tool: Option<String>,
    /// Its own launch options in Steam (without CachyPlay's part).
    pub launch_options: String,
}

/// Games other than `app_id` that ran at least `min_minutes` without a GPU crash in the last
/// 30 days, longest first, one entry per game.
pub fn working_games(
    sessions: &[Session],
    crashes: &[&CrashEvent],
    app_id: &str,
    min_minutes: i64,
    now: crate::clock::DateTime,
) -> Vec<(String, i64, Option<String>)> {
    let mut best: std::collections::BTreeMap<String, (i64, Option<String>)> = Default::default();
    for s in sessions {
        if s.app_id == app_id || crate::clock::between(s.start, now) > crate::clock::days(30) {
            continue;
        }
        let Some(d) = s.duration() else { continue };
        let mins = d.as_mins();
        if mins < min_minutes {
            continue;
        }
        let crashed = crashes.iter().any(|c| {
            c.app_id.as_deref() == Some(s.app_id.as_str())
                && c.at >= s.start
                && s.end.is_none_or(|e| c.at <= e)
        });
        if crashed {
            continue;
        }
        let e = best.entry(s.app_id.clone()).or_insert((0, None));
        if mins > e.0 {
            *e = (mins, s.tool.clone());
        }
    }
    let mut v: Vec<(String, i64, Option<String>)> =
        best.into_iter().map(|(k, (m, t))| (k, m, t)).collect();
    v.sort_by_key(|g| std::cmp::Reverse(g.1));
    v
}

/// `NAME=value` settings in launch options.
pub fn env_settings(options: &str) -> Vec<(String, String)> {
    let words: Vec<&str> = options.split_whitespace().collect();
    let end = words
        .iter()
        .position(|w| *w == "%command%")
        .unwrap_or(words.len());
    words[..end]
        .iter()
        .filter_map(|w| {
            let (k, v) = w.split_once('=')?;
            (!k.is_empty()
                && k.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .then(|| (k.to_string(), v.trim_matches('"').to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn spots_a_graphics_mode_the_game_cant_use() {
        let log = "Initialize engine version: 6000.0.74f1 (7685f01dc6be)\nForcing GfxDevice: Direct3D 12\nKernel 'MainUploadScatterInstances' not found\nException: Blitter is already initialized.\n";
        let u = unity_info(log, "SHRED A BILLION");
        assert_eq!(u.engine.as_deref(), Some("6000.0.74f1"));
        let why = u.mode_broken.unwrap();
        assert!(
            why.contains("MainUploadScatterInstances") && why.contains("Direct3D 12"),
            "{why}"
        );
        // The game's own default mode with no errors: fine.
        assert!(unity_info("Version:  Direct3D 11.0 [level 11.1]\n", "x")
            .mode_broken
            .is_none());
    }

    use super::*;

    #[test]
    fn reads_moon_sweepers_player_log() {
        let log =
            "Forcing GfxDevice: Direct3D 11\nDirect3D:\n    Version:  Direct3D 11.0 [level 11.1]\n\
                   Moon Sweepers 1.0.6 (917740c2, built 2026-11-15 11:26 UTC)\n\
                   Frame generation: this graphics card can run FSR, XeSS; FSR is the one used\n";
        let i = unity_info(log, "Moon Sweepers");
        assert_eq!(i.graphics.as_deref(), Some("Direct3D 11"));
        assert!(i.forced && i.frame_generation);
        assert_eq!(i.version.as_deref(), Some("1.0.6"));
    }

    #[test]
    fn unforced_d3d12_and_no_frame_gen() {
        let i = unity_info(
            "Direct3D:\n    Version:  Direct3D 12 [level 12.1]\nFrame generation: off\n",
            "X",
        );
        assert_eq!(i.graphics.as_deref(), Some("Direct3D 12"));
        assert!(!i.forced && !i.frame_generation);
    }

    #[test]
    fn finds_anti_cheat_folders() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(anti_cheat("1", tmp.path()), None);
        std::fs::create_dir(tmp.path().join("EasyAntiCheat")).unwrap();
        let a = anti_cheat("1", tmp.path()).unwrap();
        assert_eq!(a.kind, "Easy Anti-Cheat");
        assert!(!a.blocks_linux);
        assert!(anti_cheat("578080", tmp.path()).unwrap().blocks_linux);
    }

    #[test]
    fn env_settings_only_before_command() {
        assert_eq!(
            env_settings("PROTON_ENABLE_NVAPI=1 DXVK_HUD=fps %command% FOO=1"),
            vec![
                ("PROTON_ENABLE_NVAPI".into(), "1".into()),
                ("DXVK_HUD".into(), "fps".into())
            ]
        );
    }
}
