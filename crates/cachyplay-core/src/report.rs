//! The timestamped, privacy-scrubbed bug report bundle.

use crate::diagnose;
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
    let dir = paths.report_dir();
    let mut name = file_name(game.map(|g| g.app_id.as_str()), &clicked);
    // Two clicks in the same second get distinct names.
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
        read_capped(
            &paths.home.join(format!("steam-{}.log", g.app_id)),
            HEAD,
            TAIL,
        )
    });
    let journal = capture(
        "journalctl",
        &["-k", "-b", "-p", "warning", "--no-pager", "-n", "500"],
        TOOL_TIMEOUT,
    );
    let mut findings = Vec::new();
    for text in [proton_log.as_deref(), journal.as_deref()]
        .into_iter()
        .flatten()
    {
        findings.extend(diagnose::scan(text));
    }

    // 00-summary.md
    let mut s = String::new();
    s.push_str(&format!(
        "# CachyPlay bug report\n\nCreated: {}\nCachyPlay: {}\n\n",
        clicked.to_rfc3339(),
        crate::VERSION
    ));
    if let Some(g) = game {
        s.push_str(&format!("Game: {} (AppID {})\n\n", g.name, g.app_id));
    }
    match launches.first() {
        Some(l) => s.push_str(&format!(
            "Last launch: {} - ran {:.0}s, exit code {:?}, signal {:?}, {}\n",
            l.started_at,
            l.runtime_secs,
            l.exit_code,
            l.signal,
            if l.failed { "FAILED" } else { "ok" }
        )),
        None => s.push_str("No launch through CachyPlay has been recorded for this game yet.\n"),
    }
    if proton_log.is_none() && game.is_some() {
        s.push_str("\nNo Proton log found. For a fuller report: add PROTON_LOG=1 to the game's launch options, reproduce the problem once, then create a new report.\n");
    }
    s.push_str("\n## Matched crash patterns\n\n");
    if findings.is_empty() {
        s.push_str("None of the known patterns matched.\n");
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
        let fs_info = mount_info(&g.install_dir);
        files.push((
            "02-game.json".into(),
            serde_json::to_string_pretty(&json!({
                "app_id": g.app_id,
                "name": g.name,
                "build_id": g.build_id,
                "install_dir": g.install_dir,
                "filesystem": fs_info,
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
    if let Some(j) = journal {
        files.push(("logs/kernel-warnings.log".into(), j));
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
            "proton-cachyos",
            "proton-cachyos-slr",
            "umu-launcher",
            "game-performance",
            "steam",
        ],
        TOOL_TIMEOUT,
    )
    .map(|t| {
        t.lines()
            .filter(|l| !l.starts_with("error:"))
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    json!({
        "os": os,
        "kernel": read("/proc/sys/kernel/osrelease"),
        "cpu": cpu_model,
        "cpu_level": cpu_level(&cpuinfo),
        "gpus": gpu_vendors(Path::new("/sys/class/drm")),
        "nvidia_driver": read("/sys/module/nvidia/version"),
        "vulkan_summary": capture("vulkaninfo", &["--summary"], TOOL_TIMEOUT).map(|t| t.lines().take(60).collect::<Vec<_>>().join("\n")),
        "session_type": std::env::var("XDG_SESSION_TYPE").ok(),
        "desktop": std::env::var("XDG_CURRENT_DESKTOP").ok(),
        "ntsync": Path::new("/dev/ntsync").exists(),
        "packages": pkgs,
    })
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
        let out = redact_steamid64(&out);
        out.lines()
            .map(redact_secret_line)
            .collect::<Vec<_>>()
            .join("\n")
            + if text.ends_with('\n') { "\n" } else { "" }
    }
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
        assert_eq!(a.path.parent().unwrap(), f.home().join("Desktop"));
        let mut z = zip::ZipArchive::new(fs::File::open(&a.path).unwrap()).unwrap();
        let names: Vec<String> = (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().to_string())
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
