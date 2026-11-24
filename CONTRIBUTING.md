# Contributing to CachyPlay

Thanks for helping get more games working on Linux.

## Fix files (recipes)

Fix files live in `recipes/<appid>.toml`. They are declarative: they can only use the actions and environment variables CachyPlay allows, never shell commands.

**Rules for a new or changed fix file:**

1. Test it yourself, playing at least 15 minutes, before setting `tested = true`.
2. Say in the pull request which GPU (vendor and model), driver version and Proton build you tested with.
3. Ideally, someone with the other GPU vendor (AMD vs NVIDIA) confirms it too. Use `when = "gpu.vendor == '...'"` for vendor-specific settings.
4. Every setting needs a plain-English `why`. It is shown to users.
5. Prefer the smallest fix that works.

Allowed actions in 0.1.0: `add_game_arg`, `use_game_performance`. Recognised but applied from 0.2.0 on: `set_runner`, `winetricks`, `dll_override`, `purge_shader_cache`, `patch_config`.

Allowed environment variable prefixes: `PROTON_`, `DXVK_`, `VKD3D_`, `WINE` (except `WINEPREFIX`, `WINESERVER`, `WINELOADER`), `RADV_`, `MESA_`, `__GL_`, `MANGOHUD`, `SDL_`, `STAGING_`, `ENABLE_`.

Conditions can test `gpu.vendor`, `gpu.count`, `session`, `cpu.level` and `ntsync`, combined with `&&` and `||`.

## Code

- Run `cargo fmt`, `cargo clippy --all-targets` and `cargo test` before opening a pull request.
- Anything that writes to Steam's files goes through `manage.rs`, which keeps the snapshot, backup and verification rules in one place.
- The launch wrapper must stay fail-open. Any new step in `wrapper::prepare` must not be able to stop a game from launching.

## Bug reports

Please attach the zip from `cachyplay report <appid>`. Personal details are removed before it's written, but you can open it and check before attaching.
