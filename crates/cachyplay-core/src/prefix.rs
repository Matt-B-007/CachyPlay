// SPDX-License-Identifier: GPL-3.0-or-later
//! Resetting a game's Windows files (its Proton prefix, `steamapps/compatdata/<AppID>`): a common
//! fix that's scary by hand. Nothing is deleted: the old folder is renamed next to the original
//! (instant, same drive), Steam builds a fresh one on the next start, and Undo puts the old one
//! back. Saves that live in the prefix stay in the kept folder.

use crate::manage::Ctx;
use crate::steam::Game;
use anyhow::{bail, Result};
use std::fs;
use std::path::{Path, PathBuf};

const KEPT: &str = ".cachyplay-kept-";
const FRESH: &str = ".cachyplay-fresh-";

/// Folders kept by earlier resets, newest first.
pub fn kept(game: &Game) -> Vec<PathBuf> {
    let dir = game.library.join("steamapps/compatdata");
    let prefix = format!("{}{KEPT}", game.app_id);
    let mut v: Vec<PathBuf> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .map(|e| e.path())
        .collect();
    v.sort();
    v.reverse();
    v
}

/// Every copy CachyPlay set aside, in any Steam library: (folder, AppID, size in bytes).
pub fn all_kept(libraries: &[PathBuf]) -> Vec<(PathBuf, String, u64)> {
    let mut out = Vec::new();
    for lib in libraries {
        for e in fs::read_dir(lib.join("steamapps/compatdata"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name
                .split_once(KEPT)
                .or_else(|| name.split_once(FRESH))
                .map(|(id, _)| id.to_string());
            if let Some(id) = id {
                out.push((e.path(), id, dir_size(&e.path())));
            }
        }
    }
    out.sort();
    out
}

/// Size of a folder's files. Doesn't follow links (prefixes link to system folders).
pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let Ok(m) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            if m.is_dir() {
                todo.push(e.path());
            } else if m.is_file() {
                total += m.len();
            }
        }
    }
    total
}

/// "3.2 GB"
pub fn human_size(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{:.0} MB", (bytes as f64 / 1e6).max(1.0))
    }
}

/// Delete the copies of a game's Windows files that resets set aside (after Undo is no longer
/// wanted). Returns the space freed. The game's current Windows files are never touched.
pub fn delete_kept(ctx: &Ctx, game: &Game) -> Result<u64> {
    guard(ctx)?;
    let mut freed = 0;
    for (path, id, size) in all_kept(std::slice::from_ref(&game.library)) {
        if id != game.app_id {
            continue;
        }
        fs::remove_dir_all(&path)?;
        freed += size;
    }
    Ok(freed)
}

fn guard(ctx: &Ctx) -> Result<()> {
    if crate::steam::is_running_in(&ctx.proc_root) {
        bail!("Close Steam first, so the game isn't running while its files are moved.");
    }
    Ok(())
}

/// Move the game's prefix aside. Returns where it went.
pub fn reset(ctx: &Ctx, game: &Game) -> Result<PathBuf> {
    guard(ctx)?;
    let Some(current) = ctx.steam.compatdata(game) else {
        bail!(
            "{} has no Windows files yet (it hasn't been started with Proton).",
            game.name
        );
    };
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    let target = current.with_file_name(format!("{}{KEPT}{stamp}", game.app_id));
    fs::rename(&current, &target)?;
    Ok(target)
}

/// Put the newest kept prefix back. The fresh one Steam made meanwhile is kept too, renamed.
pub fn undo(ctx: &Ctx, game: &Game) -> Result<()> {
    guard(ctx)?;
    let Some(old) = kept(game).into_iter().next() else {
        bail!(
            "There's no earlier copy of {}'s Windows files to put back.",
            game.name
        );
    };
    let current = game.library.join("steamapps/compatdata").join(&game.app_id);
    if current.exists() {
        let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        fs::rename(
            &current,
            current.with_file_name(format!("{}{FRESH}{stamp}", game.app_id)),
        )?;
    }
    fs::rename(&old, &current)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_and_sizes_kept_copies_without_following_links() {
        let t = tempfile::tempdir().unwrap();
        let cd = t.path().join("steamapps/compatdata");
        let kept = cd.join("730.cachyplay-kept-2026-11-20_10-00-00/pfx/drive_c");
        fs::create_dir_all(&kept).unwrap();
        fs::write(kept.join("save.dat"), vec![0u8; 5000]).unwrap();
        // Prefixes link to big system folders; those don't count.
        std::os::unix::fs::symlink("/usr", kept.join("usr-link")).unwrap();
        fs::create_dir_all(cd.join("730")).unwrap();
        fs::create_dir_all(cd.join("570.cachyplay-fresh-2026-11-21_10-00-00")).unwrap();
        let k = all_kept(&[t.path().to_path_buf()]);
        assert_eq!(k.len(), 2);
        let csgo = k.iter().find(|x| x.1 == "730").unwrap();
        assert_eq!(csgo.2, 5000);
        assert_eq!(human_size(3_200_000_000), "3.2 GB");
        assert_eq!(human_size(5000), "1 MB");
    }
}
