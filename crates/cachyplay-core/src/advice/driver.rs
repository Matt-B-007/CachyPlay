// SPDX-License-Identifier: GPL-3.0-or-later
//! Driver advice: what the graphics driver history says about when a game's crashes started.

use super::*;

/// Whether a pacman package is part of the graphics driver: NVIDIA's driver and its kernel
/// modules, Mesa and its Vulkan drivers. Not tools like nvidia-settings or OpenCL.
pub fn is_driver_package(p: &str) -> bool {
    let p = p.strip_prefix("lib32-").unwrap_or(p);
    if p.starts_with("linux") {
        return p.contains("nvidia");
    }
    matches!(
        p,
        "nvidia"
            | "nvidia-open"
            | "nvidia-dkms"
            | "nvidia-open-dkms"
            | "nvidia-utils"
            | "nvidia-lts"
            | "mesa"
            | "vulkan-radeon"
            | "vulkan-intel"
            | "vulkan-nouveau"
    ) || (p.starts_with("nvidia-") && p.ends_with("-utils"))
}

/// What the driver history says about when this game's crashes started.
#[derive(Debug, Clone)]
pub struct DriverContext<'a> {
    /// This game's first GPU driver crash.
    pub first_crash: NaiveDateTime,
    /// The closest driver-related update before it (a kernel module rebuild counts).
    pub closest: Option<&'a DriverUpdate>,
    /// The newest real driver version change before it (not a rebuild of the same version).
    pub version_change: Option<&'a DriverUpdate>,
    /// Starts of this game after `version_change` and before the first crash that didn't crash,
    /// with how long each ran (seconds).
    pub ran_after: Vec<i64>,
    /// The longest this game lasted before a crash (seconds): runs shorter than this prove little.
    pub crashes_within: i64,
    /// Driver updates that only came after the first crash (they can't have caused it).
    pub after: Vec<&'a DriverUpdate>,
}

pub fn driver_context(ev: &Evidence) -> Option<DriverContext<'_>> {
    let first_crash = ev
        .events
        .iter()
        .filter(|e| e.app_id.as_deref() == Some(ev.app_id.as_str()))
        .map(|e| e.at)
        .min()?;
    let base = |v: &str| {
        v.rsplit_once('-')
            .map(|(a, _)| a.to_string())
            .unwrap_or_else(|| v.to_string())
    };
    let drivers: Vec<&DriverUpdate> = ev
        .driver_updates
        .iter()
        .filter(|u| is_driver_package(&u.package))
        .collect();
    let closest = drivers
        .iter()
        .filter(|u| u.at <= first_crash)
        .max_by_key(|u| u.at)
        .copied();
    let version_change = drivers
        .iter()
        .filter(|u| {
            u.at <= first_crash && !u.package.starts_with("linux") && base(&u.from) != base(&u.to)
        })
        .max_by_key(|u| u.at)
        .copied();
    let all: Vec<&CrashEvent> = ev.events.iter().collect();
    let runs = history::runs(&ev.sessions, &ev.app_id, &all);
    let crashes_within = runs
        .iter()
        .filter(|r| r.outcome == history::Outcome::Crashed)
        .filter_map(|r| r.session.duration())
        .map(|d| d.num_seconds())
        .max()
        .unwrap_or(0);
    let ran_after = version_change
        .map(|u| {
            runs.iter()
                .filter(|r| r.session.start > u.at && r.session.start < first_crash)
                .filter(|r| r.outcome != history::Outcome::Crashed)
                .filter_map(|r| r.session.duration())
                .map(|d| d.num_seconds())
                .collect()
        })
        .unwrap_or_default();
    let after = drivers
        .iter()
        .filter(|u| u.at > first_crash)
        .copied()
        .collect();
    Some(DriverContext {
        first_crash,
        closest,
        version_change,
        ran_after,
        crashes_within,
        after,
    })
}

/// "4 times (4–5 min each)", and whether that's long enough to count.
pub fn ran_after_note(c: &DriverContext) -> Option<String> {
    if c.ran_after.is_empty() {
        return None;
    }
    let n = c.ran_after.len();
    let lo = c.ran_after.iter().min().copied().unwrap_or(0) / 60;
    let hi = c.ran_after.iter().max().copied().unwrap_or(0) / 60;
    let span = if lo == hi {
        format!("{lo} min")
    } else {
        format!("{lo}–{hi} min")
    };
    let long_enough = c
        .ran_after
        .iter()
        .any(|d| *d > c.crashes_within && c.crashes_within > 0);
    Some(format!(
        "It ran {n} time{} after that update without crashing ({span}{}){}",
        if n == 1 { "" } else { "s" },
        if n == 1 { "" } else { " each" },
        if long_enough {
            ", longer than it lasts when it crashes, so that update alone probably isn't the cause."
        } else {
            format!(
                ", but it crashes after up to {} min, so those runs were too short to clear it.",
                c.crashes_within / 60
            )
            .leak()
        }
    ))
}
