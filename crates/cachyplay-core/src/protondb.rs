// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional ProtonDB rating for a game: how well it runs for other Linux players. Off unless the
//! user turns it on in Settings; only the game's AppID is sent, and the answer is kept for a week.

use crate::manage::{read_json, write_json};
use crate::paths::Paths;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rating {
    /// "platinum", "gold", "silver", "bronze", "borked".
    pub tier: String,
    /// Recent reports' tier, if it differs.
    pub trending: Option<String>,
    pub reports: u64,
    /// When this was fetched (RFC 3339).
    pub fetched_at: String,
}

impl Rating {
    /// What the tier means, in plain words.
    pub fn meaning(&self) -> &'static str {
        match self.tier.as_str() {
            "platinum" => "Runs perfectly out of the box for most players.",
            "gold" => "Runs perfectly after small tweaks for most players.",
            "silver" => "Runs with minor problems for most players.",
            "bronze" => "Runs, but with problems that get in the way.",
            "borked" => "Doesn't run for most players.",
            _ => "Not enough reports yet.",
        }
    }
}

const CACHE_DAYS: i64 = 7;

pub fn parse(app_json: &str) -> Option<(String, Option<String>, u64)> {
    let v: serde_json::Value = serde_json::from_str(app_json).ok()?;
    let tier = v.get("tier")?.as_str()?.to_string();
    let trending = v
        .get("trendingTier")
        .and_then(|t| t.as_str())
        .filter(|t| *t != tier)
        .map(str::to_string);
    let reports = v.get("total").and_then(|t| t.as_u64()).unwrap_or(0);
    Some((tier, trending, reports))
}

fn cache_path(paths: &Paths, app_id: &str) -> std::path::PathBuf {
    paths.state.join("protondb").join(format!("{app_id}.json"))
}

/// The game's rating: from the cache if it's less than a week old, otherwise from ProtonDB.
/// `None` when it can't be found (no reports, offline, or blocked).
pub fn rating(paths: &Paths, app_id: &str) -> Option<Rating> {
    if !app_id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let cached: Option<Rating> = read_json(&cache_path(paths, app_id));
    if let Some(r) = &cached {
        let fresh = crate::clock::instant(&r.fetched_at).is_some_and(|t| {
            crate::clock::now_instant().duration_since(t) < crate::clock::days(CACHE_DAYS)
        });
        if fresh {
            return cached;
        }
    }
    let body = if let Some(p) = std::env::var_os("CACHYPLAY_PROTONDB_FILE") {
        std::fs::read_to_string(p).ok()?
    } else {
        let url = format!("https://www.protondb.com/api/v1/reports/summaries/{app_id}.json");
        let (ok, body, _) = crate::sys::run(
            "curl",
            &["-fsSL", "--max-time", "8", "-A", "CachyPlay", &url],
            Duration::from_secs(10),
        )?;
        if !ok {
            return cached;
        }
        body
    };
    let (tier, trending, reports) = parse(&body)?;
    let r = Rating {
        tier,
        trending,
        reports,
        fetched_at: crate::clock::now_rfc3339(),
    };
    let _ = write_json(&cache_path(paths, app_id), &r);
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_summary() {
        let j = r#"{"bestReportedTier":"platinum","confidence":"strong","score":0.71,"tier":"gold","total":312,"trendingTier":"silver"}"#;
        assert_eq!(parse(j), Some(("gold".into(), Some("silver".into()), 312)));
        let same = r#"{"tier":"gold","total":3,"trendingTier":"gold"}"#;
        assert_eq!(parse(same), Some(("gold".into(), None, 3)));
        assert_eq!(parse("not json"), None);
    }
}
