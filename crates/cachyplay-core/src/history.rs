// SPDX-License-Identifier: GPL-3.0-or-later
//! A game's recent history, from Steam's own session log: when it was started, which compatibility
//! tool it used, how long it ran, and whether the GPU driver crashed during it. This answers the
//! questions a person asks first: "when did it stop working?", "what changed?", "did switching
//! Proton help?", and "do other games still run?".

use crate::crashmatch::{CrashEvent, Session};
use chrono::{Duration, NaiveDateTime};
use serde::Serialize;

/// A session this long without a crash counts as "the game worked".
pub const PLAYED: i64 = 15 * 60;
/// A session shorter than this counts as "closed almost right away".
pub const SHORT: i64 = 3 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Ran 15 minutes or more with no GPU crash.
    Played,
    /// Ran a while, then closed normally.
    Ok,
    /// Closed within 3 minutes.
    Short,
    /// The GPU driver crashed while it ran.
    Crashed,
    /// Still running (or Steam didn't log the end).
    Running,
}

impl Outcome {
    pub fn bad(self) -> bool {
        matches!(self, Outcome::Short | Outcome::Crashed)
    }
}

pub fn outcome(s: &Session, crashes: &[&CrashEvent]) -> Outcome {
    let end = s.end;
    let crashed = crashes.iter().any(|c| {
        c.app_id.as_deref() == Some(s.app_id.as_str())
            && c.at >= s.start - Duration::seconds(5)
            && end.is_none_or(|e| c.at <= e + Duration::seconds(90))
    });
    if crashed {
        return Outcome::Crashed;
    }
    match s.duration() {
        None => Outcome::Running,
        Some(d) if d.num_seconds() >= PLAYED => Outcome::Played,
        Some(d) if d.num_seconds() < SHORT => Outcome::Short,
        Some(_) => Outcome::Ok,
    }
}

/// "2 h 23 min", "4 min", "55 s".
pub fn duration_text(d: Duration) -> String {
    let s = d.num_seconds().max(0);
    if s < 90 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", (s + 30) / 60)
    } else {
        format!("{} h {} min", s / 3600, (s % 3600) / 60)
    }
}

#[derive(Debug, Clone)]
pub struct Run<'a> {
    pub session: &'a Session,
    pub outcome: Outcome,
}

/// One game's Steam sessions (oldest first) with outcomes.
pub fn runs<'a>(sessions: &'a [Session], app_id: &str, crashes: &[&CrashEvent]) -> Vec<Run<'a>> {
    let mut out: Vec<Run> = sessions
        .iter()
        .filter(|s| s.app_id == app_id && s.source == "Steam")
        .map(|s| Run {
            session: s,
            outcome: outcome(s, crashes),
        })
        .collect();
    out.sort_by_key(|r| r.session.start);
    out
}

/// The game used to work and has been failing since.
#[derive(Debug, Clone)]
pub struct Trouble<'a> {
    /// The last time it ran properly (15+ minutes, no crash), if Steam's log goes back that far.
    pub last_good: Option<&'a Run<'a>>,
    /// Every start since then (all failed or short).
    pub since: Vec<&'a Run<'a>>,
}

impl Trouble<'_> {
    pub fn first_bad(&self) -> NaiveDateTime {
        self.since[0].session.start
    }
    pub fn bad_count(&self) -> usize {
        self.since.iter().filter(|r| r.outcome.bad()).count()
    }
    pub fn crash_count(&self) -> usize {
        self.since
            .iter()
            .filter(|r| r.outcome == Outcome::Crashed)
            .count()
    }

    /// Each tool used since the trouble began, with how many of those starts failed, in order of
    /// first use.
    pub fn tools_tried(&self) -> Vec<(String, usize, usize)> {
        let mut out: Vec<(String, usize, usize)> = Vec::new();
        for r in &self.since {
            let Some(t) = &r.session.tool else { continue };
            match out.iter_mut().find(|(n, _, _)| n == t) {
                Some(e) => {
                    e.1 += 1;
                    e.2 += r.outcome.bad() as usize;
                }
                None => out.push((t.clone(), 1, r.outcome.bad() as usize)),
            }
        }
        out
    }
}

/// Find a run of failures after the last good session: at least two starts, every one of them
/// short or crashed, and none that ran properly.
pub fn trouble<'a>(runs: &'a [Run<'a>]) -> Option<Trouble<'a>> {
    let last_good_i = runs.iter().rposition(|r| r.outcome == Outcome::Played);
    let since: Vec<&Run> = runs[last_good_i.map_or(0, |i| i + 1)..]
        .iter()
        .filter(|r| r.outcome != Outcome::Running)
        .collect();
    if since.len() < 2 || !since.iter().all(|r| r.outcome.bad()) {
        return None;
    }
    // A single crash among quick restarts with no GPU errors at all could just be someone
    // checking something; require a crash or several quick exits.
    if since.iter().all(|r| r.outcome == Outcome::Short) && since.len() < 3 {
        return None;
    }
    Some(Trouble {
        last_good: last_good_i.map(|i| &runs[i]),
        since,
    })
}

/// Other games that ran properly (at least `min` long, no crash) since `t`, longest first.
pub fn others_ran_fine<'a>(
    sessions: &'a [Session],
    app_id: &str,
    t: NaiveDateTime,
    min: Duration,
    crashes: &[&CrashEvent],
) -> Vec<&'a Session> {
    let mut out: Vec<&Session> = sessions
        .iter()
        .filter(|s| s.app_id != app_id && s.source == "Steam" && s.start >= t)
        .filter(|s| s.duration().is_some_and(|d| d >= min))
        .filter(|s| outcome(s, crashes) != Outcome::Crashed)
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.duration()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crashmatch::{at, parse_steam_sessions, CrashEvent, GpuFault};

    fn line(t: &str, kind: &str, app: &str, tool: &str) -> String {
        format!("[{t}] Game process {kind} : AppID {app} \"~/.local/share/Steam/steamrt64/reaper SteamLaunch AppId={app} -- '~/.local/share/Steam/steamapps/common/SteamLinuxRuntime_4'/_v2-entry-point --verb=waitforexitandrun -- '{tool}'/proton waitforexitandrun  '/games/x.exe'\", ProcID 1\n")
    }

    fn crash(t: NaiveDateTime, app: &str) -> CrashEvent {
        CrashEvent {
            at: t,
            app_id: Some(app.into()),
            faults: vec![GpuFault {
                at: t,
                vendor: "nvidia",
                xid: Some(109),
                pid: None,
                process: None,
                line: String::new(),
            }],
        }
    }

    /// Moon Sweepers, as in the real report: fine on Nov 15, failing every time since Nov 22 with
    /// three different Proton versions; Cyberpunk ran for hours meanwhile.
    fn log() -> String {
        let exp = "~/.local/share/Steam/steamapps/common/Proton - Experimental";
        let ge = "~/.local/share/Steam/compatibilitytools.d/GE-Proton11-12-x86_64";
        let cachy = "/usr/share/steam/compatibilitytools.d/proton-cachyos-slr";
        let mut s = String::new();
        for (a, b, tool) in [
            ("2026-11-15 08:38:19", "2026-11-15 11:01:54", exp),
            ("2026-11-22 09:42:45", "2026-11-22 09:44:45", exp),
            ("2026-11-22 09:46:08", "2026-11-22 09:47:16", ge),
            ("2026-11-22 09:50:35", "2026-11-22 09:50:48", cachy),
            ("2026-11-23 12:11:17", "2026-11-23 12:12:17", exp),
            ("2026-11-24 13:51:51", "2026-11-24 13:52:40", exp),
        ] {
            s += &line(a, "added", "5335510", tool);
            s += &line(b, "removed", "5335510", tool);
        }
        s += &line("2026-11-24 07:30:33", "added", "1091500", ge);
        s += &line("2026-11-24 12:49:27", "removed", "1091500", ge);
        s
    }

    #[test]
    fn reads_tools_and_finds_when_it_broke() {
        let sessions = parse_steam_sessions(&log());
        assert_eq!(sessions[0].tool.as_deref(), Some("Proton - Experimental"));
        let crashes = [
            crash(at(2026, 11, 23, 12, 11, 26), "5335510"),
            crash(at(2026, 11, 24, 13, 52, 1), "5335510"),
        ];
        let refs: Vec<&CrashEvent> = crashes.iter().collect();
        let r = runs(&sessions, "5335510", &refs);
        assert_eq!(r.len(), 6);
        assert_eq!(r[0].outcome, Outcome::Played);
        assert_eq!(r[4].outcome, Outcome::Crashed);
        let t = trouble(&r).expect("trouble");
        assert_eq!(
            t.last_good.unwrap().session.start,
            at(2026, 11, 15, 8, 38, 19)
        );
        assert_eq!(t.first_bad(), at(2026, 11, 22, 9, 42, 45));
        assert_eq!((t.bad_count(), t.crash_count()), (5, 2));
        let tools: Vec<String> = t.tools_tried().into_iter().map(|x| x.0).collect();
        assert_eq!(
            tools,
            [
                "Proton - Experimental",
                "GE-Proton11-12-x86_64",
                "proton-cachyos-slr"
            ]
        );
        let fine = others_ran_fine(
            &sessions,
            "5335510",
            t.first_bad(),
            Duration::minutes(60),
            &refs,
        );
        assert_eq!(fine.len(), 1);
        assert_eq!(duration_text(fine[0].duration().unwrap()), "5 h 18 min");
    }

    #[test]
    fn no_trouble_when_it_still_runs() {
        let mut l = log();
        l += &line(
            "2026-11-24 15:00:00",
            "added",
            "5335510",
            "x/Proton - Experimental",
        );
        l += &line(
            "2026-11-24 16:00:00",
            "removed",
            "5335510",
            "x/Proton - Experimental",
        );
        let sessions = parse_steam_sessions(&l);
        let r = runs(&sessions, "5335510", &[]);
        assert!(trouble(&r).is_none());
    }
}
