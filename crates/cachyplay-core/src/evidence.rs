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

/// Kernel messages that appear on many PCs at every boot and have nothing to do with games.
const HARMLESS: &[&str] = &[
    "CIFS: No dialect specified on mount",
    "uses wireless extensions which will stop working for Wi-Fi 7",
    "virt/tdx: TDX not supported",
    "loading out-of-tree module taints kernel",
    "UVC non compliance",
];

/// Remove firewall log lines (they hold network addresses) and harmless boot messages.
pub fn drop_noise(text: &str) -> String {
    text.lines()
        .filter(|l| {
            !(l.contains("[UFW ")
                || (l.contains(" IN=") && l.contains(" SRC=") && l.contains(" DST="))
                || HARMLESS.iter().any(|h| l.contains(h))
                || l.trim_end().ends_with("kernel:"))
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
    let faults = remember_faults(paths, faults, now);
    let mut sessions = Vec::new();
    if let Some(c) = ctx {
        sessions.extend(crashmatch::parse_steam_sessions(&c.steam.console_logs()));
    }
    sessions.extend(crashmatch::sessions_from_launches(&all_launches(paths)));
    crashmatch::crash_events(&faults, &sessions, now)
}

/// The game's detailed Proton log: CachyPlay's own folder (it sets PROTON_LOG_DIR there) or the
/// home folder (Proton's default), whichever was written last.
pub fn proton_log(
    paths: &Paths,
    app_id: &str,
) -> Option<(std::path::PathBuf, chrono::NaiveDateTime)> {
    let name = format!("steam-{app_id}.log");
    [
        paths.state.join("proton-logs").join(&name),
        paths.home.join(&name),
    ]
    .into_iter()
    .filter_map(|p| {
        let t = fs::metadata(&p).and_then(|m| m.modified()).ok()?;
        let t = chrono::DateTime::<chrono::Local>::from(t).naive_local();
        Some((p, t))
    })
    .max_by_key(|x| x.1)
}

/// How long CachyPlay remembers graphics driver crashes it has seen.
pub const CRASH_HISTORY_DAYS: i64 = 30;

fn crash_history(paths: &Paths) -> std::path::PathBuf {
    paths.state.join("gpu-crash-lines.log")
}

/// Merge the crashes in today's kernel logs with the ones CachyPlay saw before, and save the
/// result. The kernel log only goes back two boots, so without this, two restarts erase the
/// evidence. Only the matching kernel lines are kept, for 30 days.
fn remember_faults(
    paths: &Paths,
    mut faults: Vec<crashmatch::GpuFault>,
    now: NaiveDateTime,
) -> Vec<crashmatch::GpuFault> {
    let file = crash_history(paths);
    let saved = fs::read_to_string(&file).unwrap_or_default();
    faults.extend(crashmatch::parse_kernel_faults(&saved, now));
    faults.retain(|f| now - f.at <= chrono::Duration::days(CRASH_HISTORY_DAYS));
    faults.sort_by_key(|f| f.at);
    faults.dedup_by(|a, b| a.line == b.line);
    let text: String = faults.iter().map(|f| format!("{}\n", f.line)).collect();
    if text != saved && std::env::var_os("CACHYPLAY_KERNEL_LOG_FILE").is_none() {
        let _ = fs::create_dir_all(&paths.state);
        let _ = crate::fsutil::atomic_write(&file, text.as_bytes());
    }
    faults
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
            let rest = rest.trim_start().strip_prefix("[ALPM]")?.trim_start();
            // Going back to an older driver shows up as "downgraded".
            let rest = rest
                .strip_prefix("upgraded ")
                .or_else(|| rest.strip_prefix("downgraded "))?;
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
    #[test]
    fn crashes_survive_restarts() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::isolated(tmp.path());
        let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 25)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();
        let line = "2026-11-24T13:52:01-08:00 host kernel: NVRM: Xid (PCI:0000:02:00): 109, pid=395739, name=Unity Main Thre, channel 0x1e, errorString CTX SWITCH TIMEOUT";
        let old = "2026-10-01T10:00:00-08:00 host kernel: NVRM: Xid (PCI:0000:02:00): 31, pid=1, name=x, channel 0x1";
        // Seen once in the kernel log...
        let first = remember_faults(&p, crashmatch::parse_kernel_faults(line, now), now);
        assert_eq!(first.len(), 1);
        // ...two restarts later the kernel log no longer has it, but CachyPlay does.
        let later = remember_faults(&p, vec![], now);
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].xid, Some(109));
        // Seeing it again doesn't count it twice; crashes older than 30 days are forgotten.
        let again = remember_faults(
            &p,
            crashmatch::parse_kernel_faults(&format!("{line}\n{old}"), now),
            now,
        );
        assert_eq!(again.len(), 1);
    }

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
