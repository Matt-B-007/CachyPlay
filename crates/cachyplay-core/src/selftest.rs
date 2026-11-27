//! `--self-test --quick`: can this program start, find what's built into it and read the settings?
//! A few seconds at most, no network, no other programs, nothing written. The updater runs it on a
//! new version before swapping it in; the full `--self-test` is `cachyplay doctor`.

use crate::paths::Paths;
use anyhow::{Context, Result};

/// Each passed check as a line; the first failure as the error.
pub fn quick(paths: &Paths) -> Result<Vec<String>> {
    let mut ok = vec![format!("CachyPlay {} starts", crate::VERSION)];
    anyhow::ensure!(
        paths.home.is_dir(),
        "the home folder {} is missing",
        paths.home.display()
    );
    ok.push("Finds its folders".into());
    let n = crate::recipes::check_bundled()?;
    ok.push(format!("Built-in game fixes readable ({n})"));
    let keys = crate::update::key_lines(crate::update::embedded_keys());
    anyhow::ensure!(!keys.is_empty(), "no update signing keys built in");
    for k in &keys {
        minisign_verify::PublicKey::from_base64(k)
            .map_err(|e| anyhow::anyhow!("a built-in signing key is invalid: {e}"))?;
    }
    ok.push(format!("Update signing keys valid ({})", keys.len()));
    let file = paths.config.join("settings.json");
    if file.is_file() {
        let text = std::fs::read_to_string(&file).context("can't read settings.json")?;
        let v: serde_json::Value =
            serde_json::from_str(&text).context("settings.json isn't valid")?;
        let (v, _) = crate::update::migrate(v);
        serde_json::from_value::<crate::update::Settings>(v)
            .context("settings.json can't be read by this version")?;
        ok.push("Settings readable".into());
    } else {
        ok.push("No settings saved yet".into());
    }
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_on_a_fresh_home_and_fails_on_broken_settings() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::isolated(d.path());
        std::fs::create_dir_all(&p.home).unwrap();
        quick(&p).unwrap();
        std::fs::create_dir_all(&p.config).unwrap();
        std::fs::write(p.config.join("settings.json"), "{\"channel\":\"beta\"}").unwrap();
        assert!(quick(&p).unwrap().iter().any(|l| l == "Settings readable"));
        std::fs::write(p.config.join("settings.json"), "{not json").unwrap();
        assert!(quick(&p).is_err());
    }
}
