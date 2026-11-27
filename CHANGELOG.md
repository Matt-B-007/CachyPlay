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
v0.1.0-beta.9 (plain-language diagnosis, Fix it, PC check, Proton history, settings page, themes),
v0.1.0-beta.10 (crash times no longer depend on the PC's time zone; dependency updates),
v0.1.0-beta.11 (graphics card vs built-in chip, crash history across restarts, accurate bug reports,
automatic releases), v0.1.0-beta.12 (status steps, live updates, Try these settings, Steam handling,
Find the cause, GPU logging; never published because a test failed on GitHub),
v0.1.0-beta.13 (everything from beta.12, plus the test fix), v0.1.0-beta.14 (every change
undoable and remembered, no constant refreshing, reliable start records, per-game fixes),
v0.1.0-beta.15 (never published: a script check failed on GitHub), v0.1.0-beta.16 (everything
planned for beta.15: the window never freezes, signed updates, one window, crash notice; plus the
script fix), v0.1.0-beta.17 (the family look: new colours, fonts, icons and logo, Help menu,
Ctrl+K search, keyboard shortcuts, first-run tour, standard update/report/uninstall dialogs,
optional tray icon).

### Added
- **Help menu (beta.17)**: the **?** button at the top right has Report a problem, What's new,
  Keyboard shortcuts, Show the tour and About, in the same place as in the other apps.
- **Search everything with Ctrl+K (beta.17)**: games, the open game's fixes, every Settings
  section, and actions like Report a problem, Check for updates or Close Steam for me.
- **Keyboard shortcuts (beta.17)**: Ctrl+K search, Ctrl+, Settings, **?** this list, F1 the Help
  menu, **/** search games, Esc back, Ctrl+Q quit. Buttons show their shortcut when you hover.
- **First-run tour (beta.17)**: five short steps (the game list and its traffic lights, What
  happened, Fix it, the Fixes tab, Settings). It starts by itself only on a new install, can be
  skipped, and is always in Help › Show the tour. After an update, CachyPlay offers What's new
  instead.
- **Report a problem (beta.17)**: one dialog for every report: a short summary, what happened, and
  a preview of what's included, then **Save to file**, **Copy report** or **Open log folder**.
  Sending is a separate step: **Open on GitHub** (fills in the summary and system details and asks
  you to attach the file; it says when only the owner can see the repository), **NVIDIA forum**,
  or the game's **Steam discussions**. Tell NVIDIA, Tell the developer, every error message and the
  "closed unexpectedly" notice open it. Reports now start with the app, version, channel, install
  type, distro, desktop, Wayland/X11 and kernel.
- **Skip this version (beta.17)**: in the update dialog, with Undo. Settings › Updates shows when
  CachyPlay last checked, and the daily check can be turned off.
- **Tray icon (beta.17, optional)**: off unless turned on in Settings › General. Lit while a game
  runs, a red badge when CachyPlay is off for the game you're playing, an amber dot while a fix is
  being tested, a change waits for Steam, or an update is ready, a red dot after a crash, grey when
  Steam isn't found. It needs libayatana-appindicator (CachyPlay says so if it's missing). When the
  tray is off, a crash while CachyPlay is in the background shows a desktop notification.
- **Undo right in the message (beta.17)**: resetting a game's Windows files or shader cache, and
  turning CachyPlay off for a game, show an Undo button in the message that confirms it.
- **Signed updates (beta.16)**: every release is signed, and CachyPlay checks the signature (two
  built-in keys: the one GitHub uses and an offline backup) as well as the checksum. A release
  without a signature isn't offered, and a tampered one is refused with nothing changed. Releases
  are published as drafts first, so an update is never seen half-uploaded. Releases from before
  beta.16 can't be installed by the updater any more; **Go back** still works from the copy on your
  PC. Checksums are now in `SHA256SUMS`; `SHA256SUMS.txt` is still published for beta.14 and older,
  until no supported version needs it.
- **The new version checks itself before it's installed (beta.16)**: `--self-test --quick` (starts,
  finds its files, reads your settings; no network) runs on the download before it replaces the
  current version. If it fails, you keep the version you have.
- **One window (beta.16)**: opening CachyPlay while it's already open brings the open window to the
  front instead of starting a second copy. The launch helper Steam uses is never affected.
- **"CachyPlay closed unexpectedly last time" (beta.16)**: with **Report** and **Dismiss**, when the
  app didn't close normally. A crash trace is saved when CachyPlay itself crashed (the last 10 are
  kept), and errors in the window are written to the log. Logging out or shutting down closes it
  normally, so that doesn't count.
- **Standard command-line flags (beta.16)**: `--self-test` (same as `doctor`), `--self-test --quick`,
  `--report`, `--update`, `--rollback`, `--uninstall`. The installer gains `--version X` and
  `--uninstall`, and checks signatures when minisign is installed.
- **Install and release scripts pass shellcheck (beta.16)**: GitHub now checks `install.sh` and the
  release scripts with shellcheck on every push. beta.15 was never published because its check
  found one line in `scripts/version-guard.sh` written in a way that's easy to misread; it's now a
  plain `if`.
- **One check script (beta.16)**: `scripts/check.sh` runs exactly what CI runs, and CI runs only
  that script, so a check that passes before a release passes on GitHub too. Rust is pinned in
  `rust-toolchain.toml` and shellcheck to 0.11.0, so a new tool release can't break CI overnight.
  CI steps are named for what they run.
- **Public DNS fallback (beta.16)**: if the PC's DNS can't find GitHub, updates try again through
  public DNS over HTTPS before giving up.
- **Applied for this game (beta.14)**: one card listing every fix, your settings, Proton switch and
  shader cache CachyPlay applies to a game, each with Undo, plus Undo all. It no longer disappears
  when a suggestion stops matching (Direct3D 12 couldn't be undone in the app).
- **Every try remembered (beta.14)**: a change that crashed, black-screened, or was undone because it
  made things worse stays ruled out for that game, with the reason and a date, and isn't offered
  again until you choose **Try again**. Undo asks why: made things worse, didn't help, or just
  testing. The timeline shows when each change went on and off.
- **Broken graphics modes spotted (beta.14)**: when a forced mode makes the game log missing shader
  pieces or the same rendering error over and over (SHRED A BILLION in Direct3D 12), that mode is
  ruled out for the game on the first start.
- **One change at a time (beta.14)**: CachyPlay asks before a second change goes in before the game
  has started, and the status names every change being tested together.
- **"Black screen" (beta.14)** in What happened?, counted as a problem even without a driver crash.
- **Status steps (beta.12)**: Testing the fix → Seems to work → Works with a fix, naming the fix.
  A clean run is twice the longest recent crash time (10–60 minutes); "Works with a fix" needs 3
  clean runs, or 2 adding up to an hour. Crashes from before the fix no longer keep a game red, every
  start in Steam's log counts (CachyPlay on or off), and a crash in the last seconds of a start counts
  against it. Online-only problems count only online starts.
- **Live updates (beta.12)**: the app refreshes by itself when a game starts, ends or crashes, shows
  "Playing now, 12 of 18 min", and follows Steam opening, closing ("Steam is closing…") or hiding in
  the tray.
- **Try these settings (beta.12)**: per-game settings and game options applied at launch, without
  touching Steam's launch options. Works with Steam open, and counts as a tested change.
  `cachyplay try <appid> "..."`.
- **Steam handling (beta.12)**: turning CachyPlay on or off, switching Proton and resetting Windows
  files wait for Steam to close and then happen by themselves; **Close Steam for me** and **Open
  Steam**; **Put CachyPlay back** when its part of the launch options was replaced in Steam.
- **Recent starts timeline (beta.12)**: plain outcomes, crashes inside their starts, what was on
  for each start, clean-run progress, dividers for fixes, Proton switches, driver and game updates,
  details on click, and **What happened?** (ran fine, stutter, glitches, crashed, network lag;
  online or offline).
- **Find the cause (beta.12)**: the tests in order with what's ruled out, including settings tried in
  Steam's launch options. Confidence labels Likely / Possible / Long shot that move with the
  evidence, "what would prove it" on each suggestion, and the last remaining suspect rising.
- **New suggestions (beta.12)**: fresh shader cache (with Undo), NVIDIA raw-access-chains off for Xid
  109 hangs, performance recording (MangoHud, one start), "it only happens online", **Big step: go
  back to the previous NVIDIA driver** (only when the evidence fits and the old driver is in pacman's
  cache; never automatic; undo first), and **Report it to NVIDIA** after the same error 3 times, with
  ready-made text. Fixes that worked for another game on this PC are offered with that evidence.
- **GPU log (beta.12)**: temperature, power and clocks every 2 seconds while a game runs, summed up
  in bug reports ("cool and steady", "waiting on stuck work", or "hot").
- **Driver timeline (beta.12)**: "first crash came 4 days after nvidia-utils 620.73.04 was installed;
  it ran 3 times before without driver errors", in reports and the driver suggestion.
- **Use the graphics card (beta.11)**: on PCs that also have a graphics chip built into the
  processor or motherboard, CachyPlay points the games it's on for at the graphics card (NVIDIA:
  PRIME offload settings; AMD/Intel cards: Mesa's device selection; DirectX games through DXVK and
  VKD3D too). On by default, with a switch per game. Settings you set yourself win.
- **Which chip each start used (beta.11)**: shown in the game's History, and the game page says so
  near the top if a game ran on the built-in chip.
- **Monitor check (beta.11)**: the PC check warns when the monitor is plugged into the motherboard
  instead of the graphics card. Laptops are recognised and get a note instead.
- **Crash history across restarts (beta.11)**: graphics driver crashes are remembered for 30 days.
  Before, the kernel log only reached back two starts of the PC.
- **Fuller general bug reports (beta.11)**: every game and its state, PC check and self-test
  results, the driver and kernel update timeline, hardware hints (like a USB device that fails to
  start every time), and every installed graphics and gaming package.
- **Delete old copies of Windows files (beta.11)** set aside by resets, with the space they use; the
  PC check says how much space they take. `cachyplay reset-prefix all --delete-kept`.
- **Switch to Stable now (beta.11)**: on a beta that's newer than the newest stable, choosing Stable
  offers to install that stable version straight away. `cachyplay update --to-stable`.
- **Automatic releases (beta.11)**: pushing a new version number to GitHub tests, tags and
  publishes it by itself. Nothing is published if a test fails, and a version that would rank below
  an existing release is refused.
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

### Changed
- **The family look (beta.17)**: the same colours, corners, spacing and type as Gimbot, Sluice,
  StowLock and Stream Preflight: navy-tinted dark and light backgrounds, cards with a faint accent
  edge, small accent corner brackets on the focused card and the selected game, a soft glow only on
  the main action (Fix it) and the focused control, and a faint dot grid behind empty areas (none
  when motion is reduced). Every colour lives in one file (`ui/tokens.css`), and a test checks that
  every text colour reaches 4.5:1 on every surface in both themes, plus the ink on accent buttons.
- **New accent: aerospace orange (beta.17)**: CachyPlay's colour changes from cyan to orange
  (#FF7A45, a darker #B23A0A in the light theme), so each app in the family has its own. System
  theme now follows only your desktop's light or dark setting; the accent is always CachyPlay's
  own (it no longer takes KDE's accent colour). Warnings stay amber and errors red, and every banner
  starts with its status icon, so a warning never looks like the accent.
- **Fonts and icons (beta.17)**: Inter for text and JetBrains Mono for code and logs (both built
  in, nothing downloaded), and Lucide icons.
- **New logo (beta.17)**: the shield and play button in solid orange on a dark navy rounded square,
  no gradient, sharp down to 16 px. The tray icon's "game running" state is orange too. Installed in every size from 16 to 512 px.
- **Top bar (beta.17)**: update button (only when there's an update), Help, Settings. Bug reports
  you saved moved to Settings › Privacy.
- **Settings in the standard order (beta.17)**: General, Appearance, Updates, Privacy, PC check, OBS
  game capture, Your game settings, Advanced, About & uninstall (with the red Uninstall box last).
- **Update dialog (beta.17)**: "CachyPlay X is available" with a Beta tag, "You have X", release
  notes, **Update now · Later · Skip this version**, a progress bar, then "Update installed. Restart
  to use it." On a beta while following Stable it says it installs the newest normal version.
- **Uninstall dialog (beta.17)**: says what's removed and what isn't touched. **Also delete my
  settings and logs** is off by default, so settings and logs now stay unless you tick it (the
  command line has `--delete-settings`). Ends with "CachyPlay is uninstalled. Thanks for trying it."
- **While the window is hidden (beta.17)**: still no window updates, but a light look every 10
  seconds keeps the tray icon or crash notifications current.
- **Version numbers stay in bug reports (beta.17)**: numbers like `0.11.0.1` after `==`, `v`, `@`,
  `-` or a name are no longer mistaken for IP addresses.
- **`scripts/check.sh` (beta.17)**: lists every missing tool at once with the CachyOS/Arch install
  command, and exits with 3 when only tools are missing (1 when a check fails). Colours only in a
  terminal, never with NO_COLOR. A different shellcheck version on a PC is a note, not a failure
  (GitHub still requires the pinned one).

- **The window never freezes (beta.16)**: everything that reads the disk, Steam or other programs now
  runs in the background, not on the window's thread (46 actions moved), so scanning a big library
  or checking the PC no longer stops the window responding.
- **No more checking every 3 seconds from the window (beta.16)**: a watcher in the background tells
  the window only when something changed (Steam opened or closed, a game started, ended or crashed,
  a waiting change ran). Nothing runs while the window is minimized or hidden, it looks less often
  while unfocused, and the controller check pauses too. Changes waiting for Steam to close still run
  while the window is hidden. Switching back to the window no longer reloads everything.
- **Cover art loads directly (beta.16)** from Steam's cache through a `cover://` link that can only
  return game covers, instead of being copied through the app as text.
- **Settings file has a format number (beta.16)**: older files are upgraded automatically, a copy is
  kept at most every 15 minutes (5 kept), and every file CachyPlay saves is written to a temporary
  file first and then swapped in, so a crash can't leave half a file.
- **Logs (beta.16)**: at most 1 MB per file with 3 files kept (was 2), and bug reports include this
  session's last 500 lines and the latest crash trace.
- **Bug report placeholders match the other apps (beta.16)**: `<user>` (was `$USER`), `<host>` (was
  `<hostname>`), `<ip>` for every IP address, `<secret>` (was `<redacted>`), Windows-style paths
  with the drive letter, and GitHub tokens and Bearer tokens are now hidden too.

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
- Constant refreshing (beta.14): the app no longer redraws every few seconds because Steam writes to
  its console log; only games starting or ending, crashes and changes count. While a game runs,
  only the "Playing now" card ticks over, and opened starts stay open.
- Start records (beta.14): one-start fixes (detailed Proton log, performance recording) come off when
  the start begins, so they never stay on; the GPU logger takes short readings from a thread, so
  nothing can be left running; reports use Steam's end time when CachyPlay's record wasn't finished;
  CachyPlay's log notes every start and how it ended.
- Short starts (beta.14): a start that closed early without a crash no longer counts as a failure for
  a fix or a Proton switch; finished starts count their real play time (170 s showed as 0).
- Other games' fixes (beta.14): only mentioned for similar games (same Unity version and graphics
  mode, or same NVIDIA hang and translation layer), with the reason, and never above Possible.
- NVIDIA advice (beta.14): says per game whether the raw-access-chains switch is the confirmed case
  (Direct3D 11 through DXVK) or only reported (Direct3D 12 through vkd3d). A game that only runs with
  that NVIDIA feature off now counts as evidence toward the driver, and the NVIDIA report says so.
- Driver timeline (beta.14): ignores kernel-module rebuilds (same version, new release) and games
  first played after the update.
- Reports (beta.14): the "several games" section says where each game stands; game update times are
  in local time (they were UTC); Proton names are plain ("GE-Proton11-11"); "Share that this worked"
  only appears once a fix reaches "Works with a fix" or you confirm it.
- Tests and launch helper (beta.13): starting a program that was written a moment earlier no longer
  fails with "Text file busy"; it waits a moment and tries again. This made one test fail on GitHub
  and kept beta.12 from being published.
- Window icon (beta.12): the title bar and taskbar show the CachyPlay icon instead of KDE's white
  "W". The menu entry is now named after the app's ID (plus a hidden alias), and the app fixes up
  older installs by itself.
- Bug reports (beta.12): a Proton log from an earlier start is labelled as such; the detailed Proton
  log is written to CachyPlay's own folder (so proton-cachyos logs are found) and turns itself off
  after one start; reports say when CachyPlay was off and how many newer starts ran without it;
  crash lists in another game's report say "none of these are from this game"; status, timeline,
  GPU log and performance recording are included.
- Advice (beta.12): a setting after `%command%` is no longer called "does nothing" when CachyPlay
  applies it anyway. The "Keep these settings" line moves misplaced settings in front without
  duplicates and warns that pasting it turns CachyPlay off.
- Driver downgrades appear in the driver timeline (pacman's "downgraded" lines).
- Dependabot removed (beta.12): one CI row on main and one Release row per version tag in Actions;
  `cargo audit` in CI still fails on libraries with known security problems.
- 32-bit driver check (beta.11): no more "32-bit AMD driver missing" on PCs with an NVIDIA card and
  a Ryzen processor's built-in graphics. It now checks the driver library and pacman's records
  instead of a file name Mesa changed, and only for the chip games draw on.
- Bug reports (beta.11) no longer blank out version numbers that look like network addresses, and
  now hide Steam's device ID and lobby IDs. Harmless kernel messages are left out.
- Restore all and Uninstall (beta.11) put back any Proton version CachyPlay switched (or keep it, with
  "Keep the settings"), keep the "Use the graphics card" settings with "Keep the settings", and
  say where old copies of Windows files are.
- Updates (beta.11) never offer a release that has no download attached.
- Theme (beta.11): "Follow my desktop" reads the desktop's own setting, and is dark when it doesn't
  say (before, it often turned light).
- Crash times (beta.10) are kept as the wall-clock time they were written, like Steam's own log,
  instead of being converted to the PC's current time zone. Before, a time-zone or daylight-saving
  change could stop crashes from lining up with the game that was running; it also made two tests
  fail on GitHub's UTC machines. Tests now also run in another time zone in CI.
- Dependencies updated (beta.10): zip 9, toml 1, base64 0.23, minisign-verify 0.3,
  actions/checkout v7, action-gh-release v3.
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
