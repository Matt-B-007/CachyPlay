# Security

## Reporting a security problem

Please don't open a public issue for a security problem. Email the maintainer through the address
on the GitHub profile of [Matt-B-007](https://github.com/Matt-B-007), or use GitHub's **Report a
vulnerability** button on the repository's Security tab once the repository is public. Say what you
found, how to reproduce it, and which version you used (`cachyplay --version`). You'll get an answer
within a week.

## Supported versions

The newest Stable release and the newest Beta get security fixes. Update from Settings › Updates,
or with `cachyplay --update`.

## How CachyPlay protects you

- **Signed updates.** Every release's checksum file is signed with minisign. CachyPlay has two
  public keys built in (one used by GitHub Actions, one kept offline as a backup) and refuses any
  update whose signature or SHA-256 checksum doesn't match. A release without a signature isn't
  offered. Before swapping in a new version, it runs that version's `--self-test --quick`, and it
  keeps the previous version so you can go back.
- **Never edits the system.** CachyPlay installs into your home folder and only changes Steam's
  per-game launch options and per-game files, each with Undo. It never asks for your password
  (the installer asks pacman for build tools only when you build from source, and says why).
- **Fails open.** If CachyPlay is missing or broken, games start exactly as they would without it.
- **Private bug reports.** Reports stay on your PC until you share them, and personal details are
  replaced before anything is saved (see the Privacy section of the README).
- **The window** loads only files built into the app (a strict content security policy), and cover
  art comes through a `cover://` link that can only return game covers.
