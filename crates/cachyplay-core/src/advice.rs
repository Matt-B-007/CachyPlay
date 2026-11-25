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
use crate::history;
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
    /// Steam's session log, all games (start, end, compatibility tool).
    pub sessions: Vec<crashmatch::Session>,
    /// The user's own launch options for this game (without CachyPlay's part).
    pub user_options: String,
    /// Compatibility tools installed on this PC.
    pub tools: Vec<crate::compat::Tool>,
    /// The tool this game is forced to in Steam (internal name), if any.
    pub forced_tool: Option<String>,
    /// The tool CachyPlay switched this game to, if it did.
    pub switched_tool: Option<String>,
    /// What the game's Unity log says (graphics mode, frame generation, version).
    pub unity: Option<crate::insights::UnityInfo>,
    pub anti_cheat: Option<crate::insights::AntiCheat>,
    /// Other games that ran long and clean recently, and how they're set up.
    pub working: Vec<crate::insights::WorkingGame>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Not applied yet.
    #[default]
    New,
    /// Applied; no launch since.
    Applied,
    /// Applied, and the game has run normally since.
    Worked,
    /// Applied, but the game crashed again since.
    DidntHelp,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Strong, specific evidence points here.
    Likely,
    /// Worth trying; the evidence fits but doesn't prove it.
    #[default]
    Possible,
    /// A guess or a general tip; not enough information to say.
    Unsure,
}

#[derive(Debug, Clone, Default, Serialize)]
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
    /// A compatibility tool to switch this game to with one click (needs Steam closed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch_tool: Option<crate::compat::Tool>,
    /// Applied, played since, and the player hasn't said whether it helped: ask "Did it work?".
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ask_verdict: bool,
    /// How sure CachyPlay is: "likely", "possible" or "unsure".
    pub confidence: Confidence,
    /// CachyPlay already switched the tool; offer Undo.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub switched: bool,
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
        match fix.verdict {
            Some(true) => return Status::Worked,
            Some(false) => return Status::DidntHelp,
            None => {}
        }
        // A launch that's still going says nothing yet.
        let since: Vec<&LaunchRecord> = self
            .launches
            .iter()
            .filter(|l| l.started_at > fix.applied_at && !l.running)
            .collect();
        let applied_at = chrono::DateTime::parse_from_rfc3339(&fix.applied_at)
            .ok()
            .map(|d| d.naive_local());
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
        verdict: None,
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
        out.push(Suggestion {
            id: "launch_options_order".into(),
            title: "Fix the order of your launch options".into(),
            why: vec![
                format!("Your launch options are: {}", ev.user_options.trim()),
                format!(
                    "{} {} in the wrong place, so Steam passes {} to the game as plain text and {} nothing.",
                    bad.join(" "),
                    if bad.len() == 1 { "is" } else { "are" },
                    if bad.len() == 1 { "it" } else { "them" },
                    if bad.len() == 1 { "it does" } else { "they do" }
                ),
            ],
            steps: vec![
                "Settings like NAME=value go before %command%. Game options like -dx11 go after it.".into(),
                format!("In Steam: right-click the game → Properties → Launch options, and change it to: {fixed}"),
            ],
            priority: 8,
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
        let mut priority = 15;
        if let Some(u) = gpu_driver.last() {
            let fine = others_fine(u.at);
            if !fine.is_empty() {
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
    if let Some(u) = ev.unity.as_ref().filter(|_| trouble_here) {
        if u.graphics.as_deref() == Some("Direct3D 11") {
            let mut why: Vec<String> = Vec::new();
            why.push(format!(
                "{} draws with Direct3D 11{}, which Proton turns into Vulkan with DXVK.",
                ev.game_name,
                if u.forced {
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

    out.sort_by_key(|s| s.priority);
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

fn tool_display(ev: &Evidence, folder: &str) -> String {
    ev.tools
        .iter()
        .find(|t| t.folder == folder)
        .map(|t| t.display.clone())
        .unwrap_or_else(|| crate::compat::friendly(folder))
}

/// Suggest another compatibility tool, based on which ones were already tried since the trouble
/// began and how they did.
/// How to get the Proton builds that fix the most games, if they aren't installed.
fn install_proton_steps(tools: &[crate::compat::Tool]) -> Vec<String> {
    let has = |needle: &str| {
        tools
            .iter()
            .any(|t| t.name.to_ascii_lowercase().contains(needle))
    };
    let mut v = Vec::new();
    if !has("cachyos") {
        v.push("proton-cachyos isn't installed. It's tuned for CachyOS and fixes many games: install it in a terminal with  sudo pacman -S proton-cachyos  then restart Steam.".into());
    }
    if !has("ge-proton") {
        v.push("GE-Proton isn't installed. It adds fixes for games before Valve's Proton does: get it with ProtonUp-Qt (sudo pacman -S protonup-qt), then restart Steam.".into());
    }
    v
}

fn compat_suggestion(
    ev: &Evidence,
    trouble: Option<&history::Trouble>,
    day: &dyn Fn(NaiveDateTime) -> String,
) -> Option<Suggestion> {
    let tried = trouble.map(|t| t.tools_tried()).unwrap_or_default();
    let failed: Vec<&(String, usize, usize)> =
        tried.iter().filter(|(_, _, bad)| *bad > 0).collect();
    let mut why = Vec::new();
    if let (Some(t), false) = (trouble, tried.is_empty()) {
        why.push(format!(
            "Since {} it failed with: {}.",
            day(t.first_bad()),
            tried
                .iter()
                .map(|(f, n, bad)| format!("{} ({bad} of {n} starts)", tool_display(ev, f)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        if let Some(Some(tool)) = t.last_good.map(|g| &g.session.tool) {
            why.push(format!(
                "It last worked with {} on {}.",
                tool_display(ev, tool),
                day(t.last_good.unwrap().session.start)
            ));
        }
    }
    let manual = |display: &str| {
        format!(
            "In Steam: right-click {} → Properties → Compatibility → tick \"Force the use of a specific Steam Play compatibility tool\" → pick {display}.",
            ev.game_name
        )
    };

    // Already switched by CachyPlay: report how it went, offer Undo.
    if let Some(name) = &ev.switched_tool {
        let tool = ev.tools.iter().find(|t| &t.name == name).cloned();
        let folder = tool.as_ref().map(|t| t.folder.clone()).unwrap_or_default();
        let with: Vec<&(String, usize, usize)> =
            tried.iter().filter(|(f, _, _)| *f == folder).collect();
        let status = match with.first() {
            Some((_, _, bad)) if *bad > 0 => Status::DidntHelp,
            Some(_) => Status::Worked,
            None if trouble.is_none()
                && ev
                    .events
                    .iter()
                    .all(|e| e.app_id.as_deref() != Some(ev.app_id.as_str())) =>
            {
                Status::Worked
            }
            None => Status::Applied,
        };
        return Some(Suggestion {
            id: "compat_tool".into(),
            title: format!(
                "Switched to {}",
                tool.as_ref()
                    .map(|t| t.display.clone())
                    .unwrap_or_else(|| name.clone())
            ),
            why,
            steps: vec!["Undo puts the game back on the compatibility tool it used before.".into()],
            status,
            priority: 20,
            switch_tool: tool,
            switched: true,
            ..Default::default()
        });
    }

    let used: Vec<&str> = tried.iter().map(|(f, _, _)| f.as_str()).collect();
    // Prefer CachyOS's own Proton, then GE-Proton, then Valve's Experimental.
    let rank = |t: &crate::compat::Tool| {
        let n = t.name.to_lowercase();
        if n.contains("cachyos") {
            0
        } else if n.contains("ge-proton") {
            1
        } else if n == "proton_experimental" {
            2
        } else {
            3
        }
    };
    let mut candidates: Vec<&crate::compat::Tool> = ev
        .tools
        .iter()
        .filter(|t| !used.contains(&t.folder.as_str()))
        .filter(|t| ev.forced_tool.as_deref() != Some(t.name.as_str()) || tried.is_empty())
        .collect();
    candidates.sort_by_key(|t| rank(t));

    if let Some(next) = candidates.first() {
        let long_shot = failed.len() >= 2;
        if long_shot {
            why.push(format!(
                "{} different versions failed the same way, so Proton probably isn't the cause. Still, it's quick to rule out.",
                failed.len()
            ));
        } else if why.is_empty() {
            why.push("Some games only work with certain Proton versions.".into());
        }
        Some(Suggestion {
            id: "compat_tool".into(),
            title: format!("Try {} for this game", next.display),
            why,
            steps: vec![
                manual(&next.display),
                "Or use the Switch button below (close Steam first). Undo puts it back.".into(),
            ],
            priority: if long_shot { 55 } else { 22 },
            switch_tool: Some((*next).clone()),
            ..Default::default()
        })
    } else if failed.len() >= 2 {
        why.push(
            "Every Proton version you have failed the same way, so switching again won't fix this."
                .into(),
        );
        Some(Suggestion {
            id: "compat_tool".into(),
            title: "Changing the Proton version won't fix this".into(),
            why,
            steps: install_proton_steps(&ev.tools),
            status: Status::DidntHelp,
            priority: 70,
            ..Default::default()
        })
    } else if ev.tools.is_empty() {
        Some(Suggestion {
            id: "compat_tool".into(),
            title: "Try a different Proton version".into(),
            why: if why.is_empty() {
                vec!["Some games only work with certain Proton versions.".into()]
            } else {
                why
            },
            steps: {
                let mut v = vec![manual(
                    "Proton Experimental (or GE-Proton / proton-cachyos)",
                )];
                v.extend(install_proton_steps(&ev.tools));
                v
            },
            priority: 25,
            ..Default::default()
        })
    } else {
        None
    }
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
    let unity = unity_logs
        .first()
        .and_then(|p| crate::fsutil::read_capped(p, 400, 400))
        .map(|t| crate::insights::unity_info(&t, &game.name));
    let sessions = crashmatch::parse_steam_sessions(&ctx.steam.console_logs());
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
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            .map(|t| t.naive_local()),
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
        cachyplay_on: status.as_ref().map(|s| s.applied).unwrap_or(false),
        sessions,
        user_options: status.map(|s| s.user_options).unwrap_or_default(),
        tools: crate::compat::installed(&ctx.steam.root, &ctx.steam.library_folders()),
        forced_tool: ctx.steam.compat_tool(&game.app_id),
        switched_tool: crate::compat::switched_to(ctx, &game.app_id),
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

fn confidence_word(c: Confidence) -> &'static str {
    match c {
        Confidence::Likely => "Likely",
        Confidence::Possible => "Possibly",
        Confidence::Unsure => "Not sure yet",
    }
}

/// Lower-case the first letter, for joining a title into a sentence.
fn sentence_case(s: &str) -> String {
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
    let day = |t: NaiveDateTime| t.format("%b %-d").to_string();

    let worked_fix = suggestions
        .iter()
        .any(|s| s.fix.is_some() && s.status == Status::Worked);
    let blocked = suggestions
        .iter()
        .any(|s| s.id == "anti_cheat" && s.confidence == Confidence::Likely);
    // A fix (or Proton switch) that's applied and hasn't failed: try that before anything else.
    let pending = suggestions.iter().find(|s| {
        (s.fix.is_some() || s.switched) && matches!(s.status, Status::Applied | Status::Worked)
    });
    if let (Some(p), false) = (pending, blocked) {
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
    let (light, what) = if blocked {
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
            Light::Yellow,
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
    pub has_prefix: bool,
}

pub fn insights(ctx: &crate::manage::Ctx, game: &crate::steam::Game) -> Insights {
    let ev = gather(ctx, game, &crate::evidence::kernel_logs());
    let suggestions = suggest(&ev);
    let summary = summary(&ev, &suggestions);
    let (gpu, driver) = crate::outreach::gpu_and_driver();
    let share_urls = suggestions
        .iter()
        .filter(|s| s.status == Status::Worked)
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
        has_prefix: ctx.steam.compatdata(game).is_some(),
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
            verdict: None,
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
            running: false,
        };
        // A launch that's still going says nothing yet, and nobody is asked.
        ev.launches = vec![LaunchRecord {
            running: true,
            ..rec("2026-11-23T18:55:00-08:00", false)
        }];
        assert_eq!(suggest(&ev)[0].status, Status::Applied);
        assert!(!suggest(&ev)[0].ask_verdict);
        ev.launches = vec![rec("2026-11-23T18:55:00-08:00", false)];
        assert_eq!(suggest(&ev)[0].status, Status::Worked);
        assert!(
            suggest(&ev)[0].ask_verdict,
            "played since: ask if it worked"
        );
        // The player's answer beats the guess.
        ev.fixes.fixes[0].verdict = Some(false);
        assert_eq!(suggest(&ev)[0].status, Status::DidntHelp);
        assert!(!suggest(&ev)[0].ask_verdict);
        ev.fixes.fixes[0].verdict = None;
        ev.launches.push(rec("2026-11-23T20:00:00-08:00", true));
        assert_eq!(suggest(&ev)[0].status, Status::DidntHelp);
    }

    /// The Nov 24 report: Moon Sweepers worked on Nov 15, the game updated Nov 18, it has failed
    /// every start since Nov 22 with three Proton versions, the driver changed Nov 23, and
    /// Cyberpunk ran for 5 hours on the new driver.
    fn matt_nov24() -> Evidence {
        let exp = "~/.local/share/Steam/steamapps/common/Proton - Experimental";
        let ge12 = "~/.local/share/Steam/compatibilitytools.d/GE-Proton11-12-x86_64";
        let cachy = "/usr/share/steam/compatibilitytools.d/proton-cachyos-slr";
        let ge11 = "~/.local/share/Steam/compatibilitytools.d/GE-Proton11-11-x86_64";
        let line = |t: &str, kind: &str, app: &str, tool: &str| {
            format!("[{t}] Game process {kind} : AppID {app} \"reaper SteamLaunch AppId={app} -- '{tool}'/proton waitforexitandrun  'x.exe'\", ProcID 1\n")
        };
        let mut log = String::new();
        for (a, b, app, tool) in [
            ("2026-11-15 08:38:19", "2026-11-15 11:01:54", "5335510", exp),
            ("2026-11-22 09:42:45", "2026-11-22 09:44:45", "5335510", exp),
            (
                "2026-11-22 09:46:08",
                "2026-11-22 09:47:16",
                "5335510",
                ge12,
            ),
            (
                "2026-11-22 09:50:35",
                "2026-11-22 09:50:48",
                "5335510",
                cachy,
            ),
            ("2026-11-23 12:11:17", "2026-11-23 12:12:17", "5335510", exp),
            ("2026-11-23 18:26:35", "2026-11-23 18:27:12", "5335510", exp),
            (
                "2026-11-23 18:36:34",
                "2026-11-23 18:40:34",
                "1240440",
                ge11,
            ),
            (
                "2026-11-24 07:30:33",
                "2026-11-24 12:49:27",
                "1091500",
                ge12,
            ),
            ("2026-11-24 13:51:51", "2026-11-24 13:52:40", "5335510", exp),
        ] {
            log += &line(a, "added", app, tool);
            log += &line(b, "removed", app, tool);
        }
        let sessions = parse_steam_sessions(&log);
        let kernel = "2026-11-23T12:11:26-08:00 h kernel: NVRM: Xid (PCI:0000:02:00): 109, pid=4874, name=Unity Main Thre, channel 0xa
2026-11-23T18:26:44-08:00 h kernel: NVRM: Xid (PCI:0000:02:00): 109, pid=224093, name=Unity Main Thre, channel 0x10
2026-11-23T18:40:28-08:00 h kernel: NVRM: Xid (PCI:0000:02:00): 109, pid=236921, name=00 MAIN, channel 0x10
2026-11-24T13:52:01-08:00 h kernel: NVRM: Xid (PCI:0000:02:00): 109, pid=395739, name=Unity Main Thre, channel 0x1e";
        let now = at(2026, 11, 24, 13, 53, 0);
        let events = crashmatch::crash_events(&parse_kernel_faults(kernel, now), &sessions, now);
        let tool = |name: &str, folder: &str, display: &str| crate::compat::Tool {
            name: name.into(),
            folder: folder.into(),
            display: display.into(),
        };
        Evidence {
            app_id: "5335510".into(),
            game_name: "Moon Sweepers".into(),
            game_updated: Some(at(2026, 11, 18, 18, 47, 5)),
            events,
            names: [
                ("5335510".to_string(), "Moon Sweepers".to_string()),
                ("1240440".to_string(), "Halo Infinite".to_string()),
                ("1091500".to_string(), "Cyberpunk 2077".to_string()),
            ]
            .into(),
            driver_updates: vec![DriverUpdate {
                at: at(2026, 11, 23, 12, 7, 7),
                package: "linux-cachyos-nvidia-open".into(),
                from: "7.3.5-2".into(),
                to: "7.3.6-1".into(),
            }],
            uses_proton: true,
            is_unity: true,
            has_proton_log: true,
            sessions,
            tools: vec![
                tool(
                    "proton_experimental",
                    "Proton - Experimental",
                    "Proton Experimental",
                ),
                tool("GE-Proton11-11", "GE-Proton11-11-x86_64", "GE-Proton11-11"),
                tool("GE-Proton11-12", "GE-Proton11-12-x86_64", "GE-Proton11-12"),
                tool(
                    "proton-cachyos-slr",
                    "proton-cachyos-slr",
                    "proton-cachyos (SLR)",
                ),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn nov24_points_at_the_game_update_not_the_driver() {
        let s = suggest(&matt_nov24());
        let ids: Vec<&str> = s.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids[0], "game_update", "{ids:?}");
        let top = &s[0];
        assert_eq!(top.title, "This started with the game's Nov 18 update");
        let why = top.why.join(" | ");
        assert!(why.contains("ran normally on Nov 15 (2 h 23 min)"), "{why}");
        assert!(
            why.contains("Since Nov 22, all 6 starts failed (3 starts with a GPU driver crash)"),
            "{why}"
        );
        assert!(
            why.contains("Cyberpunk 2077 ran 5 h 18 min on Nov 24"),
            "{why}"
        );

        let driver = s.iter().find(|x| x.id == "driver_rollback").unwrap();
        assert_eq!(driver.priority, 50);
        let dwhy = driver.why.join(" | ");
        assert!(
            dwhy.contains("same driver")
                && dwhy.contains("started failing on Nov 22, before this update"),
            "{dwhy}"
        );

        let compat = s.iter().find(|x| x.id == "compat_tool").unwrap();
        assert_eq!(compat.title, "Try GE-Proton11-11 for this game");
        assert_eq!(compat.switch_tool.as_ref().unwrap().name, "GE-Proton11-11");
        let cwhy = compat.why.join(" | ");
        assert!(cwhy.contains("Proton Experimental (4 of 4 starts), GE-Proton11-12 (1 of 1 starts), proton-cachyos (SLR) (1 of 1 starts)"), "{cwhy}");
        assert!(
            cwhy.contains("last worked with Proton Experimental on Nov 15")
                && cwhy.contains("3 different versions"),
            "{cwhy}"
        );
        assert!(compat.priority > driver.priority, "a long shot comes late");
    }

    /// The real Nov 24 crash with everything CachyPlay now reads: the Unity log (Direct3D 11,
    /// forced; its own FSR frame generation) and how the games that run fine are set up.
    #[test]
    fn nov24_full_picture_regression() {
        let mut ev = matt_nov24();
        ev.unity = Some(crate::insights::UnityInfo {
            graphics: Some("Direct3D 11".into()),
            forced: true,
            frame_generation: true,
            version: Some("1.0.6".into()),
        });
        ev.working = vec![crate::insights::WorkingGame {
            app_id: "1091500".into(),
            name: "Cyberpunk 2077".into(),
            minutes: 318,
            tool: Some("GE-Proton11-12-x86_64".into()),
            launch_options: "obs-gamecapture %command%".into(),
        }];
        let s = suggest(&ev);
        let ids: Vec<&str> = s.iter().map(|x| x.id.as_str()).collect();
        // The cause first, then the things to try in the game, then long shots.
        assert_eq!(ids[0], "game_update", "{ids:?}");
        assert_eq!(s[0].confidence, Confidence::Likely);
        let pos = |id: &str| {
            ids.iter()
                .position(|x| *x == id)
                .unwrap_or_else(|| panic!("{id} missing: {ids:?}"))
        };
        assert!(pos("game_frame_generation") < pos("unity_d3d12"));
        assert!(pos("unity_d3d12") < pos("unity_vulkan"));
        assert!(pos("unity_vulkan") < pos("driver_rollback"));
        let d3d12 = &s[pos("unity_d3d12")];
        assert_eq!(d3d12.fix.as_ref().unwrap().args, vec!["-force-d3d12"]);
        assert!(d3d12.why.join(" ").contains("Cyberpunk 2077"));
        // Never confidently blame the driver other games run fine on.
        assert_eq!(s[pos("driver_rollback")].confidence, Confidence::Unsure);
        // Cyberpunk only adds a wrapper, no NAME=value settings: nothing to copy.
        assert!(!ids.iter().any(|i| i.starts_with("copy_")));
        // No anti-cheat in this game.
        assert!(!ids.contains(&"anti_cheat"));
    }

    #[test]
    fn nov24_summary_is_three_plain_lines() {
        let mut ev = matt_nov24();
        ev.unity = Some(crate::insights::UnityInfo {
            graphics: Some("Direct3D 11".into()),
            forced: true,
            frame_generation: true,
            version: None,
        });
        let s = suggest(&ev);
        let sum = summary(&ev, &s);
        assert_eq!(sum.light, Light::Red);
        assert!(
            sum.what
                .starts_with("Every start since Nov 22 failed (6 times)"),
            "{}",
            sum.what
        );
        assert_eq!(
            sum.why.as_deref(),
            Some("Likely: this started with the game's Nov 18 update.")
        );
        assert!(
            sum.todo
                .as_deref()
                .unwrap()
                .starts_with("Check the game's news"),
            "{:?}",
            sum.todo
        );
        assert_eq!(sum.fix_id.as_deref(), Some("unity_d3d12"));
    }

    #[test]
    fn developer_report_is_short_and_specific() {
        let mut ev = matt_nov24();
        ev.unity = Some(crate::insights::UnityInfo {
            graphics: Some("Direct3D 11".into()),
            forced: true,
            frame_generation: true,
            version: Some("1.0.6".into()),
        });
        let t = crate::outreach::developer_report(
            &ev,
            Some("620.73.04"),
            Some("NVIDIA GeForce RTX 5090"),
        );
        assert!(t.starts_with("Moon Sweepers 1.0.6 crashes on Linux"), "{t}");
        assert!(
            t.contains("Every start since Nov 22 fails (6 starts"),
            "{t}"
        );
        assert!(t.contains("NVIDIA Xid 109"), "{t}");
        assert!(
            t.contains("Direct3D 11 (forced by the game); the game's frame generation is on"),
            "{t}"
        );
        assert!(
            t.contains("NVIDIA GeForce RTX 5090, driver 620.73.04"),
            "{t}"
        );
        assert!(t.lines().count() <= 10);
        let s = suggest(&ev);
        let url = crate::outreach::share_fix_url(
            &ev,
            s.iter().find(|x| x.fix.is_some()).unwrap(),
            None,
            None,
        );
        assert!(url.starts_with("https://github.com/Matt-B-007/CachyPlay/issues/new?labels=fix-worked&title=Fix%20worked"), "{url}");
    }

    #[test]
    fn summary_waits_for_an_applied_fix_before_offering_another() {
        let mut ev = matt_nov24();
        ev.unity = Some(crate::insights::UnityInfo {
            graphics: Some("Direct3D 11".into()),
            forced: true,
            frame_generation: false,
            version: None,
        });
        ev.fixes.fixes.push(fix(
            "unity_d3d12",
            "Start in Direct3D 12",
            &[],
            &["-force-d3d12"],
            false,
        ));
        ev.fixes.fixes[0].applied_at = "2026-11-24T23:00:00-08:00".into();
        let sum = summary(&ev, &suggest(&ev));
        assert_eq!(sum.light, Light::Yellow);
        assert!(
            sum.what
                .contains("You applied \"Try the game's Direct3D 12 mode\""),
            "{}",
            sum.what
        );
        assert_eq!(
            sum.fix_id, None,
            "no stacking fixes while one is being tried"
        );
        // Once the player says it didn't help, the next fix is offered.
        ev.fixes.fixes[0].verdict = Some(false);
        let sum = summary(&ev, &suggest(&ev));
        assert_eq!(sum.light, Light::Red);
        assert!(sum.fix_id.is_some());
    }

    #[test]
    fn copies_settings_from_a_game_that_works() {
        let mut ev = matt_nov24();
        ev.working = vec![crate::insights::WorkingGame {
            app_id: "1240440".into(),
            name: "Halo Infinite".into(),
            minutes: 120,
            tool: Some("GE-Proton11-11-x86_64".into()),
            launch_options: "PROTON_ENABLE_NVAPI=1 SOMETHING_ELSE=1 %command%".into(),
        }];
        let s = suggest(&ev);
        let c = s.iter().find(|x| x.id == "copy_1240440").unwrap();
        let f = c.fix.as_ref().unwrap();
        // Only settings CachyPlay allows in fixes are copied.
        assert_eq!(
            f.env.get("PROTON_ENABLE_NVAPI").map(String::as_str),
            Some("1")
        );
        assert!(!f.env.contains_key("SOMETHING_ELSE"));
        assert_eq!(c.confidence, Confidence::Unsure);
    }

    #[test]
    fn warns_first_about_anti_cheat_that_blocks_linux() {
        let mut ev = matt_nov24();
        ev.anti_cheat = Some(crate::insights::AntiCheat {
            kind: "BattlEye".into(),
            blocks_linux: true,
        });
        let s = suggest(&ev);
        assert_eq!(s[0].id, "anti_cheat");
        assert_eq!(s[0].confidence, Confidence::Likely);
    }

    #[test]
    fn says_so_when_every_proton_failed() {
        let mut ev = matt_nov24();
        ev.tools.retain(|t| t.name != "GE-Proton11-11");
        let s = suggest(&ev);
        let compat = s.iter().find(|x| x.id == "compat_tool").unwrap();
        assert_eq!(compat.title, "Changing the Proton version won't fix this");
        assert_eq!(compat.status, Status::DidntHelp);
        assert!(compat.switch_tool.is_none());
    }

    #[test]
    fn spots_launch_options_in_the_wrong_order() {
        assert_eq!(
            misplaced_launch_options("PROTON_ENABLE_NVAPI=1"),
            Some((
                vec!["PROTON_ENABLE_NVAPI=1".into()],
                "PROTON_ENABLE_NVAPI=1 %command%".into()
            ))
        );
        assert_eq!(
            misplaced_launch_options("%command% -nosplash DXVK_HUD=1"),
            Some((
                vec!["DXVK_HUD=1".into()],
                "DXVK_HUD=1 %command% -nosplash".into()
            ))
        );
        assert_eq!(
            misplaced_launch_options("-force-d3d11 obs-gamecapture %command%"),
            Some((
                vec!["-force-d3d11".into()],
                "obs-gamecapture %command% -force-d3d11".into()
            ))
        );
        assert_eq!(
            misplaced_launch_options("PROTON_LOG=1 gamemoderun %command% -dx11"),
            None
        );
        assert_eq!(misplaced_launch_options("-nosplash"), None);
        assert_eq!(misplaced_launch_options(""), None);
        let mut ev = matt_nov24();
        ev.user_options = "PROTON_ENABLE_NVAPI=1".into();
        let s = suggest(&ev);
        let f = s.iter().find(|x| x.id == "launch_options_order").unwrap();
        assert!(
            f.steps[1].ends_with("PROTON_ENABLE_NVAPI=1 %command%"),
            "{:?}",
            f.steps
        );
    }
}
