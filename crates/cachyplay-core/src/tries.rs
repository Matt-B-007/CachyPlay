// SPDX-License-Identifier: GPL-3.0-or-later
//! A permanent record of every change tried for a game: when it went on, when it came off, and why.
//! Undoing a fix used to erase it, so a fix that made things worse came back as "new". Now a fix
//! that crashed, black-screened or was undone because it made things worse stays ruled out for that
//! game (and only that game) until the player chooses "Try again".

use crate::manage::{read_json, write_json};
use crate::paths::Paths;
use anyhow::Result;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Try {
    /// Fix id ("unity_d3d12"), "your_settings", "shader_cache" or "proton:<tool>".
    pub id: String,
    pub label: String,
    /// RFC 3339.
    pub on_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_at: Option<String>,
    /// "worse", "didnt_help", "testing" (the player's answer) or "auto" (CachyPlay saw it fail).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_reason: Option<String>,
    /// Ruled out for this game, in plain words ("Black screen: the game doesn't include Direct3D 12").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ruled_out: Option<String>,
}

impl Try {
    fn time(s: &str) -> Option<NaiveDateTime> {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.naive_local())
    }
    pub fn on(&self) -> Option<NaiveDateTime> {
        Self::time(&self.on_at)
    }
    pub fn off(&self) -> Option<NaiveDateTime> {
        self.off_at.as_deref().and_then(Self::time)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tries {
    #[serde(default)]
    pub tries: Vec<Try>,
}

impl Tries {
    /// Why this change is ruled out for this game, if it is (newest try decides).
    pub fn ruled_out(&self, id: &str) -> Option<&Try> {
        self.tries
            .iter()
            .rev()
            .find(|t| t.id == id)
            .filter(|t| t.ruled_out.is_some())
    }
    pub fn all_ruled_out(&self) -> Vec<&Try> {
        let mut seen: Vec<&str> = Vec::new();
        let mut out = Vec::new();
        for t in self.tries.iter().rev() {
            if seen.contains(&t.id.as_str()) {
                continue;
            }
            seen.push(&t.id);
            if t.ruled_out.is_some() {
                out.push(t);
            }
        }
        out
    }
}

fn file(paths: &Paths, app_id: &str) -> PathBuf {
    paths.data.join("tries").join(format!("{app_id}.json"))
}

pub fn load(paths: &Paths, app_id: &str) -> Tries {
    read_json(&file(paths, app_id)).unwrap_or_default()
}

fn save(paths: &Paths, app_id: &str, t: &Tries) -> Result<()> {
    write_json(&file(paths, app_id), t)
}

/// A change went on. Trying something again starts a new, not-ruled-out try.
pub fn record_on(paths: &Paths, app_id: &str, id: &str, label: &str) -> Result<()> {
    let mut t = load(paths, app_id);
    if t.tries.iter().any(|x| x.id == id && x.off_at.is_none()) {
        return Ok(());
    }
    t.tries.push(Try {
        id: id.into(),
        label: label.into(),
        on_at: chrono::Local::now().to_rfc3339(),
        off_at: None,
        off_reason: None,
        ruled_out: None,
    });
    save(paths, app_id, &t)
}

/// A change came off. "worse" and "didnt_help" rule it out for this game.
pub fn record_off(paths: &Paths, app_id: &str, id: &str, reason: Option<&str>) -> Result<()> {
    let mut t = load(paths, app_id);
    if let Some(x) = t
        .tries
        .iter_mut()
        .rev()
        .find(|x| x.id == id && x.off_at.is_none())
    {
        x.off_at = Some(chrono::Local::now().to_rfc3339());
        x.off_reason = reason.map(str::to_string);
        if x.ruled_out.is_none() {
            x.ruled_out = match reason {
                Some("worse") => Some("You undid it because it made things worse.".into()),
                Some("didnt_help") => Some("You undid it because it didn't help.".into()),
                _ => None,
            };
        }
        save(paths, app_id, &t)?;
    }
    Ok(())
}

/// CachyPlay saw a change fail (a crash with it on, or the game's log showing it can't work).
pub fn rule_out(paths: &Paths, app_id: &str, id: &str, label: &str, why: &str) -> Result<()> {
    let mut t = load(paths, app_id);
    match t.tries.iter_mut().rev().find(|x| x.id == id) {
        Some(x) if x.ruled_out.as_deref() == Some(why) => return Ok(()),
        Some(x) => x.ruled_out = Some(why.into()),
        None => t.tries.push(Try {
            id: id.into(),
            label: label.into(),
            on_at: chrono::Local::now().to_rfc3339(),
            off_at: None,
            off_reason: Some("auto".into()),
            ruled_out: Some(why.into()),
        }),
    }
    save(paths, app_id, &t)
}

/// "Try again": clear the ruled-out mark so the suggestion is offered once more.
pub fn try_again(paths: &Paths, app_id: &str, id: &str) -> Result<()> {
    let mut t = load(paths, app_id);
    for x in t.tries.iter_mut().filter(|x| x.id == id) {
        x.ruled_out = None;
    }
    save(paths, app_id, &t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undone_fixes_stay_ruled_out_until_tried_again() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::isolated(tmp.path());
        record_on(&p, "5472080", "unity_d3d12", "Start in Direct3D 12").unwrap();
        record_on(&p, "5472080", "unity_d3d12", "Start in Direct3D 12").unwrap();
        assert_eq!(load(&p, "5472080").tries.len(), 1, "no duplicate while on");
        record_off(&p, "5472080", "unity_d3d12", Some("worse")).unwrap();
        let t = load(&p, "5472080");
        assert!(t
            .ruled_out("unity_d3d12")
            .unwrap()
            .ruled_out
            .as_deref()
            .unwrap()
            .contains("made things worse"));
        assert!(t.tries[0].off().is_some());
        // "Just testing" doesn't rule anything out.
        record_on(&p, "5472080", "shader_cache", "Fresh shader cache").unwrap();
        record_off(&p, "5472080", "shader_cache", Some("testing")).unwrap();
        assert!(load(&p, "5472080").ruled_out("shader_cache").is_none());
        // Other games aren't affected.
        assert!(load(&p, "1240440").ruled_out("unity_d3d12").is_none());
        try_again(&p, "5472080", "unity_d3d12").unwrap();
        assert!(load(&p, "5472080").ruled_out("unity_d3d12").is_none());
        rule_out(
            &p,
            "5472080",
            "unity_d3d12",
            "Start in Direct3D 12",
            "Black screen",
        )
        .unwrap();
        assert_eq!(load(&p, "5472080").all_ruled_out().len(), 1);
    }
}
