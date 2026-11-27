//! Per-game fix files ("recipes"): declarative TOML using a fixed list of named actions.
//! A recipe can never run arbitrary commands.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Recipes shipped inside the binary. Version 0.2.0 adds signed online updates on top.
const BUNDLED: &[(&str, &str)] = &[("1240440", include_str!("../../../recipes/1240440.toml"))];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema: u32,
    pub app_id: u64,
    pub name: String,
    /// Only `true` after someone tested this file on real hardware (see CONTRIBUTING.md).
    #[serde(default)]
    pub tested: bool,
    #[serde(default)]
    pub verified: Option<String>,
    #[serde(default)]
    pub anti_cheat: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub runner: Option<Runner>,
    #[serde(default)]
    pub env: BTreeMap<String, EnvVar>,
    #[serde(default, rename = "action")]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub crash: CrashSettings,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runner {
    #[serde(default)]
    pub prefer: Vec<String>,
    #[serde(default)]
    pub platform: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvVar {
    pub value: String,
    #[serde(default)]
    pub when: Option<String>,
    pub why: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub id: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub when: Option<String>,
    pub why: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrashSettings {
    #[serde(default = "default_min_runtime")]
    pub min_runtime_secs: u64,
}

impl Default for CrashSettings {
    fn default() -> Self {
        CrashSettings {
            min_runtime_secs: default_min_runtime(),
        }
    }
}

fn default_min_runtime() -> u64 {
    30
}

/// Every action id the schema knows. Anything else is rejected when the file is loaded.
pub const KNOWN_ACTIONS: &[&str] = &[
    "add_game_arg",
    "use_game_performance",
    // Known to the schema but applied from version 0.2.0 on; 0.1.0 lists them as skipped.
    "set_runner",
    "winetricks",
    "dll_override",
    "purge_shader_cache",
    "patch_config",
];

/// Actions 0.1.0 can apply at launch time.
pub const SUPPORTED_ACTIONS: &[&str] = &["add_game_arg", "use_game_performance"];

/// Environment variables a recipe may set. Keeps a stolen signing key from injecting
/// things like LD_PRELOAD.
pub fn env_allowed(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "PROTON_", "DXVK_", "VKD3D_", "WINE", "RADV_", "MESA_", "__GL_", "MANGOHUD", "SDL_",
        "STAGING_", "ENABLE_",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
        && !matches!(name, "WINEPREFIX" | "WINESERVER" | "WINELOADER")
}

impl Recipe {
    pub fn parse(text: &str) -> Result<Recipe> {
        let r: Recipe = toml::from_str(text)?;
        if r.schema != 1 {
            bail!(
                "unsupported recipe schema {} (this CachyPlay understands schema 1)",
                r.schema
            );
        }
        for a in &r.actions {
            if !KNOWN_ACTIONS.contains(&a.id.as_str()) {
                bail!("unknown action id {:?}", a.id);
            }
        }
        for k in r.env.keys() {
            if !env_allowed(k) {
                bail!("environment variable {k} is not allowed in recipes");
            }
        }
        for cond in r
            .env
            .values()
            .filter_map(|e| e.when.as_deref())
            .chain(r.actions.iter().filter_map(|a| a.when.as_deref()))
        {
            parse_condition(cond).with_context(|| format!("bad condition {cond:?}"))?;
        }
        Ok(r)
    }
}

/// Find the recipe for a game: a local override in `~/.config/cachyplay/recipes/` wins,
/// then downloaded recipes, then the ones bundled in the binary.
pub fn find(app_id: &str, config_dir: &Path, data_recipes: &Path) -> Result<Option<Recipe>> {
    for dir in [config_dir.join("recipes"), data_recipes.to_path_buf()] {
        let p = dir.join(format!("{app_id}.toml"));
        if p.is_file() {
            let text = fs::read_to_string(&p)?;
            return Recipe::parse(&text)
                .with_context(|| format!("in {}", p.display()))
                .map(Some);
        }
    }
    match BUNDLED.iter().find(|(id, _)| *id == app_id) {
        Some((_, text)) => Recipe::parse(text).map(Some),
        None => Ok(None),
    }
}

/// Parse every recipe built into this program (used by the quick self-test). Returns how many.
pub fn check_bundled() -> Result<usize> {
    for (id, text) in BUNDLED {
        Recipe::parse(text).with_context(|| format!("built-in recipe {id}"))?;
    }
    Ok(BUNDLED.len())
}

/// Facts about this PC that recipe conditions can test.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub values: BTreeMap<String, String>,
}

impl Facts {
    pub fn detect() -> Facts {
        let mut values = BTreeMap::new();
        let vendors = gpu_vendors(Path::new("/sys/class/drm"));
        // Prefer a discrete GPU's vendor when several are present.
        let primary = ["nvidia", "amd", "intel"]
            .iter()
            .find(|v| vendors.iter().any(|x| x == *v))
            .map(|s| s.to_string());
        values.insert(
            "gpu.vendor".into(),
            primary.unwrap_or_else(|| "unknown".into()),
        );
        values.insert("gpu.count".into(), vendors.len().to_string());
        values.insert(
            "session".into(),
            std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into()),
        );
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        values.insert("cpu.level".into(), cpu_level(&cpuinfo).into());
        values.insert(
            "ntsync".into(),
            Path::new("/dev/ntsync").exists().to_string(),
        );
        Facts { values }
    }

    pub fn eval(&self, cond: &str) -> Result<bool> {
        let parsed = parse_condition(cond)?;
        Ok(parsed.iter().any(|all| {
            all.iter().all(|(k, eq, v)| {
                let actual = self.values.get(k).map(String::as_str).unwrap_or("");
                (actual == v) == *eq
            })
        }))
    }
}

/// `a == 'x' && b != 'y' || c == 'z'` → OR of ANDs of (key, is_equal, value).
fn parse_condition(cond: &str) -> Result<Vec<Vec<(String, bool, String)>>> {
    cond.split("||")
        .map(|clause| {
            clause
                .split("&&")
                .map(|atom| {
                    let atom = atom.trim();
                    let (k, eq, v) = if let Some((k, v)) = atom.split_once("!=") {
                        (k, false, v)
                    } else if let Some((k, v)) = atom.split_once("==") {
                        (k, true, v)
                    } else {
                        bail!("expected `key == 'value'` or `key != 'value'`, got {atom:?}");
                    };
                    let v = v.trim();
                    let v = v
                        .strip_prefix('\'')
                        .and_then(|v| v.strip_suffix('\''))
                        .with_context(|| format!("value must be in single quotes: {v}"))?;
                    let k = k.trim();
                    if !["gpu.vendor", "gpu.count", "session", "cpu.level", "ntsync"].contains(&k) {
                        bail!("unknown fact {k:?}");
                    }
                    Ok((k.to_string(), eq, v.to_string()))
                })
                .collect()
        })
        .collect()
}

pub fn gpu_vendors(drm: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(drm) else {
        return out;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let vendor = fs::read_to_string(e.path().join("device/vendor")).unwrap_or_default();
        let v = match vendor.trim() {
            "0x10de" => "nvidia",
            "0x1002" => "amd",
            "0x8086" => "intel",
            _ => continue,
        };
        out.push(v.to_string());
    }
    out
}

/// x86-64 microarchitecture level, judged from CPU flags (v3 needs AVX2, BMI2, FMA, MOVBE).
pub fn cpu_level(cpuinfo: &str) -> &'static str {
    let flags = cpuinfo
        .lines()
        .find_map(|l| {
            l.strip_prefix("flags")
                .and_then(|r| r.split_once(':'))
                .map(|(_, f)| f)
        })
        .unwrap_or("");
    let has = |f: &str| flags.split_whitespace().any(|x| x == f);
    if ["avx512f", "avx512bw", "avx512cd", "avx512dq", "avx512vl"]
        .iter()
        .all(|f| has(f))
        && has("avx2")
    {
        "v4"
    } else if ["avx2", "bmi2", "fma", "movbe"].iter().all(|f| has(f)) {
        "v3"
    } else if ["sse4_2", "popcnt", "ssse3"].iter().all(|f| has(f)) {
        "v2"
    } else {
        "v1"
    }
}

/// What a recipe resolves to on this PC.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub env: Vec<(String, String, String)>, // (name, value, why)
    pub extra_args: Vec<String>,
    pub game_performance: bool,
    /// Start the game through obs-gamecapture (set by the wrapper from your OBS setting, not by fix files).
    pub obs_capture: bool,
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
    pub min_runtime_secs: u64,
}

pub fn plan(recipe: &Recipe, facts: &Facts) -> Plan {
    let mut p = Plan {
        min_runtime_secs: recipe.crash.min_runtime_secs,
        ..Default::default()
    };
    let applies = |when: &Option<String>| {
        when.as_deref()
            .map(|c| facts.eval(c).unwrap_or(false))
            .unwrap_or(true)
    };
    for (k, v) in &recipe.env {
        if applies(&v.when) {
            p.env.push((k.clone(), v.value.clone(), v.why.clone()));
            p.applied.push(format!("{k}={}: {}", v.value, v.why));
        }
    }
    for a in &recipe.actions {
        if !applies(&a.when) {
            continue;
        }
        match a.id.as_str() {
            "add_game_arg" => {
                p.extra_args.extend(a.args.iter().cloned());
                p.applied
                    .push(format!("Game arguments {}: {}", a.args.join(" "), a.why));
            }
            "use_game_performance" => {
                p.game_performance = true;
                p.applied
                    .push(format!("CachyOS game-performance mode: {}", a.why));
            }
            other => p.skipped.push(format!(
                "{other} ({}): needs CachyPlay 0.2.0 or newer",
                a.why
            )),
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(vendor: &str) -> Facts {
        let mut f = Facts::default();
        f.values.insert("gpu.vendor".into(), vendor.into());
        f.values.insert("session".into(), "wayland".into());
        f
    }

    #[test]
    fn bundled_recipes_parse() {
        for (id, text) in BUNDLED {
            let r = Recipe::parse(text).unwrap_or_else(|e| panic!("{id}: {e:#}"));
            assert_eq!(r.app_id.to_string(), *id);
        }
    }

    #[test]
    fn conditions() {
        let f = facts("nvidia");
        assert!(f.eval("gpu.vendor == 'nvidia'").unwrap());
        assert!(!f.eval("gpu.vendor == 'amd'").unwrap());
        assert!(f
            .eval("gpu.vendor == 'amd' || session == 'wayland'")
            .unwrap());
        assert!(!f
            .eval("gpu.vendor == 'nvidia' && session != 'wayland'")
            .unwrap());
        assert!(f.eval("rm -rf /").is_err());
        assert!(f.eval("nope == 'x'").is_err());
    }

    #[test]
    fn rejects_unknown_actions_and_dangerous_env() {
        let base = "schema = 1\napp_id = 1\nname = \"x\"\n";
        assert!(
            Recipe::parse(&format!("{base}[[action]]\nid = \"shell\"\nwhy = \"x\"\n")).is_err()
        );
        assert!(Recipe::parse(&format!(
            "{base}[env]\nLD_PRELOAD = {{ value = \"/tmp/x.so\", why = \"x\" }}\n"
        ))
        .is_err());
        assert!(Recipe::parse(&format!("{base}surprise = true\n")).is_err());
        assert!(Recipe::parse(&format!(
            "{base}[env]\nPROTON_ENABLE_NVAPI = {{ value = \"1\", why = \"x\" }}\n"
        ))
        .is_ok());
    }

    #[test]
    fn plan_respects_conditions_and_skips_future_actions() {
        let r = Recipe::parse(
            "schema = 1\napp_id = 1\nname = \"x\"\n[env]\nPROTON_ENABLE_NVAPI = { value = \"1\", when = \"gpu.vendor == 'nvidia'\", why = \"DLSS\" }\n[[action]]\nid = \"winetricks\"\nargs = [\"vcrun2022\"]\nwhy = \"runtime\"\n[[action]]\nid = \"add_game_arg\"\nargs = [\"-dx11\"]\nwhy = \"stable\"\n",
        )
        .unwrap();
        let p = plan(&r, &facts("nvidia"));
        assert_eq!(p.env.len(), 1);
        assert_eq!(p.extra_args, vec!["-dx11"]);
        assert_eq!(p.skipped.len(), 1);
        assert!(plan(&r, &facts("amd")).env.is_empty());
    }

    #[test]
    fn cpu_levels() {
        assert_eq!(
            cpu_level("flags\t\t: sse4_2 popcnt ssse3 avx2 bmi2 fma movbe\n"),
            "v3"
        );
        assert_eq!(cpu_level("flags\t\t: sse2\n"), "v1");
    }
}
