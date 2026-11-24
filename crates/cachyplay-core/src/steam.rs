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
    /// Last login time from loginusers.vdf (0 if unknown).
    pub timestamp: u64,
}

#[derive(Debug, Clone)]
pub struct Game {
    pub app_id: String,
    pub name: String,
    pub install_dir: PathBuf,
    pub library: PathBuf,
    pub build_id: String,
    /// When Steam last updated the game (unix time from the app manifest).
    pub last_updated: Option<i64>,
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
                timestamp: u
                    .get("Timestamp")
                    .and_then(|n| n.str())
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(0),
            });
        }
        Ok(out)
    }

    /// Account folders under `userdata/` that hold a real config (skips `0`, Steam's anonymous folder).
    pub fn account_dirs(&self) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(self.root.join("userdata"))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        out.retain(|id| {
            id != "0"
                && id.chars().all(|c| c.is_ascii_digit())
                && self.localconfig_path(id).is_file()
        });
        out.sort();
        out
    }

    /// The account Steam last used. In order: the one marked MostRecent, the newest login
    /// Timestamp, the only account folder, or the folder whose settings changed most recently.
    pub fn current_account(&self) -> Result<Account> {
        let accounts = self.accounts().unwrap_or_default();
        let dirs = self.account_dirs();
        let has_dir = |a: &Account| dirs.contains(&a.account_id);
        if let Some(a) = accounts.iter().find(|a| a.most_recent && has_dir(a)) {
            return Ok(a.clone());
        }
        if let Some(a) = accounts
            .iter()
            .filter(|a| a.timestamp > 0 && has_dir(a))
            .max_by_key(|a| a.timestamp)
        {
            return Ok(a.clone());
        }
        let newest = dirs.iter().max_by_key(|id| {
            fs::metadata(self.localconfig_path(id))
                .and_then(|m| m.modified())
                .ok()
        });
        match newest {
            Some(id) => Ok(accounts
                .iter()
                .find(|a| &a.account_id == id)
                .cloned()
                .unwrap_or(Account {
                    account_id: id.clone(),
                    persona: String::new(),
                    most_recent: true,
                    timestamp: 0,
                })),
            None => bail!("No Steam account found. Open Steam and log in once, then try again."),
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
                    last_updated: get("LastUpdated").parse().ok(),
                    library: lib.clone(),
                    app_id,
                });
            }
        }
        games.sort_by_key(|g| g.name.to_lowercase());
        games
    }

    /// The compatibility tool chosen for a game in Steam (e.g. "proton_experimental"),
    /// or None when Steam's default is used.
    pub fn compat_tool(&self, app_id: &str) -> Option<String> {
        let text = fs::read_to_string(self.root.join("config/config.vdf")).ok()?;
        let doc = Doc::parse(&text).ok()?;
        let name = doc.get_str(&[
            "InstallConfigStore",
            "Software",
            "Valve",
            "Steam",
            "CompatToolMapping",
            app_id,
            "name",
        ])?;
        (!name.is_empty()).then(|| name.to_string())
    }

    /// Steam's own console log (current and previous run), used to see which game was running when.
    pub fn console_logs(&self) -> String {
        let mut out = String::new();
        for name in [
            "console_log.previous.txt",
            "console_log.txt",
            "console-linux.txt",
        ] {
            if let Ok(t) = fs::read(self.root.join("logs").join(name)) {
                out.push_str(&String::from_utf8_lossy(&t));
                out.push('\n');
            }
        }
        out
    }

    /// The game's Proton prefix, if it runs through Proton.
    pub fn compatdata(&self, game: &Game) -> Option<PathBuf> {
        let p = game.library.join("steamapps/compatdata").join(&game.app_id);
        p.is_dir().then_some(p)
    }

    /// Unity games write Player.log (and Player-prev.log) inside the Proton prefix.
    pub fn unity_logs(&self, game: &Game) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let base = match self.compatdata(game) {
            Some(c) => c.join("pfx/drive_c/users/steamuser/AppData/LocalLow"),
            None => return out,
        };
        let Ok(companies) = fs::read_dir(&base) else {
            return out;
        };
        for company in companies.flatten() {
            let Ok(products) = fs::read_dir(company.path()) else {
                continue;
            };
            for product in products.flatten() {
                for name in ["Player.log", "Player-prev.log"] {
                    let p = product.path().join(name);
                    if p.is_file() {
                        out.push(p);
                    }
                }
            }
        }
        out
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
        for id in ["10", "22202"] {
            let c = root.join("userdata").join(id).join("config");
            fs::create_dir_all(&c).unwrap();
            fs::write(
                c.join("localconfig.vdf"),
                "\"UserLocalConfigStore\"\n{\n}\n",
            )
            .unwrap();
        }
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

    fn steam_with(users: &str, dirs: &[&str]) -> (tempfile::TempDir, Steam) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".local/share/Steam");
        fs::create_dir_all(root.join("config")).unwrap();
        fs::create_dir_all(root.join("steamapps")).unwrap();
        fs::write(
            root.join("config/loginusers.vdf"),
            format!("\"users\"\n{{\n{users}}}\n"),
        )
        .unwrap();
        for d in dirs {
            let c = root.join("userdata").join(d).join("config");
            fs::create_dir_all(&c).unwrap();
            fs::write(
                c.join("localconfig.vdf"),
                "\"UserLocalConfigStore\"\n{\n}\n",
            )
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let s = Steam::locate(dir.path()).unwrap();
        (dir, s)
    }

    #[test]
    fn several_accounts_without_most_recent_flag() {
        // Matt's case: an anonymous "0" folder plus real accounts, no MostRecent marker.
        let user = |id64: &str, ts: &str| {
            format!("\t\"{id64}\"\n\t{{\n\t\t\"PersonaName\"\t\t\"p{ts}\"\n\t\t\"Timestamp\"\t\t\"{ts}\"\n\t}}\n")
        };
        let users = user("76561197960265738", "100") + &user("76561197960287930", "200");
        let (_d, s) = steam_with(&users, &["0", "10", "22202"]);
        assert_eq!(
            s.current_account().unwrap().account_id,
            "22202",
            "newest timestamp wins"
        );
        // No timestamps at all: the most recently changed settings folder wins; "0" is ignored.
        let (_d, s) = steam_with("", &["0", "22202", "10"]);
        assert_eq!(s.account_dirs(), vec!["10", "22202"]);
        assert_eq!(s.current_account().unwrap().account_id, "10");
        let (_d, s) = steam_with("", &["0"]);
        assert!(s.current_account().is_err());
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
