// SPDX-License-Identifier: GPL-3.0-or-later
//! "Start the game's shader cache fresh": move Steam's shader cache for one game aside (Steam and
//! the driver rebuild it), with Undo. Only the newest set-aside copy is kept.

use crate::manage::Ctx;
use crate::steam::Game;
use anyhow::{bail, Result};
use std::fs;
use std::path::PathBuf;

const ASIDE: &str = ".cachyplay-old";

fn cache_dir(game: &Game) -> PathBuf {
    game.library
        .join("steamapps/shadercache")
        .join(&game.app_id)
}

fn aside_dir(game: &Game) -> PathBuf {
    game.library
        .join("steamapps/shadercache")
        .join(format!("{}{ASIDE}", game.app_id))
}

pub fn has_cache(game: &Game) -> bool {
    cache_dir(game).is_dir()
}

pub fn has_aside(game: &Game) -> bool {
    aside_dir(game).is_dir()
}

fn game_running(ctx: &Ctx, game: &Game) -> bool {
    crate::crashmatch::parse_steam_sessions(&ctx.steam.console_logs())
        .iter()
        .any(|s| {
            s.app_id == game.app_id
                && s.end.is_none()
                && chrono::Local::now().naive_local() - s.start < chrono::Duration::hours(12)
        })
}

/// Move the cache aside. Works with Steam open, as long as the game isn't running.
pub fn reset(ctx: &Ctx, game: &Game) -> Result<()> {
    if game_running(ctx, game) {
        bail!("{} is running. Close it first.", game.name);
    }
    let from = cache_dir(game);
    if !from.is_dir() {
        bail!("{} has no shader cache yet; nothing to reset.", game.name);
    }
    let to = aside_dir(game);
    if to.exists() {
        fs::remove_dir_all(&to)?;
    }
    fs::rename(&from, &to)?;
    Ok(())
}

/// Put the set-aside cache back (the rebuilt one is removed).
pub fn undo(ctx: &Ctx, game: &Game) -> Result<()> {
    if game_running(ctx, game) {
        bail!("{} is running. Close it first.", game.name);
    }
    let aside = aside_dir(game);
    if !aside.is_dir() {
        bail!("There's no set-aside shader cache to put back.");
    }
    let cur = cache_dir(game);
    if cur.exists() {
        fs::remove_dir_all(&cur)?;
    }
    fs::rename(&aside, &cur)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::FakeSteam;

    #[test]
    fn moves_the_cache_aside_and_back() {
        let f = FakeSteam::new();
        let ctx = f.ctx();
        let g = ctx.steam.find_game("1240440").unwrap();
        assert!(reset(&ctx, &g).is_err(), "no cache yet");
        fs::create_dir_all(cache_dir(&g).join("nvidiav1")).unwrap();
        fs::write(cache_dir(&g).join("nvidiav1/x.bin"), "old").unwrap();
        reset(&ctx, &g).unwrap();
        assert!(!has_cache(&g) && has_aside(&g));
        // Steam rebuilds it...
        fs::create_dir_all(cache_dir(&g)).unwrap();
        undo(&ctx, &g).unwrap();
        assert_eq!(
            fs::read_to_string(cache_dir(&g).join("nvidiav1/x.bin")).unwrap(),
            "old"
        );
        assert!(!has_aside(&g));
    }
}
