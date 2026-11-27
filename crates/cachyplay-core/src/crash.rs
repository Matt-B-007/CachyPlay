// SPDX-License-Identifier: GPL-3.0-or-later
//! "CachyPlay closed unexpectedly last time": a running marker is written when the desktop app
//! starts and removed when it closes normally. If the marker is still there at the next start,
//! the app didn't close normally (a crash, the system ending it, a power cut). A panic hook adds
//! the details when the cause was a crash inside CachyPlay. Traces from the last 10 crashes are
//! kept in ~/.local/state/cachyplay/crashes.

use crate::fsutil::{atomic_write, read_capped};
use crate::paths::Paths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const TRACES_KEPT: usize = 10;
/// The process name the marker belongs to (as /proc/<pid>/comm shows it).
pub const APP_COMM: &str = "cachyplay-gui";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Marker {
    pid: u32,
    started: String,
    version: String,
}

/// What the notice shows.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LastCrash {
    /// When the session that didn't close normally started.
    pub started: String,
    pub version: String,
    /// The crash trace, if CachyPlay itself crashed (not the system ending it).
    pub details: Option<String>,
    pub file: Option<String>,
}

fn marker(paths: &Paths) -> PathBuf {
    paths.state.join("running.json")
}

pub fn crash_dir(paths: &Paths) -> PathBuf {
    paths.state.join("crashes")
}

/// Whether a CachyPlay window with this process id is still running (a zombie that has ended but
/// not been cleaned up yet doesn't count).
fn alive(proc_root: &Path, pid: u32) -> bool {
    let dir = proc_root.join(pid.to_string());
    let named = fs::read_to_string(dir.join("comm")).is_ok_and(|c| c.trim() == APP_COMM);
    let ended = fs::read_to_string(dir.join("stat")).is_ok_and(|st| {
        st.rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .is_some_and(|state| state == "Z" || state == "X")
    });
    named && !ended
}

/// Call first thing when the app starts. Returns the previous session if it didn't close
/// normally. Writes nothing: another copy may still be running (a second launch, or a copy that's
/// just closing), so the marker is only claimed later with `claim`.
pub fn check(paths: &Paths, proc_root: &Path) -> Option<LastCrash> {
    let own = std::process::id();
    let file = marker(paths);
    let m: Marker = crate::manage::read_json(&file)?;
    if m.pid == own || alive(proc_root, m.pid) {
        return None;
    }
    let since = fs::metadata(&file).and_then(|x| x.modified()).ok();
    let trace = newest_trace(paths).filter(|(_, t)| since.is_none_or(|s| *t >= s));
    crate::logging::write(paths, "The previous session didn't close normally.");
    Some(LastCrash {
        started: m.started,
        version: m.version,
        details: trace.as_ref().and_then(|(p, _)| read_capped(p, 80, 40)),
        file: trace.map(|(p, _)| p.display().to_string()),
    })
}

/// Mark this session as running, unless another copy that's still alive holds the marker.
pub fn claim(paths: &Paths, proc_root: &Path) {
    let own = std::process::id();
    let file = marker(paths);
    if let Some(m) = crate::manage::read_json::<Marker>(&file) {
        if m.pid != own && alive(proc_root, m.pid) {
            return;
        }
    }
    let mine = Marker {
        pid: own,
        started: crate::manage::now(),
        version: crate::VERSION.into(),
    };
    if let Ok(text) = serde_json::to_string(&mine) {
        let _ = atomic_write(&file, text.as_bytes());
    }
}

/// `check` then `claim`, for callers that don't need them apart.
pub fn start(paths: &Paths, proc_root: &Path) -> Option<LastCrash> {
    let previous = check(paths, proc_root);
    claim(paths, proc_root);
    previous
}

/// Call when the app closes normally.
pub fn clean_exit(paths: &Paths) {
    let file = marker(paths);
    let m: Option<Marker> = crate::manage::read_json(&file);
    if m.is_some_and(|m| m.pid == std::process::id()) {
        let _ = fs::remove_file(file);
    }
}

fn newest_trace(paths: &Paths) -> Option<(PathBuf, std::time::SystemTime)> {
    fs::read_dir(crash_dir(paths))
        .ok()?
        .flatten()
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
        .max_by_key(|(_, t)| *t)
}

/// Write a crash trace (keeping the newest `TRACES_KEPT`). Returns the file.
pub fn write_trace(paths: &Paths, text: &str) -> Option<PathBuf> {
    let dir = crash_dir(paths);
    fs::create_dir_all(&dir).ok()?;
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S%.3f");
    let file = dir.join(format!("crash-{stamp}.txt"));
    atomic_write(&file, text.as_bytes()).ok()?;
    let mut all: Vec<PathBuf> = fs::read_dir(&dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    all.retain(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("crash-"))
    });
    all.sort();
    while all.len() > TRACES_KEPT {
        let _ = fs::remove_file(all.remove(0));
    }
    Some(file)
}

/// Record panics (with a backtrace) as crash traces, then carry on with the normal panic output.
pub fn install_panic_hook(paths: &Paths, program: &'static str) {
    let paths = paths.clone();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let trace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current();
        let text = format!(
            "{program} {} crashed at {}\nthread: {}\n{info}\n\nbacktrace:\n{trace}\n",
            crate::VERSION,
            crate::manage::now(),
            thread.name().unwrap_or("unnamed"),
        );
        let _ = write_trace(&paths, &text);
        crate::logging::write(&paths, &format!("crash: {info}"));
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_a_session_that_did_not_close_normally() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        let proc_root = d.path().join("proc");
        fs::create_dir_all(&proc_root).unwrap();
        // First start: nothing to report. Normal close removes the marker.
        assert_eq!(start(&p, &proc_root), None);
        clean_exit(&p);
        assert_eq!(start(&p, &proc_root), None);
        // This session "crashes": a stale marker from another (dead) process plus a trace.
        let stale = Marker {
            pid: 999_999,
            started: "2026-11-26T20:00:00-08:00".into(),
            version: "0.1.0-beta.14".into(),
        };
        fs::write(marker(&p), serde_json::to_string(&stale).unwrap()).unwrap();
        write_trace(&p, "cachyplay-gui crashed\nboom").unwrap();
        let c = start(&p, &proc_root).expect("noticed");
        assert_eq!(c.version, "0.1.0-beta.14");
        assert!(c.details.unwrap().contains("boom"));
        // Shown once: the next start is clean again.
        clean_exit(&p);
        assert_eq!(start(&p, &proc_root), None);
    }

    #[test]
    fn a_second_launch_leaves_the_running_copy_alone() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        let proc_root = d.path().join("proc");
        fs::create_dir_all(proc_root.join("4242")).unwrap();
        fs::write(proc_root.join("4242/comm"), "cachyplay-gui\n").unwrap();
        fs::write(proc_root.join("4242/stat"), "4242 (cachyplay-gui) S 1 4242").unwrap();
        let other = Marker {
            pid: 4242,
            started: "x".into(),
            version: "y".into(),
        };
        fs::create_dir_all(&p.state).unwrap();
        fs::write(marker(&p), serde_json::to_string(&other).unwrap()).unwrap();
        assert_eq!(start(&p, &proc_root), None, "not a crash");
        clean_exit(&p);
        assert!(marker(&p).is_file(), "the running copy's marker is kept");
        // A copy that has ended but not been cleaned up (a zombie) isn't running.
        fs::write(proc_root.join("4242/stat"), "4242 (cachyplay-gui) Z 1 4242").unwrap();
        assert!(!alive(&proc_root, 4242));
        fs::write(proc_root.join("4242/stat"), "4242 (cachyplay-gui) S 1 4242").unwrap();
        // That copy then closes without its Exit handler running (it was still shutting down
        // when this one started): this copy claims the marker, and it's not reported as a crash.
        fs::remove_dir_all(proc_root.join("4242")).unwrap();
        claim(&p, &proc_root);
        let m: Marker = crate::manage::read_json(&marker(&p)).unwrap();
        assert_eq!(m.pid, std::process::id());
    }

    #[test]
    fn keeps_the_last_ten_traces() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        for i in 0..14 {
            write_trace(&p, &format!("trace {i}")).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(fs::read_dir(crash_dir(&p)).unwrap().count(), TRACES_KEPT);
    }
}
