#!/bin/sh
# Every check CI runs, in the same order with the same flags. GitHub's CI calls this script and
# nothing else for its checks, and StowLock runs it before pushing, so a pass here means a pass
# there.
#
#   sh scripts/check.sh          all checks
#   sh scripts/check.sh tests    only the tests (the Arch Linux CI job)
#
# Exit codes: 0 = every check passed, 1 = a check failed, 3 = only tools are missing (all of them
# are listed first, with the command that installs them). You don't need any of this to install
# or use CachyPlay; it's for making releases.
set -eu
cd "$(dirname "$0")/.." || exit 1

# CI installs exactly this shellcheck. Another version on a PC gives a note, not a failure.
SHELLCHECK_VERSION=0.11.0
PART="${1:-all}"

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    BLUE=$(printf '\033[1;36m')
    RED=$(printf '\033[1;31m')
    GREEN=$(printf '\033[1;32m')
    YELLOW=$(printf '\033[1;33m')
    OFF=$(printf '\033[0m')
else
    BLUE=""
    RED=""
    GREEN=""
    YELLOW=""
    OFF=""
fi

step() { printf '\n%s==>%s %s\n' "$BLUE" "$OFF" "$*"; }
note() { printf '%sNote:%s %s\n' "$YELLOW" "$OFF" "$*"; }
fail() {
    printf '%sCheck failed:%s %s\n' "$RED" "$OFF" "$*" >&2
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

# ---------- 1. Every tool first, so all missing ones are listed at once ----------
MISSING_PACMAN=""
MISSING_OTHER=""
missing() { # missing "<what>" "<Arch package>" "<other way to install>"
    printf '  - %s\n' "$1" >&2
    case " $MISSING_PACMAN " in
    *" $2 "*) ;;
    *) MISSING_PACMAN="$MISSING_PACMAN $2" ;;
    esac
    [ -z "$3" ] || MISSING_OTHER="$MISSING_OTHER
    $3"
}

check_tools() {
    found_missing=0
    say_missing() {
        if [ "$found_missing" = 0 ]; then
            printf '%sMissing tools:%s\n' "$RED" "$OFF" >&2
            found_missing=1
        fi
        missing "$@"
    }
    have cargo || say_missing "cargo (Rust)" rustup "Other systems: https://rustup.rs"
    if have cargo && [ "$PART" = all ]; then
        cargo clippy --version >/dev/null 2>&1 || say_missing "clippy" rustup "rustup component add clippy"
        cargo fmt --version >/dev/null 2>&1 || say_missing "rustfmt" rustup "rustup component add rustfmt"
    fi
    have cc || say_missing "a C compiler" base-devel ""
    have pkg-config || say_missing "pkg-config" pkgconf ""
    if have pkg-config; then
        pkg-config --exists webkit2gtk-4.1 2>/dev/null || say_missing "WebKitGTK 4.1 (for the desktop app)" webkit2gtk-4.1 "Ubuntu/Debian: libwebkit2gtk-4.1-dev"
        pkg-config --exists librsvg-2.0 2>/dev/null || say_missing "librsvg" librsvg "Ubuntu/Debian: librsvg2-dev"
        pkg-config --exists openssl 2>/dev/null || say_missing "OpenSSL" openssl "Ubuntu/Debian: libssl-dev"
    fi
    have zstd || say_missing "zstd (update tests)" zstd ""
    if [ "$PART" = all ]; then
        have shellcheck || say_missing "shellcheck" shellcheck "Other systems: pip install shellcheck-py==$SHELLCHECK_VERSION.1"
        have cargo-audit || say_missing "cargo-audit" cargo-audit "Other systems: cargo install cargo-audit --locked"
    fi
    if [ "$found_missing" = 1 ]; then
        printf '\nInstall them on CachyOS / Arch with:\n    sudo pacman -S --needed%s\n' "$MISSING_PACMAN" >&2
        [ -z "$MISSING_OTHER" ] || printf '%s\n' "$MISSING_OTHER" | sed '/^$/d' >&2
        printf '(rustup replaces Arch'"'"'s "rust" package; it reads rust-toolchain.toml.)\n' >&2
        exit 3
    fi
}

# ---------- 2. The checks ----------

# install.sh, uninstall.sh, get.sh and everything in scripts/ and tools/ that exists.
shell_scripts() {
    for f in install.sh uninstall.sh get.sh scripts/* tools/*; do
        if [ -f "$f" ]; then
            printf '%s\n' "$f"
        fi
    done
}

check_shell_scripts() {
    step "Check shell scripts (sh -n / bash -n + shellcheck, every severity)"
    got=$(shellcheck --version | sed -n 's/^version: //p')
    if [ "$got" != "$SHELLCHECK_VERSION" ]; then
        # On GitHub (CI=true) the pinned version is required; on a PC it's only a note.
        [ -z "${CI:-}" ] || fail "CI must use shellcheck $SHELLCHECK_VERSION, found $got"
        note "shellcheck $got here; CI uses $SHELLCHECK_VERSION. Results usually match."
    fi
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
    cargo fmt --all --check || fail "the code isn't formatted: run cargo fmt --all"
}

check_lints() {
    step "Check Rust lints (cargo clippy, warnings are errors)"
    cargo clippy --workspace --all-targets --locked -- -D warnings || fail "clippy found problems (above)"
}

run_tests() {
    step "Run all tests (cargo test)"
    cargo test --workspace --locked || fail "a test failed (above)"
    step "Run all tests in another time zone (TZ=Asia/Tokyo; times must not depend on where the PC is)"
    TZ=Asia/Tokyo cargo test --workspace --locked || fail "a test failed in another time zone (above)"
}

check_audit() {
    step "Check dependencies for known security problems (cargo audit)"
    cargo audit || fail "cargo audit found a security problem in a dependency (above)"
}

check_release_build() {
    step "Release build passes its own quick self-test (what the updater runs before installing)"
    cargo build --release --locked -p cachyplay -p cachyplay-gui || fail "the release build failed"
    ./target/release/cachyplay --self-test --quick || fail "the release build fails its quick self-test"
    ./target/release/cachyplay-gui --self-test --quick || fail "the desktop app fails its quick self-test"
}

case "$PART" in
all | tests) ;;
*)
    printf 'Unknown part %s (use: all, tests)\n' "$PART" >&2
    exit 1
    ;;
esac

check_tools
case "$PART" in
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
esac

printf '\n%sAll checks passed.%s\n' "$GREEN" "$OFF"
