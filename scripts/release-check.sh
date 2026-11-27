#!/bin/sh
# Optional check before pushing a new version: scripts/release-check.sh [v0.1.0-beta.11]
# GitHub runs the tests and publishes by itself when the version in Cargo.toml is new; this
# catches problems on your PC first.
set -eu
cd "$(dirname "$0")/.."
ver=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
tag="${1:-v$ver}"
step() { printf '\n\033[1;36m==>\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31mRelease check failed:\033[0m %s\n' "$*" >&2; exit 1; }

step "Version matches the tag"
[ "v$ver" = "$tag" ] || fail "Cargo.toml says $ver but the tag is $tag"
grep -q "\"$ver\"" Cargo.lock || fail "Cargo.lock is out of date: run cargo update --workspace"

step "Version ranks above every existing release"
if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    gh release list -R Matt-B-007/CachyPlay --limit 200 --json tagName --jq '.[].tagName' |
        sh scripts/version-guard.sh "$ver" || fail "pick a higher version number"
else
    git tag 2>/dev/null | sh scripts/version-guard.sh "$ver" || fail "pick a higher version number"
fi

step "CHANGELOG mentions $ver"
grep -q "${ver#v}" CHANGELOG.md || fail "add $ver to CHANGELOG.md"

step "Every CI check (scripts/check.sh)"
sh scripts/check.sh || fail "scripts/check.sh failed (see above)"

step "Release build"
cargo build --release --locked -p cachyplay -p cachyplay-gui
got=$(./target/release/cachyplay --version)
[ "$got" = "cachyplay $ver" ] || fail "the build reports '$got'"
./target/release/cachyplay --self-test --quick >/dev/null || fail "the release build fails its quick self-test"
./target/release/cachyplay-gui --self-test --quick >/dev/null || fail "the desktop app fails its quick self-test"

step "Installs and runs from a throwaway home folder"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
HOME="$tmp" ./target/release/cachyplay install >/dev/null
HOME="$tmp" "$tmp/.local/bin/cachyplay" --version | grep -q "$ver" || fail "installed copy doesn't run"
[ -x "$tmp/.local/bin/cachyplay-gui" ] || fail "the app wasn't installed next to the command"

step "Installer script is valid"
sh -n install.sh
sh -n scripts/version-guard.sh

printf '\n\033[1;32mAll good.\033[0m Push to main (or upload with StowLock). GitHub tests, tags and publishes %s by itself.\n' "$tag"
