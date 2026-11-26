//! Installing a downloaded copy into its own location (~/.local/bin), and removing it again.
//! Package-manager installs (/usr/bin) are owned by pacman and never touched here.

use crate::fsutil::atomic_write;
use crate::manage::{read_json, write_json};
use crate::paths::Paths;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const ICON_SVG: &str = include_str!("../../../assets/cachyplay.svg");

/// The GUI is a separate binary on purpose: the launch wrapper (`cachyplay`) runs before every
/// game and must start instantly without loading WebKit/GTK.
pub const GUI_BIN: &str = "cachyplay-gui";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub files: Vec<PathBuf>,
}

/// The app's ID (tauri.conf.json "identifier"): the name Wayland knows the window by.
pub const APP_ID: &str = "io.github.matt-b-007.cachyplay";

pub fn desktop_entry(gui: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=CachyPlay\nGenericName=Game Helper\nComment=One-click game tuning, crash help and bug reports\nExec={}\nIcon=cachyplay\nTerminal=false\nCategories=Game;Utility;\nKeywords=steam;proton;gaming;cachyos;\nStartupWMClass=cachyplay-gui\n",
        gui.display()
    )
}

/// Copy the running binary to ~/.local/bin/cachyplay, plus `cachyplay-gui` if it sits next to it,
/// and add an icon and a menu entry. Returns the installed `cachyplay` path.
pub fn install_user(paths: &Paths, from: &Path) -> Result<PathBuf> {
    if from.starts_with("/usr") {
        bail!("This copy was installed by the package manager; nothing to do.");
    }
    let bin = paths.user_bin();
    copy_exe(from, &bin)?;
    let mut files = vec![bin.clone()];

    let icon = paths
        .home
        .join(".local/share/icons/hicolor/scalable/apps/cachyplay.svg");
    atomic_write(&icon, ICON_SVG.as_bytes())?;
    files.push(icon);

    let gui_src = from.with_file_name(GUI_BIN);
    if gui_src.is_file() {
        let gui = bin.with_file_name(GUI_BIN);
        if gui_src != gui {
            copy_exe(&gui_src, &gui)?;
        }
        files.push(gui.clone());
        // Wayland desktops match a window to its menu entry by name, and that's where the title
        // bar and taskbar icon come from. The window calls itself by the app's ID (or, on some
        // setups, by its program name), so there's an entry for each; the second is hidden from
        // the menu. The old "cachyplay.desktop" matched neither, which showed KDE's fallback "W".
        let apps = paths.home.join(".local/share/applications");
        let _ = fs::remove_file(apps.join("cachyplay.desktop"));
        let main = apps.join(format!("{APP_ID}.desktop"));
        atomic_write(&main, desktop_entry(&gui).as_bytes())?;
        files.push(main);
        let alias = apps.join(format!("{GUI_BIN}.desktop"));
        atomic_write(
            &alias,
            format!("{}NoDisplay=true\n", desktop_entry(&gui)).as_bytes(),
        )?;
        files.push(alias);
    }

    write_json(
        &paths.install_manifest(),
        &Manifest {
            version: crate::VERSION.into(),
            files,
        },
    )?;
    Ok(bin)
}

/// Bring an existing install's menu entries up to date (an update replaces the programs, not
/// the entries). Only for the copy in ~/.local/bin, and only when something is out of date.
pub fn refresh_menu_entries(paths: &Paths) -> bool {
    let gui = paths.user_bin().with_file_name(GUI_BIN);
    if !gui.is_file() {
        return false;
    }
    let apps = paths.home.join(".local/share/applications");
    let main = apps.join(format!("{APP_ID}.desktop"));
    let alias = apps.join(format!("{GUI_BIN}.desktop"));
    let old = apps.join("cachyplay.desktop");
    let want = desktop_entry(&gui);
    let current = fs::read_to_string(&main).unwrap_or_default();
    if current == want && alias.exists() && !old.exists() {
        return false;
    }
    let _ = fs::remove_file(&old);
    let ok = atomic_write(&main, want.as_bytes()).is_ok()
        && atomic_write(&alias, format!("{want}NoDisplay=true\n").as_bytes()).is_ok();
    if ok {
        if let Some(mut m) = read_json::<Manifest>(&paths.install_manifest()) {
            m.files.retain(|f| f != &old);
            for f in [&main, &alias] {
                if !m.files.contains(f) {
                    m.files.push(f.clone());
                }
            }
            let _ = write_json(&paths.install_manifest(), &m);
        }
    }
    ok
}

fn copy_exe(from: &Path, to: &Path) -> Result<()> {
    if from == to {
        return Ok(());
    }
    let bytes = fs::read(from).with_context(|| format!("reading {}", from.display()))?;
    atomic_write(to, &bytes)?;
    fs::set_permissions(to, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

/// Remove the files `install_user` created, plus CachyPlay's caches and logs.
/// Save backups and bug reports are kept unless `remove_backups` is set.
/// Only paths CachyPlay recorded or owns are touched.
pub fn remove_user_files(paths: &Paths, remove_backups: bool) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    let manifest: Manifest = read_json(&paths.install_manifest()).unwrap_or_default();
    for f in manifest
        .files
        .into_iter()
        .chain(crate::update::prev_files(paths))
    {
        if fs::remove_file(&f).is_ok() {
            removed.push(f);
        }
    }
    let mut dirs = vec![
        paths.cache.clone(),
        paths.state.clone(),
        paths.config.clone(),
        paths.data.join("recipes"),
    ];
    for sub in ["snapshots", "changes", "fixes"] {
        dirs.push(paths.data.join(sub));
    }
    if remove_backups {
        dirs.push(paths.data.clone());
    }
    for d in dirs {
        if d.exists() && fs::remove_dir_all(&d).is_ok() {
            removed.push(d);
        }
    }
    let _ = fs::remove_file(paths.install_manifest());
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_and_remove_user_copy() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::isolated(d.path());
        let dl = d.path().join("Downloads/cachyplay");
        fs::create_dir_all(dl.parent().unwrap()).unwrap();
        fs::write(&dl, b"#!/bin/sh\n").unwrap();
        fs::write(dl.with_file_name(GUI_BIN), b"#!/bin/sh\n").unwrap();
        let bin = install_user(&paths, &dl).unwrap();
        assert!(d.path().join(".local/bin/cachyplay-gui").exists());
        let desktop = fs::read_to_string(
            d.path()
                .join(".local/share/applications/io.github.matt-b-007.cachyplay.desktop"),
        )
        .unwrap();
        assert!(desktop.contains(".local/bin/cachyplay-gui"), "{desktop}");
        let alias = fs::read_to_string(
            d.path()
                .join(".local/share/applications/cachyplay-gui.desktop"),
        )
        .unwrap();
        assert!(alias.contains("NoDisplay=true") && alias.contains("Icon=cachyplay"));
        assert!(!d
            .path()
            .join(".local/share/applications/cachyplay.desktop")
            .exists());
        assert_eq!(bin, d.path().join(".local/bin/cachyplay"));
        assert!(crate::sys::is_executable(&bin));
        assert!(d
            .path()
            .join(".local/share/icons/hicolor/scalable/apps/cachyplay.svg")
            .exists());
        fs::create_dir_all(paths.data.join("backups")).unwrap();
        remove_user_files(&paths, false).unwrap();
        assert!(!bin.exists());
        assert!(
            paths.data.join("backups").exists(),
            "backups kept by default"
        );
        assert!(dl.exists(), "the downloaded file itself is not touched");
    }
}
