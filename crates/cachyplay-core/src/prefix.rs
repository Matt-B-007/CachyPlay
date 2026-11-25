//! Resetting a game's Windows files (its Proton prefix, `steamapps/compatdata/<AppID>`): a common
//! fix that's scary by hand. Nothing is deleted: the old folder is renamed next to the original
//! (instant, same drive), Steam builds a fresh one on the next start, and Undo puts the old one
//! back. Saves that live in the prefix stay in the kept folder.

use crate::manage::Ctx;
use crate::steam::Game;
use anyhow::{bail, Result};
use std::fs;
use std::path::PathBuf;

const KEPT: &str = ".cachyplay-kept-";

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
            current.with_file_name(format!("{}.cachyplay-fresh-{stamp}", game.app_id)),
        )?;
    }
    fs::rename(&old, &current)?;
    Ok(())
}
