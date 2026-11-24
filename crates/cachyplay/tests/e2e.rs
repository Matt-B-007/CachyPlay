//! End-to-end tests: the real `cachyplay` binary against a fake home with a fake Steam install.

use cachyplay_core::testutil::FakeSteam;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_cachyplay");

fn cmd(f: &FakeSteam, args: &[&str]) -> Command {
    let mut c = Command::new(BIN);
    c.args(args)
        .env_clear()
        .env("HOME", f.home())
        .env("PATH", "/usr/bin:/bin")
        .env("CACHYPLAY_PROC_ROOT", f.home().join("proc"))
        .env("CACHYPLAY_NO_NOTIFY", "1");
    c
}

fn ok(o: Output) -> String {
    let s = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(
        o.status.success(),
        "stdout:\n{s}\nstderr:\n{}",
        String::from_utf8_lossy(&o.stderr)
    );
    s
}

fn launch_options(f: &FakeSteam) -> String {
    let text = fs::read_to_string(f.localconfig()).unwrap();
    let doc = cachyplay_core::vdf::Doc::parse(&text).unwrap();
    let mut p = cachyplay_core::steam::APPS_PATH.to_vec();
    p.extend(["1240440", "LaunchOptions"]);
    doc.get_str(&p).unwrap_or("").to_string()
}

/// A stand-in for the game: records its arguments and one env var, exits with $EXIT.
fn fake_game(dir: &Path) -> PathBuf {
    let p = dir.join("fake-game");
    fs::write(&p, "#!/bin/sh\necho \"args:$*\" > \"$OUT\"\necho \"nvapi:${PROTON_ENABLE_NVAPI:-unset}\" >> \"$OUT\"\nexit ${EXIT:-0}\n").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    p
}

/// Run a launch-options string the way Steam does: substitute %command%, run through sh.
fn steam_launch(
    f: &FakeSteam,
    options: &str,
    game: &Path,
    extra_env: &[(&str, &str)],
) -> (i32, String) {
    let out = f.home().join("game-output");
    let _ = fs::remove_file(&out);
    let line = options.replace("%command%", &game.display().to_string());
    let mut c = Command::new("sh");
    c.arg("-c")
        .arg(&line)
        .env_clear()
        .env("HOME", f.home())
        .env("PATH", "/usr/bin:/bin")
        .env("OUT", &out)
        .env("SteamAppId", "1240440")
        .env("CACHYPLAY_NO_NOTIFY", "1");
    for (k, v) in extra_env {
        c.env(k, v);
    }
    let status = c.status().unwrap();
    (
        status.code().unwrap_or(-1),
        fs::read_to_string(&out).unwrap_or_default(),
    )
}

#[test]
fn full_lifecycle() {
    let f = FakeSteam::new();
    let original = fs::read_to_string(f.localconfig()).unwrap();

    let list = ok(cmd(&f, &["list"]).output().unwrap());
    assert!(
        list.contains("Halo Infinite") && list.contains("fix available"),
        "{list}"
    );
    assert!(!list.contains("Proton Experimental"), "tools are hidden");

    // Not installed (running from the build folder): refuses to touch games.
    let o = cmd(&f, &["apply", "1240440"]).output().unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("not an installed location"));
    assert_eq!(fs::read_to_string(f.localconfig()).unwrap(), original);

    // Steam running: refuses.
    f.start_steam();
    let o = cmd(&f, &["apply", "1240440"])
        .env("CACHYPLAY_DEV", "1")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&o.stderr).contains("Steam is running"));
    fs::remove_dir_all(f.home().join("proc/4242")).unwrap();

    ok(cmd(&f, &["apply", "1240440"])
        .env("CACHYPLAY_DEV", "1")
        .output()
        .unwrap());
    let lo = launch_options(&f);
    assert!(lo.ends_with("%command% -nosplash"), "{lo}");

    // Launch through the string Steam would run: goes through the wrapper, keeps -nosplash.
    let game = fake_game(f.home());
    let (code, out) = steam_launch(&f, &lo, &game, &[("EXIT", "0")]);
    assert_eq!(code, 0);
    assert!(out.contains("args:-nosplash"), "{out}");
    let launches: Vec<_> = fs::read_dir(f.home().join(".local/state/cachyplay/launches"))
        .unwrap()
        .collect();
    assert_eq!(launches.len(), 1, "launch was recorded by the wrapper");

    // Exit codes pass through to Steam.
    let (code, _) = steam_launch(&f, &lo, &game, &[("EXIT", "3")]);
    assert_eq!(code, 3);

    // FAIL OPEN 1: an internal panic still launches the game.
    let (code, out) = steam_launch(&f, &lo, &game, &[("CACHYPLAY_TEST_PANIC", "1")]);
    assert_eq!(code, 0);
    assert!(out.contains("args:-nosplash"), "{out}");

    // FAIL OPEN 2: binary gone (package removed without restoring): the game still launches.
    let gone = lo.replace(BIN, "/nonexistent/cachyplay");
    assert_ne!(gone, lo);
    let (code, out) = steam_launch(&f, &gone, &game, &[]);
    assert_eq!(code, 0);
    assert!(out.contains("args:-nosplash"), "{out}");

    // Status and report.
    let st = ok(cmd(&f, &["status", "1240440"]).output().unwrap());
    assert!(
        st.contains("CachyPlay:        on") && st.contains("NOT YET TESTED"),
        "{st}"
    );
    let rep = ok(cmd(&f, &["report", "1240440"]).output().unwrap());
    assert!(rep.contains("cachyplay-bug-1240440-"), "{rep}");
    let desktop: Vec<_> = fs::read_dir(f.home().join("Desktop"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(desktop.len(), 1);

    // Restore-all dry run changes nothing; real restore is byte-identical to the original.
    let before = fs::read_to_string(f.localconfig()).unwrap();
    ok(cmd(&f, &["restore-all", "--dry-run"]).output().unwrap());
    assert_eq!(fs::read_to_string(f.localconfig()).unwrap(), before);
    ok(cmd(&f, &["restore-all"]).output().unwrap());
    assert_eq!(fs::read_to_string(f.localconfig()).unwrap(), original);
}

#[test]
fn install_then_uninstall() {
    let f = FakeSteam::new();
    let dl = f.home().join("Downloads/cachyplay");
    fs::create_dir_all(dl.parent().unwrap()).unwrap();
    fs::copy(BIN, &dl).unwrap();
    let original = fs::read_to_string(f.localconfig()).unwrap();

    let out = ok(Command::new(&dl)
        .args(["install"])
        .env_clear()
        .env("HOME", f.home())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap());
    assert!(out.contains(".local/bin/cachyplay"), "{out}");
    let installed = f.home().join(".local/bin/cachyplay");
    assert!(installed.exists());

    // The installed copy is accepted without CACHYPLAY_DEV and writes its own path into Steam.
    let run = |args: &[&str]| {
        Command::new(&installed)
            .args(args)
            .env_clear()
            .env("HOME", f.home())
            .env("PATH", "/usr/bin:/bin")
            .env("CACHYPLAY_PROC_ROOT", f.home().join("proc"))
            .output()
            .unwrap()
    };
    ok(run(&["apply", "1240440"]));
    assert!(launch_options(&f).contains(&format!("{} %command%", installed.display())));

    ok(run(&["uninstall", "--yes"]));
    assert_eq!(
        fs::read_to_string(f.localconfig()).unwrap(),
        original,
        "games restored on uninstall"
    );
    assert!(!installed.exists(), "binary removed");
}

#[test]
fn uninstall_works_without_steam_or_account() {
    // Nothing was ever turned on, and Steam isn't even there: uninstall must still succeed.
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(BIN)
        .args(["uninstall", "--yes"])
        .env_clear()
        .env("HOME", dir.path())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn general_report_names_the_crashing_games() {
    // Matt's real kernel log (identifiers replaced) plus Steam's game start/stop lines.
    let f = FakeSteam::new();
    let root = f.home().join(".local/share/Steam");
    fs::write(
        root.join("steamapps/appmanifest_5335510.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"5335510\"\n\t\"name\"\t\t\"Moon Sweepers\"\n\t\"installdir\"\t\t\"Moon Sweepers\"\n\t\"buildid\"\t\t\"20999001\"\n\t\"LastUpdated\"\t\t\"1763700000\"\n}\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("steamapps/common/Moon Sweepers")).unwrap();
    // Unity log inside Moon Sweepers' Proton prefix.
    let unity = root.join("steamapps/compatdata/5335510/pfx/drive_c/users/steamuser/AppData/LocalLow/DragDrop Games/Moon Sweepers");
    fs::create_dir_all(&unity).unwrap();
    fs::write(unity.join("Player.log"), "Initialize engine version\n[graphics] vkQueuePresentKHR failed with error 'VK_SUBOPTIMAL_KHR'\nCrash!!!\n").unwrap();
    fs::create_dir_all(root.join("logs")).unwrap();
    fs::write(
        root.join("logs/console_log.txt"),
        "[2026-11-23 18:33:59] Game process added : AppID 5335510 \"x\", ProcID 235300, IP 0.0.0.0:0\n\
         [2026-11-23 18:36:00] Game process removed: AppID 5335510 \"x\", ProcID 235300\n\
         [2026-11-23 18:37:15] Game process added : AppID 1240440 \"x\", ProcID 236800, IP 0.0.0.0:0\n",
    )
    .unwrap();
    let log = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../cachyplay-core/tests/fixtures/kernel-warnings-short.log"
    );

    let out = ok(cmd(&f, &["report"])
        .env("CACHYPLAY_KERNEL_LOG_FILE", log)
        .output()
        .unwrap());
    // Halo crashed most recently, so the general report is filed under Halo automatically.
    assert!(out.contains("cachyplay-bug-1240440-"), "{out}");
    let zip_path = fs::read_dir(f.home().join("Desktop"))
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();
    let mut z = zip::ZipArchive::new(fs::File::open(&zip_path).unwrap()).unwrap();
    let mut read = |suffix: &str| {
        let name = (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().to_string())
            .find(|n| n.ends_with(suffix))
            .unwrap_or_else(|| panic!("no {suffix}"));
        let mut s = String::new();
        std::io::Read::read_to_string(&mut z.by_name(&name).unwrap(), &mut s).unwrap();
        s
    };
    let summary = read("00-summary.md");
    assert!(
        summary.contains("**Game: Halo Infinite (AppID 1240440)** — picked automatically"),
        "{summary}"
    );
    assert!(summary.contains("18:40:28: Halo Infinite"), "{summary}");
    assert!(summary.contains("18:34:54: Moon Sweepers"), "{summary}");
    assert!(
        summary.contains("(+38s)"),
        "Xid 31 follows Xid 109 by 38 s: {summary}"
    );
    assert!(
        summary.contains("Several different games hit GPU driver errors"),
        "{summary}"
    );
    let halo_pos = summary.find("18:40:28").unwrap();
    let moon_pos = summary.find("18:34:54").unwrap();
    assert!(halo_pos < moon_pos, "newest first");
    let kernel = read("logs/kernel-warnings.log");
    assert!(
        !kernel.contains("UFW") && !kernel.contains("02:00:5e") && !kernel.contains("GPU-00000000"),
        "{kernel}"
    );
    assert!(read("logs/steam-game-sessions.log").contains("AppID 5335510"));

    // A report for Moon Sweepers itself includes its Unity log and the update date.
    ok(cmd(&f, &["report", "5335510"])
        .env("CACHYPLAY_KERNEL_LOG_FILE", log)
        .output()
        .unwrap());
    let moon_zip = fs::read_dir(f.home().join("Desktop"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().contains("5335510"))
        .unwrap();
    let mut z = zip::ZipArchive::new(fs::File::open(&moon_zip).unwrap()).unwrap();
    let names: Vec<String> = (0..z.len())
        .map(|i| z.by_index(i).unwrap().name().to_string())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with("logs/unity-Player.log")),
        "{names:?}"
    );
    let sname = names
        .iter()
        .find(|n| n.ends_with("00-summary.md"))
        .unwrap()
        .clone();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut z.by_name(&sname).unwrap(), &mut s).unwrap();
    assert!(s.contains("Game last updated by Steam"), "{s}");
    assert!(s.contains("Runs with: Proton"), "{s}");
    assert!(
        s.contains("unity_crash") && s.contains("vulkan_present"),
        "{s}"
    );
}

#[test]
fn suggests_and_applies_the_vulkan_addon_fix() {
    let f = FakeSteam::new();
    let root = f.home().join(".local/share/Steam");
    fs::write(
        root.join("steamapps/appmanifest_5335510.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"5335510\"\n\t\"name\"\t\t\"Moon Sweepers\"\n\t\"installdir\"\t\t\"Moon Sweepers\"\n\t\"buildid\"\t\t\"1\"\n}\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("steamapps/common/Moon Sweepers")).unwrap();
    fs::create_dir_all(root.join("steamapps/compatdata/5335510/pfx")).unwrap();
    fs::create_dir_all(root.join("logs")).unwrap();
    fs::write(
        root.join("logs/console_log.txt"),
        "[2026-11-23 18:33:59] Game process added : AppID 5335510 \"x\", ProcID 1\n\
         [2026-11-23 18:36:00] Game process removed: AppID 5335510 \"x\", ProcID 1\n\
         [2026-11-23 18:37:15] Game process added : AppID 1240440 \"x\", ProcID 2\n",
    )
    .unwrap();
    // An always-on third-party Vulkan add-on, like AFMF on Matt's PC.
    let layers = f.home().join(".local/share/vulkan/implicit_layer.d");
    fs::create_dir_all(&layers).unwrap();
    fs::write(
        layers.join("zz_test_framegen.json"),
        r#"{"file_format_version":"1.0.0","layer":{"name":"VK_LAYER_ZZTEST_framegen","description":"Test frame generation layer"}}"#,
    )
    .unwrap();
    let log = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../cachyplay-core/tests/fixtures/kernel-warnings-short.log"
    );
    let run = |args: &[&str]| {
        cmd(&f, args)
            .env("CACHYPLAY_KERNEL_LOG_FILE", log)
            .env("CACHYPLAY_DEV", "1")
            .output()
            .unwrap()
    };

    let out = ok(run(&["suggest", "5335510"]));
    assert!(
        out.contains("Turn off the extra Vulkan add-ons for this game"),
        "{out}"
    );
    assert!(
        out.contains("Halo Infinite") && out.contains("Moon Sweepers"),
        "{out}"
    );
    assert!(
        out.contains("cachyplay fix 5335510 disable_vulkan_layers"),
        "{out}"
    );

    // One click: CachyPlay is turned on for the game and the fix is stored.
    let out = ok(run(&["fix", "5335510", "disable_vulkan_layers"]));
    assert!(
        out.contains("Turned CachyPlay on for Moon Sweepers"),
        "{out}"
    );

    // A real launch through Steam's string now hides the add-on from the game.
    let text = fs::read_to_string(f.localconfig()).unwrap();
    let doc = cachyplay_core::vdf::Doc::parse(&text).unwrap();
    let mut p = cachyplay_core::steam::APPS_PATH.to_vec();
    p.extend(["5335510", "LaunchOptions"]);
    let lo = doc.get_str(&p).unwrap().to_string();
    let game = f.home().join("probe-game");
    fs::write(
        &game,
        "#!/bin/sh\necho \"layers:${VK_LOADER_LAYERS_DISABLE:-none}\" > \"$OUT\"\n",
    )
    .unwrap();
    fs::set_permissions(&game, fs::Permissions::from_mode(0o755)).unwrap();
    let out_file = f.home().join("probe-out");
    let status = Command::new("sh")
        .arg("-c")
        .arg(lo.replace("%command%", &game.display().to_string()))
        .env_clear()
        .env("HOME", f.home())
        .env("PATH", "/usr/bin:/bin")
        .env("OUT", &out_file)
        .env("SteamAppId", "5335510")
        .env("CACHYPLAY_NO_NOTIFY", "1")
        .status()
        .unwrap();
    assert!(status.success());
    let seen = fs::read_to_string(&out_file).unwrap();
    assert!(seen.contains("VK_LAYER_ZZTEST_framegen"), "{seen}");

    // The suggestion now shows as applied; undo removes it.
    assert!(ok(run(&["suggest", "5335510"])).contains("[applied"));
    assert!(ok(run(&["fix", "5335510", "disable_vulkan_layers", "--undo"])).contains("Undid"));
}

#[test]
fn uninstall_can_keep_the_settings_that_made_a_game_work() {
    let f = FakeSteam::new();
    let root = f.home().join(".local/share/Steam");
    fs::write(
        root.join("steamapps/appmanifest_5335510.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"5335510\"\n\t\"name\"\t\t\"Moon Sweepers\"\n\t\"installdir\"\t\t\"Moon Sweepers\"\n\t\"buildid\"\t\t\"1\"\n}\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("steamapps/common/Moon Sweepers")).unwrap();
    fs::create_dir_all(root.join("steamapps/compatdata/5335510/pfx")).unwrap();
    fs::create_dir_all(root.join("logs")).unwrap();
    fs::write(
        root.join("logs/console_log.txt"),
        "[2026-11-23 18:33:59] Game process added : AppID 5335510 \"x\", ProcID 1\n\
         [2026-11-23 18:36:00] Game process removed: AppID 5335510 \"x\", ProcID 1\n\
         [2026-11-23 18:37:15] Game process added : AppID 1240440 \"x\", ProcID 2\n",
    )
    .unwrap();
    let layers = f.home().join(".local/share/vulkan/implicit_layer.d");
    fs::create_dir_all(&layers).unwrap();
    fs::write(
        layers.join("zz.json"),
        r#"{"layer":{"name":"VK_LAYER_ZZTEST_framegen","description":"Test frame generation"}}"#,
    )
    .unwrap();
    let log = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../cachyplay-core/tests/fixtures/kernel-warnings-short.log"
    );
    let run = |args: &[&str]| {
        cmd(&f, args)
            .env("CACHYPLAY_KERNEL_LOG_FILE", log)
            .env("CACHYPLAY_DEV", "1")
            .output()
            .unwrap()
    };

    ok(run(&["fix", "5335510", "disable_vulkan_layers"]));
    let shown = ok(run(&["settings", "--export"]));
    assert!(
        shown.contains("Moon Sweepers") && shown.contains("VK_LOADER_LAYERS_DISABLE="),
        "{shown}"
    );
    assert!(f
        .home()
        .join("Desktop/CachyPlay game settings.txt")
        .exists());

    let out = ok(run(&["uninstall", "--yes", "--keep-settings"]));
    assert!(out.contains("kept its settings in Steam"), "{out}");

    // Steam now has plain launch options: no CachyPlay, but the fix is still there.
    let text = fs::read_to_string(f.localconfig()).unwrap();
    let doc = cachyplay_core::vdf::Doc::parse(&text).unwrap();
    let mut p = cachyplay_core::steam::APPS_PATH.to_vec();
    p.extend(["5335510", "LaunchOptions"]);
    let lo = doc.get_str(&p).unwrap().to_string();
    assert!(!lo.contains("cachyplay"), "{lo}");
    assert!(
        lo.contains("VK_LOADER_LAYERS_DISABLE=") && lo.contains("VK_LAYER_ZZTEST_framegen"),
        "{lo}"
    );

    let game = f.home().join("probe-game");
    fs::write(
        &game,
        "#!/bin/sh\necho \"layers:${VK_LOADER_LAYERS_DISABLE:-none}\" > \"$OUT\"\n",
    )
    .unwrap();
    fs::set_permissions(&game, fs::Permissions::from_mode(0o755)).unwrap();
    let out_file = f.home().join("probe-out");
    let status = Command::new("sh")
        .arg("-c")
        .arg(lo.replace("%command%", &game.display().to_string()))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("OUT", &out_file)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(fs::read_to_string(&out_file)
        .unwrap()
        .contains("VK_LAYER_ZZTEST_framegen"));
    // The copy on the Desktop survives the uninstall.
    assert!(f
        .home()
        .join("Desktop/CachyPlay game settings.txt")
        .exists());
}

#[test]
fn obs_capture_starts_the_game_through_obs_gamecapture() {
    let f = FakeSteam::new();
    ok(cmd(&f, &["apply", "1240440"])
        .env("CACHYPLAY_DEV", "1")
        .output()
        .unwrap());
    let lo = launch_options(&f);
    let game = fake_game(f.home());

    // A PATH with only `sh`, so the real obs-gamecapture (if this PC has it) can't be found.
    let tools = f.home().join("tools");
    fs::create_dir_all(&tools).unwrap();
    std::os::unix::fs::symlink("/bin/sh", tools.join("sh")).unwrap();
    let bare = tools.display().to_string();

    // Turned on, but obs-gamecapture isn't installed: the game still starts normally.
    let o = ok(cmd(&f, &["obs", "1240440", "on"])
        .env("CACHYPLAY_DEV", "1")
        .env("PATH", &bare)
        .output()
        .unwrap());
    assert!(o.contains("isn't installed"), "{o}");
    let (code, out) = steam_launch(&f, &lo, &game, &[("PATH", &bare)]);
    assert_eq!(code, 0);
    assert!(out.contains("args:-nosplash"), "{out}");

    // A stand-in obs-gamecapture that leaves a mark and runs the game, like the real one.
    let bin = f.home().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let obs = bin.join("obs-gamecapture");
    fs::write(
        &obs,
        "#!/bin/sh\necho captured > \"$OUT.obs\"\nexport OBS_VKCAPTURE=1\nexec \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&obs, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{bare}", bin.display());
    let mark = f.home().join("game-output.obs");
    let (code, out) = steam_launch(&f, &lo, &game, &[("PATH", &path)]);
    assert_eq!(code, 0);
    assert!(out.contains("args:-nosplash"), "{out}");
    assert!(
        mark.exists(),
        "the game was started through obs-gamecapture"
    );

    // Kept when uninstalling with "keep settings".
    let s = ok(cmd(&f, &["settings"]).env("PATH", &path).output().unwrap());
    assert!(s.contains("obs-gamecapture %command% -nosplash"), "{s}");

    // Turned off: no longer wrapped.
    fs::remove_file(&mark).unwrap();
    ok(cmd(&f, &["obs", "1240440", "off"]).output().unwrap());
    steam_launch(&f, &lo, &game, &[("PATH", &path)]);
    assert!(!mark.exists());
}
