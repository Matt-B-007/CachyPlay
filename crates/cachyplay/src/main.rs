//! `cachyplay` command-line interface. All logic lives in `cachyplay-core`.

use anyhow::{bail, Context, Result};
use cachyplay_core::manage::{touched_accounts, Ctx, Outcome};
use cachyplay_core::paths::{self, Paths};
use cachyplay_core::{install, keep, launch_options, recipes, report, steam, update, wrapper};
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
    /// Suggest fixes for a game, worked out from crashes, logs and your system
    Suggest { app_id: String },
    /// Apply a suggested fix to a game (or undo it with --undo)
    Fix {
        app_id: String,
        /// The suggestion id shown by `cachyplay suggest`
        id: String,
        #[arg(long)]
        undo: bool,
    },
    /// Show the settings CachyPlay applies to each game, as Steam launch options you can keep
    Settings {
        /// Also save them to a text file on your Desktop
        #[arg(long)]
        export: bool,
    },
    /// OBS game capture: start a game through obs-gamecapture so OBS records just the game.
    /// `cachyplay obs 1240440 on`, `cachyplay obs 1240440 off`, `cachyplay obs all on`, or no arguments to show it
    Obs {
        /// A game's AppID, or `all` for every game CachyPlay is on for
        target: Option<String>,
        /// on or off
        state: Option<String>,
    },
    /// Settings to try for one game, applied by CachyPlay at launch without touching Steam's
    /// launch options: `cachyplay try 1240440 "VKD3D_CONFIG=no_upload_hvv"`. Empty text removes them
    Try {
        app_id: String,
        /// NAME=value settings and game options; leave out %command%
        settings: Option<String>,
    },
    /// On PCs with a graphics card and a built-in chip: make a game use the card (on, the
    /// default) or let it pick (off). `cachyplay card 1240440 off`, or no state to show the chips
    Card {
        /// A game's AppID
        app_id: Option<String>,
        /// on or off
        state: Option<String>,
    },
    /// Show or change a game's compatibility tool (Proton version), like Steam's
    /// Properties → Compatibility. `cachyplay proton 5335510` lists them,
    /// `cachyplay proton 5335510 GE-Proton11-12` switches, `… undo` puts it back. Steam must be closed
    Proton {
        app_id: String,
        /// Tool name (from the list), or `undo`
        tool: Option<String>,
    },
    /// Check this PC for common gaming problems, and CachyPlay's own setup (also: --self-test).
    /// With --quick: only check this program starts and can read its settings (no network)
    #[command(alias = "self-test")]
    Doctor {
        #[arg(long)]
        quick: bool,
    },
    /// Set a game's Windows files (Proton prefix) aside so Steam makes fresh ones; --undo puts them back
    ResetPrefix {
        /// A game's AppID, or `all` with --delete-kept
        app_id: String,
        #[arg(long)]
        undo: bool,
        /// Delete the old copies earlier resets set aside (Undo is no longer possible)
        #[arg(long, conflicts_with = "undo")]
        delete_kept: bool,
    },
    /// Answer "Did it work?" for a fix: `cachyplay worked <appid> <fix-id> yes|no`
    Worked {
        app_id: String,
        fix: String,
        answer: String,
    },
    /// Install this downloaded copy to ~/.local/bin (its own location)
    Install,
    /// Restore all games, then remove CachyPlay's files
    Uninstall {
        /// Also delete save backups and config backups
        #[arg(long)]
        remove_backups: bool,
        /// Write each game's working settings into Steam so they keep working without CachyPlay
        #[arg(long)]
        keep_settings: bool,
        /// Also delete CachyPlay's settings, logs, and each game's fixes and history
        #[arg(long)]
        delete_settings: bool,
        /// Don't ask for confirmation
        #[arg(long, short)]
        yes: bool,
    },
    /// Check for a newer CachyPlay and install it
    Update {
        /// Only check, don't install
        #[arg(long)]
        check: bool,
        /// Go back to the version installed before the last update
        #[arg(long, conflicts_with = "check")]
        roll_back: bool,
        /// On a beta newer than the newest stable: install that stable version now
        #[arg(long, conflicts_with_all = ["check", "roll_back"])]
        to_stable: bool,
    },
    /// Show or change the update channel: stable or beta
    Channel { channel: Option<String> },
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
    match real_main(standard_flags(args)) {
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

/// The flags every app in the family accepts, mapped onto CachyPlay's commands:
/// --self-test [--quick] → doctor, --report [appid] → report, --update → update,
/// --rollback → update --roll-back, --uninstall → uninstall. (--version is built in.)
fn standard_flags(mut args: Vec<OsString>) -> Vec<OsString> {
    let Some(first) = args.get(1).and_then(|a| a.to_str()).map(str::to_string) else {
        return args;
    };
    let replacement: &[&str] = match first.as_str() {
        "--self-test" => &["doctor"],
        "--report" => &["report"],
        "--update" => &["update"],
        "--rollback" | "--roll-back" => &["update", "--roll-back"],
        "--uninstall" => &["uninstall"],
        _ => return args,
    };
    args.splice(1..2, replacement.iter().map(OsString::from));
    args
}

fn real_main(args: Vec<OsString>) -> Result<ExitCode> {
    let cli = Cli::parse_from(args);
    let paths = Paths::from_env()?;
    if let Cmd::Doctor { quick: true } = cli.cmd {
        // Read-only: skip the settings backup below.
    } else {
        cachyplay_core::housekeeping::protect_settings(&paths);
    }
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
        Cmd::Suggest { app_id } => {
            let c = ctx(&paths)?;
            let g = c
                .steam
                .find_game(&app_id)
                .with_context(|| format!("game {app_id} is not installed"))?;
            let list = cachyplay_core::advice::for_game(&c, &g);
            if list.is_empty() {
                println!(
                    "No suggestions for {}: CachyPlay found no crashes or problems to act on.",
                    g.name
                );
            }
            for sg in list {
                let state = match sg.status {
                    cachyplay_core::advice::Status::New => "",
                    cachyplay_core::advice::Status::Applied => "  [applied]",
                    cachyplay_core::advice::Status::Worked => "  [applied: worked]",
                    cachyplay_core::advice::Status::DidntHelp => "  [applied: didn't help]",
                };
                println!("\n{}{state}\n  id: {}", sg.title, sg.id);
                for w in &sg.why {
                    println!("  - {w}");
                }
                match &sg.fix {
                    Some(_) => println!("  Apply:  cachyplay fix {app_id} {}", sg.id),
                    None => sg.steps.iter().for_each(|st| println!("  Step:   {st}")),
                }
                match (&sg.switch_tool, sg.switched) {
                    (Some(_), true) => println!("  Undo:   cachyplay proton {app_id} undo"),
                    (Some(t), false) => println!("  Switch: cachyplay proton {app_id} {}", t.name),
                    _ => {}
                }
            }
        }
        Cmd::Proton { app_id, tool } => {
            let c = ctx(&paths)?;
            let g = c
                .steam
                .find_game(&app_id)
                .with_context(|| format!("game {app_id} is not installed"))?;
            let tools =
                cachyplay_core::compat::installed(&c.steam.root, &c.steam.library_folders());
            match tool.as_deref() {
                None => {
                    println!(
                        "{} uses: {}",
                        g.name,
                        c.steam
                            .compat_tool(&app_id)
                            .unwrap_or_else(|| "Steam's default".into())
                    );
                    println!("Installed compatibility tools:");
                    for t in &tools {
                        println!("  {:<24} {}", t.name, t.display);
                    }
                    println!(
                        "Switch with: cachyplay proton {app_id} <name>   (Steam must be closed)"
                    );
                }
                Some("undo") => {
                    cachyplay_core::compat::undo(&c, &app_id)?;
                    println!(
                        "{} is back on {}.",
                        g.name,
                        c.steam
                            .compat_tool(&app_id)
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "Steam's default".into())
                    );
                }
                Some(name) => {
                    let t = tools
                        .iter()
                        .find(|t| t.name == name || t.folder == name)
                        .with_context(|| format!("{name:?} isn't installed. Run `cachyplay proton {app_id}` to see what is."))?;
                    cachyplay_core::compat::switch(&c, &app_id, t)?;
                    println!(
                        "{} now uses {}. Start it from Steam to try. Undo: cachyplay proton {app_id} undo",
                        g.name, t.display
                    );
                }
            }
        }
        Cmd::Fix { app_id, id, undo } => {
            let c = ctx(&paths)?;
            let g = c
                .steam
                .find_game(&app_id)
                .with_context(|| format!("game {app_id} is not installed"))?;
            if undo {
                if cachyplay_core::fixes::remove(&paths, &app_id, &id)? {
                    println!("Undid \"{id}\" for {}.", g.name);
                } else {
                    println!("\"{id}\" wasn't applied to {}.", g.name);
                }
            } else {
                let bin = paths::installed_binary(&paths).ok();
                println!(
                    "{}",
                    cachyplay_core::advice::apply_suggestion(&c, &g, &id, bin.as_deref())?
                );
            }
        }
        Cmd::Settings { export } => {
            let c = ctx(&paths)?;
            let games = keep::all(&c);
            if games.is_empty() {
                println!("No game has CachyPlay settings right now.");
            }
            for g in &games {
                println!(
                    "\n{} ({})\n  Launch options without CachyPlay:\n    {}",
                    g.name, g.app_id, g.launch_options
                );
                for st in &g.settings {
                    println!("  - {}: {}  ({})", st.label, st.value, st.source);
                }
            }
            if export {
                let file = paths.report_dir().join("CachyPlay game settings.txt");
                cachyplay_core::fsutil::atomic_write(&file, keep::export_text(&games).as_bytes())?;
                println!("\nSaved to {}", file.display());
            }
        }
        Cmd::Doctor { quick: true } => {
            // Must not write anything: the updater runs this on a new version before installing it.
            match cachyplay_core::selftest::quick(&paths) {
                Ok(lines) => {
                    for l in lines {
                        println!("[ok  ] {l}");
                    }
                    println!("Quick self-test passed.");
                }
                Err(e) => {
                    eprintln!("[FAIL] {e:#}");
                    return Ok(ExitCode::from(1));
                }
            }
        }
        Cmd::Doctor { quick: false } => doctor(&paths)?,
        Cmd::ResetPrefix {
            app_id,
            undo,
            delete_kept,
        } => {
            let c = ctx(&paths)?;
            if delete_kept {
                let ids: Vec<String> = if app_id == "all" {
                    let mut v: Vec<String> =
                        cachyplay_core::prefix::all_kept(&c.steam.library_folders())
                            .into_iter()
                            .map(|k| k.1)
                            .collect();
                    v.dedup();
                    v
                } else {
                    vec![app_id.clone()]
                };
                let mut freed = 0;
                for id in ids {
                    if let Some(g) = c.steam.find_game(&id) {
                        freed += cachyplay_core::prefix::delete_kept(&c, &g)?;
                    }
                }
                println!("Freed {}.", cachyplay_core::prefix::human_size(freed));
                return Ok(ExitCode::SUCCESS);
            }
            let g = c
                .steam
                .find_game(&app_id)
                .with_context(|| format!("game {app_id} is not installed"))?;
            if undo {
                cachyplay_core::prefix::undo(&c, &g)?;
                println!("Put {}'s earlier Windows files back.", g.name);
            } else {
                let kept = cachyplay_core::prefix::reset(&c, &g)?;
                println!("Set {}'s Windows files aside at {}. Steam makes fresh ones on the next start; `cachyplay reset-prefix {app_id} --undo` puts them back.", g.name, kept.display());
            }
        }
        Cmd::Worked {
            app_id,
            fix,
            answer,
        } => {
            let yes = match answer.as_str() {
                "yes" | "y" => true,
                "no" | "n" => false,
                _ => bail!("answer yes or no"),
            };
            cachyplay_core::fixes::set_verdict(&paths, &app_id, &fix, yes)?;
            println!("Noted.");
        }
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
            keep_settings,
            delete_settings,
            yes,
        } => {
            println!("This will:");
            if keep_settings {
                println!("  1. Write each game's working settings into its Steam launch options (and save a copy to your Desktop)");
            } else {
                println!("  1. Restore the launch options of every game CachyPlay changed (your own options are kept)");
            }
            println!(
                "     and put back any compatibility tool (Proton version) CachyPlay switched{}",
                if keep_settings {
                    " (kept with --keep-settings)"
                } else {
                    ""
                }
            );
            if delete_settings {
                println!("  2. Remove CachyPlay, and its settings, logs, fix files and records");
            } else {
                println!("  2. Remove CachyPlay (its settings and logs stay; add --delete-settings to remove them)");
            }
            println!(
                "  3. {} save and config backups",
                if remove_backups { "DELETE" } else { "Keep" }
            );
            println!("  Bug reports on your Desktop are never deleted.");
            if !yes && !confirm("Continue?")? {
                println!("Cancelled. Nothing was changed.");
                return Ok(ExitCode::SUCCESS);
            }
            if keep_settings {
                if let Ok(c) = Ctx::for_restore(paths.clone()) {
                    let games = keep::all(&c);
                    if !games.is_empty() {
                        let file = paths.report_dir().join("CachyPlay game settings.txt");
                        cachyplay_core::fsutil::atomic_write(
                            &file,
                            keep::export_text(&games).as_bytes(),
                        )?;
                        println!("Saved a copy of your game settings to {}", file.display());
                    }
                }
            }
            let kept_note = Ctx::for_restore(paths.clone())
                .ok()
                .and_then(|c| cachyplay_core::compat::kept_copies_note(&c));
            restore_all_with(&paths, false, keep_settings)?;
            let removed = install::remove_user_files(&paths, remove_backups, delete_settings)?;
            if let Some(n) = kept_note {
                println!("{n}");
            }
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
        Cmd::Update {
            check,
            roll_back,
            to_stable,
        } => {
            if roll_back {
                println!("{}", update::roll_back(&paths)?);
                return Ok(ExitCode::SUCCESS);
            }
            let st = update::check(&paths);
            println!("Installed: {}   Channel: {:?}", st.current, st.channel);
            if let Some(e) = st.error {
                bail!(e);
            }
            match (&st.latest, st.available) {
                (Some(r), true) => {
                    println!("Newer version available: {}", r.tag);
                    if check {
                        return Ok(ExitCode::SUCCESS);
                    }
                    match st.kind {
                        update::InstallKind::User => println!("{}", update::install(&paths, r)?),
                        update::InstallKind::Package => println!(
                            "Update with your package manager:  {}",
                            st.command.unwrap_or_default()
                        ),
                        update::InstallKind::NotInstalled => println!(
                            "Install CachyPlay first (`./cachyplay install`), then update."
                        ),
                    }
                }
                _ if st.no_releases => println!(
                    "No CachyPlay release has been published on GitHub yet, so there's nothing to update to."
                ),
                _ => {
                    println!("You have the newest {:?} version.", st.channel);
                    if let Some(st_rel) = &st.stable_now {
                        if to_stable && st.kind == update::InstallKind::User {
                            println!("{}", update::install(&paths, st_rel)?);
                            return Ok(ExitCode::SUCCESS);
                        }
                        println!(
                            "You're on a beta newer than the newest stable ({}). You'll get stable updates from the next one on, or switch now with:  cachyplay update --to-stable",
                            st_rel.tag
                        );
                    }
                    if let Some(b) = &st.newer_beta {
                        println!(
                            "A newer beta is out: {}. Get it with:  cachyplay channel beta && cachyplay update",
                            b.tag
                        );
                    }
                }
            }
        }
        Cmd::Try { app_id, settings } => match settings {
            None => {
                let current = cachyplay_core::fixes::load(&paths, &app_id)
                    .fixes
                    .into_iter()
                    .find(|f| f.id == cachyplay_core::fixes::YOUR_SETTINGS);
                println!(
                    "{}",
                    current
                        .map(|f| f.label)
                        .unwrap_or_else(|| "No settings to try for this game.".into())
                );
            }
            Some(text) => match cachyplay_core::fixes::set_your_settings(&paths, &app_id, &text)? {
                Some(f) => println!(
                    "Saved \"{}\". Used the next time the game starts through CachyPlay.",
                    f.label
                ),
                None => println!("Removed your settings for {app_id}."),
            },
        },
        Cmd::Card { app_id, state } => {
            use cachyplay_core::gpus;
            let list = gpus::list(&cachyplay_core::pccheck::Root::system());
            for g in &list {
                println!(
                    "{} ({}){}",
                    g.describe(),
                    g.pci,
                    if g.displays.is_empty() {
                        String::new()
                    } else {
                        format!(", monitor on {}", g.displays.join(", "))
                    }
                );
            }
            if !gpus::has_both(&list) {
                println!("Only one kind of graphics chip here, so there's nothing to choose.");
            }
            match (app_id, state.as_deref()) {
                (Some(id), Some(s @ ("on" | "off"))) => {
                    gpus::set_prefer_card(&paths, &id, s == "on")?;
                    println!("Use the graphics card for {id}: {s}");
                }
                (Some(id), None) => println!(
                    "Use the graphics card for {id}: {}",
                    if gpus::prefer_card_for(&paths, &id) {
                        "on"
                    } else {
                        "off"
                    }
                ),
                (Some(_), Some(other)) => bail!("use `on` or `off`, not {other:?}"),
                (None, _) => {}
            }
        }
        Cmd::Obs { target, state } => {
            use cachyplay_core::obs;
            let on = match state.as_deref() {
                None => None,
                Some("on") => Some(true),
                Some("off") => Some(false),
                Some(other) => bail!("use `on` or `off`, not {other:?}"),
            };
            match (target.as_deref(), on) {
                (None, _) | (Some(_), None) => {
                    let s = obs::load(&paths);
                    println!(
                        "OBS game capture: {}",
                        if s.all {
                            "on for every game CachyPlay is on for"
                        } else {
                            "per game"
                        }
                    );
                    if !s.games.is_empty() {
                        println!("On for: {}", s.games.join(", "));
                    }
                    if !obs::installed() {
                        println!(
                            "obs-gamecapture isn't installed. Install it with:  {}",
                            obs::INSTALL_COMMAND
                        );
                    }
                }
                (Some("all"), Some(on)) => {
                    obs::set_all(&paths, on)?;
                    println!(
                        "OBS game capture for every game: {}",
                        if on { "on" } else { "off" }
                    );
                }
                (Some(app_id), Some(on)) => {
                    let ctx = ctx(&paths)?;
                    let game = ctx
                        .steam
                        .find_game(app_id)
                        .with_context(|| format!("game {app_id} is not installed"))?;
                    if on && !ctx.status(&game).is_ok_and(|s| s.applied) {
                        let bin = paths::installed_binary(&paths)?;
                        ctx.apply(&game, &bin)?;
                        println!("Turned CachyPlay on for {}.", game.name);
                    }
                    obs::set_game(&paths, app_id, on)?;
                    println!(
                        "OBS game capture for {}: {}",
                        game.name,
                        if on { "on" } else { "off" }
                    );
                    if on && !obs::installed() {
                        println!(
                            "obs-gamecapture isn't installed yet. Install it with:  {}",
                            obs::INSTALL_COMMAND
                        );
                    }
                }
            }
        }
        Cmd::Channel { channel } => {
            let mut s = update::Settings::load(&paths);
            match channel.as_deref() {
                None => {}
                Some("stable") => s.channel = update::Channel::Stable,
                Some("beta") => s.channel = update::Channel::Beta,
                Some(other) => bail!("unknown channel {other:?}; use `stable` or `beta`"),
            }
            s.save(&paths)?;
            println!("Update channel: {:?}", s.channel);
            if update::install_kind(&paths) == update::InstallKind::Package {
                println!(
                    "Installed from the AUR: switch packages with  {}",
                    update::package_command(s.channel)
                );
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
        Outcome::Kept => "kept its settings in Steam",
    }
}

fn restore_all(paths: &Paths, dry_run: bool) -> Result<usize> {
    restore_all_with(paths, dry_run, false)
}

/// Restore every managed game; with `keep`, games with settings get them written into Steam instead.
fn restore_all_with(paths: &Paths, dry_run: bool, keep_settings: bool) -> Result<usize> {
    let accounts = touched_accounts(paths);
    let switches =
        std::fs::read_dir(paths.data.join("compat-tool")).is_ok_and(|mut d| d.next().is_some());
    if accounts.is_empty() && !switches {
        return Ok(0); // nothing was ever changed; Steam isn't needed
    }
    let mut base = Ctx::for_restore(paths.clone())?;
    if let Some(p) = std::env::var_os("CACHYPLAY_PROC_ROOT") {
        base.proc_root = PathBuf::from(p); // tests only
    }
    let mut count = 0;
    for account in accounts {
        let c = base.for_account(&account);
        let kept: std::collections::BTreeMap<String, String> = if keep_settings {
            keep::all(&c)
                .into_iter()
                .map(|g| (g.app_id, g.launch_options))
                .collect()
        } else {
            Default::default()
        };
        for app in c.managed_apps() {
            count += 1;
            if let (false, Some(lo)) = (dry_run, kept.get(&app)) {
                c.keep_settings(&app, lo)?;
                println!("{app} (account {account}): kept its settings in Steam: {lo}");
                continue;
            }
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
    if dry_run {
        for (id, set) in cachyplay_core::compat::all_switched(&base) {
            println!("would put {id}'s compatibility tool back (CachyPlay set it to {set})");
        }
    } else {
        for line in cachyplay_core::compat::revert_all(&base, keep_settings) {
            println!("{line}");
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
    let show = |title: &str, checks: Vec<cachyplay_core::pccheck::Check>| {
        use cachyplay_core::pccheck::Level;
        println!("\n{title}");
        for c in checks {
            let tag = match c.level {
                Level::Ok => "ok  ",
                Level::Warn => "WARN",
                Level::Bad => "BAD ",
                Level::Info => "info",
            };
            println!("[{tag}] {}: {}", c.title, c.detail);
            if let Some(cmd) = c.command {
                println!("       Fix: {cmd}");
            }
        }
    };
    let libraries = Ctx::new(paths.clone())
        .map(|c| c.steam.library_folders())
        .unwrap_or_default();
    show(
        "PC check",
        cachyplay_core::pccheck::pc_checks(&cachyplay_core::pccheck::Root::system(), &libraries),
    );
    show(
        "CachyPlay self-test",
        cachyplay_core::pccheck::self_test(paths),
    );
    Ok(())
}

fn confirm(q: &str) -> Result<bool> {
    print!("{q} [y/N] ");
    std::io::stdout().flush()?;
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(matches!(s.trim(), "y" | "Y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(a: &[&str]) -> Vec<String> {
        standard_flags(a.iter().map(OsString::from).collect())
            .into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn standard_flags_map_to_commands() {
        assert_eq!(
            map(&["cachyplay", "--self-test", "--quick"]),
            ["cachyplay", "doctor", "--quick"]
        );
        assert_eq!(
            map(&["cachyplay", "--rollback"]),
            ["cachyplay", "update", "--roll-back"]
        );
        assert_eq!(
            map(&["cachyplay", "--report", "1240440"]),
            ["cachyplay", "report", "1240440"]
        );
        assert_eq!(
            map(&["cachyplay", "--uninstall", "-y"]),
            ["cachyplay", "uninstall", "-y"]
        );
        assert_eq!(map(&["cachyplay", "--update"]), ["cachyplay", "update"]);
        assert_eq!(map(&["cachyplay", "list"]), ["cachyplay", "list"]);
        for a in [
            ["cachyplay", "doctor", "--quick"],
            ["cachyplay", "self-test", "--quick"],
        ] {
            let cli = Cli::try_parse_from(a).unwrap();
            assert!(matches!(cli.cmd, Cmd::Doctor { quick: true }));
        }
    }
}
