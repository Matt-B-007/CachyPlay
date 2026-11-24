//! Finding Steam, the logged-in account, library folders and installed games.

use crate::vdf::Doc;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Offset between a 64-bit SteamID and the 32-bit account ID used for `userdata/<id>`.
const STEAMID64_BASE: u64 = 76_561_197_960_265_728;

pub const APPS_PATH: [&str; 5] = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    Native,
    Flatpak,
}

#[derive(Debug, Clone)]
pub struct Steam {
    pub root: PathBuf,
    pub kind: InstallKind,
}

#[derive(Debug, Clone)]
pub struct Account {
    /// 32-bit account ID: the folder name under `userdata/`.
    pub account_id: String,
    pub persona: String,
    pub most_recent: bool,
}

#[derive(Debug, Clone)]
pub struct Game {
    pub app_id: String,
    pub name: String,
    pub install_dir: PathBuf,
    pub library: PathBuf,
    pub build_id: String,
}

impl Steam {
    /// Locate Steam under `home`. Native installs win over Flatpak when both exist.
    pub fn locate(home: &Path) -> Result<Steam> {
        let native = [
            home.join(".local/share/Steam"),
            home.join(".steam/steam"),
            home.join(".steam/root"),
        ];
        for p in native {
            if p.join("steamapps").is_dir() || p.join("userdata").is_dir() {
                let root = fs::canonicalize(&p).unwrap_or(p);
                return Ok(Steam {
                    root,
                    kind: InstallKind::Native,
                });
            }
        }
        let flatpak = home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
        if flatpak.is_dir() {
            return Ok(Steam {
                root: flatpak,
                kind: InstallKind::Flatpak,
            });
        }
        bail!("Steam was not found in {}", home.display())
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let path = self.root.join("config/loginusers.vdf");
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let doc = Doc::parse(&text)?;
        let users = doc
            .lookup(&["users"])
            .context("loginusers.vdf has no \"users\" block")?;
        let mut out = Vec::new();
        for u in users.children() {
            let Ok(id64) = u.key.parse::<u64>() else {
                continue;
            };
            let Some(account) = id64.checked_sub(STEAMID64_BASE) else {
                continue;
            };
            out.push(Account {
                account_id: account.to_string(),
                persona: u
                    .get("PersonaName")
                    .and_then(|n| n.str())
                    .unwrap_or("")
                    .to_string(),
                most_recent: u.get("MostRecent").and_then(|n| n.str()) == Some("1"),
            });
        }
        Ok(out)
    }

    /// The account Steam last logged into, falling back to the only `userdata` folder.
    pub fn current_account(&self) -> Result<Account> {
        let accounts = self.accounts().unwrap_or_default();
        if let Some(a) = accounts.iter().find(|a| a.most_recent) {
            return Ok(a.clone());
        }
        let dirs: Vec<_> = fs::read_dir(self.root.join("userdata"))
            .map(|rd| rd.flatten().filter(|e| e.path().is_dir()).collect())
            .unwrap_or_default();
        match dirs.as_slice() {
            [one] => Ok(Account {
                account_id: one.file_name().to_string_lossy().into_owned(),
                persona: String::new(),
                most_recent: true,
            }),
            [] => bail!("no Steam account found; log into Steam once first"),
            _ => bail!("several Steam accounts found and none is marked as most recent; log into Steam once"),
        }
    }

    pub fn localconfig_path(&self, account_id: &str) -> PathBuf {
        self.root
            .join("userdata")
            .join(account_id)
            .join("config/localconfig.vdf")
    }

    pub fn library_folders(&self) -> Vec<PathBuf> {
        let mut out = vec![self.root.clone()];
        let path = self.root.join("steamapps/libraryfolders.vdf");
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(doc) = Doc::parse(&text) {
                if let Some(lf) = doc.lookup(&["libraryfolders"]) {
                    for entry in lf.children() {
                        if let Some(p) = entry.get("path").and_then(|n| n.str()) {
                            let p = PathBuf::from(p);
                            let canon = fs::canonicalize(&p).unwrap_or(p);
                            if !out.contains(&canon) {
                                out.push(canon);
                            }
                        }
                    }
                }
            }
        }
        out
    }

    pub fn games(&self) -> Vec<Game> {
        let mut games = Vec::new();
        for lib in self.library_folders() {
            let Ok(rd) = fs::read_dir(lib.join("steamapps")) else {
                continue;
            };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                    continue;
                }
                let Ok(text) = fs::read_to_string(e.path()) else {
                    continue;
                };
                let Ok(doc) = Doc::parse(&text) else { continue };
                let get = |k: &str| doc.get_str(&["AppState", k]).unwrap_or("").to_string();
                let app_id = get("appid");
                if app_id.is_empty() || is_tool(&get("name")) {
                    continue;
                }
                games.push(Game {
                    install_dir: lib.join("steamapps/common").join(get("installdir")),
                    name: get("name"),
                    build_id: get("buildid"),
                    library: lib.clone(),
                    app_id,
                });
            }
        }
        games.sort_by_key(|g| g.name.to_lowercase());
        games
    }

    pub fn find_game(&self, app_id: &str) -> Option<Game> {
        self.games().into_iter().find(|g| g.app_id == app_id)
    }
}

/// Proton builds and the Steam Linux Runtime show up as "games"; hide them.
fn is_tool(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with("proton")
        || n.starts_with("steam linux runtime")
        || n.starts_with("steamworks common")
}

/// True when a Steam client process is running (checked via /proc).
pub fn is_running() -> bool {
    is_running_in(Path::new("/proc"))
}

pub fn is_running_in(proc_root: &Path) -> bool {
    let Ok(rd) = fs::read_dir(proc_root) else {
        return false;
    };
    rd.flatten().any(|e| {
        let pid_dir = e.path();
        if !e
            .file_name()
            .to_string_lossy()
            .chars()
            .all(|c| c.is_ascii_digit())
        {
            return false;
        }
        let comm = fs::read_to_string(pid_dir.join("comm")).unwrap_or_default();
        matches!(comm.trim(), "steam" | "steamwebhelper")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steamid64_to_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".local/share/Steam");
        fs::create_dir_all(root.join("config")).unwrap();
        fs::create_dir_all(root.join("steamapps")).unwrap();
        fs::write(
            root.join("config/loginusers.vdf"),
            "\"users\"\n{\n\t\"76561197960265738\"\n\t{\n\t\t\"PersonaName\"\t\t\"a\"\n\t\t\"MostRecent\"\t\t\"0\"\n\t}\n\t\"76561197960287930\"\n\t{\n\t\t\"PersonaName\"\t\t\"b\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}\n}\n",
        )
        .unwrap();
        let s = Steam::locate(dir.path()).unwrap();
        assert_eq!(s.kind, InstallKind::Native);
        let a = s.current_account().unwrap();
        assert_eq!(a.account_id, "22202");
        assert_eq!(a.persona, "b");
    }

    #[test]
    fn detects_flatpak() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(
            dir.path()
                .join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        )
        .unwrap();
        assert_eq!(
            Steam::locate(dir.path()).unwrap().kind,
            InstallKind::Flatpak
        );
    }

    #[test]
    fn steam_process_detection() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("123")).unwrap();
        fs::write(dir.path().join("123/comm"), "bash\n").unwrap();
        assert!(!is_running_in(dir.path()));
        fs::create_dir_all(dir.path().join("456")).unwrap();
        fs::write(dir.path().join("456/comm"), "steam\n").unwrap();
        assert!(is_running_in(dir.path()));
    }
}
