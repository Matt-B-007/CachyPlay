// SPDX-License-Identifier: GPL-3.0-or-later
//! Keeping the settings that made a game work, without CachyPlay.
//!
//! CachyPlay applies a game's settings at launch time (from its fix file and the fixes you applied).
//! This module turns those into a plain Steam launch-options line, e.g.
//!
//! ```text
//! VK_LOADER_LAYERS_DISABLE=VK_LAYER_AFMF PROTON_ENABLE_NVAPI=1 game-performance %command% -nosplash
//! ```
//!
//! so you can see exactly what's active, copy it, save it to a file, or have CachyPlay write it into
//! Steam when you uninstall, and the game keeps running the same way.

use crate::launch_options::{self, shell_quote, COMMAND};
use crate::manage::{read_json, write_json, ChangeLog, Ctx, Snapshot};
use crate::paths::Paths;
use crate::recipes::{self, Facts};
use crate::steam::Game;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Setting {
    /// Plain-English name.
    pub label: String,
    /// What goes into the launch options (`NAME=value`, a wrapper, or a game argument).
    pub value: String,
    pub why: String,
    /// "fix file" or "your fix".
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameSettings {
    pub app_id: String,
    pub name: String,
    pub settings: Vec<Setting>,
    /// The Steam launch options that reproduce these settings without CachyPlay.
    pub launch_options: String,
}

/// Everything CachyPlay applies to a game at launch, excluding temporary diagnostic fixes.
pub fn settings_for(ctx: &Ctx, game: &Game, facts: &Facts) -> Vec<Setting> {
    let mut env: Vec<(String, String, String, String)> = Vec::new(); // name, value, why, source
    let mut args: Vec<Setting> = Vec::new();
    let mut gp: Option<Setting> = None;
    if let Ok(Some(r)) = recipes::find(&game.app_id, &ctx.paths.config, &ctx.paths.recipes()) {
        let plan = recipes::plan(&r, facts);
        for (k, v, why) in plan.env {
            env.push((k, v, why, "fix file".into()));
        }
        if plan.game_performance {
            let installed = crate::sys::which("game-performance").is_some();
            gp = Some(Setting {
                label: "CachyOS performance mode".into(),
                value: "game-performance".into(),
                why: if installed {
                    "Switches CachyOS to its low-latency gaming profile while the game runs".into()
                } else {
                    "Not installed on this PC, so it isn't in the launch options".into()
                },
                source: "fix file".into(),
            });
        }
        if !plan.extra_args.is_empty() {
            args.push(Setting {
                label: "Game arguments".into(),
                value: plan.extra_args.join(" "),
                why: "From the game's fix file".into(),
                source: "fix file".into(),
            });
        }
    }
    for f in crate::fixes::load(&ctx.paths, &game.app_id)
        .fixes
        .into_iter()
        .filter(|f| !f.temporary)
    {
        for (k, v) in &f.env {
            env.retain(|(name, ..)| name != k);
            env.push((
                k.clone(),
                v.clone(),
                "A fix you applied from a suggestion".into(),
                "your fix".into(),
            ));
        }
        if !f.args.is_empty() {
            args.push(Setting {
                label: f.label.clone(),
                value: f.args.join(" "),
                why: "A fix you applied".into(),
                source: "your fix".into(),
            });
        }
    }
    // Two graphics chips: the settings that keep the game on the graphics card.
    if crate::gpus::prefer_card_for(&ctx.paths, &game.app_id) {
        let gpus = crate::gpus::list(&crate::pccheck::Root::system());
        for (k, v) in crate::gpus::prefer_card_env(&gpus) {
            if !env.iter().any(|(name, ..)| *name == k) {
                env.push((
                    k,
                    v,
                    "Makes the game use the graphics card, not the built-in chip".into(),
                    "your setting".into(),
                ));
            }
        }
    }
    let obs =
        (crate::obs::enabled_for(&ctx.paths, &game.app_id) && crate::obs::installed()).then(|| {
            Setting {
            label: "OBS game capture".into(),
            value: crate::obs::TOOL.into(),
            why: "Lets OBS's Game Capture source record just this game instead of the whole screen"
                .into(),
            source: "your setting".into(),
        }
        });
    let mut out: Vec<Setting> = env
        .into_iter()
        .map(|(k, v, why, source)| Setting {
            label: friendly(&k),
            value: format!("{k}={v}"),
            why,
            source,
        })
        .collect();
    out.extend(obs);
    out.extend(gp);
    out.extend(args);
    out
}

fn friendly(k: &str) -> String {
    match k {
        "VK_LOADER_LAYERS_DISABLE" => "Vulkan add-ons off".into(),
        "PROTON_ENABLE_NVAPI" => "NVIDIA DLSS & Reflex".into(),
        "PROTON_ENABLE_WAYLAND" => "Native Wayland window".into(),
        other => other.into(),
    }
}

const WRAPPERS: [&str; 2] = ["game-performance", crate::obs::TOOL];

fn is_setting(tok: &str) -> bool {
    tok.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty()
            && k.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    })
}

/// Steam launch options that apply `settings` without CachyPlay, keeping the user's own options.
/// `current` may contain CachyPlay's launch helper; it is removed.
pub fn standalone_launch_options(
    current: &str,
    settings: &[Setting],
    have_game_performance: bool,
) -> String {
    let base = launch_options::strip(current);
    // NAME=value after %command% does nothing (Steam passes it to the game as text). Move it in
    // front, or drop it when it's already there.
    let base = match base.split_once(COMMAND) {
        Some((before, after)) if after.split_whitespace().any(is_setting) => {
            let mut front: Vec<String> = before.split_whitespace().map(str::to_string).collect();
            let mut rest: Vec<&str> = Vec::new();
            for tok in after.split_whitespace() {
                if is_setting(tok) {
                    let name = tok.split('=').next().unwrap_or("");
                    if !front.iter().any(|f| f.starts_with(&format!("{name}="))) {
                        front.push(tok.to_string());
                    }
                } else {
                    rest.push(tok);
                }
            }
            let mut b = front.join(" ");
            if !b.is_empty() {
                b.push(' ');
            }
            b.push_str(COMMAND);
            for r in rest {
                b.push(' ');
                b.push_str(r);
            }
            b
        }
        _ => base,
    };
    let base = if base.trim().is_empty() {
        COMMAND.to_string()
    } else if base.contains(COMMAND) {
        base
    } else {
        format!("{COMMAND} {}", base.trim())
    };
    let mut prefix: Vec<String> = Vec::new();
    for s in settings
        .iter()
        .filter(|s| s.value.contains('=') && !s.value.starts_with('-'))
    {
        let (k, v) = s.value.split_once('=').unwrap_or((&s.value, ""));
        // Don't duplicate a variable the user already set themselves.
        if !base
            .split(COMMAND)
            .next()
            .unwrap_or("")
            .contains(&format!("{k}="))
        {
            prefix.push(format!("{k}={}", shell_quote(v)));
        }
    }
    // Programs that start the game (obs-gamecapture is only listed when it's installed).
    let wanted = |tool: &str| settings.iter().any(|s| s.value == tool) && !base.contains(tool);
    if wanted(crate::obs::TOOL) {
        prefix.push(crate::obs::TOOL.into());
    }
    if have_game_performance && wanted("game-performance") {
        prefix.push("game-performance".into());
    }
    let suffix: Vec<&str> = settings
        .iter()
        .filter(|s| !s.value.contains('=') && !WRAPPERS.contains(&s.value.as_str()))
        .map(|s| s.value.as_str())
        .collect();
    let i = base.find(COMMAND).unwrap_or(0);
    let (before, after) = base.split_at(i);
    let mut out = String::new();
    out.push_str(before);
    for p in &prefix {
        out.push_str(p);
        out.push(' ');
    }
    out.push_str(after);
    for a in suffix {
        if !after.contains(a) {
            out.push(' ');
            out.push_str(a);
        }
    }
    out
}

/// What "Keep this fix in Steam" changed, so Undo can put it back exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeptInSteam {
    pub app_id: String,
    /// Steam's launch options before (usually with CachyPlay's launch helper).
    pub before: String,
    /// The launch options written: the fix, without CachyPlay.
    pub after: String,
    pub at: String,
    pub snapshot: Option<Snapshot>,
    pub changes: Option<ChangeLog>,
}

fn kept_path(paths: &Paths, app_id: &str) -> PathBuf {
    paths
        .data
        .join("kept-in-steam")
        .join(format!("{app_id}.json"))
}

pub fn kept_in_steam(paths: &Paths, app_id: &str) -> Option<KeptInSteam> {
    read_json(&kept_path(paths, app_id))
}

/// Where a game's fix lives, for "Where the fix lives" on its page.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FixHome {
    /// CachyPlay adds the fix when the game starts (CachyPlay is on for it).
    pub by_cachyplay: bool,
    /// The fix is in Steam's own launch options, so it works with CachyPlay off.
    pub in_steam: bool,
    /// "Keep this fix in Steam" was used and Steam's launch options haven't changed since.
    pub can_undo: bool,
    /// Steam's launch options after "Keep this fix in Steam".
    pub launch_options: String,
}

/// Whether one setting is already in the player's own launch options (`-dx12` counts for
/// CachyPlay's `-force-d3d12`). Programs that aren't installed are left out, as when writing them.
fn in_options(own: &str, s: &Setting, have_game_performance: bool) -> bool {
    let (front, back) = own.split_once(COMMAND).unwrap_or((own, ""));
    if let Some((k, v)) = s
        .value
        .split_once('=')
        .filter(|_| !s.value.starts_with('-'))
    {
        return front.split_whitespace().any(|t| {
            t.split_once('=')
                .is_some_and(|(tk, tv)| tk == k && tv.trim_matches(['\'', '"']) == v)
        });
    }
    if WRAPPERS.contains(&s.value.as_str()) {
        return front.split_whitespace().any(|t| t == s.value)
            || (s.value == "game-performance" && !have_game_performance);
    }
    back.split_whitespace()
        .any(|t| crate::progress::same_option(t, &s.value))
}

pub fn fix_home(
    ctx: &Ctx,
    game: &Game,
    settings: &[Setting],
    have_game_performance: bool,
) -> Option<FixHome> {
    if settings.is_empty() {
        return None;
    }
    let current = ctx
        .launch_options(&game.app_id)
        .ok()
        .flatten()
        .unwrap_or_default();
    let own = launch_options::strip(&current);
    Some(FixHome {
        by_cachyplay: launch_options::is_applied(&current),
        in_steam: settings
            .iter()
            .all(|s| in_options(&own, s, have_game_performance)),
        // Still undoable after CachyPlay was switched back on: only its own part was added.
        can_undo: kept_in_steam(&ctx.paths, &game.app_id)
            .is_some_and(|k| k.after.trim() == own.trim()),
        launch_options: standalone_launch_options(&current, settings, have_game_performance),
    })
}

/// Write the game's fix into Steam's own launch options and turn CachyPlay off for it, so it keeps
/// working without CachyPlay. Steam must be closed. Undo puts everything back.
pub fn keep_in_steam(
    ctx: &Ctx,
    game: &Game,
    settings: &[Setting],
    have_game_performance: bool,
) -> Result<String> {
    if settings.is_empty() {
        bail!("{} has no fix to keep.", game.name);
    }
    ctx.require_editable()?;
    let current = ctx.launch_options(&game.app_id)?.unwrap_or_default();
    let after = standalone_launch_options(&current, settings, have_game_performance);
    let record = KeptInSteam {
        app_id: game.app_id.clone(),
        before: current,
        after: after.clone(),
        at: crate::manage::now(),
        snapshot: ctx.snapshot(&game.app_id),
        changes: ctx.change_log(&game.app_id),
    };
    let path = kept_path(&ctx.paths, &game.app_id);
    write_json(&path, &record)?;
    if let Err(e) = ctx.keep_settings(&game.app_id, &after) {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(format!(
        "{}'s fix is in Steam's launch options now, so it keeps working with CachyPlay off.",
        game.name
    ))
}

/// Undo "Keep this fix in Steam": Steam's launch options and CachyPlay's records go back to how
/// they were. Refuses if the launch options were changed in Steam since.
pub fn undo_keep_in_steam(ctx: &Ctx, game: &Game) -> Result<String> {
    ctx.require_editable()?;
    let path = kept_path(&ctx.paths, &game.app_id);
    let k = kept_in_steam(&ctx.paths, &game.app_id)
        .ok_or_else(|| anyhow!("There's nothing to undo for {}.", game.name))?;
    let current = ctx.launch_options(&game.app_id)?.unwrap_or_default();
    if launch_options::strip(&current).trim() != k.after.trim() {
        bail!(
            "{}'s launch options were changed in Steam since, so CachyPlay left them as they are. Use the CachyPlay switch to turn it back on.",
            game.name
        );
    }
    ctx.restore_managed(
        &game.app_id,
        &k.before,
        k.snapshot.as_ref(),
        k.changes.as_ref(),
    )?;
    let _ = std::fs::remove_file(&path);
    Ok(if launch_options::is_applied(&k.before) {
        format!(
            "Undone: CachyPlay adds {}'s fix again, and Steam's launch options are back as they were.",
            game.name
        )
    } else {
        format!(
            "Undone: {}'s launch options are back as they were.",
            game.name
        )
    })
}

/// The same, with this PC's facts (for the window and the waiting-for-Steam queue).
pub fn keep_in_steam_here(ctx: &Ctx, game: &Game) -> Result<String> {
    let settings = settings_for(ctx, game, &Facts::detect());
    keep_in_steam(
        ctx,
        game,
        &settings,
        crate::sys::which("game-performance").is_some(),
    )
}

/// Every game CachyPlay manages or has fixes for, with its settings and standalone launch options.
pub fn all(ctx: &Ctx) -> Vec<GameSettings> {
    let facts = Facts::detect();
    let have_gp = crate::sys::which("game-performance").is_some();
    let mut ids = ctx.managed_apps();
    ids.extend(crate::fixes::games_with_fixes(&ctx.paths));
    ids.sort();
    ids.dedup();
    let mut out = Vec::new();
    for id in ids {
        let Some(game) = ctx.steam.find_game(&id) else {
            continue;
        };
        let settings = settings_for(ctx, &game, &facts);
        if settings.is_empty() {
            continue;
        }
        let current = ctx.launch_options(&id).ok().flatten().unwrap_or_default();
        out.push(GameSettings {
            launch_options: standalone_launch_options(&current, &settings, have_gp),
            app_id: id,
            name: game.name,
            settings,
        });
    }
    out.sort_by_key(|g| g.name.to_lowercase());
    out
}

/// A plain-text copy of every game's settings, for keeping or moving to another PC.
pub fn export_text(games: &[GameSettings]) -> String {
    let mut s = String::from("CachyPlay game settings\n=======================\n\n");
    s.push_str(&format!(
        "Saved {} by CachyPlay {}.\n\n",
        crate::clock::now().strftime("%Y-%m-%d %H:%M"),
        crate::VERSION
    ));
    s.push_str("To use these without CachyPlay: in Steam, right-click the game > Properties > General >\nLaunch options, and paste the line shown for that game.\n\n");
    if games.is_empty() {
        s.push_str("No game has CachyPlay settings right now.\n");
    }
    for g in games {
        s.push_str(&format!("{} (AppID {})\n", g.name, g.app_id));
        s.push_str(&format!("  Launch options:  {}\n", g.launch_options));
        for st in &g.settings {
            s.push_str(&format!(
                "  - {}: {}  ({}; {})\n",
                st.label, st.value, st.why, st.source
            ));
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn misplaced_settings_move_in_front_without_duplicates() {
        // Halo, Nov 25: "PROTON_ENABLE_NVAPI=1 ... %command% PROTON_ENABLE_NVAPI=1".
        let out = standalone_launch_options(
            "%command% PROTON_ENABLE_NVAPI=1",
            &[st("PROTON_ENABLE_NVAPI=1"), st("game-performance")],
            true,
        );
        assert_eq!(out, "PROTON_ENABLE_NVAPI=1 game-performance %command%");
        assert_eq!(out.matches("PROTON_ENABLE_NVAPI").count(), 1);
    }

    use super::*;

    #[test]
    fn keep_the_fix_in_steam_and_undo_it() {
        let f = crate::testutil::FakeSteam::new();
        let ctx = f.ctx();
        let game = ctx.steam.find_game("1240440").unwrap();
        ctx.apply(&game, std::path::Path::new("/usr/bin/cachyplay"))
            .unwrap();
        let wrapped = ctx.launch_options("1240440").unwrap().unwrap();
        let settings = [st("PROTON_ENABLE_NVAPI=1"), st("-force-d3d12")];
        let home = fix_home(&ctx, &game, &settings, false).unwrap();
        assert!(
            home.by_cachyplay && !home.in_steam && !home.can_undo,
            "{home:?}"
        );

        keep_in_steam(&ctx, &game, &settings, false).unwrap();
        let kept = ctx.launch_options("1240440").unwrap().unwrap();
        assert_eq!(
            kept,
            "PROTON_ENABLE_NVAPI=1 %command% -nosplash -force-d3d12"
        );
        assert!(!ctx.managed_apps().contains(&"1240440".to_string()));
        let home = fix_home(&ctx, &game, &settings, false).unwrap();
        assert!(
            !home.by_cachyplay && home.in_steam && home.can_undo,
            "{home:?}"
        );

        // Switching CachyPlay back on keeps Undo available, and Undo takes the fix out of Steam.
        ctx.apply(&game, std::path::Path::new("/usr/bin/cachyplay"))
            .unwrap();
        let home = fix_home(&ctx, &game, &settings, false).unwrap();
        assert!(
            home.by_cachyplay && home.in_steam && home.can_undo,
            "{home:?}"
        );
        undo_keep_in_steam(&ctx, &game).unwrap();
        assert_eq!(ctx.launch_options("1240440").unwrap().unwrap(), wrapped);
        assert!(ctx.managed_apps().contains(&"1240440".to_string()));
        assert!(kept_in_steam(&ctx.paths, "1240440").is_none());
        // Turning CachyPlay off later still restores the original options.
        ctx.undo("1240440").unwrap();
        assert_eq!(
            ctx.launch_options("1240440").unwrap().unwrap(),
            "%command% -nosplash"
        );
    }

    #[test]
    fn undo_leaves_launch_options_edited_in_steam_alone() {
        let f = crate::testutil::FakeSteam::new();
        let ctx = f.ctx();
        let game = ctx.steam.find_game("1240440").unwrap();
        keep_in_steam(&ctx, &game, &[st("-force-d3d12")], false).unwrap();
        ctx.keep_settings("1240440", "%command% -windowed").unwrap();
        assert!(undo_keep_in_steam(&ctx, &game).is_err());
        assert_eq!(
            ctx.launch_options("1240440").unwrap().unwrap(),
            "%command% -windowed"
        );
    }

    #[test]
    fn a_fix_typed_differently_in_steam_counts_as_there() {
        let own = "PROTON_ENABLE_NVAPI=1 %command% -dx12";
        assert!(in_options(own, &st("-force-d3d12"), false));
        assert!(in_options(own, &st("PROTON_ENABLE_NVAPI=1"), false));
        assert!(!in_options(own, &st("PROTON_ENABLE_NVAPI=0"), false));
        assert!(!in_options(own, &st("-force-vulkan"), false));
        assert!(
            in_options(own, &st("game-performance"), false),
            "not installed: nothing to add"
        );
        assert!(!in_options(own, &st("game-performance"), true));
    }

    fn st(value: &str) -> Setting {
        Setting {
            label: value.into(),
            value: value.into(),
            why: String::new(),
            source: "your fix".into(),
        }
    }

    #[test]
    fn builds_standalone_launch_options() {
        let wrapped = launch_options::apply(
            "PROTON_LOG=1 %command% -nosplash",
            std::path::Path::new("/usr/bin/cachyplay"),
        );
        let out = standalone_launch_options(
            &wrapped,
            &[
                st("VK_LOADER_LAYERS_DISABLE=VK_LAYER_AFMF,VK_LAYER_MAKO_render"),
                st("PROTON_ENABLE_NVAPI=1"),
                st("game-performance"),
                st("obs-gamecapture"),
                st("-screen-fullscreen 0"),
            ],
            true,
        );
        assert_eq!(
            out,
            "PROTON_LOG=1 VK_LOADER_LAYERS_DISABLE=VK_LAYER_AFMF,VK_LAYER_MAKO_render PROTON_ENABLE_NVAPI=1 obs-gamecapture game-performance %command% -nosplash -screen-fullscreen 0"
        );
        assert!(
            !launch_options::is_applied(&out),
            "no CachyPlay helper left"
        );
    }

    #[test]
    fn empty_and_args_only_options() {
        assert_eq!(
            standalone_launch_options("", &[st("PROTON_ENABLE_NVAPI=1")], false),
            "PROTON_ENABLE_NVAPI=1 %command%"
        );
        assert_eq!(
            standalone_launch_options("-novid", &[st("A_X=1")], false),
            "A_X=1 %command% -novid"
        );
        // game-performance is only written if it's installed.
        assert_eq!(
            standalone_launch_options("", &[st("game-performance")], false),
            "%command%"
        );
    }

    #[test]
    fn does_not_add_obs_twice() {
        assert_eq!(
            standalone_launch_options("obs-gamecapture %command%", &[st("obs-gamecapture")], false),
            "obs-gamecapture %command%"
        );
    }

    #[test]
    fn does_not_duplicate_user_variables() {
        let out = standalone_launch_options(
            "PROTON_ENABLE_NVAPI=0 %command%",
            &[st("PROTON_ENABLE_NVAPI=1")],
            false,
        );
        assert_eq!(
            out, "PROTON_ENABLE_NVAPI=0 %command%",
            "the user's own choice wins"
        );
    }
}
