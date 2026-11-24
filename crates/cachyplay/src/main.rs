//! `cachyplay` command-line interface. All logic lives in `cachyplay-core`.

use anyhow::{bail, Context, Result};
use cachyplay_core::manage::{touched_accounts, Ctx, Outcome};
use cachyplay_core::paths::{self, Paths};
use cachyplay_core::{install, launch_options, recipes, report, steam, wrapper};
use clap::{Parser, Subcommand};
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "cachyplay",
    version,
    about = "One-click game tuning, crash help and bug reports for CachyOS"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List installed Steam games and whether CachyPlay manages them
    List,
    /// Show what CachyPlay changes for a game, and any tweaks you made yourself
    Status { app_id: String },
    /// Turn CachyPlay on for one or more games (Steam must be closed)
    Apply {
        #[arg(required = true)]
        app_ids: Vec<String>,
    },
    /// Turn CachyPlay off for one or more games and restore their launch options
    Undo {
        #[arg(required = true)]
        app_ids: Vec<String>,
    },
    /// Restore every game CachyPlay changed, for every Steam account
    RestoreAll {
        /// Only show what would be restored
        #[arg(long)]
        dry_run: bool,
    },
    /// Save a timestamped bug report to your Desktop
    Report { app_id: Option<String> },
    /// Check this PC for common gaming problems
    Doctor,
    /// Install this downloaded copy to ~/.local/bin (its own location)
    Install,
    /// Restore all games, then remove CachyPlay's files
    Uninstall {
        /// Also delete save backups and config backups
        #[arg(long)]
        remove_backups: bool,
        /// Don't ask for confirmation
        #[arg(long, short)]
        yes: bool,
    },
    /// Launch wrapper used by Steam: cachyplay run -- %command%
    #[command(hide = true)]
    Run {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<OsString>,
    },
}

fn main() -> ExitCode {
    // The wrapper path must not depend on argument parsing succeeding: handle it first, raw.
    let args: Vec<OsString> = std::env::args_os().collect();
    if args.get(1).is_some_and(|a| a == "run") {
        let rest: Vec<OsString> = args[2..]
            .iter()
            .skip_while(|a| *a == "--")
            .cloned()
            .collect();
        return run_wrapper(rest);
    }
    match real_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn run_wrapper(command: Vec<OsString>) -> ExitCode {
    let code = match Paths::from_env() {
        Ok(p) => wrapper::run(&p, command),
        // Can't even find HOME: still launch the game.
        Err(_) => match std::process::Command::new(&command[0])
            .args(&command[1..])
            .status()
        {
            Ok(s) => s.code().unwrap_or(1),
            Err(_) => 127,
        },
    };
    ExitCode::from(code.clamp(0, 255) as u8)
}

fn ctx(paths: &Paths) -> Result<Ctx> {
    let mut c = Ctx::new(paths.clone())?;
    if let Some(p) = std::env::var_os("CACHYPLAY_PROC_ROOT") {
        c.proc_root = PathBuf::from(p); // tests only
    }
    Ok(c)
}

fn real_main() -> Result<ExitCode> {
    let cli = Cli::parse();
    let paths = Paths::from_env()?;
    match cli.cmd {
        Cmd::List => {
            let c = ctx(&paths)?;
            println!(
                "Steam account: {} ({})",
                if c.account.persona.is_empty() {
                    "?"
                } else {
                    &c.account.persona
                },
                c.account.account_id
            );
            if c.steam.kind == steam::InstallKind::Flatpak {
                println!(
                    "Note: Flatpak Steam isn't supported yet in 0.1.0; nothing can be changed."
                );
            }
            println!("{:<10} {:<10} GAME", "APPID", "CACHYPLAY");
            for g in c.steam.games() {
                let st = c.status(&g).ok();
                let label = match &st {
                    Some(s) if s.applied => "on",
                    _ => "off",
                };
                let mut notes = Vec::new();
                if recipes::find(&g.app_id, &paths.config, &paths.recipes())
                    .ok()
                    .flatten()
                    .is_some()
                {
                    notes.push("fix available");
                }
                if st
                    .as_ref()
                    .is_some_and(|s| !s.user_options.is_empty() || !s.tweaks.is_empty())
                {
                    notes.push("your tweaks");
                }
                let notes = if notes.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", notes.join(", "))
                };
                println!("{:<10} {:<10} {}{}", g.app_id, label, g.name, notes);
            }
        }
        Cmd::Status { app_id } => {
            let c = ctx(&paths)?;
            let g = c
                .steam
                .find_game(&app_id)
                .with_context(|| format!("game {app_id} is not installed"))?;
            let s = c.status(&g)?;
            println!("{} ({})", g.name, g.app_id);
            println!(
                "  CachyPlay:        {}",
                if s.applied { "on" } else { "off" }
            );
            println!(
                "  Launch options:   {}",
                if s.launch_options.is_empty() {
                    "(none)"
                } else {
                    &s.launch_options
                }
            );
            println!(
                "  Your own options: {}",
                if s.user_options.is_empty() {
                    "(none)"
                } else {
                    &s.user_options
                }
            );
            if s.edited_since {
                println!("  Note: launch options were edited in Steam after CachyPlay changed them. Undo keeps your edits.");
            }
            for t in &s.tweaks {
                println!("  Your tweak:       {t}");
            }
            match recipes::find(&app_id, &paths.config, &paths.recipes())? {
                Some(r) => {
                    let p = recipes::plan(&r, &recipes::Facts::detect());
                    println!("\n  Fix file: {}{}", r.name, if r.tested { "" } else { "  (NOT YET TESTED)" });
                    for a in &p.applied {
                        println!("    applies: {a}");
                    }
                    for a in &p.skipped {
                        println!("    later:   {a}");
                    }
                }
                None => println!("\n  No fix file for this game yet. CachyPlay still records launches and helps with bug reports."),
            }
            for l in wrapper::recent_launches(&paths, &app_id, 3) {
                println!(
                    "  Launch {}: {:.0}s, exit {:?}{}",
                    l.started_at,
                    l.runtime_secs,
                    l.exit_code,
                    if l.failed { "  FAILED" } else { "" }
                );
            }
        }
        Cmd::Apply { app_ids } => {
            let bin = paths::installed_binary(&paths)?;
            let c = ctx(&paths)?;
            for id in app_ids {
                let g = c
                    .steam
                    .find_game(&id)
                    .with_context(|| format!("game {id} is not installed"))?;
                match c.apply(&g, &bin)? {
                    Outcome::AlreadyApplied => println!("{}: already on", g.name),
                    _ => println!(
                        "{}: CachyPlay is on. Your own launch options were kept.",
                        g.name
                    ),
                }
            }
        }
        Cmd::Undo { app_ids } => {
            let c = ctx(&paths)?;
            for id in app_ids {
                println!("{id}: {}", describe(c.undo(&id)?));
            }
        }
        Cmd::RestoreAll { dry_run } => {
            let n = restore_all(&paths, dry_run)?;
            if n == 0 {
                println!("Nothing to restore.");
            }
        }
        Cmd::Report { app_id } => {
            let c = ctx(&paths).ok();
            let g = match (&app_id, &c) {
                (Some(id), Some(c)) => Some(
                    c.steam
                        .find_game(id)
                        .with_context(|| format!("game {id} is not installed"))?,
                ),
                (Some(id), None) => bail!("can't look up game {id}: Steam not found"),
                _ => None,
            };
            let r = report::create(&paths, c.as_ref(), g.as_ref())?;
            println!("Bug report saved: {}", r.path.display());
            println!("Contains: {}", r.files.join(", "));
            println!("Personal details (home folder, username, Steam IDs, hostname, tokens) were removed.");
        }
        Cmd::Doctor => doctor(&paths)?,
        Cmd::Install => {
            let exe = std::env::current_exe()?;
            let bin = install::install_user(&paths, &exe)?;
            println!("Installed to {}", bin.display());
            if exe != bin {
                println!("You can now delete the downloaded file: {}", exe.display());
            }
            let on_path = std::env::var_os("PATH").is_some_and(|p| {
                std::env::split_paths(&p).any(|d| bin.parent() == Some(d.as_path()))
            });
            if !on_path {
                println!("Tip: add ~/.local/bin to your PATH to type `cachyplay` anywhere. (Steam doesn't need this: it uses the full path.)");
            }
        }
        Cmd::Uninstall {
            remove_backups,
            yes,
        } => {
            println!("This will:");
            println!("  1. Restore the launch options of every game CachyPlay changed (your own options are kept)");
            println!("  2. Remove CachyPlay's cache, logs, fix files and records");
            println!(
                "  3. {} save and config backups",
                if remove_backups { "DELETE" } else { "Keep" }
            );
            println!("  Bug reports on your Desktop are never deleted.");
            if !yes && !confirm("Continue?")? {
                println!("Cancelled. Nothing was changed.");
                return Ok(ExitCode::SUCCESS);
            }
            restore_all(&paths, false)?;
            let removed = install::remove_user_files(&paths, remove_backups)?;
            for r in removed {
                println!("Removed {}", r.display());
            }
            let exe = std::env::current_exe().unwrap_or_default();
            if exe.starts_with("/usr") {
                println!("\nAll games restored. Finish by removing the package:  sudo pacman -Rns cachyplay");
            } else {
                println!("\nCachyPlay has been removed.");
            }
        }
        Cmd::Run { .. } => unreachable!("handled in main"),
    }
    Ok(ExitCode::SUCCESS)
}

fn describe(o: Outcome) -> &'static str {
    match o {
        Outcome::Restored => "restored the original launch options",
        Outcome::RemovedKeptUserEdits => "removed CachyPlay and kept the edits you made in Steam",
        Outcome::NothingToUndo => "CachyPlay wasn't on for this game",
        Outcome::Applied | Outcome::AlreadyApplied => "unchanged",
    }
}

fn restore_all(paths: &Paths, dry_run: bool) -> Result<usize> {
    let base = ctx(paths)?;
    let mut count = 0;
    for account in touched_accounts(paths) {
        let c = base.for_account(&account);
        for app in c.managed_apps() {
            count += 1;
            if dry_run {
                let lo = c.launch_options(&app).ok().flatten().unwrap_or_default();
                println!(
                    "would restore {app} (account {account}): {:?} -> {:?}",
                    lo,
                    launch_options::strip(&lo)
                );
            } else {
                println!("{app} (account {account}): {}", describe(c.undo(&app)?));
            }
        }
    }
    Ok(count)
}

fn doctor(paths: &Paths) -> Result<()> {
    let ok = |b: bool| if b { "ok  " } else { "WARN" };
    match paths::installed_binary(paths) {
        Ok(p) => println!("[ok  ] Installed at {}", p.display()),
        Err(_) => println!(
            "[WARN] Not installed yet. Run `./cachyplay install` (or install the AUR package)."
        ),
    }
    match Ctx::new(paths.clone()) {
        Ok(c) => {
            println!(
                "[{}] Steam found ({:?}) at {}",
                ok(c.steam.kind == steam::InstallKind::Native),
                c.steam.kind,
                c.steam.root.display()
            );
            println!("[ok  ] Steam account {}", c.account.account_id);
            println!(
                "[{}] Steam is {}",
                ok(!steam::is_running()),
                if steam::is_running() {
                    "running (close it before turning games on or off)"
                } else {
                    "closed"
                }
            );
        }
        Err(e) => println!("[WARN] {e:#}"),
    }
    let f = recipes::Facts::detect();
    println!(
        "[ok  ] GPU vendor: {}   session: {}   CPU level: x86-64-{}",
        f.values["gpu.vendor"], f.values["session"], f.values["cpu.level"]
    );
    println!(
        "[{}] ntsync device (/dev/ntsync)",
        ok(f.values["ntsync"] == "true")
    );
    for tool in ["game-performance", "umu-run", "vulkaninfo"] {
        println!(
            "[{}] {tool}",
            ok(cachyplay_core::sys::which(tool).is_some())
        );
    }
    Ok(())
}

fn confirm(q: &str) -> Result<bool> {
    print!("{q} [y/N] ");
    std::io::stdout().flush()?;
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(matches!(s.trim(), "y" | "Y" | "yes"))
}
