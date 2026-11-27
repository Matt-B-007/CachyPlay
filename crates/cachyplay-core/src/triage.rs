// SPDX-License-Identifier: GPL-3.0-or-later
//! The second pass over the suggestions: what's already been tried (also outside CachyPlay, in
//! Steam's launch options), what that rules out, how sure each remaining suspect is, what would
//! prove it, and the "Find the cause" ladder. Also the rules that need that bigger picture: a fresh
//! shader cache, the NVIDIA feature switch for Xid 109, performance recording, going back to the
//! previous driver, and reporting to NVIDIA.

use crate::advice::{Confidence, Evidence, Status, Suggestion};
use crate::history::{self, Outcome};
use crate::pccheck::Root;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// The same NVIDIA error this many times in one game: offer the NVIDIA report.
pub const REPORT_AFTER: usize = 3;

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub title: String,
    /// "ruled_out", "helped", "testing", "next", "todo"
    pub status: &'static str,
    pub detail: Option<String>,
    pub suggestion_id: Option<String>,
}

/// "Find the cause": the tests in order, with what's been crossed off.
#[derive(Debug, Clone, Serialize)]
pub struct FindCause {
    pub ruled_out: Vec<String>,
    pub steps: Vec<Step>,
    /// "Ruled out: Proton, add-ons. Left: the driver."
    pub summary: String,
}

fn day(t: chrono::NaiveDateTime) -> String {
    t.format("%b %-d %H:%M").to_string()
}

/// Settings tried by editing Steam's launch options: (settings, start, crashed?, seconds).
fn steam_tries(ev: &Evidence) -> Vec<(Vec<String>, chrono::NaiveDateTime, bool, i64)> {
    let crashes: Vec<_> = ev.events.iter().collect();
    history::runs(&ev.sessions, &ev.app_id, &crashes)
        .into_iter()
        .filter(|r| !r.session.settings.is_empty() && r.session.end.is_some())
        .map(|r| {
            let note_bad = ev
                .notes
                .get(r.session.start)
                .is_some_and(|n| n.graphics_problem());
            (
                r.session.settings.clone(),
                r.session.start,
                r.outcome == Outcome::Crashed || note_bad,
                r.session.duration().map(|d| d.num_seconds()).unwrap_or(0),
            )
        })
        .collect()
}

/// A suggestion's fix was tried in Steam's launch options: mark it, and say how it went.
pub fn mark_steam_tries(ev: &Evidence, out: &mut [Suggestion]) {
    let tries = steam_tries(ev);
    for s in out.iter_mut() {
        let Some(f) = &s.fix else { continue };
        if f.env.is_empty() || s.status == Status::DidntHelp {
            continue;
        }
        // Any of the fix's settings counts: a DirectX 12 game only reads the vkd3d one, for example.
        let tried = |set: &Vec<String>| -> Vec<String> {
            f.env
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .filter(|kv| set.contains(kv))
                .collect()
        };
        let matches = |set: &Vec<String>| !tried(set).is_empty();
        if let Some((set, at, _, _)) = tries.iter().find(|t| t.2 && matches(&t.0)) {
            s.status = Status::DidntHelp;
            let part = tried(set);
            s.why.push(if part.len() < f.env.len() {
                format!(
                    "You tried {} in Steam's launch options ({}), and it crashed anyway.",
                    part.join(" "),
                    day(*at)
                )
            } else {
                format!(
                    "You tried this in Steam's launch options ({}), and it crashed anyway.",
                    day(*at)
                )
            });
        } else if let Some((_, at, _, secs)) = tries
            .iter()
            .find(|t| !t.2 && matches(&t.0) && t.3 >= history::SHORT)
        {
            s.why.push(format!(
                "You tried this in Steam's launch options ({}): it ran {} without a crash.",
                day(*at),
                history::duration_text(chrono::Duration::seconds(*secs))
            ));
        }
    }
}

/// Plain-words list of what's been ruled out for this game.
pub fn ruled_out(ev: &Evidence, out: &[Suggestion]) -> Vec<String> {
    let mut v: Vec<String> = out
        .iter()
        .filter(|s| s.status == Status::DidntHelp)
        .map(|s| {
            let t = s.title.trim_end_matches('.');
            // "Changing the Proton version won't fix this" → "The Proton version".
            match t
                .strip_prefix("Changing the ")
                .and_then(|r| r.strip_suffix(" won't fix this"))
            {
                Some(what) => format!("The {what}"),
                None => t.to_string(),
            }
        })
        .collect();
    // Proton: the same crash on two or more versions.
    let crashes: Vec<_> = ev.events.iter().collect();
    let runs = history::runs(&ev.sessions, &ev.app_id, &crashes);
    let mut crashed_tools: Vec<String> = runs
        .iter()
        .filter(|r| r.outcome == Outcome::Crashed)
        .filter_map(|r| r.session.tool.as_deref().map(crate::compat::friendly))
        .collect();
    crashed_tools.sort();
    crashed_tools.dedup();
    if crashed_tools.len() >= 2 && !v.iter().any(|x| x == "The Proton version") {
        v.push(format!(
            "The Proton version (the same crash on {})",
            crashed_tools.join(" and ")
        ));
    }
    // Settings tried in Steam that crashed and aren't one of CachyPlay's suggestions.
    for (set, at, bad, _) in steam_tries(ev) {
        let joined = set.join(" ");
        let known = out.iter().any(|s| {
            s.fix.as_ref().is_some_and(|f| {
                f.env
                    .iter()
                    .any(|(k, val)| set.contains(&format!("{k}={val}")))
            })
        });
        if bad && !known && !v.iter().any(|x| x.contains(&joined)) {
            v.push(format!("{joined} (tried {}, crashed)", day(at)));
        }
    }
    // Your settings tried through CachyPlay and crashed.
    if let Some(s) = out
        .iter()
        .find(|s| s.id == crate::fixes::YOUR_SETTINGS && s.status == Status::DidntHelp)
    {
        if !v.contains(&s.title) {
            v.push(s.title.clone());
        }
    }
    // Changes ruled out earlier and since undone (they may not be suggestions any more).
    for t in ev.tries.all_ruled_out() {
        let label = t.label.trim_end_matches('.').to_string();
        if !v
            .iter()
            .any(|x| x.contains(&label) || label.contains(x.as_str()))
        {
            v.push(label);
        }
    }
    v.dedup();
    v
}

fn lower(c: Confidence) -> Confidence {
    match c {
        Confidence::Likely => Confidence::Possible,
        _ => Confidence::Unsure,
    }
}

/// What would prove each kind of suspect.
fn proof_for(id: &str) -> Option<&'static str> {
    Some(match id {
        "disable_vulkan_layers" => "If the game stops crashing with the add-ons off, one of them is the cause.",
        "switch_proton" | "compat_tool" | "install_proton_steps" => "If it stops crashing on another Proton version, that was it. The same crash on two versions rules Proton out.",
        "unity_d3d12" | "unity_vulkan" => "If it runs clean in the other graphics mode, the game's default mode is the problem.",
        "driver_rollback" | "driver_downgrade" => "If the game stops crashing on the previous driver, it's the driver.",
        "shader_cache" => "If it runs clean once the cache has rebuilt, a damaged saved shader was the cause.",
        "nvidia_raw_access_chains" => "If the crash stops with this feature off, it's the known NVIDIA/translation-layer hang.",
        "gpu_stock_clocks" => "If it stops at stock clocks, the overclock or undervolt was the cause.",
        "game_frame_generation" => "If it stops with frame generation off, that setting is the trigger.",
        "launch_options_order" => "Moving the setting in front of %command% makes Steam actually use it.",
        _ => return None,
    })
}

/// Confidence that moves with the evidence, the proof line, and the last suspect rising.
pub fn adjust(ev: &Evidence, out: &mut [Suggestion]) {
    for s in out.iter_mut() {
        if s.proof.is_none() {
            s.proof = proof_for(&s.id).map(str::to_string);
        }
        if s.status == Status::DidntHelp {
            s.confidence = lower(s.confidence);
        }
    }
    let ruled = out.iter().filter(|s| s.status == Status::DidntHelp).count();
    let left: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.status != Status::DidntHelp
                && (s.fix.is_some() || s.switch_tool.is_some() || s.big_step || s.action.is_some())
        })
        .map(|(i, _)| i)
        .collect();
    if ruled >= 2 && left.len() == 1 {
        let s = &mut out[left[0]];
        s.confidence = Confidence::Likely;
        s.why.push(
            "Everything else tried so far has been ruled out, so this is now the main suspect."
                .into(),
        );
    }
    let _ = ev;
}

/// "Find the cause": the testable suggestions in order, with their state.
pub fn find_cause(ev: &Evidence, out: &[Suggestion]) -> FindCause {
    let ruled = ruled_out(ev, out);
    let mut next_given = false;
    let steps: Vec<Step> = out
        .iter()
        .filter(|s| {
            s.fix.is_some()
                || s.switch_tool.is_some()
                || s.action.is_some()
                || s.big_step
                || s.switched
        })
        .map(|s| {
            let status = match s.status {
                Status::DidntHelp => "ruled_out",
                Status::Worked => "helped",
                Status::Applied => "testing",
                Status::New if s.switched => "testing",
                Status::New if !next_given => {
                    next_given = true;
                    "next"
                }
                Status::New => "todo",
            };
            Step {
                title: s.title.clone(),
                status,
                detail: s.proof.clone(),
                suggestion_id: Some(s.id.clone()),
            }
        })
        .collect();
    let left: Vec<&str> = steps
        .iter()
        .filter(|s| matches!(s.status, "next" | "todo" | "testing"))
        .map(|s| s.title.as_str())
        .collect();
    let summary = match (ruled.is_empty(), left.is_empty()) {
        (true, _) => "Nothing ruled out yet. Try the steps in order, one per start.".to_string(),
        (false, true) => format!(
            "Ruled out: {}. No more tests on the list; report it (see below).",
            ruled.join("; ")
        ),
        (false, false) => format!(
            "Ruled out: {}. Left: {}.",
            ruled.join("; "),
            left.join("; ")
        ),
    };
    FindCause {
        ruled_out: ruled,
        steps,
        summary,
    }
}

/// Facts about going back to the previous NVIDIA driver.
#[derive(Debug, Clone, Default)]
pub struct OldDriver {
    pub from: String,
    pub to: String,
    /// Cached package files for the previous version (pacman's cache).
    pub files: Vec<PathBuf>,
    /// Installed packages that carry the driver version and need going back too.
    pub packages: Vec<String>,
    /// Packages for which no older file was found.
    pub missing: Vec<String>,
    /// A prebuilt kernel module package (like linux-cachyos-nvidia-open) is installed, whose
    /// matching older build CachyPlay can't check.
    pub prebuilt_module: Option<String>,
}

/// Look in pacman's database and cache for the previous driver.
pub fn old_driver(root: &Root, from: &str, to: &str) -> OldDriver {
    let ver = |v: &str| {
        v.split('-')
            .next()
            .unwrap_or(v)
            .trim_start_matches(|c: char| !c.is_ascii_digit())
            .to_string()
    };
    let (old, new) = (ver(from), ver(to));
    let mut d = OldDriver {
        from: old.clone(),
        to: new.clone(),
        ..Default::default()
    };
    if old.is_empty() || new.is_empty() {
        return d;
    }
    let installed: Vec<String> = fs::read_dir(root.p("/var/lib/pacman/local"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let cache: Vec<String> = fs::read_dir(root.p("/var/cache/pacman/pkg"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".pkg.tar.zst") || n.ends_with(".pkg.tar.xz"))
        .collect();
    for dir in &installed {
        // "nvidia-utils-620.73.04-1": name, then "-<version>-<rel>".
        let Some(pos) = dir
            .find(&format!("-{new}"))
            .or_else(|| dir.find(&format!("-1:{new}")))
        else {
            if dir.contains("-nvidia") && dir.starts_with("linux-") {
                d.prebuilt_module = Some(dir.rsplitn(3, '-').nth(2).unwrap_or(dir).to_string());
            }
            continue;
        };
        let name = &dir[..pos];
        d.packages.push(name.to_string());
        match cache.iter().find(|c| {
            c.starts_with(&format!("{name}-{old}-")) || c.starts_with(&format!("{name}-1:{old}-"))
        }) {
            Some(f) => d.files.push(root.p(&format!("/var/cache/pacman/pkg/{f}"))),
            None => d.missing.push(name.to_string()),
        }
    }
    d.packages.sort();
    d.files.sort();
    d
}

/// The newer rules, added after the main ones.
pub fn extra_rules(ev: &Evidence, out: &mut Vec<Suggestion>, root: &Root) {
    let mine: Vec<&crate::crashmatch::CrashEvent> = ev
        .events
        .iter()
        .filter(|e| e.app_id.as_deref() == Some(ev.app_id.as_str()))
        .collect();
    let nvidia_hangs = mine
        .iter()
        .filter(|e| e.faults.iter().any(|f| f.xid == Some(109)))
        .count();
    let fix = |id: &str, label: &str, env: &[(&str, &str)], temporary: bool| crate::fixes::Fix {
        id: id.into(),
        label: label.into(),
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
        temporary,
        ..Default::default()
    };
    let status = |id: &str| ev.status_of(id);

    // Shader cache: a damaged saved shader can hang the card on particular maps.
    if !mine.is_empty() && ev.uses_proton {
        out.push(Suggestion {
            id: "shader_cache".into(),
            title: "Start the game's shader cache fresh".into(),
            why: vec!["Games save compiled shaders, and a damaged one can hang the graphics card on particular maps or modes.".into()],
            steps: vec!["CachyPlay moves the cache aside (Undo puts it back). The first start afterwards stutters a little while shaders rebuild.".into()],
            action: Some("reset_shader_cache".into()),
            status: status("shader_cache"),
            priority: 32,
            confidence: Confidence::Unsure,
            ..Default::default()
        });
    }

    // NVIDIA's raw access chains: a known cause of Xid 109 hangs through DXVK, maybe vkd3d too.
    // Which layer the game uses: its Unity log, or what Proton loaded.
    let layer: Option<String> = match ev.unity.as_ref().map(|u| (u.graphics.clone(), u.forced)) {
        Some((Some(g), false)) if g.starts_with("Direct3D 11") => Some("dxvk".into()),
        Some((Some(g), false)) if g.starts_with("Direct3D 12") => Some("vkd3d".into()),
        Some((_, true)) => Some("dxvk".into()),
        _ => ev.proton_layer.clone(),
    };
    if nvidia_hangs > 0 && ev.uses_proton {
        out.push(Suggestion {
            id: "nvidia_raw_access_chains".into(),
            title: "Turn off one NVIDIA Vulkan feature for this game".into(),
            why: vec![
                "This crash is an NVIDIA Xid 109 hang. A known cause in DirectX games run through Proton is the VK_NV_raw_access_chains feature; turning it off stopped the hang in other games.".into(),
                match layer.as_deref() {
                    Some("dxvk") => format!("{} runs in Direct3D 11, through DXVK: that's the case where this is a confirmed fix.", ev.game_name),
                    Some("vkd3d") => format!("{} runs in Direct3D 12, through vkd3d: there it's only reported, not confirmed.", ev.game_name),
                    _ => "Confirmed for Direct3D 11 games (DXVK); for Direct3D 12 (vkd3d) it's reported but not confirmed.".into(),
                },
            ],
            fix: Some(fix(
                "nvidia_raw_access_chains",
                "NVIDIA raw access chains off",
                &[("VKD3D_DISABLE_EXTENSIONS", "VK_NV_raw_access_chains"), ("DXVK_CONFIG", "dxvk.enableNvRawAccessChains = False")],
                false,
            )),
            status: status("nvidia_raw_access_chains"),
            priority: if layer.as_deref() == Some("dxvk") { 23 } else { 34 },
            confidence: if layer.as_deref() == Some("dxvk") { Confidence::Possible } else { Confidence::Unsure },
            ..Default::default()
        });
    }

    // Stutter or lag: record FPS and frame times on the next start.
    let lag = ev.notes.notes.values().any(|n| n.kind == "stutter");
    if lag {
        let mango = ev
            .layers
            .iter()
            .any(|l| l.name.to_lowercase().contains("mangohud"));
        out.push(Suggestion {
            id: "record_performance".into(),
            title: "Record FPS and frame times on the next start".into(),
            why: vec!["You said it stuttered. A recording shows whether it's low FPS, uneven frame times or short freezes, and when.".into()],
            fix: mango.then(|| fix(
                "record_performance",
                "Record performance (one start)",
                &[("MANGOHUD", "1"), ("MANGOHUD_CONFIG", "no_display,autostart_log=1,log_duration=0")],
                true,
            )),
            steps: if mango {
                vec!["It's for one start only and shows nothing on screen. Save a bug report afterwards; the recording is attached.".into()]
            } else {
                vec!["MangoHud records it. Install it once: sudo pacman -S --needed mangohud lib32-mangohud".into()]
            },
            status: status("record_performance"),
            priority: 36,
            confidence: Confidence::Possible,
            ..Default::default()
        });
    }

    // Only online: say so, since it changes what's likely.
    if ev.notes.online_only && !mine.is_empty() {
        out.push(Suggestion {
            id: "online_only".into(),
            title: "It only happens online".into(),
            why: vec!["You said offline play is fine and online play crashes. Online modes use different maps, more players and effects, and sometimes anti-cheat, so graphics work differs even on the same settings.".into()],
            steps: vec![
                "Only online starts count toward \"Works with a fix\" now.".into(),
                "Tell the developer it only happens online: that narrows it down a lot for them.".into(),
            ],
            priority: 60,
            confidence: Confidence::Possible,
            ..Default::default()
        });
    }

    // Going back to the previous driver: only when it fits, never automatic.
    if let Some(t) = crate::report::driver_timeline(ev) {
        // The last real driver version change before the first crash (not nvidia-settings or
        // OpenCL, and not a kernel module rebuild of the same version).
        let upd = crate::advice::driver_context(ev)
            .and_then(|c| c.version_change)
            .filter(|u| u.package.contains("nvidia") && !mine.is_empty());
        let cheap_done = out.iter().filter(|s| s.status == Status::DidntHelp).count()
            + usize::from(
                ruled_out(ev, out)
                    .iter()
                    .any(|r| r.starts_with("The Proton version")),
            );
        let newer_since = mine.iter().map(|e| e.at).max().and_then(|last| {
            ev.driver_updates.iter().find(|u| {
                u.package.contains("nvidia")
                    && crate::advice::is_driver_package(&u.package)
                    && u.at > last
            })
        });
        if let (Some(u), true) = (upd, nvidia_hangs > 0 && cheap_done >= 3) {
            let od = old_driver(root, &u.from, &u.to);
            let mut why = vec![t.clone()];
            let mut steps = Vec::new();
            if let Some(n) = newer_since {
                why.push(format!("A newer driver ({} {}) arrived since the last crash: test the game again on it first.", n.package, n.to));
            }
            if !od.files.is_empty() && od.missing.is_empty() && od.prebuilt_module.is_none() {
                let files: Vec<String> = od.files.iter().map(|f| f.display().to_string()).collect();
                steps.push(format!(
                    "Undo (back to {} afterwards): sudo pacman -Syu",
                    od.to
                ));
                steps.push(format!(
                    "Go back to {}: sudo pacman -U {}",
                    od.from,
                    files.join(" ")
                ));
                steps.push(
                    "Restart the PC. CachyPlay checks the driver version after the restart.".into(),
                );
                steps.push(format!("Keep pacman from updating it straight back: add \"IgnorePkg = {}\" under [options] in /etc/pacman.conf (remove that line later to get updates again).", od.packages.join(" ")));
            } else {
                why.push(match &od.prebuilt_module {
                    Some(m) => format!("Your kernel's NVIDIA module comes prebuilt ({m}), and its older build has to match; CachyPlay can't check that, so it doesn't give downgrade commands."),
                    None if od.files.is_empty() => format!("The previous driver ({}) isn't in pacman's cache on this PC, so going back would mean downloading it.", od.from),
                    None => format!("Some packages have no older copy in pacman's cache: {}.", od.missing.join(", ")),
                });
                steps.push("Safer way back: restart and pick a snapshot from before the update in the boot menu (CachyOS keeps them).".into());
            }
            steps.push(
                "Or wait: CachyPlay notices when a newer NVIDIA driver arrives and says so here."
                    .into(),
            );
            steps.push("Or report it to NVIDIA (see \"Report it to NVIDIA\").".into());
            out.push(Suggestion {
                id: "driver_downgrade".into(),
                title: "Big step: go back to the previous NVIDIA driver".into(),
                why,
                steps,
                big_step: true,
                priority: 70,
                confidence: Confidence::Possible,
                ..Default::default()
            });
        }
    }

    // The same NVIDIA error again and again: report it.
    if nvidia_hangs >= REPORT_AFTER {
        out.push(Suggestion {
            id: "report_nvidia".into(),
            title: "Report it to NVIDIA".into(),
            why: vec![format!(
                "The same NVIDIA error (Xid 109) has happened {nvidia_hangs} times in {}. NVIDIA fixes driver hangs they can reproduce.",
                ev.game_name
            )],
            steps: vec![
                "In a terminal, run: sudo nvidia-bug-report.sh (it writes nvidia-bug-report.log.gz in the current folder).".into(),
                "Post on forums.developer.nvidia.com in the Linux category: copy the text from \"Tell NVIDIA\" on the History tab, and attach nvidia-bug-report.log.gz and a CachyPlay bug report.".into(),
            ],
            priority: 75,
            confidence: Confidence::Possible,
            ..Default::default()
        });
    }
}

/// Short text for NVIDIA's forum.
pub fn nvidia_report(
    ev: &Evidence,
    driver: Option<&str>,
    gpu: Option<&str>,
    ruled: &[String],
) -> String {
    let mine: Vec<_> = ev
        .events
        .iter()
        .filter(|e| e.app_id.as_deref() == Some(ev.app_id.as_str()))
        .collect();
    let mut s = format!(
        "{} hangs with Xid 109 (CTX SWITCH TIMEOUT) on Linux\n\nGPU: {}\nDriver: {}\nGame: {} (Steam AppID {}), through Proton\nCrashes: {} in the last 30 days",
        ev.game_name,
        gpu.unwrap_or("(fill in: nvidia-smi shows it)"),
        driver.unwrap_or("(fill in)"),
        ev.game_name,
        ev.app_id,
        mine.len()
    );
    if let Some(t) = crate::report::driver_timeline(ev) {
        s.push_str(&format!("\n{}", t.trim_start_matches("Driver timeline: ")));
    }
    if ev.notes.online_only {
        s.push_str("\nOnly happens in online matches; offline play is fine.");
    }
    for g in &ev.nvidia_switch_helped {
        s.push_str(&format!("\n{g} (same Xid 109 hang, Direct3D 11 through DXVK) runs normally with VK_NV_raw_access_chains disabled (dxvk.enableNvRawAccessChains = False)."));
    }
    if !ruled.is_empty() {
        s.push_str(&format!("\nRuled out: {}.", ruled.join("; ")));
    }
    s.push_str("\n\nnvidia-bug-report.log.gz and a CachyPlay report are attached.");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crashmatch::{CrashEvent, GpuFault, Session};

    fn t(x: &str) -> chrono::NaiveDateTime {
        chrono::NaiveDateTime::parse_from_str(x, "%Y-%m-%d %H:%M").unwrap()
    }
    fn halo_day() -> Evidence {
        let s = |start: &str, end: &str, tool: &str, set: &[&str]| Session {
            app_id: "1240440".into(),
            start: t(start),
            end: Some(t(end)),
            source: "Steam",
            tool: Some(tool.into()),
            settings: set.iter().map(|x| x.to_string()).collect(),
        };
        let c = |at: &str| CrashEvent {
            at: t(at),
            app_id: Some("1240440".into()),
            faults: vec![GpuFault {
                xid: Some(109),
                ..Default::default()
            }],
        };
        Evidence {
            app_id: "1240440".into(),
            game_name: "Halo Infinite".into(),
            uses_proton: true,
            sessions: vec![
                s(
                    "2026-11-25 16:48",
                    "2026-11-25 16:53",
                    "GE-Proton11-11",
                    &[],
                ),
                s(
                    "2026-11-25 16:55",
                    "2026-11-25 16:59",
                    "proton-cachyos-slr",
                    &[],
                ),
                s(
                    "2026-11-25 19:20",
                    "2026-11-25 19:27",
                    "proton-cachyos-slr",
                    &["VKD3D_DISABLE_EXTENSIONS=VK_NV_raw_access_chains"],
                ),
                s(
                    "2026-11-25 19:36",
                    "2026-11-25 19:42",
                    "proton-cachyos-slr",
                    &["VKD3D_CONFIG=no_upload_hvv"],
                ),
            ],
            events: vec![
                c("2026-11-25 16:53"),
                c("2026-11-25 16:59"),
                c("2026-11-25 19:26"),
                c("2026-11-25 19:41"),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn remembers_what_was_tried_in_steam_and_what_that_rules_out() {
        let ev = halo_day();
        let mut out = Vec::new();
        extra_rules(&ev, &mut out, &Root::system());
        mark_steam_tries(&ev, &mut out);
        let nv = out
            .iter()
            .find(|s| s.id == "nvidia_raw_access_chains")
            .unwrap();
        assert_eq!(nv.status, Status::DidntHelp, "{:?}", nv.why);
        assert!(nv.why.last().unwrap().contains("Steam's launch options"));
        let r = ruled_out(&ev, &out);
        assert!(
            r.iter()
                .any(|x| x.starts_with("The Proton version (the same crash on GE-Proton11-11 and")),
            "{r:?}"
        );
        assert!(
            r.iter()
                .any(|x| x.starts_with("VKD3D_CONFIG=no_upload_hvv (tried")),
            "{r:?}"
        );
        assert!(
            r.iter()
                .any(|x| x.starts_with("Turn off one NVIDIA Vulkan feature")),
            "{r:?}"
        );
        // Four Xid 109s: report to NVIDIA.
        assert!(out.iter().any(|s| s.id == "report_nvidia"));
        let fc = find_cause(&ev, &out);
        assert!(fc.summary.starts_with("Ruled out:"), "{}", fc.summary);
        assert!(fc.steps.iter().any(|s| s.status == "next"));
    }

    #[test]
    fn last_suspect_rises_and_ruled_out_ones_drop() {
        let ev = halo_day();
        let mk = |id: &str, st: Status| Suggestion {
            id: id.into(),
            title: id.into(),
            status: st,
            fix: Some(crate::fixes::Fix::default()),
            confidence: Confidence::Likely,
            ..Default::default()
        };
        let mut out = vec![
            mk("a", Status::DidntHelp),
            mk("b", Status::DidntHelp),
            Suggestion {
                confidence: Confidence::Unsure,
                ..mk("c", Status::New)
            },
        ];
        adjust(&ev, &mut out);
        assert_eq!(out[0].confidence, Confidence::Possible);
        assert_eq!(out[2].confidence, Confidence::Likely);
        assert!(out[2].why.last().unwrap().contains("main suspect"));
    }

    #[test]
    fn finds_the_previous_driver_in_pacmans_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        for d in [
            "var/lib/pacman/local/nvidia-utils-620.73.04-1",
            "var/lib/pacman/local/lib32-nvidia-utils-620.73.04-1",
            "var/lib/pacman/local/nvidia-open-dkms-620.73.04-1",
            "var/lib/pacman/local/mesa-1:26.3.1-5",
        ] {
            fs::create_dir_all(r.join(d)).unwrap();
        }
        fs::create_dir_all(r.join("var/cache/pacman/pkg")).unwrap();
        for f in [
            "nvidia-utils-620.70.06-1-x86_64.pkg.tar.zst",
            "lib32-nvidia-utils-620.70.06-1-x86_64.pkg.tar.zst",
            "nvidia-open-dkms-620.70.06-1-x86_64.pkg.tar.zst",
        ] {
            fs::write(r.join("var/cache/pacman/pkg").join(f), "").unwrap();
        }
        let d = old_driver(&Root(r.to_path_buf()), "620.70.06-1", "620.73.04-1");
        assert_eq!(
            d.packages,
            vec!["lib32-nvidia-utils", "nvidia-open-dkms", "nvidia-utils"]
        );
        assert_eq!(d.files.len(), 3);
        assert!(d.missing.is_empty() && d.prebuilt_module.is_none());
        // A prebuilt kernel module can't be checked: no commands then.
        fs::create_dir_all(r.join("var/lib/pacman/local/linux-cachyos-nvidia-open-7.3.6-2"))
            .unwrap();
        assert_eq!(
            old_driver(&Root(r.to_path_buf()), "620.70.06-1", "620.73.04-1")
                .prebuilt_module
                .as_deref(),
            Some("linux-cachyos-nvidia-open")
        );
    }
}
