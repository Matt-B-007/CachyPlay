//! Fixes you applied to a game from a suggestion (one click, undo any time).
//!
//! They live in `~/.local/share/cachyplay/fixes/<appid>.json` and are applied by the launch
//! wrapper at game start, alongside the game's fix file. Nothing in Steam's files changes, so
//! applying or undoing a fix works even while Steam is open (as long as CachyPlay is on for the
//! game). Only allowed environment variables and plain game arguments are possible.

use crate::manage::{now, read_json, write_json};
use crate::paths::Paths;
use crate::recipes::env_allowed;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Fix {
    /// The suggestion this came from, e.g. "disable_vulkan_layers".
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub applied_at: String,
    /// Diagnostic-only fixes (like PROTON_LOG=1) that should be removed after use.
    #[serde(default)]
    pub temporary: bool,
    /// What the player said when asked "Did it work?" (true = yes). Beats CachyPlay's own guess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameFixes {
    pub app_id: String,
    #[serde(default)]
    pub fixes: Vec<Fix>,
}

fn path(paths: &Paths, app_id: &str) -> PathBuf {
    paths.data.join("fixes").join(format!("{app_id}.json"))
}

pub fn load(paths: &Paths, app_id: &str) -> GameFixes {
    read_json(&path(paths, app_id)).unwrap_or_else(|| GameFixes {
        app_id: app_id.into(),
        fixes: vec![],
    })
}

/// Environment variables a fix may set: the recipe allow-list, plus the loader switch for add-ons.
pub fn fix_env_allowed(k: &str) -> bool {
    env_allowed(k) || k == "VK_LOADER_LAYERS_DISABLE"
}

/// The id of the "Try these settings" entry.
pub const YOUR_SETTINGS: &str = "your_settings";

/// Variables a player may not set through "Try these settings": they change how programs load or
/// where they look for things, not how a game draws.
const NEVER: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "PATH",
    "HOME",
    "USER",
    "SHELL",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "PYTHONPATH",
    "PERL5LIB",
    "STEAM_COMPAT_DATA_PATH",
    "STEAM_COMPAT_CLIENT_INSTALL_PATH",
    "WINEPREFIX",
];

pub fn your_setting_allowed(k: &str) -> bool {
    !NEVER.contains(&k)
        && !k.is_empty()
        && k.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Parse "VKD3D_CONFIG=no_upload_hvv -force-d3d12" into settings and game options.
pub fn parse_settings(text: &str) -> Result<(BTreeMap<String, String>, Vec<String>)> {
    let mut env = BTreeMap::new();
    let mut args = Vec::new();
    for tok in text.split_whitespace() {
        if tok == "%command%" {
            bail!("Leave out %command%: CachyPlay adds these for you.");
        }
        if tok.contains(['`', ';', '$', '|', '&', '<', '>', '\'', '"']) {
            bail!("{tok:?} has characters that aren't allowed here.");
        }
        match tok.split_once('=') {
            Some((k, v)) if !k.is_empty() && !k.starts_with('-') => {
                let ok_name = k
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                    && !k.starts_with(|c: char| c.is_ascii_digit());
                if !ok_name {
                    bail!(
                        "{k:?} isn't a setting name (settings look like NAME=value, in capitals)."
                    );
                }
                if NEVER.contains(&k) {
                    bail!("{k} can't be set here.");
                }
                env.insert(k.to_string(), v.to_string());
            }
            _ => args.push(tok.to_string()),
        }
    }
    Ok((env, args))
}

/// "Try these settings": your own settings and game options for one game, applied by CachyPlay at
/// launch (Steam's launch options stay as they are). Empty text removes them.
pub fn set_your_settings(paths: &Paths, app_id: &str, text: &str) -> Result<Option<Fix>> {
    let text = text.trim();
    if text.is_empty() {
        remove(paths, app_id, YOUR_SETTINGS)?;
        return Ok(None);
    }
    let (env, args) = parse_settings(text)?;
    let fix = Fix {
        id: YOUR_SETTINGS.into(),
        label: format!("Your settings: {text}"),
        env,
        args,
        applied_at: now(),
        temporary: false,
        verdict: None,
    };
    let mut all = load(paths, app_id);
    if all
        .fixes
        .iter()
        .any(|f| f.id == YOUR_SETTINGS && f.label != fix.label)
    {
        let _ = crate::tries::record_off(paths, app_id, YOUR_SETTINGS, Some("testing"));
    }
    let _ = crate::tries::record_on(paths, app_id, YOUR_SETTINGS, &fix.label);
    all.fixes.retain(|f| f.id != YOUR_SETTINGS);
    all.fixes.push(fix.clone());
    write_json(&path(paths, app_id), &all)?;
    Ok(Some(fix))
}

pub fn apply(paths: &Paths, app_id: &str, mut fix: Fix) -> Result<()> {
    if let Some(k) = fix.env.keys().find(|k| !fix_env_allowed(k)) {
        bail!("{k} can't be set by a fix");
    }
    if fix
        .args
        .iter()
        .any(|a| a.contains(';') || a.contains('`') || a.contains('$'))
    {
        bail!("invalid game argument");
    }
    fix.applied_at = now();
    if !fix.temporary {
        let _ = crate::tries::record_on(paths, app_id, &fix.id, &fix.label);
    }
    let mut all = load(paths, app_id);
    all.fixes.retain(|f| f.id != fix.id);
    all.fixes.push(fix);
    write_json(&path(paths, app_id), &all)
}

pub fn remove(paths: &Paths, app_id: &str, id: &str) -> Result<bool> {
    let _ = crate::tries::record_off(paths, app_id, id, None);
    let mut all = load(paths, app_id);
    let before = all.fixes.len();
    all.fixes.retain(|f| f.id != id);
    if all.fixes.is_empty() {
        let _ = fs::remove_file(path(paths, app_id));
    } else {
        write_json(&path(paths, app_id), &all)?;
    }
    Ok(all.fixes.len() != before)
}

/// Record the player's answer to "Did it work?" for an applied fix.
pub fn set_verdict(paths: &Paths, app_id: &str, id: &str, worked: bool) -> Result<()> {
    let mut all = load(paths, app_id);
    let Some(f) = all.fixes.iter_mut().find(|f| f.id == id) else {
        bail!("that fix isn't applied to this game");
    };
    f.verdict = Some(worked);
    write_json(&path(paths, app_id), &all)
}

/// Every game that has fixes applied.
pub fn games_with_fixes(paths: &Paths) -> Vec<String> {
    fs::read_dir(paths.data.join("fixes"))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .strip_suffix(".json")
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn your_settings_are_parsed_and_checked() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        let f = set_your_settings(&p, "1240440", "VKD3D_CONFIG=no_upload_hvv  -force-d3d12")
            .unwrap()
            .unwrap();
        assert_eq!(f.env["VKD3D_CONFIG"], "no_upload_hvv");
        assert_eq!(f.args, vec!["-force-d3d12"]);
        assert_eq!(
            f.label,
            "Your settings: VKD3D_CONFIG=no_upload_hvv  -force-d3d12"
        );
        assert_eq!(load(&p, "1240440").fixes.len(), 1);
        for bad in [
            "LD_PRELOAD=/tmp/x.so",
            "game-performance %command%",
            "X=$(rm)",
            "lower=1",
            "A=1;B",
        ] {
            assert!(set_your_settings(&p, "1240440", bad).is_err(), "{bad}");
        }
        assert!(set_your_settings(&p, "1240440", "  ").unwrap().is_none());
        assert!(load(&p, "1240440").fixes.is_empty());
    }

    use super::*;

    fn fix(id: &str, env: &[(&str, &str)]) -> Fix {
        Fix {
            id: id.into(),
            label: id.into(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            args: vec![],
            applied_at: String::new(),
            temporary: false,
            verdict: None,
        }
    }

    #[test]
    fn apply_replace_and_remove() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        apply(
            &p,
            "1",
            fix("layers", &[("VK_LOADER_LAYERS_DISABLE", "VK_LAYER_AFMF")]),
        )
        .unwrap();
        apply(
            &p,
            "1",
            fix(
                "layers",
                &[(
                    "VK_LOADER_LAYERS_DISABLE",
                    "VK_LAYER_AFMF,VK_LAYER_MAKO_render",
                )],
            ),
        )
        .unwrap();
        let f = load(&p, "1");
        assert_eq!(f.fixes.len(), 1, "same id replaces");
        assert_eq!(
            f.fixes[0].env["VK_LOADER_LAYERS_DISABLE"],
            "VK_LAYER_AFMF,VK_LAYER_MAKO_render"
        );
        assert_eq!(games_with_fixes(&p), vec!["1"]);
        assert!(remove(&p, "1", "layers").unwrap());
        assert!(load(&p, "1").fixes.is_empty());
        assert!(games_with_fixes(&p).is_empty());
    }

    #[test]
    fn refuses_unsafe_values() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        assert!(apply(&p, "1", fix("x", &[("LD_PRELOAD", "/tmp/evil.so")])).is_err());
        let mut f = fix("y", &[]);
        f.args = vec!["-a; rm -rf ~".into()];
        assert!(apply(&p, "1", f).is_err());
    }
}
