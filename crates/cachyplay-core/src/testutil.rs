//! A fake home folder with a realistic native Steam install, for tests.

use crate::manage::Ctx;
use crate::paths::Paths;
use crate::steam::Steam;
use std::fs;
use std::path::{Path, PathBuf};

pub const ACCOUNT_ID: &str = "22202";

pub struct FakeSteam {
    pub dir: tempfile::TempDir,
}

impl FakeSteam {
    pub fn new() -> FakeSteam {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let root = home.join(".local/share/Steam");
        let w = |p: PathBuf, s: &str| {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, s).unwrap();
        };
        w(root.join("config/loginusers.vdf"), "\"users\"\n{\n\t\"76561197960287930\"\n\t{\n\t\t\"AccountName\"\t\t\"tester\"\n\t\t\"PersonaName\"\t\t\"Tester\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}\n}\n");
        w(
            root.join(format!("userdata/{ACCOUNT_ID}/config/localconfig.vdf")),
            "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"1240440\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1763900000\"\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"%command% -nosplash\"\n\t\t\t\t\t}\n\t\t\t\t\t\"570\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n\t\"friends\"\n\t{\n\t\t\"PersonaName\"\t\t\"Tester\"\n\t}\n}\n",
        );
        w(
            root.join("steamapps/libraryfolders.vdf"),
            &format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                root.display()
            ),
        );
        for (id, name, dir) in [
            ("1240440", "Halo Infinite", "Halo Infinite"),
            ("570", "Dota 2", "dota 2 beta"),
            ("1493710", "Proton Experimental", "Proton - Experimental"),
        ] {
            w(
                root.join(format!("steamapps/appmanifest_{id}.acf")),
                &format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{name}\"\n\t\"installdir\"\t\t\"{dir}\"\n\t\"buildid\"\t\t\"20000000\"\n}}\n"),
            );
            fs::create_dir_all(root.join("steamapps/common").join(dir)).unwrap();
        }
        fs::create_dir_all(home.join("proc/1")).unwrap();
        fs::write(home.join("proc/1/comm"), "systemd\n").unwrap();
        fs::create_dir_all(home.join("Desktop")).unwrap();
        FakeSteam { dir }
    }

    pub fn home(&self) -> &Path {
        self.dir.path()
    }

    pub fn localconfig(&self) -> PathBuf {
        self.home().join(format!(
            ".local/share/Steam/userdata/{ACCOUNT_ID}/config/localconfig.vdf"
        ))
    }

    pub fn start_steam(&self) {
        fs::create_dir_all(self.home().join("proc/4242")).unwrap();
        fs::write(self.home().join("proc/4242/comm"), "steam\n").unwrap();
    }

    pub fn paths(&self) -> Paths {
        Paths::isolated(self.home())
    }

    pub fn ctx(&self) -> Ctx {
        let steam = Steam::locate(self.home()).unwrap();
        let account = steam.current_account().unwrap();
        Ctx {
            paths: self.paths(),
            steam,
            account,
            proc_root: self.home().join("proc"),
        }
    }
}

impl Default for FakeSteam {
    fn default() -> Self {
        Self::new()
    }
}
