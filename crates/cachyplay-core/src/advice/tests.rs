// SPDX-License-Identifier: GPL-3.0-or-later
#[test]
fn every_applied_change_is_listed_with_undo() {
    let f = crate::testutil::FakeSteam::new();
    let ctx = f.ctx();
    let g = ctx.steam.find_game("1240440").unwrap();
    crate::fixes::apply(
        &ctx.paths,
        "1240440",
        fix(
            "unity_d3d12",
            "Start in Direct3D 12",
            &[],
            &["-force-d3d12"],
            false,
        ),
    )
    .unwrap();
    crate::fixes::set_your_settings(&ctx.paths, "1240440", "VKD3D_CONFIG=no_upload_hvv").unwrap();
    let ins = insights(&ctx, &g);
    let ids: Vec<&str> = ins.applied.iter().map(|a| a.id.as_str()).collect();
    assert!(
        ids.contains(&"unity_d3d12") && ids.contains(&"your_settings"),
        "{ids:?}"
    );
    // Both went on since the last start: stacking another should be warned about.
    assert_eq!(
        ins.changed_since_start.len(),
        2,
        "{:?}",
        ins.changed_since_start
    );
    // Undoing because it made things worse keeps it crossed off.
    crate::tries::record_off(&ctx.paths, "1240440", "unity_d3d12", Some("worse")).unwrap();
    crate::fixes::remove(&ctx.paths, "1240440", "unity_d3d12").unwrap();
    let ins = insights(&ctx, &g);
    assert!(!ins.applied.iter().any(|a| a.id == "unity_d3d12"));
    assert!(
        ins.find_cause
            .ruled_out
            .iter()
            .any(|r| r.contains("Direct3D 12")),
        "{:?}",
        ins.find_cause.ruled_out
    );
}

#[test]
fn a_forced_mode_that_breaks_the_game_stays_visible_and_ruled_out() {
    // SHRED A BILLION, Nov 25: "Start in Direct3D 12" gave a black screen. The game's log then
    // said Direct3D 12, so the suggestion (and its Undo) used to disappear.
    let log = "Initialize engine version: 6000.0.74f1\nForcing GfxDevice: Direct3D 12\nKernel 'MainUploadScatterInstances' not found\n";
    let mut ev = Evidence {
        app_id: "5472080".into(),
        game_name: "SHRED A BILLION".into(),
        uses_proton: true,
        unity: Some(crate::insights::unity_info(log, "SHRED A BILLION")),
        ..Default::default()
    };
    ev.fixes.fixes.push(fix(
        "unity_d3d12",
        "Start in Direct3D 12",
        &[],
        &["-force-d3d12"],
        false,
    ));
    let out = suggest(&ev);
    let d3d12 = out
        .iter()
        .find(|s| s.id == "unity_d3d12")
        .expect("still listed while it's on");
    assert_eq!(d3d12.status, Status::DidntHelp);
    assert!(
        d3d12.why[0].contains("MainUploadScatterInstances"),
        "{:?}",
        d3d12.why
    );
    // After Undo, the try is remembered: still listed, still ruled out, with "Try again".
    ev.fixes.fixes.clear();
    ev.unity = Some(crate::insights::unity_info(
        "Version:  Direct3D 11.0 [level 11.1]\n",
        "SHRED A BILLION",
    ));
    ev.tries.tries.push(crate::tries::Try {
        id: "unity_d3d12".into(),
        label: "Try the game's Direct3D 12 mode".into(),
        on_at: "2026-11-25T21:07:07-08:00".into(),
        off_at: Some("2026-11-25T21:30:00-08:00".into()),
        off_reason: None,
        ruled_out: Some("Black screen: the game doesn't include Direct3D 12.".into()),
    });
    let out = suggest(&ev);
    let d3d12 = out.iter().find(|s| s.id == "unity_d3d12").unwrap();
    assert_eq!(d3d12.status, Status::DidntHelp);
    assert!(d3d12.try_again);
    assert!(
        d3d12.why[0].starts_with("Ruled out for this game (Nov 25): Black screen"),
        "{:?}",
        d3d12.why
    );
}

use super::*;
use crate::crashmatch::{at, parse_kernel_faults, parse_steam_sessions};

const LOG: &str = include_str!("../../tests/fixtures/kernel-warnings-short.log");
const CONSOLE: &str = "[2026-11-23 12:09:58] Game process added : AppID 5335510 \"x\", ProcID 1\n\
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
        ..Default::default()
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
    // Closing early without a crash proves nothing: the player may have closed it.
    ev.launches.push(rec("2026-11-23T20:00:00-08:00", true));
    assert_eq!(suggest(&ev)[0].status, Status::Worked);
    // Undone after it made things worse: stays ruled out for this game.
    let saved = ev.fixes.clone();
    ev.fixes.fixes.clear();
    ev.tries.tries.push(crate::tries::Try {
        id: saved.fixes[0].id.clone(),
        label: saved.fixes[0].label.clone(),
        on_at: "2026-11-23T18:00:00-08:00".into(),
        off_at: Some("2026-11-23T20:10:00-08:00".into()),
        off_reason: Some("worse".into()),
        ruled_out: Some("You undid it because it made things worse.".into()),
    });
    assert_eq!(
        suggest(&ev)
            .iter()
            .find(|x| x.id == saved.fixes[0].id)
            .unwrap()
            .status,
        Status::DidntHelp
    );
    ev.tries.tries.clear();
    ev.fixes = saved;
    // Ran on the built-in chip last time: say so near the top.
    ev.launches.push(LaunchRecord {
        gpu: Some("the built-in AMD graphics chip".into()),
        gpu_integrated: Some(true),
        ..rec("2026-11-23T21:00:00-08:00", false)
    });
    let s = suggest(&ev);
    let b = s
        .iter()
        .find(|x| x.id == "builtin_gpu")
        .expect("builtin_gpu suggestion");
    assert!(b.why[0].contains("built-in AMD graphics chip"));
    // On the card the next time: the suggestion goes away.
    ev.launches.push(LaunchRecord {
        gpu: Some("the NVIDIA graphics card".into()),
        gpu_integrated: Some(false),
        ..rec("2026-11-23T22:00:00-08:00", false)
    });
    assert!(suggest(&ev).iter().all(|x| x.id != "builtin_gpu"));
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

/// Halo Infinite, Nov 25–27 (Matt's beta.17 report): seven crashes on proton-cachyos-slr,
/// then CachyPlay "switched" the game to that same Proton.
fn halo_nov27(switched: bool) -> Evidence {
    let cachy = "/usr/share/steam/compatibilitytools.d/proton-cachyos-slr";
    let ge11 = "~/.local/share/Steam/compatibilitytools.d/GE-Proton11-11-x86_64";
    let line = |t: &str, kind: &str, tool: &str| {
        format!("[{t}] Game process {kind} : AppID 1240440 \"reaper SteamLaunch AppId=1240440 -- '{tool}'/proton waitforexitandrun  'x.exe'\", ProcID 1\n")
    };
    let mut log = String::new();
    for (a, b, tool) in [
        ("2026-11-23 18:28:28", "2026-11-23 18:32:28", ge11),
        ("2026-11-24 13:55:36", "2026-11-24 14:00:36", ge11),
        ("2026-11-25 16:48:43", "2026-11-25 16:53:50", ge11),
        ("2026-11-25 16:55:43", "2026-11-25 16:59:25", cachy),
        ("2026-11-25 17:04:59", "2026-11-25 17:08:25", cachy),
        ("2026-11-25 17:13:27", "2026-11-25 17:16:25", cachy),
        ("2026-11-25 17:20:10", "2026-11-25 17:29:33", cachy),
        ("2026-11-25 19:20:06", "2026-11-25 19:26:51", cachy),
        ("2026-11-25 19:36:05", "2026-11-25 19:41:59", cachy),
    ] {
        log += &line(a, "added", tool);
        log += &line(b, "removed", tool);
    }
    let sessions = parse_steam_sessions(&log);
    let kernel: String = ["2026-11-25T16:53:47", "2026-11-25T16:59:21", "2026-11-25T17:08:21", "2026-11-25T17:16:21", "2026-11-25T17:29:29", "2026-11-25T19:26:47", "2026-11-25T19:41:55"]
        .iter()
        .map(|t| format!("{t}-08:00 h kernel: NVRM: Xid (PCI:0000:02:00): 109, pid=1, name=HaloInfinite.ex, channel 0x10\n"))
        .collect();
    let now = at(2026, 11, 27, 8, 7, 0);
    let events = crashmatch::crash_events(&parse_kernel_faults(&kernel, now), &sessions, now);
    let tool = |name: &str, folder: &str, display: &str| crate::compat::Tool {
        name: name.into(),
        folder: folder.into(),
        display: display.into(),
    };
    Evidence {
        app_id: "1240440".into(),
        game_name: "Halo Infinite".into(),
        events,
        sessions,
        tools: vec![
            tool(
                "proton_experimental",
                "Proton - Experimental",
                "Proton Experimental",
            ),
            tool("GE-Proton11-11", "GE-Proton11-11-x86_64", "GE-Proton11-11"),
            tool(
                "proton-cachyos-slr",
                "proton-cachyos-slr",
                "proton-cachyos-11.0-20261015 (steam linux runtime)",
            ),
        ],
        forced_tool: Some("proton-cachyos-slr".into()),
        uses_proton: true,
        switched_tool: switched.then(|| "proton-cachyos-slr".into()),
        switched_at: switched.then(|| at(2026, 11, 26, 9, 0, 0)),
        now: Some(now),
        ..Default::default()
    }
}

#[test]
fn driver_history_is_judged_from_the_first_crash() {
    let up = |at: DateTime, p: &str, from: &str, to: &str| crate::evidence::DriverUpdate {
        at,
        package: p.into(),
        from: from.into(),
        to: to.into(),
    };
    let mut ev = halo_nov27(false);
    ev.driver_updates = vec![
        up(
            at(2026, 11, 20, 6, 20, 38),
            "nvidia-utils",
            "620.70.06-2",
            "620.73.04-1",
        ),
        up(
            at(2026, 11, 21, 5, 54, 46),
            "nvidia-settings",
            "620.70.06-1",
            "620.73.04-1",
        ),
        up(
            at(2026, 11, 25, 7, 48, 49),
            "linux-cachyos-nvidia-open",
            "7.3.6-1",
            "7.3.6-2",
        ),
        up(
            at(2026, 11, 26, 13, 14, 25),
            "linux-cachyos-nvidia-open",
            "7.3.6-2",
            "7.3.6-3",
        ),
    ];
    let c = driver_context(&ev).unwrap();
    assert_eq!(
        c.closest.unwrap().to,
        "7.3.6-2",
        "the update just before the first crash, not the Nov 26 one"
    );
    assert_eq!(
        c.version_change.unwrap().package,
        "nvidia-utils",
        "nvidia-settings isn't the driver"
    );
    assert_eq!(
        c.ran_after.len(),
        2,
        "Nov 23 and Nov 24 ran without crashing"
    );
    let note = ran_after_note(&c).unwrap();
    assert!(note.contains("too short to clear it"), "{note}");
    assert_eq!(c.after.len(), 1);
    let line = crate::report::driver_timeline(&ev).unwrap();
    assert!(
        line.contains("9 hours after linux-cachyos-nvidia-open 7.3.6-1 → 7.3.6-2"),
        "{line}"
    );
    assert!(
        line.contains("last changed on Nov 20 (nvidia-utils"),
        "{line}"
    );
    assert!(
        !line.contains("nvidia-settings") && !line.contains("7.3.6-3"),
        "{line}"
    );
    assert!(is_driver_package("lib32-nvidia-utils") && !is_driver_package("opencl-nvidia"));
    assert!(
        !is_driver_package("linux-cachyos") && is_driver_package("linux-cachyos-lts-nvidia-open")
    );
}

#[test]
fn never_offers_or_counts_a_switch_to_the_proton_it_crashed_on() {
    // Not switched yet: proton-cachyos-slr (6 crashes) is never the suggestion.
    let s = suggest(&halo_nov27(false));
    if let Some(c) = s.iter().find(|x| x.id == "compat_tool") {
        let t = c.switch_tool.as_ref().map(|t| t.name.as_str());
        assert_ne!(t, Some("proton-cachyos-slr"), "{}", c.title);
        assert_ne!(t, Some("GE-Proton11-11"), "{}", c.title);
    }
    // Switched to it anyway: it's crossed off at once, with the reason.
    let s = suggest(&halo_nov27(true));
    let c = s.iter().find(|x| x.id == "compat_tool").unwrap();
    assert_eq!(c.status, Status::DidntHelp, "{c:?}");
    assert!(
        c.why
            .join(" ")
            .contains("already crashed on this Proton 6 times"),
        "{:?}",
        c.why
    );
    // Names for the same build match: folder, internal name and the -x86_64 form.
    assert!(same_tool("GE-Proton11-11-x86_64", "GE-Proton11-11"));
    assert!(same_tool("proton-cachyos-slr", "proton-cachyos-slr"));
    assert!(!same_tool("proton-cachyos", "proton-cachyos-slr"));
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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
    });
    let t =
        crate::outreach::developer_report(&ev, Some("620.73.04"), Some("NVIDIA GeForce RTX 5090"));
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
        ..Default::default()
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
            .starts_with("Testing the fix: Start in Direct3D 12"),
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
