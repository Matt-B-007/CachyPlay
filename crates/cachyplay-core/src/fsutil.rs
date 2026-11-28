// SPDX-License-Identifier: GPL-3.0-or-later
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
    let stamp = crate::clock::stamp_ms();
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

/// Lines worth keeping from the middle of a long Proton log: errors and warnings, graphics
/// (DXVK, VKD3D, Vulkan), and which of the game's own files and graphics libraries loaded.
pub static IMPORTANT_LOG_LINE: &[&str] = &[
    "err:",
    "warn:  ",
    "info:  ",
    "DXVK",
    "VKD3D",
    "vkd3d",
    "Vulkan",
    "VK_ERROR",
    "DEVICE_LOST",
    "steamapps",
    "d3d11",
    "d3d12",
    "dxgi",
    "nvapi",
    "nvngx",
    "Unhandled exception",
    "page fault",
];

/// Like `read_capped`, but also keeps (up to 600) lines from the cut middle that contain any of
/// `keep`, so a long log still shows its errors and graphics setup.
pub fn read_capped_keep(path: &Path, head: usize, tail: usize, keep: &[&str]) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= head + tail {
        return Some(text.into_owned());
    }
    let middle = &lines[head..lines.len() - tail];
    let kept: Vec<&str> = middle
        .iter()
        .copied()
        .filter(|l| keep.iter().any(|k| l.contains(k)))
        .take(600)
        .collect();
    let mut out = lines[..head].join("\n");
    out.push_str(&format!(
        "\n--- [{} LINES CUT; KEPT {} IMPORTANT ONES BELOW] ---\n",
        middle.len() - kept.len(),
        kept.len()
    ));
    if !kept.is_empty() {
        out.push_str(&kept.join("\n"));
        out.push_str("\n--- [END OF KEPT LINES] ---\n");
    }
    out.push_str(&lines[lines.len() - tail..].join("\n"));
    out.push('\n');
    Some(out)
}

/// Write a new version of one of Steam's VDF files and check it: the file must read back exactly
/// as written, still parse, and differ from the old version only in the values under `allowed`
/// (lower-cased key paths, "a/b/c"). Otherwise the old file is put back and the write fails, so
/// a bug can never damage the rest of Steam's settings.
pub fn verified_vdf_write(
    path: &Path,
    old: &str,
    new: &str,
    allowed: &[String],
) -> anyhow::Result<()> {
    use anyhow::{bail, Context};
    let before = crate::vdf::Doc::parse(old)?.flatten();
    let after = crate::vdf::Doc::parse(new)
        .context("the new version of the file doesn't parse; nothing was written")?
        .flatten();
    let touched = |k: &str| {
        allowed
            .iter()
            .any(|a| k == a || k.starts_with(&format!("{a}/")) || a.starts_with(k))
    };
    let changed: Vec<&String> = before
        .keys()
        .chain(after.keys())
        .filter(|k| before.get(*k) != after.get(*k) && !touched(k))
        .collect();
    if let Some(k) = changed.first() {
        bail!(
            "refusing to write {}: the change would also touch \"{k}\"",
            path.display()
        );
    }
    atomic_write(path, new.as_bytes())?;
    let back = std::fs::read_to_string(path).unwrap_or_default();
    if back != new {
        let _ = atomic_write(path, old.as_bytes());
        bail!(
            "{} didn't read back as written, so the original was restored",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn verified_vdf_write_only_allows_the_named_value() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("localconfig.vdf");
        let old = "\"Root\"\n{\n\t\"apps\"\n\t{\n\t\t\"10\"\n\t\t{\n\t\t\t\"LaunchOptions\"\t\t\"a\"\n\t\t}\n\t}\n\t\"Other\"\t\t\"keep\"\n}\n";
        std::fs::write(&f, old).unwrap();
        let allowed = vec!["root/apps/10/launchoptions".to_string()];
        let good = old.replace("\"a\"", "\"b\"");
        super::verified_vdf_write(&f, old, &good, &allowed).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), good);
        // Touching anything else is refused and the file is left alone.
        let bad = good.replace("keep", "changed");
        let e = super::verified_vdf_write(&f, &good, &bad, &allowed).unwrap_err();
        assert!(e.to_string().contains("root/other"), "{e}");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), good);
        // A version that doesn't parse is never written.
        assert!(super::verified_vdf_write(&f, &good, "\"Root\" {", &allowed).is_err());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), good);
    }

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
        let mut text: String = (0..100).map(|i| format!("line{i}\n")).collect();
        text = text.replace("line50\n", "err: device lost\n");
        fs::write(&p, &text).unwrap();
        let kept = read_capped_keep(&p, 3, 4, &["err:"]).unwrap();
        assert!(
            kept.contains("KEPT 1 IMPORTANT") && kept.contains("err: device lost"),
            "{kept}"
        );
        assert!(out.ends_with("line99\n"));
    }
}
