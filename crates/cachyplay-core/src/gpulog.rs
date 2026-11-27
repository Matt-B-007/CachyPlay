// SPDX-License-Identifier: GPL-3.0-or-later
//! The graphics card's temperature, power and clock speeds every 2 seconds while a game runs,
//! so a bug report can say "the card was cool and steady when it hung" without the player running
//! anything in a terminal. NVIDIA through `nvidia-smi`, AMD through the kernel's sensor files.
//! Best effort: if neither is available, nothing is recorded and the game isn't affected.

use crate::paths::Paths;
use chrono::NaiveDateTime;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const KEEP: usize = 20;
const HEADER: &str = "timestamp, temperature.gpu, power.draw [W], clocks.current.graphics [MHz], clocks.current.memory [MHz]\n";
const TS: &str = "%Y/%m/%d %H:%M:%S%.3f";

pub struct Recorder {
    stop: Arc<AtomicBool>,
}

impl Recorder {
    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn dir(paths: &Paths) -> PathBuf {
    paths.state.join("gpu-logs")
}

fn file_for(paths: &Paths, app_id: &str, started_at: &str) -> PathBuf {
    let stamp: String = started_at
        .chars()
        .take(19)
        .map(|c| if c == ':' { '-' } else { c })
        .collect();
    dir(paths).join(format!("{app_id}-{stamp}.csv"))
}

fn prune(paths: &Paths) {
    let mut files: Vec<PathBuf> = fs::read_dir(dir(paths))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    if files.len() > KEEP {
        for f in &files[..files.len() - KEEP] {
            let _ = fs::remove_file(f);
        }
    }
}

fn on_path(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
        .unwrap_or(false)
}

/// Start recording for one game start. Returns None when there's nothing to read the card with.
pub fn start(paths: &Paths, app_id: &str, started_at: &str) -> Option<Recorder> {
    if std::env::var_os("CACHYPLAY_NO_GPU_LOG").is_some() {
        return None;
    }
    let _ = fs::create_dir_all(dir(paths));
    prune(paths);
    let path = file_for(paths, app_id, started_at);
    let gpus = crate::gpus::list(&crate::pccheck::Root::system());
    let card = gpus
        .iter()
        .find(|g| !g.integrated)
        .or(gpus.first())?
        .clone();
    let stop = Arc::new(AtomicBool::new(false));
    if card.vendor == "nvidia" && on_path("nvidia-smi") {
        // One short query every 2 seconds from a thread, not a long-running nvidia-smi: if the
        // launch helper is stopped, nothing is left running.
        let flag = stop.clone();
        std::thread::Builder::new()
            .name("gpulog".into())
            .spawn(move || {
                let Ok(mut f) = fs::File::create(&path) else {
                    return;
                };
                let _ = f.write_all(HEADER.as_bytes());
                for _ in 0..(6 * 3600 / 2) {
                    if flag.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Ok(out) = Command::new("nvidia-smi")
                        .args([
                            "--query-gpu=timestamp,temperature.gpu,power.draw,clocks.gr,clocks.mem",
                            "--format=csv,noheader",
                            "--id=0",
                        ])
                        .stdin(Stdio::null())
                        .stderr(Stdio::null())
                        .output()
                    {
                        let _ = f.write_all(&out.stdout);
                    }
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            })
            .ok()?;
        return Some(Recorder { stop });
    }
    // AMD (and Intel cards with hwmon): read the sensor files ourselves.
    let hw = fs::read_dir(
        Path::new("/sys/class/drm")
            .join(&card.card)
            .join("device/hwmon"),
    )
    .ok()?
    .flatten()
    .next()?
    .path();
    let flag = stop.clone();
    std::thread::Builder::new()
        .name("gpulog".into())
        .spawn(move || {
            let Ok(mut f) = fs::File::create(&path) else {
                return;
            };
            let _ = f.write_all(HEADER.as_bytes());
            let num = |name: &str| {
                fs::read_to_string(hw.join(name))
                    .ok()
                    .and_then(|v| v.trim().parse::<f64>().ok())
            };
            for _ in 0..(6 * 3600 / 2) {
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                let line = format!(
                    "{}, {:.0}, {:.2} W, {:.0} MHz, {:.0} MHz\n",
                    chrono::Local::now().format(TS),
                    num("temp1_input").unwrap_or(0.0) / 1000.0,
                    num("power1_average")
                        .or_else(|| num("power1_input"))
                        .unwrap_or(0.0)
                        / 1e6,
                    num("freq1_input").unwrap_or(0.0) / 1e6,
                    num("freq2_input").unwrap_or(0.0) / 1e6,
                );
                let _ = f.write_all(line.as_bytes());
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        })
        .ok()?;
    Some(Recorder { stop })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub at: NaiveDateTime,
    pub temp: f64,
    pub watts: f64,
    pub clock: f64,
}

pub fn parse(text: &str) -> Vec<Sample> {
    text.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split(',').map(str::trim).collect();
            let at = NaiveDateTime::parse_from_str(c.first()?, TS).ok()?;
            let n = |s: &str| s.split_whitespace().next()?.parse::<f64>().ok();
            Some(Sample {
                at,
                temp: n(c.get(1)?)?,
                watts: n(c.get(2)?)?,
                clock: n(c.get(3)?)?,
            })
        })
        .collect()
}

/// The newest GPU log for a game, if any.
pub fn latest(paths: &Paths, app_id: &str) -> Option<(PathBuf, String)> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir(paths))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(&format!("{app_id}-")))
        })
        .collect();
    files.sort();
    let p = files.pop()?;
    let t = fs::read_to_string(&p).ok()?;
    Some((p, t))
}

/// One plain sentence about the card, ideally around a crash.
pub fn describe(samples: &[Sample], crash_at: Option<NaiveDateTime>) -> Option<String> {
    let s: Vec<&Sample> = samples
        .iter()
        .filter(|x| x.watts > 0.0 || x.temp > 0.0)
        .collect();
    if s.len() < 3 {
        return None;
    }
    let max_t = s.iter().map(|x| x.temp).fold(0.0, f64::max);
    let max_w = s.iter().map(|x| x.watts).fold(0.0, f64::max);
    let mut out =
        format!("During the start the card reached at most {max_t:.0} °C and {max_w:.0} W.");
    if let Some(c) = crash_at {
        let before: Vec<&&Sample> = s
            .iter()
            .filter(|x| x.at <= c && x.at >= c - chrono::Duration::seconds(30))
            .collect();
        if let (Some(first), Some(last)) = (before.first(), before.last()) {
            let hot = before.iter().any(|x| x.temp >= 85.0);
            let stalled = last.watts < first.watts * 0.6 && last.clock >= first.clock * 0.9;
            out.push_str(&format!(
                " In the 30 seconds before the crash: {:.0}–{:.0} °C, {:.0}–{:.0} W.",
                before.iter().map(|x| x.temp).fold(f64::MAX, f64::min),
                before.iter().map(|x| x.temp).fold(0.0, f64::max),
                before.iter().map(|x| x.watts).fold(f64::MAX, f64::min),
                before.iter().map(|x| x.watts).fold(0.0, f64::max),
            ));
            out.push_str(if hot {
                " The card was hot; check its cooling and fans."
            } else if stalled {
                " Power fell while the clock stayed high: the card was waiting on stuck work, not overheating or short of power."
            } else {
                " It was cool and steady, so overheating or power is unlikely."
            });
        }
    }
    Some(out)
}

/// The newest MangoHud recording written after `since` ("Record performance").
pub fn latest_performance(
    paths: &Paths,
    since: Option<NaiveDateTime>,
) -> Option<(PathBuf, String)> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> =
        fs::read_dir(paths.state.join("performance"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "csv"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
    files.sort();
    let (t, p) = files.pop()?;
    let written = chrono::DateTime::<chrono::Local>::from(t).naive_local();
    if since.is_some_and(|s| written < s) {
        return None;
    }
    Some((p.clone(), fs::read_to_string(&p).ok()?))
}

/// "Average 142 FPS, 1% low 61 FPS, 23 hitches over 50 ms (worst 410 ms)."
pub fn performance_summary(mangohud_csv: &str) -> Option<String> {
    let mut lines = mangohud_csv.lines();
    let header = lines.by_ref().find(|l| l.starts_with("fps,"))?;
    let cols: Vec<&str> = header.split(',').collect();
    let fi = cols.iter().position(|c| *c == "fps")?;
    let ti = cols.iter().position(|c| *c == "frametime")?;
    let mut fps = Vec::new();
    let mut ft = Vec::new();
    for l in lines {
        let v: Vec<&str> = l.split(',').collect();
        if let (Some(f), Some(t)) = (
            v.get(fi).and_then(|x| x.parse::<f64>().ok()),
            v.get(ti).and_then(|x| x.parse::<f64>().ok()),
        ) {
            fps.push(f);
            ft.push(t);
        }
    }
    if ft.len() < 10 {
        return None;
    }
    let avg = fps.iter().sum::<f64>() / fps.len() as f64;
    let mut sorted = ft.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p99 = sorted[(sorted.len() * 99 / 100).min(sorted.len() - 1)];
    let hitches = ft.iter().filter(|t| **t > 50.0).count();
    let worst = sorted.last().copied().unwrap_or(0.0);
    Some(format!(
        "Average {avg:.0} FPS, 1% low {:.0} FPS, {hitches} hitch{} over 50 ms (worst {worst:.0} ms).",
        1000.0 / p99.max(0.1),
        if hitches == 1 { "" } else { "es" }
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn sums_up_a_mangohud_recording() {
        let mut csv = String::from("os,cpu,gpu\nLinux,9950X3D,RTX 5090\nfps,frametime,cpu_load\n");
        for _ in 0..98 {
            csv.push_str("144,6.9,20\n");
        }
        csv.push_str("10,100,20\n12,80,20\n");
        let s = performance_summary(&csv).unwrap();
        assert!(s.starts_with("Average 141 FPS"), "{s}");
        assert!(s.contains("2 hitches over 50 ms (worst 100 ms)"), "{s}");
    }

    use super::*;

    const MATT: &str = "timestamp, temperature.gpu, power.draw [W], clocks.current.graphics [MHz], clocks.current.memory [MHz], pstate
2026/11/25 17:29:13.525, 57.84, 234.65 W, 2910 MHz, 14001 MHz, P0
2026/11/25 17:29:15.525, 57.78, 235.61 W, 2910 MHz, 14001 MHz, P0
2026/11/25 17:29:17.525, 58.45, 246.91 W, 2902 MHz, 14001 MHz, P0
2026/11/25 17:29:19.526, 52.27, 100.15 W, 2550 MHz, 14001 MHz, P0
2026/11/25 17:29:21.526, 57.49, 208.82 W, 2902 MHz, 14001 MHz, P0
2026/11/25 17:29:25.526, 53.93, 172.99 W, 2940 MHz, 14001 MHz, P0
2026/11/25 17:29:27.526, 53.38, 117.43 W, 2940 MHz, 14001 MHz, P0
2026/11/25 17:29:29.527, 52.32, 113.23 W, 2940 MHz, 14001 MHz, P0
";

    #[test]
    fn reads_the_log_from_halo_and_explains_it() {
        let s = parse(MATT);
        assert_eq!(s.len(), 8);
        let crash =
            NaiveDateTime::parse_from_str("2026-11-25 17:29:29", "%Y-%m-%d %H:%M:%S").unwrap();
        let d = describe(&s, Some(crash)).unwrap();
        assert!(d.contains("at most 58 °C and 247 W"), "{d}");
        assert!(d.contains("waiting on stuck work"), "{d}");
    }

    #[test]
    fn keeps_the_newest_logs_only() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::isolated(t.path());
        fs::create_dir_all(dir(&p)).unwrap();
        for i in 0..25 {
            fs::write(dir(&p).join(format!("1-{i:03}.csv")), "x").unwrap();
        }
        prune(&p);
        assert_eq!(fs::read_dir(dir(&p)).unwrap().count(), KEEP);
        assert!(latest(&p, "1").unwrap().0.ends_with("1-024.csv"));
    }
}
