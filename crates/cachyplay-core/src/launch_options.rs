//! Editing a game's Steam `LaunchOptions` string without disturbing the user's own options.
//!
//! CachyPlay inserts exactly one segment, directly in front of `%command%`:
//!
//! ```text
//! sh -c '[ -x "$0" ] && exec "$0" run -- "$@"; exec "$@"' /usr/bin/cachyplay %command%
//! ```
//!
//! This is the **fail-open** form: `$0` is the absolute path of the installed binary. If it is
//! missing (e.g. the package was removed without restoring first) or cannot be executed, the
//! shell falls through to `exec "$@"`, which runs the game exactly as Steam asked.
//! Everything the user had before `%command%` (env vars, other wrappers) and after it (game
//! arguments like `-nosplash`) is kept in place.
//!
//! VERIFY ON REAL STEAM: this relies on Steam running launch options through a shell and
//! word-splitting the expanded `%command%`. The integration test in the README covers it.

use std::path::Path;

pub const COMMAND: &str = "%command%";
const SCRIPT: &str = r#"sh -c '[ -x "$0" ] && exec "$0" run -- "$@"; exec "$@"'"#;

/// The segment CachyPlay inserts before `%command%`, including the trailing space.
pub fn wrapper_segment(bin: &Path) -> String {
    format!("{SCRIPT} {} ", shell_quote(&bin.to_string_lossy()))
}

/// Add the wrapper to `original`. Idempotent: an existing CachyPlay segment is replaced
/// (so a moved binary path is updated) rather than added twice.
pub fn apply(original: &str, bin: &Path) -> String {
    let base = strip(original);
    let seg = wrapper_segment(bin);
    match base.find(COMMAND) {
        Some(i) => format!("{}{seg}{}", &base[..i], &base[i..]),
        // No %command%: Steam treats the whole string as game arguments.
        None if base.trim().is_empty() => format!("{seg}{COMMAND}"),
        None => format!("{seg}{COMMAND} {}", base.trim()),
    }
}

pub fn is_applied(options: &str) -> bool {
    find_segment(options).is_some()
}

/// Remove CachyPlay's segment, leaving everything else exactly as it was.
pub fn strip(options: &str) -> String {
    match find_segment(options) {
        Some((start, end)) => {
            let out = format!("{}{}", &options[..start], &options[end..]);
            if out.trim() == COMMAND {
                String::new()
            } else {
                out
            }
        }
        None => options.to_string(),
    }
}

/// Byte range of `SCRIPT <path> ` immediately preceding `%command%`.
fn find_segment(options: &str) -> Option<(usize, usize)> {
    let start = options.find(SCRIPT)?;
    let after = &options[start + SCRIPT.len()..];
    let cmd = after.find(COMMAND)?;
    // Only a single path token may sit between the script and %command%.
    let between = after[..cmd].trim();
    if between.is_empty() || split_one_token(between).is_none() {
        return None;
    }
    Some((start, start + SCRIPT.len() + cmd))
}

fn split_one_token(s: &str) -> Option<()> {
    if s.starts_with('\'') {
        return (s.len() >= 2 && s.ends_with('\'') && !s[1..s.len() - 1].contains('\''))
            .then_some(());
    }
    (!s.chars().any(char::is_whitespace)).then_some(())
}

pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+:@%,".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn bin() -> PathBuf {
        PathBuf::from("/usr/bin/cachyplay")
    }

    #[test]
    fn empty_options() {
        let out = apply("", &bin());
        assert_eq!(
            out,
            r#"sh -c '[ -x "$0" ] && exec "$0" run -- "$@"; exec "$@"' /usr/bin/cachyplay %command%"#
        );
        assert_eq!(strip(&out), "");
    }

    #[test]
    fn keeps_user_prefix_and_args() {
        let orig = "PROTON_LOG=1 gamemoderun %command% -nosplash -novid";
        let out = apply(orig, &bin());
        assert!(out.starts_with("PROTON_LOG=1 gamemoderun sh -c "));
        assert!(out.ends_with("/usr/bin/cachyplay %command% -nosplash -novid"));
        assert_eq!(strip(&out), orig);
    }

    #[test]
    fn args_without_command() {
        let out = apply("-nosplash", &bin());
        assert!(out.ends_with("cachyplay %command% -nosplash"));
        assert_eq!(strip(&out), "%command% -nosplash");
    }

    #[test]
    fn idempotent_and_updates_path() {
        let once = apply("%command% -x", &bin());
        assert_eq!(apply(&once, &bin()), once);
        let moved = apply(&once, Path::new("/home/me/.local/bin/cachyplay"));
        assert!(moved.contains("/home/me/.local/bin/cachyplay %command% -x"));
        assert_eq!(moved.matches("sh -c").count(), 1);
    }

    #[test]
    fn quotes_paths_with_spaces() {
        let out = apply("", Path::new("/home/a b/.local/bin/cachyplay"));
        assert!(out.contains("'/home/a b/.local/bin/cachyplay' %command%"));
        assert!(is_applied(&out));
        assert_eq!(strip(&out), "");
    }

    #[test]
    fn leaves_unrelated_options_alone() {
        assert!(!is_applied("%command% -sh"));
        assert_eq!(strip("FOO=1 %command%"), "FOO=1 %command%");
    }
}
