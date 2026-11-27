// SPDX-License-Identifier: GPL-3.0-or-later
//! Fix suggestions, worked out on your PC from the evidence CachyPlay collects. No AI, no network:
//! each suggestion is a fixed rule ("GPU crashes in several games + an always-on Vulkan add-on →
//! try the game without that add-on"), and each one lists the evidence that triggered it.
//!
//! Suggestions that only touch one game (environment variables, game arguments) can be applied with
//! one click and undone any time. Suggestions that would change the whole system (drivers, kernels,
//! GPU clocks) are explained as steps, never done automatically.

mod driver;
mod insights;
mod proton;
mod rules;
mod summary;
#[cfg(test)]
mod tests;

pub use driver::*;
pub use insights::*;
use proton::*;
pub use rules::*;
pub use summary::*;

use crate::crashmatch::{self, CrashEvent};
use crate::evidence::DriverUpdate;
use crate::fixes::{Fix, GameFixes};
use crate::history;
use crate::vklayers::{self, Layer};
use crate::wrapper::LaunchRecord;
use chrono::NaiveDateTime;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Everything the rules look at, for one game.
#[derive(Debug, Clone, Default)]
pub struct Evidence {
    pub app_id: String,
    pub game_name: String,
    /// Steam's last update of the game.
    pub game_updated: Option<NaiveDateTime>,
    /// All GPU crashes (any game), newest first.
    pub events: Vec<CrashEvent>,
    /// AppID → game name, for crashes in other games.
    pub names: BTreeMap<String, String>,
    pub layers: Vec<Layer>,
    pub driver_updates: Vec<DriverUpdate>,
    /// Rule ids that matched in this game's own logs (Proton log, Unity Player.log).
    pub log_findings: Vec<String>,
    pub has_proton_log: bool,
    /// Which DirectX translation layer the Proton log shows: "dxvk" or "vkd3d".
    pub proton_layer: Option<String>,
    pub is_unity: bool,
    pub uses_proton: bool,
    pub launches: Vec<LaunchRecord>,
    pub fixes: GameFixes,
    pub cachyplay_on: bool,
    /// Steam's session log, all games (start, end, compatibility tool).
    pub sessions: Vec<crashmatch::Session>,
    /// The user's own launch options for this game (without CachyPlay's part).
    pub user_options: String,
    /// Compatibility tools installed on this PC.
    pub tools: Vec<crate::compat::Tool>,
    /// The tool this game is forced to in Steam (internal name), if any.
    pub forced_tool: Option<String>,
    /// The tool CachyPlay switched this game to, if it did.
    pub switched_tool: Option<String>,
    /// What the game's Unity log says (graphics mode, frame generation, version).
    pub unity: Option<crate::insights::UnityInfo>,
    pub anti_cheat: Option<crate::insights::AntiCheat>,
    /// Other games that ran long and clean recently, and how they're set up.
    pub working: Vec<crate::insights::WorkingGame>,
    /// When CachyPlay switched the compatibility tool.
    pub switched_at: Option<NaiveDateTime>,
    /// What the player said about each start ("What happened?").
    pub notes: crate::notes::RunNotes,
    /// The time to judge "now" by (tests set it; None means the clock).
    pub now: Option<NaiveDateTime>,
    /// Fixes that worked for other games on this PC: (game name, fix id, why they're similar).
    pub worked_elsewhere: Vec<(String, String, String)>,
    /// Every change tried for this game, including undone ones.
    pub tries: crate::tries::Tries,
    /// Games (any, on this PC) that run once NVIDIA's raw access chains are switched off.
    pub nvidia_switch_helped: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Not applied yet.
    #[default]
    New,
    /// Applied; no launch since.
    Applied,
    /// Applied, and the game has run normally since.
    Worked,
    /// Applied, but the game crashed again since.
    DidntHelp,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Strong, specific evidence points here.
    Likely,
    /// Worth trying; the evidence fits but doesn't prove it.
    #[default]
    Possible,
    /// A guess or a general tip; not enough information to say.
    Unsure,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Suggestion {
    pub id: String,
    pub title: String,
    /// The facts that triggered this suggestion, in plain English.
    pub why: Vec<String>,
    /// One-click, undoable change for this game only.
    pub fix: Option<Fix>,
    /// Steps for things CachyPlay won't change by itself (system-wide).
    pub steps: Vec<String>,
    pub status: Status,
    /// Lower comes first.
    pub priority: u8,
    /// A compatibility tool to switch this game to with one click (needs Steam closed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch_tool: Option<crate::compat::Tool>,
    /// Applied, played since, and the player hasn't said whether it helped: ask "Did it work?".
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ask_verdict: bool,
    /// How sure CachyPlay is: "likely", "possible" or "unsure".
    pub confidence: Confidence,
    /// CachyPlay already switched the tool; offer Undo.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub switched: bool,
    /// What would prove this is the cause.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
    /// A one-click action that isn't a launch setting ("reset_shader_cache").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// A bigger change to the PC (like going back to an older driver): shown last, never automatic.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub big_step: bool,
    /// Ruled out for this game earlier; offer "Try again".
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub try_again: bool,
}

impl Evidence {
    pub fn now(&self) -> NaiveDateTime {
        self.now
            .unwrap_or_else(|| chrono::Local::now().naive_local())
    }

    fn game_events(&self) -> Vec<&CrashEvent> {
        self.events
            .iter()
            .filter(|e| e.app_id.as_deref() == Some(self.app_id.as_str()))
            .collect()
    }

    /// Names of the different programs that hit GPU errors.
    fn crashed_programs(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for e in &self.events {
            if !seen.insert(e.program_key()) {
                continue;
            }
            out.push(match &e.app_id {
                Some(id) => self
                    .names
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| format!("AppID {id}")),
                None => format!(
                    "a program named \"{}\"",
                    e.faults[0].process.clone().unwrap_or_default()
                ),
            });
        }
        out
    }

    pub(crate) fn status_of(&self, id: &str) -> Status {
        let Some(fix) = self.fixes.fixes.iter().find(|f| f.id == id) else {
            // Undone, but tried before and ruled out for this game: stays crossed off.
            return if self.tries.ruled_out(id).is_some() {
                Status::DidntHelp
            } else {
                Status::New
            };
        };
        if self.tries.ruled_out(id).is_some_and(|t| t.off_at.is_none()) {
            return Status::DidntHelp;
        }
        match fix.verdict {
            Some(true) => return Status::Worked,
            Some(false) => return Status::DidntHelp,
            None => {}
        }
        // A launch that's still going says nothing yet.
        let since: Vec<&LaunchRecord> = self
            .launches
            .iter()
            .filter(|l| l.started_at > fix.applied_at && !l.running)
            .collect();
        let applied_at = chrono::DateTime::parse_from_rfc3339(&fix.applied_at)
            .ok()
            .map(|d| d.naive_local());
        let crashed_since = applied_at.is_some_and(|t| self.game_events().iter().any(|e| e.at > t));
        // Starts in Steam's log count too, even when CachyPlay was off for them.
        let steam_runs: Vec<&crashmatch::Session> = self
            .sessions
            .iter()
            .filter(|s| s.app_id == self.app_id && applied_at.is_some_and(|t| s.start > t))
            .filter(|s| {
                s.duration()
                    .is_some_and(|d| d.num_seconds() >= history::SHORT)
            })
            .collect();
        // A short start without a crash proves nothing (the player may have closed it); only a
        // crash, or the player saying it went wrong, counts against the fix.
        let bad_note = self.sessions.iter().any(|s| {
            s.app_id == self.app_id
                && applied_at.is_some_and(|t| s.start > t)
                && self
                    .notes
                    .get(s.start)
                    .is_some_and(|n| n.graphics_problem())
        });
        if crashed_since || bad_note {
            Status::DidntHelp
        } else if !steam_runs.is_empty()
            || since
                .iter()
                .any(|l| l.runtime_secs >= history::SHORT as f64)
        {
            Status::Worked
        } else {
            Status::Applied
        }
    }
}

fn fix(id: &str, label: &str, env: &[(&str, &str)], args: &[&str], temporary: bool) -> Fix {
    Fix {
        id: id.into(),
        label: label.into(),
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        args: args.iter().map(|a| a.to_string()).collect(),
        applied_at: String::new(),
        temporary,
        verdict: None,
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}
