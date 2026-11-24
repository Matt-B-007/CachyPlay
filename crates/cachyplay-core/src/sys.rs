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
