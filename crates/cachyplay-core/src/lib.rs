//! CachyPlay core library. Every feature lives here so the CLI and the GUI stay thin.

pub mod diagnose;
pub mod fsutil;
pub mod install;
pub mod launch_options;
pub mod library;
pub mod manage;
pub mod paths;
pub mod recipes;
pub mod report;
pub mod steam;
pub mod sys;
pub mod vdf;
pub mod wrapper;

#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
