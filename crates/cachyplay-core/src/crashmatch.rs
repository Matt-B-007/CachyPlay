//! Working out which game was running when the GPU driver crashed.
//!
//! Kernel GPU-fault lines (NVIDIA "Xid", AMD ring timeouts, GPU hangs) carry a time, a process ID
//! and a short thread name, but not the game. Steam's own log records when each game started and
//! stopped ("Game process added/removed : AppID …"), and CachyPlay records its own launches.
//! Matching the fault time against those sessions names the game, even when CachyPlay was not
//! turned on for it. When several different programs hit GPU faults, that points to the driver or
//! the system rather than one broken game.

use crate::wrapper::LaunchRecord;
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime};
use regex::Regex;
use std::collections::BTreeSet;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub struct GpuFault {
    pub at: NaiveDateTime,
    pub vendor: &'static str,
    /// NVIDIA Xid code, if any.
    pub xid: Option<u32>,
    pub pid: Option<u32>,
    /// Thread or process name from the kernel line (may be truncated, e.g. "Unity Main Thre").
    pub process: Option<String>,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub app_id: String,
    pub start: NaiveDateTime,
    pub end: Option<NaiveDateTime>,
    pub source: &'static str,
    /// The compatibility tool Steam started it with (folder name, e.g. "Proton - Experimental",
    /// "GE-Proton11-12-x86_64"), or None for a native game / unknown.
    pub tool: Option<String>,
}

impl Session {
    /// How long it ran, if it has ended.
    pub fn duration(&self) -> Option<chrono::Duration> {
        self.end.map(|e| e - self.start)
    }
}

/// The compatibility tool in a Steam command line: the folder that holds the `proton` script.
/// `…/compatibilitytools.d/GE-Proton11-12-x86_64'/proton waitforexitandrun …` → GE-Proton11-12-x86_64.
pub fn tool_in_command(line: &str) -> Option<String> {
    let i = line.find("/proton waitforexitandrun")?;
    let before = line[..i].trim_end_matches(['\'', '"']);
    let name = before.rsplit('/').next()?.trim_matches(['\'', '"']);
    (!name.is_empty()).then(|| name.to_string())
}

#[derive(Debug, Clone)]
pub struct CrashEvent {
    pub at: NaiveDateTime,
    pub app_id: Option<String>,
    pub faults: Vec<GpuFault>,
}

impl CrashEvent {
    /// The program that crashed: Steam AppID if matched, otherwise the kernel's process name.
    pub fn program_key(&self) -> String {
        match &self.app_id {
            Some(id) => format!("app:{id}"),
            None => format!(
                "proc:{}",
                self.faults[0].process.clone().unwrap_or_default()
            ),
        }
    }
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("valid regex"))
}

/// Parse a journal line's timestamp: `2026-11-23T18:34:54-08:00 …` (short-iso)
/// or `Nov 23 18:34:54 …` (short; the year is taken from `now`).
fn line_time(line: &str, now: NaiveDateTime) -> Option<NaiveDateTime> {
    let first = line.split_whitespace().next()?;
    if first.len() >= 19 && first.as_bytes().get(4) == Some(&b'-') {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(first) {
            return Some(dt.naive_local());
        }
        if let Ok(dt) = chrono::DateTime::parse_from_str(first, "%Y-%m-%dT%H:%M:%S%z") {
            return Some(dt.naive_local());
        }
    }
    let mut it = line.split_whitespace();
    let (mon, day, time) = (it.next()?, it.next()?, it.next()?);
    let s = format!("{} {mon} {day} {time}", now.year());
    let t = NaiveDateTime::parse_from_str(&s, "%Y %b %d %H:%M:%S").ok()?;
    // A December log read in January belongs to last year.
    Some(if t > now + chrono::Duration::days(1) {
        t.with_year(t.year() - 1).unwrap_or(t)
    } else {
        t
    })
}

/// GPU driver faults in kernel log text.
pub fn parse_kernel_faults(text: &str, now: NaiveDateTime) -> Vec<GpuFault> {
    static XID: OnceLock<Regex> = OnceLock::new();
    static PID: OnceLock<Regex> = OnceLock::new();
    static NAME: OnceLock<Regex> = OnceLock::new();
    static AMD: OnceLock<Regex> = OnceLock::new();
    let xid = re(&XID, r"NVRM: Xid \([^)]*\): (\d+)");
    let pid = re(&PID, r"pid[=:\s]+(\d+)");
    let name = re(&NAME, r"name=([^,]+),");
    let amd = re(
        &AMD,
        r"(?i)amdgpu.*(ring \S+ timeout|GPU reset|page fault|GPU HANG)|GPU HANG",
    );
    let mut out = Vec::new();
    for line in text.lines() {
        let (vendor, code) = if let Some(c) = xid.captures(line) {
            ("nvidia", c[1].parse().ok())
        } else if amd.is_match(line) {
            ("amd", None)
        } else {
            continue;
        };
        let Some(at) = line_time(line, now) else {
            continue;
        };
        let process = name
            .captures(line)
            .map(|c| c[1].trim().to_string())
            .or_else(|| {
                line.split("process ")
                    .nth(1)
                    .map(|r| r.split_whitespace().next().unwrap_or("").to_string())
            });
        out.push(GpuFault {
            at,
            vendor,
            xid: code,
            pid: pid.captures(line).and_then(|c| c[1].parse().ok()),
            process,
            line: line.trim().to_string(),
        });
    }
    out.sort_by_key(|f| f.at);
    out
}

/// Game sessions from Steam's console log (`logs/console_log.txt`).
pub fn parse_steam_sessions(text: &str) -> Vec<Session> {
    static LINE: OnceLock<Regex> = OnceLock::new();
    let line_re = re(
        &LINE,
        r"^\[(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})\]\s+Game process (added|removed|updated)\s*:\s*AppID (\d+)",
    );
    let mut open: Vec<Session> = Vec::new();
    let mut done: Vec<Session> = Vec::new();
    for line in text.lines() {
        let Some(c) = line_re.captures(line.trim()) else {
            continue;
        };
        let Ok(at) = NaiveDateTime::parse_from_str(&c[1], "%Y-%m-%d %H:%M:%S") else {
            continue;
        };
        let app = c[3].to_string();
        match &c[2] {
            "added" => {
                if !open.iter().any(|s| s.app_id == app) {
                    open.push(Session {
                        app_id: app,
                        start: at,
                        end: None,
                        source: "Steam",
                        tool: tool_in_command(line),
                    });
                }
            }
            "removed" => {
                if let Some(i) = open.iter().position(|s| s.app_id == app) {
                    let mut s = open.remove(i);
                    s.end = Some(at);
                    done.push(s);
                }
            }
            _ => {}
        }
    }
    done.extend(open);
    done.sort_by_key(|s| s.start);
    done
}

/// Sessions from CachyPlay's own launch records.
pub fn sessions_from_launches(records: &[LaunchRecord]) -> Vec<Session> {
    records
        .iter()
        .filter_map(|r| {
            let start = chrono::DateTime::parse_from_rfc3339(&r.started_at)
                .ok()?
                .naive_local();
            let end = (!r.running)
                .then(|| start + chrono::Duration::milliseconds((r.runtime_secs * 1000.0) as i64));
            Some(Session {
                app_id: r.app_id.clone(),
                start,
                end,
                source: "CachyPlay",
                tool: None,
            })
        })
        .collect()
}

/// Steam AppIDs that are tools, not games (Steam Linux Runtime, Proton builds).
fn is_tool_app(app_id: &str) -> bool {
    matches!(
        app_id,
        "1070560"
            | "1391110"
            | "1628350"
            | "2180100"
            | "1493710"
            | "228980"
            | "2805730"
            | "3658110"
    )
}

/// The session running at `at` (a small grace period covers the crash itself).
pub fn session_at(sessions: &[Session], at: NaiveDateTime, now: NaiveDateTime) -> Option<&Session> {
    let grace = chrono::Duration::seconds(90);
    sessions
        .iter()
        .filter(|s| !is_tool_app(&s.app_id))
        .filter(|s| {
            s.start - chrono::Duration::seconds(5) <= at && at <= s.end.unwrap_or(now) + grace
        })
        .max_by_key(|s| s.start) // the most recently started game, if several overlap
}

/// Group faults into crash events (same process within a few minutes) and name each game.
pub fn crash_events(
    faults: &[GpuFault],
    sessions: &[Session],
    now: NaiveDateTime,
) -> Vec<CrashEvent> {
    let mut events: Vec<CrashEvent> = Vec::new();
    for f in faults {
        let same = events.last_mut().filter(|e| {
            let last = e.faults.last().expect("non-empty");
            let close = f.at - last.at <= chrono::Duration::minutes(3);
            close && (f.pid.is_none() || last.pid.is_none() || f.pid == last.pid)
        });
        match same {
            Some(e) => e.faults.push(f.clone()),
            None => events.push(CrashEvent {
                at: f.at,
                app_id: session_at(sessions, f.at, now).map(|s| s.app_id.clone()),
                faults: vec![f.clone()],
            }),
        }
    }
    events.reverse(); // newest first
    events
}

/// True when GPU faults came from two or more different programs: a driver or system problem.
pub fn several_programs(events: &[CrashEvent]) -> bool {
    events
        .iter()
        .map(CrashEvent::program_key)
        .collect::<BTreeSet<_>>()
        .len()
        >= 2
}

/// Plain-English meaning of common NVIDIA Xid codes.
pub fn xid_meaning(xid: u32) -> &'static str {
    match xid {
        13 => "a graphics engine error, usually caused by the game or driver",
        31 => "the game's graphics work read GPU memory that wasn't there",
        43 => "the GPU stopped the game's graphics work after an error",
        45 => "the driver ended the game's GPU work (often after another error)",
        48 | 63 | 64 | 94 | 95 => "a GPU memory error (can mean a hardware problem)",
        62 | 119 | 120 => "the GPU's internal firmware stopped responding",
        69 => "a graphics engine exception",
        79 => "the GPU dropped off the PCIe bus (often power, overclock or hardware)",
        109 => "the game's graphics work hung and the GPU couldn't switch away from it",
        _ => "a GPU driver error",
    }
}

pub fn now_local() -> NaiveDateTime {
    Local::now().naive_local()
}

/// For tests: a fixed local time.
pub fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, mo, d)
        .and_then(|x| x.and_hms_opt(h, mi, s))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = include_str!("../tests/fixtures/kernel-warnings-short.log");

    /// Steam console lines as Steam writes them (format to be confirmed on a real system).
    const CONSOLE: &str = "\
[2026-11-23 12:09:58] Game process added : AppID 5335510 \"/home/x/.local/share/Steam/ubuntu12_32/reaper SteamLaunch AppId=5335510\", ProcID 4801, IP 0.0.0.0:0
[2026-11-23 12:12:40] Game process removed: AppID 5335510 \"…\", ProcID 4801
[2026-11-23 18:20:01] Game process added : AppID 1070560 \"steam-runtime\", ProcID 224000
[2026-11-23 18:25:10] Game process added : AppID 5335510 \"…\", ProcID 224050
[2026-11-23 18:27:30] Game process removed: AppID 5335510 \"…\", ProcID 224050
[2026-11-23 18:33:59] Game process added : AppID 5335510 \"…\", ProcID 235300
[2026-11-23 18:36:00] Game process removed: AppID 5335510 \"…\", ProcID 235300
[2026-11-23 18:37:15] Game process added : AppID 1240440 \"…\", ProcID 236800
";

    #[test]
    fn real_log_faults_are_parsed_and_grouped() {
        let now = at(2026, 11, 23, 18, 41, 0);
        let faults = parse_kernel_faults(LOG, now);
        assert_eq!(faults.len(), 7, "{faults:#?}");
        assert_eq!(faults[0].xid, Some(109));
        assert_eq!(faults[0].pid, Some(4874));
        assert_eq!(faults[0].process.as_deref(), Some("Unity Main Thre"));
        assert_eq!(faults[6].process.as_deref(), Some("00 MAIN"));

        let sessions = parse_steam_sessions(CONSOLE);
        let events = crash_events(&faults, &sessions, now);
        // 4 crashes: 12:11 (Moon Sweepers), 18:26, 18:34 and 18:40 (Halo), newest first.
        assert_eq!(events.len(), 4);
        assert_eq!(
            events[0].app_id.as_deref(),
            Some("1240440"),
            "Halo was running at 18:40"
        );
        assert_eq!(events[0].faults.len(), 1);
        assert_eq!(events[1].app_id.as_deref(), Some("5335510"));
        assert_eq!(events[1].faults.len(), 2, "Xid 109 then Xid 31 = one crash");
        assert_eq!(events[3].app_id.as_deref(), Some("5335510"));
        assert!(several_programs(&events));
    }

    #[test]
    fn without_steam_log_process_names_still_separate_programs() {
        let now = at(2026, 11, 23, 18, 41, 0);
        let events = crash_events(&parse_kernel_faults(LOG, now), &[], now);
        assert!(events.iter().all(|e| e.app_id.is_none()));
        assert!(several_programs(&events), "Unity Main Thre vs 00 MAIN");
    }

    #[test]
    fn single_game_is_not_flagged_as_system_wide() {
        let now = at(2026, 11, 23, 18, 30, 0);
        let only_moon: String = LOG
            .lines()
            .filter(|l| !l.contains("00 MAIN"))
            .collect::<Vec<_>>()
            .join("\n");
        let events = crash_events(
            &parse_kernel_faults(&only_moon, now),
            &parse_steam_sessions(CONSOLE),
            now,
        );
        assert!(!several_programs(&events));
    }

    #[test]
    fn short_iso_timestamps() {
        let now = at(2026, 11, 23, 19, 0, 0);
        let f = parse_kernel_faults("2026-11-23T18:40:28-08:00 host kernel: NVRM: Xid (PCI:0000:02:00): 109, pid=1, name=00 MAIN, channel 0x10", now);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].xid, Some(109));
    }

    #[test]
    fn amd_faults() {
        let now = at(2026, 11, 23, 19, 0, 0);
        let f = parse_kernel_faults("Nov 23 18:00:01 h kernel: amdgpu 0000:03:00.0: amdgpu: ring gfx_0.0.0 timeout, signaled seq=1, emitted seq=2", now);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].vendor, "amd");
    }
}
