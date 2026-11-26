# Releasing CachyPlay

Run the checklist every time. Most release mistakes so far (a version that didn't match its tag,
a missing release, a test that only passed on one PC) are caught by it.

## Checklist

Releases are automatic. GitHub publishes a release whenever the version in `Cargo.toml` is one
that hasn't been released yet. There are no tags to make by hand.

1. Bump `version` in the root `Cargo.toml` (`0.1.0-beta.N`, or `0.1.0` for stable), then
   `cargo update --workspace`.
2. Add the changes to `CHANGELOG.md` under that version.
3. Optional but recommended: `scripts/release-check.sh`. It runs everything GitHub will run, plus
   an install into a throwaway home folder.
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
