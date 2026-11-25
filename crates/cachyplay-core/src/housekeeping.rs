//! Keeping CachyPlay's own files small and safe across versions: trims old launch records and
//! logs so they never fill a drive, and keeps a copy of the settings folder before a new version
//! first touches it. Bug reports on the Desktop are the user's files and are never deleted.

use crate::paths::Paths;
use std::fs;

/// Launch records kept (newest first); older ones are deleted.
pub const MAX_LAUNCHES: usize = 400;
/// CachyPlay's own log is rotated past this size.
pub const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// Settings backups kept from earlier versions.
pub const KEEP_CONFIG_BACKUPS: usize = 3;

/// Trim launch records and rotate the log. Best-effort; never fails.
pub fn trim(paths: &Paths) {
    let mut files: Vec<_> = fs::read_dir(paths.launches())
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    if files.len() > MAX_LAUNCHES {
        // Names end in a sortable timestamp; sort by that, not by AppID.
        files.sort_by_key(|p| {
            p.file_name()
                .map(|n| {
                    let n = n.to_string_lossy();
                    n.split_once('-')
                        .map(|(_, t)| t.to_string())
                        .unwrap_or_default()
                })
                .unwrap_or_default()
        });
        for p in &files[..files.len() - MAX_LAUNCHES] {
            let _ = fs::remove_file(p);
        }
    }
    let log = paths.log_file();
    if fs::metadata(&log).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = fs::rename(&log, log.with_extension("log.old"));
    }
}

/// Before this version first uses the settings folder, keep a copy of it as written by the
/// previous version, so an update can never lose someone's fixes or settings.
pub fn protect_settings(paths: &Paths) {
    let marker = paths.config.join("version");
    let previous = fs::read_to_string(&marker).unwrap_or_default();
    let previous = previous.trim();
    if previous == crate::VERSION {
        return;
    }
    if !previous.is_empty() && paths.config.is_dir() {
        let backup = paths
            .config
            .with_file_name(format!("cachyplay-config-before-{}", crate::VERSION));
        if !backup.exists() && copy_dir(&paths.config, &backup).is_ok() {
            prune_config_backups(paths);
        }
    }
    let _ = fs::create_dir_all(&paths.config);
    let _ = fs::write(&marker, crate::VERSION);
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)?.flatten() {
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

fn prune_config_backups(paths: &Paths) {
    let Some(parent) = paths.config.parent() else {
        return;
    };
    let mut backups: Vec<(std::time::SystemTime, std::path::PathBuf)> = fs::read_dir(parent)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("cachyplay-config-before-")
                })
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .collect()
        })
        .unwrap_or_default();
    backups.sort_by_key(|b| std::cmp::Reverse(b.0));
    for (_, p) in backups.iter().skip(KEEP_CONFIG_BACKUPS) {
        let _ = fs::remove_dir_all(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_launch_records_to_the_newest() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::isolated(tmp.path());
        fs::create_dir_all(p.launches()).unwrap();
        for i in 0..(MAX_LAUNCHES + 5) {
            // Different AppIDs, so sorting must use the timestamp part.
            let app = if i % 2 == 0 { "9" } else { "10" };
            fs::write(
                p.launches()
                    .join(format!("{app}-2026-01-01_00-00-{i:05}.json")),
                "{}",
            )
            .unwrap();
        }
        trim(&p);
        let left: Vec<String> = fs::read_dir(p.launches())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), MAX_LAUNCHES);
        assert!(
            !left.iter().any(|n| n.ends_with("-00000.json")),
            "oldest removed"
        );
        assert!(left
            .iter()
            .any(|n| n.ends_with(&format!("-{:05}.json", MAX_LAUNCHES + 4))));
    }

    #[test]
    fn keeps_a_copy_of_settings_from_the_previous_version() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::isolated(tmp.path());
        fs::create_dir_all(&p.config).unwrap();
        fs::write(p.config.join("version"), "0.1.0-beta.1").unwrap();
        fs::write(p.config.join("settings.json"), "{\"channel\":\"beta\"}").unwrap();
        protect_settings(&p);
        let backup = p
            .config
            .with_file_name(format!("cachyplay-config-before-{}", crate::VERSION));
        assert!(backup.join("settings.json").is_file());
        assert_eq!(
            fs::read_to_string(p.config.join("version")).unwrap(),
            crate::VERSION
        );
        // Runs once per version.
        fs::remove_dir_all(&backup).unwrap();
        protect_settings(&p);
        assert!(!backup.exists());
    }
}
