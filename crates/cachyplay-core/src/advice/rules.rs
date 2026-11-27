// SPDX-License-Identifier: GPL-3.0-or-later
//! The suggestion rules: each one is a fixed check over the evidence, with its reasons.

use super::*;

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

    let all_crashes: Vec<&CrashEvent> = ev.events.iter().collect();
    let runs = history::runs(&ev.sessions, &ev.app_id, &all_crashes);
    let trouble = history::trouble(&runs);
    let day = |t: NaiveDateTime| t.format("%b %-d").to_string();
    let others_fine = |since: NaiveDateTime| -> Vec<String> {
        history::others_ran_fine(
            &ev.sessions,
            &ev.app_id,
            since,
            chrono::Duration::minutes(60),
            &all_crashes,
        )
        .into_iter()
        .take(2)
        .map(|s| {
            format!(
                "{} ran {} on {}",
                ev.names
                    .get(&s.app_id)
                    .cloned()
                    .unwrap_or_else(|| format!("AppID {}", s.app_id)),
                history::duration_text(s.duration().unwrap_or_default()),
                day(s.start)
            )
        })
        .collect()
    };

    // 0. It worked, the game updated, and it has failed every start since: the update is the
    //    most likely cause, more than a driver that other games still run fine on.
    let mut update_rule = false;
    if let (Some(t), Some(up)) = (&trouble, ev.game_updated) {
        let after_good = t.last_good.is_none_or(|g| up >= g.session.start);
        if after_good && up <= t.first_bad() && t.first_bad() - up <= chrono::Duration::days(30) {
            update_rule = true;
            let mut why = Vec::new();
            if let Some(g) = t.last_good {
                why.push(format!(
                    "It ran normally on {} ({}).",
                    day(g.session.start),
                    history::duration_text(g.session.duration().unwrap_or_default())
                ));
            }
            why.push(format!("Steam updated the game on {}.", day(up)));
            let crashes = t.crash_count();
            why.push(format!(
                "Since {}, all {} starts failed{}.",
                day(t.first_bad()),
                t.since.len(),
                if crashes > 0 {
                    format!(
                        " ({} with a GPU driver crash)",
                        plural(crashes, "start", "starts")
                    )
                } else {
                    " within a few minutes".to_string()
                }
            ));
            let fine = others_fine(t.first_bad());
            if !fine.is_empty() {
                why.push(format!(
                    "Other games still run fine: {}. So your driver and PC are OK.",
                    fine.join("; ")
                ));
            }
            out.push(Suggestion {
                id: "game_update".into(),
                title: format!("This started with the game's {} update", day(up)),
                why,
                steps: vec![
                    "Check the game's news and discussions in Steam for a known problem with this update. Developers often post a fix or a workaround.".into(),
                    "If the game added new graphics options (frame generation, upscaling, ray tracing), turn them off and try again.".into(),
                    "Report it to the developer and attach a CachyPlay bug report for this game. It shows the crash started with their update.".into(),
                ],
                priority: 5,
                ..Default::default()
            });
        }
    }

    // 0b. Launch options in the wrong order (a setting after the game, or a game option before it).
    if let Some((bad, fixed)) = misplaced_launch_options(&ev.user_options) {
        // While CachyPlay is on, it moves misplaced settings to the right place itself.
        let handled = ev.cachyplay_on && bad.iter().all(|b| b.contains('='));
        out.push(Suggestion {
            id: "launch_options_order".into(),
            title: "Fix the order of your launch options".into(),
            why: vec![
                format!("Your launch options are: {}", ev.user_options.trim()),
                if handled {
                    format!(
                        "{} {} in the wrong place for Steam, but CachyPlay moves {} to the right place while it's on, so {} working now. It only matters if you turn CachyPlay off.",
                        bad.join(" "),
                        if bad.len() == 1 { "is" } else { "are" },
                        if bad.len() == 1 { "it" } else { "them" },
                        if bad.len() == 1 { "it's" } else { "they're" }
                    )
                } else {
                    format!(
                        "{} {} in the wrong place, so Steam passes {} to the game as plain text and {} nothing.",
                        bad.join(" "),
                        if bad.len() == 1 { "is" } else { "are" },
                        if bad.len() == 1 { "it" } else { "them" },
                        if bad.len() == 1 { "it does" } else { "they do" }
                    )
                },
            ],
            confidence: if handled { Confidence::Unsure } else { Confidence::Possible },
            steps: vec![
                "Settings like NAME=value go before %command%. Game options like -dx11 go after it.".into(),
                format!("In Steam: right-click the game → Properties → Launch options, and change it to: {fixed}"),
            ],
            priority: if handled { 85 } else { 8 },
            ..Default::default()
        });
    }

    // 0c. Another compatibility tool (Proton version).
    if ev.uses_proton && (trouble.is_some() || !game_events.is_empty()) {
        out.extend(compat_suggestion(ev, trouble.as_ref(), &day));
    }

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
            ..Default::default()
        });
    }

    // 2. A graphics driver update shortly before system-wide trouble.
    if several {
        let ctx = driver_context(ev);
        let mut why: Vec<String> = crash_line().into_iter().collect();
        let fmt_gap = |from: NaiveDateTime, to: NaiveDateTime| {
            let h = (to - from).num_hours();
            if h < 48 {
                format!("{h} hour{}", if h == 1 { "" } else { "s" })
            } else {
                format!("{} days", h / 24)
            }
        };
        let mut priority = 15;
        let closest = ctx.as_ref().and_then(|c| c.closest);
        if let (Some(c), Some(u)) = (&ctx, closest) {
            let rebuild = u.package.starts_with("linux");
            why.push(format!(
                "{} was updated from {} to {} on {}, {} before {}'s first crash{}.",
                u.package,
                u.from,
                u.to,
                u.at.format("%b %-d at %H:%M"),
                fmt_gap(u.at, c.first_crash),
                ev.game_name,
                if rebuild {
                    " (the kernel and its NVIDIA module)"
                } else {
                    ""
                }
            ));
            if let Some(v) = c.version_change.filter(|v| !std::ptr::eq(*v, u)) {
                why.push(format!(
                    "The driver itself last changed on {} ({} {} → {}).",
                    v.at.format("%b %-d"),
                    v.package,
                    v.from,
                    v.to
                ));
            }
            if let Some(note) = ran_after_note(c) {
                why.push(note);
            }
        } else if let Some(c) = &ctx {
            if let Some(a) = c.after.first() {
                why.push(format!(
                    "No driver update came before the crashes started; {} ({}) came after, so it isn't the cause.",
                    a.package,
                    a.at.format("%b %-d")
                ));
                priority = 50;
            }
        }
        why.push("Problems that start in several games at once usually come from the driver, not the games.".into());
        if let Some(u) = closest {
            let fine = others_fine(u.at);
            if !ev.nvidia_switch_helped.is_empty() {
                why.push(format!(
                    "{} only runs with an NVIDIA-specific Vulkan feature switched off, which points at how this driver handles it, even though other games run fine.",
                    ev.nvidia_switch_helped.join(" and ")
                ));
            } else if !fine.is_empty() {
                why.push(format!(
                    "But other games ran fine on this same driver ({}), so a broken driver is less likely.",
                    fine.join("; ")
                ));
                priority = 50;
            }
            if let Some(t) = &trouble {
                if t.first_bad() < u.at {
                    why.push(format!(
                        "{} started failing on {}, before this update.",
                        ev.game_name,
                        day(t.first_bad())
                    ));
                    priority = 50;
                }
            }
        }
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
            priority,
            ..Default::default()
        });
        out.push(Suggestion {
            id: "gpu_stock_clocks".into(),
            title: "Test with the GPU at stock settings".into(),
            why: vec!["Unstable GPU clocks or undervolting cause this kind of driver error in any game.".into()],
            fix: None,
            steps: vec!["If you use LACT, GreenWithEnvy or nvidia-settings to overclock or undervolt, reset to defaults and test again.".into()],
            status: Status::New,
            priority: 45,
            ..Default::default()
        });
    }

    // 3. Only this game, and it updated recently.
    if !several && !game_events.is_empty() && !update_rule {
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
        let steps = vec![
            "Report it to the game's developer and attach a CachyPlay bug report for this game."
                .into(),
        ];
        out.push(Suggestion {
            id: "game_specific".into(),
            title: "Tell the game's developer".into(),
            why,
            fix: None,
            steps,
            status: Status::New,
            priority: 25,
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        });
    }

    let trouble_here = trouble.is_some() || !game_events.is_empty();

    // 7. Unity games stuck on Direct3D 11: try the graphics modes the working games use.
    // Keep showing these while one is on or has been tried, even though the game's log then names
    // the forced mode instead of Direct3D 11 (otherwise the card, and its Undo, disappears).
    let mode_fix_used = ["unity_d3d12", "unity_vulkan"].iter().any(|id| {
        ev.fixes.fixes.iter().any(|f| f.id == *id) || ev.tries.tries.iter().any(|t| t.id == *id)
    });
    if let Some(u) = ev.unity.as_ref().filter(|_| trouble_here || mode_fix_used) {
        if u.graphics.as_deref() == Some("Direct3D 11") || mode_fix_used {
            let mut why: Vec<String> = Vec::new();
            why.push(format!(
                "{} draws with Direct3D 11{} by default, which Proton turns into Vulkan with DXVK.",
                ev.game_name,
                if u.forced && u.graphics.as_deref() == Some("Direct3D 11") {
                    " (the game picks it at start)"
                } else {
                    ""
                }
            ));
            let d3d12_users: Vec<&str> = ev
                .working
                .iter()
                .filter(|w| w.tool.is_some())
                .map(|w| w.name.as_str())
                .take(2)
                .collect();
            if !d3d12_users.is_empty() {
                why.push(format!(
                    "Games that run fine here ({}) use a different graphics path.",
                    d3d12_users.join(", ")
                ));
            }
            why.push("Unity games can often be told to use Direct3D 12 or Vulkan instead. It only works if the developer included it; if the game won't start, undo it.".into());
            out.push(Suggestion {
                id: "unity_d3d12".into(),
                title: "Try the game's Direct3D 12 mode".into(),
                why: why.clone(),
                fix: Some(fix(
                    "unity_d3d12",
                    "Start in Direct3D 12",
                    &[],
                    &["-force-d3d12"],
                    false,
                )),
                status: ev.status_of("unity_d3d12"),
                priority: 22,
                confidence: Confidence::Possible,
                ..Default::default()
            });
            out.push(Suggestion {
                id: "unity_vulkan".into(),
                title: "Try the game's Vulkan mode".into(),
                why,
                fix: Some(fix(
                    "unity_vulkan",
                    "Start in Vulkan",
                    &[],
                    &["-force-vulkan"],
                    false,
                )),
                status: ev.status_of("unity_vulkan"),
                priority: 27,
                confidence: Confidence::Unsure,
                ..Default::default()
            });
        }
        // 8. The game's own frame generation.
        if u.frame_generation {
            out.push(Suggestion {
                id: "game_frame_generation".into(),
                title: "Turn off the game's frame generation".into(),
                why: vec![
                    format!("{}'s log shows it turns on its own frame generation.", ev.game_name),
                    "Frame generation is new in many games and a common cause of crashes, especially stacked with other frame-generation add-ons.".into(),
                ],
                steps: vec![
                    "In the game: Settings → Graphics → turn off Frame Generation (it may be called FSR 3 Frame Gen, XeSS-FG or DLSS-FG).".into(),
                    "If it crashes before you can reach the settings, try the Direct3D 12 suggestion first, then change it.".into(),
                ],
                priority: 12,
                confidence: Confidence::Possible,
                ..Default::default()
            });
        }
    }

    // 9. Anti-cheat that blocks Linux: say so before anyone wastes time on fixes.
    if let Some(a) = &ev.anti_cheat {
        if a.blocks_linux || trouble_here || failed_launches > 0 {
            out.push(Suggestion {
                id: "anti_cheat".into(),
                title: if a.blocks_linux {
                    "This game's anti-cheat blocks Linux".into()
                } else {
                    format!("This game uses {}", a.kind)
                },
                why: vec![if a.blocks_linux {
                    format!("{}'s {} doesn't allow Linux or Steam Deck players online. No setting on your PC can change that.", ev.game_name, a.kind)
                } else {
                    format!("{} ships {}. Developers have to turn on Linux support for it; if they haven't, online play fails no matter what you change.", ev.game_name, a.kind)
                }],
                steps: vec!["Check the game's status on areweanticheatyet.com and ProtonDB before trying other fixes.".into()],
                priority: if a.blocks_linux { 1 } else { 35 },
                confidence: if a.blocks_linux { Confidence::Likely } else { Confidence::Unsure },
                ..Default::default()
            });
        }
    }

    // 9b. The game drew on the weak graphics chip built into the processor or motherboard.
    if let Some(l) = ev
        .launches
        .iter()
        .rev()
        .find(|l| l.gpu_integrated.is_some())
    {
        if l.gpu_integrated == Some(true) {
            out.push(Suggestion {
                id: "builtin_gpu".into(),
                title: "This game ran on the built-in graphics chip".into(),
                why: vec![format!(
                    "The last time {} ran, it drew on {} instead of your graphics card. That's much slower and can cause stutter or crashes.",
                    ev.game_name,
                    l.gpu.as_deref().unwrap_or("the built-in chip")
                )],
                steps: vec![
                    "Make sure \"Use the graphics card\" is on for this game in CachyPlay (it is unless you turned it off).".into(),
                    "Look in the game's own graphics settings for a \"Display adapter\" or \"GPU\" option and pick your graphics card.".into(),
                    "If your monitor is plugged into the motherboard, plug it into the graphics card.".into(),
                ],
                priority: 4,
                confidence: Confidence::Likely,
                ..Default::default()
            });
        }
    }

    // 10. Settings that games running fine here use.
    if trouble_here {
        let mine: Vec<String> = crate::insights::env_settings(&ev.user_options)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        for w in &ev.working {
            let envs: Vec<(String, String)> = crate::insights::env_settings(&w.launch_options)
                .into_iter()
                .filter(|(k, _)| crate::fixes::fix_env_allowed(k) && !mine.contains(k))
                .collect();
            if envs.is_empty() {
                continue;
            }
            let pairs: Vec<(&str, &str)> =
                envs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let id = format!("copy_{}", w.app_id);
            out.push(Suggestion {
                id: id.clone(),
                title: format!("Use the settings {} runs with", w.name),
                why: vec![
                    format!(
                        "{} ran {} recently without problems.",
                        w.name,
                        history::duration_text(chrono::Duration::minutes(w.minutes))
                    ),
                    format!(
                        "It starts with: {}",
                        envs.iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                ],
                fix: Some(fix(
                    &id,
                    &format!("Settings from {}", w.name),
                    &pairs,
                    &[],
                    false,
                )),
                status: ev.status_of(&id),
                priority: 32,
                confidence: Confidence::Unsure,
                ..Default::default()
            });
            break;
        }
    }

    // How sure each rule is, unless the rule set it itself.
    for s in out.iter_mut() {
        s.confidence = match s.id.as_str() {
            "game_update" | "launch_options_order" | "install_runtime" => Confidence::Likely,
            "disable_vulkan_layers" if several => Confidence::Likely,
            "driver_rollback" if s.priority >= 50 => Confidence::Unsure,
            "gpu_stock_clocks" | "capture_proton_log" => Confidence::Unsure,
            _ => s.confidence,
        };
    }
    // Ask "Did it work?" for fixes the game has been played with since and nobody answered for.
    for s in out.iter_mut() {
        let Some(f) = ev.fixes.fixes.iter().find(|f| f.id == s.id) else {
            continue;
        };
        if f.verdict.is_some() || f.temporary {
            continue;
        }
        let applied = chrono::DateTime::parse_from_rfc3339(&f.applied_at)
            .ok()
            .map(|d| d.naive_local());
        let played_since = ev
            .launches
            .iter()
            .any(|l| l.started_at > f.applied_at && !l.running)
            || applied.is_some_and(|t| {
                ev.sessions
                    .iter()
                    .any(|x| x.app_id == ev.app_id && x.start > t && x.end.is_some())
            });
        s.ask_verdict = played_since;
    }

    // The forced graphics mode broke the game's renderer: rule that fix out for this game.
    if let Some(u) = &ev.unity {
        if let (Some(why), Some(mode)) = (&u.mode_broken, &u.graphics) {
            let id = if mode.contains("12") {
                "unity_d3d12"
            } else if mode.contains("Vulkan") {
                "unity_vulkan"
            } else {
                ""
            };
            if let Some(s) = out.iter_mut().find(|s| s.id == id) {
                s.status = Status::DidntHelp;
                s.why.insert(0, why.clone());
            }
        }
    }
    // Ruled out earlier for this game (crashed, black screen, or undone because it made things
    // worse): say why, struck through, with "Try again".
    for s in out.iter_mut() {
        if let Some(t) = ev.tries.ruled_out(&s.id) {
            let when = t
                .off()
                .or(t.on())
                .map(|d| d.format("%b %-d").to_string())
                .unwrap_or_default();
            let why = format!(
                "Ruled out for this game ({when}): {}",
                t.ruled_out.clone().unwrap_or_default()
            );
            if !s.why.contains(&why) {
                s.why.insert(0, why);
            }
            s.status = Status::DidntHelp;
            s.try_again = true;
        }
    }
    crate::triage::extra_rules(ev, &mut out, &crate::pccheck::Root::system());
    crate::triage::mark_steam_tries(ev, &mut out);
    for s in out.iter_mut() {
        if let Some((game, _, why)) = ev.worked_elsewhere.iter().find(|(_, id, _)| *id == s.id) {
            if s.status != Status::DidntHelp {
                s.why.push(format!("A similar game was fixed by this: {game} ({why}). It may not apply here: each game's build decides."));
                // Another game's success is a hint, never more than "Possible".
                if s.confidence == Confidence::Unsure {
                    s.confidence = Confidence::Possible;
                }
            }
        }
    }
    crate::triage::adjust(ev, &mut out);
    out.sort_by_key(|s| (s.big_step, s.priority));
    out
}

/// Settings (`NAME=value`) after the game, or game options (`-flag`) before it. Returns the
/// misplaced words and the corrected launch options.
pub fn misplaced_launch_options(options: &str) -> Option<(Vec<String>, String)> {
    let is_env = |w: &str| {
        let mut parts = w.splitn(2, '=');
        let name = parts.next().unwrap_or("");
        parts.next().is_some()
            && !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            && name.chars().next().is_some_and(|c| !c.is_ascii_digit())
    };
    let words: Vec<&str> = options.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let cmd = words.iter().position(|w| *w == "%command%");
    let (before, after): (Vec<&str>, Vec<&str>) = match cmd {
        Some(i) => (words[..i].to_vec(), words[i + 1..].to_vec()),
        // Without %command%, Steam adds everything after the game.
        None => (vec![], words.clone()),
    };
    let late_env: Vec<&str> = after.iter().copied().filter(|w| is_env(w)).collect();
    // Game options before the program that starts the game ("-dx11 gamemoderun %command%").
    let early_args: Vec<&str> = if before.iter().any(|w| !w.starts_with('-') && !is_env(w)) {
        before
            .iter()
            .copied()
            .filter(|w| w.starts_with('-'))
            .collect()
    } else {
        vec![]
    };
    if late_env.is_empty() && early_args.is_empty() {
        return None;
    }
    let mut fixed: Vec<&str> = late_env.clone();
    fixed.extend(before.iter().copied().filter(|w| !early_args.contains(w)));
    fixed.push("%command%");
    fixed.extend(after.iter().copied().filter(|w| !late_env.contains(w)));
    fixed.extend(early_args.iter().copied());
    let bad = late_env
        .iter()
        .chain(early_args.iter())
        .map(|w| w.to_string())
        .collect();
    Some((bad, fixed.join(" ")))
}
