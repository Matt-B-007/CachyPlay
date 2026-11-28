// SPDX-License-Identifier: GPL-3.0-or-later
//! Texts for the people who can fix a problem: a short report for the game's developer, and a
//! prefilled GitHub issue to share that a fix worked (so it can become a tested fix for everyone).
//! Nothing is sent by CachyPlay; the user reads and sends it.

use crate::advice::{Evidence, Suggestion};
use crate::history;

/// A few lines a developer can act on, without personal details.
pub fn developer_report(ev: &Evidence, driver: Option<&str>, gpu: Option<&str>) -> String {
    let runs = history::runs(
        &ev.sessions,
        &ev.app_id,
        &ev.events.iter().collect::<Vec<_>>(),
    );
    let trouble = history::trouble(&runs);
    let mut lines = Vec::new();
    let version = ev
        .unity
        .as_ref()
        .and_then(|u| u.version.clone())
        .map(|v| format!(" {v}"))
        .unwrap_or_default();
    lines.push(format!(
        "{}{} crashes on Linux (Steam + Proton).",
        ev.game_name, version
    ));
    if let Some(t) = &trouble {
        let quick = t
            .since
            .iter()
            .filter_map(|r| r.session.duration())
            .map(|d| d.as_secs())
            .min();
        lines.push(format!(
            "Every start since {} fails ({} starts{}).",
            t.first_bad().strftime("%b %-d"),
            t.since.len(),
            quick
                .map(|s| format!(", the quickest after {s} seconds"))
                .unwrap_or_default()
        ));
        if let Some(g) = t.last_good {
            lines.push(format!(
                "It ran fine on {}.",
                g.session.start.strftime("%b %-d")
            ));
        }
    }
    let kinds: Vec<String> = {
        let mut k: Vec<String> = ev
            .events
            .iter()
            .filter(|e| e.app_id.as_deref() == Some(ev.app_id.as_str()))
            .flat_map(|e| {
                e.faults.iter().map(|f| match f.xid {
                    Some(x) => format!("NVIDIA Xid {x}"),
                    None => format!("{} GPU fault", f.vendor),
                })
            })
            .collect();
        k.sort();
        k.dedup();
        k
    };
    if !kinds.is_empty() {
        lines.push(format!(
            "Graphics driver errors during the crash: {}.",
            kinds.join(", ")
        ));
    }
    if let Some(u) = &ev.unity {
        if let Some(g) = &u.graphics {
            lines.push(format!(
                "Graphics mode in the game's last log: {g}{}{}.",
                if u.forced {
                    " (forced by the game)"
                } else {
                    ""
                },
                if u.frame_generation {
                    "; the game's frame generation is on"
                } else {
                    ""
                }
            ));
        }
    }
    let mut sys = Vec::new();
    if let Some(g) = gpu {
        sys.push(g.to_string());
    }
    if let Some(d) = driver {
        sys.push(format!("driver {d}"));
    }
    if !sys.is_empty() {
        lines.push(format!("PC: {}.", sys.join(", ")));
    }
    if let Some(t) = &trouble {
        let tools: Vec<String> = t
            .tools_tried()
            .into_iter()
            .map(|(name, _, _)| crate::compat::friendly(&name))
            .collect();
        if !tools.is_empty() {
            lines.push(format!("Tried with: {}.", tools.join(", ")));
        }
    }
    let fine: Vec<String> = ev
        .working
        .iter()
        .take(2)
        .map(|w| {
            format!(
                "{} ({})",
                w.name,
                history::duration_text(crate::clock::mins(w.minutes))
            )
        })
        .collect();
    if !fine.is_empty() {
        lines.push(format!(
            "Other games run fine on the same PC: {}.",
            fine.join(", ")
        ));
    }
    lines.push(
        "A CachyPlay bug report with the logs (personal details removed) is attached.".into(),
    );
    lines.join("\n")
}

/// The graphics card's name and driver version, for reports (best-effort, may be None).
pub fn gpu_and_driver() -> (Option<String>, Option<String>) {
    let driver = std::fs::read_to_string("/sys/module/nvidia/version")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let summary = crate::sys::run(
        "vulkaninfo",
        &["--summary"],
        std::time::Duration::from_secs(5),
    )
    .map(|(_, out, _)| out)
    .unwrap_or_default();
    let mut gpu = None;
    let mut discrete = false;
    for l in summary.lines() {
        let l = l.trim();
        if l.starts_with("deviceType") {
            discrete = l.contains("DISCRETE");
        }
        if let Some(name) = l.strip_prefix("deviceName") {
            let name = name.trim_start_matches([' ', '=']).trim().to_string();
            if gpu.is_none() || discrete {
                gpu = Some(name);
            }
            if discrete {
                break;
            }
        }
    }
    (gpu, driver)
}

/// Where players discuss the game on Steam.
pub fn discussions_url(app_id: &str) -> String {
    format!("https://steamcommunity.com/app/{app_id}/discussions/")
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A prefilled GitHub issue saying a fix worked. The user sees it in the browser and decides
/// whether to send it.
pub fn share_fix_url(
    ev: &Evidence,
    s: &Suggestion,
    driver: Option<&str>,
    gpu: Option<&str>,
) -> String {
    let title = format!(
        "Fix worked: {} for {} ({})",
        s.title, ev.game_name, ev.app_id
    );
    let mut body = vec![
        format!("**Game:** {} (AppID {})", ev.game_name, ev.app_id),
        format!("**Fix:** {} (`{}`)", s.title, s.id),
    ];
    if let Some(f) = &s.fix {
        let env: Vec<String> = f.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let tech = [env, f.args.clone()].concat().join(" ");
        if !tech.is_empty() {
            body.push(format!("**Setting:** `{tech}`"));
        }
    }
    if let Some(t) = &s.switch_tool {
        body.push(format!("**Compatibility tool:** {}", t.display));
    }
    if let Some(g) = gpu {
        body.push(format!("**GPU:** {g}"));
    }
    if let Some(d) = driver {
        body.push(format!("**Driver:** {d}"));
    }
    body.push(format!("**CachyPlay:** {}", crate::VERSION));
    body.push(String::new());
    body.push("_Sent from CachyPlay's \"Share that this worked\" button. Please add anything else that helped._".into());
    format!(
        "https://github.com/{}/issues/new?labels=fix-worked&title={}&body={}",
        crate::update::REPO,
        urlencode(&title),
        urlencode(&body.join("\n"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencodes_safely() {
        assert_eq!(urlencode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    }
}
