// SPDX-License-Identifier: GPL-3.0-or-later
//! Proton advice: which Proton to try next, never one the game already crashed on.

use super::*;

pub(crate) fn tool_display(ev: &Evidence, folder: &str) -> String {
    ev.tools
        .iter()
        .find(|t| t.folder == folder)
        .map(|t| t.display.clone())
        .unwrap_or_else(|| crate::compat::friendly(folder))
}

/// Suggest another compatibility tool, based on which ones were already tried since the trouble
/// began and how they did.
/// How to get the Proton builds that fix the most games, if they aren't installed.
pub(crate) fn install_proton_steps(tools: &[crate::compat::Tool]) -> Vec<String> {
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

/// Whether two Proton names mean the same build: Steam's internal name, the folder name in launch
/// commands, or its friendly form ("GE-Proton11-12-x86_64" = "GE-Proton11-12").
pub(crate) fn same_tool(a: &str, b: &str) -> bool {
    let norm = |x: &str| crate::compat::friendly(x).to_ascii_lowercase();
    norm(a) == norm(b)
}

/// The Proton builds this game's GPU driver crashes happened on (folder names, with how many
/// crashes), from every start Steam logged, not just the latest run of failures.
pub(crate) fn crashed_on(ev: &Evidence) -> Vec<(String, usize)> {
    let all: Vec<&CrashEvent> = ev.events.iter().collect();
    let mut out: Vec<(String, usize)> = Vec::new();
    for r in history::runs(&ev.sessions, &ev.app_id, &all) {
        if r.outcome != history::Outcome::Crashed {
            continue;
        }
        let Some(t) = &r.session.tool else { continue };
        match out.iter_mut().find(|(n, _)| same_tool(n, t)) {
            Some(e) => e.1 += 1,
            None => out.push((t.clone(), 1)),
        }
    }
    out
}

pub(crate) fn compat_suggestion(
    ev: &Evidence,
    trouble: Option<&history::Trouble>,
    day: &dyn Fn(DateTime) -> String,
) -> Option<Suggestion> {
    let tried = trouble.map(|t| t.tools_tried()).unwrap_or_default();
    let crashed = crashed_on(ev);
    let crashed_with = |t: &crate::compat::Tool| {
        crashed
            .iter()
            .find(|(f, _)| same_tool(f, &t.folder) || same_tool(f, &t.name))
            .map(|(_, n)| *n)
    };
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
        let with: Vec<&(String, usize, usize)> = tried
            .iter()
            .filter(|(f, _, _)| same_tool(f, &folder))
            .collect();
        // Switched to the Proton it was already crashing on: nothing to test, it's ruled out.
        let before = tool.as_ref().and_then(|t| {
            let n = crashed_with(t)?;
            let since = ev.switched_at?;
            let after = ev
                .events
                .iter()
                .filter(|e| e.app_id.as_deref() == Some(ev.app_id.as_str()) && e.at > since)
                .count();
            Some((n, after))
        });
        if let Some((n, _)) = before {
            why.insert(
                0,
                format!(
                    "The game already crashed on this Proton {n} time{} before the switch, so it can't be the fix.",
                    if n == 1 { "" } else { "s" }
                ),
            );
        }
        let status = match with.first() {
            _ if before.is_some() => Status::DidntHelp,
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
    let crashed_names: Vec<String> = crashed
        .iter()
        .map(|(f, n)| {
            let shown = ev
                .tools
                .iter()
                .find(|t| same_tool(&t.folder, f))
                .map(|t| t.display.clone())
                .unwrap_or_else(|| crate::compat::friendly(f));
            format!("{shown} ({n} crash{})", if *n == 1 { "" } else { "es" })
        })
        .collect();
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
        .filter(|t| !used.iter().any(|u| same_tool(u, &t.folder)))
        // Never the Proton the game already crashed on, or the one it's on now while failing.
        .filter(|t| crashed_with(t).is_none())
        .filter(|t| {
            ev.forced_tool
                .as_deref()
                .is_none_or(|f| !same_tool(f, &t.name))
                || (tried.is_empty() && crashed.is_empty())
        })
        .collect();
    candidates.sort_by_key(|t| rank(t));

    if let Some(next) = candidates.first() {
        if !crashed_names.is_empty() && why.is_empty() {
            why.push(format!("It crashed on: {}.", crashed_names.join(", ")));
        }
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
