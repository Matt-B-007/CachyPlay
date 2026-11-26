//! A one-minute check of the PC for the things that most often stop games on Linux, and a
//! self-test of CachyPlay's own setup. Every problem comes with a plain explanation and, where
//! possible, a command to copy. Nothing here changes the system.

use crate::paths::Paths;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Ok,
    /// Worth fixing; some games will have trouble.
    Warn,
    /// Will stop games from working.
    Bad,
    /// Just so you know.
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: String,
    pub title: String,
    pub level: Level,
    pub detail: String,
    /// A command to copy into a terminal, if that's how it's fixed.
    pub command: Option<String>,
}

fn check(
    id: &str,
    title: &str,
    level: Level,
    detail: impl Into<String>,
    command: Option<&str>,
) -> Check {
    Check {
        id: id.into(),
        title: title.into(),
        level,
        detail: detail.into(),
        command: command.map(str::to_string),
    }
}

/// Where to look for /proc, /sys, /dev and /usr (tests point this at a fake tree).
#[derive(Debug, Clone)]
pub struct Root(pub PathBuf);

impl Root {
    pub fn system() -> Root {
        Root(
            std::env::var_os("CACHYPLAY_SYS_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/")),
        )
    }
    pub fn p(&self, abs: &str) -> PathBuf {
        self.0.join(abs.trim_start_matches('/'))
    }
    pub fn read(&self, abs: &str) -> Option<String> {
        fs::read_to_string(self.p(abs)).ok()
    }
    pub fn exists(&self, abs: &str) -> bool {
        self.p(abs).exists()
    }
}

/// The filesystem type of the mount that holds `path`, from /proc/mounts.
pub fn fs_type(root: &Root, path: &Path) -> Option<String> {
    let mounts = root.read("/proc/mounts")?;
    let path = path.to_string_lossy();
    mounts
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let _dev = f.next()?;
            let mnt = f.next()?.replace("\\040", " ");
            let ty = f.next()?.to_string();
            let inside = path == mnt.as_str()
                || mnt == "/"
                || path.starts_with(&format!("{}/", mnt.trim_end_matches('/')));
            inside.then_some((mnt.len(), ty))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, ty)| ty)
}

fn gpu_vendors(root: &Root) -> Vec<&'static str> {
    let mut v = Vec::new();
    for e in fs::read_dir(root.p("/sys/class/drm"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let vendor = fs::read_to_string(e.path().join("device/vendor")).unwrap_or_default();
        let ven = match vendor.trim() {
            "0x10de" => "nvidia",
            "0x1002" => "amd",
            "0x8086" => "intel",
            _ => continue,
        };
        if !v.contains(&ven) {
            v.push(ven);
        }
    }
    v
}

/// The 32-bit Vulkan driver for a vendor is installed. Checks the library itself and pacman's
/// database, not the manifest's file name, which Mesa has changed between versions.
fn has_lib32_vulkan(root: &Root, vendor: &str) -> bool {
    let (libs, pkg): (&[&str], &str) = match vendor {
        "nvidia" => (
            &[
                "/usr/lib32/libGLX_nvidia.so.0",
                "/usr/lib32/libnvidia-glcore.so.0",
            ],
            "lib32-nvidia-utils-",
        ),
        "amd" => (&["/usr/lib32/libvulkan_radeon.so"], "lib32-vulkan-radeon-"),
        "intel" => (&["/usr/lib32/libvulkan_intel.so"], "lib32-vulkan-intel-"),
        _ => return true,
    };
    if libs.iter().any(|l| root.exists(l)) {
        return true;
    }
    let in_pacman = fs::read_dir(root.p("/var/lib/pacman/local"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            // "lib32-vulkan-radeon-1:26.3.1-5": the name, then a version that starts with a digit.
            n.strip_prefix(pkg)
                .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()))
        });
    let icd = fs::read_dir(root.p("/usr/share/vulkan/icd.d"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.contains("i686")
                && n.contains(match vendor {
                    "amd" => "radeon",
                    v => v,
                })
        });
    in_pacman || icd
}

/// Check the PC. `libraries` are the Steam library folders.
pub fn pc_checks(root: &Root, libraries: &[PathBuf]) -> Vec<Check> {
    let mut out = Vec::new();

    // Games on an NTFS drive: Proton can't create the files it needs there.
    let ntfs: Vec<String> = libraries
        .iter()
        .filter(|l| fs_type(root, l).is_some_and(|t| t == "ntfs" || t == "ntfs3" || t == "fuseblk"))
        .map(|l| l.display().to_string())
        .collect();
    out.push(if ntfs.is_empty() {
        check("ntfs", "Game drives", Level::Ok, "Your Steam libraries are on Linux drives.", None)
    } else {
        check(
            "ntfs",
            "Games on a Windows (NTFS) drive",
            Level::Bad,
            format!("{} is on an NTFS drive. Windows games often won't start from there under Proton. Move the library to a Linux drive (ext4 or btrfs) in Steam → Settings → Storage.", ntfs.join(", ")),
            None,
        )
    });

    // 32-bit graphics drivers: many games and Steam itself need them, but only for the chip games
    // draw on. With a graphics card in the PC, the built-in chip's 32-bit driver doesn't matter.
    let vendors = gpu_vendors(root);
    let gpus = crate::gpus::list(root);
    let mut needed: Vec<&str> = gpus
        .iter()
        .filter(|g| !g.integrated)
        .map(|g| g.vendor)
        .collect();
    if needed.is_empty() {
        needed = gpus.iter().map(|g| g.vendor).collect();
    }
    if gpus.is_empty() {
        needed = vendors.clone();
    }
    needed.dedup();
    for v in &needed {
        let (pkg, name) = match *v {
            "nvidia" => ("lib32-nvidia-utils", "NVIDIA"),
            "amd" => ("lib32-vulkan-radeon", "AMD"),
            _ => ("lib32-vulkan-intel", "Intel"),
        };
        out.push(if has_lib32_vulkan(root, v) {
            check(&format!("lib32_{v}"), &format!("32-bit {name} driver"), Level::Ok, "Installed.", None)
        } else {
            check(
                &format!("lib32_{v}"),
                &format!("32-bit {name} driver missing"),
                Level::Bad,
                "Older games (and parts of Steam) are 32-bit and can't use your graphics card without it.",
                Some(&format!("sudo pacman -S --needed {pkg}")),
            )
        });
    }

    // Two graphics chips: games should use the strong one, and the monitor belongs on the card.
    if crate::gpus::has_both(&gpus) {
        let card = gpus
            .iter()
            .find(|g| !g.integrated)
            .map(|g| g.describe())
            .unwrap_or_default();
        out.push(check(
            "hybrid_gpu",
            "Two graphics chips",
            Level::Info,
            format!("This PC has a graphics chip built into the processor or motherboard as well as {card}. CachyPlay points the games it's on for at the card, and checks which chip each game actually used."),
            None,
        ));
        if let Some((level, title, detail)) = crate::gpus::monitor_note(root, &gpus) {
            out.push(check("monitor_port", &title, level, detail, None));
        }
    }

    // Copies of games' Windows files that resets set aside: say how much space they use.
    let kept = crate::prefix::all_kept(libraries);
    if !kept.is_empty() {
        let total: u64 = kept.iter().map(|k| k.2).sum();
        let mut ids: Vec<&str> = kept.iter().map(|k| k.1.as_str()).collect();
        ids.dedup();
        out.push(check(
            "kept_prefixes",
            "Old copies of games' Windows files",
            Level::Info,
            format!("Resets set aside {} old cop{} ({}), using {}. Once the games run fine, delete them on each game's Settings tab, or all at once with the command below.", kept.len(), if kept.len() == 1 { "y" } else { "ies" }, if ids.len() == 1 { "1 game".to_string() } else { format!("{} games", ids.len()) }, crate::prefix::human_size(total)),
            Some("cachyplay reset-prefix all --delete-kept"),
        ));
    }

    // NVIDIA display driver mode-setting: needed for Wayland, OBS capture and smooth frames.
    if vendors.contains(&"nvidia") {
        let modeset = root
            .read("/sys/module/nvidia_drm/parameters/modeset")
            .unwrap_or_default();
        out.push(match modeset.trim() {
            "Y" | "1" => check("nvidia_modeset", "NVIDIA mode-setting", Level::Ok, "On.", None),
            "" => check("nvidia_modeset", "NVIDIA driver not loaded", Level::Warn, "The NVIDIA driver isn't loaded right now, so games may run on the built-in graphics chip.", None),
            _ => check(
                "nvidia_modeset",
                "NVIDIA mode-setting is off",
                Level::Warn,
                "Wayland, OBS game capture and smooth frame pacing need it. Turn it on with the kernel option nvidia-drm.modeset=1, then restart.",
                None,
            ),
        });
    }

    // Game performance mode.
    let perf = ["/usr/bin/game-performance", "/usr/bin/gamemoderun"]
        .iter()
        .any(|p| root.exists(p));
    out.push(if perf {
        check(
            "game_mode",
            "Game performance mode",
            Level::Ok,
            "Available.",
            None,
        )
    } else {
        check(
            "game_mode",
            "No game performance mode",
            Level::Info,
            "game-performance (CachyOS) or GameMode switches the CPU to full speed while you play.",
            Some("sudo pacman -S --needed cachyos-settings"),
        )
    });

    // ntsync: faster Windows-style thread syncing for Proton.
    out.push(if root.exists("/dev/ntsync") {
        check("ntsync", "ntsync", Level::Ok, "Available; Proton uses it to run games more smoothly.", None)
    } else {
        check("ntsync", "ntsync not available", Level::Info, "Newer kernels have it; it makes some games run smoother. The CachyOS kernel includes it.", None)
    });

    // Memory map limit: some big games crash below this.
    let maps: u64 = root
        .read("/proc/sys/vm/max_map_count")
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0);
    out.push(if maps >= 1_048_576 || maps == 0 {
        check("max_map_count", "Memory map limit", Level::Ok, "High enough for big games.", None)
    } else {
        check(
            "max_map_count",
            "Memory map limit is low",
            Level::Warn,
            format!("It's {maps}; some big games (like Hogwarts Legacy or CS2) crash below 1048576."),
            Some("echo 'vm.max_map_count = 2147483642' | sudo tee /etc/sysctl.d/80-gaming.conf && sudo sysctl --system"),
        )
    });
    out
}

/// Check CachyPlay's own setup.
pub fn self_test(paths: &Paths) -> Vec<Check> {
    let mut out = Vec::new();
    out.push(match crate::paths::installed_binary(paths) {
        Ok(p) => check(
            "installed",
            "CachyPlay installed",
            Level::Ok,
            p.display().to_string(),
            None,
        ),
        Err(_) => check(
            "installed",
            "CachyPlay isn't installed",
            Level::Bad,
            "Steam needs it in a fixed place. Use the Install button, or run the installer.",
            None,
        ),
    });
    let ctx = match crate::manage::Ctx::new(paths.clone()) {
        Ok(c) => c,
        Err(e) => {
            out.push(check(
                "steam",
                "Can't read Steam",
                Level::Bad,
                format!("{e:#}"),
                None,
            ));
            return out;
        }
    };
    out.push(check(
        "steam",
        "Steam found",
        Level::Ok,
        ctx.steam.root.display().to_string(),
        None,
    ));
    out.push(match ctx.format_problem() {
        None => check(
            "format",
            "Steam's settings file",
            Level::Ok,
            "In a format CachyPlay knows.",
            None,
        ),
        Some(why) => check(
            "format",
            "Steam's settings file looks different",
            Level::Warn,
            why,
            None,
        ),
    });
    let managed = ctx.managed_apps();
    let broken: Vec<String> = managed
        .iter()
        .filter(|id| {
            ctx.steam
                .find_game(id)
                .is_some_and(|g| ctx.status(&g).is_ok_and(|s| !s.applied))
        })
        .cloned()
        .collect();
    out.push(if broken.is_empty() {
        check("launch_options", "Launch options", Level::Ok, format!("Intact for {} game(s).", managed.len()), None)
    } else {
        check(
            "launch_options",
            "Launch options changed outside CachyPlay",
            Level::Warn,
            format!("CachyPlay's part is missing for AppID {}. Turn CachyPlay off and on again for those games.", broken.join(", ")),
            None,
        )
    });
    let backups = paths.backups(&ctx.account.account_id);
    let readable = fs::read_dir(&backups)
        .map(|rd| rd.flatten().all(|e| fs::File::open(e.path()).is_ok()))
        .unwrap_or(true);
    out.push(if readable {
        check("backups", "Backups", Level::Ok, "Readable.", None)
    } else {
        check(
            "backups",
            "A backup can't be read",
            Level::Warn,
            backups.display().to_string(),
            None,
        )
    });
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn rtx_card_with_ryzen_graphics_needs_only_the_nvidia_32bit_driver() {
        // Matt's PC: an RTX 5090 plus the Radeon chip in the Ryzen. Beta.10 warned that the 32-bit
        // AMD driver was missing even though lib32-vulkan-radeon was installed.
        let (_t, r) = tree();
        put(&r, "/proc/mounts", "/dev/nvme0n1p2 / btrfs rw 0 0\n");
        put(&r, "/sys/class/drm/card1/device/vendor", "0x10de\n");
        put(&r, "/sys/class/drm/card1/device/pci_slot", "0000:01:00.0\n");
        put(&r, "/sys/class/drm/card2/device/vendor", "0x1002\n");
        put(&r, "/sys/class/drm/card2/device/pci_slot", "0000:7c:00.0\n");
        put(
            &r,
            "/sys/class/drm/card2/device/mem_info_vram_total",
            "536870912\n",
        );
        put(&r, "/usr/lib32/libGLX_nvidia.so.0", "");
        let c = pc_checks(&r, &[]);
        assert!(c
            .iter()
            .any(|x| x.id == "lib32_nvidia" && x.level == Level::Ok));
        assert!(c.iter().all(|x| x.id != "lib32_amd"), "{c:#?}");
        assert!(c.iter().any(|x| x.id == "hybrid_gpu"));
    }

    #[test]
    fn finds_32bit_drivers_by_library_or_package() {
        let (_t, r) = tree();
        assert!(!has_lib32_vulkan(&r, "amd"));
        put(
            &r,
            "/var/lib/pacman/local/lib32-vulkan-radeon-1:26.3.1-5/desc",
            "",
        );
        assert!(has_lib32_vulkan(&r, "amd"));
        let (_t2, r2) = tree();
        put(&r2, "/usr/lib32/libvulkan_radeon.so", "");
        assert!(has_lib32_vulkan(&r2, "amd"));
        let (_t3, r3) = tree();
        // A different package whose name starts the same way isn't the driver.
        put(
            &r3,
            "/var/lib/pacman/local/lib32-vulkan-radeon-debug-1:26.3.1-5/desc",
            "",
        );
        assert!(!has_lib32_vulkan(&r3, "amd"));
    }

    use super::*;

    fn tree() -> (tempfile::TempDir, Root) {
        let t = tempfile::tempdir().unwrap();
        let r = Root(t.path().to_path_buf());
        (t, r)
    }

    fn put(r: &Root, abs: &str, text: &str) {
        let p = r.p(abs);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn spots_ntfs_libraries_and_missing_32bit_driver() {
        let (_t, r) = tree();
        put(
            &r,
            "/proc/mounts",
            "/dev/nvme0n1p2 / btrfs rw 0 0\n/dev/sda1 /mnt/win\\040games ntfs3 rw 0 0\n",
        );
        put(&r, "/sys/class/drm/card0/device/vendor", "0x10de\n");
        put(&r, "/sys/class/drm/card1/device/vendor", "0x1002\n");
        put(&r, "/sys/module/nvidia_drm/parameters/modeset", "N\n");
        put(&r, "/proc/sys/vm/max_map_count", "65530\n");
        put(
            &r,
            "/sys/class/drm/card1/device/mem_info_vram_total",
            "17163091968\n",
        );
        put(&r, "/usr/share/vulkan/icd.d/radeon_icd.i686.json", "{}");
        let c = pc_checks(
            &r,
            &[
                PathBuf::from("/mnt/win games/SteamLibrary"),
                PathBuf::from("/home/m/.local/share/Steam"),
            ],
        );
        let level = |id: &str| c.iter().find(|x| x.id == id).unwrap().level.clone();
        assert_eq!(level("ntfs"), Level::Bad);
        assert!(c[0].detail.contains("/mnt/win games"));
        assert_eq!(level("lib32_nvidia"), Level::Bad);
        assert_eq!(level("lib32_amd"), Level::Ok);
        // Two graphics cards (NVIDIA and a big Radeon): both need their 32-bit driver, and
        // neither is a built-in chip.
        assert!(c.iter().all(|x| x.id != "hybrid_gpu"));
        assert_eq!(level("nvidia_modeset"), Level::Warn);
        assert_eq!(level("max_map_count"), Level::Warn);
        assert_eq!(level("ntsync"), Level::Info);
    }

    #[test]
    fn healthy_cachyos_nvidia_pc() {
        let (_t, r) = tree();
        put(&r, "/proc/mounts", "/dev/nvme0n1p2 / btrfs rw 0 0\n");
        put(&r, "/sys/class/drm/card0/device/vendor", "0x10de\n");
        put(&r, "/sys/module/nvidia_drm/parameters/modeset", "Y\n");
        put(&r, "/proc/sys/vm/max_map_count", "2147483642\n");
        put(&r, "/usr/lib32/libGLX_nvidia.so.0", "");
        put(&r, "/usr/bin/game-performance", "");
        put(&r, "/dev/ntsync", "");
        let c = pc_checks(&r, &[PathBuf::from("/home/m/.local/share/Steam")]);
        assert!(c.iter().all(|x| x.level == Level::Ok), "{c:#?}");
    }
}
