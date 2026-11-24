//! Crash-safe file writes and size-capped log reading.

use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Write `bytes` to `path` atomically: temp file in the same directory, fsync, rename.
/// A crash at any point leaves either the old file or the new one, never a half-written file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("path has no parent directory")?;
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .context("path has no file name")?
        .to_string_lossy();
    let tmp = dir.join(format!(".{name}.cachyplay-tmp-{}", std::process::id()));
    let result = (|| -> Result<()> {
        let mut f = File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
        if let Ok(meta) = fs::metadata(path) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Copy `src` into `dir` with a timestamp suffix, keeping only the newest `keep` copies.
pub fn backup(src: &Path, dir: &Path, keep: usize) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let name = src
        .file_name()
        .context("no file name")?
        .to_string_lossy()
        .into_owned();
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S%.3f");
    let dest = dir.join(format!("{name}.{stamp}.bak"));
    fs::copy(src, &dest).with_context(|| format!("backing up {}", src.display()))?;
    let mut olds: Vec<_> = fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| {
                    n.to_string_lossy().starts_with(&format!("{name}."))
                        && n.to_string_lossy().ends_with(".bak")
                })
                .unwrap_or(false)
        })
        .collect();
    olds.sort();
    while olds.len() > keep {
        let _ = fs::remove_file(olds.remove(0));
    }
    Ok(dest)
}

/// Read a log, keeping the first `head` and last `tail` lines with a marker in between.
pub fn read_capped(path: &Path, head: usize, tail: usize) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= head + tail {
        return Some(text.into_owned());
    }
    let cut = lines.len() - head - tail;
    let mut out = lines[..head].join("\n");
    out.push_str(&format!("\n--- [TRUNCATED {cut} LINES] ---\n"));
    out.push_str(&lines[lines.len() - tail..].join("\n"));
    out.push('\n');
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.vdf");
        fs::write(&p, "old").unwrap();
        atomic_write(&p, b"new").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "new");
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_write_keeps_original() {
        let d = tempfile::tempdir().unwrap();
        // Target is a directory, so the final rename fails.
        let p = d.path().join("target");
        fs::create_dir(&p).unwrap();
        fs::write(p.join("keep"), "x").unwrap();
        assert!(atomic_write(&p, b"new").is_err());
        assert!(p.join("keep").exists());
        assert_eq!(
            fs::read_dir(d.path()).unwrap().count(),
            1,
            "temp file cleaned up"
        );
    }

    #[test]
    fn backups_are_rotated() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("localconfig.vdf");
        fs::write(&src, "x").unwrap();
        let bk = d.path().join("bk");
        for _ in 0..7 {
            backup(&src, &bk, 5).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(fs::read_dir(&bk).unwrap().count(), 5);
    }

    #[test]
    fn caps_long_logs() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("log");
        let body: String = (0..100).map(|i| format!("line{i}\n")).collect();
        fs::write(&p, body).unwrap();
        let out = read_capped(&p, 3, 4).unwrap();
        assert!(out.starts_with("line0\nline1\nline2\n--- [TRUNCATED 93 LINES] ---\nline96"));
        assert!(out.ends_with("line99\n"));
    }
}
