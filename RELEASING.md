# Releasing CachyPlay

Run the checklist every time. Most release mistakes so far (a version that didn't match its tag,
a missing release, a test that only passed on one PC) are caught by it.

## Checklist

1. Bump `version` in the root `Cargo.toml` (`0.1.0-beta.N`, or `0.1.0` for stable), then
   `cargo update --workspace`.
2. Add the changes to `CHANGELOG.md` under that version.
3. Run `scripts/release-check.sh v0.1.0-beta.N`. It checks the version matches the tag, the
   changelog mentions it, formatting, lints and all tests, makes a release build, installs it into
   a throwaway home folder and runs it.
4. Push the code, then push the tag `v0.1.0-beta.N`. The Release workflow builds the download and
   publishes the release (as a pre-release when the version contains `-beta.`).
5. On a real PC: update to it with the in-app **Update** button and start a game through it.

## Staged releases

- Every version ships to the **Beta** channel first.
- After about a week with no new problem reports, release the same code as the next stable
  version (drop the `-beta.N`), which reaches the **Stable** channel.
- If a beta breaks something, fix forward with the next beta; users can also press
  **Go back to the previous version** in Settings.

## Signing releases (recommended)

Checksums prove a download wasn't damaged; a signature proves it came from you. Once set up,
CachyPlay refuses any update that isn't signed with your key.

1. On your PC: `sudo pacman -S minisign`, then `minisign -G -W -p minisign.pub -s minisign.key`
   (`-W` makes a key without a password, so GitHub can use it).
2. Copy `minisign.pub` over `packaging/minisign.pub` in the repository and commit it.
3. In GitHub: Settings → Secrets and variables → Actions → New repository secret named
   `MINISIGN_KEY`, with the full contents of `minisign.key`. Keep the key file somewhere safe and
   never commit it.
4. From the next release on, the workflow attaches `SHA256SUMS.txt.minisig`, and builds that
   contain your public key only install signed updates.

Releases made before step 2 stay installable: builds without a key skip the signature check.
