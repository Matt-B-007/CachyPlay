//! App updates from GitHub Releases, with a Stable or Beta channel.
//!
//! - Package installs (/usr/bin, from the AUR) are updated by pacman: we only show the command.
//! - User installs (~/.local/bin) update in place: download, verify the SHA-256 checksum, check the
//!   new binary runs and reports the expected version, keep the old one as `.prev`, then swap.
//! - Game-fix files are separate and never need an app update (signed fix updates arrive in 0.2.0).
//!
//! Network access goes through `curl`. While the repository is private, `curl` can't see it, so
//! the GitHub CLI (`gh`, if installed and signed in) is used instead.

use crate::fsutil::atomic_write;
use crate::install::{Manifest, GUI_BIN};
use crate::manage::{read_json, write_json};
use crate::paths::Paths;
use crate::sys;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const REPO: &str = "Matt-B-007/CachyPlay";
const NET_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Stable,
    Beta,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub channel: Channel,
}

impl Settings {
    pub fn load(paths: &Paths) -> Settings {
        read_json(&paths.config.join("settings.json")).unwrap_or_default()
    }
    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_json(&paths.config.join("settings.json"), self)
    }
}

/// A version like `0.1.0` or `0.1.0-beta.2`. A release sorts after its own betas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub nums: [u64; 3],
    pub pre: Option<(String, u64)>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim().trim_start_matches('v');
        let (base, pre) = match s.split_once('-') {
            Some((b, p)) => (b, Some(p)),
            None => (s, None),
        };
        let mut it = base.split('.').map(|n| n.parse::<u64>());
        let nums = [it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?];
        if it.next().is_some() {
            return None;
        }
        let pre = pre.map(|p| match p.split_once('.') {
            Some((label, n)) => (label.to_string(), n.parse().unwrap_or(0)),
            None => (p.to_string(), 0),
        });
        Some(Version { nums, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.nums
            .cmp(&other.nums)
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Release {
    pub tag: String,
    pub prerelease: bool,
    pub notes: String,
    pub page: String,
    /// Name of the Linux build attached to the release, if any.
    pub asset: Option<String>,
    pub asset_url: Option<String>,
    pub sums_url: Option<String>,
}

/// How this copy of CachyPlay gets updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallKind {
    /// ~/.local/bin: CachyPlay updates itself.
    User,
    /// /usr/bin: pacman/paru updates it.
    Package,
    /// Running from a build or download folder.
    NotInstalled,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateStatus {
    pub current: String,
    pub channel: Channel,
    pub kind: InstallKind,
    pub latest: Option<Release>,
    pub available: bool,
    /// For package installs: the command that updates CachyPlay.
    pub command: Option<String>,
    pub can_roll_back: bool,
    pub error: Option<String>,
}

pub fn install_kind(paths: &Paths) -> InstallKind {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|e| fs::canonicalize(e).ok())
        .unwrap_or_default();
    if exe.starts_with("/usr") {
        InstallKind::Package
    } else if exe.parent() == paths.user_bin().parent() {
        InstallKind::User
    } else {
        InstallKind::NotInstalled
    }
}

pub fn package_command(channel: Channel) -> String {
    match channel {
        Channel::Stable => "paru -S cachyplay".into(),
        Channel::Beta => "paru -S cachyplay-beta".into(),
    }
}

/// Parse the GitHub releases API response.
pub fn parse_releases(json: &str) -> Result<Vec<Release>> {
    let v: serde_json::Value =
        serde_json::from_str(json).context("GitHub sent an unexpected answer")?;
    let arr = v.as_array().context("GitHub sent an unexpected answer")?;
    let mut out = Vec::new();
    for r in arr {
        if r["draft"].as_bool().unwrap_or(false) {
            continue;
        }
        let Some(tag) = r["tag_name"].as_str() else {
            continue;
        };
        let assets = r["assets"].as_array().cloned().unwrap_or_default();
        let find = |pred: &dyn Fn(&str) -> bool| {
            assets
                .iter()
                .find(|a| a["name"].as_str().is_some_and(pred))
                .map(|a| {
                    (
                        a["name"].as_str().unwrap_or("").to_string(),
                        a["browser_download_url"].as_str().unwrap_or("").to_string(),
                    )
                })
        };
        let asset = find(&|n: &str| n.ends_with("-x86_64-linux.tar.zst"));
        let sums = find(&|n: &str| n == "SHA256SUMS.txt");
        out.push(Release {
            tag: tag.to_string(),
            prerelease: r["prerelease"].as_bool().unwrap_or(false),
            notes: r["body"].as_str().unwrap_or("").trim().to_string(),
            page: r["html_url"].as_str().unwrap_or("").to_string(),
            asset: asset.as_ref().map(|a| a.0.clone()),
            asset_url: asset.map(|a| a.1),
            sums_url: sums.map(|s| s.1),
        });
    }
    Ok(out)
}

/// The newest release on a channel. Beta sees betas and stable releases; Stable only releases.
pub fn pick(releases: &[Release], channel: Channel) -> Option<Release> {
    releases
        .iter()
        .filter(|r| channel == Channel::Beta || !r.prerelease)
        .filter_map(|r| Version::parse(&r.tag).map(|v| (v, r)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r)| r.clone())
}

fn gh_ready() -> bool {
    sys::run("gh", &["auth", "status"], NET_TIMEOUT).is_some_and(|(ok, _, _)| ok)
}

fn fetch_releases() -> Result<Vec<Release>> {
    // Tests and development: read a saved API response instead of asking GitHub.
    if let Some(p) = std::env::var_os("CACHYPLAY_RELEASES_FILE") {
        return parse_releases(&fs::read_to_string(p)?);
    }
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=30");
    if let Some((true, body, _)) = sys::run(
        "curl",
        &[
            "-fsSL",
            "-H",
            "Accept: application/vnd.github+json",
            "-A",
            "CachyPlay-updater",
            &url,
        ],
        NET_TIMEOUT,
    ) {
        return parse_releases(&body);
    }
    // Private repository (or GitHub unreachable via curl): try the signed-in GitHub CLI.
    if let Some((true, body, _)) = sys::run(
        "gh",
        &["api", &format!("repos/{REPO}/releases?per_page=30")],
        NET_TIMEOUT,
    ) {
        return parse_releases(&body);
    }
    if sys::which("curl").is_none() {
        bail!("curl is not installed, so CachyPlay can't check for updates.");
    }
    bail!("Couldn't reach GitHub. Check your internet connection. (While the repository is private, updates need the GitHub CLI `gh` signed in.)")
}

pub fn check(paths: &Paths) -> UpdateStatus {
    let settings = Settings::load(paths);
    let kind = install_kind(paths);
    let mut status = UpdateStatus {
        current: crate::VERSION.into(),
        channel: settings.channel,
        kind,
        latest: None,
        available: false,
        command: (kind == InstallKind::Package).then(|| package_command(settings.channel)),
        can_roll_back: paths.user_bin().with_extension("prev").is_file(),
        error: None,
    };
    match fetch_releases() {
        Ok(releases) => {
            let latest = pick(&releases, settings.channel);
            let cur = Version::parse(crate::VERSION);
            status.available = match (&latest, &cur) {
                (Some(r), Some(c)) => Version::parse(&r.tag).is_some_and(|v| v > *c),
                _ => false,
            };
            status.latest = latest;
        }
        Err(e) => status.error = Some(format!("{e:#}")),
    }
    status
}

/// Download, verify and install `release` over a user (~/.local/bin) install.
pub fn install(paths: &Paths, release: &Release) -> Result<String> {
    if install_kind(paths) != InstallKind::User {
        bail!("Only copies installed in ~/.local/bin can update themselves.");
    }
    let asset = release
        .asset
        .as_deref()
        .context("This release has no Linux build attached yet. Try again in a few minutes.")?;
    let dir = paths.cache.join("update");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    let asset_path = dir.join(asset);
    let sums_path = dir.join("SHA256SUMS.txt");

    let curl_ok = |url: &Option<String>, to: &Path| {
        url.as_deref().is_some_and(|u| {
            sys::run(
                "curl",
                &[
                    "-fsSL",
                    "-A",
                    "CachyPlay-updater",
                    "-o",
                    &to.to_string_lossy(),
                    u,
                ],
                DOWNLOAD_TIMEOUT,
            )
            .is_some_and(|(ok, _, _)| ok)
        })
    };
    let got = curl_ok(&release.asset_url, &asset_path) && curl_ok(&release.sums_url, &sums_path);
    if !got {
        if !gh_ready() {
            bail!("Couldn't download the update from GitHub.");
        }
        let d = dir.to_string_lossy().into_owned();
        let r = sys::run(
            "gh",
            &[
                "release",
                "download",
                &release.tag,
                "-R",
                REPO,
                "-p",
                asset,
                "-p",
                "SHA256SUMS.txt",
                "-D",
                &d,
                "--clobber",
            ],
            DOWNLOAD_TIMEOUT,
        );
        if !r.as_ref().is_some_and(|(ok, _, _)| *ok) {
            bail!(
                "Couldn't download the update: {}",
                r.map(|x| x.2).unwrap_or_default().trim()
            );
        }
    }
    install_from_files(paths, &asset_path, &sums_path, &release.tag)
}

/// Verify and install from an already-downloaded release archive.
pub fn install_from_files(paths: &Paths, archive: &Path, sums: &Path, tag: &str) -> Result<String> {
    let name = archive
        .file_name()
        .context("bad archive path")?
        .to_string_lossy()
        .into_owned();
    let sums_text = fs::read_to_string(sums).context("checksum file missing")?;
    let expected = sums_text
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            let (h, f) = (it.next()?, it.next()?);
            (f.trim_start_matches('*') == name).then(|| h.to_lowercase())
        })
        .context("The checksum file doesn't list this download. Not installing.")?;
    let actual = sys::run("sha256sum", &[&archive.to_string_lossy()], NET_TIMEOUT)
        .and_then(|(ok, out, _)| {
            ok.then(|| out.split_whitespace().next().unwrap_or("").to_lowercase())
        })
        .context("Couldn't compute the download's checksum")?;
    if actual != expected {
        bail!("The download is damaged or was changed (checksum mismatch). Nothing was installed.");
    }

    let unpack = archive.with_file_name("unpacked");
    let _ = fs::remove_dir_all(&unpack);
    fs::create_dir_all(&unpack)?;
    let ok = sys::run(
        "tar",
        &[
            "--zstd",
            "-xf",
            &archive.to_string_lossy(),
            "-C",
            &unpack.to_string_lossy(),
        ],
        DOWNLOAD_TIMEOUT,
    )
    .is_some_and(|(ok, _, _)| ok);
    if !ok {
        bail!("Couldn't unpack the update (is zstd installed?). Nothing was installed.");
    }
    let new_cli = find_file(&unpack, "cachyplay")
        .context("The update doesn't contain the cachyplay program.")?;
    let new_gui = find_file(&unpack, GUI_BIN);

    // The new program must run and report the version we expect.
    let want = Version::parse(tag).context("bad version tag")?;
    fs::set_permissions(&new_cli, fs::Permissions::from_mode(0o755))?;
    let reported = sys::run(&new_cli.to_string_lossy(), &["--version"], NET_TIMEOUT)
        .and_then(|(ok, out, _)| ok.then_some(out))
        .and_then(|out| out.split_whitespace().last().and_then(Version::parse));
    if reported.as_ref() != Some(&want) {
        bail!("The downloaded program didn't start correctly. Nothing was installed.");
    }

    let bin = paths.user_bin();
    let gui = bin.with_file_name(GUI_BIN);
    swap(&new_cli, &bin)?;
    if let Some(g) = &new_gui {
        swap(g, &gui)?;
    }
    let mut manifest: Manifest = read_json(&paths.install_manifest()).unwrap_or_default();
    manifest.version = tag.trim_start_matches('v').to_string();
    let _ = write_json(&paths.install_manifest(), &manifest);
    let _ = fs::remove_dir_all(archive.parent().unwrap_or(&unpack));
    Ok(format!("Updated to {tag}. Restart CachyPlay to use it."))
}

/// Keep the current file as `<name>.prev`, then atomically replace it.
fn swap(new: &Path, target: &Path) -> Result<()> {
    if target.is_file() {
        fs::copy(target, prev_path(target))?;
    }
    atomic_write(target, &fs::read(new)?)?;
    fs::set_permissions(target, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn prev_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".prev");
    PathBuf::from(s)
}

/// Go back to the version installed before the last update.
pub fn roll_back(paths: &Paths) -> Result<String> {
    let bin = paths.user_bin();
    let gui = bin.with_file_name(GUI_BIN);
    let mut restored = false;
    for target in [bin, gui] {
        let prev = prev_path(&target);
        if prev.is_file() {
            atomic_write(&target, &fs::read(&prev)?)?;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;
            fs::remove_file(&prev)?;
            restored = true;
        }
    }
    if !restored {
        bail!("There's no previous version to go back to.");
    }
    Ok("Went back to the previous version. Restart CachyPlay to use it.".into())
}

pub fn prev_files(paths: &Paths) -> Vec<PathBuf> {
    let bin = paths.user_bin();
    [prev_path(&bin), prev_path(&bin.with_file_name(GUI_BIN))]
        .into_iter()
        .filter(|p| p.exists())
        .collect()
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let rd = fs::read_dir(dir).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_file(&p, name) {
                return Some(f);
            }
        } else if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_order() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("v0.1.0") > v("0.1.0-beta.2"));
        assert!(v("0.1.0-beta.10") > v("0.1.0-beta.2"));
        assert!(v("0.2.0-beta.1") > v("0.1.0"));
        assert!(v("1.0.0") > v("0.9.9"));
        assert_eq!(v("v0.1.0"), v("0.1.0"));
        assert!(Version::parse("1.2").is_none());
    }

    const API: &str = r#"[
      {"tag_name":"v0.2.0-beta.1","prerelease":true,"draft":false,"body":"beta notes","html_url":"https://x/b",
       "assets":[{"name":"cachyplay-v0.2.0-beta.1-x86_64-linux.tar.zst","browser_download_url":"https://x/b.tar.zst"},
                 {"name":"SHA256SUMS.txt","browser_download_url":"https://x/bsums"}]},
      {"tag_name":"v0.1.0","prerelease":false,"draft":false,"body":"stable notes","html_url":"https://x/s","assets":[]},
      {"tag_name":"v9.9.9","prerelease":false,"draft":true,"body":"","html_url":"","assets":[]}
    ]"#;

    #[test]
    fn picks_by_channel_and_ignores_drafts() {
        let rs = parse_releases(API).unwrap();
        assert_eq!(rs.len(), 2);
        assert_eq!(pick(&rs, Channel::Stable).unwrap().tag, "v0.1.0");
        let beta = pick(&rs, Channel::Beta).unwrap();
        assert_eq!(beta.tag, "v0.2.0-beta.1");
        assert_eq!(
            beta.asset.as_deref(),
            Some("cachyplay-v0.2.0-beta.1-x86_64-linux.tar.zst")
        );
    }

    #[test]
    fn settings_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        assert_eq!(Settings::load(&p).channel, Channel::Stable);
        Settings {
            channel: Channel::Beta,
        }
        .save(&p)
        .unwrap();
        assert_eq!(Settings::load(&p).channel, Channel::Beta);
    }

    /// Build a release archive whose `cachyplay` prints the given version.
    fn fake_release(dir: &Path, tag: &str, corrupt: bool) -> (PathBuf, PathBuf) {
        let name = format!("cachyplay-{tag}-x86_64-linux");
        let root = dir.join("build").join(&name);
        fs::create_dir_all(&root).unwrap();
        let ver = tag.trim_start_matches('v');
        for bin in ["cachyplay", GUI_BIN] {
            let p = root.join(bin);
            fs::write(&p, format!("#!/bin/sh\necho \"{bin} {ver}\"\n")).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let dl = dir.join("dl");
        fs::create_dir_all(&dl).unwrap();
        let archive = dl.join(format!("{name}.tar.zst"));
        let st = std::process::Command::new("tar")
            .args(["--zstd", "-cf"])
            .arg(&archive)
            .arg("-C")
            .arg(dir.join("build"))
            .arg(&name)
            .status()
            .unwrap();
        assert!(st.success());
        let out = std::process::Command::new("sha256sum")
            .arg(&archive)
            .output()
            .unwrap();
        let mut hash = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .unwrap()
            .to_string();
        if corrupt {
            hash = "0".repeat(64);
        }
        let sums = dl.join("SHA256SUMS.txt");
        fs::write(&sums, format!("{hash}  {name}.tar.zst\n")).unwrap();
        (archive, sums)
    }

    #[test]
    fn installs_verified_update_and_rolls_back() {
        if sys::which("zstd").is_none() {
            eprintln!("skipping: zstd not installed");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        let bin = p.user_bin();
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        fs::write(&bin, "old-cli").unwrap();
        fs::write(bin.with_file_name(GUI_BIN), "old-gui").unwrap();

        let (archive, sums) = fake_release(d.path(), "v0.2.0-beta.1", false);
        let msg = install_from_files(&p, &archive, &sums, "v0.2.0-beta.1").unwrap();
        assert!(msg.contains("v0.2.0-beta.1"));
        assert!(fs::read_to_string(&bin).unwrap().contains("0.2.0-beta.1"));
        assert_eq!(fs::read_to_string(prev_path(&bin)).unwrap(), "old-cli");
        assert_eq!(prev_files(&p).len(), 2);

        roll_back(&p).unwrap();
        assert_eq!(fs::read_to_string(&bin).unwrap(), "old-cli");
        assert_eq!(
            fs::read_to_string(bin.with_file_name(GUI_BIN)).unwrap(),
            "old-gui"
        );
        assert!(prev_files(&p).is_empty());
    }

    #[test]
    fn refuses_bad_checksum_or_wrong_version() {
        if sys::which("zstd").is_none() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        let bin = p.user_bin();
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        fs::write(&bin, "old-cli").unwrap();

        let (archive, sums) = fake_release(d.path(), "v0.2.0", true);
        assert!(install_from_files(&p, &archive, &sums, "v0.2.0")
            .unwrap_err()
            .to_string()
            .contains("checksum"));
        let (archive, sums) = fake_release(&d.path().join("b"), "v0.2.0", false);
        assert!(
            install_from_files(&p, &archive, &sums, "v0.3.0").is_err(),
            "version mismatch is refused"
        );
        assert_eq!(
            fs::read_to_string(&bin).unwrap(),
            "old-cli",
            "nothing changed"
        );
    }
}
