//! Steam compatibility tools (Proton versions): which ones are installed, which one a game is forced
//! to use, and switching a game to another one, the same as Steam's Properties → Compatibility →
//! "Force the use of a specific Steam Play compatibility tool". Needs Steam closed, like every
//! change CachyPlay makes to Steam's files, and can be undone.

use crate::fsutil::{atomic_write, backup};
use crate::manage::{read_json, write_json, Ctx};
use crate::vdf::Doc;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tool {
    /// Steam's internal name, written to config.vdf ("proton_experimental", "GE-Proton11-12").
    pub name: String,
    /// The tool's folder name, as it appears in Steam's launch commands.
    pub folder: String,
    /// What Steam shows in its menu.
    pub display: String,
}

/// A friendly name for a tool folder: "GE-Proton11-12-x86_64" → "GE-Proton11-12".
pub fn friendly(folder: &str) -> String {
    folder.trim_end_matches("-x86_64").to_string()
}

fn tool_from_vdf(dir: &Path) -> Option<Tool> {
    let text = fs::read_to_string(dir.join("compatibilitytool.vdf")).ok()?;
    let doc = Doc::parse(&text).ok()?;
    let tools = doc.lookup(&["compatibilitytools", "compat_tools"])?;
    let first = tools.children().first()?;
    let folder = dir.file_name()?.to_string_lossy().into_owned();
    let display = first
        .get("display_name")
        .and_then(|n| n.str())
        .map(str::to_string)
        .unwrap_or_else(|| friendly(&folder));
    Some(Tool {
        name: first.key.clone(),
        folder,
        display,
    })
}

/// Compatibility tools installed on this PC: Proton Experimental (from Steam) and everything in the
/// compatibilitytools.d folders (GE-Proton, proton-cachyos, …).
pub fn installed(steam_root: &Path, libraries: &[PathBuf]) -> Vec<Tool> {
    let mut out: Vec<Tool> = Vec::new();
    for lib in libraries {
        if lib
            .join("steamapps/common/Proton - Experimental/proton")
            .is_file()
        {
            out.push(Tool {
                name: "proton_experimental".into(),
                folder: "Proton - Experimental".into(),
                display: "Proton Experimental".into(),
            });
            break;
        }
    }
    for dir in [
        steam_root.join("compatibilitytools.d"),
        PathBuf::from("/usr/share/steam/compatibilitytools.d"),
    ] {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for d in dirs {
            if let Some(t) = tool_from_vdf(&d) {
                if !out.iter().any(|x| x.name == t.name) {
                    out.push(t);
                }
            }
        }
    }
    out
}

const MAPPING: [&str; 5] = [
    "InstallConfigStore",
    "Software",
    "Valve",
    "Steam",
    "CompatToolMapping",
];

fn config_path(ctx: &Ctx) -> PathBuf {
    ctx.steam.root.join("config/config.vdf")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct UndoRecord {
    /// What the game was forced to before CachyPlay changed it ("" or None: Steam's default).
    previous: Option<String>,
    /// What CachyPlay set.
    set: String,
}

fn undo_path(ctx: &Ctx, app_id: &str) -> PathBuf {
    ctx.paths
        .data
        .join("compat-tool")
        .join(format!("{app_id}.json"))
}

fn write_mapping(ctx: &Ctx, app_id: &str, name: &str) -> Result<()> {
    let path = config_path(ctx);
    let text = fs::read_to_string(&path)
        .with_context(|| format!("reading {} (has Steam run at least once?)", path.display()))?;
    fn field<'a>(app_id: &'a str, key: &'a str) -> Vec<&'a str> {
        let mut p: Vec<&str> = MAPPING.to_vec();
        p.push(app_id);
        p.push(key);
        p
    }
    let mut new = Doc::parse(&text)?.with_str(&field(app_id, "name"), name)?;
    new = Doc::parse(&new)?.with_str(&field(app_id, "config"), "")?;
    new = Doc::parse(&new)?.with_str(
        &field(app_id, "priority"),
        if name.is_empty() { "0" } else { "250" },
    )?;
    backup(&path, &ctx.paths.data.join("backups/steam-config"), 5)?;
    let mut allowed: Vec<String> = MAPPING.iter().map(|k| k.to_ascii_lowercase()).collect();
    allowed.push(app_id.to_string());
    crate::fsutil::verified_vdf_write(&path, &text, &new, &[allowed.join("/")])?;
    if ctx.steam.compat_tool(app_id).unwrap_or_default() != name {
        let _ = atomic_write(&path, text.as_bytes());
        bail!("verification failed after writing Steam's compatibility setting for {app_id}; the original was restored");
    }
    Ok(())
}

/// Force `app_id` to use `tool`. Remembers what it was before, for undo.
pub fn switch(ctx: &Ctx, app_id: &str, tool: &Tool) -> Result<()> {
    ctx.require_editable()?;
    let previous = ctx.steam.compat_tool(app_id);
    write_mapping(ctx, app_id, &tool.name)?;
    let p = undo_path(ctx, app_id);
    let mut rec: UndoRecord = read_json(&p).unwrap_or(UndoRecord {
        previous,
        set: String::new(),
    });
    rec.set = tool.name.clone();
    write_json(&p, &rec)
}

/// The tool CachyPlay switched this game to, if it did.
pub fn switched_to(ctx: &Ctx, app_id: &str) -> Option<String> {
    read_json::<UndoRecord>(&undo_path(ctx, app_id)).map(|r| r.set)
}

/// Put the game back on the tool it used before CachyPlay changed it.
pub fn undo(ctx: &Ctx, app_id: &str) -> Result<()> {
    ctx.require_editable()?;
    let p = undo_path(ctx, app_id);
    let rec: UndoRecord =
        read_json(&p).context("CachyPlay hasn't changed this game's compatibility tool")?;
    write_mapping(ctx, app_id, rec.previous.as_deref().unwrap_or(""))?;
    fs::remove_file(&p)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::FakeSteam;

    fn tool_dir(root: &Path, folder: &str, name: &str, display: &str) {
        let d = root.join("compatibilitytools.d").join(folder);
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("compatibilitytool.vdf"),
            format!("\"compatibilitytools\"\n{{\n\t\"compat_tools\"\n\t{{\n\t\t\"{name}\"\n\t\t{{\n\t\t\t\"install_path\" \".\"\n\t\t\t\"display_name\" \"{display}\"\n\t\t}}\n\t}}\n}}\n"),
        )
        .unwrap();
    }

    #[test]
    fn finds_tools_and_switches_with_undo() {
        let f = FakeSteam::new();
        let root = f.steam_root();
        tool_dir(
            &root,
            "GE-Proton11-12-x86_64",
            "GE-Proton11-12",
            "GE-Proton11-12",
        );
        fs::create_dir_all(root.join("steamapps/common/Proton - Experimental")).unwrap();
        fs::write(
            root.join("steamapps/common/Proton - Experimental/proton"),
            "",
        )
        .unwrap();
        let tools = installed(&root, std::slice::from_ref(&root));
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"proton_experimental") && names.contains(&"GE-Proton11-12"),
            "{names:?}"
        );
        let ge = tools.iter().find(|t| t.name == "GE-Proton11-12").unwrap();
        assert_eq!(ge.folder, "GE-Proton11-12-x86_64");

        let ctx = f.ctx();
        let before = ctx.steam.compat_tool("1240440");
        switch(&ctx, "1240440", ge).unwrap();
        assert_eq!(
            ctx.steam.compat_tool("1240440").as_deref(),
            Some("GE-Proton11-12")
        );
        assert_eq!(
            switched_to(&ctx, "1240440").as_deref(),
            Some("GE-Proton11-12")
        );
        undo(&ctx, "1240440").unwrap();
        assert_eq!(ctx.steam.compat_tool("1240440"), before);
        assert!(switched_to(&ctx, "1240440").is_none());
    }

    #[test]
    fn friendly_names() {
        assert_eq!(friendly("GE-Proton11-12-x86_64"), "GE-Proton11-12");
        assert_eq!(friendly("Proton - Experimental"), "Proton - Experimental");
    }
}
