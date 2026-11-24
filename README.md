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

CachyPlay always runs from its own installed location, never from your Downloads folder. Steam's launch options point to that location by its full path.

**From the AUR (recommended):**

```bash
paru -S cachyplay        # stable
paru -S cachyplay-beta   # beta channel (replaces the stable package)
```

**From a downloaded release:**

```bash
tar -xf cachyplay-v0.1.0-beta.2-x86_64-linux.tar.zst
cd cachyplay-v0.1.0-beta.2-x86_64-linux
./cachyplay-gui          # then press Install at the top, or run: ./cachyplay install
```

Both put CachyPlay in `~/.local/bin` and add it to your app menu. After installing, you can delete the downloaded files. CachyPlay refuses to change games when run from anywhere else.

The desktop app needs WebKitGTK (`webkit2gtk-4.1`), which CachyOS installs with most desktops.

## Use

Open **CachyPlay** from your app menu. Your games are listed on the left; any that crashed recently are pinned at the top under **Needs attention**. Pick a game and CachyPlay tells you in one sentence how it's doing, with one main button for the next step (for example **Apply recommended fix** or **Turn on CachyPlay**). The **Fixes** tab lists every suggestion, the **Settings** tab shows exactly what CachyPlay changes and why, and **History** lists crashes and launches. Then start the game from Steam as usual. The bug icon at the top right saves and lists bug reports; the gear icon opens settings, updates and uninstall.

Close Steam before turning games on or off: Steam rewrites its settings when it exits and would undo the change. The app tells you when Steam is open.

Everything is also available from the terminal:

```bash
cachyplay list                 # your games, and whether CachyPlay is on for each
cachyplay status 1240440       # what CachyPlay changes, and tweaks you made yourself
cachyplay apply 1240440        # turn CachyPlay on for a game
cachyplay undo 1240440         # turn it off and restore your launch options
cachyplay report 1240440       # save a bug report zip to your Desktop
cachyplay doctor               # check this PC for common problems
```

Then start the game from Steam as usual.

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

CachyPlay checks GitHub for a newer version when it starts, and never interrupts you. When one exists, a small **Update available** button appears at the top. Click it to read what's new, then **Update now**. Two clicks, then restart CachyPlay.

- **Stable or Beta:** pick a channel in Settings → Updates. Beta gets new features sooner.
- **Safe updates:** the download is checked against its SHA-256 checksum and test-run before anything is replaced. The previous version is kept, so **Go back to the previous version** in Settings undoes an update.
- **AUR installs** are updated by pacman. The app shows the command (`paru -S cachyplay`, or `paru -S cachyplay-beta` for the beta channel).
- From the terminal: `cachyplay update`, `cachyplay update --check`, `cachyplay channel beta`.

While the GitHub repository is private, updates only work if the GitHub CLI (`gh`) is installed and signed in.

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

These behaviours can't be tested in CI and must be checked once on a real CachyOS + Steam setup:

1. **Steam runs launch options through a shell.** Apply CachyPlay to a game, launch it, and confirm a record appears in `~/.local/state/cachyplay/launches/`.
2. **Fail open with a missing binary.** Temporarily rename the installed binary, launch the game from Steam, and confirm it still starts. Then restore the name.
3. **Bug reports name the game.** Start a game from Steam, then save a general bug report: its summary should name that game under "GPU crashes" if a GPU error happened. This relies on Steam's `logs/console_log.txt` containing "Game process added : AppID …" lines.
4. **Turning off Vulkan add-ons reaches the game.** Apply "Turn off the extra Vulkan add-ons" to a game, start it with MangoHud or a frame-generation layer that's normally on, and confirm it's gone. This checks that `VK_LOADER_LAYERS_DISABLE` passes through Steam's runtime container.
5. **Steam keeps the edit.** Apply with Steam closed, open Steam, and confirm the launch options in the game's Properties match.

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

**Releasing:** set `version` in `Cargo.toml` to exactly match the tag (tag `v0.1.0-beta.3` needs `version = "0.1.0-beta.3"`), then push the tag. The Release workflow refuses mismatches, because the app compares its own version with GitHub's releases.

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
