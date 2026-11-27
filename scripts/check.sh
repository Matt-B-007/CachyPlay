#!/bin/sh
# Every check CI runs, in the same order with the same flags. GitHub's CI calls this script and
# nothing else for its checks, so a pass here means a pass there.
#
#   sh scripts/check.sh          all checks
#   sh scripts/check.sh tests    only the tests (the Arch Linux CI job)
#
# Needed: Rust (the version in rust-toolchain.toml, with clippy and rustfmt), shellcheck (the
# version below: pip install shellcheck-py==0.11.0.1) and cargo-audit (cargo install cargo-audit).
# A missing tool stops the check with the command that installs it; nothing is skipped quietly.
# You don't need to run this to use or install CachyPlay; it's for making releases.
set -eu
cd "$(dirname "$0")/.." || exit 1

SHELLCHECK_VERSION=0.11.0

step() { printf '\n\033[1;36m==>\033[0m %s\n' "$*"; }
fail() {
    printf '\033[1;31mCheck failed:\033[0m %s\n' "$*" >&2
    exit 1
}
need() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 isn't installed. $2"
}

# install.sh, uninstall.sh, get.sh and everything in scripts/ and tools/ that exists.
shell_scripts() {
    for f in install.sh uninstall.sh get.sh scripts/* tools/*; do
        [ -f "$f" ] && printf '%s\n' "$f"
    done
}

check_shell_scripts() {
    step "Check shell scripts (sh -n / bash -n + shellcheck $SHELLCHECK_VERSION, every severity)"
    need shellcheck "Install it with: pip install shellcheck-py==$SHELLCHECK_VERSION.1"
    got=$(shellcheck --version | sed -n 's/^version: //p')
    [ "$got" = "$SHELLCHECK_VERSION" ] ||
        fail "shellcheck $got is installed, but CI uses $SHELLCHECK_VERSION (results can differ). Install it with: pip install shellcheck-py==$SHELLCHECK_VERSION.1"
    for f in $(shell_scripts); do
        case "$(head -n 1 "$f")" in
        *bash*)
            bash -n "$f" || fail "$f has a syntax error"
            shellcheck -s bash --severity=style "$f" || fail "shellcheck found problems in $f"
            ;;
        *)
            sh -n "$f" || fail "$f has a syntax error"
            shellcheck -s sh --severity=style "$f" || fail "shellcheck found problems in $f"
            ;;
        esac
        echo "ok  $f"
    done
}

check_format() {
    step "Check Rust formatting (cargo fmt --check)"
    cargo fmt --all --check || fail "run: cargo fmt --all"
}

check_lints() {
    step "Check Rust lints (cargo clippy, warnings are errors)"
    cargo clippy --workspace --all-targets --locked -- -D warnings
}

run_tests() {
    step "Run all tests (cargo test)"
    cargo test --workspace --locked
    step "Run all tests in another time zone (TZ=Asia/Tokyo; times must not depend on where the PC is)"
    TZ=Asia/Tokyo cargo test --workspace --locked
}

check_audit() {
    step "Check dependencies for known security problems (cargo audit)"
    need cargo-audit "Install it with: cargo install cargo-audit --locked"
    cargo audit
}

check_release_build() {
    step "Release build passes its own quick self-test (what the updater runs before installing)"
    cargo build --release --locked -p cachyplay -p cachyplay-gui
    ./target/release/cachyplay --self-test --quick
    ./target/release/cachyplay-gui --self-test --quick
}

case "${1:-all}" in
tests)
    run_tests
    ;;
all)
    check_shell_scripts
    check_format
    check_lints
    run_tests
    check_audit
    check_release_build
    ;;
*) fail "unknown part '$1' (use: all, tests)" ;;
esac

printf '\n\033[1;32mAll checks passed.\033[0m\n'
