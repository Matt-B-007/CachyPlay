# Releasing CachyPlay

Run the checklist every time. Most release mistakes so far (a version that didn't match its tag,
a missing release, a test that only passed on one PC) are caught by it.

## Checklist

Releases are automatic. GitHub publishes a release whenever the version in `Cargo.toml` is one
that hasn't been released yet. There are no tags to make by hand.

1. Bump `version` in the root `Cargo.toml` (`0.1.0-beta.N`, or `0.1.0` for stable), then
   `cargo update --workspace`.
2. Add the changes to `CHANGELOG.md` under that version.
3. Run `sh scripts/check.sh` (or `scripts/release-check.sh`, which runs it plus version checks and
   an install into a throwaway home folder). `scripts/check.sh` is exactly what CI runs, in the
   same order with the same flags: shell scripts (`sh -n`/`bash -n` and shellcheck at every
   severity), `cargo fmt --check`, clippy with warnings as errors, all tests (also in Tokyo time),
   `cargo audit`, and the release build's quick self-test. It needs the pinned tools: the Rust in
   `rust-toolchain.toml` (rustup installs it), `pip install shellcheck-py==0.11.0.1` and
   `cargo install cargo-audit --locked`. None of these are needed to install or use CachyPlay.
4. Push to `main` (or upload with StowLock). GitHub then shows two rows in the Actions tab: **CI** on `main` and **Release** on the new tag. CI:
   - stops quietly if that version is already released,
   - refuses a version that ranks below an existing release (for example `0.1.0-beta.12` after
     `0.1.0` is out; use `0.1.1-beta.1`),
   - runs all tests (in two time zones) and publishes nothing if one fails,
   - tags the commit `vX.Y.Z[-beta.N]` and starts **Release**, which builds and publishes it (as a
     pre-release when the version contains `-beta.`). A tag pushed by hand also starts Release,
     which then runs the tests itself first.
5. On a real PC: update to it with the in-app **Update** button and start a game through it.

Pushing changes without bumping the version only runs the tests; nothing is released.

## Version order

`0.1.0-beta.11` < `0.1.0` < `0.1.1-beta.1` < `0.1.1`. A beta always ranks below the finished
version with the same number, so a beta for after a stable release needs the next number. People
on the Stable channel never get betas; people on Beta get whichever is highest, beta or stable.
Someone on a beta newer than the newest stable who switches to Stable sees **Switch to Stable now**.

## Keeping CI green

- **Pinned tools:** Rust (with clippy and rustfmt) is pinned in `rust-toolchain.toml` and
  shellcheck to 0.11.0 in `scripts/check.sh` and CI, so a new tool release can't break CI
  overnight. Raise them on purpose, run `scripts/check.sh`, and fix what it finds in that release.
- **First-time cleanups get their own release:** the first reformat or lint cleanup after adding
  or raising a tool goes in a release that changes nothing else.
- **If a Release run fails,** check the Releases page for a leftover draft. The workflow deletes a
  leftover draft itself the next time it runs for the same version.

## Known `cargo audit` warnings

`cargo audit` (in `scripts/check.sh`) fails only on security problems. These warnings are known,
don't fail it, and are checked again whenever dependencies are updated:

| Library | Warning | Comes from | What we'd do |
|---|---|---|---|
| chrono | unmaintained (RUSTSEC-2026-0543) | CachyPlay itself (dates and times) | Move to its successor once one is settled; a behaviour change, so its own release |
| zbus_names, zvariant, zvariant_derive | unmaintained (RUSTSEC-2026-0370/0371/0372) | tauri-plugin-single-instance (via zbus 5) | Update the plugin when it moves off them |
| glib | unsound iterator (RUSTSEC-2024-0429) | GTK 0.18, used by Tauri | Fixed when Tauri moves to newer GTK bindings; CachyPlay doesn't use that iterator |
| proc-macro-error | unmaintained (RUSTSEC-2024-0370) | GTK 0.18 build macros, used by Tauri | Same as glib (build time only) |

## Staged releases

- Every version ships to the **Beta** channel first.
- After about a week with no new problem reports, release the same code as the next stable
  version (drop the `-beta.N`), which reaches the **Stable** channel.
- If a beta breaks something, fix forward with the next beta; users can also press
  **Go back to the previous version** in Settings.

## Signing releases (required from beta.16)

Every release is signed with minisign; the Release workflow refuses to publish without the key.

- **Two key pairs.** The **primary** secret key is the repository secret `MINISIGN_SECRET_KEY`
  (made with `minisign -G -W`, no password). The **backup** secret key is kept offline and is only
  used if the primary is lost or leaked. Both public keys are in `packaging/minisign.pub` (and in
  `install.sh`), and the app accepts a signature from either.
- **How a release is published:** build → create a **draft** release → upload the download,
  `SHA256SUMS`, `SHA256SUMS.minisig` and `install.sh` → publish. Updaters never see a release
  without its signature, and treat an unsigned one as "not available yet".
- **Old file names:** `SHA256SUMS.txt` and `SHA256SUMS.txt.minisig` are still uploaded (same
  content) for beta.14 and earlier, whose updater looks for that name. Drop them once no supported
  version needs them, and say so in the CHANGELOG.
- **Changing to the backup key:** put the backup secret into `MINISIGN_SECRET_KEY`, make a new
  backup pair offline, and replace the old primary public key in `packaging/minisign.pub` and
  `install.sh` with the new backup's in the next release.
- If signing fails with a password prompt, the secret key has a password: make it again with `-W`.

Build provenance attestations are added automatically once the repository is public.
