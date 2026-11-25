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
    let clicked = chrono::Local::now();
    let now = clicked.naive_local();
    let dir = paths.report_dir();

    let logs = evidence::kernel_logs();
    let events = evidence::crash_events(paths, ctx, &logs, now);
    let (journal_now, journal_prev) = (logs.this_boot.clone(), logs.previous_boot.clone());

    // A general report names the game that crashed most recently, if we can tell.
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
    let proton_log = game.and_then(|g| {
        crate::fsutil::read_capped_keep(
            &paths.home.join(format!("steam-{}.log", g.app_id)),
            HEAD,
            TAIL,
            crate::fsutil::IMPORTANT_LOG_LINE,
        )
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
                Some(l) if l.running => s.push_str(&format!(
                    "- CachyPlay is on. The last launch started at {} and hasn't closed yet: the game is still running, or it hung (for example after a GPU crash).\n",
                    l.started_at
                )),
                Some(l) => s.push_str(&format!(
                    "- Last launch through CachyPlay: {}, ran {:.0}s, {}{}\n",
                    l.started_at,
                    l.runtime_secs,
                    match (l.exit_code, l.signal) {
                        (Some(c), _) => format!("exit code {c}"),
                        (None, Some(sig)) => format!("stopped by signal {sig}"),
                        _ => "exit status unknown".into(),
                    },
                    if l.failed { " (FAILED)" } else { "" }
                )),
                None if ctx.is_some_and(|c| c.status(g).is_ok_and(|st| st.applied)) => s.push_str(
                    "- CachyPlay is on for this game, but no launch through it has finished yet (the game may still be running, or hung).\n",
                ),
                None => s.push_str("- CachyPlay isn't turned on for this game, so no launch timing was recorded.\n"),
            }
            if let Some(c) = ctx {
                let sessions = crate::crashmatch::parse_steam_sessions(&c.steam.console_logs());
                let crash_refs: Vec<&crate::crashmatch::CrashEvent> = events.iter().collect();
                let runs = crate::history::runs(&sessions, &g.app_id, &crash_refs);
                if !runs.is_empty() {
                    s.push_str("\n## Recent starts (from Steam's log, newest first)\n\n");
                    for r in runs.iter().rev().take(12) {
                        let ran = r
                            .session
                            .duration()
                            .map(|d| format!("ran {}", crate::history::duration_text(d)))
                            .unwrap_or_else(|| "still running".into());
                        let tool = r
                            .session
                            .tool
                            .as_deref()
                            .map(|t| format!(" with {}", crate::compat::friendly(t)))
                            .unwrap_or_default();
                        let what = match r.outcome {
                            crate::history::Outcome::Crashed => " — GPU driver crash",
                            crate::history::Outcome::Short => " — closed within 3 minutes",
                            crate::history::Outcome::Played => " — played normally",
                            _ => "",
                        };
                        s.push_str(&format!(
                            "- {}: {ran}{tool}{what}\n",
                            r.session.start.format("%Y-%m-%d %H:%M")
                        ));
                    }
                }
            }
        }
        None => s.push_str("General report (not tied to one game).\n"),
    }

    s.push_str("\n## GPU crashes (newest first)\n\n");
    if events.is_empty() {
        s.push_str("No GPU driver errors in the kernel log for this boot or the previous one.\n");
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
    } else if !events.is_empty() {
        s.push_str("\nAll GPU errors came from the same game, which points to that game (or how it uses the driver) rather than the whole system.\n");
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
    if proton_log.is_none() && game.is_some() {
        s.push_str("\nNo Proton log found. For a fuller report: add `PROTON_LOG=1` at the start of the game's launch options in Steam, reproduce the problem once, then save a new report.\n");
    }

    s.push_str("\n## Matched patterns in the game's logs\n\n");
    if findings.is_empty() {
        s.push_str(if game.is_some() {
            "None of the known patterns matched.\n"
        } else {
            "(No game logs in a general report.)\n"
        });
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
    Ok(Report {
        path: dest,
        files: files.into_iter().map(|(p, _)| p).collect(),
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
    let pkgs = capture(
        "pacman",
        &[
            "-Q",
            "linux-cachyos",
            "mesa",
            "lib32-mesa",
            "vulkan-radeon",
            "nvidia-utils",
            "lib32-nvidia-utils",
            "nvidia-open-dkms",
            "linux-cachyos-nvidia-open",
            "proton-cachyos",
            "proton-cachyos-slr",
            "umu-launcher",
            "cachyos-settings",
            "steam",
        ],
        TOOL_TIMEOUT,
    )
    .map(|t| {
        let mut v: Vec<String> = t
            .lines()
            .filter(|l| !l.starts_with("error:") && !l.trim().is_empty())
            .map(str::to_string)
            .collect();
        v.dedup();
        let mut seen = std::collections::BTreeSet::new();
        v.retain(|l| seen.insert(l.clone()));
        v
    });
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
            // Windows-style paths inside Proton logs: Z:\home\you
            pairs.push((home.replace('/', "\\"), "~".to_string()));
        }
        if let Some(user) = std::env::var("USER").ok().filter(|u| u.len() > 2) {
            pairs.push((user, "$USER".into()));
        }
        if let Some(host) = fs::read_to_string("/etc/hostname")
            .ok()
            .map(|h| h.trim().to_string())
            .filter(|h| h.len() > 2)
        {
            pairs.push((host, "<hostname>".into()));
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
            (Regex::new(r"\b(?:[0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b").unwrap(), "<ipv6>"),
            (Regex::new(r"\b(?:[0-9a-fA-F]{1,4}:){1,6}:(?:[0-9a-fA-F]{1,4}:?){0,6}\b").unwrap(), "<ipv6>"),
            (Regex::new(&format!(r"\b(?:{octet}\.){{3}}{octet}\b")).unwrap(), "<ip>"),
        ]
    });
    let mut out = text.to_string();
    for (re, to) in pats {
        out = re
            .replace_all(&out, |c: &regex::Captures| {
                let m = &c[0];
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
            return format!("{k}=<redacted>");
        }
    }
    line.to_string()
}

#[cfg(test)]
mod tests {
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
        assert!(
            clean.contains("Xid (PCI:0000:02:00): 109"),
            "crash lines kept"
        );
        assert!(clean.contains("620.73.04"), "driver version kept");
        assert!(clean.contains("12:11:26"), "times kept");
        assert_eq!(clean.lines().filter(|l| l.contains("Xid")).count(), 7);
        assert!(redact_patterns("addr fe80::3cc:177e:6db:b541 x").contains("<ipv6>"));
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
        assert_eq!(out, "~/x userdata/<account>/config id <steamid> by <persona>\nGITHUB_TOKEN=<redacted>\nok=1\n");
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
