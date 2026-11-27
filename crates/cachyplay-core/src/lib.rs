//! CachyPlay core library. Every feature lives here so the CLI and the GUI stay thin.

pub mod advice;
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
