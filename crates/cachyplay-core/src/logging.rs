// SPDX-License-Identifier: GPL-3.0-or-later
//! CachyPlay's own log, ~/.local/state/cachyplay/cachyplay.log: at most 1 MB per file and 3 files
//! kept (`cachyplay.log`, `.log.1`, `.log.2`), plus the last ~500 lines of this session in memory
//! so a bug report can include them even when the file was just rotated.

use crate::paths::Paths;
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub const MAX_BYTES: u64 = 1024 * 1024;
pub const FILES_KEPT: usize = 3;
pub const BUFFER_LINES: usize = 500;

static BUFFER: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// Add one line to the log (and the in-memory buffer). Best-effort; never fails.
pub fn write(paths: &Paths, msg: &str) {
    let line = format!("{} {msg}", crate::manage::now());
    remember(&line);
    let _ = fs::create_dir_all(&paths.state);
    rotate(paths);
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.log_file())
    {
        let _ = writeln!(f, "{line}");
    }
}

fn remember(line: &str) {
    if let Ok(mut b) = BUFFER.lock() {
        if b.len() >= BUFFER_LINES {
            b.pop_front();
        }
        b.push_back(line.to_string());
    }
}

/// The lines logged by this program since it started (newest last, at most `BUFFER_LINES`).
pub fn recent() -> Vec<String> {
    BUFFER
        .lock()
        .map(|b| b.iter().cloned().collect())
        .unwrap_or_default()
}

/// `cachyplay.log.N`
pub fn numbered(paths: &Paths, n: usize) -> PathBuf {
    let mut s = paths.log_file().into_os_string();
    s.push(format!(".{n}"));
    PathBuf::from(s)
}

/// Move a full log aside: .log → .log.1 → .log.2, dropping the oldest.
pub fn rotate(paths: &Paths) {
    let log = paths.log_file();
    // The single old copy kept before beta.16.
    let legacy = log.with_extension("log.old");
    if legacy.is_file() && !numbered(paths, 1).exists() {
        let _ = fs::rename(&legacy, numbered(paths, 1));
    } else if legacy.is_file() {
        let _ = fs::remove_file(&legacy);
    }
    if !fs::metadata(&log).is_ok_and(|m| m.len() > MAX_BYTES) {
        return;
    }
    let _ = fs::remove_file(numbered(paths, FILES_KEPT - 1));
    for n in (1..FILES_KEPT - 1).rev() {
        let _ = fs::rename(numbered(paths, n), numbered(paths, n + 1));
    }
    let _ = fs::rename(&log, numbered(paths, 1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_at_one_megabyte_and_keeps_three_files() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        fs::create_dir_all(&p.state).unwrap();
        fs::write(p.log_file().with_extension("log.old"), "legacy").unwrap();
        for round in 0..4 {
            fs::write(p.log_file(), vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
            write(&p, &format!("round {round}"));
        }
        assert!(p.log_file().is_file());
        assert!(numbered(&p, 1).is_file());
        assert!(numbered(&p, 2).is_file());
        assert!(!numbered(&p, 3).exists(), "only three files");
        assert!(!p.log_file().with_extension("log.old").exists());
        let current = fs::read_to_string(p.log_file()).unwrap();
        assert!(current.ends_with("round 3\n"), "{current}");
        assert!(recent().iter().any(|l| l.ends_with("round 3")));
    }

    #[test]
    fn memory_buffer_is_capped() {
        for i in 0..(BUFFER_LINES + 20) {
            remember(&format!("line {i}"));
        }
        let r = recent();
        assert!(r.len() <= BUFFER_LINES);
        assert!(r
            .last()
            .unwrap()
            .ends_with(&format!("line {}", BUFFER_LINES + 19)));
    }
}
