// SPDX-License-Identifier: GPL-3.0-or-later
//! CachyPlay core library. Every feature lives here so the CLI and the GUI stay thin.

pub mod advice;
pub mod clock;
pub mod compat;
pub mod crash;
pub mod crashmatch;
pub mod diagnose;
pub mod evidence;
pub mod fixes;
pub mod fsutil;
pub mod gpulog;
pub mod gpus;
pub mod history;
pub mod housekeeping;
pub mod insights;
pub mod install;
pub mod keep;
pub mod launch_options;
pub mod library;
pub mod live;
pub mod logging;
pub mod manage;
pub mod notes;
pub mod obs;
pub mod outreach;
pub mod paths;
pub mod pccheck;
pub mod prefix;
pub mod progress;
pub mod protondb;
pub mod recipes;
pub mod report;
pub mod selftest;
pub mod shadercache;
pub mod steam;
pub mod sys;
pub mod triage;
pub mod tries;
pub mod update;
pub mod vdf;
pub mod vklayers;
pub mod wrapper;

#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// "Moon Sweepers's fix" reads better as "Moon Sweepers' fix": names (capitalised words) that end
/// in s take only an apostrophe.
pub fn possessives(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word_start = 0;
    let mut i = 0;
    let b = text.as_bytes();
    while i < b.len() {
        if !b[i].is_ascii_alphanumeric() {
            if b[i] == b'\''
                && i > word_start
                && b[i - 1] == b's'
                && b.get(i + 1) == Some(&b's')
                && !b.get(i + 2).is_some_and(|c| c.is_ascii_alphanumeric())
                && (b[word_start].is_ascii_uppercase() || b[word_start].is_ascii_digit())
            {
                out.push('\'');
                i += 2;
                word_start = i;
                continue;
            }
            word_start = i + 1;
        }
        // Copy one whole character (names can hold non-ASCII letters).
        let ch_len = text[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
}

#[cfg(test)]
mod possessive_tests {
    #[test]
    fn names_ending_in_s() {
        let p = super::possessives;
        assert_eq!(
            p("Keep Moon Sweepers's fix in Steam"),
            "Keep Moon Sweepers' fix in Steam"
        );
        assert_eq!(
            p("Halo Infinite's fix, Steam's options"),
            "Halo Infinite's fix, Steam's options"
        );
        assert_eq!(p("the boss's level"), "the boss's level");
        assert_eq!(p("Hades's log; Pokémon's log"), "Hades' log; Pokémon's log");
    }
}
