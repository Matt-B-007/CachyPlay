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
