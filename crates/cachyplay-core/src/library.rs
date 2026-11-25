//! What the GUI (and CLI) show: game summaries with one status badge each, and a detail view
//! with the before/after changes table. Plain data, ready to serialize.

use crate::crashmatch::CrashEvent;
use crate::manage::Ctx;
use crate::paths::{self, Paths};
use crate::recipes::{self, Facts, Recipe};
use crate::steam::{self, Game, InstallKind};
use crate::sys;
use crate::wrapper::{recent_launches, LaunchRecord};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Badge {
    /// The last launch through CachyPlay failed; a diagnosis is waiting.
    LastFailed,
    /// The GPU driver crashed while this game was running (from the kernel log).
    Crashed,
    /// CachyPlay is on for this game.
    Ready,
    /// A fix file exists but CachyPlay is off.
    FixAvailable,
    /// The user changed launch options or game files themselves.
    YourTweaks,
    /// Nothing special.
    Off,
}

#[derive(Debug, Clone, Serialize)]
pub struct SteamInfo {
    pub kind: String,
    pub root: String,
    pub running: bool,
    pub account_id: String,
    pub persona: String,
    /// Why games can't be changed right now, in plain English (None = editable).
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameSummary {
    pub app_id: String,
    pub name: String,
    pub enabled: bool,
    pub has_fix: bool,
    pub fix_tested: bool,
    pub user_tweaks: bool,
    pub last_failed: bool,
    /// GPU driver crashes while this game ran (this boot and the previous one).
    pub gpu_crashes: usize,
    pub anti_cheat: Option<String>,
    pub badge: Badge,
    /// Traffic light for the game list: green runs fine, yellow works with a fix, red broken,
    /// grey not played lately.
    pub light: crate::advice::Light,
}

#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub version: String,
    /// Path of the installed `cachyplay` command Steam will call, if installed.
    pub installed_cli: Option<String>,
    pub steam: Option<SteamInfo>,
    pub steam_error: Option<String>,
    pub games: Vec<GameSummary>,
    /// Set when GPU driver crashes hit several different programs (a driver or system problem).
    pub system_crashes: Option<SystemCrashes>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemCrashes {
    pub crashes: usize,
    /// Names of the programs that crashed (game names, or the process name if unknown).
    pub programs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrashInfo {
    /// "18:34" today, else "Nov 22 18:34".
    pub when: String,
    pub what: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangeRow {
    pub setting: String,
    /// The technical name (env var, flag), shown small under the plain label.
    pub technical: Option<String>,
    pub stock: String,
    pub cachyplay: String,
    pub why: String,
    /// "active" (applied when CachyPlay is on), "later" (needs a newer CachyPlay).
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineNode {
    pub label: String,
    /// "ok", "warn", "bad" or "neutral".
    pub state: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FixInfo {
    pub name: String,
    pub tested: bool,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameDetail {
    pub summary: GameSummary,
    pub launch_options: String,
    pub user_options: String,
    pub edited_since: bool,
    pub tweaks: Vec<String>,
    pub fix: Option<FixInfo>,
    pub changes: Vec<ChangeRow>,
    pub pipeline: Vec<PipelineNode>,
    pub launches: Vec<LaunchRecord>,
    pub install_dir: String,
    /// Steam launch options that keep this game's settings without CachyPlay (None: nothing to keep).
    pub standalone_launch_options: Option<String>,
    /// GPU driver crashes while this game ran, newest first.
    pub crashes: Vec<CrashInfo>,
    /// OBS game capture for this game.
    pub obs: crate::obs::ObsInfo,
}

/// The installed `cachyplay` command (what Steam's launch options point to).
/// `CACHYPLAY_DEV_BIN` overrides it for development and tests.
pub fn installed_cli(paths: &Paths) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CACHYPLAY_DEV_BIN").map(PathBuf::from) {
        return sys::is_executable(&p).then_some(p);
    }
    paths::allowed_install_locations(paths)
        .into_iter()
        .find(|p| sys::is_executable(p))
}

pub fn steam_info(ctx: &Ctx) -> SteamInfo {
    let running = steam::is_running_in(&ctx.proc_root);
    let blocked_reason = if ctx.steam.kind == InstallKind::Flatpak {
        Some("Flatpak Steam isn't supported yet. CachyPlay won't change anything.".into())
    } else if let Some(why) = ctx.format_problem() {
        Some(why)
    } else if running {
        Some(
            "Close Steam to turn games on or off. Steam rewrites its settings when it exits."
                .into(),
        )
    } else {
        None
    };
    SteamInfo {
        kind: match ctx.steam.kind {
            InstallKind::Native => "native".into(),
            InstallKind::Flatpak => "flatpak".into(),
        },
        root: ctx.steam.root.display().to_string(),
        running,
        account_id: ctx.account.account_id.clone(),
        persona: ctx.account.persona.clone(),
        blocked_reason,
    }
}

fn find_recipe(paths: &Paths, app_id: &str) -> Option<Recipe> {
    recipes::find(app_id, &paths.config, &paths.recipes())
        .ok()
        .flatten()
}

/// How many GPU crashes each game had (this boot and the previous one).
pub fn crash_counts(ctx: &Ctx) -> std::collections::BTreeMap<String, usize> {
    counts(&current_events(ctx))
}

fn current_events(ctx: &Ctx) -> Vec<crate::crashmatch::CrashEvent> {
    let logs = crate::evidence::kernel_logs();
    crate::evidence::crash_events(&ctx.paths, Some(ctx), &logs, crate::crashmatch::now_local())
}

fn counts(events: &[crate::crashmatch::CrashEvent]) -> std::collections::BTreeMap<String, usize> {
    let mut m = std::collections::BTreeMap::new();
    for e in events {
        if let Some(id) = &e.app_id {
            *m.entry(id.clone()).or_insert(0) += 1;
        }
    }
    m
}

fn system_crashes(ctx: &Ctx, events: &[crate::crashmatch::CrashEvent]) -> Option<SystemCrashes> {
    if !crate::crashmatch::several_programs(events) {
        return None;
    }
    let mut programs: Vec<String> = Vec::new();
    for e in events {
        let name = match &e.app_id {
            Some(id) => ctx
                .steam
                .find_game(id)
                .map(|g| g.name)
                .unwrap_or_else(|| format!("AppID {id}")),
            None => format!(
                "\"{}\"",
                e.faults[0]
                    .process
                    .clone()
                    .unwrap_or_else(|| "unknown".into())
            ),
        };
        if !programs.contains(&name) {
            programs.push(name);
        }
    }
    Some(SystemCrashes {
        crashes: events.len(),
        programs,
    })
}

/// Plain-English description of one crash event.
fn describe_crash(e: &crate::crashmatch::CrashEvent) -> String {
    let codes: Vec<u32> = e.faults.iter().filter_map(|f| f.xid).collect();
    if codes.contains(&109) && codes.contains(&31) {
        "Graphics work hung, then read missing GPU memory".into()
    } else if let Some(x) = codes.first() {
        let m = crate::crashmatch::xid_meaning(*x);
        let mut c = m.chars();
        c.next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or_default()
    } else {
        "The graphics driver reported a fault".into()
    }
}

fn crash_when(at: chrono::NaiveDateTime) -> String {
    if at.date() == crate::crashmatch::now_local().date() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%b %-d %H:%M").to_string()
    }
}

pub fn summary(ctx: &Ctx, game: &Game) -> GameSummary {
    summary_with(ctx, game, 0)
}

pub fn summary_with(ctx: &Ctx, game: &Game, gpu_crashes: usize) -> GameSummary {
    let status = ctx.status(game).ok();
    let recipe = find_recipe(&ctx.paths, &game.app_id);
    let enabled = status.as_ref().is_some_and(|s| s.applied);
    let user_tweaks = status
        .as_ref()
        .is_some_and(|s| !s.user_options.trim().is_empty() || !s.tweaks.is_empty());
    let last_failed = enabled
        && recent_launches(&ctx.paths, &game.app_id, 1)
            .first()
            .is_some_and(|l| l.failed);
    let has_fix = recipe.is_some();
    let badge = if last_failed {
        Badge::LastFailed
    } else if gpu_crashes > 0 {
        Badge::Crashed
    } else if enabled {
        Badge::Ready
    } else if has_fix {
        Badge::FixAvailable
    } else if user_tweaks {
        Badge::YourTweaks
    } else {
        Badge::Off
    };
    GameSummary {
        app_id: game.app_id.clone(),
        name: game.name.clone(),
        enabled,
        has_fix,
        fix_tested: recipe.as_ref().is_some_and(|r| r.tested),
        user_tweaks,
        last_failed,
        gpu_crashes,
        anti_cheat: recipe.and_then(|r| r.anti_cheat),
        light: if matches!(badge, Badge::LastFailed | Badge::Crashed) {
            crate::advice::Light::Red
        } else {
            crate::advice::Light::Grey
        },
        badge,
    }
}

/// Refine each game's light with Steam's session log (read once for the whole list).
fn set_lights(ctx: &Ctx, games: &mut [GameSummary], events: &[CrashEvent]) {
    use crate::advice::Light;
    use crate::history::{self, Outcome};
    let sessions = crate::crashmatch::parse_steam_sessions(&ctx.steam.console_logs());
    let crash_refs: Vec<&CrashEvent> = events.iter().collect();
    for g in games.iter_mut() {
        let fixes = crate::fixes::load(&ctx.paths, &g.app_id).fixes;
        // A fix applied after the last crash, and not judged useless: it's being tried.
        let newest_fix = fixes
            .iter()
            .filter(|f| f.verdict != Some(false) && !f.temporary)
            .filter_map(|f| chrono::DateTime::parse_from_rfc3339(&f.applied_at).ok())
            .map(|t| t.with_timezone(&chrono::Local).naive_local())
            .max();
        let last_crash = events
            .iter()
            .filter(|e| e.app_id.as_deref() == Some(g.app_id.as_str()))
            .map(|e| e.at)
            .max();
        if g.enabled && newest_fix.is_some_and(|f| last_crash.is_none_or(|c| f > c)) {
            g.light = Light::Yellow;
            continue;
        }
        if g.light == Light::Red {
            continue;
        }
        let runs = history::runs(&sessions, &g.app_id, &crash_refs);
        let has_fixes = !fixes.is_empty();
        g.light = if history::trouble(&runs).is_some() {
            Light::Red
        } else if has_fixes && g.enabled {
            Light::Yellow
        } else if runs
            .last()
            .is_some_and(|r| matches!(r.outcome, Outcome::Played | Outcome::Ok))
        {
            Light::Green
        } else {
            Light::Grey
        };
    }
}

pub fn overview(paths: &Paths, ctx: Result<&Ctx, String>) -> Overview {
    let installed_cli = installed_cli(paths).map(|p| p.display().to_string());
    match ctx {
        Ok(ctx) => {
            let events = current_events(ctx);
            Overview {
                version: crate::VERSION.into(),
                installed_cli,
                steam: Some(steam_info(ctx)),
                steam_error: None,
                games: {
                    let crashes = counts(&events);
                    let mut games: Vec<GameSummary> = ctx
                        .steam
                        .games()
                        .iter()
                        .map(|g| summary_with(ctx, g, crashes.get(&g.app_id).copied().unwrap_or(0)))
                        .collect();
                    set_lights(ctx, &mut games, &events);
                    games
                },
                system_crashes: system_crashes(ctx, &events),
            }
        }
        Err(e) => Overview {
            version: crate::VERSION.into(),
            installed_cli,
            steam: None,
            steam_error: Some(e),
            games: vec![],
            system_crashes: None,
        },
    }
}

pub fn detail(ctx: &Ctx, game: &Game, facts: &Facts) -> GameDetail {
    let status = ctx.status(game).ok();
    let recipe = find_recipe(&ctx.paths, &game.app_id);
    let mut changes = vec![ChangeRow {
        setting: "CachyPlay launch helper".into(),
        technical: Some("cachyplay run -- %command%".into()),
        stock: "Off".into(),
        cachyplay: "On".into(),
        why: "Applies this game's fixes and notices crashes. If anything goes wrong, the game starts normally.".into(),
        status: "active".into(),
    }];
    if let Some(r) = &recipe {
        let plan = recipes::plan(r, facts);
        for (k, v, why) in &plan.env {
            changes.push(ChangeRow {
                setting: friendly_env(k).unwrap_or(k).to_string(),
                technical: Some(format!("{k}={v}")),
                stock: "Not set".into(),
                cachyplay: v.clone(),
                why: why.clone(),
                status: "active".into(),
            });
        }
        if plan.game_performance {
            let have = sys::which("game-performance").is_some();
            let mut why = r
                .actions
                .iter()
                .find(|a| a.id == "use_game_performance")
                .map(|a| a.why.clone())
                .unwrap_or_default();
            if !have {
                why.push_str(
                    ". Install the game-performance package to use it; until then this is skipped.",
                );
            }
            changes.push(ChangeRow {
                setting: "CachyOS performance mode".into(),
                technical: Some("game-performance".into()),
                stock: "Off".into(),
                cachyplay: "On".into(),
                why,
                status: "active".into(),
            });
        }
        if !plan.extra_args.is_empty() {
            changes.push(ChangeRow {
                setting: "Game arguments".into(),
                technical: Some(plan.extra_args.join(" ")),
                stock: "None".into(),
                cachyplay: plan.extra_args.join(" "),
                why: r
                    .actions
                    .iter()
                    .find(|a| a.id == "add_game_arg")
                    .map(|a| a.why.clone())
                    .unwrap_or_default(),
                status: "active".into(),
            });
        }
        if let Some(runner) = r.runner.as_ref().filter(|r| !r.prefer.is_empty()) {
            changes.push(ChangeRow {
                setting: "Proton version".into(),
                technical: Some(format!("fallback order: {}", runner.prefer.join(" → "))),
                stock: "Steam's choice".into(),
                cachyplay: runner.prefer[0].clone(),
                why: "Coming in CachyPlay 0.2.0. Set it in Steam's game Properties for now.".into(),
                status: "later".into(),
            });
        }
        for a in r
            .actions
            .iter()
            .filter(|a| !recipes::SUPPORTED_ACTIONS.contains(&a.id.as_str()))
        {
            if a.when
                .as_deref()
                .is_some_and(|c| !facts.eval(c).unwrap_or(false))
            {
                continue;
            }
            changes.push(ChangeRow {
                setting: friendly_action(&a.id).into(),
                technical: Some(if a.args.is_empty() {
                    a.id.clone()
                } else {
                    format!("{} {}", a.id, a.args.join(" "))
                }),
                stock: "—".into(),
                cachyplay: "Planned".into(),
                why: format!("{} (coming in CachyPlay 0.2.0)", a.why),
                status: "later".into(),
            });
        }
    }

    for f in crate::fixes::load(&ctx.paths, &game.app_id).fixes {
        let technical: Vec<String> = f
            .env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .chain(f.args.iter().cloned())
            .collect();
        changes.push(ChangeRow {
            setting: f.label.clone(),
            technical: Some(technical.join(" ")),
            stock: "Off".into(),
            cachyplay: "On".into(),
            why: if f.temporary {
                "A fix you applied from a suggestion. Temporary: undo it once you've saved a bug report.".into()
            } else {
                "A fix you applied from a suggestion. Undo it any time under Suggested fixes.".into()
            },
            status: "fix".into(),
        });
    }
    let user_options = status
        .as_ref()
        .map(|s| s.user_options.clone())
        .unwrap_or_default();
    let obs = crate::obs::info(&ctx.paths, &game.app_id, &user_options);
    if obs.on {
        changes.push(ChangeRow {
            setting: "OBS game capture".into(),
            technical: Some("obs-gamecapture %command%".into()),
            stock: "Off".into(),
            cachyplay: "On".into(),
            why: if obs.installed {
                "OBS's Game Capture source records just this game instead of the whole screen."
                    .into()
            } else {
"Skipped until obs-vkcapture is installed (the command is in the OBS game capture box below)."
                    .into()
            },
            status: "fix".into(),
        });
    }
    let launches = recent_launches(&ctx.paths, &game.app_id, 5);
    let events = current_events(ctx);
    let crash_list: Vec<CrashInfo> = events
        .iter()
        .filter(|e| e.app_id.as_deref() == Some(game.app_id.as_str()))
        .map(|e| CrashInfo {
            when: crash_when(e.at),
            what: describe_crash(e),
        })
        .collect();
    let crashes = crash_list.len();
    let keepable = crate::keep::settings_for(ctx, game, facts);
    let standalone_launch_options = (!keepable.is_empty()).then(|| {
        let current = ctx
            .launch_options(&game.app_id)
            .ok()
            .flatten()
            .unwrap_or_default();
        crate::keep::standalone_launch_options(
            &current,
            &keepable,
            sys::which("game-performance").is_some(),
        )
    });
    GameDetail {
        standalone_launch_options,
        crashes: crash_list,
        summary: summary_with(ctx, game, crashes),
        launch_options: status
            .as_ref()
            .map(|s| s.launch_options.clone())
            .unwrap_or_default(),
        user_options,
        obs,
        edited_since: status.as_ref().is_some_and(|s| s.edited_since),
        tweaks: status.map(|s| s.tweaks).unwrap_or_default(),
        fix: recipe.as_ref().map(|r| FixInfo {
            name: r.name.clone(),
            tested: r.tested,
            notes: r.notes.clone(),
        }),
        changes,
        pipeline: pipeline(game, facts, launches.first()),
        launches,
        install_dir: game.install_dir.display().to_string(),
    }
}

fn pipeline(game: &Game, facts: &Facts, last: Option<&LaunchRecord>) -> Vec<PipelineNode> {
    let game_node = if !game.install_dir.is_dir() {
        node(
            "Game",
            "bad",
            "Install folder is missing. Verify the game files in Steam.",
        )
    } else if let Some(w) = risky_filesystem(&game.install_dir) {
        node("Game", "warn", &w)
    } else {
        match last {
            Some(l) if l.failed => node(
                "Game",
                "bad",
                &format!("Last launch closed after {:.0}s", l.runtime_secs),
            ),
            Some(_) => node("Game", "ok", "Last launch ran normally"),
            None => node("Game", "neutral", "Not launched through CachyPlay yet"),
        }
    };
    let vendor = facts.values.get("gpu.vendor").cloned().unwrap_or_default();
    let gpu = match vendor.as_str() {
        "nvidia" => node("GPU", "ok", "NVIDIA"),
        "amd" => node("GPU", "ok", "AMD"),
        "intel" => node("GPU", "ok", "Intel"),
        _ => node("GPU", "neutral", "Not detected"),
    };
    let session = facts.values.get("session").cloned().unwrap_or_default();
    let display = match session.as_str() {
        "wayland" => node("Display", "ok", "Wayland"),
        "x11" => node("Display", "ok", "X11"),
        _ => node("Display", "neutral", "Unknown session"),
    };
    vec![
        game_node,
        node("Proton", "neutral", "Chosen in Steam"),
        gpu,
        display,
    ]
}

fn node(label: &str, state: &str, detail: &str) -> PipelineNode {
    PipelineNode {
        label: label.into(),
        state: state.into(),
        detail: detail.into(),
    }
}

/// Warn when a game sits on NTFS/exFAT or a noexec mount (common silent killers of Proton games).
fn risky_filesystem(dir: &Path) -> Option<String> {
    let mounts = fs::read_to_string("/proc/self/mounts").ok()?;
    let dir = fs::canonicalize(dir).ok()?;
    let best = mounts
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (_, mnt, fstype, opts) = (it.next()?, it.next()?, it.next()?, it.next()?);
            let mnt = mnt.replace("\\040", " ");
            dir.starts_with(&mnt)
                .then(|| (mnt.len(), fstype.to_string(), opts.to_string()))
        })
        .max_by_key(|(len, _, _)| *len)?;
    let (_, fstype, opts) = best;
    if ["ntfs", "ntfs3", "fuseblk", "exfat", "vfat"].contains(&fstype.as_str()) {
        Some(format!("Installed on a {fstype} drive. Proton games often fail there; a Linux drive (ext4/btrfs) is safer."))
    } else if opts.split(',').any(|o| o == "noexec") {
        Some("Installed on a drive mounted with noexec. Games can't start from there.".into())
    } else {
        None
    }
}

fn friendly_env(k: &str) -> Option<&'static str> {
    Some(match k {
        "PROTON_ENABLE_NVAPI" => "NVIDIA DLSS & Reflex",
        "PROTON_ENABLE_WAYLAND" => "Native Wayland window",
        "PROTON_LOG" => "Proton logging",
        "PROTON_USE_WINED3D" => "OpenGL renderer (compatibility)",
        "DXVK_HDR" | "ENABLE_HDR_WSI" => "HDR output",
        "MANGOHUD" => "MangoHud overlay",
        _ => return None,
    })
}

fn friendly_action(id: &str) -> &'static str {
    match id {
        "winetricks" => "Windows runtime install",
        "dll_override" => "DLL override",
        "purge_shader_cache" => "Shader cache refresh",
        "patch_config" => "Game config change",
        "set_runner" => "Proton version",
        _ => "Other change",
    }
}

/// Local cover art from Steam's own cache (no network): portrait capsule, else header.
pub fn cover_art(steam_root: &Path, app_id: &str) -> Option<PathBuf> {
    let cache = steam_root.join("appcache/librarycache");
    for name in [
        format!("{app_id}_library_600x900.jpg"),
        format!("{app_id}_header.jpg"),
    ] {
        let p = cache.join(&name);
        if p.is_file() {
            return Some(p);
        }
    }
    // Newer Steam clients: librarycache/<appid>/[<hash>/]library_600x900.jpg
    let dir = cache.join(app_id);
    let mut found: Vec<PathBuf> = Vec::new();
    collect(&dir, 0, &mut found);
    for want in ["library_600x900", "library_capsule", "header"] {
        if let Some(p) = found.iter().find(|p| {
            p.file_stem()
                .is_some_and(|s| s.to_string_lossy().starts_with(want))
        }) {
            return Some(p.clone());
        }
    }
    None
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 2 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, depth + 1, out);
        } else if p.extension().is_some_and(|x| x == "jpg" || x == "png") {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::FakeSteam;

    #[test]
    fn badges_follow_state() {
        let f = FakeSteam::new();
        let ctx = f.ctx();
        let halo = ctx.steam.find_game("1240440").unwrap();
        let dota = ctx.steam.find_game("570").unwrap();
        // Halo has a fix file; the user already has -nosplash.
        assert_eq!(summary(&ctx, &halo).badge, Badge::FixAvailable);
        assert!(summary(&ctx, &halo).user_tweaks);
        assert_eq!(summary(&ctx, &dota).badge, Badge::Off);
        ctx.apply(&halo, Path::new("/usr/bin/cachyplay")).unwrap();
        assert_eq!(summary(&ctx, &halo).badge, Badge::Ready);
    }

    #[test]
    fn detail_lists_changes_with_reasons() {
        let f = FakeSteam::new();
        let ctx = f.ctx();
        let halo = ctx.steam.find_game("1240440").unwrap();
        let mut facts = Facts::default();
        facts.values.insert("gpu.vendor".into(), "nvidia".into());
        let d = detail(&ctx, &halo, &facts);
        let names: Vec<_> = d.changes.iter().map(|c| c.setting.as_str()).collect();
        assert!(names.contains(&"NVIDIA DLSS & Reflex"), "{names:?}");
        assert!(names.contains(&"Proton version"));
        assert!(d.changes.iter().all(|c| !c.why.is_empty()));
        assert_eq!(d.user_options, "%command% -nosplash");
        facts.values.insert("gpu.vendor".into(), "amd".into());
        assert!(!detail(&ctx, &halo, &facts)
            .changes
            .iter()
            .any(|c| c.setting == "NVIDIA DLSS & Reflex"));
    }

    #[test]
    fn finds_cover_art_in_both_layouts() {
        let f = FakeSteam::new();
        let root = f.home().join(".local/share/Steam");
        let old = root.join("appcache/librarycache/570_library_600x900.jpg");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, b"x").unwrap();
        assert_eq!(cover_art(&root, "570"), Some(old));
        let new = root.join("appcache/librarycache/1240440/abc123/library_600x900.jpg");
        fs::create_dir_all(new.parent().unwrap()).unwrap();
        fs::write(&new, b"x").unwrap();
        assert_eq!(cover_art(&root, "1240440"), Some(new));
    }
}
