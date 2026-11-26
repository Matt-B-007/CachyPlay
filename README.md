<div align="center">
  <img src="assets/cachyplay.svg" alt="CachyPlay logo" width="120" />
  <h1>CachyPlay</h1>
  <p><strong>One-click Linux game tuning, crash help and timestamped bug reports for CachyOS.</strong></p>
</div>

---

CachyPlay sits between Steam and your game. It applies known fixes for that game, shows you every change it makes, records how each launch went, and can undo everything. It never edits your system: drivers, kernels and packages are left alone.

> **Status: 0.1.0 (beta).** Desktop app, command-line tool and core engine. Fix files are examples
> until they are tested on real hardware (`tested = false`).

## Install

One command in a terminal. While the repository is private it uses the GitHub CLI, so sign in once first with `gh auth login` (CachyOS: `sudo pacman -S github-cli` if you don't have it):

```bash
gh api repos/Matt-B-007/CachyPlay/contents/install.sh -H "Accept: application/vnd.github.raw" | sh
```

That's it. It downloads the newest ready-made build from GitHub Releases, checks its checksum, installs it to `~/.local/bin` (its own location, never Downloads), adds it to your app menu and opens it. From then on CachyPlay **updates itself**: when there's a newer version, an **Update to v…** button appears at the top. Two clicks and a restart.

- For the newest beta instead of stable, put `-s -- --beta` after `sh`.
- Once the repository is public, the command becomes `curl -fsSL https://raw.githubusercontent.com/Matt-B-007/CachyPlay/main/install.sh | sh` and no GitHub sign-in is needed.
- Running it again just reinstalls; your settings stay.

**From the zip instead:** on GitHub click **Code → Download ZIP**, extract it, right-click inside the folder → **Open Terminal Here**, and run `sh install.sh`. If no ready-made build of that version is on GitHub yet, it builds CachyPlay on your PC (after asking to install the build tools with pacman). Then you can delete the folder.

Installer options: `--beta`, `--from-source`, `--no-launch`. The desktop app needs WebKitGTK (`webkit2gtk-4.1`), which CachyOS installs with most desktops.

## Use

Open **CachyPlay** from your app menu. Your games are on the left with a traffic light: green runs fine, yellow works with a fix, red not working, grey not played lately. Pick a game and the top of the page tells you, in three lines, **what happened**, **why** (and how sure CachyPlay is), and **what to do**. If there's a one-click fix, press **Fix it**. After you play, CachyPlay asks "Did it work?", and your answer decides what it suggests next.

- **Fixes** lists every suggestion, most likely first, each with Undo.
- **Settings** (per game) shows exactly what CachyPlay changes, **Use the graphics card** (on PCs that also have a chip built into the processor or motherboard), OBS game capture, the line to keep your settings without CachyPlay, and **Reset the game's Windows files** (plus deleting the old copies once you don't need Undo).
- **History** lists crashes (remembered for 30 days, across restarts) and starts (with the graphics chip each one used), and has **Tell the developer**: a short report to post in the game's Steam discussions.
- When a game is broken, a card compares it with games that run fine on your PC.

The gear icon opens Settings: updates, a **PC check** for common Linux gaming problems, light/dark theme (it follows your desktop, and is dark when the desktop doesn't say), big-screen mode for the TV with controller support, optional ProtonDB ratings, and uninstall. Problems the PC check finds also show as a banner.

From a terminal: `cachyplay doctor` (PC check and self-test), `cachyplay suggest <appid>`, `cachyplay fix <appid> <fix>`, `cachyplay reset-prefix <appid> [--undo | --delete-kept]`, `cachyplay card <appid> on|off`, `cachyplay report <appid>`.

Need help? See [SUPPORT.md](SUPPORT.md). Making a release? See [RELEASING.md](RELEASING.md).

## How it works

`apply` adds one segment in front of `%command%` in the game's Steam launch options. Everything you had there stays where it was:

```text
before:  PROTON_LOG=1 %command% -nosplash
after:   PROTON_LOG=1 sh -c '[ -x "$0" ] && exec "$0" run -- "$@"; exec "$@"' /usr/bin/cachyplay %command% -nosplash
```

The `sh -c` part makes it **fail open**: if CachyPlay is missing or broken, the game starts exactly as it would without it. Inside the wrapper, any error or crash while preparing also falls back to launching the game unmodified.

When the game starts, CachyPlay applies the game's fix file (environment variables, game arguments, CachyOS `game-performance`). When it exits, CachyPlay records how long it ran and its exit code.

## Safety

- The original launch options are saved before the first change and never overwritten.
- `localconfig.vdf` is edited in place, changing only the one value. The file is written atomically, re-read to verify, and the last 5 versions are backed up in `~/.local/share/cachyplay/backups/`.
- Undo restores the exact original. If you edited the launch options in Steam afterwards, undo removes only CachyPlay's part and keeps your edits.
- Nothing is written while Steam is running.
- Each Steam account is handled separately.
- Flatpak Steam is detected but not supported yet; CachyPlay won't change anything there.

## Privacy

CachyPlay collects no telemetry. In later versions its only network requests are checking GitHub for fix-file and app updates. Bug reports stay on your machine until you choose to share them. Before a report is written, your home folder path, username, Steam IDs, Steam name, hostname and anything that looks like a token are removed from it.

## Suggested fixes

When a game crashes, CachyPlay works out what to try, on your PC, with no AI and no internet. It looks at:

- GPU driver errors in the kernel log, and which game was running each time
- whether **several different games** are crashing (a driver or system problem) or only one (a game problem)
- Vulkan add-ons that load into every game (frame generation, upscalers, overlays)
- recent driver upgrades, when the game last updated, and the game's own logs (Proton, Unity)

Each suggestion lists the evidence behind it. Fixes for one game (such as turning off Vulkan add-ons for that game, or starting it in a window) are one click and can be undone. System-wide steps (rolling back a driver, resetting GPU clocks) are explained, never done automatically. After you play again, the suggestion shows whether it worked.

From the terminal: `cachyplay suggest <appid>`, `cachyplay fix <appid> <id>`, `cachyplay fix <appid> <id> --undo`.

## Updates

CachyPlay checks GitHub for a newer version when it starts, and never interrupts you. When one exists, an **Update to v…** button appears at the top and in Settings → Updates. Click it to read what's new, then **Update now**. Two clicks, then restart CachyPlay.

- **Stable or Beta:** pick a channel in Settings → Updates. Beta gets new features sooner. Beta builds follow the Beta channel until you choose; on Stable, CachyPlay tells you when a newer beta is out and offers **Switch to Beta and update**.
- **Beta and Stable together:** a beta ranks below the finished version with the same number (`0.1.0-beta.11` < `0.1.0` < `0.1.1-beta.1`). If you're on a beta that's newer than the newest Stable and switch to Stable, CachyPlay offers **Switch to Stable now**; otherwise you get Stable from its next release.
- **Updates come from GitHub Releases.** GitHub publishes one by itself whenever the version number in the code goes up and all tests pass. A release whose build failed is never offered.
- **Safe updates:** the download is checked against its SHA-256 checksum and test-run before anything is replaced. The previous version is kept, so **Go back to the previous version** in Settings undoes an update.
- **AUR installs** are updated by pacman. The app shows the command (`paru -S cachyplay`, or `paru -S cachyplay-beta` for the beta channel).
- From the terminal: `cachyplay update`, `cachyplay update --check`, `cachyplay update --to-stable`, `cachyplay channel beta`.

While the GitHub repository is private, updates only work if the GitHub CLI (`gh`) is installed and signed in.

## OBS game capture

To record just the game in OBS instead of your whole desktop, games need to start through `obs-gamecapture` (the same as putting `obs-gamecapture %command%` in Steam's launch options). CachyPlay does that for you:

1. Install it once: `sudo pacman -S --needed obs-vkcapture lib32-obs-vkcapture`.
2. In CachyPlay, open the game → **Settings** tab → turn on **OBS game capture**. Or turn it on for every game in Settings → OBS game capture.
3. In OBS, add a **Game Capture** source, then start the game from Steam.

From the terminal: `cachyplay obs 1240440 on`, `cachyplay obs all on`. If `obs-gamecapture` isn't installed, games still start normally. It's kept when you uninstall with "Keep the settings". On NVIDIA, capture needs `nvidia-drm.modeset=1`, which current NVIDIA drivers on CachyOS turn on by default.

## Bug reports

Use **Save bug report** on a game to capture that game's logs, or the **Bug reports** button at the top for a general report. Reports are saved to your Desktop as `cachyplay-bug-…-YYYY-MM-DD_HH-MM-SS.zip`, and the Bug reports screen lists them. To get help, open an issue on GitHub and attach the zip. If a game closes early, CachyPlay shows a desktop notification.

## Uninstall

```bash
cachyplay uninstall --keep-settings   # keeps working game settings in Steam, then removes CachyPlay
sudo pacman -Rns cachyplay     # only if you installed from the AUR
```

**Keep what works:** with `--keep-settings` (on by default in the app), each game's working settings, such as a fix you applied, are written straight into its Steam launch options before CachyPlay is removed, so the game keeps running the same way. A copy goes to `CachyPlay game settings.txt` on your Desktop. You can see and copy every game's settings any time in Settings → Your game settings, or with `cachyplay settings`.

Save and config backups are kept unless you pass `--remove-backups`. Bug reports on your Desktop are never deleted. You can also uninstall from the app: Settings → Uninstall CachyPlay.

To remove every trace by hand, close Steam, run `cachyplay uninstall` first (it puts your games back), then:

```bash
rm -f ~/.local/bin/cachyplay ~/.local/bin/cachyplay-gui
rm -f ~/.local/share/applications/cachyplay.desktop ~/.local/share/icons/hicolor/scalable/apps/cachyplay.svg
rm -rf ~/.local/share/cachyplay ~/.local/state/cachyplay ~/.cache/cachyplay ~/.config/cachyplay
```

## Verify on a real system before release

Things CI can't test. Confirmed on a real CachyOS + Steam PC so far:

- **Steam runs launch options through a shell** (launch records appear in `~/.local/state/cachyplay/launches/`).
- **Steam's `console_log.txt` names the game** ("Game process added : AppID …" lines), so bug reports and the Proton history know which game crashed.
- **Fail open**: games start normally when CachyPlay is missing.

Still to check once per release on a real PC:

1. **Turning off Vulkan add-ons reaches the game.** Apply "Turn off the extra Vulkan add-ons" to a game, start it with MangoHud or a frame-generation layer that's normally on, and confirm it's gone. This checks that `VK_LOADER_LAYERS_DISABLE` passes through Steam's runtime container.
2. **Use the graphics card** (PCs with a built-in chip too). Start a game, then check its History tab: the start should say "On NVIDIA graphics card" (or AMD). This checks that the settings pass through Steam's runtime and that CachyPlay can see which chip the game opened.
3. **Steam keeps the edit.** Apply with Steam closed, open Steam, and confirm the launch options in the game's Properties match.
4. **Update and Switch to Stable now** work from the app with the new release.

## Build from source

```bash
sudo pacman -S --needed rust webkit2gtk-4.1   # build dependencies on CachyOS / Arch
cargo test --workspace
cargo build --workspace --release             # target/release/cachyplay and cachyplay-gui
```

| Crate | What it is |
| --- | --- |
| `crates/cachyplay-core` | All logic: Steam files, snapshots, fix files, launch wrapper, bug reports |
| `crates/cachyplay` | Command-line tool and the launch wrapper Steam calls |
| `crates/cachyplay-gui` | Desktop app (Tauri 2, plain HTML/CSS/JS in `ui/`) |

**Releasing:** bump `version` in `Cargo.toml` and push to `main`. GitHub runs the tests and, if they pass, tags and publishes the release by itself. See [RELEASING.md](RELEASING.md).

The desktop app is a separate program on purpose: the launch wrapper runs before every game, so it stays small and starts instantly without loading WebKit.

## Roadmap

| Version | Focus |
| --- | --- |
| **v0.1.0** | Desktop app, command-line tool, core engine; 5 tested fix files |
| v0.2.0 | Signed fix-file updates, one-click crash fixes, Proton manager, save backups |
| v0.3.0 | Hardware-aware fixes, driver-vs-game crash triage, pre-flight checks |
| v1.0.0 | Heroic and Lutris, gamepad navigation, Gamescope and upscaler options |

## License

MIT. See [LICENSE](LICENSE). The bundled Manrope font is under the SIL Open Font License (`crates/cachyplay-gui/ui/fonts/OFL-Manrope.txt`).
