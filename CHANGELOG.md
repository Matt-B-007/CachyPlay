# Changelog

All notable changes are listed here. Versions follow [Semantic Versioning](https://semver.org/).

## [0.1.0] - Unreleased

Beta builds: v0.1.0-beta.1 (core + command line), v0.1.0-beta.2 (desktop app, updates, bug reports),
v0.1.0-beta.3 (bug reports name the crashing game, privacy fixes),
v0.1.0-beta.4 (suggested fixes worked out from crashes, logs and your system),
v0.1.0-beta.5 (see and keep your game settings without CachyPlay),
v0.1.0-beta.6 (new layout: game list on the left, one clear status and button per game, tabs),
v0.1.0-beta.7 (one-command installer, working update button, OBS game capture),
v0.1.0-beta.8 (installer falls back to the newest beta; OBS test fix),
v0.1.0-beta.9 (plain-language diagnosis, Fix it, PC check, Proton history, settings page, themes).

### Added
- **Three-line summary (beta.9)** at the top of every game: what happened, why (with how sure
  CachyPlay is: Likely, Possible or Not sure yet), and what to do. One **Fix it** button applies the
  most likely one-click fix, and says what it will try.
- **Traffic lights (beta.9)** in the game list: green runs fine, yellow works with (or is trying) a
  fix, red not working, grey not played lately.
- **"Did it work?" (beta.9)**: after playing with a fix, CachyPlay asks once. The answer beats its
  own guess and decides the next suggestion. While a fix is being tried, no other fix is stacked.
- **Learning from games that work (beta.9)**: a comparison card puts a broken game next to games
  that ran long and clean on the same PC (Proton version, graphics mode, launch options), and
  offers settings a working game uses, one at a time.
- **Proton history (beta.9)**: which Proton each start used and how it went; one-click switch with
  Undo; "you've tried 3 versions, switching won't help"; help installing proton-cachyos or
  GE-Proton when they're missing.
- **Game-update detection (beta.9)**: when a game broke right after its own update and other
  games still run fine, CachyPlay says so instead of blaming the driver.
- **Unity tips (beta.9)**: reads the game's own log; suggests trying `-force-d3d12` /
  `-force-vulkan` for Direct3D 11 games that crash, and turning off the game's own frame
  generation.
- **Anti-cheat warning (beta.9)** before anyone tries fixes that can't work.
- **PC check (beta.9)**: games on NTFS drives, missing 32-bit drivers, two graphics chips, NVIDIA
  mode-setting, game performance mode, ntsync, memory map limit. Problems show as a banner with a
  command to copy. Also in `cachyplay doctor`.
- **Reset the game's Windows files (beta.9)** (Proton prefix): set aside, never deleted, with Undo.
  Also `cachyplay reset-prefix <appid> [--undo]`.
- **Tell the developer (beta.9)**: a short, specific report (version, when it broke, driver errors,
  graphics mode, Proton versions tried, games that work) with Copy and a link to the game's Steam
  discussions. **Share that this worked** opens a prefilled GitHub issue for fixes that helped.
- **Optional ProtonDB rating (beta.9)**, off by default; only the game's number is sent; kept a week.
- **Settings page (beta.9)** with a sidebar (Updates, PC check, Display, Privacy, OBS, your game
  settings, self-test, restore and uninstall), **light theme**, **KDE accent color**, and
  **big-screen mode** with controller support for the TV / Big Picture.
- **Self-test (beta.9)**: Settings → About & self-test, or `cachyplay doctor`.
- `cachyplay worked <appid> <fix> yes|no` answers "Did it work?" from the terminal.
- One-command installer (beta.7): `sh install.sh` in the unzipped folder. It downloads the
  ready-made build of that version from GitHub Releases (checksum-checked), or builds it on your PC
  if there isn't one, after asking to install the build tools. Also works piped from `gh api` with
  nothing downloaded, and inside release downloads. Opens CachyPlay when done.
- OBS game capture (beta.7): a switch per game (game → Settings tab) or for every game, which
  starts games through `obs-gamecapture` so OBS's Game Capture source records just the game.
  Skipped if obs-vkcapture isn't installed, kept on uninstall with "Keep the settings",
  `cachyplay obs <appid|all> on|off` in the terminal.
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

### Reliability (beta.9)
- Every change to Steam's files is checked after writing: if anything other than the intended
  setting changed, or the file doesn't read back as written, the original is put back.
- If Steam's settings file is in a format CachyPlay doesn't know, it changes nothing and keeps only
  its read-only features until an update supports it.
- A copy of CachyPlay's settings is kept before a new version first uses them.
- Launch records and logs are trimmed so they never fill a drive (bug reports are never deleted).
- The background update check asks GitHub at most once a day; every feature but updates works offline.
- Optional release signing (minisign): once a key is set up, unsigned updates are refused.
- Tests for the launch helper's speed and every way it could fail, for your real Moon Sweepers case,
  for Steam libraries with spaces and symlinks; CI on an Arch-based system; dependency audit and
  automatic dependency updates; `scripts/release-check.sh`, RELEASING.md and SUPPORT.md.

### Fixed
- Launches are recorded when they start (beta.9), so a game that hangs after a GPU crash shows as
  "running or stopped responding" instead of "CachyPlay isn't turned on".
- The driver-update list includes every NVIDIA package (nvidia-utils, lib32-nvidia-utils,
  nvidia-settings, …), Mesa's Vulkan drivers and CachyOS kernels with NVIDIA modules (beta.9).
- Steam libraries listed through a symlink are no longer counted twice (beta.9).
- Installer (beta.8): when no stable release exists yet, it installs the newest beta instead of
  stopping with "Couldn't find a CachyPlay release", and its error now says why nothing was found.
- Tests (beta.8): the OBS capture test no longer fails on PCs that have obs-vkcapture installed.
- Updates (beta.7): Settings → Updates now has an **Update to v…** button (before, it only said an
  update existed). Beta builds follow the Beta channel until you pick one, so they see newer betas;
  on Stable, a newer beta is offered with **Switch to Beta and update**. When nothing has been
  published on GitHub yet, it says so instead of claiming you're up to date.
- Bug reports no longer contain MAC addresses, IP addresses or the GPU's unique ID, and firewall log
  lines are left out entirely.
- General reports no longer say "this game"; duplicate package lines removed.
- Steam account detection when several account folders exist and none is marked "most recent".
  Uninstall and "Restore all games" no longer need the current account at all.

### Not yet included
- Tested fix files (the bundled Halo Infinite file is an untested example).
