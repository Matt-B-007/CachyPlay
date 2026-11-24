//! Fix suggestions, worked out on your PC from the evidence CachyPlay collects. No AI, no network:
//! each suggestion is a fixed rule ("GPU crashes in several games + an always-on Vulkan add-on →
//! try the game without that add-on"), and each one lists the evidence that triggered it.
//!
//! Suggestions that only touch one game (environment variables, game arguments) can be applied with
//! one click and undone any time. Suggestions that would change the whole system (drivers, kernels,
//! GPU clocks) are explained as steps, never done automatically.

use crate::crashmatch::{self, CrashEvent};
use crate::evidence::DriverUpdate;
use crate::fixes::{Fix, GameFixes};
use crate::vklayers::{self, Layer};
use crate::wrapper::LaunchRecord;
use chrono::NaiveDateTime;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Everything the rules look at, for one game.
#[derive(Debug, Clone, Default)]
pub struct Evidence {
    pub app_id: String,
    pub game_name: String,
    /// Steam's last update of the game.
    pub game_updated: Option<NaiveDateTime>,
    /// All GPU crashes (any game), newest first.
    pub events: Vec<CrashEvent>,
    /// AppID → game name, for crashes in other games.
    pub names: BTreeMap<String, String>,
    pub layers: Vec<Layer>,
    pub driver_updates: Vec<DriverUpdate>,
    /// Rule ids that matched in this game's own logs (Proton log, Unity Player.log).
    pub log_findings: Vec<String>,
    pub has_proton_log: bool,
    pub is_unity: bool,
    pub uses_proton: bool,
    pub launches: Vec<LaunchRecord>,
    pub fixes: GameFixes,
    pub cachyplay_on: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Not applied yet.
    New,
    /// Applied; no launch since.
    Applied,
    /// Applied, and the game has run normally since.
    Worked,
    /// Applied, but the game crashed again since.
    DidntHelp,
}

#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    pub id: String,
    pub title: String,
    /// The facts that triggered this suggestion, in plain English.
    pub why: Vec<String>,
    /// One-click, undoable change for this game only.
    pub fix: Option<Fix>,
    /// Steps for things CachyPlay won't change by itself (system-wide).
    pub steps: Vec<String>,
    pub status: Status,
    /// Lower comes first.
    pub priority: u8,
}

impl Evidence {
    fn game_events(&self) -> Vec<&CrashEvent> {
        self.events
            .iter()
            .filter(|e| e.app_id.as_deref() == Some(self.app_id.as_str()))
            .collect()
    }

    /// Names of the different programs that hit GPU errors.
    fn crashed_programs(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for e in &self.events {
            if !seen.insert(e.program_key()) {
                continue;
            }
            out.push(match &e.app_id {
                Some(id) => self
                    .names
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| format!("AppID {id}")),
                None => format!(
                    "a program named \"{}\"",
                    e.faults[0].process.clone().unwrap_or_default()
                ),
            });
        }
        out
    }

    fn status_of(&self, id: &str) -> Status {
        let Some(fix) = self.fixes.fixes.iter().find(|f| f.id == id) else {
            return Status::New;
        };
        let since: Vec<&LaunchRecord> = self
            .launches
            .iter()
            .filter(|l| l.started_at > fix.applied_at)
            .collect();
        let applied_at = chrono::DateTime::parse_from_rfc3339(&fix.applied_at)
            .ok()
            .map(|d| d.with_timezone(&chrono::Local).naive_local());
        let crashed_since = applied_at.is_some_and(|t| self.game_events().iter().any(|e| e.at > t));
        if crashed_since || since.iter().any(|l| l.failed) {
            Status::DidntHelp
        } else if since.iter().any(|l| !l.failed) {
            Status::Worked
        } else {
            Status::Applied
        }
    }
}

fn fix(id: &str, label: &str, env: &[(&str, &str)], args: &[&str], temporary: bool) -> Fix {
    Fix {
        id: id.into(),
        label: label.into(),
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        args: args.iter().map(|a| a.to_string()).collect(),
        applied_at: String::new(),
        temporary,
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

pub fn suggest(ev: &Evidence) -> Vec<Suggestion> {
    let mut out = Vec::new();
    let game_events = ev.game_events();
    let programs = ev.crashed_programs();
    let several = crashmatch::several_programs(&ev.events);
    let gpu_trouble = !game_events.is_empty()
        || several
        || ev
            .log_findings
            .iter()
            .any(|f| f == "gpu_reset" || f == "vulkan_present");
    let failed_launches = ev.launches.iter().filter(|l| l.failed).count();

    let crash_line = || -> Option<String> {
        if several {
            Some(format!(
                "The GPU driver crashed in {} different programs: {}.",
                programs.len(),
                programs.join(", ")
            ))
        } else if !game_events.is_empty() {
            Some(format!(
                "{} hit GPU driver errors {}.",
                ev.game_name,
                plural(game_events.len(), "time", "times")
            ))
        } else {
            None
        }
    };

    // 1. Third-party Vulkan add-ons loaded into every game.
    let suspects: Vec<&Layer> = ev.layers.iter().filter(|l| l.suspect()).collect();
    if gpu_trouble && !suspects.is_empty() {
        let mut why: Vec<String> = crash_line().into_iter().collect();
        why.push(format!(
            "These add-ons load into every game: {}.",
            suspects
                .iter()
                .map(|l| vklayers::friendly(l))
                .collect::<Vec<_>>()
                .join("; ")
        ));
        if several {
            why.push("Every crashing game had them loaded, so they're a prime suspect.".into());
        }
        out.push(Suggestion {
            id: "disable_vulkan_layers".into(),
            title: "Turn off the extra Vulkan add-ons for this game".into(),
            why,
            fix: Some(fix(
                "disable_vulkan_layers",
                "Vulkan add-ons off",
                &[(
                    "VK_LOADER_LAYERS_DISABLE",
                    &vklayers::disable_value(&suspects),
                )],
                &[],
                false,
            )),
            steps: vec![],
            status: ev.status_of("disable_vulkan_layers"),
            priority: if several { 10 } else { 20 },
        });
    }

    // 2. A graphics driver update shortly before system-wide trouble.
    if several {
        let gpu_driver: Vec<&DriverUpdate> = ev
            .driver_updates
            .iter()
            .filter(|u| u.package != "linux-cachyos")
            .collect();
        let mut why: Vec<String> = crash_line().into_iter().collect();
        if let Some(u) = gpu_driver.last() {
            why.push(format!(
                "{} was updated from {} to {} on {}.",
                u.package,
                u.from,
                u.to,
                u.at.format("%b %-d at %H:%M")
            ));
        }
        why.push("Problems that start in several games at once usually come from the driver, not the games.".into());
        out.push(Suggestion {
            id: "driver_rollback".into(),
            title: "Check whether a driver update caused this".into(),
            why,
            fix: None,
            steps: vec![
                "If the games worked before that update: restart and pick a snapshot from before it in the boot menu (CachyOS keeps them), or use Snapper/Timeshift.".into(),
                "If that fixes it, stay on the older driver until a newer one comes out, and report the problem on the NVIDIA developer forum with a CachyPlay bug report.".into(),
            ],
            status: Status::New,
            priority: 15,
        });
        out.push(Suggestion {
            id: "gpu_stock_clocks".into(),
            title: "Test with the GPU at stock settings".into(),
            why: vec!["Unstable GPU clocks or undervolting cause this kind of driver error in any game.".into()],
            fix: None,
            steps: vec!["If you use LACT, GreenWithEnvy or nvidia-settings to overclock or undervolt, reset to defaults and test again.".into()],
            status: Status::New,
            priority: 45,
        });
    }

    // 3. Only this game, and it updated recently.
    if !several && !game_events.is_empty() {
        let first = game_events.iter().map(|e| e.at).min();
        let mut why: Vec<String> = crash_line().into_iter().collect();
        if let (Some(up), Some(first)) = (ev.game_updated, first) {
            if up <= first && first - up <= chrono::Duration::days(30) {
                why.push(format!(
                    "The game updated on {}, before these crashes.",
                    up.format("%b %-d")
                ));
            }
        }
        why.push("No other game is crashing, so the problem is specific to this game.".into());
        let mut steps = vec![];
        if ev.uses_proton {
            steps.push("In Steam: right-click the game → Properties → Compatibility → tick \"Force the use…\" and try Proton Experimental, then GE-Proton.".into());
        }
        steps.push(
            "Report it to the game's developer and attach a CachyPlay bug report for this game."
                .into(),
        );
        out.push(Suggestion {
            id: "game_specific".into(),
            title: "Try a different Proton version, and tell the developer".into(),
            why,
            fix: None,
            steps,
            status: Status::New,
            priority: 25,
        });
    }

    // 4. Unity game failing to present frames: run in a window.
    if ev.is_unity && ev.log_findings.iter().any(|f| f == "vulkan_present") {
        out.push(Suggestion {
            id: "unity_windowed".into(),
            title: "Start the game in a window".into(),
            why: vec![format!(
                "{}'s Unity log shows a failure while putting frames on screen.",
                ev.game_name
            )],
            fix: Some(fix(
                "unity_windowed",
                "Start in a window",
                &[],
                &["-screen-fullscreen", "0"],
                false,
            )),
            steps: vec![],
            status: ev.status_of("unity_windowed"),
            priority: 30,
        });
    }

    // 5. Missing Windows runtime.
    if ev.log_findings.iter().any(|f| f == "missing_dll") {
        out.push(Suggestion {
            id: "install_runtime".into(),
            title: "Install the missing Windows runtime".into(),
            why: vec!["The Proton log shows a Windows library the game needs is missing.".into()],
            fix: None,
            steps: vec![format!(
                "Run: protontricks {} vcrun2022 (one-click install arrives in CachyPlay 0.2.0).",
                ev.app_id
            )],
            status: Status::New,
            priority: 20,
        });
    }

    // 6. Not enough information yet: capture a detailed log on the next launch.
    if (gpu_trouble || failed_launches > 0) && !ev.has_proton_log && ev.uses_proton {
        out.push(Suggestion {
            id: "capture_proton_log".into(),
            title: "Record a detailed log next time you play".into(),
            why: vec!["There's no Proton log yet, so the game's own view of the crash is missing.".into()],
            fix: Some(fix("capture_proton_log", "Detailed Proton log (temporary)", &[("PROTON_LOG", "1")], &[], true)),
            steps: vec!["After the next crash, save a bug report for this game, then undo this (the log gets big).".into()],
            status: ev.status_of("capture_proton_log"),
            priority: 40,
        });
    }

    out.sort_by_key(|s| s.priority);
    out
}

/// Gather the evidence for one game from this PC and work out suggestions.
pub fn for_game(ctx: &crate::manage::Ctx, game: &crate::steam::Game) -> Vec<Suggestion> {
    suggest(&gather(ctx, game, &crate::evidence::kernel_logs()))
}

pub fn gather(
    ctx: &crate::manage::Ctx,
    game: &crate::steam::Game,
    logs: &crate::evidence::KernelLogs,
) -> Evidence {
    let paths = &ctx.paths;
    let now = crashmatch::now_local();
    let events = crate::evidence::crash_events(paths, Some(ctx), logs, now);
    let names = ctx
        .steam
        .games()
        .into_iter()
        .map(|g| (g.app_id, g.name))
        .collect();
    let proton_log = crate::fsutil::read_capped(
        &paths.home.join(format!("steam-{}.log", game.app_id)),
        500,
        4000,
    );
    let unity_logs = ctx.steam.unity_logs(game);
    let mut log_findings: Vec<String> = Vec::new();
    for text in proton_log.iter().cloned().chain(
        unity_logs
            .iter()
            .filter_map(|p| crate::fsutil::read_capped(p, 300, 3000)),
    ) {
        log_findings.extend(
            crate::diagnose::scan(&text)
                .into_iter()
                .map(|f| f.rule.to_string()),
        );
    }
    log_findings.sort();
    log_findings.dedup();
    let launches: Vec<LaunchRecord> = crate::evidence::all_launches(paths)
        .into_iter()
        .filter(|l| l.app_id == game.app_id)
        .collect();
    Evidence {
        app_id: game.app_id.clone(),
        game_name: game.name.clone(),
        game_updated: game
            .last_updated
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            .map(|t| t.with_timezone(&chrono::Local).naive_local()),
        events,
        names,
        layers: vklayers::detect(&paths.home),
        driver_updates: crate::evidence::read_driver_updates(),
        log_findings,
        has_proton_log: proton_log.is_some(),
        is_unity: !unity_logs.is_empty(),
        uses_proton: ctx.steam.compatdata(game).is_some()
            || ctx.steam.compat_tool(&game.app_id).is_some(),
        launches,
        fixes: crate::fixes::load(paths, &game.app_id),
        cachyplay_on: ctx.status(game).map(|s| s.applied).unwrap_or(false),
    }
}

/// Apply a suggestion's one-click fix. CachyPlay must be on for the game (fixes are applied by its
/// launch helper); if it's off, it is turned on first, which needs Steam closed.
pub fn apply_suggestion(
    ctx: &crate::manage::Ctx,
    game: &crate::steam::Game,
    id: &str,
    bin: Option<&std::path::Path>,
) -> anyhow::Result<String> {
    let s = for_game(ctx, game)
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| anyhow::anyhow!("That suggestion no longer applies."))?;
    let fix = s.fix.ok_or_else(|| {
        anyhow::anyhow!("This suggestion has steps to follow, not a one-click fix.")
    })?;
    let mut turned_on = false;
    if !ctx.status(game).map(|st| st.applied).unwrap_or(false) {
        let bin =
            bin.ok_or_else(|| anyhow::anyhow!("Install CachyPlay first, then apply fixes."))?;
        ctx.apply(game, bin)
            .map_err(|e| anyhow::anyhow!("Fixes work through CachyPlay's launch helper, which needs to be turned on for this game first. {e:#}"))?;
        turned_on = true;
    }
    let label = fix.label.clone();
    crate::fixes::apply(&ctx.paths, &game.app_id, fix)?;
    Ok(if turned_on {
        format!("Turned CachyPlay on for {} and applied \"{label}\". Start the game from Steam to try it.", game.name)
    } else {
        format!(
            "Applied \"{label}\" to {}. Start the game from Steam to try it.",
            game.name
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crashmatch::{at, parse_kernel_faults, parse_steam_sessions};

    const LOG: &str = include_str!("../tests/fixtures/kernel-warnings-short.log");
    const CONSOLE: &str =
        "[2026-11-23 12:09:58] Game process added : AppID 5335510 \"x\", ProcID 1\n\
[2026-11-23 12:12:40] Game process removed: AppID 5335510 \"x\", ProcID 1\n\
[2026-11-23 18:25:10] Game process added : AppID 5335510 \"x\", ProcID 2\n\
[2026-11-23 18:27:30] Game process removed: AppID 5335510 \"x\", ProcID 2\n\
[2026-11-23 18:33:59] Game process added : AppID 5335510 \"x\", ProcID 3\n\
[2026-11-23 18:36:00] Game process removed: AppID 5335510 \"x\", ProcID 3\n\
[2026-11-23 18:37:15] Game process added : AppID 1240440 \"x\", ProcID 4\n";

    fn layer(name: &str, desc: &str, active: bool, trusted: bool) -> Layer {
        Layer {
            name: name.into(),
            description: desc.into(),
            manifest: String::new(),
            active,
            trusted,
        }
    }

    fn matt(log: &str) -> Evidence {
        let now = at(2026, 11, 23, 18, 45, 0);
        let events = crashmatch::crash_events(
            &parse_kernel_faults(log, now),
            &parse_steam_sessions(CONSOLE),
            now,
        );
        Evidence {
            app_id: "5335510".into(),
            game_name: "Moon Sweepers".into(),
            game_updated: Some(at(2026, 11, 21, 9, 0, 0)),
            events,
            names: [
                ("5335510".to_string(), "Moon Sweepers".to_string()),
                ("1240440".to_string(), "Halo Infinite".to_string()),
            ]
            .into(),
            layers: vec![
                layer(
                    "VK_LAYER_AFMF",
                    "AMD Fluid Motion Frames style frame generation for Linux",
                    true,
                    false,
                ),
                layer(
                    "VK_LAYER_MAKO_render",
                    "MAKO Renderer graphics layer",
                    true,
                    false,
                ),
                layer(
                    "VK_LAYER_MANGOHUD_overlay_x86_64",
                    "Vulkan Hud Overlay",
                    false,
                    false,
                ),
                layer(
                    "VK_LAYER_VALVE_steam_overlay_64",
                    "Steam Overlay Layer",
                    true,
                    true,
                ),
            ],
            driver_updates: vec![DriverUpdate {
                at: at(2026, 11, 20, 10, 0, 0),
                package: "nvidia-utils".into(),
                from: "620.60.01-1".into(),
                to: "620.73.04-1".into(),
            }],
            uses_proton: true,
            is_unity: true,
            ..Default::default()
        }
    }

    #[test]
    fn matts_case_recommends_turning_off_mako_and_afmf_first() {
        let s = suggest(&matt(LOG));
        let ids: Vec<&str> = s.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids[0], "disable_vulkan_layers", "{ids:?}");
        assert_eq!(ids[1], "driver_rollback");
        assert!(ids.contains(&"gpu_stock_clocks") && ids.contains(&"capture_proton_log"));
        assert!(
            !ids.contains(&"game_specific"),
            "several games crash, so it's not game-specific"
        );
        let top = &s[0];
        let env = &top.fix.as_ref().unwrap().env;
        assert_eq!(
            env["VK_LOADER_LAYERS_DISABLE"], "VK_LAYER_AFMF,VK_LAYER_MAKO_render",
            "only always-on third-party layers"
        );
        assert!(
            top.why[0].contains("2 different programs: Halo Infinite, Moon Sweepers"),
            "{:?}",
            top.why
        );
        assert!(s[1]
            .why
            .iter()
            .any(|w| w.contains("620.60.01-1 to 620.73.04-1")));
    }

    #[test]
    fn single_game_case_points_at_the_game() {
        let only_moon: String = LOG
            .lines()
            .filter(|l| !l.contains("00 MAIN"))
            .collect::<Vec<_>>()
            .join("\n");
        let s = suggest(&matt(&only_moon));
        let ids: Vec<&str> = s.iter().map(|x| x.id.as_str()).collect();
        assert!(
            ids.contains(&"game_specific") && !ids.contains(&"driver_rollback"),
            "{ids:?}"
        );
        let g = s.iter().find(|x| x.id == "game_specific").unwrap();
        assert!(
            g.why.iter().any(|w| w.contains("updated on Nov 21")),
            "{:?}",
            g.why
        );
    }

    #[test]
    fn no_crashes_no_noise() {
        let mut ev = matt("");
        ev.events.clear();
        assert!(suggest(&ev).is_empty());
    }

    #[test]
    fn tracks_whether_a_fix_worked() {
        let mut ev = matt(LOG);
        ev.fixes.fixes.push(Fix {
            id: "disable_vulkan_layers".into(),
            label: "x".into(),
            env: Default::default(),
            args: vec![],
            applied_at: "2026-11-23T18:50:00-08:00".into(),
            temporary: false,
        });
        assert_eq!(suggest(&ev)[0].status, Status::Applied);
        let rec = |t: &str, failed| LaunchRecord {
            app_id: "5335510".into(),
            started_at: t.into(),
            runtime_secs: 3600.0,
            exit_code: Some(0),
            signal: None,
            failed,
            passthrough: false,
            applied: vec![],
            skipped: vec![],
            prep_error: None,
        };
        ev.launches = vec![rec("2026-11-23T18:55:00-08:00", false)];
        assert_eq!(suggest(&ev)[0].status, Status::Worked);
        ev.launches.push(rec("2026-11-23T20:00:00-08:00", true));
        assert_eq!(suggest(&ev)[0].status, Status::DidntHelp);
    }
}
