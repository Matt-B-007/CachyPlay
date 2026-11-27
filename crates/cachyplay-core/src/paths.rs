// SPDX-License-Identifier: GPL-3.0-or-later
//! Where CachyPlay keeps its files (XDG base directories) and where it is allowed to run from.

use anyhow::{bail, Context, Result};
use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
}

impl Paths {
    pub fn from_env() -> Result<Paths> {
        let home = PathBuf::from(env::var_os("HOME").context("HOME is not set")?);
        Ok(Paths::for_home(&home))
    }

    /// XDG directories, honouring `XDG_*_HOME` when set to an absolute path.
    pub fn for_home(home: &Path) -> Paths {
        let xdg = |var: &str, default: &str| {
            env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(default))
                .join("cachyplay")
        };
        Paths {
            home: home.to_path_buf(),
            config: xdg("XDG_CONFIG_HOME", ".config"),
            data: xdg("XDG_DATA_HOME", ".local/share"),
            state: xdg("XDG_STATE_HOME", ".local/state"),
            cache: xdg("XDG_CACHE_HOME", ".cache"),
        }
    }

    /// Default XDG locations under `home`, ignoring `XDG_*` variables (tests, fake homes).
    pub fn isolated(home: &Path) -> Paths {
        Paths {
            home: home.to_path_buf(),
            config: home.join(".config/cachyplay"),
            data: home.join(".local/share/cachyplay"),
            state: home.join(".local/state/cachyplay"),
            cache: home.join(".cache/cachyplay"),
        }
    }

    pub fn snapshots(&self, account: &str) -> PathBuf {
        self.data.join("snapshots").join(account)
    }
    pub fn changes(&self, account: &str) -> PathBuf {
        self.data.join("changes").join(account)
    }
    pub fn backups(&self, account: &str) -> PathBuf {
        self.data.join("backups").join(account)
    }
    pub fn recipes(&self) -> PathBuf {
        self.data.join("recipes")
    }
    pub fn launches(&self) -> PathBuf {
        self.state.join("launches")
    }
    pub fn log_file(&self) -> PathBuf {
        self.state.join("cachyplay.log")
    }
    pub fn install_manifest(&self) -> PathBuf {
        self.data.join("install-manifest.json")
    }
    pub fn user_bin(&self) -> PathBuf {
        self.home.join(".local/bin/cachyplay")
    }

    /// Where bug reports go: the XDG Desktop, then ~/Desktop, ~/Downloads, then home.
    pub fn report_dir(&self) -> PathBuf {
        let from_xdg = std::fs::read_to_string(self.home.join(".config/user-dirs.dirs"))
            .ok()
            .and_then(|t| {
                t.lines().find_map(|l| {
                    let v = l.strip_prefix("XDG_DESKTOP_DIR=")?.trim_matches('"');
                    Some(PathBuf::from(
                        v.replace("$HOME", &self.home.to_string_lossy()),
                    ))
                })
            });
        [
            from_xdg,
            Some(self.home.join("Desktop")),
            Some(self.home.join("Downloads")),
        ]
        .into_iter()
        .flatten()
        .find(|p| p.is_dir())
        .unwrap_or_else(|| self.home.clone())
    }
}

/// The places a real install lives. CachyPlay only edits games when running from one of these,
/// so Steam's launch options never point into ~/Downloads or a build folder.
pub fn allowed_install_locations(paths: &Paths) -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/bin/cachyplay"),
        PathBuf::from("/usr/local/bin/cachyplay"),
        paths.user_bin(),
    ]
}

/// The installed binary path to write into Steam, or an error explaining how to install.
/// Set `CACHYPLAY_DEV=1` to allow any location (development and tests only).
pub fn installed_binary(paths: &Paths) -> Result<PathBuf> {
    let exe = env::current_exe().context("cannot determine CachyPlay's own location")?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    if env::var_os("CACHYPLAY_DEV").is_some_and(|v| v == "1") {
        return Ok(exe);
    }
    check_location(&exe, paths)
}

pub fn check_location(exe: &Path, paths: &Paths) -> Result<PathBuf> {
    for allowed in allowed_install_locations(paths) {
        let canon = std::fs::canonicalize(&allowed).unwrap_or(allowed.clone());
        if exe == canon || exe == allowed {
            return Ok(allowed);
        }
    }
    bail!(
        "CachyPlay is running from {}, which is not an installed location.\n\
         Install it first so Steam can always find it:\n  \
         ./cachyplay install      (copies it to {})\n\
         or install the AUR package `cachyplay`. After installing, you can delete the downloaded file.",
        exe.display(),
        paths.user_bin().display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_folder_is_rejected() {
        let p = Paths::for_home(Path::new("/home/u"));
        let err = check_location(Path::new("/home/u/Downloads/cachyplay"), &p).unwrap_err();
        assert!(err.to_string().contains("cachyplay install"));
    }

    #[test]
    fn installed_locations_are_accepted() {
        let p = Paths::for_home(Path::new("/home/u"));
        assert_eq!(
            check_location(Path::new("/usr/bin/cachyplay"), &p).unwrap(),
            PathBuf::from("/usr/bin/cachyplay")
        );
        assert!(check_location(Path::new("/home/u/.local/bin/cachyplay"), &p).is_ok());
    }
}
