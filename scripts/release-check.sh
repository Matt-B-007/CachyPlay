#!/bin/sh
# Run before tagging a release: scripts/release-check.sh v0.1.0-beta.9
set -eu
tag="${1:?usage: scripts/release-check.sh vX.Y.Z[-beta.N]}"
cd "$(dirname "$0")/.."
ver=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
step() { printf '\n\033[1;36m==>\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31mRelease check failed:\033[0m %s\n' "$*" >&2; exit 1; }

step "Version matches the tag"
[ "v$ver" = "$tag" ] || fail "Cargo.toml says $ver but the tag is $tag"
grep -q "\"$ver\"" Cargo.lock || fail "Cargo.lock is out of date: run cargo update --workspace"

step "CHANGELOG mentions $ver"
grep -q "${ver#v}" CHANGELOG.md || fail "add $ver to CHANGELOG.md"

step "Formatting, lints, tests"
cargo fmt --all --check || fail "run cargo fmt --all"
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

step "Release build"
cargo build --release --locked -p cachyplay -p cachyplay-gui
got=$(./target/release/cachyplay --version)
[ "$got" = "cachyplay $ver" ] || fail "the build reports '$got'"

step "Installs and runs from a throwaway home folder"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
HOME="$tmp" ./target/release/cachyplay install >/dev/null
HOME="$tmp" "$tmp/.local/bin/cachyplay" --version | grep -q "$ver" || fail "installed copy doesn't run"
[ -x "$tmp/.local/bin/cachyplay-gui" ] || fail "the app wasn't installed next to the command"

step "Installer script is valid"
sh -n install.sh

printf '\n\033[1;32mAll good.\033[0m Push the code, then: git tag %s && git push origin %s\n' "$tag" "$tag"
