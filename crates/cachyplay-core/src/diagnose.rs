// SPDX-License-Identifier: GPL-3.0-or-later
//! Matching log lines against known crash patterns, and telling a likely driver fault apart
//! from a game or Proton problem. 0.1.0 reports matches; 0.2.0 turns them into one-click fixes.

#[derive(Debug, Clone, PartialEq)]
pub enum Bucket {
    LikelyDriver,
    GameOrProton,
}

pub struct Rule {
    pub id: &'static str,
    /// Case-insensitive substrings; any one matching triggers the rule.
    pub patterns: &'static [&'static str],
    pub diagnosis: &'static str,
    pub suggestion: &'static str,
    pub bucket: Bucket,
}

pub const RULES: &[Rule] = &[
    Rule {
        id: "gpu_reset",
        patterns: &["NVRM: Xid", "amdgpu: ring", "GPU reset", "GPU HANG", "VK_ERROR_DEVICE_LOST"],
        diagnosis: "The graphics driver reset or timed out.",
        suggestion: "Try without Wayland/HDR flags first. If it happens in several games since a driver update, it's likely a driver regression.",
        bucket: Bucket::LikelyDriver,
    },
    Rule {
        id: "oom",
        patterns: &["Out of memory: Killed process", "oom-kill"],
        diagnosis: "The system ran out of memory.",
        suggestion: "Close other apps (especially browsers) and lower texture settings.",
        bucket: Bucket::GameOrProton,
    },
    Rule {
        id: "missing_dll",
        patterns: &["err:module:import_dll"],
        diagnosis: "A Windows library the game needs is missing.",
        suggestion: "Usually fixed by installing a runtime (e.g. vcrun2022) into the game's prefix, or by a different Proton build.",
        bucket: Bucket::GameOrProton,
    },
    Rule {
        id: "anticheat",
        patterns: &["EasyAntiCheat", "BattlEye"],
        diagnosis: "Anti-cheat was involved in the failed launch.",
        suggestion: "Make sure the Proton EasyAntiCheat / BattlEye Runtime is installed and the game uses an SLR Proton build.",
        bucket: Bucket::GameOrProton,
    },
    Rule {
        id: "unity_crash",
        patterns: &["Crash!!!", "Received signal SIGSEGV", "Native Crash Reporting", "UnityPlayer.dll caused"],
        diagnosis: "The game itself (Unity engine) crashed; its Player.log shows where.",
        suggestion: "Check the lines just above the crash in the Unity log. If it started after a game update, report it to the game's developer with this bug report.",
        bucket: Bucket::GameOrProton,
    },
    Rule {
        id: "vulkan_present",
        patterns: &["vkQueuePresentKHR failed", "VK_ERROR_SURFACE_LOST", "VK_ERROR_OUT_OF_DATE"],
        diagnosis: "Showing a frame on screen failed (Vulkan present error).",
        suggestion: "Try the game without Vulkan add-ons (VK_LOADER_LAYERS_DISABLE=*AFMF*,*MAKO* %command%) and in a window instead of fullscreen.",
        bucket: Bucket::GameOrProton,
    },
    Rule {
        id: "unimplemented",
        patterns: &["unimplemented function"],
        diagnosis: "The game used a Windows feature this Proton version doesn't have.",
        suggestion: "Try a newer Proton build (Proton Experimental or a recent GE-Proton).",
        bucket: Bucket::GameOrProton,
    },
];

#[derive(Debug, Clone)]
pub struct Finding {
    pub rule: &'static str,
    pub diagnosis: &'static str,
    pub suggestion: &'static str,
    pub bucket: Bucket,
    pub lines: Vec<String>,
}

/// Scan log text; returns one finding per matched rule with up to 5 example lines.
pub fn scan(text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for rule in RULES {
        // Newest lines first: logs grow downward, and the latest crash matters most.
        let mut lines: Vec<String> = text
            .lines()
            .filter(|l| {
                let low = l.to_lowercase();
                rule.patterns
                    .iter()
                    .any(|p| low.contains(&p.to_lowercase()))
            })
            .map(|l| l.chars().take(300).collect())
            .collect();
        let start = lines.len().saturating_sub(6);
        lines.drain(..start);
        lines.reverse();
        if !lines.is_empty() {
            out.push(Finding {
                rule: rule.id,
                diagnosis: rule.diagnosis,
                suggestion: rule.suggestion,
                bucket: rule.bucket.clone(),
                lines,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_patterns() {
        let log = "123: err:module:import_dll Library vcruntime140_1.dll not found\n[ 55.1] NVRM: Xid (PCI:0000:01:00): 79\n";
        let f = scan(log);
        assert_eq!(
            f.iter().map(|x| x.rule).collect::<Vec<_>>(),
            vec!["gpu_reset", "missing_dll"]
        );
        assert_eq!(f[0].bucket, Bucket::LikelyDriver);
        assert!(scan("all good").is_empty());
    }
}
