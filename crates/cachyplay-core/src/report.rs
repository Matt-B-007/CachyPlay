//! The timestamped, privacy-scrubbed bug report bundle.

use crate::crashmatch;
use crate::diagnose;
use crate::evidence;
use crate::fsutil::read_capped;
use crate::manage::Ctx;
use crate::paths::Paths;
use crate::recipes::{cpu_level, gpu_vendors};
use crate::steam::Game;
use crate::sys::capture;
use crate::wrapper::recent_launches;
use anyhow::{Context, Result};
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const HEAD: usize = 500;
const TAIL: usize = 4000;
const TOOL_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Report {
    pub path: PathBuf,
    pub files: Vec<String>,
    /// The report's summary as saved (personal details already removed), for "Copy report".
    pub text: String,
}

/// What the person typed in the Report a problem dialog.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct UserNote {
    pub summary: String,
    pub description: String,
}

/// The lines every report starts with (House Standard): app, version, channel, install type,
/// distro, desktop, Wayland/X11, kernel.
pub fn header(paths: &Paths) -> Vec<(&'static str, String)> {
    let read = |p: &str| fs::read_to_string(p).ok().map(|s| s.trim().to_string());
    let distro = read("/etc/os-release")
        .and_then(|t| {
            t.lines().find_map(|l| {
                l.strip_prefix("PRETTY_NAME=")
                    .map(|v| v.trim_matches('"').to_string())
            })
        })
        .unwrap_or_else(|| "unknown".into());
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let session = match env("XDG_SESSION_TYPE").as_deref() {
        Some("wayland") => "Wayland".to_string(),
        Some("x11") => "X11".to_string(),
        Some(other) => other.to_string(),
        None if env("WAYLAND_DISPLAY").is_some() => "Wayland".into(),
        None if env("DISPLAY").is_some() => "X11".into(),
        None => "unknown".into(),
    };
    let settings = crate::update::Settings::load_read_only(paths);
    let kind = match crate::update::install_kind(paths) {
        crate::update::InstallKind::User => "user install (~/.local/bin)",
        crate::update::InstallKind::Package => "package (AUR)",
        crate::update::InstallKind::NotInstalled => "not installed (running from a folder)",
    };
    vec![
        ("App", "CachyPlay".into()),
        ("Version", crate::VERSION.into()),
        ("Channel", format!("{:?}", settings.channel)),
        ("Install type", kind.into()),
        ("Distro", distro),
        (
            "Desktop",
            env("XDG_CURRENT_DESKTOP").unwrap_or_else(|| "unknown".into()),
        ),
        ("Session", session),
        (
            "Kernel",
            read("/proc/sys/kernel/osrelease").unwrap_or_else(|| "unknown".into()),
        ),
    ]
}

/// `header` as lines of text.
pub fn header_text(paths: &Paths) -> String {
    header(paths)
        .into_iter()
        .map(|(k, v)| format!("{k}: {v}\n"))
        .collect()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SavedReport {
    pub name: String,
    pub path: String,
    /// AppID from the file name, if the report was for a game.
    pub app_id: Option<String>,
    /// "YYYY-MM-DD HH:MM:SS" from the file name.
    pub created: String,
    pub size_kb: u64,
}

/// Bug reports already saved in the report folder, newest first.
pub fn saved_reports(paths: &Paths) -> Vec<SavedReport> {
    let dir = paths.report_dir();
    let mut out: Vec<SavedReport> = fs::read_dir(&dir)
        .map(|rd| rd.flatten().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name
                .strip_prefix("cachyplay-bug-")?
                .strip_suffix(".zip")?
                .to_string();
            // stem: [<appid>-]YYYY-MM-DD_HH-MM-SS[-N]
            // The day_hour part (with '_') sits at index 2 for general reports, 3 for game reports.
            let parts: Vec<&str> = stem.split('-').collect();
            let (app_id, rest) = match parts.iter().position(|p| p.contains('_'))? {
                2 => (None, &parts[..]),
                3 if parts[0].chars().all(|c| c.is_ascii_digit()) => {
                    (Some(parts[0].to_string()), &parts[1..])
                }
                _ => return None,
            };
            if rest.len() < 5 {
                return None;
            }
            let (day, hour) = rest[2].split_once('_')?;
            let created = format!(
                "{}-{}-{} {}:{}:{}",
                rest[0], rest[1], day, hour, rest[3], rest[4]
            );
            Some(SavedReport {
                path: e.path().display().to_string(),
                size_kb: e.metadata().map(|m| m.len().div_ceil(1024)).unwrap_or(0),
                name,
                app_id,
                created,
            })
        })
        .collect();
    out.sort_by(|a, b| b.created.cmp(&a.created));
    out
}

/// File name for a report, using the time captured at the moment of the click.
pub fn file_name(app_id: Option<&str>, at: &chrono::DateTime<chrono::Local>) -> String {
    let stamp = at.format("%Y-%m-%d_%H-%M-%S");
    match app_id {
        Some(id) => format!("cachyplay-bug-{id}-{stamp}.zip"),
        None => format!("cachyplay-bug-{stamp}.zip"),
    }
}

pub fn create(paths: &Paths, ctx: Option<&Ctx>, game: Option<&Game>) -> Result<Report> {
    create_with(paths, ctx, game, None)
}

/// A report with what the person wrote in the Report a problem dialog at the top.
pub fn create_with(
    paths: &Paths,
    ctx: Option<&Ctx>,
    game: Option<&Game>,
    note: Option<&UserNote>,
) -> Result<Report> {
    let clicked = chrono::Local::now();
    let now = clicked.naive_local();
    let dir = paths.report_dir();

    let logs = evidence::kernel_logs();
    let events = evidence::crash_events(paths, ctx, &logs, now);
    let (journal_now, journal_prev) = (logs.this_boot.clone(), logs.previous_boot.clone());

    // A general report names the game that crashed most recently, if we can tell.
    let general = game.is_none();
    let auto_game: Option<Game> = match (game, ctx) {
        (None, Some(c)) => events
            .iter()
            .find_map(|e| e.app_id.as_ref())
            .and_then(|id| c.steam.find_game(id)),
        _ => None,
    };
    let game = game.or(auto_game.as_ref());
    let game_name = |app_id: &str| -> String {
        ctx.and_then(|c| c.steam.find_game(app_id))
            .map(|g| g.name)
            .unwrap_or_else(|| format!("AppID {app_id}"))
    };

    let mut name = file_name(game.map(|g| g.app_id.as_str()), &clicked);
    let base = name.trim_end_matches(".zip").to_string();
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("{base}-{n}.zip");
        n += 1;
    }
    let redact = Redactor::new(paths, ctx);
    let mut files: Vec<(String, String)> = Vec::new();

    let launches = game
        .map(|g| recent_launches(paths, &g.app_id, 5))
        .unwrap_or_default();
    let sessions_all = ctx
        .map(|c| crate::crashmatch::parse_steam_sessions(&c.steam.console_logs()))
        .unwrap_or_default();
    let last_start = game.and_then(|g| {
        sessions_all
            .iter()
            .filter(|x| x.app_id == g.app_id)
            .map(|x| x.start)
            .max()
    });
    // A Proton log older than the game's latest start is from an earlier run: say so.
    let mut proton_note: Option<String> = None;
    let proton_log = game.and_then(|g| {
        let (path, written) = crate::evidence::proton_log(paths, &g.app_id)?;
        let text = crate::fsutil::read_capped_keep(&path, HEAD, TAIL, crate::fsutil::IMPORTANT_LOG_LINE)?;
        let tool = text
            .lines()
            .find_map(|l| l.strip_prefix("Proton: "))
            .map(|t| t.split_whitespace().last().unwrap_or(t).to_string());
        if let Some(st) = last_start.filter(|st| written < *st - chrono::Duration::seconds(5)) {
            let n = format!(
                "This Proton log is from an earlier start (written {}{}), not the latest start ({}).",
                written.format("%b %-d %H:%M"),
                tool.as_deref().map(|t| format!(", {t}")).unwrap_or_default(),
                st.format("%b %-d %H:%M")
            );
            proton_note = Some(n.clone());
            Some(format!("# NOTE: {n}\n{text}"))
        } else {
            Some(text)
        }
    });
    let unity_logs: Vec<(String, String)> = match (game, ctx) {
        (Some(g), Some(c)) => c
            .steam
            .unity_logs(g)
            .iter()
            .filter_map(|p| {
                Some((
                    p.file_name()?.to_string_lossy().into_owned(),
                    read_capped(p, 300, 3000)?,
                ))
            })
            .collect(),
        _ => vec![],
    };
    let mut findings = Vec::new();
    for text in proton_log.iter().chain(unity_logs.iter().map(|(_, t)| t)) {
        findings.extend(diagnose::scan(text));
    }

    // ---------- 00-summary.md ----------
    let mut s = String::new();
    s.push_str(&format!(
        "# CachyPlay bug report\n\nCreated: {}\nCachyPlay: {}\n\n",
        clicked.format("%Y-%m-%d %H:%M:%S"),
        crate::VERSION
    ));
    match game {
        Some(g) => {
            s.push_str(&format!("**Game: {} (AppID {})**", g.name, g.app_id));
            if auto_game.is_some() {
                s.push_str(
                    " — picked automatically: it was running during the most recent GPU crash.",
                );
            }
            s.push('\n');
            if let Some(ts) = g
                .last_updated
                .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            {
                let local = ts.with_timezone(&chrono::Local);
                let days = (clicked - local).num_days();
                s.push_str(&format!(
                    "- Game last updated by Steam: {} ({} day{} before this report), build {}\n",
                    local.format("%Y-%m-%d %H:%M"),
                    days,
                    if days == 1 { "" } else { "s" },
                    g.build_id
                ));
            }
            if let Some(c) = ctx {
                let runs = match (c.steam.compat_tool(&g.app_id), c.steam.compatdata(g)) {
                    (Some(tool), _) => format!("Proton, forced in Steam: {tool}"),
                    (None, Some(_)) => "Proton (Steam's default)".to_string(),
                    (None, None) => "no Proton prefix found (runs natively, or hasn't been started with Proton yet)".to_string(),
                };
                s.push_str(&format!("- Runs with: {runs}\n"));
            }
            match launches.first() {
                Some(l) if l.running => {
                    // CachyPlay's record wasn't finished; Steam's log may know how it ended.
                    let at = chrono::DateTime::parse_from_rfc3339(&l.started_at).ok().map(|d| d.naive_local());
                    let steam_end = at.and_then(|t| {
                        sessions_all
                            .iter()
                            .find(|x| x.app_id == g.app_id && (x.start - t).num_seconds().abs() <= 15)
                            .and_then(|x| x.end.map(|e| (e - x.start).num_seconds()))
                    });
                    s.push_str(&match steam_end {
                        Some(secs) => format!(
                            "- Last start through CachyPlay: {}, ran {secs}s (from Steam's log; CachyPlay's own record of it wasn't finished)\n",
                            at.map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| l.started_at.clone())
                        ),
                        None => format!(
                            "- CachyPlay is on. The last start began at {} and is still running, or hung (for example after a GPU crash).\n",
                            l.started_at
                        ),
                    });
                }
                Some(l) => {
                    let at = chrono::DateTime::parse_from_rfc3339(&l.started_at).ok().map(|d| d.naive_local());
                    let newer = at
                        .map(|t| {
                            sessions_all
                                .iter()
                                .filter(|x| x.app_id == g.app_id && x.start > t + chrono::Duration::seconds(10))
                                .count()
                        })
                        .unwrap_or(0);
                    s.push_str(&format!(
                        "- Last start through CachyPlay: {}, ran {:.0}s, {}{}{}\n",
                        at.map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| l.started_at.clone()),
                        l.runtime_secs,
                        match (l.exit_code, l.signal) {
                            (Some(c), _) => format!("exit code {c}"),
                            (None, Some(sig)) => format!("stopped by signal {sig}"),
                            _ => "exit status unknown".into(),
                        },
                        if l.failed { " (FAILED)" } else { "" },
                        match newer {
                            0 => String::new(),
                            1 => ". The 1 newer start ran without CachyPlay".into(),
                            n => format!(". The {n} newer starts ran without CachyPlay"),
                        }
                    ));
                }
                None if ctx.is_some_and(|c| c.status(g).is_ok_and(|st| st.applied)) => s.push_str(
                    "- CachyPlay is on for this game, but no launch through it has finished yet (the game may still be running, or hung).\n",
                ),
                None => s.push_str("- CachyPlay isn't turned on for this game, so no launch timing was recorded.\n"),
            }
            if let Some(c) = ctx {
                if let Ok(st) = c.status(g) {
                    if !st.applied {
                        s.push_str(if st.edited_since {
                            "- CachyPlay is OFF for this game right now: its part of the launch options was replaced in Steam.\n"
                        } else {
                            "- CachyPlay is off for this game right now.\n"
                        });
                    }
                }
                let ev = crate::advice::gather(c, g, &logs);
                let p = crate::progress::progress(&ev, ev.now());
                s.push_str(&format!("\n## Status\n\n**{}**", p.headline));
                if let Some(d) = &p.detail {
                    s.push_str(&format!(" {d}"));
                }
                s.push_str(&format!(
                    "\n- A clean run is {}{}.\n",
                    p.needed_text,
                    if p.online_only {
                        ", online (the problem only happens online)"
                    } else {
                        ""
                    }
                ));
                if let Some(t) = driver_timeline(&ev) {
                    s.push_str(&format!("- {t}\n"));
                }
                let items = crate::progress::timeline(&ev, ev.now(), 12);
                if !items.is_empty() {
                    s.push_str("\n## Recent starts (newest first, from Steam's log)\n\n");
                    for it in &items {
                        match it {
                            crate::progress::Item::Run(r) => {
                                let mut bits: Vec<String> = Vec::new();
                                if let Some(t) = &r.tool {
                                    bits.push(crate::compat::friendly(t));
                                }
                                bits.push(if r.through_cachyplay {
                                    if r.applied.is_empty() {
                                        "CachyPlay on".into()
                                    } else {
                                        format!("CachyPlay: {}", r.applied.join("; "))
                                    }
                                } else {
                                    "CachyPlay off".into()
                                });
                                if let Some(gpu) = &r.gpu {
                                    bits.push(format!("on {gpu}"));
                                }
                                if let Some(n) = r.note.as_ref().and_then(|n| n.describe()) {
                                    let online = match r.note.as_ref().and_then(|n| n.online) {
                                        Some(true) => ", online",
                                        Some(false) => ", offline",
                                        None => "",
                                    };
                                    bits.push(format!("{n}{online}"));
                                }
                                if let Some(cn) = &r.counts {
                                    bits.push(cn.clone());
                                }
                                s.push_str(&format!(
                                    "- {}: {} ({})\n",
                                    r.start.replace('T', " "),
                                    r.text,
                                    bits.join(" · ")
                                ));
                            }
                            crate::progress::Item::Event { at, text, .. } => {
                                s.push_str(&format!("  - — {}: {text}\n", at.replace('T', " ")));
                            }
                        }
                    }
                }
            }
        }
        None => s.push_str("General report (not tied to one game).\n"),
    }

    s.push_str("\n## GPU crashes (newest first)\n\n");
    if events.is_empty() {
        s.push_str(&format!(
            "No graphics driver crashes in the last {} days (CachyPlay remembers them across restarts).\n",
            crate::evidence::CRASH_HISTORY_DAYS
        ));
    }
    for e in events.iter().take(10) {
        let who = match &e.app_id {
            Some(id) => game_name(id),
            None => format!(
                "unknown game (process \"{}\")",
                e.faults[0].process.clone().unwrap_or_else(|| "?".into())
            ),
        };
        s.push_str(&format!(
            "- **{}: {}**\n",
            e.at.format("%Y-%m-%d %H:%M:%S"),
            who
        ));
        for f in &e.faults {
            let what = match f.xid {
                Some(x) => format!("NVIDIA Xid {x}: {}", crashmatch::xid_meaning(x)),
                None => format!("{} GPU fault", f.vendor.to_uppercase()),
            };
            let offset = (f.at - e.at).num_seconds();
            let when = if offset > 0 {
                format!(" (+{offset}s)")
            } else {
                String::new()
            };
            s.push_str(&format!("  - {what}{when}\n"));
        }
    }
    if crashmatch::several_programs(&events) {
        s.push_str("\n**Several different games hit GPU driver errors.** That points to the graphics driver, a Vulkan add-on (layer) that every game loads, or GPU clocks/power, rather than one broken game. Check whether the NVIDIA driver updated around when the problems started (`grep -i 'upgraded nvidia' /var/log/pacman.log`).\n");
        // Which of them are already sorted out.
        if let Some(c) = ctx {
            let mut ids: Vec<&str> = events.iter().filter_map(|e| e.app_id.as_deref()).collect();
            ids.sort();
            ids.dedup();
            for id in ids {
                if let Some(gm) = c.steam.find_game(id) {
                    let ev = crate::advice::gather(c, &gm, &logs);
                    let p = crate::progress::progress(&ev, ev.now());
                    s.push_str(&format!("- {}: {}\n", gm.name, p.headline));
                }
            }
        }
    } else if let Some(e) = events.first() {
        let who = e
            .app_id
            .as_deref()
            .map(&game_name)
            .unwrap_or_else(|| "one program".into());
        let this = game.map(|g| e.app_id.as_deref() == Some(g.app_id.as_str()));
        match (this, game) {
            (Some(false), Some(g)) => s.push_str(&format!(
                "\nNone of these crashes are from {}. They're all from {who}, and are listed because they happened recently.\n",
                g.name
            )),
            _ => s.push_str(&format!(
                "\nAll graphics driver errors came from {who}. That points to {who} or how it uses the driver, not the whole system; check the driver timeline in the Status section too.\n"
            )),
        }
    }
    if let (Some(g), Some(c)) = (game, ctx) {
        if let Some((file, text)) = crate::gpulog::latest(&c.paths, &g.app_id) {
            let samples = crate::gpulog::parse(&text);
            let crash = events
                .iter()
                .filter(|e| e.app_id.as_deref() == Some(g.app_id.as_str()))
                .map(|e| e.at)
                .find(|t| {
                    samples.first().is_some_and(|f| *t >= f.at)
                        && samples
                            .last()
                            .is_some_and(|l| *t <= l.at + chrono::Duration::seconds(10))
                });
            if let Some(d) = crate::gpulog::describe(&samples, crash) {
                s.push_str(&format!(
                    "\n## Graphics card during the last start\n\n{d} (from {}, attached)\n",
                    file.file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ));
                files.push((
                    format!(
                        "logs/gpu-{}",
                        file.file_name()
                            .map(|f| f.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    ),
                    tail_lines(&text, 400),
                ));
            }
        }
    }
    if let (Some(g), Some(c)) = (game, ctx) {
        let suggestions = crate::advice::suggest(&crate::advice::gather(c, g, &logs));
        if !suggestions.is_empty() {
            s.push_str("\n## Suggested fixes (from CachyPlay's rules, in order)\n\n");
            for (i, sg) in suggestions.iter().enumerate() {
                let state = match sg.status {
                    crate::advice::Status::New => "",
                    crate::advice::Status::Applied => " — applied, not tried yet",
                    crate::advice::Status::Worked => " — applied, and the game has run fine since",
                    crate::advice::Status::DidntHelp => " — applied, but the game crashed again",
                };
                s.push_str(&format!("{}. **{}**{state}\n", i + 1, sg.title));
                for w in &sg.why {
                    s.push_str(&format!("   - {w}\n"));
                }
                if let Some(f) = &sg.fix {
                    let t: Vec<String> = f
                        .env
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .chain(f.args.iter().cloned())
                        .collect();
                    s.push_str(&format!(
                        "   - One-click fix in CachyPlay (sets `{}` for this game only)\n",
                        t.join(" ")
                    ));
                }
                for st in &sg.steps {
                    s.push_str(&format!("   - Step: {st}\n"));
                }
            }
        }
    }
    if let (Some(_), Some(st)) = (game, last_start) {
        if let Some((file, text)) = crate::gpulog::latest_performance(paths, Some(st)) {
            if let Some(sum) = crate::gpulog::performance_summary(&text) {
                let name = file
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                s.push_str(&format!("\n## Performance recording (last start)\n\n{sum} (MangoHud recording {name}, attached)\n"));
                files.push((format!("logs/performance-{name}"), tail_lines(&text, 3000)));
            }
        }
    }
    if let Some(n) = &proton_note {
        s.push_str(&format!("\n**Note:** {n} Turn on \"Detailed Proton log\" for the game and reproduce the problem once for a current one.\n"));
    }
    if proton_log.is_none() && game.is_some() {
        s.push_str("\nNo Proton log found. For a fuller report: add `PROTON_LOG=1` at the start of the game's launch options in Steam, reproduce the problem once, then save a new report.\n");
    }

    if game.is_some() {
        s.push_str("\n## Matched patterns in the game's logs\n\n");
        if findings.is_empty() {
            s.push_str("None of the known patterns matched.\n");
        }
    }
    for f in &findings {
        let bucket = match f.bucket {
            diagnose::Bucket::LikelyDriver => "likely driver issue",
            diagnose::Bucket::GameOrProton => "game or Proton issue",
        };
        s.push_str(&format!(
            "- **{}** ({bucket}): {}\n  - Suggestion: {}\n",
            f.rule, f.diagnosis, f.suggestion
        ));
        for l in &f.lines {
            s.push_str(&format!("  - `{l}`\n"));
        }
    }
    let hints = hardware_hints(
        &[journal_prev.as_deref(), journal_now.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n"),
    );
    if !hints.is_empty() {
        s.push_str("\n## Hardware hints\n\n");
        for h in &hints {
            s.push_str(&format!("- {h}\n"));
        }
    }
    if general {
        s.push_str(&general_sections(paths, ctx));
    }
    // The standard header, and what the person wrote, go first.
    let mut top = String::from("## About this report\n\n");
    for (k, v) in header(paths) {
        top.push_str(&format!("- {k}: {v}\n"));
    }
    if let Some(n) =
        note.filter(|n| !n.summary.trim().is_empty() || !n.description.trim().is_empty())
    {
        top.push_str("\n## In your words\n\n");
        if !n.summary.trim().is_empty() {
            top.push_str(&format!("**{}**\n\n", n.summary.trim()));
        }
        if !n.description.trim().is_empty() {
            top.push_str(&format!("{}\n", n.description.trim()));
        }
    }
    let s = match s.split_once('\n') {
        Some((title, rest)) if title.starts_with("# ") => format!("{title}\n\n{top}\n{rest}"),
        _ => format!("{top}\n{s}"),
    };
    files.push(("00-summary.md".into(), s));

    files.push((
        "01-system.json".into(),
        serde_json::to_string_pretty(&system_info())?,
    ));

    if let (Some(g), Some(ctx)) = (game, ctx) {
        let status = ctx.status(g).ok();
        files.push((
            "02-game.json".into(),
            serde_json::to_string_pretty(&json!({
                "app_id": g.app_id,
                "name": g.name,
                "picked_automatically": auto_game.is_some(),
                "build_id": g.build_id,
                "last_updated": g.last_updated.and_then(|t| chrono::DateTime::from_timestamp(t, 0)).map(|t| t.with_timezone(&chrono::Local).to_rfc3339()),
                "compat_tool": ctx.steam.compat_tool(&g.app_id),
                "has_proton_prefix": ctx.steam.compatdata(g).is_some(),
                "install_dir": g.install_dir,
                "filesystem": mount_info(&g.install_dir),
                "launch_options": status.as_ref().map(|s| &s.launch_options),
                "cachyplay_applied": status.as_ref().map(|s| s.applied),
                "edited_outside_cachyplay": status.as_ref().map(|s| s.edited_since),
                "steam_install": format!("{:?}", ctx.steam.kind),
                "recent_launches": launches,
            }))?,
        ));
        files.push((
            "03-changes.json".into(),
            serde_json::to_string_pretty(&json!({
                "snapshot": ctx.snapshot(&g.app_id),
                "change_log": ctx.change_log(&g.app_id),
            }))?,
        ));
        files.push((
            "04-user-tweaks.json".into(),
            serde_json::to_string_pretty(&json!({
                "user_launch_options": status.as_ref().map(|s| &s.user_options),
                "file_tweaks": status.as_ref().map(|s| &s.tweaks),
            }))?,
        ));
    }
    if let Some(l) = proton_log {
        files.push((
            format!(
                "logs/proton-steam-{}.log",
                game.map(|g| g.app_id.as_str()).unwrap_or("game")
            ),
            l,
        ));
    }
    for (name, text) in unity_logs {
        files.push((format!("logs/unity-{name}"), text));
    }
    if let Some(j) = journal_now {
        files.push(("logs/kernel-warnings.log".into(), tail_lines(&j, 1500)));
    }
    if let Some(j) =
        journal_prev.filter(|j| !j.trim().is_empty() && !j.contains("-- No entries --"))
    {
        files.push((
            "logs/kernel-warnings-previous-boot.log".into(),
            tail_lines(&j, 800),
        ));
    }
    if let Some(c) = ctx {
        // Only Steam's game start/stop lines: enough to see what ran when, nothing else.
        let console = c.steam.console_logs();
        let lines: Vec<&str> = console
            .lines()
            .filter(|l| l.contains("Game process"))
            .collect();
        if !lines.is_empty() {
            files.push((
                "logs/steam-game-sessions.log".into(),
                tail_lines(&lines.join("\n"), 300),
            ));
        }
    }
    if let Some(l) = read_capped(&paths.log_file(), HEAD, TAIL) {
        files.push(("logs/cachyplay.log".into(), l));
    }
    // This session's lines from memory (kept even if the log file was just rotated).
    let session = crate::logging::recent();
    if !session.is_empty() {
        files.push(("logs/this-session.txt".into(), session.join("\n") + "\n"));
    }
    // The newest crash trace of the desktop app, if any.
    if let Some(trace) = std::fs::read_dir(crate::crash::crash_dir(paths))
        .ok()
        .and_then(|rd| rd.flatten().map(|e| e.path()).max())
        .and_then(|p| read_capped(&p, HEAD, TAIL))
    {
        files.push(("logs/last-crash.txt".into(), trace));
    }

    // Write to a temp file next to the destination, then rename: no half-written reports.
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!(".{name}.part"));
    let folder = name.trim_end_matches(".zip");
    {
        let f = fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        let mut zip = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (path, body) in &files {
            zip.start_file(format!("{folder}/{path}"), opts)?;
            zip.write_all(redact.apply(body).as_bytes())?;
        }
        zip.finish()?.sync_all()?;
    }
    let dest = dir.join(&name);
    fs::rename(&tmp, &dest)?;
    let text = files
        .iter()
        .find(|(p, _)| p == "00-summary.md")
        .map(|(_, body)| redact.apply(body))
        .unwrap_or_default();
    Ok(Report {
        path: dest,
        files: files.into_iter().map(|(p, _)| p).collect(),
        text,
    })
}

fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n") + "\n"
}

pub use crate::evidence::drop_noise;

fn system_info() -> serde_json::Value {
    let read = |p: &str| fs::read_to_string(p).ok().map(|s| s.trim().to_string());
    let os = read("/etc/os-release").and_then(|t| {
        t.lines().find_map(|l| {
            l.strip_prefix("PRETTY_NAME=")
                .map(|v| v.trim_matches('"').to_string())
        })
    });
    let cpuinfo = read("/proc/cpuinfo").unwrap_or_default();
    let cpu_model = cpuinfo.lines().find_map(|l| {
        l.strip_prefix("model name")
            .and_then(|r| r.split_once(':'))
            .map(|(_, v)| v.trim().to_string())
    });
    let pkgs = capture("pacman", &["-Q"], TOOL_TIMEOUT).map(|t| gaming_packages(&t));
    json!({
        "os": os,
        "kernel": read("/proc/sys/kernel/osrelease"),
        "cpu": cpu_model,
        "cpu_level": cpu_level(&cpuinfo),
        "gpus": gpu_vendors(Path::new("/sys/class/drm")),
        "nvidia_driver": read("/sys/module/nvidia/version"),
        "vulkan_devices": capture("vulkaninfo", &["--summary"], TOOL_TIMEOUT).map(|t| vulkan_devices(&t)),
        "vulkan_layers": capture("vulkaninfo", &["--summary"], TOOL_TIMEOUT).map(|t| vulkan_layers(&t)),
        "session_type": std::env::var("XDG_SESSION_TYPE").ok(),
        "desktop": std::env::var("XDG_CURRENT_DESKTOP").ok(),
        "ntsync": Path::new("/dev/ntsync").exists(),
        "game_performance": crate::sys::which("game-performance").map(|p| p.display().to_string()),
        "packages": pkgs,
        "recent_driver_updates": recent_driver_updates(),
    })
}

/// The "Devices:" part of `vulkaninfo --summary`: one block per GPU (name, type, driver).
pub fn vulkan_devices(summary: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_devices = false;
    for l in summary.lines() {
        if l.trim_start().starts_with("Devices:") {
            in_devices = true;
            continue;
        }
        if in_devices {
            let t = l.trim();
            if t.is_empty() || t.chars().all(|c| c == '=') {
                continue;
            }
            out.push(t.to_string());
            if out.len() >= 80 {
                break;
            }
        }
    }
    out
}

/// Vulkan layer names (implicit layers get loaded into every game).
pub fn vulkan_layers(summary: &str) -> Vec<String> {
    summary
        .lines()
        .filter_map(|l| {
            l.split_whitespace()
                .next()
                .filter(|w| w.starts_with("VK_LAYER_"))
                .map(str::to_string)
        })
        .collect()
}

/// The last few pacman upgrades of graphics drivers and kernels, with dates.
/// "Halo's first graphics crash came 4 days after NVIDIA 620.73.04 was installed; it ran 3
/// times before that update without driver errors."
pub fn driver_timeline(ev: &crate::advice::Evidence) -> Option<String> {
    let first = ev
        .events
        .iter()
        .filter(|e| e.app_id.as_deref() == Some(ev.app_id.as_str()))
        .map(|e| e.at)
        .min()?;
    // Only real driver version changes: "7.3.6-1 → 7.3.6-2" is a rebuild of the same thing, and
    // linux-* packages are kernels and their prebuilt modules, versioned by kernel, not driver.
    let ver = |v: &str| {
        v.rsplit_once('-')
            .map(|(a, _)| a.to_string())
            .unwrap_or_else(|| v.to_string())
    };
    let upd = ev
        .driver_updates
        .iter()
        .filter(|u| u.at <= first && !u.package.starts_with("linux") && ver(&u.from) != ver(&u.to))
        .max_by_key(|u| u.at)?;
    let clean_before = ev
        .sessions
        .iter()
        .filter(|x| x.app_id == ev.app_id && x.start < upd.at)
        .filter(|x| {
            x.duration()
                .is_some_and(|d| d.num_seconds() >= crate::history::SHORT)
        })
        .count();
    if clean_before == 0 {
        // The game wasn't played before that update, so there's nothing to compare with.
        return None;
    }
    let days = (first - upd.at).num_days();
    Some(format!(
        "Driver timeline: {}'s first graphics crash came {} after {} {} → {} was installed ({}){}.",
        ev.game_name,
        match days {
            0 => "the same day".to_string(),
            1 => "1 day".to_string(),
            d => format!("{d} days"),
        },
        upd.package,
        upd.from,
        upd.to,
        upd.at.format("%b %-d"),
        if clean_before > 0 {
            format!(
                "; it ran {clean_before} time{} before that update without driver errors",
                if clean_before == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        }
    ))
}

/// The extra sections of a general report: games and their state, the PC check, CachyPlay's
/// self-test, and recent driver updates as a timeline.
fn general_sections(paths: &Paths, ctx: Option<&Ctx>) -> String {
    use crate::pccheck::Level;
    let mut s = String::new();
    if let Some(c) = ctx {
        let o = crate::library::overview(paths, Ok(c));
        let light = |l: crate::advice::Light| match l {
            crate::advice::Light::Red => "Not working",
            crate::advice::Light::Yellow => "Works with (or trying) a fix",
            crate::advice::Light::Green => "Runs fine",
            crate::advice::Light::Grey => "Not played lately",
        };
        let mut games: Vec<_> = o
            .games
            .iter()
            .filter(|g| g.light != crate::advice::Light::Grey || g.enabled)
            .collect();
        games.sort_by_key(|g| match g.light {
            crate::advice::Light::Red => 0,
            crate::advice::Light::Yellow => 1,
            crate::advice::Light::Green => 2,
            crate::advice::Light::Grey => 3,
        });
        s.push_str("\n## Games\n\n");
        if games.is_empty() {
            s.push_str("No games played lately, and CachyPlay isn't on for any game.\n");
        }
        for g in games {
            s.push_str(&format!(
                "- **{}** (AppID {}): {}{}\n",
                g.name,
                g.app_id,
                light(g.light),
                if g.enabled { "; CachyPlay is on" } else { "" }
            ));
        }
    }
    let libraries = ctx.map(|c| c.steam.library_folders()).unwrap_or_default();
    let checks = crate::pccheck::pc_checks(&crate::pccheck::Root::system(), &libraries);
    let shown: Vec<_> = checks.iter().filter(|c| c.level != Level::Ok).collect();
    s.push_str("\n## PC check\n\n");
    if shown.is_empty() {
        s.push_str(&format!("All {} checks passed.\n", checks.len()));
    }
    for c in &shown {
        let tag = match c.level {
            Level::Bad => "Problem",
            Level::Warn => "Worth fixing",
            _ => "Note",
        };
        s.push_str(&format!("- {tag}: **{}**. {}\n", c.title, c.detail));
        if let Some(cmd) = &c.command {
            s.push_str(&format!("  Command: `{cmd}`\n"));
        }
    }
    let selftest = crate::pccheck::self_test(paths);
    let failing: Vec<_> = selftest.iter().filter(|c| c.level != Level::Ok).collect();
    s.push_str("\n## CachyPlay self-test\n\n");
    if failing.is_empty() {
        s.push_str("CachyPlay is installed and set up correctly.\n");
    }
    for c in failing {
        s.push_str(&format!("- **{}**: {}\n", c.title, c.detail));
    }
    let updates = crate::evidence::read_driver_updates();
    if !updates.is_empty() {
        s.push_str("\n## Recent driver and kernel updates\n\n");
        for u in updates.iter().rev().take(12) {
            s.push_str(&format!(
                "- {}: {} {} → {}\n",
                u.at.format("%b %-d %H:%M"),
                u.package,
                u.from,
                u.to
            ));
        }
    }
    s
}

/// Packages that matter for games, from `pacman -Q` output: every graphics driver package (the
/// same list as driver updates), plus Proton, Steam and the usual gaming tools.
pub fn gaming_packages(pacman_q: &str) -> Vec<String> {
    const TOOLS: &[&str] = &[
        "steam",
        "umu-launcher",
        "cachyos-settings",
        "gamemode",
        "lib32-gamemode",
        "mangohud",
        "lib32-mangohud",
        "gamescope",
        "obs-vkcapture",
        "lib32-obs-vkcapture",
        "vulkan-icd-loader",
        "lib32-vulkan-icd-loader",
    ];
    pacman_q
        .lines()
        .filter(|l| {
            let pkg = l.split_whitespace().next().unwrap_or("");
            crate::evidence::is_graphics_package(pkg)
                || pkg.starts_with("proton")
                || pkg.starts_with("vulkan-mesa")
                || pkg.starts_with("lib32-vulkan-mesa")
                || TOOLS.contains(&pkg)
        })
        .map(str::to_string)
        .collect()
}

/// Hardware problems the kernel log shows at every start, in plain words.
pub fn hardware_hints(kernel_log: &str) -> Vec<String> {
    let mut ports: Vec<String> = Vec::new();
    for l in kernel_log.lines() {
        if l.contains("unable to enumerate USB device")
            || l.contains("device not accepting address")
        {
            // "usb usb8-port6: unable to enumerate" or "usb 8-6: device not accepting address"
            let port = l
                .split("usb ")
                .nth(1)
                .and_then(|r| r.split(':').next())
                .map(|p| p.trim_start_matches("usb").replace("-port", "-"))
                .unwrap_or_default();
            if !port.is_empty() && !ports.contains(&port) {
                ports.push(port);
            }
        }
    }
    ports
        .into_iter()
        .map(|p| format!("A USB device on port {p} doesn't start (\"device not accepting address\"). If a controller, headset or receiver isn't working, plug it into a different USB port. If everything works, you can ignore this."))
        .collect()
}

fn recent_driver_updates() -> Vec<String> {
    let Ok(log) = fs::read_to_string("/var/log/pacman.log") else {
        return vec![];
    };
    let mut v: Vec<String> = log
        .lines()
        .filter(|l| {
            l.contains("upgraded")
                && ["nvidia", "mesa", "linux-cachyos", "vulkan-"]
                    .iter()
                    .any(|k| l.contains(k))
        })
        .map(|l| l.chars().take(200).collect())
        .collect();
    let start = v.len().saturating_sub(15);
    v.drain(..start);
    v
}

/// Filesystem type and mount options of the folder holding the game (NTFS / noexec traps).
fn mount_info(path: &Path) -> serde_json::Value {
    let out = capture(
        "findmnt",
        &["-n", "-o", "FSTYPE,OPTIONS", "-T", &path.to_string_lossy()],
        TOOL_TIMEOUT,
    );
    match out
        .as_deref()
        .map(str::trim)
        .and_then(|l| l.split_once(char::is_whitespace))
    {
        Some((fstype, opts)) => {
            let opts = opts.trim();
            let warn = ["ntfs", "ntfs3", "fuseblk", "exfat", "vfat"].contains(&fstype)
                || opts.split(',').any(|o| o == "noexec");
            json!({ "type": fstype, "options": opts, "warning": warn })
        }
        None => json!(null),
    }
}

/// Removes personal details from everything written into the report.
pub struct Redactor {
    pairs: Vec<(String, String)>,
}

impl Redactor {
    pub fn new(paths: &Paths, ctx: Option<&Ctx>) -> Redactor {
        let mut pairs = Vec::new();
        let home = paths.home.to_string_lossy().into_owned();
        if home.len() > 1 {
            pairs.push((home.clone(), "~".to_string()));
            // Windows-style paths inside Proton logs: Z:\home\you (with or without the drive)
            pairs.push((format!("Z:{}", home.replace('/', "\\")), "~".to_string()));
            pairs.push((format!("z:{}", home.replace('/', "\\")), "~".to_string()));
            pairs.push((home.replace('/', "\\"), "~".to_string()));
        }
        if let Some(user) = std::env::var("USER").ok().filter(|u| u.len() > 2) {
            pairs.push((user, "<user>".into()));
        }
        if let Some(host) = fs::read_to_string("/etc/hostname")
            .ok()
            .map(|h| h.trim().to_string())
            .filter(|h| h.len() > 2)
        {
            pairs.push((host, "<host>".into()));
        }
        if let Some(ctx) = ctx {
            if ctx.account.account_id.len() > 3 {
                pairs.push((ctx.account.account_id.clone(), "<account>".into()));
            }
            if ctx.account.persona.len() > 2 {
                pairs.push((ctx.account.persona.clone(), "<persona>".into()));
            }
        }
        pairs.sort_by_key(|(a, _)| std::cmp::Reverse(a.len()));
        Redactor { pairs }
    }

    pub fn apply(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (from, to) in &self.pairs {
            out = out.replace(from, to);
        }
        let out = redact_patterns(&redact_steamid64(&out));
        out.lines()
            .map(redact_secret_line)
            .collect::<Vec<_>>()
            .join("\n")
            + if text.ends_with('\n') { "\n" } else { "" }
    }
}

/// Network and hardware identifiers: MAC addresses, IP addresses, NVIDIA GPU UUIDs.
pub fn redact_patterns(text: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static PATS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let pats = PATS.get_or_init(|| {
        let octet = r"(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)";
        vec![
            (Regex::new(r"GPU-[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}").unwrap(), "GPU-<id>"),
            (Regex::new(r"\b(?:[0-9a-fA-F]{2}:){5,}[0-9a-fA-F]{2}\b").unwrap(), "<mac>"),
            (Regex::new(r"\b(?:[0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b").unwrap(), "<ip>"),
            (Regex::new(r"\b(?:[0-9a-fA-F]{1,4}:){1,6}:(?:[0-9a-fA-F]{1,4}:?){0,6}\b").unwrap(), "<ip>"),
            // Access tokens that can appear in logs and environment dumps.
            (Regex::new(r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b").unwrap(), "<secret>"),
            (Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{16,}").unwrap(), "Bearer <secret>"),
            (Regex::new(&format!(r"\b(?:{octet}\.){{3}}{octet}\b")).unwrap(), "<ip>"),
        ]
    });
    // Identifiers that sit next to a label: keep the label, hide the value.
    static LABELED: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let labeled = LABELED.get_or_init(|| {
        vec![
            // A graphics card's unique ID (Vulkan's deviceUUID; NVIDIA's is per-card).
            (Regex::new(r"(deviceUUID\s*=\s*)[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}").unwrap(), "${1}<id>"),
            // Steam multiplayer lobbies you joined.
            (Regex::new(r"(\+connect_lobby\s+)\d+").unwrap(), "${1}<lobby>"),
        ]
    });
    let mut out = text.to_string();
    for (re, to) in labeled {
        out = re.replace_all(&out, *to).into_owned();
    }
    for (re, to) in pats {
        let snapshot = out.clone();
        out = re
            .replace_all(&snapshot, |c: &regex::Captures| {
                let whole = c.get(0).unwrap();
                let m = whole.as_str();
                if *to == "<ip>" && looks_like_version(&snapshot, whole.start(), whole.end()) {
                    return m.to_string();
                }
                // Keep obviously harmless addresses that help debugging.
                if *to == "<ip>" && (m == "0.0.0.0" || m.starts_with("127.")) {
                    m.to_string()
                } else {
                    (*to).to_string()
                }
            })
            .into_owned();
    }
    out
}

/// Four dotted numbers that are a version, not a network address: a package version followed by
/// its release number ("steam 1.0.0.85-3"), or a value on a "…Version" line ("conformanceVersion
/// = 1.4.3.0").
/// A dotted number that's a version, not an IP address: `v0.11.0.1`, `shellcheck-py==0.11.0.1`,
/// `pkg@1.2.3.4`, `foo-1.2.3.4`, `version 1.2.3.4`, or followed by `-1` (a package release).
fn looks_like_version(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    if before.is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '@' | '-' | '_'))
        || text[..start].ends_with("==")
        || text[..start].ends_with(">=")
        || text[..start].ends_with("<=")
        || text[..start].ends_with("~=")
    {
        return true;
    }
    let after = &text[end..];
    let mut a = after.chars();
    if a.next() == Some('-') && a.next().is_some_and(|c| c.is_ascii_digit()) {
        return true;
    }
    let line_start = text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    text[line_start..start]
        .to_ascii_lowercase()
        .contains("version")
}

fn redact_steamid64(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if text[i..].starts_with("7656119")
            && i + 17 <= b.len()
            && b[i..i + 17].iter().all(u8::is_ascii_digit)
            && (i == 0 || !b[i - 1].is_ascii_digit())
            && (i + 17 == b.len() || !b[i + 17].is_ascii_digit())
        {
            out.push_str("<steamid>");
            i += 17;
        } else {
            let ch = text[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn redact_secret_line(line: &str) -> String {
    const KEYS: [&str; 6] = ["TOKEN", "SECRET", "PASSWORD", "PASSWD", "APIKEY", "API_KEY"];
    if let Some((k, _)) = line.split_once('=') {
        let up = k.to_uppercase();
        if !k.contains(' ') && KEYS.iter().any(|s| up.contains(s)) {
            return format!("{k}=<secret>");
        }
    }
    line.to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn driver_timeline_points_at_the_update_before_the_first_crash() {
        use crate::crashmatch::{CrashEvent, GpuFault, Session};
        let t = |x: &str| chrono::NaiveDateTime::parse_from_str(x, "%Y-%m-%d %H:%M").unwrap();
        let ev = crate::advice::Evidence {
            app_id: "1240440".into(),
            game_name: "Halo Infinite".into(),
            events: vec![CrashEvent {
                at: t("2026-11-25 16:53"),
                app_id: Some("1240440".into()),
                faults: vec![GpuFault::default()],
            }],
            driver_updates: vec![crate::evidence::DriverUpdate {
                at: t("2026-11-21 12:00"),
                package: "nvidia-utils".into(),
                from: "620.70.06-1".into(),
                to: "620.73.04-1".into(),
            }],
            sessions: vec![Session {
                app_id: "1240440".into(),
                start: t("2026-11-07 16:06"),
                end: Some(t("2026-11-07 16:14")),
                source: "Steam",
                tool: None,
                settings: vec![],
            }],
            ..Default::default()
        };
        let line = driver_timeline(&ev).unwrap();
        assert_eq!(line, "Driver timeline: Halo Infinite's first graphics crash came 4 days after nvidia-utils 620.70.06-1 → 620.73.04-1 was installed (Nov 21); it ran 1 time before that update without driver errors.");
        // A kernel-module rebuild (same version, new release) isn't a driver change.
        let mut rebuild = ev.clone();
        rebuild.driver_updates = vec![crate::evidence::DriverUpdate {
            at: t("2026-11-25 07:48"),
            package: "linux-cachyos-nvidia-open".into(),
            from: "7.3.6-1".into(),
            to: "7.3.6-2".into(),
        }];
        assert!(driver_timeline(&rebuild).is_none());
        // A game first played after the update: nothing to compare with.
        let mut new_game = ev.clone();
        new_game.sessions.clear();
        assert!(driver_timeline(&new_game).is_none());
    }

    #[test]
    fn lists_every_graphics_package() {
        let q = "firefox 140-1\nlib32-vulkan-radeon 3:26.3.1-5\nvulkan-mesa-implicit-layers 3:26.3.1-5\nlib32-nvidia-utils 620.73.04-1\nnvidia-settings 620.73.04-1\nlinux-cachyos-headers 7.3.6-2\nproton-cachyos-slr 1:11.0-1\nsteam 1.0.0.85-3\nobs-vkcapture 1.5-1\n";
        let p = gaming_packages(q);
        for want in [
            "lib32-vulkan-radeon",
            "vulkan-mesa-implicit-layers",
            "lib32-nvidia-utils",
            "nvidia-settings",
            "proton-cachyos-slr",
            "steam 1.0.0.85-3",
            "obs-vkcapture",
        ] {
            assert!(
                p.iter().any(|l| l.starts_with(want)),
                "{want} missing from {p:?}"
            );
        }
        assert!(!p
            .iter()
            .any(|l| l.starts_with("firefox") || l.starts_with("linux-cachyos-headers")));
    }

    #[test]
    fn explains_a_usb_device_that_never_starts() {
        let log = "2026-11-25T07:50:57-08:00 cachyos kernel: usb 8-6: device not accepting address 6, error -71\n2026-11-25T07:51:08-08:00 cachyos kernel: usb usb8-port6: unable to enumerate USB device\n";
        let h = hardware_hints(log);
        assert_eq!(h.len(), 1, "{h:?}");
        assert!(h[0].starts_with("A USB device on port 8-6 doesn't start"));
        assert!(hardware_hints("nothing here").is_empty());
    }

    use super::*;
    use crate::testutil::FakeSteam;

    #[test]
    fn real_log_is_scrubbed_and_firewall_lines_dropped() {
        let log = include_str!("../tests/fixtures/kernel-warnings-short.log");
        let clean = redact_patterns(&drop_noise(log));
        assert!(!clean.contains("UFW"), "firewall lines removed");
        assert!(!clean.contains("02:00:5e"), "MAC removed");
        assert!(!clean.contains("192.168.1.50") && !clean.contains("203.0.113.7"));
        assert!(!clean.contains("GPU-00000000"), "GPU UUID removed");
        assert!(clean.contains("GPU-<id>"));
        // Versions that look like addresses are kept; real identifiers are hidden.
        let t = redact_patterns("\"steam 1.0.0.85-3\"\nconformanceVersion = 1.4.3.0\nserver 192.168.1.20 up\ndeviceUUID         = df5ac9e6-eac4-feb8-506c-c5f52cb98b0d\ndriverUUID = 9e20a2ff-3e91-5184-8cb2-81aca6a96652\nMoonSweepers.exe -force-d3d11 +connect_lobby 109775243962158414\", ProcID 1");
        assert!(t.contains("steam 1.0.0.85-3"), "{t}");
        assert!(t.contains("conformanceVersion = 1.4.3.0"), "{t}");
        assert!(t.contains("server <ip> up"), "{t}");
        assert!(t.contains("deviceUUID         = <id>"), "{t}");
        assert!(
            t.contains("driverUUID = 9e20a2ff"),
            "driver IDs are the same on every PC: {t}"
        );
        assert!(t.contains("+connect_lobby <lobby>"), "{t}");
        assert!(
            clean.contains("Xid (PCI:0000:02:00): 109"),
            "crash lines kept"
        );
        assert!(clean.contains("620.73.04"), "driver version kept");
        assert!(clean.contains("12:11:26"), "times kept");
        assert_eq!(clean.lines().filter(|l| l.contains("Xid")).count(), 7);
        assert!(redact_patterns("addr fe80::3cc:177e:6db:b541 x").contains("<ip>"));
        // Version numbers are not IP addresses.
        for v in [
            "shellcheck-py==0.11.0.1",
            "pip install foo>=1.2.3.4",
            "v1.2.3.4",
            "lib@2.0.0.1",
            "mesa-25.1.0.1",
            "proton_9.0.1.2",
            "version: 10.0.0.1",
        ] {
            assert_eq!(redact_patterns(v), v, "{v}");
        }
        assert_eq!(redact_patterns("from 10.0.0.1 port"), "from <ip> port");
        assert_eq!(redact_patterns("IP=192.168.1.20"), "IP=<ip>");
        assert_eq!(
            redact_patterns("IP 0.0.0.0:0 and 1.4.357"),
            "IP 0.0.0.0:0 and 1.4.357"
        );
    }

    #[test]
    fn vulkan_device_section_is_kept() {
        let summary = "Instance Layers: count = 2\nVK_LAYER_AFMF   afmf\nVK_LAYER_MAKO_render  mako\n\nDevices:\n========\nGPU0:\n\tdeviceName = NVIDIA GeForce RTX 5090\n\tdriverName = NVIDIA\nGPU1:\n\tdeviceName = AMD Radeon Graphics (RADV RAPHAEL_MENDOCINO)\n";
        let d = vulkan_devices(summary);
        assert!(
            d.iter().any(|l| l.contains("RTX 5090")) && d.iter().any(|l| l.contains("RADV")),
            "{d:?}"
        );
        assert_eq!(
            vulkan_layers(summary),
            vec!["VK_LAYER_AFMF", "VK_LAYER_MAKO_render"]
        );
    }

    #[test]
    fn file_name_format() {
        let t = chrono::NaiveDate::from_ymd_opt(2026, 11, 21)
            .unwrap()
            .and_hms_opt(21, 52, 4)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .unwrap();
        assert_eq!(
            file_name(Some("1240440"), &t),
            "cachyplay-bug-1240440-2026-11-21_21-52-04.zip"
        );
        assert_eq!(file_name(None, &t), "cachyplay-bug-2026-11-21_21-52-04.zip");
    }

    #[test]
    fn redacts_personal_details() {
        let f = FakeSteam::new();
        let ctx = f.ctx();
        let r = Redactor::new(&f.paths(), Some(&ctx));
        let home = f.home().display().to_string();
        let out = r.apply(&format!("{home}/x userdata/22202/config id 76561197960287930 by Tester\nGITHUB_TOKEN=abc\nok=1\n"));
        assert_eq!(out, "~/x userdata/<account>/config id <steamid> by <persona>\nGITHUB_TOKEN=<secret>\nok=1\n");
        // Windows-style paths in Proton logs, tokens, and network addresses.
        let win = home.replace('/', "\\");
        let out = r.apply(&format!("Z:{win}\\Games\\x.exe {win}\\y ghp_abcdefghijklmnopqrstuvwxyz0123 Authorization: Bearer abcdefghijklmnop1234 10.1.2.3 aa:bb:cc:dd:ee:ff\n"));
        assert_eq!(
            out,
            "~\\Games\\x.exe ~\\y <secret> Authorization: Bearer <secret> <ip> <mac>\n"
        );
        // Game and app ids are never touched, only 17-digit Steam account ids.
        assert_eq!(
            r.apply("AppID 1240440 and 76561197960287930"),
            "AppID 1240440 and <steamid>"
        );
    }

    #[test]
    fn reports_start_with_the_standard_header_and_your_words() {
        let f = FakeSteam::new();
        let ctx = f.ctx();
        let note = UserNote {
            summary: "Halo freezes online".into(),
            description: format!("It froze at {}/x after 5 min", f.home().display()),
        };
        let r = create_with(&f.paths(), Some(&ctx), None, Some(&note)).unwrap();
        for key in [
            "App: CachyPlay",
            "Version:",
            "Channel:",
            "Install type:",
            "Distro:",
            "Desktop:",
            "Session:",
            "Kernel:",
        ] {
            assert!(r.text.contains(key), "missing {key} in:\n{}", r.text);
        }
        assert!(r.text.contains("**Halo freezes online**"));
        assert!(
            r.text.contains("It froze at ~/x"),
            "your words are redacted too: {}",
            r.text
        );
        let header = header_text(&f.paths());
        assert!(header.starts_with("App: CachyPlay\nVersion: "));
    }

    #[test]
    fn creates_zip_on_desktop() {
        let f = FakeSteam::new();
        let ctx = f.ctx();
        let game = ctx.steam.find_game("1240440").unwrap();
        fs::write(
            f.home().join("steam-1240440.log"),
            "err:module:import_dll Library d3dcompiler_47.dll not found\n",
        )
        .unwrap();
        let a = create(&f.paths(), Some(&ctx), Some(&game)).unwrap();
        let b = create(&f.paths(), Some(&ctx), Some(&game)).unwrap();
        assert_ne!(a.path, b.path, "two reports never share a name");
        let general = create(&f.paths(), Some(&ctx), None).unwrap();
        let saved = saved_reports(&f.paths());
        assert_eq!(saved.len(), 3);
        assert!(
            saved
                .iter()
                .filter(|r| r.app_id.as_deref() == Some("1240440"))
                .count()
                == 2
        );
        assert!(saved
            .iter()
            .any(|r| r.app_id.is_none() && r.path == general.path.display().to_string()));
        assert_eq!(saved[0].created.len(), 19, "{:?}", saved[0]);
        assert_eq!(a.path.parent().unwrap(), f.home().join("Desktop"));
        let mut z = zip::ZipArchive::new(fs::File::open(&a.path).unwrap()).unwrap();
        let names: Vec<String> = (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().unwrap().to_string())
            .collect();
        assert!(
            names.iter().any(|n| n.ends_with("00-summary.md")),
            "{names:?}"
        );
        let mut summary = String::new();
        let summary_name = names
            .iter()
            .find(|n| n.ends_with("00-summary.md"))
            .unwrap()
            .clone();
        std::io::Read::read_to_string(&mut z.by_name(&summary_name).unwrap(), &mut summary)
            .unwrap();
        assert!(summary.contains("missing_dll"), "{summary}");
        assert!(!summary.contains(&f.home().display().to_string()));
    }
}
