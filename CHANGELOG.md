# Changelog

All notable changes are listed here. Versions follow [Semantic Versioning](https://semver.org/).

## [0.1.0] - Unreleased

### Added
- Desktop app (`cachyplay-gui`, Tauri 2): game library with Steam cover art and status badges, a
  launch-path view, a before/after changes table with plain-English reasons, an on/off switch per game,
  a crash card after failed launches, one-click bug reports, Install, Restore all, and Uninstall.
- Core engine (`cachyplay-core`) and command-line tool (`cachyplay`).
- Native Steam discovery: libraries, installed games, logged-in account (per-account handling).
- Byte-preserving `localconfig.vdf` editing with snapshots, atomic writes, verification and 5 rotating backups.
- `apply` / `undo` / `restore-all`: adds a fail-open launch wrapper while keeping the user's own launch options.
- Launch wrapper `cachyplay run`: applies fix files, records runtime and exit code, falls back to an unmodified launch on any error.
- Fix-file format (schema 1) with a fixed list of actions, allowed-variable list and hardware conditions.
- Detection of user tweaks: renamed files/folders, drop-in DLL and ASI mods, own launch options.
- Timestamped, redacted bug report zip on the Desktop with known crash-pattern matching.
- `install` copies a downloaded build to `~/.local/bin`; `uninstall` restores every game first.
- Refuses to edit games while Steam is running, under Flatpak Steam, or when not installed.

### Not yet included
- Tested fix files (the bundled Halo Infinite file is an untested example).
