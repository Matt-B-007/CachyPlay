//! Where a game stands, step by step, and the timeline behind it.
//!
//! Status steps after a change (a fix, your own settings, or a Proton switch):
//!   Testing the fix → Seems to work (1 clean run) → Works with a fix (3 clean runs, or 2 adding up
//!   to an hour). Any graphics crash after the change → Not working again.
//! A clean run lasts twice as long as the longest recent crash took (at least 10 minutes, at most
//! 60; 20 when there's no crash to compare with), and ends without a graphics driver crash. When
//! the problem only happens online, only online starts count.
//! Crashes from before the change stay in the history but no longer make the game red.
//!
//! Every start in Steam's log counts, whether or not CachyPlay was on for it.

use crate::advice::{Evidence, Light};
use crate::crashmatch::Session;
use crate::history::{self, Outcome};
use crate::notes::RunNote;
use chrono::{Duration, NaiveDateTime};
use serde::Serialize;

pub const MIN_CLEAN: i64 = 10 * 60;
pub const MAX_CLEAN: i64 = 60 * 60;
pub const DEFAULT_CLEAN: i64 = 20 * 60;
/// Runs needed for "Works with a fix" (or 2 adding up to an hour).
pub const RUNS_FOR_WORKS: usize = 3;
/// Crashes older than this don't set the clean-run length.
const LOOKBACK_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    NotWorking,
    Testing,
    SeemsToWork,
    WorksWithFix,
    RunsFine,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub stage: Stage,
    pub light: Light,
    /// "Works with a fix: Start in Direct3D 12"
    pub headline: String,
    /// "3 clean runs since Nov 25 (2 h 10 min)."
    pub detail: Option<String>,
    /// The change being tested, if any.
    pub fix_label: Option<String>,
    #[serde(skip)]
    pub changed_at: Option<NaiveDateTime>,
    pub needed_secs: i64,
    /// "18 min without a crash (it used to crash within 3–9 min)"
    pub needed_text: String,
    pub clean_runs: usize,
    pub clean_secs: i64,
    pub playing_secs: Option<i64>,
    pub online_only: bool,
    /// Start time of the newest ended start that needs "Was that online?" answered.
    pub ask_online: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunRow {
    /// Start time as "2026-11-25T19:20:06" (the key for notes).
    pub start: String,
    pub secs: Option<i64>,
    pub running: bool,
    /// "crash", "fine", "short", "didnt_start", "running", "ok", "problem"
    pub outcome: &'static str,
    /// "Graphics crash after 4 min", "Ran fine · 45 min", "Didn't start (closed after 0 s)"
    pub text: String,
    pub tool: Option<String>,
    /// Fixes and settings CachyPlay applied to this start (empty if CachyPlay was off).
    pub applied: Vec<String>,
    pub through_cachyplay: bool,
    pub gpu: Option<String>,
    pub gpu_integrated: bool,
    /// "✓ clean run 1 of 3", "too short to count", "offline, doesn't count for this problem"
    pub counts: Option<String>,
    pub note: Option<RunNote>,
    /// Plain-words crash message for the details.
    pub crash_detail: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Item {
    Run(RunRow),
    Event {
        at: String,
        text: String,
        what: &'static str,
    },
}

struct Start<'a> {
    session: &'a Session,
    outcome: Outcome,
    /// Seconds from start to the first graphics crash in it.
    crash_after: Option<i64>,
    note: Option<&'a RunNote>,
}

impl Start<'_> {
    fn secs(&self) -> Option<i64> {
        self.session.duration().map(|d| d.num_seconds().max(0))
    }
    fn bad(&self) -> bool {
        self.outcome == Outcome::Crashed || self.note.is_some_and(|n| n.graphics_problem())
    }
}

fn starts<'a>(ev: &'a Evidence) -> Vec<Start<'a>> {
    let crashes: Vec<_> = ev.events.iter().collect();
    history::runs(&ev.sessions, &ev.app_id, &crashes)
        .into_iter()
        .map(|r| {
            let s = r.session;
            let crash_after = ev
                .events
                .iter()
                .filter(|c| c.app_id.as_deref() == Some(ev.app_id.as_str()))
                .filter(|c| {
                    c.at >= s.start - Duration::seconds(5)
                        && s.end.is_none_or(|e| c.at <= e + Duration::seconds(90))
                })
                .map(|c| (c.at - s.start).num_seconds().max(0))
                .min();
            Start {
                session: s,
                outcome: r.outcome,
                crash_after,
                note: ev.notes.get(s.start),
            }
        })
        .collect()
}

fn mins(secs: i64) -> String {
    history::duration_text(Duration::seconds(secs))
}

/// The clean-run length for this game, and the longest recent crash time it's based on.
pub fn clean_length(ev: &Evidence, now: NaiveDateTime) -> (i64, Option<(i64, i64)>) {
    let times: Vec<i64> = starts(ev)
        .iter()
        .filter(|s| s.session.start > now - Duration::days(LOOKBACK_DAYS))
        .filter_map(|s| {
            s.crash_after
                .or_else(|| s.bad().then(|| s.secs()).flatten())
        })
        .collect();
    match (times.iter().min(), times.iter().max()) {
        (Some(&lo), Some(&hi)) => ((2 * hi).clamp(MIN_CLEAN, MAX_CLEAN), Some((lo, hi))),
        _ => (DEFAULT_CLEAN, None),
    }
}

/// The newest change being tested: (label, when).
fn change(ev: &Evidence) -> Option<(String, NaiveDateTime)> {
    let parse = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.naive_local())
    };
    let mut best: Option<(String, NaiveDateTime)> = ev
        .fixes
        .fixes
        .iter()
        .filter(|f| !f.temporary)
        .filter_map(|f| Some((f.label.clone(), parse(&f.applied_at)?)))
        .max_by_key(|x| x.1);
    if let (Some(tool), Some(at)) = (&ev.switched_tool, ev.switched_at) {
        if best.as_ref().is_none_or(|b| at > b.1) {
            best = Some((format!("Switch to {tool}"), at));
        }
    }
    best
}

pub fn progress(ev: &Evidence, now: NaiveDateTime) -> Progress {
    let all = starts(ev);
    let (needed, range) = clean_length(ev, now);
    let needed_text = match range {
        Some((lo, hi)) if lo == hi => format!(
            "{} without a crash (it used to crash after {})",
            mins(needed),
            mins(hi)
        ),
        Some((lo, hi)) => format!(
            "{} without a crash (it used to crash within {}–{})",
            mins(needed),
            mins(lo),
            mins(hi)
        ),
        None => format!("{} without a crash", mins(needed)),
    };
    let online_only = ev.notes.online_only;
    let counts_mode = |s: &Start| !online_only || s.note.and_then(|n| n.online) == Some(true);
    let changed = change(ev);
    let after: Vec<&Start> = all
        .iter()
        .filter(|s| changed.as_ref().is_none_or(|c| s.session.start > c.1))
        .collect();
    let playing = all
        .iter()
        .rev()
        .find(|s| s.session.end.is_none() && now - s.session.start < Duration::hours(12))
        .map(|s| (now - s.session.start).num_seconds().max(0));
    let ended_after: Vec<&&Start> = after.iter().filter(|s| s.session.end.is_some()).collect();
    let clean: Vec<&&&Start> = ended_after
        .iter()
        .filter(|s| !s.bad() && counts_mode(s) && s.secs().unwrap_or(0) >= needed)
        .collect();
    let clean_secs: i64 = ended_after
        .iter()
        .filter(|s| !s.bad() && counts_mode(s) && s.secs().unwrap_or(0) >= history::SHORT)
        .filter_map(|s| s.secs())
        .sum();
    let ask_online = online_only
        .then(|| {
            ended_after
                .iter()
                .rev()
                .find(|s| !s.bad() && s.note.and_then(|n| n.online).is_none())
        })
        .flatten()
        .map(|s| crate::notes::key(s.session.start));
    let day = |t: NaiveDateTime| t.format("%b %-d").to_string();
    let mut p = Progress {
        stage: Stage::Unknown,
        light: Light::Grey,
        headline: String::new(),
        detail: None,
        fix_label: changed.as_ref().map(|c| c.0.clone()),
        changed_at: changed.as_ref().map(|c| c.1),
        needed_secs: needed,
        needed_text,
        clean_runs: clean.len(),
        clean_secs,
        playing_secs: playing,
        online_only,
        ask_online,
    };
    let set = |p: &mut Progress, stage, light, headline: String, detail: Option<String>| {
        p.stage = stage;
        p.light = light;
        p.headline = headline;
        p.detail = detail;
    };

    if let Some((label, at)) = &changed {
        let verdict = ev
            .fixes
            .fixes
            .iter()
            .find(|f| &f.label == label)
            .and_then(|f| f.verdict);
        let bad_after = after.iter().filter(|s| s.bad()).count();
        if verdict == Some(false) {
            set(
                &mut p,
                Stage::NotWorking,
                Light::Red,
                format!("You said \"{label}\" didn't help"),
                Some("Undo it and try the next suggestion.".into()),
            );
        } else if bad_after > 0 {
            set(
                &mut p,
                Stage::NotWorking,
                Light::Red,
                format!("Crashed again after \"{label}\""),
                Some(format!(
                    "{} since {}. It may not be the cause; try the next suggestion.",
                    history_count(bad_after, "bad start"),
                    day(*at)
                )),
            );
        } else if verdict == Some(true) {
            set(
                &mut p,
                Stage::WorksWithFix,
                Light::Yellow,
                format!("Works with a fix: {label}"),
                Some("You confirmed it works.".into()),
            );
        } else if let Some(secs) = playing.filter(|_| after.iter().any(|s| s.session.end.is_none()))
        {
            set(
                &mut p,
                Stage::Testing,
                Light::Yellow,
                format!("Testing the fix: {label}"),
                Some(format!("Playing now, {} of {}.", mins(secs), mins(needed))),
            );
        } else if clean.len() >= RUNS_FOR_WORKS || (clean.len() >= 2 && clean_secs >= 3600) {
            set(
                &mut p,
                Stage::WorksWithFix,
                Light::Yellow,
                format!("Works with a fix: {label}"),
                Some(format!(
                    "{} since {} ({} in all).",
                    history_count(clean.len(), "clean run"),
                    day(*at),
                    mins(clean_secs)
                )),
            );
        } else if !clean.is_empty() || clean_secs >= needed {
            let left = RUNS_FOR_WORKS - clean.len().min(RUNS_FOR_WORKS);
            set(
                &mut p,
                Stage::SeemsToWork,
                Light::Yellow,
                format!("Seems to work: {label}"),
                Some(format!(
                    "{} without a crash since {}. {} more to be sure.",
                    mins(clean_secs),
                    day(*at),
                    history_count(left.max(1), "clean run")
                )),
            );
        } else if !ended_after.is_empty() {
            set(
                &mut p,
                Stage::Testing,
                Light::Yellow,
                format!("Testing the fix: {label}"),
                Some(format!(
                    "No crash so far, but too short to count yet ({} of {}).",
                    mins(clean_secs),
                    mins(needed)
                )),
            );
        } else {
            let msg = format!(
                "Applied {}. Start the game to try it: a clean run is {}.",
                day(*at),
                p.needed_text
            );
            set(
                &mut p,
                Stage::Testing,
                Light::Yellow,
                format!("Testing the fix: {label}"),
                Some(msg),
            );
        }
        return p;
    }

    // No change being tested: judge by the latest starts.
    let last_bad = all.iter().rev().find(|s| s.bad()).map(|s| s.session.start);
    let clean_since_bad = all
        .iter()
        .filter(|s| last_bad.is_none_or(|b| s.session.start > b))
        .filter(|s| {
            s.session.end.is_some()
                && !s.bad()
                && counts_mode(s)
                && s.secs().unwrap_or(0) >= needed.min(history::PLAYED)
        })
        .count();
    if let Some(secs) = playing {
        set(
            &mut p,
            Stage::Testing,
            if last_bad.is_some() && clean_since_bad == 0 {
                Light::Red
            } else {
                Light::Yellow
            },
            "Playing now".into(),
            Some(format!("{} so far.", mins(secs))),
        );
        if last_bad.is_some() {
            p.detail = Some(format!("{} so far; {}.", mins(secs), p.needed_text));
        }
    } else if let Some(b) = last_bad.filter(|b| *b > now - Duration::days(LOOKBACK_DAYS)) {
        if clean_since_bad > 0 {
            set(
                &mut p,
                Stage::RunsFine,
                Light::Green,
                "Runs fine again".into(),
                Some(format!(
                    "{} since the last problem on {}.",
                    history_count(clean_since_bad, "clean run"),
                    day(b)
                )),
            );
        } else {
            set(
                &mut p,
                Stage::NotWorking,
                Light::Red,
                "Not working".into(),
                None,
            );
        }
    } else if all
        .iter()
        .any(|s| s.session.end.is_some() && s.secs().unwrap_or(0) >= history::SHORT)
    {
        set(
            &mut p,
            Stage::RunsFine,
            Light::Green,
            "Runs fine".into(),
            None,
        );
    } else if !all.is_empty() {
        set(
            &mut p,
            Stage::Unknown,
            Light::Grey,
            "Not enough play to tell".into(),
            None,
        );
    } else {
        set(
            &mut p,
            Stage::Unknown,
            Light::Grey,
            "Not played since CachyPlay started watching".into(),
            None,
        );
    }
    p
}

fn history_count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

/// The game's starts, newest first, with dividers for fixes, Proton switches, driver and game
/// updates at the moment they happened.
pub fn timeline(ev: &Evidence, now: NaiveDateTime, limit: usize) -> Vec<Item> {
    let p = progress(ev, now);
    let all = starts(ev);
    let changed_at = p.changed_at;
    let mut clean_n = 0;
    let mut rows: Vec<(NaiveDateTime, Item)> = Vec::new();
    for s in &all {
        let st = s.session.start;
        let secs = s.secs();
        let launch = ev.launches.iter().find(|l| {
            chrono::DateTime::parse_from_rfc3339(&l.started_at)
                .map(|d| (d.naive_local() - st).num_seconds().abs() <= 10)
                .unwrap_or(false)
        });
        let (outcome, text) = match (s.outcome, secs, s.crash_after) {
            (Outcome::Crashed, _, Some(c)) => {
                ("crash", format!("Graphics crash after {}", mins(c)))
            }
            (Outcome::Crashed, _, None) => ("crash", "Graphics crash".into()),
            (Outcome::Running, _, _)
                if s.session.end.is_none() && now - st < Duration::hours(12) =>
            {
                (
                    "running",
                    format!("Playing now · {}", mins((now - st).num_seconds())),
                )
            }
            (_, Some(x), _) if x < 15 => (
                "didnt_start",
                format!("Didn't start (closed after {})", mins(x)),
            ),
            (_, Some(x), _) if s.note.is_some_and(|n| n.graphics_problem()) => {
                ("problem", format!("Problem · {}", mins(x)))
            }
            (_, Some(x), _) if x < history::SHORT => ("short", format!("Closed after {}", mins(x))),
            (_, Some(x), _) => ("fine", format!("Ran fine · {}", mins(x))),
            _ => ("ok", "No end time in Steam's log".into()),
        };
        let after_change = changed_at.is_some_and(|c| st > c);
        let counts = if !after_change || s.session.end.is_none() || s.bad() {
            None
        } else if p.online_only && s.note.and_then(|n| n.online) == Some(false) {
            Some("offline, doesn't count for this problem".to_string())
        } else if secs.unwrap_or(0) >= p.needed_secs
            && (!p.online_only || s.note.and_then(|n| n.online) == Some(true))
        {
            clean_n += 1;
            Some(format!("✓ clean run {clean_n} of {RUNS_FOR_WORKS}"))
        } else if p.online_only && s.note.and_then(|n| n.online).is_none() {
            Some("was it online? answer to count it".into())
        } else {
            Some(format!(
                "too short to count (needs {})",
                mins(p.needed_secs)
            ))
        };
        let crash_detail = s.crash_after.map(|_| {
            "The graphics driver stopped the game's graphics work because it stopped responding (NVIDIA Xid / GPU hang).".to_string()
        });
        rows.push((
            st,
            Item::Run(RunRow {
                start: crate::notes::key(st),
                secs,
                running: outcome == "running",
                outcome,
                text,
                tool: s.session.tool.clone(),
                applied: launch.map(|l| l.applied.clone()).unwrap_or_default(),
                through_cachyplay: launch.is_some(),
                gpu: launch.and_then(|l| l.gpu.clone()),
                gpu_integrated: launch.is_some_and(|l| l.gpu_integrated == Some(true)),
                counts,
                note: s.note.cloned(),
                crash_detail,
                exit_code: launch.and_then(|l| l.exit_code),
            }),
        ));
    }
    let first = all.first().map(|s| s.session.start);
    let mut events: Vec<(NaiveDateTime, &'static str, String)> = Vec::new();
    let parse = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.naive_local())
    };
    for f in &ev.fixes.fixes {
        if let Some(t) = parse(&f.applied_at) {
            events.push((t, "fix", format!("Turned on: {}", f.label)));
        }
    }
    if let (Some(tool), Some(at)) = (&ev.switched_tool, ev.switched_at) {
        events.push((at, "proton", format!("Switched to {tool}")));
    }
    for u in &ev.driver_updates {
        events.push((
            u.at,
            "driver",
            format!("{} updated {} → {}", u.package, u.from, u.to),
        ));
    }
    if let Some(g) = ev.game_updated {
        events.push((g, "game", format!("{} updated by Steam", ev.game_name)));
    }
    for (t, what, text) in events {
        if first.is_some_and(|f| t >= f) {
            rows.push((
                t,
                Item::Event {
                    at: crate::notes::key(t),
                    text,
                    what,
                },
            ));
        }
    }
    rows.sort_by_key(|b| std::cmp::Reverse(b.0));
    // Keep `limit` starts (and the events between them).
    let mut out = Vec::new();
    let mut n = 0;
    for (_, item) in rows {
        if matches!(item, Item::Run(_)) {
            if n == limit {
                break;
            }
            n += 1;
        }
        out.push(item);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crashmatch::{CrashEvent, GpuFault};
    use crate::fixes::{Fix, GameFixes};

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }
    fn session(start: &str, mins: i64) -> Session {
        Session {
            app_id: "1".into(),
            start: t(start),
            end: Some(t(start) + Duration::minutes(mins)),
            source: "Steam",
            tool: Some("proton-cachyos".into()),
            settings: vec![],
        }
    }
    fn crash(at: &str) -> CrashEvent {
        CrashEvent {
            at: t(at),
            app_id: Some("1".into()),
            faults: vec![GpuFault {
                at: t(at),
                xid: Some(109),
                ..Default::default()
            }],
        }
    }
    fn halo() -> Evidence {
        // Crashes 4, 3 and 9 minutes in: a clean run needs 18 minutes.
        Evidence {
            app_id: "1".into(),
            game_name: "Halo".into(),
            sessions: vec![
                session("2026-11-25 16:00:00", 5),
                session("2026-11-25 17:00:00", 4),
                session("2026-11-25 18:00:00", 10),
            ],
            events: vec![
                crash("2026-11-25 16:04:00"),
                crash("2026-11-25 17:03:00"),
                crash("2026-11-25 18:09:00"),
            ],
            ..Default::default()
        }
    }
    fn with_fix(mut ev: Evidence, at: &str) -> Evidence {
        ev.fixes = GameFixes {
            app_id: "1".into(),
            fixes: vec![Fix {
                id: "d3d12".into(),
                label: "Start in Direct3D 12".into(),
                applied_at: format!("{}-08:00", t(at).format("%Y-%m-%dT%H:%M:%S")),
                ..Default::default()
            }],
        };
        ev
    }
    const NOW: &str = "2026-11-26 12:00:00";

    #[test]
    fn clean_run_is_twice_the_longest_crash() {
        let ev = halo();
        let p = progress(&ev, t(NOW));
        assert_eq!(p.needed_secs, 18 * 60);
        assert!(
            p.needed_text.contains("within 3 min–9 min"),
            "{}",
            p.needed_text
        );
        assert_eq!(p.stage, Stage::NotWorking);
        assert_eq!(p.light, Light::Red);
        // Limits: never under 10 minutes or over an hour.
        let mut quick = halo();
        quick.events = vec![crash("2026-11-25 16:00:20")];
        assert_eq!(progress(&quick, t(NOW)).needed_secs, MIN_CLEAN);
        let none = Evidence {
            app_id: "1".into(),
            ..Default::default()
        };
        assert_eq!(progress(&none, t(NOW)).needed_secs, DEFAULT_CLEAN);
    }

    #[test]
    fn steps_from_testing_to_works_with_a_fix() {
        let mut ev = with_fix(halo(), "2026-11-25 19:00:00");
        // Old crashes don't count any more: testing, not red.
        let p = progress(&ev, t(NOW));
        assert_eq!((p.stage, p.light), (Stage::Testing, Light::Yellow));
        assert!(p.headline.contains("Start in Direct3D 12"));
        // Playing now.
        ev.sessions.push(Session {
            end: None,
            ..session("2026-11-26 11:48:00", 0)
        });
        let p = progress(&ev, t(NOW));
        assert_eq!(p.stage, Stage::Testing);
        assert!(
            p.detail
                .as_deref()
                .unwrap()
                .starts_with("Playing now, 12 min of 18 min"),
            "{:?}",
            p.detail
        );
        ev.sessions.pop();
        // Too short to count.
        ev.sessions.push(session("2026-11-25 19:10:00", 12));
        assert_eq!(progress(&ev, t(NOW)).stage, Stage::Testing);
        // One clean run: seems to work.
        ev.sessions.push(session("2026-11-25 20:00:00", 25));
        let p = progress(&ev, t(NOW));
        assert_eq!(p.stage, Stage::SeemsToWork, "{p:?}");
        // Two more clean runs: works with a fix.
        ev.sessions.push(session("2026-11-25 21:00:00", 20));
        ev.sessions.push(session("2026-11-25 22:00:00", 20));
        let p = progress(&ev, t(NOW));
        assert_eq!(p.stage, Stage::WorksWithFix);
        assert_eq!(p.headline, "Works with a fix: Start in Direct3D 12");
        // A new crash after the fix: not working again.
        ev.sessions.push(session("2026-11-26 10:00:00", 8));
        ev.events.push(crash("2026-11-26 10:07:00"));
        assert_eq!(progress(&ev, t(NOW)).stage, Stage::NotWorking);
    }

    #[test]
    fn two_runs_adding_up_to_an_hour_are_enough() {
        let mut ev = with_fix(halo(), "2026-11-25 19:00:00");
        ev.sessions.push(session("2026-11-25 20:00:00", 40));
        ev.sessions.push(session("2026-11-25 21:00:00", 30));
        assert_eq!(progress(&ev, t(NOW)).stage, Stage::WorksWithFix);
    }

    #[test]
    fn online_only_problems_count_only_online_runs() {
        let mut ev = with_fix(halo(), "2026-11-25 19:00:00");
        ev.notes.online_only = true;
        ev.sessions.push(session("2026-11-25 20:00:00", 30));
        let p = progress(&ev, t(NOW));
        assert_eq!(p.stage, Stage::Testing);
        assert_eq!(p.ask_online.as_deref(), Some("2026-11-25T20:00:00"));
        ev.notes.notes.insert(
            "2026-11-25T20:00:00".into(),
            RunNote {
                kind: "fine".into(),
                online: Some(true),
            },
        );
        assert_eq!(progress(&ev, t(NOW)).stage, Stage::SeemsToWork);
    }

    #[test]
    fn your_answer_wins() {
        let mut ev = with_fix(halo(), "2026-11-25 19:00:00");
        ev.fixes.fixes[0].verdict = Some(false);
        assert_eq!(progress(&ev, t(NOW)).stage, Stage::NotWorking);
        ev.fixes.fixes[0].verdict = Some(true);
        assert_eq!(progress(&ev, t(NOW)).stage, Stage::WorksWithFix);
        // Saying "it stuttered" counts as a problem even without a driver crash.
        ev.fixes.fixes[0].verdict = None;
        ev.sessions.push(session("2026-11-25 20:00:00", 30));
        ev.notes.notes.insert(
            "2026-11-25T20:00:00".into(),
            RunNote {
                kind: "stutter".into(),
                online: None,
            },
        );
        assert_eq!(progress(&ev, t(NOW)).stage, Stage::NotWorking);
    }

    #[test]
    fn crashes_followed_by_clean_play_are_not_red() {
        let mut ev = halo();
        ev.sessions.push(session("2026-11-25 20:00:00", 40));
        let p = progress(&ev, t(NOW));
        assert_eq!((p.stage, p.light), (Stage::RunsFine, Light::Green));
    }

    #[test]
    fn timeline_reads_like_a_story() {
        let mut ev = with_fix(halo(), "2026-11-25 19:00:00");
        ev.sessions.push(session("2026-11-25 20:00:00", 25));
        ev.sessions.push(Session {
            end: Some(t("2026-11-25 21:00:00")),
            ..session("2026-11-25 21:00:00", 0)
        });
        let items = timeline(&ev, t(NOW), 10);
        let texts: Vec<String> = items
            .iter()
            .map(|i| match i {
                Item::Run(r) => format!("{} | {}", r.text, r.counts.clone().unwrap_or_default()),
                Item::Event { text, .. } => format!("-- {text}"),
            })
            .collect();
        assert_eq!(
            texts[0],
            "Didn't start (closed after 0 s) | too short to count (needs 18 min)"
        );
        assert_eq!(texts[1], "Ran fine · 25 min | ✓ clean run 1 of 3");
        assert_eq!(texts[2], "-- Turned on: Start in Direct3D 12");
        assert_eq!(texts[3], "Graphics crash after 9 min | ");
        assert_eq!(
            timeline(&ev, t(NOW), 2)
                .iter()
                .filter(|i| matches!(i, Item::Run(_)))
                .count(),
            2
        );
    }
}
