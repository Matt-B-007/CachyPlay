// SPDX-License-Identifier: GPL-3.0-or-later
//! What the player said about a start: "What happened?" (ran fine, stutter, graphics glitches,
//! crashed, network lag) and whether it was online. Stored per game, keyed by the start time.

use crate::manage::{read_json, write_json};
use crate::paths::Paths;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const KINDS: &[&str] = &[
    "fine",
    "black_screen",
    "stutter",
    "glitches",
    "crash",
    "network",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunNote {
    /// One of [`KINDS`], or empty when only "online" was answered.
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
}

impl RunNote {
    /// A problem with the game's graphics (network lag doesn't count against a graphics fix).
    pub fn graphics_problem(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "stutter" | "glitches" | "crash" | "black_screen"
        )
    }
    pub fn describe(&self) -> Option<&'static str> {
        Some(match self.kind.as_str() {
            "fine" => "You said it ran fine",
            "black_screen" => "You said the screen stayed black",
            "stutter" => "You said it stuttered",
            "glitches" => "You said the graphics glitched",
            "crash" => "You said it crashed",
            "network" => "You said it was network lag",
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunNotes {
    /// Start time ("2026-11-25T19:20:06") → note.
    #[serde(default)]
    pub notes: BTreeMap<String, RunNote>,
    /// The problem only happens online, so only online starts count as clean runs.
    #[serde(default)]
    pub online_only: bool,
}

impl RunNotes {
    pub fn get(&self, start: chrono::NaiveDateTime) -> Option<&RunNote> {
        self.notes.get(&key(start))
    }
}

pub fn key(start: chrono::NaiveDateTime) -> String {
    start.format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn file(paths: &Paths, app_id: &str) -> PathBuf {
    paths.data.join("run-notes").join(format!("{app_id}.json"))
}

pub fn load(paths: &Paths, app_id: &str) -> RunNotes {
    read_json(&file(paths, app_id)).unwrap_or_default()
}

/// Save what happened in one start. `kind` may be empty to answer only "online?".
pub fn set(
    paths: &Paths,
    app_id: &str,
    start: &str,
    kind: &str,
    online: Option<bool>,
) -> Result<RunNotes> {
    if !kind.is_empty() && !KINDS.contains(&kind) {
        bail!("unknown answer {kind:?}");
    }
    let mut n = load(paths, app_id);
    let e = n.notes.entry(start.to_string()).or_default();
    if !kind.is_empty() {
        e.kind = kind.to_string();
    }
    if online.is_some() {
        e.online = online;
    }
    // "Crashes online, fine offline" → the problem is online-only from now on.
    let problem_online = n
        .notes
        .values()
        .any(|x| x.graphics_problem() && x.online == Some(true));
    let fine_offline = n
        .notes
        .values()
        .any(|x| x.kind == "fine" && x.online == Some(false));
    if problem_online && fine_offline {
        n.online_only = true;
    }
    write_json(&file(paths, app_id), &n)?;
    Ok(n)
}

pub fn set_online_only(paths: &Paths, app_id: &str, on: bool) -> Result<()> {
    let mut n = load(paths, app_id);
    n.online_only = on;
    write_json(&file(paths, app_id), &n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_only_is_learned_from_answers() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::isolated(t.path());
        set(&p, "1240440", "2026-11-25T17:20:10", "fine", Some(false)).unwrap();
        assert!(!load(&p, "1240440").online_only);
        let n = set(&p, "1240440", "2026-11-25T19:20:06", "crash", Some(true)).unwrap();
        assert!(n.online_only);
        assert!(set(&p, "1240440", "x", "exploded", None).is_err());
    }
}
