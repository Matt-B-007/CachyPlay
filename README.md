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
tar -xf cachyplay-v0.1.0-x86_64-linux.tar.zst
cd cachyplay-v0.1.0-x86_64-linux
./cachyplay-gui          # then press Install at the top, or run: ./cachyplay install
```

Both put CachyPlay in `~/.local/bin` and add it to your app menu. After installing, you can delete the downloaded files. CachyPlay refuses to change games when run from anywhere else.

The desktop app needs WebKitGTK (`webkit2gtk-4.1`), which CachyOS installs with most desktops.

## Use

Open **CachyPlay** from your app menu. Pick a game, read what CachyPlay would change and why, and flip the switch. Then start the game from Steam as usual. If a launch fails, the game shows a "Last launch failed" badge with a button to save a bug report.

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

## Uninstall

```bash
cachyplay uninstall            # restores every game first, then removes CachyPlay's files
sudo pacman -Rns cachyplay     # only if you installed from the AUR
```

Save and config backups are kept unless you pass `--remove-backups`. Bug reports on your Desktop are never deleted.

## Verify on a real system before release

These behaviours can't be tested in CI and must be checked once on a real CachyOS + Steam setup:

1. **Steam runs launch options through a shell.** Apply CachyPlay to a game, launch it, and confirm a record appears in `~/.local/state/cachyplay/launches/`.
2. **Fail open with a missing binary.** Temporarily rename the installed binary, launch the game from Steam, and confirm it still starts. Then restore the name.
3. **Steam keeps the edit.** Apply with Steam closed, open Steam, and confirm the launch options in the game's Properties match.

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
