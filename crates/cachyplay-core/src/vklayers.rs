// SPDX-License-Identifier: GPL-3.0-or-later
//! Vulkan "implicit layers": add-ons the Vulkan loader injects into every game (frame generation,
//! overlays, upscalers, capture tools). Each one installs a small JSON manifest; reading those tells
//! us which add-ons are always on, which only switch on with an environment variable, and how to
//! turn them off for one game (`VK_LOADER_LAYERS_DISABLE`, supported by the Vulkan loader 1.3.234+).

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Layer {
    pub name: String,
    pub description: String,
    pub manifest: String,
    /// Loaded into every Vulkan program unless turned off.
    pub active: bool,
    /// Ships with Steam, Mesa, the NVIDIA driver or gamescope: not a suspect.
    pub trusted: bool,
}

impl Layer {
    /// A third-party add-on that is loaded into every game.
    pub fn suspect(&self) -> bool {
        self.active && !self.trusted
    }
}

const TRUSTED_PREFIXES: &[&str] = &[
    "VK_LAYER_VALVE_",
    "VK_LAYER_MESA_",
    "VK_LAYER_NV_",
    "VK_LAYER_FROG_gamescope",
    "VK_LAYER_KHRONOS_",
];

/// Standard places Vulkan implicit layer manifests live.
pub fn manifest_dirs(home: &Path) -> Vec<PathBuf> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    vec![
        PathBuf::from("/usr/share/vulkan/implicit_layer.d"),
        PathBuf::from("/usr/local/share/vulkan/implicit_layer.d"),
        PathBuf::from("/etc/vulkan/implicit_layer.d"),
        home.join(".config/vulkan/implicit_layer.d"),
        data_home.join("vulkan/implicit_layer.d"),
    ]
}

pub fn detect(home: &Path) -> Vec<Layer> {
    detect_in(&manifest_dirs(home), &|k| std::env::var(k).ok())
}

/// `env` looks up environment variables (injected for tests).
pub fn detect_in(dirs: &[PathBuf], env: &dyn Fn(&str) -> Option<String>) -> Vec<Layer> {
    let mut out: Vec<Layer> = Vec::new();
    for dir in dirs {
        let Ok(rd) = fs::read_dir(dir) else { continue };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        for file in files {
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let layers: Vec<&serde_json::Value> = match (&v["layer"], &v["layers"]) {
                (l @ serde_json::Value::Object(_), _) => vec![l],
                (_, serde_json::Value::Array(a)) => a.iter().collect(),
                _ => continue,
            };
            for l in layers {
                let Some(name) = l["name"].as_str() else {
                    continue;
                };
                if out.iter().any(|x| x.name == name) {
                    continue;
                }
                let env_matches = |key: &str| -> bool {
                    l[key].as_object().is_some_and(|m| {
                        m.iter().any(|(k, want)| {
                            env(k).is_some_and(|have| {
                                want.as_str().is_none_or(|w| w == have || w.is_empty())
                            })
                        })
                    })
                };
                let needs_enable = l["enable_environment"]
                    .as_object()
                    .is_some_and(|m| !m.is_empty());
                let active = (!needs_enable || env_matches("enable_environment"))
                    && !env_matches("disable_environment");
                out.push(Layer {
                    name: name.to_string(),
                    description: l["description"].as_str().unwrap_or("").to_string(),
                    manifest: file.display().to_string(),
                    active,
                    trusted: TRUSTED_PREFIXES.iter().any(|p| name.starts_with(p)),
                });
            }
        }
    }
    out
}

/// The value for `VK_LOADER_LAYERS_DISABLE` that turns these layers off.
pub fn disable_value(layers: &[&Layer]) -> String {
    layers
        .iter()
        .map(|l| l.name.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// A short, friendly name for a layer ("AMD Fluid Motion Frames style frame generation" → as is).
pub fn friendly(l: &Layer) -> String {
    if l.description.is_empty() {
        l.name.trim_start_matches("VK_LAYER_").replace('_', " ")
    } else {
        l.description.chars().take(70).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, file: &str, json: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(file), json).unwrap();
    }

    #[test]
    fn finds_always_on_third_party_layers() {
        let d = tempfile::tempdir().unwrap();
        let sys = d.path().join("usr/share/vulkan/implicit_layer.d");
        write(
            &sys,
            "afmf.json",
            r#"{"file_format_version":"1.0.0","layer":{"name":"VK_LAYER_AFMF","description":"AMD Fluid Motion Frames style frame generation for Linux","disable_environment":{"DISABLE_AFMF":"1"}}}"#,
        );
        write(
            &sys,
            "mako.json",
            r#"{"file_format_version":"1.2.0","layers":[{"name":"VK_LAYER_MAKO_render","description":"MAKO Renderer graphics layer"},{"name":"VK_LAYER_MAKO_spatial_scaling","description":"MAKO Renderer spatial-scaling layer"}]}"#,
        );
        write(
            &sys,
            "mangohud.json",
            r#"{"layer":{"name":"VK_LAYER_MANGOHUD_overlay_x86_64","description":"Vulkan Hud Overlay","enable_environment":{"MANGOHUD":"1"},"disable_environment":{"DISABLE_MANGOHUD":"1"}}}"#,
        );
        write(
            &sys,
            "steam.json",
            r#"{"layer":{"name":"VK_LAYER_VALVE_steam_overlay_64","description":"Steam Overlay Layer"}}"#,
        );
        write(&sys, "broken.json", "not json");

        let no_env = |_: &str| None;
        let layers = detect_in(std::slice::from_ref(&sys), &no_env);
        let suspects: Vec<&str> = layers
            .iter()
            .filter(|l| l.suspect())
            .map(|l| l.name.as_str())
            .collect();
        assert_eq!(
            suspects,
            vec![
                "VK_LAYER_AFMF",
                "VK_LAYER_MAKO_render",
                "VK_LAYER_MAKO_spatial_scaling"
            ]
        );
        assert!(
            !layers
                .iter()
                .find(|l| l.name.contains("MANGOHUD"))
                .unwrap()
                .active,
            "MangoHud only when MANGOHUD=1"
        );

        // With MANGOHUD=1 it is active; with DISABLE_AFMF=1 AFMF is off.
        let env = |k: &str| match k {
            "MANGOHUD" | "DISABLE_AFMF" => Some("1".to_string()),
            _ => None,
        };
        let layers = detect_in(&[sys], &env);
        assert!(
            layers
                .iter()
                .find(|l| l.name.contains("MANGOHUD"))
                .unwrap()
                .active
        );
        assert!(
            !layers
                .iter()
                .find(|l| l.name == "VK_LAYER_AFMF")
                .unwrap()
                .active
        );
        let refs: Vec<&Layer> = layers
            .iter()
            .filter(|l| l.name.starts_with("VK_LAYER_MAKO"))
            .collect();
        assert_eq!(
            disable_value(&refs),
            "VK_LAYER_MAKO_render,VK_LAYER_MAKO_spatial_scaling"
        );
    }
}
