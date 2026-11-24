//! CachyPlay core library. Every feature lives here so the CLI and the GUI stay thin.

pub mod advice;
pub mod crashmatch;
pub mod diagnose;
pub mod evidence;
pub mod fixes;
pub mod fsutil;
pub mod install;
pub mod keep;
pub mod launch_options;
pub mod library;
pub mod manage;
pub mod obs;
pub mod paths;
pub mod recipes;
pub mod report;
pub mod steam;
pub mod sys;
pub mod update;
pub mod vdf;
pub mod vklayers;
pub mod wrapper;

#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
