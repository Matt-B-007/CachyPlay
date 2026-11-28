// SPDX-License-Identifier: GPL-3.0-or-later
//! The three-line summary (What happened / Why / What to do) and the game's traffic light.

use super::*;

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
    let proton_log = crate::evidence::proton_log(paths, &game.app_id)
        .and_then(|(p, _)| crate::fsutil::read_capped(&p, 500, 4000));
    let proton_layer = proton_log.as_deref().and_then(|l| {
        if l.contains("d3d12.dll") || l.contains("vkd3d") {
            Some("vkd3d".to_string())
        } else if l.contains("dxgi.dll") || l.contains("d3d11.dll") || l.contains("dxvk") {
            Some("dxvk".to_string())
        } else {
            None
        }
    });
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
    let unity = unity_logs
        .first()
        .and_then(|p| crate::fsutil::read_capped(p, 400, 400))
        .map(|t| crate::insights::unity_info(&t, &game.name));
    let sessions = crashmatch::parse_steam_sessions(&ctx.steam.console_logs());
    let events_for_switch = events.clone();
    let crash_refs: Vec<&CrashEvent> = events.iter().collect();
    let working = crate::insights::working_games(&sessions, &crash_refs, &game.app_id, 60, now)
        .into_iter()
        .take(4)
        .filter_map(|(id, minutes, tool)| {
            let g = ctx.steam.find_game(&id)?;
            let launch_options = ctx.status(&g).map(|s| s.user_options).unwrap_or_default();
            Some(crate::insights::WorkingGame {
                app_id: id,
                name: g.name,
                minutes,
                tool,
                launch_options,
            })
        })
        .collect();
    let launches: Vec<LaunchRecord> = crate::evidence::all_launches(paths)
        .into_iter()
        .filter(|l| l.app_id == game.app_id)
        .collect();
    let status = ctx.status(game).ok();
    Evidence {
        app_id: game.app_id.clone(),
        game_name: game.name.clone(),
        game_updated: game
            .last_updated
            // Steam stores the time in UTC; everything else here is the PC's own clock.
            .and_then(crate::clock::from_unix)
            .map(|t| t.datetime()),
        events,
        names,
        layers: vklayers::detect(&paths.home),
        driver_updates: crate::evidence::read_driver_updates(),
        log_findings,
        proton_layer,
        has_proton_log: proton_log.is_some(),
        is_unity: !unity_logs.is_empty(),
        uses_proton: ctx.steam.compatdata(game).is_some()
            || ctx.steam.compat_tool(&game.app_id).is_some(),
        launches,
        fixes: crate::fixes::load(paths, &game.app_id),
        cachyplay_on: status.as_ref().map(|s| s.applied).unwrap_or(false),
        sessions,
        user_options: status.map(|s| s.user_options).unwrap_or_default(),
        tools: crate::compat::installed(&ctx.steam.root, &ctx.steam.library_folders()),
        forced_tool: ctx.steam.compat_tool(&game.app_id),
        switched_tool: crate::compat::switched_to(ctx, &game.app_id),
        switched_at: crate::compat::switched_at(ctx, &game.app_id),
        notes: crate::notes::load(paths, &game.app_id),
        now: None,
        worked_elsewhere: worked_elsewhere(ctx, game),
        tries: crate::tries::load(paths, &game.app_id),
        nvidia_switch_helped: nvidia_switch_helped(ctx, &events_for_switch),
        unity,
        anti_cheat: crate::insights::anti_cheat(&game.app_id, &game.install_dir),
        working,
    }
}

/// Green: runs fine. Yellow: works with a fix, or not sure yet. Red: broken right now. Grey:
/// not played since CachyPlay has been watching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Light {
    Green,
    Yellow,
    Red,
    Grey,
}

/// The three lines at the top of a game: what happened, why, what to do.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub light: Light,
    pub what: String,
    pub why: Option<String>,
    pub todo: Option<String>,
    /// The suggestion behind the "Fix it" button, if one can be applied with a click.
    pub fix_id: Option<String>,
    pub fix_label: Option<String>,
}

pub(crate) fn confidence_word(c: Confidence) -> &'static str {
    match c {
        Confidence::Likely => "Likely",
        Confidence::Possible => "Possibly",
        Confidence::Unsure => "Long shot",
    }
}

/// Lower-case the first letter, for joining a title into a sentence.
pub(crate) fn sentence_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f)
            if !s.starts_with("CachyPlay") && !s.starts_with("Proton") && !s.starts_with("GE-") =>
        {
            f.to_lowercase().collect::<String>() + c.as_str()
        }
        Some(f) => f.to_string() + c.as_str(),
        None => String::new(),
    }
}

pub fn summary(ev: &Evidence, suggestions: &[Suggestion]) -> Summary {
    let crashes = history::runs(
        &ev.sessions,
        &ev.app_id,
        &ev.events.iter().collect::<Vec<_>>(),
    );
    let trouble = history::trouble(&crashes);
    let game_crashes = ev.game_events().len();
    let last_launch = ev.launches.last();
    let day = |t: DateTime| t.strftime("%b %-d").to_string();

    let worked_fix = suggestions
        .iter()
        .any(|s| s.fix.is_some() && s.status == Status::Worked);
    let blocked = suggestions
        .iter()
        .any(|s| s.id == "anti_cheat" && s.confidence == Confidence::Likely);
    // The status steps decide first: a change being tested, or clean play since the last problem.
    let prog = crate::progress::progress(ev, ev.now());
    let asks = suggestions.iter().any(|s| s.ask_verdict);
    if !blocked && prog.fix_label.is_some() && prog.stage != crate::progress::Stage::NotWorking {
        use crate::progress::Stage;
        return Summary {
            light: prog.light,
            what: format!("{}.", prog.headline),
            why: prog.detail.clone(),
            todo: Some(match prog.stage {
                _ if prog.ask_online.is_some() => {
                    "Answer \"Was that online?\" under History so the last start can count.".into()
                }
                _ if !ev.cachyplay_on && !prog.fix_label.as_deref().is_some_and(|l| l.starts_with("Switch to")) => {
                    "Turn on the CachyPlay switch for this game so the fix is used, then start the game from Steam.".into()
                }
                _ if asks => "Tell CachyPlay below whether it works now.".into(),
                Stage::WorksWithFix => "Nothing to do. Keep the fix on.".into(),
                Stage::SeemsToWork => format!(
                    "Keep playing as usual; a clean run is {}.",
                    prog.needed_text
                ),
                _ => "Start the game from Steam as usual to try it.".into(),
            }),
            fix_id: None,
            fix_label: None,
        };
    }
    if !blocked && prog.stage == crate::progress::Stage::RunsFine {
        return Summary {
            light: Light::Green,
            what: format!("{}.", prog.headline),
            why: prog.detail.clone(),
            todo: None,
            fix_id: None,
            fix_label: None,
        };
    }
    // A fix (or Proton switch) that's applied and hasn't failed: try that before anything else.
    let pending = suggestions.iter().find(|s| {
        (s.fix.is_some() || s.switched) && matches!(s.status, Status::Applied | Status::Worked)
    });
    if let (Some(p), false, None) = (pending, blocked, &prog.fix_label) {
        let worked = p.status == Status::Worked;
        return Summary {
            light: Light::Yellow,
            what: if worked {
                format!("It has run since you applied \"{}\".", p.title)
            } else {
                format!("You applied \"{}\". It hasn't been played since.", p.title)
            },
            why: Some("If it crashes again, CachyPlay marks that fix as not helping and suggests the next one.".into()),
            todo: Some(if p.ask_verdict {
                "Tell CachyPlay below whether it works now.".into()
            } else {
                "Start the game from Steam as usual to try it.".into()
            }),
            fix_id: None,
            fix_label: None,
        };
    }
    let (light, what) = if !blocked && prog.fix_label.is_some() {
        (Light::Red, format!("{}.", prog.headline))
    } else if blocked {
        (
            Light::Red,
            format!("{} can't be played online on Linux.", ev.game_name),
        )
    } else if let Some(t) = &trouble {
        let n = t.since.len();
        let crash_part = if t.crash_count() > 0 {
            ", with a graphics driver crash"
        } else {
            ""
        };
        (
            Light::Red,
            if n == 1 {
                format!(
                    "It failed the last time you started it ({}){crash_part}.",
                    day(t.first_bad())
                )
            } else {
                format!(
                    "Every start since {} failed ({} times){crash_part}.",
                    day(t.first_bad()),
                    n
                )
            },
        )
    } else if game_crashes > 0 {
        (
            Light::Red,
            format!(
                "The graphics driver crashed {} while it ran.",
                if game_crashes == 1 {
                    "once".to_string()
                } else {
                    format!("{game_crashes} times")
                }
            ),
        )
    } else if last_launch.is_some_and(|l| l.running) {
        (
            Light::Yellow,
            "It's running now (or stopped responding).".into(),
        )
    } else if last_launch.is_some_and(|l| l.failed) {
        (Light::Red, "The last start closed early.".into())
    } else if worked_fix {
        (
            Light::Green,
            "It works with a fix CachyPlay applied.".into(),
        )
    } else if crashes
        .iter()
        .rev()
        .take(3)
        .any(|r| r.outcome == history::Outcome::Played)
        || last_launch.is_some_and(|l| !l.failed)
    {
        (Light::Green, "It's been running fine.".into())
    } else {
        (
            Light::Grey,
            "Not played since CachyPlay started watching.".into(),
        )
    };

    let actionable = |s: &&Suggestion| s.status != Status::DidntHelp;
    let top = if light == Light::Green {
        None
    } else {
        suggestions.iter().find(actionable)
    };
    let why = top.map(|s| {
        format!(
            "{}: {}.",
            confidence_word(s.confidence),
            sentence_case(&s.title).trim_end_matches('.')
        )
    });
    // "Fix it" applies the first suggestion that can be done with one click and hasn't been tried.
    let fix = if light == Light::Green {
        None
    } else {
        suggestions.iter().find(|s| {
            (s.fix.is_some() || s.switch_tool.is_some()) && s.status == Status::New && !s.switched
        })
    };
    let todo = match (top, fix) {
        (Some(t), _) if t.fix.is_none() && t.switch_tool.is_none() && !t.steps.is_empty() => {
            Some(t.steps[0].clone())
        }
        (_, Some(f)) => Some(match (&f.fix, &f.switch_tool) {
            (Some(x), _) => format!("Press Fix it to try: {}.", x.label),
            (None, Some(t)) => format!("Press Fix it to switch to {}.", t.display),
            _ => unreachable!(),
        }),
        (Some(t), None) => t.steps.first().cloned(),
        (None, None) => None,
    };
    Summary {
        light,
        what,
        why,
        todo,
        fix_id: fix.map(|f| f.id.clone()),
        fix_label: fix.map(|f| match (&f.fix, &f.switch_tool) {
            (Some(x), _) => x.label.clone(),
            (None, Some(t)) => format!("Switch to {}", t.display),
            _ => String::new(),
        }),
    }
}
