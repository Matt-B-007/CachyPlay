//! Collecting the facts that bug reports and fix suggestions are built on: kernel GPU errors,
//! which game was running when, recent driver upgrades. Shared by the app, reports and the CLI.

use crate::crashmatch::{self, CrashEvent};
use crate::manage::Ctx;
use crate::paths::Paths;
use crate::sys::capture;
use crate::wrapper::LaunchRecord;
use chrono::NaiveDateTime;
use std::fs;
use std::time::Duration;

const TOOL_TIMEOUT: Duration = Duration::from_secs(5);

pub struct KernelLogs {
    pub this_boot: Option<String>,
    pub previous_boot: Option<String>,
}

/// Remove firewall log lines: they hold network addresses and say nothing about games.
pub fn drop_noise(text: &str) -> String {
    text.lines()
        .filter(|l| {
            !(l.contains("[UFW ")
                || (l.contains(" IN=") && l.contains(" SRC=") && l.contains(" DST=")))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Kernel warnings from this boot and the previous one (in case a crash froze the PC and you
/// restarted). `CACHYPLAY_KERNEL_LOG_FILE` replays a saved log instead (tests).
pub fn kernel_logs() -> KernelLogs {
    if let Some(f) = std::env::var_os("CACHYPLAY_KERNEL_LOG_FILE") {
        return KernelLogs {
            this_boot: fs::read_to_string(f).ok().map(|t| drop_noise(&t)),
            previous_boot: None,
        };
    }
    let read = |boot: &str| {
        capture(
            "journalctl",
            &[
                "-k",
                "-b",
                boot,
                "-p",
                "warning",
                "--no-pager",
                "-o",
                "short-iso",
                "-n",
                "3000",
            ],
            TOOL_TIMEOUT,
        )
        .map(|t| drop_noise(&t))
        .filter(|t| !t.trim().is_empty() && !t.contains("-- No entries --"))
    };
    KernelLogs {
        this_boot: read("0"),
        previous_boot: read("-1"),
    }
}

/// GPU crashes, newest first, each matched to the game that was running if possible.
pub fn crash_events(
    paths: &Paths,
    ctx: Option<&Ctx>,
    logs: &KernelLogs,
    now: NaiveDateTime,
) -> Vec<CrashEvent> {
    let mut faults = Vec::new();
    for t in [logs.previous_boot.as_deref(), logs.this_boot.as_deref()]
        .into_iter()
        .flatten()
    {
        faults.extend(crashmatch::parse_kernel_faults(t, now));
    }
    faults.sort_by_key(|f| f.at);
    faults.dedup_by(|a, b| a.line == b.line);
    let mut sessions = Vec::new();
    if let Some(c) = ctx {
        sessions.extend(crashmatch::parse_steam_sessions(&c.steam.console_logs()));
    }
    sessions.extend(crashmatch::sessions_from_launches(&all_launches(paths)));
    crashmatch::crash_events(&faults, &sessions, now)
}

pub fn all_launches(paths: &Paths) -> Vec<LaunchRecord> {
    let mut v: Vec<LaunchRecord> = fs::read_dir(paths.launches())
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| crate::manage::read_json::<LaunchRecord>(&e.path()))
                .map(|r| r.settle(chrono::Local::now()))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    v
}

#[derive(Debug, Clone, PartialEq)]
pub struct DriverUpdate {
    pub at: NaiveDateTime,
    pub package: String,
    pub from: String,
    pub to: String,
}

/// Graphics driver and kernel upgrades from pacman's log, oldest first.
pub fn driver_updates(pacman_log: &str) -> Vec<DriverUpdate> {
    pacman_log
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix('[')?;
            let (stamp, rest) = rest.split_once(']')?;
            let rest = rest
                .trim_start()
                .strip_prefix("[ALPM]")?
                .trim_start()
                .strip_prefix("upgraded ")?;
            let (pkg, vers) = rest.split_once(' ')?;
            if !is_graphics_package(pkg) {
                return None;
            }
            let (from, to) = vers
                .trim()
                .trim_start_matches('(')
                .trim_end_matches(')')
                .split_once(" -> ")?;
            let at = chrono::DateTime::parse_from_str(stamp, "%Y-%m-%dT%H:%M:%S%z")
                .map(|d| d.naive_local())
                .or_else(|_| NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M"))
                .ok()?;
            Some(DriverUpdate {
                at,
                package: pkg.to_string(),
                from: from.to_string(),
                to: to.to_string(),
            })
        })
        .collect()
}

/// Graphics driver and kernel packages: every NVIDIA package (nvidia-utils, lib32-nvidia-utils,
/// nvidia-settings, nvidia-open-dkms, …), Mesa and its Vulkan drivers, and CachyOS kernels with
/// their NVIDIA modules.
pub fn is_graphics_package(pkg: &str) -> bool {
    let p = pkg.strip_prefix("lib32-").unwrap_or(pkg);
    p.starts_with("nvidia")
        || p.contains("-nvidia")
        || p == "mesa"
        || p.starts_with("vulkan-")
        || p == "linux-cachyos"
        || p == "linux-cachyos-lts"
        || p.starts_with("linux-cachyos-") && p.ends_with("-nvidia-open")
}

pub fn read_driver_updates() -> Vec<DriverUpdate> {
    fs::read_to_string("/var/log/pacman.log")
        .map(|t| driver_updates(&t))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pacman_upgrades() {
        let log = "[2026-11-19T10:01:02-0800] [ALPM] upgraded nvidia-utils (620.60.01-1 -> 620.73.04-1)\n\
                   [2026-11-19T10:01:03-0800] [ALPM] upgraded firefox (1 -> 2)\n\
                   [2026-11-23T07:00:00-0800] [ALPM] upgraded linux-cachyos (7.3.5-1 -> 7.3.6-1)\n\
                   [2026-11-23T07:00:01-0800] [ALPM] installed mesa (1)\n";
        let u = driver_updates(log);
        assert_eq!(u.len(), 2);
        for p in [
            "nvidia-utils",
            "lib32-nvidia-utils",
            "nvidia-settings",
            "nvidia-open-dkms",
            "linux-cachyos-nvidia-open",
            "linux-cachyos-lts-nvidia-open",
            "vulkan-radeon",
            "lib32-mesa",
        ] {
            assert!(is_graphics_package(p), "{p}");
        }
        for p in ["firefox", "linux-cachyos-headers", "mesa-utils"] {
            assert!(!is_graphics_package(p), "{p}");
        }
        assert_eq!(u[0].package, "nvidia-utils");
        assert_eq!(u[0].from, "620.60.01-1");
        assert_eq!(u[0].to, "620.73.04-1");
    }
}
