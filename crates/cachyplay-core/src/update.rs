//! App updates from GitHub Releases, with a Stable or Beta channel.
//!
//! - Package installs (/usr/bin, from the AUR) are updated by pacman: we only show the command.
//! - User installs (~/.local/bin) update in place: download, verify the minisign signature of the
//!   checksum file and the SHA-256 checksum, run the new build's `--self-test --quick`, keep the old
//!   one as `.prev`, then swap. A release without a signature is "not available yet".
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
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const REPO: &str = "Matt-B-007/CachyPlay";
const NET_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Beta,
}

/// Until you pick a channel, a beta build follows Beta (otherwise it would never see the next beta)
/// and a stable build follows Stable.
pub fn default_channel() -> Channel {
    if crate::VERSION.contains('-') {
        Channel::Beta
    } else {
        Channel::Stable
    }
}

impl Default for Channel {
    fn default() -> Self {
        default_channel()
    }
}

/// The settings file's format. Bump it and add a step to `migrate` when the format changes.
pub const SETTINGS_VERSION: u32 = 1;
/// A copy of the settings is kept at most this often (seconds), newest 5 kept.
const SETTINGS_BACKUP_EVERY: u64 = 15 * 60;
const SETTINGS_BACKUPS_KEPT: usize = 5;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Format of this file (see `SETTINGS_VERSION`). Missing means the format before beta.16.
    #[serde(default)]
    pub settings_version: u32,
    #[serde(default)]
    pub channel: Channel,
    /// Look up games' ProtonDB ratings (sends only the AppID). Off unless turned on.
    #[serde(default)]
    pub protondb: bool,
    /// Bigger text and controller-friendly navigation, for the couch.
    #[serde(default)]
    pub big_screen: bool,
    /// "system" (follow the desktop), "dark" or "light".
    #[serde(default)]
    pub theme: Option<String>,
}

impl Settings {
    fn file(paths: &Paths) -> PathBuf {
        paths.config.join("settings.json")
    }

    /// Read the settings, bringing an older file up to the current format (and saving it).
    pub fn load(paths: &Paths) -> Settings {
        let Some(raw) = read_json::<serde_json::Value>(&Self::file(paths)) else {
            return Settings {
                settings_version: SETTINGS_VERSION,
                ..Default::default()
            };
        };
        let (value, changed) = migrate(raw);
        let s: Settings = serde_json::from_value(value).unwrap_or_default();
        if changed {
            let _ = s.save(paths);
        }
        s
    }

    /// Save with a temp file and rename, keeping a rolling copy of the previous file.
    pub fn save(&self, paths: &Paths) -> Result<()> {
        let file = Self::file(paths);
        if file.is_file() {
            let dir = paths.config.join("backups");
            let newest = fs::read_dir(&dir)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| {
                            e.file_name()
                                .to_string_lossy()
                                .starts_with("settings.json.")
                        })
                        .filter_map(|e| e.metadata().ok()?.modified().ok())
                        .max()
                })
                .ok()
                .flatten();
            let due = newest
                .and_then(|t| t.elapsed().ok())
                .is_none_or(|age| age.as_secs() >= SETTINGS_BACKUP_EVERY);
            if due {
                let _ = crate::fsutil::backup(&file, &dir, SETTINGS_BACKUPS_KEPT);
            }
        }
        let mut s = self.clone();
        s.settings_version = SETTINGS_VERSION;
        write_json(&file, &s)
    }
}

/// One step per format version: `MIGRATIONS[n]` turns a version-n file into version n+1.
type Migration = fn(&mut serde_json::Value);
const MIGRATIONS: [Migration; SETTINGS_VERSION as usize] = [
    // 0 → 1 (beta.16): only the version number was added.
    |_| {},
];

/// Bring a settings file of any earlier format up to `SETTINGS_VERSION`. Returns the new value and
/// whether anything changed. Each step handles one version, so old files go through all of them.
pub fn migrate(mut v: serde_json::Value) -> (serde_json::Value, bool) {
    if !v.is_object() {
        v = serde_json::json!({});
    }
    let start = v["settings_version"].as_u64().unwrap_or(0) as u32;
    let mut ver = start;
    while ver < SETTINGS_VERSION {
        MIGRATIONS[ver as usize](&mut v);
        ver += 1;
    }
    v["settings_version"] = serde_json::json!(ver.max(start));
    (v, ver != start)
}

/// A version like `0.1.0` or `0.1.0-beta.2`, compared by the semver rules (a release sorts after
/// its own betas, and beta.10 after beta.2).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub semver::Version);

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        semver::Version::parse(s.trim().trim_start_matches('v'))
            .ok()
            .map(Version)
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
    /// The minisign signature of the checksum file. Without it the release isn't offered.
    pub sig_url: Option<String>,
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
    /// On Stable with nothing newer there: a newer beta, offered with a "switch to Beta" button.
    pub newer_beta: Option<Release>,
    /// GitHub answered, but no CachyPlay release has been published yet.
    pub no_releases: bool,
    /// Channel set to Stable while running a beta that's newer than the newest stable: that
    /// stable version, offered with a "Switch to Stable now" button (it's an older version).
    pub stable_now: Option<Release>,
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
        // New name first; releases before beta.16 only have SHA256SUMS.txt.
        let sums =
            find(&|n: &str| n == "SHA256SUMS").or_else(|| find(&|n: &str| n == "SHA256SUMS.txt"));
        let sig = sums
            .as_ref()
            .and_then(|(name, _)| find(&|n: &str| n == format!("{name}.minisig")));
        out.push(Release {
            tag: tag.to_string(),
            prerelease: r["prerelease"].as_bool().unwrap_or(false),
            notes: r["body"].as_str().unwrap_or("").trim().to_string(),
            page: r["html_url"].as_str().unwrap_or("").to_string(),
            asset: asset.as_ref().map(|a| a.0.clone()),
            asset_url: asset.map(|a| a.1),
            sums_url: sums.map(|s| s.1),
            sig_url: sig.map(|s| s.1),
        });
    }
    Ok(out)
}

/// The newest release on a channel. Beta sees betas and stable releases; Stable only releases.
pub fn pick(releases: &[Release], channel: Channel) -> Option<Release> {
    releases
        .iter()
        .filter(|r| channel == Channel::Beta || !r.prerelease)
        // A release whose build failed has nothing to download; offering it would only fail.
        .filter(|r| r.asset.is_some())
        // Unsigned: not available yet (still being published) or from before signing began.
        .filter(|r| r.sig_url.is_some())
        .filter_map(|r| Version::parse(&r.tag).map(|v| (v, r)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r)| r.clone())
}

/// Running a beta newer than every stable release: the newest stable release, to go back to.
/// None when running a stable version or when a newer stable exists (that's a normal update).
pub fn stable_now(releases: &[Release], current: &str) -> Option<Release> {
    let cur = Version::parse(current)?;
    if cur.0.pre.is_empty() {
        return None;
    }
    let stable = pick(releases, Channel::Stable)?;
    (Version::parse(&stable.tag)? < cur).then_some(stable)
}

/// Run curl; if the system's DNS can't find GitHub, try once more through public DNS over HTTPS.
fn curl(args: &[&str], timeout: Duration) -> Option<(bool, String, String)> {
    let first = sys::run("curl", args, timeout);
    let dns_failed = first
        .as_ref()
        .is_some_and(|(ok, _, err)| !ok && dns_failure(err));
    if !dns_failed {
        return first;
    }
    for doh in DOH_SERVERS {
        let mut with: Vec<&str> = vec!["--doh-url", doh];
        with.extend_from_slice(args);
        let r = sys::run("curl", &with, timeout);
        if r.as_ref().is_some_and(|(ok, _, _)| *ok) {
            return r;
        }
    }
    first
}

/// Public DNS-over-HTTPS servers, tried in order when the system's DNS fails.
const DOH_SERVERS: [&str; 2] = ["https://1.1.1.1/dns-query", "https://9.9.9.9/dns-query"];

/// curl's message when a host name can't be looked up (exit code 6).
pub fn dns_failure(stderr: &str) -> bool {
    stderr.contains("Could not resolve host") || stderr.contains("(6)")
}

fn gh_ready() -> bool {
    sys::run("gh", &["auth", "status"], NET_TIMEOUT).is_some_and(|(ok, _, _)| ok)
}

/// Releases, from a copy at most a day old unless `force` (the user pressed "Check for updates").
/// GitHub allows 60 requests an hour without signing in, so the background check stays polite.
fn releases_cached(paths: &Paths, force: bool) -> Result<Vec<Release>> {
    let cache = paths.state.join("releases.json");
    if !force && std::env::var_os("CACHYPLAY_RELEASES_FILE").is_none() {
        let fresh = fs::metadata(&cache)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < std::time::Duration::from_secs(20 * 3600));
        if fresh {
            if let Ok(r) = fs::read_to_string(&cache)
                .map_err(anyhow::Error::from)
                .and_then(|t| parse_releases(&t))
            {
                return Ok(r);
            }
        }
    }
    let raw = fetch_raw()?;
    let releases = parse_releases(&raw)?;
    if std::env::var_os("CACHYPLAY_RELEASES_FILE").is_none() {
        let _ = fs::create_dir_all(&paths.state);
        let _ = crate::fsutil::atomic_write(&cache, raw.as_bytes());
    }
    Ok(releases)
}

/// GitHub's answer, as text.
fn fetch_raw() -> Result<String> {
    // Tests and development: read a saved API response instead of asking GitHub.
    if let Some(p) = std::env::var_os("CACHYPLAY_RELEASES_FILE") {
        return Ok(fs::read_to_string(p)?);
    }
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=30");
    if let Some((true, body, _)) = curl(
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
        return Ok(body);
    }
    // Private repository (or GitHub unreachable via curl): try the signed-in GitHub CLI.
    if let Some((true, body, _)) = sys::run(
        "gh",
        &["api", &format!("repos/{REPO}/releases?per_page=30")],
        NET_TIMEOUT,
    ) {
        return Ok(body);
    }
    if sys::which("curl").is_none() {
        bail!("curl is not installed, so CachyPlay can't check for updates.");
    }
    bail!("Couldn't reach GitHub. Check your internet connection. (While the repository is private, updates need the GitHub CLI `gh` signed in.)")
}

pub fn check(paths: &Paths) -> UpdateStatus {
    check_with(paths, true)
}

/// The startup check: uses a copy of GitHub's answer up to a day old.
pub fn check_background(paths: &Paths) -> UpdateStatus {
    check_with(paths, false)
}

fn check_with(paths: &Paths, force: bool) -> UpdateStatus {
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
        newer_beta: None,
        no_releases: false,
        stable_now: None,
    };
    match releases_cached(paths, force) {
        Ok(releases) => {
            let latest = pick(&releases, settings.channel);
            let cur = Version::parse(crate::VERSION);
            let newer = |r: &Option<Release>| match (r, &cur) {
                (Some(r), Some(c)) => Version::parse(&r.tag).is_some_and(|v| v > *c),
                _ => false,
            };
            status.available = newer(&latest);
            if !status.available && settings.channel == Channel::Stable {
                let beta = pick(&releases, Channel::Beta);
                if newer(&beta) {
                    status.newer_beta = beta;
                }
            }
            if !status.available && settings.channel == Channel::Stable {
                status.stable_now = stable_now(&releases, crate::VERSION);
            }
            status.no_releases = releases.is_empty();
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
    let sums_name = release
        .sums_url
        .as_deref()
        .and_then(|u| u.rsplit('/').next())
        .filter(|n| *n == "SHA256SUMS.txt")
        .unwrap_or("SHA256SUMS");
    let sums_path = dir.join(sums_name);
    if release.sig_url.is_none() {
        bail!("This release isn't signed yet, so it isn't available. Try again in a few minutes.");
    }

    let curl_ok = |url: &Option<String>, to: &Path| {
        url.as_deref().is_some_and(|u| {
            curl(
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
    let got = curl_ok(&release.asset_url, &asset_path)
        && curl_ok(&release.sums_url, &sums_path)
        && curl_ok(&release.sig_url, &sig_path(&sums_path));
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
                &format!("{sums_name}*"),
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

/// The public keys releases are signed with (packaging/minisign.pub): the primary key used by
/// GitHub Actions and an offline backup key. A signature from either is accepted.
const SIGNING_KEYS: &str = include_str!("../../../packaging/minisign.pub");

/// The key lines (base64) in a minisign public key file; comment lines are skipped.
pub fn key_lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
        .collect()
}

/// The signature file next to a checksum file: `SHA256SUMS` → `SHA256SUMS.minisig`.
pub fn sig_path(sums: &Path) -> PathBuf {
    let mut s = sums.as_os_str().to_owned();
    s.push(".minisig");
    PathBuf::from(s)
}

/// Check the checksum file's minisign signature against every embedded key; one must match.
/// With no keys at all (only possible in tests) nothing is checked.
pub fn verify_signature(keys: &str, sums_text: &str, sig_path: &Path) -> Result<()> {
    let keys: Vec<minisign_verify::PublicKey> = key_lines(keys)
        .into_iter()
        .map(|k| {
            minisign_verify::PublicKey::from_base64(k)
                .map_err(|e| anyhow::anyhow!("CachyPlay's signing key is invalid: {e}"))
        })
        .collect::<Result<_>>()?;
    if keys.is_empty() {
        return Ok(());
    }
    let sig_text = fs::read_to_string(sig_path)
        .context("This update isn't signed, so CachyPlay won't install it.")?;
    let sig = minisign_verify::Signature::decode(&sig_text)
        .map_err(|e| anyhow::anyhow!("The update's signature is unreadable: {e}"))?;
    if keys
        .iter()
        .any(|pk| pk.verify(sums_text.as_bytes(), &sig, false).is_ok())
    {
        return Ok(());
    }
    bail!("The update's signature doesn't match. It may have been tampered with; nothing was installed.")
}

/// The signing keys built into this copy of CachyPlay.
pub fn embedded_keys() -> &'static str {
    SIGNING_KEYS
}

/// Verify and install from an already-downloaded release archive.
pub fn install_from_files(paths: &Paths, archive: &Path, sums: &Path, tag: &str) -> Result<String> {
    install_from_files_with(paths, archive, sums, tag, SIGNING_KEYS)
}

fn install_from_files_with(
    paths: &Paths,
    archive: &Path,
    sums: &Path,
    tag: &str,
    keys: &str,
) -> Result<String> {
    let name = archive
        .file_name()
        .context("bad archive path")?
        .to_string_lossy()
        .into_owned();
    let sums_text = fs::read_to_string(sums).context("checksum file missing")?;
    verify_signature(keys, &sums_text, &sig_path(sums))?;
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
    // The new build checks itself (starts, finds its files, reads settings) before it replaces
    // this one. No network and no other programs, so it takes a few seconds at most.
    let mut to_test = vec![new_cli.clone()];
    if let Some(g) = &new_gui {
        fs::set_permissions(g, fs::Permissions::from_mode(0o755))?;
        to_test.push(g.clone());
    }
    for bin in to_test {
        let ok = sys::run(
            &bin.to_string_lossy(),
            &["--self-test", "--quick"],
            NET_TIMEOUT,
        )
        .is_some_and(|(ok, _, _)| ok);
        if !ok {
            bail!(
                "The new version failed its own check, so it wasn't installed. You still have {}.",
                crate::VERSION
            );
        }
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
    fn rel(tag: &str, asset: bool) -> Release {
        Release {
            tag: tag.into(),
            prerelease: tag.contains('-'),
            notes: String::new(),
            page: String::new(),
            asset: asset.then(|| format!("cachyplay-{tag}-x86_64-linux.tar.zst")),
            asset_url: None,
            sums_url: None,
            sig_url: Some("https://x/SHA256SUMS.minisig".into()),
        }
    }

    #[test]
    fn unsigned_releases_are_not_available_yet() {
        let mut unsigned = rel("v0.1.0-beta.16", true);
        unsigned.sig_url = None;
        let r = vec![rel("v0.1.0-beta.15", true), unsigned];
        assert_eq!(pick(&r, Channel::Beta).unwrap().tag, "v0.1.0-beta.15");
    }

    #[test]
    fn both_signing_keys_are_built_in() {
        let keys = key_lines(embedded_keys());
        assert_eq!(keys.len(), 2, "primary and backup");
        for k in keys {
            minisign_verify::PublicKey::from_base64(k).expect("valid key");
        }
    }

    #[test]
    fn a_tampered_or_unsigned_update_is_refused() {
        if sys::which("zstd").is_none() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        let bin = p.user_bin();
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        fs::write(&bin, "old-cli").unwrap();
        // Unsigned: refused.
        let (archive, sums) = fake_release(d.path(), "v0.2.0", false);
        let e = install_from_files(&p, &archive, &sums, "v0.2.0").unwrap_err();
        assert!(e.to_string().contains("isn't signed"), "{e}");
        // Signed by some other key: refused.
        let other = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/signing/SHA256SUMS.txt.minisig");
        fs::copy(other, sig_path(&sums)).unwrap();
        let e = install_from_files(&p, &archive, &sums, "v0.2.0").unwrap_err();
        assert!(e.to_string().contains("doesn't match"), "{e}");
        assert_eq!(
            fs::read_to_string(&bin).unwrap(),
            "old-cli",
            "nothing changed"
        );
    }

    #[test]
    fn settings_files_are_migrated_and_backed_up() {
        let (v, changed) = migrate(serde_json::json!({"channel": "beta"}));
        assert!(changed);
        assert_eq!(v["settings_version"], SETTINGS_VERSION);
        assert_eq!(v["channel"], "beta");
        let (_, again) = migrate(v);
        assert!(!again, "already current");
        // A newer file than this version knows is left as it is.
        let (v, changed) = migrate(serde_json::json!({"settings_version": 99}));
        assert!(!changed);
        assert_eq!(v["settings_version"], 99);

        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        fs::create_dir_all(&p.config).unwrap();
        fs::write(p.config.join("settings.json"), r#"{"channel":"stable"}"#).unwrap();
        let s = Settings::load(&p);
        assert_eq!(s.channel, Channel::Stable);
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(p.config.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            saved["settings_version"], SETTINGS_VERSION,
            "migrated file saved"
        );
        let backups = fs::read_dir(p.config.join("backups")).unwrap().count();
        assert_eq!(backups, 1, "old file kept before the first save");
        s.save(&p).unwrap();
        assert_eq!(
            fs::read_dir(p.config.join("backups")).unwrap().count(),
            1,
            "at most every 15 minutes"
        );
    }

    #[test]
    fn recognises_dns_failures() {
        assert!(dns_failure(
            "curl: (6) Could not resolve host: api.github.com"
        ));
        assert!(!dns_failure(
            "curl: (22) The requested URL returned error: 404"
        ));
    }

    #[test]
    fn skips_releases_without_a_download() {
        // beta.9's build failed and it was published empty: never offer it.
        let r = vec![rel("v0.1.0-beta.8", true), rel("v0.1.0-beta.9", false)];
        assert_eq!(pick(&r, Channel::Beta).unwrap().tag, "v0.1.0-beta.8");
    }

    #[test]
    fn switch_to_stable_now() {
        let r = vec![rel("v0.1.0", true), rel("v0.1.1-beta.2", true)];
        // On the newer beta with Stable chosen: offer to go back to 0.1.0.
        assert_eq!(stable_now(&r, "0.1.1-beta.2").unwrap().tag, "v0.1.0");
        // On a beta of 0.1.0: 0.1.0 is simply newer, a normal update, not this.
        assert!(stable_now(&r, "0.1.0-beta.11").is_none());
        // On stable already: nothing.
        assert!(stable_now(&r, "0.1.0").is_none());
        // No stable release yet: nothing to switch to.
        assert!(stable_now(&[rel("v0.1.0-beta.10", true)], "0.1.0-beta.10").is_none());
    }

    #[test]
    fn checks_update_signatures_when_a_key_is_set() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signing");
        let key = fs::read_to_string(dir.join("minisign.pub")).unwrap();
        let sums = fs::read_to_string(dir.join("SHA256SUMS.txt")).unwrap();
        let sig = dir.join("SHA256SUMS.txt.minisig");
        verify_signature(&key, &sums, &sig).unwrap();
        // A changed checksum file is refused.
        let e = verify_signature(&key, &sums.replace("abc123", "evil99"), &sig).unwrap_err();
        assert!(e.to_string().contains("doesn't match"), "{e}");
        // No signature with a key set: refused.
        assert!(verify_signature(&key, &sums, &dir.join("missing.minisig")).is_err());
        // Either of several keys is accepted.
        let both = format!("{}\n{key}", embedded_keys());
        verify_signature(&both, &sums, &sig).unwrap();
        // Only the built-in keys: this test signature isn't theirs.
        assert!(verify_signature(embedded_keys(), &sums, &sig).is_err());
    }

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
        assert!(v("0.1.0-beta.15") > v("0.1.0-beta.9"));
        assert!(v("0.1.0-rc.1") > v("0.1.0-beta.20"));
    }

    const API: &str = r#"[
      {"tag_name":"v0.2.0-beta.1","prerelease":true,"draft":false,"body":"beta notes","html_url":"https://x/b",
       "assets":[{"name":"cachyplay-v0.2.0-beta.1-x86_64-linux.tar.zst","browser_download_url":"https://x/b.tar.zst"},
                 {"name":"SHA256SUMS","browser_download_url":"https://x/bsums"},
                 {"name":"SHA256SUMS.minisig","browser_download_url":"https://x/bsums.minisig"},
                 {"name":"SHA256SUMS.txt","browser_download_url":"https://x/old"}]},
      {"tag_name":"v0.1.0","prerelease":false,"draft":false,"body":"stable notes","html_url":"https://x/s",
       "assets":[{"name":"cachyplay-v0.1.0-x86_64-linux.tar.zst","browser_download_url":"https://x/s.tar.zst"},
                 {"name":"SHA256SUMS.txt","browser_download_url":"https://x/ssums"},
                 {"name":"SHA256SUMS.txt.minisig","browser_download_url":"https://x/ssums.minisig"}]},
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
        assert_eq!(
            beta.sums_url.as_deref(),
            Some("https://x/bsums"),
            "new name preferred"
        );
        assert_eq!(beta.sig_url.as_deref(), Some("https://x/bsums.minisig"));
        let stable = pick(&rs, Channel::Stable).unwrap();
        assert_eq!(
            stable.sig_url.as_deref(),
            Some("https://x/ssums.minisig"),
            "old name still read"
        );
    }

    #[test]
    fn settings_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        assert_eq!(Settings::load(&p).channel, default_channel());
        Settings {
            channel: Channel::Beta,
            ..Default::default()
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
        let msg = install_from_files_with(&p, &archive, &sums, "v0.2.0-beta.1", "").unwrap();
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
        assert!(install_from_files_with(&p, &archive, &sums, "v0.2.0", "")
            .unwrap_err()
            .to_string()
            .contains("checksum"));
        let (archive, sums) = fake_release(&d.path().join("b"), "v0.2.0", false);
        assert!(
            install_from_files_with(&p, &archive, &sums, "v0.3.0", "").is_err(),
            "version mismatch is refused"
        );
        assert_eq!(
            fs::read_to_string(&bin).unwrap(),
            "old-cli",
            "nothing changed"
        );
    }
}
