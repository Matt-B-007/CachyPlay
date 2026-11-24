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
use crate::manage::Ctx;
use crate::recipes::{self, Facts};
use crate::steam::Game;
use serde::Serialize;

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
    let mut out: Vec<Setting> = env
        .into_iter()
        .map(|(k, v, why, source)| Setting {
            label: friendly(&k),
            value: format!("{k}={v}"),
            why,
            source,
        })
        .collect();
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

/// Steam launch options that apply `settings` without CachyPlay, keeping the user's own options.
/// `current` may contain CachyPlay's launch helper; it is removed.
pub fn standalone_launch_options(
    current: &str,
    settings: &[Setting],
    have_game_performance: bool,
) -> String {
    let base = launch_options::strip(current);
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
    if have_game_performance
        && settings.iter().any(|s| s.value == "game-performance")
        && !base.contains("game-performance")
    {
        prefix.push("game-performance".into());
    }
    let suffix: Vec<&str> = settings
        .iter()
        .filter(|s| !s.value.contains('=') && s.value != "game-performance")
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
        chrono::Local::now().format("%Y-%m-%d %H:%M"),
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
    use super::*;

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
                st("-screen-fullscreen 0"),
            ],
            true,
        );
        assert_eq!(
            out,
            "PROTON_LOG=1 VK_LOADER_LAYERS_DISABLE=VK_LAYER_AFMF,VK_LAYER_MAKO_render PROTON_ENABLE_NVAPI=1 game-performance %command% -nosplash -screen-fullscreen 0"
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
