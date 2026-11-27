// SPDX-License-Identifier: GPL-3.0-or-later
//! Small helpers for running system tools safely.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Find an executable on PATH.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| is_executable(p))
}

pub fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Run a command; returns (succeeded, stdout, stderr). Killed after `timeout`.
/// `None` if the program is missing or timed out.
pub fn run(program: &str, args: &[&str], timeout: Duration) -> Option<(bool, String, String)> {
    let exe = which(program)?;
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    // Read pipes on threads so a chatty program can't fill a pipe and block forever.
    let mut out = child.stdout.take()?;
    let mut err = child.stderr.take()?;
    let t_out = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut s);
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut s);
        s
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = String::from_utf8_lossy(&t_out.join().ok()?).into_owned();
    let err = String::from_utf8_lossy(&t_err.join().ok()?).into_owned();
    Some((status.success(), out, err))
}

/// Run a command and capture stdout+stderr, killing it after `timeout`.
/// Returns `None` if the tool is missing or timed out.
pub fn capture(program: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let exe = which(program)?;
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let out = child.wait_with_output().ok()?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(s)
}

/// Whether the desktop uses a dark or light theme: "dark", "light", or None if it doesn't say.
/// Reads KDE's colors first (the window background's brightness, then the scheme's name), then
/// GNOME's color-scheme setting.
pub fn desktop_theme(home: &std::path::Path) -> Option<&'static str> {
    if let Ok(text) = std::fs::read_to_string(home.join(".config/kdeglobals")) {
        if let Some(t) = kde_theme(&text) {
            return Some(t);
        }
    }
    let out = capture(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "color-scheme"],
        std::time::Duration::from_secs(2),
    )?;
    match out.trim().trim_matches('\'') {
        "prefer-dark" => Some("dark"),
        "prefer-light" => Some("light"),
        _ => None,
    }
}

pub fn kde_theme(kdeglobals: &str) -> Option<&'static str> {
    let mut section = "";
    for line in kdeglobals.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line;
            continue;
        }
        if section == "[Colors:Window]" {
            if let Some(v) = line.strip_prefix("BackgroundNormal=") {
                let rgb: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if rgb.len() >= 3 {
                    let lum = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
                    return Some(if lum < 128.0 { "dark" } else { "light" });
                }
            }
        }
    }
    let scheme = kdeglobals
        .lines()
        .find_map(|l| l.trim().strip_prefix("ColorScheme="))?
        .to_lowercase();
    Some(if scheme.contains("dark") {
        "dark"
    } else {
        "light"
    })
}

#[cfg(test)]
mod theme_tests {
    use super::kde_theme;

    #[test]
    fn reads_kde_theme() {
        assert_eq!(
            kde_theme("[Colors:Window]\nBackgroundNormal=32,35,38\n"),
            Some("dark")
        );
        assert_eq!(kde_theme("[Colors:View]\nBackgroundNormal=255,255,255\n[Colors:Window]\nBackgroundNormal=239,240,241\n"), Some("light"));
        assert_eq!(
            kde_theme("[General]\nColorScheme=BreezeDark\n"),
            Some("dark")
        );
        assert_eq!(kde_theme("[General]\nAccentColor=61,174,233\n"), None);
    }
}
