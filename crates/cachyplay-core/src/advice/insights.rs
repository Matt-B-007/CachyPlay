// SPDX-License-Identifier: GPL-3.0-or-later
//! Everything the game page shows, gathered from the PC, and applying a suggestion.

use super::*;

/// Everything the game page shows, worked out in one go.
#[derive(Debug, Clone, Serialize)]
pub struct Insights {
    pub suggestions: Vec<Suggestion>,
    pub summary: Summary,
    /// How this game is set up, next to games that run fine.
    pub this_setup: crate::insights::WorkingGame,
    pub this_graphics: Option<String>,
    pub working: Vec<crate::insights::WorkingGame>,
    pub anti_cheat: Option<crate::insights::AntiCheat>,
    pub developer_report: String,
    pub discussions_url: String,
    /// Prefilled "this fix worked" links, by suggestion id, for fixes that worked.
    pub share_urls: BTreeMap<String, String>,
    /// Earlier copies of the game's Windows files kept by "Reset".
    pub kept_prefixes: usize,
    /// Space the kept copies use, e.g. "3.2 GB".
    pub kept_size: Option<String>,
    pub has_prefix: bool,
    /// Status steps (testing / seems to work / works with a fix) and the clean-run target.
    pub progress: crate::progress::Progress,
    /// Starts, newest first, with dividers for fixes and updates.
    pub timeline: Vec<crate::progress::Item>,
    /// "Find the cause": tests in order, and what's been ruled out.
    pub find_cause: crate::triage::FindCause,
    /// Vulkan add-ons installed on this PC, and whether each actually loads into games.
    pub layers: Vec<LayerInfo>,
    /// Text for NVIDIA's forum, when the NVIDIA report is suggested.
    pub nvidia_report: Option<String>,
    pub shader_cache_aside: bool,
    /// Everything CachyPlay applies to this game right now, each with its own Undo.
    pub applied: Vec<AppliedItem>,
    /// Changes made since the game last started (to warn before stacking another).
    pub changed_since_start: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppliedItem {
    /// "fix", "settings", "proton", "shader_cache"
    pub kind: &'static str,
    pub id: String,
    pub label: String,
    /// "Nov 25 21:31"
    pub since: String,
    /// The game's log or a crash says it made things worse.
    pub worse: Option<String>,
    pub temporary: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayerInfo {
    pub name: String,
    pub what: String,
    /// Loads into every Vulkan game unless turned off.
    pub active: bool,
    /// Ships with Steam, Mesa, the NVIDIA driver or gamescope.
    pub trusted: bool,
}

/// Games here that run once NVIDIA's raw access chains are off: the fix is on, they've played 2+
/// minutes since, and they haven't crashed since.
pub(crate) fn nvidia_switch_helped(ctx: &crate::manage::Ctx, events: &[CrashEvent]) -> Vec<String> {
    let launches = crate::evidence::all_launches(&ctx.paths);
    let mut out = Vec::new();
    for app in crate::fixes::games_with_fixes(&ctx.paths) {
        let Some(f) = crate::fixes::load(&ctx.paths, &app)
            .fixes
            .into_iter()
            .find(|f| f.id == "nvidia_raw_access_chains")
        else {
            continue;
        };
        let Some(on) = crate::clock::wall(&f.applied_at) else {
            continue;
        };
        let played = launches
            .iter()
            .any(|l| l.app_id == app && l.started_at > f.applied_at && l.runtime_secs >= 120.0);
        let crashed = events
            .iter()
            .any(|e| e.app_id.as_deref() == Some(app.as_str()) && e.at > on);
        if played && !crashed {
            out.push(ctx.steam.find_game(&app).map(|g| g.name).unwrap_or(app));
        }
    }
    out
}

pub(crate) fn unity_of(
    ctx: &crate::manage::Ctx,
    game: &crate::steam::Game,
) -> Option<crate::insights::UnityInfo> {
    ctx.steam
        .unity_logs(game)
        .first()
        .and_then(|p| crate::fsutil::read_capped(p, 400, 200))
        .map(|t| crate::insights::unity_info(&t, &game.name))
}

/// Fixes that worked for a *similar* game here: (game name, fix id, why they're similar). One fix
/// rarely fixes every game: a graphics-mode fix only carries over between games on the same
/// Unity version and default graphics mode, and a translation-layer fix only between games using
/// the same graphics mode and hitting the same NVIDIA error. Anything else isn't mentioned.
pub(crate) fn worked_elsewhere(
    ctx: &crate::manage::Ctx,
    game: &crate::steam::Game,
) -> Vec<(String, String, String)> {
    let paths = &ctx.paths;
    let launches = crate::evidence::all_launches(paths);
    let now = crate::clock::now_local();
    let events =
        crate::evidence::crash_events(paths, Some(ctx), &crate::evidence::kernel_logs(), now);
    let hangs = |app: &str| {
        events.iter().any(|e| {
            e.app_id.as_deref() == Some(app) && e.faults.iter().any(|f| f.xid == Some(109))
        })
    };
    let me = unity_of(ctx, game);
    let major = |u: &crate::insights::UnityInfo| {
        u.engine
            .as_deref()
            .map(|e| e.split('.').next().unwrap_or(e).to_string())
    };
    // The mode a game runs in by itself (before any forcing): a forced log doesn't say, so assume
    // Unity's default on Windows.
    let default_mode = |u: &crate::insights::UnityInfo| {
        if u.forced {
            "Direct3D 11".to_string()
        } else {
            u.graphics.clone().unwrap_or_default()
        }
    };
    let mut out = Vec::new();
    for app in crate::fixes::games_with_fixes(paths)
        .into_iter()
        .filter(|a| *a != game.app_id)
    {
        let Some(other) = ctx.steam.find_game(&app) else {
            continue;
        };
        let them = unity_of(ctx, &other);
        for f in crate::fixes::load(paths, &app)
            .fixes
            .into_iter()
            .filter(|f| !f.temporary && f.id != crate::fixes::YOUR_SETTINGS)
        {
            let ran = launches
                .iter()
                .filter(|l| l.app_id == app && l.started_at > f.applied_at && !l.running)
                .any(|l| l.runtime_secs >= history::PLAYED as f64);
            if !(f.verdict == Some(true) || (f.verdict.is_none() && ran)) {
                continue;
            }
            let similar = match f.id.as_str() {
                "unity_d3d12" | "unity_vulkan" => match (&me, &them) {
                    (Some(a), Some(b))
                        if major(a).is_some()
                            && major(a) == major(b)
                            && default_mode(a) == default_mode(b) =>
                    {
                        Some(format!(
                            "also Unity {}, same default graphics mode",
                            major(a).unwrap_or_default()
                        ))
                    }
                    _ => None,
                },
                "nvidia_raw_access_chains" => {
                    let mode =
                        |u: &Option<crate::insights::UnityInfo>| u.as_ref().map(default_mode);
                    (hangs(&game.app_id) && hangs(&app) && mode(&me) == mode(&them))
                        .then(|| "same NVIDIA Xid 109 hang, same graphics mode".to_string())
                }
                _ => None,
            };
            if let Some(why) = similar {
                out.push((other.name.clone(), f.id, why));
            }
        }
    }
    out
}

pub fn insights(ctx: &crate::manage::Ctx, game: &crate::steam::Game) -> Insights {
    let ev = gather(ctx, game, &crate::evidence::kernel_logs());
    let suggestions = suggest(&ev);
    // Remember what CachyPlay saw fail, so it stays ruled out after Undo.
    for s in suggestions
        .iter()
        .filter(|s| s.status == Status::DidntHelp && !s.try_again)
    {
        let active = ev
            .fixes
            .fixes
            .iter()
            .any(|f| Some(&f.id) == s.fix.as_ref().map(|x| &x.id));
        if active {
            let why = s
                .why
                .first()
                .cloned()
                .unwrap_or_else(|| "It crashed again with this on.".into());
            let _ = crate::tries::rule_out(&ctx.paths, &game.app_id, &s.id, &s.title, &why);
        }
    }
    let parse = |t: &str| crate::clock::wall(t);
    let mut applied: Vec<AppliedItem> = ev
        .fixes
        .fixes
        .iter()
        .map(|f| AppliedItem {
            kind: match f.id.as_str() {
                crate::fixes::YOUR_SETTINGS => "settings",
                "shader_cache" => "shader_cache",
                _ => "fix",
            },
            id: f.id.clone(),
            label: f.label.clone(),
            since: parse(&f.applied_at)
                .map(|d| d.strftime("%b %-d %H:%M").to_string())
                .unwrap_or_default(),
            worse: suggestions
                .iter()
                .find(|s| {
                    s.fix.as_ref().is_some_and(|x| x.id == f.id) && s.status == Status::DidntHelp
                })
                .and_then(|s| s.why.first().cloned()),
            temporary: f.temporary,
        })
        .collect();
    if let Some(tool) = &ev.switched_tool {
        applied.push(AppliedItem {
            kind: "proton",
            id: format!("proton:{tool}"),
            label: format!("Switch to {}", crate::compat::friendly(tool)),
            since: ev
                .switched_at
                .map(|d| d.strftime("%b %-d %H:%M").to_string())
                .unwrap_or_default(),
            worse: None,
            temporary: false,
        });
    }
    let last_start = ev
        .sessions
        .iter()
        .filter(|x| x.app_id == game.app_id)
        .map(|x| x.start)
        .max();
    let changed_since_start: Vec<String> = ev
        .tries
        .tries
        .iter()
        .filter(|t| {
            t.off_at.is_none() && t.on().is_some_and(|on| last_start.is_none_or(|s| on > s))
        })
        .map(|t| t.label.clone())
        .collect();
    let summary = summary(&ev, &suggestions);
    let (gpu, driver) = crate::outreach::gpu_and_driver();
    // "Share that this worked" once a fix has really proved itself: "Works with a fix", or the
    // player confirmed it.
    let prog = crate::progress::progress(&ev, ev.now());
    let proven = |s: &Suggestion| {
        let fixed = s.fix.as_ref();
        fixed.is_some_and(|f| {
            ev.fixes
                .fixes
                .iter()
                .any(|x| x.id == f.id && x.verdict == Some(true))
        }) || (prog.stage == crate::progress::Stage::WorksWithFix
            && fixed.is_some_and(|f| prog.headline.contains(&f.label)))
    };
    let share_urls = suggestions
        .iter()
        .filter(|s| s.status == Status::Worked && proven(s))
        .map(|s| {
            (
                s.id.clone(),
                crate::outreach::share_fix_url(&ev, s, driver.as_deref(), gpu.as_deref()),
            )
        })
        .collect();
    let last_tool = ev
        .sessions
        .iter()
        .rev()
        .find(|s| s.app_id == game.app_id)
        .and_then(|s| s.tool.clone());
    Insights {
        developer_report: crate::outreach::developer_report(&ev, driver.as_deref(), gpu.as_deref()),
        discussions_url: crate::outreach::discussions_url(&game.app_id),
        this_setup: crate::insights::WorkingGame {
            app_id: game.app_id.clone(),
            name: game.name.clone(),
            minutes: 0,
            tool: last_tool,
            launch_options: ev.user_options.clone(),
        },
        this_graphics: ev.unity.as_ref().and_then(|u| u.graphics.clone()),
        working: ev.working.clone(),
        anti_cheat: ev.anti_cheat.clone(),
        share_urls,
        kept_prefixes: crate::prefix::kept(game).len(),
        kept_size: {
            let k: u64 = crate::prefix::all_kept(std::slice::from_ref(&game.library))
                .iter()
                .filter(|x| x.1 == game.app_id)
                .map(|x| x.2)
                .sum();
            (k > 0).then(|| crate::prefix::human_size(k))
        },
        has_prefix: ctx.steam.compatdata(game).is_some(),
        find_cause: crate::triage::find_cause(&ev, &suggestions),
        layers: ev
            .layers
            .iter()
            .map(|l| LayerInfo {
                name: l.name.clone(),
                what: vklayers::friendly(l),
                active: l.active,
                trusted: l.trusted,
            })
            .collect(),
        nvidia_report: suggestions
            .iter()
            .any(|s| s.id == "report_nvidia")
            .then(|| {
                crate::triage::nvidia_report(
                    &ev,
                    driver.as_deref(),
                    gpu.as_deref(),
                    &crate::triage::ruled_out(&ev, &suggestions),
                )
            }),
        shader_cache_aside: crate::shadercache::has_aside(game),
        applied,
        changed_since_start,
        progress: crate::progress::progress(&ev, ev.now()),
        timeline: crate::progress::timeline(&ev, ev.now(), 10),
        summary,
        suggestions,
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
