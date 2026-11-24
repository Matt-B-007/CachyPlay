# Changelog

All notable changes are listed here. Versions follow [Semantic Versioning](https://semver.org/).

## [0.1.0] - Unreleased

Beta builds: v0.1.0-beta.1 (core + command line), v0.1.0-beta.2 (desktop app, updates, bug reports),
v0.1.0-beta.3 (bug reports name the crashing game, privacy fixes),
v0.1.0-beta.4 (suggested fixes worked out from crashes, logs and your system),
v0.1.0-beta.5 (see and keep your game settings without CachyPlay),
v0.1.0-beta.6 (new layout: game list on the left, one clear status and button per game, tabs).

### Added
- New app layout (beta.6): a searchable game list on the left with crashing games pinned under
  "Needs attention"; each game opens with one plain sentence about its state and one main button
  (for example "Apply recommended fix"); Overview, Fixes, Settings and History tabs; icon buttons for
  bug reports and settings; a banner when the graphics driver crashed in several games, which points
  at the driver or a shared add-on rather than the games. Arrow keys move through the game list and tabs.
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

- Updates from GitHub Releases with a Stable or Beta channel: two clicks in the app, or `cachyplay update`.
  Downloads are checked against SHA-256 checksums and test-run before installing; the previous version
  is kept for one-click roll back. AUR installs show the paru command instead.
- Bug reports screen: list of saved reports, general (not game-specific) reports, link to GitHub issues.
- Desktop notification when a game closes early.

- Bug reports name the game that crashed: GPU driver errors in the kernel log are matched against
  Steam's record of which game was running, and CachyPlay's own launches. A general report is filed
  under the most recently crashed game automatically.
- Bug reports list GPU crashes newest first, explain NVIDIA Xid codes in plain English, and flag when
  several different games hit driver errors (a driver or system problem, not one broken game).
- Game reports include the game's Unity `Player.log`, when Steam last updated the game, which Proton
  version it uses, the kernel log from the previous boot, recent driver upgrades from pacman's log,
  the full Vulkan GPU list and loaded Vulkan layers.

- Suggested fixes: CachyPlay works out what to try from GPU crashes, which games were affected,
  Vulkan add-ons loaded into every game, driver upgrades, game updates and the game's own logs. No AI
  and no internet: fixed rules, each showing the evidence behind it. Fixes for one game apply with
  one click and can be undone; system-wide steps (driver rollback, GPU clocks) are explained, never
  done automatically. CachyPlay tracks whether a fix worked after the next launch. Also in bug
  reports and `cachyplay suggest` / `cachyplay fix`.
- Detects always-on third-party Vulkan add-ons (implicit layers) from their manifests and can turn
  them off for one game (`VK_LOADER_LAYERS_DISABLE`).
- "Crashed recently" badge and crash card for games the GPU driver crashed in, even when CachyPlay
  isn't turned on for them.

- "Your game settings" in Settings, and on each game: every setting CachyPlay applies, written as a
  Steam launch-options line you can copy, plus "Save a copy to Desktop" (`CachyPlay game settings.txt`).
  Also `cachyplay settings [--export]`.
- Uninstall can keep the settings that made your games work: each game's settings are written straight
  into its Steam launch options (no CachyPlay needed) and a copy is saved to the Desktop. On by
  default in the app; `cachyplay uninstall --keep-settings` in the terminal.

### Fixed
- Bug reports no longer contain MAC addresses, IP addresses or the GPU's unique ID, and firewall log
  lines are left out entirely.
- General reports no longer say "this game"; duplicate package lines removed.
- Steam account detection when several account folders exist and none is marked "most recent".
  Uninstall and "Restore all games" no longer need the current account at all.

### Not yet included
- Tested fix files (the bundled Halo Infinite file is an untested example).
