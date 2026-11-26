//! Graphics chips: which ones the PC has, which one a running game uses, and how to point games
//! at the graphics card instead of the weaker chip built into the processor or motherboard.
//!
//! Everything here reads files under `/sys`, `/proc` and `/dev` through [`Root`], so tests can
//! build a fake PC in a temporary folder.

use crate::manage::{read_json, write_json};
use crate::paths::Paths;
use crate::pccheck::Root;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;

/// AMD chips with less video memory than this are treated as built into the processor.
const IGPU_VRAM_LIMIT: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Gpu {
    /// "card0"
    pub card: String,
    /// "nvidia", "amd" or "intel"
    pub vendor: &'static str,
    /// PCI address, "0000:01:00.0"
    pub pci: String,
    /// PCI device id without 0x, "2b85"
    pub device_id: String,
    /// "renderD128", the node games open for drawing.
    pub render: Option<String>,
    /// Built into the processor or motherboard.
    pub integrated: bool,
    /// Display outputs with a monitor plugged in, "DP-1".
    pub displays: Vec<String>,
}

impl Gpu {
    pub fn vendor_name(&self) -> &'static str {
        match self.vendor {
            "nvidia" => "NVIDIA",
            "amd" => "AMD",
            _ => "Intel",
        }
    }
    /// "the NVIDIA graphics card" / "the built-in AMD graphics chip"
    pub fn describe(&self) -> String {
        if self.integrated {
            format!("the built-in {} graphics chip", self.vendor_name())
        } else {
            format!("the {} graphics card", self.vendor_name())
        }
    }
    fn vendor_id(&self) -> &'static str {
        match self.vendor {
            "nvidia" => "10de",
            "amd" => "1002",
            _ => "8086",
        }
    }
}

fn hex(s: &str) -> String {
    s.trim().trim_start_matches("0x").to_lowercase()
}

/// The graphics chips in this PC, graphics cards first.
pub fn list(root: &Root) -> Vec<Gpu> {
    let drm = root.p("/sys/class/drm");
    let mut names: Vec<String> = fs::read_dir(&drm)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut out: Vec<Gpu> = Vec::new();
    for card in names
        .iter()
        .filter(|n| n.starts_with("card") && !n.contains('-'))
    {
        let dev = drm.join(card).join("device");
        let vendor = match hex(&fs::read_to_string(dev.join("vendor")).unwrap_or_default()).as_str()
        {
            "10de" => "nvidia",
            "1002" => "amd",
            "8086" => "intel",
            _ => continue,
        };
        let pci = fs::read_link(&dev)
            .ok()
            .and_then(|l| l.file_name().map(|f| f.to_string_lossy().into_owned()))
            .or_else(|| {
                // Fake trees in tests write the address into a file instead of a link.
                fs::read_to_string(dev.join("pci_slot"))
                    .ok()
                    .map(|s| s.trim().to_string())
            })
            .unwrap_or_default();
        let device_id = hex(&fs::read_to_string(dev.join("device")).unwrap_or_default());
        let render = fs::read_dir(dev.join("drm"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|n| n.starts_with("renderD"));
        let integrated = match vendor {
            "nvidia" => false,
            "amd" => fs::read_to_string(dev.join("mem_info_vram_total"))
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .is_none_or(|v| v < IGPU_VRAM_LIMIT),
            // Intel's built-in graphics always sits on the first PCI bus; Arc cards don't.
            _ => pci.starts_with("0000:00:") || pci.is_empty(),
        };
        let displays = names
            .iter()
            .filter(|n| n.starts_with(&format!("{card}-")))
            .filter(|n| {
                fs::read_to_string(drm.join(n).join("status"))
                    .is_ok_and(|s| s.trim() == "connected")
            })
            .map(|n| n[card.len() + 1..].to_string())
            .collect();
        if out.iter().any(|g| g.pci == pci && !pci.is_empty()) {
            continue;
        }
        out.push(Gpu {
            card: card.clone(),
            vendor,
            pci,
            device_id,
            render,
            integrated,
            displays,
        });
    }
    out.sort_by_key(|g| g.integrated);
    out
}

/// True when the PC has both a graphics card and a built-in chip.
pub fn has_both(gpus: &[Gpu]) -> bool {
    gpus.iter().any(|g| g.integrated) && gpus.iter().any(|g| !g.integrated)
}

pub fn is_laptop(root: &Root) -> bool {
    fs::read_dir(root.p("/sys/class/power_supply"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("BAT"))
}

/// Settings that make a game draw on the graphics card, for a PC with two chips. Covers OpenGL,
/// Vulkan, and DirectX games run through DXVK and VKD3D (which use Vulkan underneath).
pub fn prefer_card_env(gpus: &[Gpu]) -> Vec<(String, String)> {
    let Some(card) = gpus.iter().find(|g| !g.integrated) else {
        return vec![];
    };
    if !has_both(gpus) {
        return vec![];
    }
    let mut env: Vec<(&str, String)> = Vec::new();
    if card.vendor == "nvidia" {
        env.push(("__NV_PRIME_RENDER_OFFLOAD", "1".into()));
        env.push(("__VK_LAYER_NV_optimus", "NVIDIA_only".into()));
        env.push(("__GLX_VENDOR_LIBRARY_NAME", "nvidia".into()));
        env.push(("DXVK_FILTER_DEVICE_NAME", "NVIDIA".into()));
        env.push(("VKD3D_FILTER_DEVICE_NAME", "NVIDIA".into()));
    } else {
        // AMD or Intel card next to a built-in chip: Mesa picks by PCI address and device id.
        if !card.pci.is_empty() {
            env.push((
                "DRI_PRIME",
                format!("pci-{}", card.pci.replace([':', '.'], "_")),
            ));
        }
        if !card.device_id.is_empty() {
            env.push((
                "MESA_VK_DEVICE_SELECT",
                format!("{}:{}", card.vendor_id(), card.device_id),
            ));
        }
    }
    env.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// Which graphics chip a process (or any process it started) has open for drawing.
pub fn chip_in_use(root: &Root, pid: u32, gpus: &[Gpu]) -> Option<Gpu> {
    let mut pids = vec![pid];
    pids.extend(descendants(root, pid));
    let mut found: Option<Gpu> = None;
    for p in pids {
        for e in fs::read_dir(root.p(&format!("/proc/{p}/fd")))
            .into_iter()
            .flatten()
            .flatten()
        {
            let Ok(target) = fs::read_link(e.path()) else {
                continue;
            };
            let t = target.to_string_lossy();
            let hit = if let Some(node) = t.strip_prefix("/dev/dri/") {
                gpus.iter()
                    .find(|g| g.render.as_deref() == Some(node) || g.card == node)
            } else if t.starts_with("/dev/nvidia")
                && t[11..].chars().all(|c| c.is_ascii_digit())
                && t.len() > 11
            {
                gpus.iter().find(|g| g.vendor == "nvidia")
            } else {
                None
            };
            if let Some(g) = hit {
                // A game that opens both chips (to list them) draws on the card, if it has one open.
                if found.as_ref().is_none_or(|f| f.integrated && !g.integrated) {
                    found = Some(g.clone());
                }
            }
        }
    }
    found
}

fn descendants(root: &Root, pid: u32) -> Vec<u32> {
    let mut parent_of: Vec<(u32, u32)> = Vec::new();
    for e in fs::read_dir(root.p("/proc"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(p) = e.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let stat = fs::read_to_string(e.path().join("stat")).unwrap_or_default();
        // "pid (name with spaces) S ppid ..."
        if let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) {
            if let Some(ppid) = rest.split_whitespace().nth(1).and_then(|v| v.parse().ok()) {
                parent_of.push((p, ppid));
            }
        }
    }
    let mut out = Vec::new();
    let mut todo = vec![pid];
    while let Some(cur) = todo.pop() {
        for (child, _) in parent_of.iter().filter(|(_, pp)| *pp == cur) {
            if !out.contains(child) && out.len() < 4096 {
                out.push(*child);
                todo.push(*child);
            }
        }
    }
    out
}

/// A note about where the monitor is plugged in, when that costs performance.
pub fn monitor_note(root: &Root, gpus: &[Gpu]) -> Option<(crate::pccheck::Level, String, String)> {
    use crate::pccheck::Level;
    if !has_both(gpus) {
        return None;
    }
    let card = gpus.iter().find(|g| !g.integrated)?;
    let builtin = gpus.iter().find(|g| g.integrated)?;
    if is_laptop(root) {
        return Some((
            Level::Info,
            "Laptop with two graphics chips".into(),
            format!("This is normal for a laptop. CachyPlay points the games it's on for at {} so they don't run on the weaker built-in chip.", card.describe()),
        ));
    }
    if !builtin.displays.is_empty() && card.displays.is_empty() {
        return Some((
            Level::Warn,
            "Monitor plugged into the motherboard".into(),
            format!("Your monitor is connected to the motherboard ({}), not to {}. Games still draw on the card, but every frame then has to be copied to the motherboard, which costs performance and adds delay. Plug the monitor cable into the graphics card instead.", builtin.displays.join(", "), card.describe()),
        ));
    }
    None
}

/// Per-game "use the graphics card" setting. On by default; you can turn it off for a game.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CardSettings {
    /// AppIDs where CachyPlay leaves the choice of chip to the game.
    #[serde(default)]
    pub off: Vec<String>,
}

fn file(paths: &Paths) -> std::path::PathBuf {
    paths.config.join("graphics-card.json")
}

pub fn prefer_card_for(paths: &Paths, app_id: &str) -> bool {
    let s: CardSettings = read_json(&file(paths)).unwrap_or_default();
    !s.off.iter().any(|g| g == app_id)
}

pub fn set_prefer_card(paths: &Paths, app_id: &str, on: bool) -> Result<()> {
    let mut s: CardSettings = read_json(&file(paths)).unwrap_or_default();
    s.off.retain(|g| g != app_id);
    if !on {
        s.off.push(app_id.to_string());
        s.off.sort();
    }
    write_json(&file(paths), &s)
}

/// Settings file path, for uninstall.
pub fn settings_file(paths: &Paths) -> std::path::PathBuf {
    file(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel.trim_start_matches('/'));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    /// A desktop with an RTX card and the Radeon chip built into a Ryzen processor.
    fn ryzen_with_rtx(monitor_on_board: bool) -> (tempfile::TempDir, Root) {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().to_path_buf();
        let r = r.as_path();
        put(r, "/sys/class/drm/card1/device/vendor", "0x10de\n");
        put(r, "/sys/class/drm/card1/device/device", "0x2b85\n");
        put(r, "/sys/class/drm/card1/device/pci_slot", "0000:01:00.0\n");
        put(r, "/sys/class/drm/card1/device/drm/renderD128/x", "");
        put(r, "/sys/class/drm/card2/device/vendor", "0x1002\n");
        put(r, "/sys/class/drm/card2/device/device", "0x13c0\n");
        put(r, "/sys/class/drm/card2/device/pci_slot", "0000:7c:00.0\n");
        put(
            r,
            "/sys/class/drm/card2/device/mem_info_vram_total",
            "536870912\n",
        );
        put(r, "/sys/class/drm/card2/device/drm/renderD129/x", "");
        let (on_card, on_board) = if monitor_on_board {
            ("disconnected", "connected")
        } else {
            ("connected", "disconnected")
        };
        put(r, "/sys/class/drm/card1-DP-1/status", on_card);
        put(r, "/sys/class/drm/card2-HDMI-A-1/status", on_board);
        let root = Root(r.to_path_buf());
        (t, root)
    }

    #[test]
    fn tells_the_card_from_the_built_in_chip() {
        let (_t, root) = ryzen_with_rtx(false);
        let g = list(&root);
        assert_eq!(g.len(), 2);
        assert_eq!((g[0].vendor, g[0].integrated), ("nvidia", false));
        assert_eq!((g[1].vendor, g[1].integrated), ("amd", true));
        assert_eq!(g[0].render.as_deref(), Some("renderD128"));
        assert_eq!(g[0].displays, vec!["DP-1"]);
        assert!(has_both(&g));
        let env = prefer_card_env(&g);
        assert!(env.contains(&("__NV_PRIME_RENDER_OFFLOAD".into(), "1".into())));
        assert!(env.contains(&("DXVK_FILTER_DEVICE_NAME".into(), "NVIDIA".into())));
        assert!(monitor_note(&root, &g).is_none());
    }

    #[test]
    fn amd_card_next_to_intel_chip_uses_mesa_selection() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        put(r, "/sys/class/drm/card0/device/vendor", "0x8086\n");
        put(r, "/sys/class/drm/card0/device/pci_slot", "0000:00:02.0\n");
        put(r, "/sys/class/drm/card1/device/vendor", "0x1002\n");
        put(r, "/sys/class/drm/card1/device/device", "0x744c\n");
        put(r, "/sys/class/drm/card1/device/pci_slot", "0000:03:00.0\n");
        put(
            r,
            "/sys/class/drm/card1/device/mem_info_vram_total",
            "25753026560\n",
        );
        let g = list(&Root(r.to_path_buf()));
        let env = prefer_card_env(&g);
        assert!(
            env.contains(&("DRI_PRIME".into(), "pci-0000_03_00_0".into())),
            "{env:?}"
        );
        assert!(env.contains(&("MESA_VK_DEVICE_SELECT".into(), "1002:744c".into())));
    }

    #[test]
    fn single_card_needs_nothing() {
        let t = tempfile::tempdir().unwrap();
        put(t.path(), "/sys/class/drm/card0/device/vendor", "0x10de\n");
        let g = list(&Root(t.path().to_path_buf()));
        assert!(!has_both(&g));
        assert!(prefer_card_env(&g).is_empty());
    }

    #[test]
    fn warns_about_a_monitor_on_the_motherboard_but_not_on_laptops() {
        let (t, root) = ryzen_with_rtx(true);
        let g = list(&root);
        let (level, title, _) = monitor_note(&root, &g).unwrap();
        assert_eq!(level, crate::pccheck::Level::Warn);
        assert!(title.contains("motherboard"));
        put(
            t.path(),
            "/sys/class/power_supply/BAT0/status",
            "Discharging",
        );
        let (level, _, _) = monitor_note(&root, &g).unwrap();
        assert_eq!(level, crate::pccheck::Level::Info);
    }

    #[test]
    fn finds_the_chip_a_game_draws_on() {
        let (t, root) = ryzen_with_rtx(false);
        let g = list(&root);
        let r = t.path();
        // Steam's wrapper (100) started proton (101), which started the game (102).
        put(r, "/proc/100/stat", "100 (cachyplay) S 1 0");
        put(r, "/proc/101/stat", "101 (proton) S 100 0");
        put(r, "/proc/102/stat", "102 (Moon Sweepers.e) S 101 0");
        fs::create_dir_all(r.join("proc/102/fd")).unwrap();
        symlink("/dev/dri/renderD129", r.join("proc/102/fd/5")).unwrap();
        assert!(chip_in_use(&root, 100, &g).unwrap().integrated);
        symlink("/dev/nvidia0", r.join("proc/102/fd/6")).unwrap();
        assert!(!chip_in_use(&root, 100, &g).unwrap().integrated);
        assert!(chip_in_use(&root, 999, &g).is_none());
    }

    #[test]
    fn per_game_setting() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::isolated(t.path());
        assert!(prefer_card_for(&p, "730"));
        set_prefer_card(&p, "730", false).unwrap();
        assert!(!prefer_card_for(&p, "730"));
        set_prefer_card(&p, "730", true).unwrap();
        assert!(prefer_card_for(&p, "730"));
    }
}
