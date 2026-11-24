#!/bin/sh
# CachyPlay installer.
#
#   From the downloaded folder (zip from GitHub, or a release download):
#       sh install.sh
#   Without downloading anything (needs the GitHub CLI signed in while the repository is private):
#       gh api repos/Matt-B-007/CachyPlay/contents/install.sh -H "Accept: application/vnd.github.raw" | sh
#
# Options:  --beta         newest beta instead of the newest stable release
#           --from-source  build on this PC instead of downloading a ready-made build
#           --no-launch    don't open CachyPlay at the end
#
# What it does: finds a ready-made build (in this folder, or on GitHub Releases), checks its
# SHA-256 checksum, and runs `cachyplay install`, which copies CachyPlay to ~/.local/bin and adds it
# to your app menu. If no ready-made build exists for this version, it builds one from the source
# code in this folder (installing the build tools with pacman, after asking). Nothing goes outside
# your home folder except those pacman packages. Safe to run again: it just reinstalls.

set -eu

REPO="Matt-B-007/CachyPlay"
CHANNEL=stable
FROM_SOURCE=0
LAUNCH=1

# Messages go to stderr, so functions can hand back a path on stdout.
say() { printf '\033[1;36m==>\033[0m %s\n' "$*" >&2; }
warn() { printf '\033[1;33m==>\033[0m %s\n' "$*" >&2; }
die() {
    printf '\033[1;31mError:\033[0m %s\n' "$*" >&2
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

for arg in "$@"; do
    case "$arg" in
    --beta) CHANNEL=beta ;;
    --from-source) FROM_SOURCE=1 ;;
    --no-launch) LAUNCH=0 ;;
    -h | --help)
        sed -n '2,18p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    *) die "Unknown option: $arg (try --help)" ;;
    esac
done

[ "$(id -u)" -ne 0 ] || die "Run this as your normal user, without sudo. CachyPlay installs into your home folder."
[ "$(uname -m)" = x86_64 ] || die "CachyPlay builds are for x86_64 PCs."

# The folder this script is in, if it was run from a file (empty when piped into sh).
HERE=""
case "$0" in
*install.sh) HERE=$(cd "$(dirname "$0")" && pwd) ;;
esac

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

ask() { # ask "Question?" -> 0 for yes. Reads the keyboard even when this script is piped.
    printf '%s [Y/n] ' "$1" >&2
    if [ -r /dev/tty ]; then
        read -r reply </dev/tty || reply=n
    else
        reply=n
    fi
    case "$reply" in "" | y | Y | yes | Yes) return 0 ;; *) return 1 ;; esac
}

gh_ok() { have gh && gh auth status >/dev/null 2>&1; }

# Print the newest release tag on a channel ("stable" or "beta"); empty if there is none.
newest_tag() {
    if gh_ok; then
        if [ "$1" = beta ]; then
            gh api "repos/$REPO/releases?per_page=30" --jq '[.[] | select(.draft | not)][0].tag_name // empty' 2>/dev/null || true
        else
            gh api "repos/$REPO/releases?per_page=30" --jq '[.[] | select((.draft | not) and (.prerelease | not))][0].tag_name // empty' 2>/dev/null || true
        fi
    elif have curl; then
        if [ "$1" = beta ]; then
            url="https://api.github.com/repos/$REPO/releases?per_page=1"
        else
            url="https://api.github.com/repos/$REPO/releases/latest"
        fi
        curl -fsSL -A cachyplay-installer "$url" 2>/dev/null |
            grep -o '"tag_name": *"[^"]*"' | head -n 1 | sed 's/.*"\([^"]*\)"$/\1/' || true
    fi
}

# Print the tag of the release to install (empty if none can be found).
pick_tag() {
    if [ -n "$1" ]; then
        echo "$1"
        return
    fi
    tag=$(newest_tag "$CHANNEL")
    if [ -z "$tag" ] && [ "$CHANNEL" = stable ]; then
        # No stable release yet (CachyPlay is in beta): use the newest beta.
        tag=$(newest_tag beta)
        [ -z "$tag" ] || warn "There's no stable release yet, so installing the newest beta."
    fi
    echo "$tag"
}

# Download and unpack a release build into $TMP. Prints the unpacked folder; fails quietly.
download_release() {
    tag="$1"
    name="cachyplay-$tag-x86_64-linux"
    ok=0
    if gh_ok && gh release download "$tag" -R "$REPO" -p "$name.tar.zst" -p SHA256SUMS.txt -D "$TMP" --clobber >/dev/null 2>&1; then
        ok=1
    elif have curl &&
        curl -fsSL -o "$TMP/$name.tar.zst" "https://github.com/$REPO/releases/download/$tag/$name.tar.zst" 2>/dev/null &&
        curl -fsSL -o "$TMP/SHA256SUMS.txt" "https://github.com/$REPO/releases/download/$tag/SHA256SUMS.txt" 2>/dev/null; then
        ok=1
    fi
    [ "$ok" = 1 ] || return 3
    (cd "$TMP" && grep " $name.tar.zst\$" SHA256SUMS.txt | sha256sum -c --status) ||
        die "The download of $tag didn't match its checksum. Nothing was installed. Try again later."
    tar --zstd -xf "$TMP/$name.tar.zst" -C "$TMP" || die "Couldn't unpack $name.tar.zst (is zstd installed?)"
    [ -x "$TMP/$name/cachyplay" ] || return 3
    echo "$TMP/$name"
}

build_from_source() {
    src="$1"
    say "Building CachyPlay from the source code in $src (takes a few minutes the first time)"
    missing=""
    have cargo || missing="$missing rust"
    have cc || missing="$missing base-devel"
    have pkg-config || missing="$missing pkgconf"
    if have pkg-config; then
        pkg-config --exists webkit2gtk-4.1 2>/dev/null || missing="$missing webkit2gtk-4.1"
        pkg-config --exists librsvg-2.0 2>/dev/null || missing="$missing librsvg"
        pkg-config --exists openssl 2>/dev/null || missing="$missing openssl"
    else
        missing="$missing webkit2gtk-4.1 librsvg openssl"
    fi
    have xdotool || missing="$missing xdotool"
    if [ -n "$missing" ]; then
        say "These build tools are needed:$missing"
        have pacman || die "Install them with your package manager, then run this again."
        if ask "Install them now with pacman? (asks for your password)"; then
            # shellcheck disable=SC2086
            sudo pacman -S --needed $missing >&2 || die "pacman couldn't install the build tools."
        else
            die "Can't build without them. Install them with: sudo pacman -S --needed$missing"
        fi
    fi
    if ! cargo --version >/dev/null 2>&1; then
        # rustup is installed but has no toolchain yet.
        { have rustup && rustup default stable >&2; } || die "Rust isn't set up. Run: rustup default stable"
    fi
    (cd "$src" && cargo build --release --locked -p cachyplay -p cachyplay-gui >&2) ||
        die "The build failed. Save the output above and report it on GitHub."
    echo "$src/target/release"
}

finish() {
    from="$1"
    say "Installing"
    # Keep the "installed to" line; drop the tips meant for running `cachyplay install` by hand.
    "$from/cachyplay" install | grep -v -e "delete the downloaded file" -e "^Tip:" >&2
    gui="$HOME/.local/bin/cachyplay-gui"
    echo
    say "CachyPlay $("$HOME/.local/bin/cachyplay" --version | sed 's/^cachyplay //') is installed. Open it from your app menu any time."
    if [ -n "$HERE" ]; then
        echo "    You can delete the downloaded folder now: $HERE"
    fi
    if [ "$LAUNCH" = 1 ] && [ -x "$gui" ] && { [ -n "${WAYLAND_DISPLAY:-}" ] || [ -n "${DISPLAY:-}" ]; }; then
        if have setsid; then
            setsid "$gui" >/dev/null 2>&1 </dev/null &
        else
            nohup "$gui" >/dev/null 2>&1 </dev/null &
        fi
        say "Opening CachyPlay"
    fi
    exit 0
}

# 1. A release download: the ready-made programs are right here.
if [ "$FROM_SOURCE" = 0 ] && [ -n "$HERE" ] && [ -x "$HERE/cachyplay" ] && [ -x "$HERE/cachyplay-gui" ]; then
    say "Found a ready-made build in this folder"
    finish "$HERE"
fi

# 2. The source code (zip from GitHub): get the ready-made build of the same version if there is one.
SOURCE=""
WANT=""
if [ -n "$HERE" ] && [ -f "$HERE/Cargo.toml" ] && [ -d "$HERE/crates/cachyplay" ]; then
    SOURCE="$HERE"
    WANT="v$(grep -m1 '^version' "$HERE/Cargo.toml" | cut -d'"' -f2)"
fi

if [ "$FROM_SOURCE" = 0 ]; then
    TAG=$(pick_tag "$WANT")
    if [ -n "$TAG" ]; then
        say "Downloading CachyPlay $TAG from GitHub"
        rc=0
        DIR=$(download_release "$TAG") || rc=$?
        [ "$rc" = 0 ] && finish "$DIR"
        [ "$rc" = 3 ] || exit 1 # checksum or unpack problem: already explained
        warn "No ready-made build of $TAG is on GitHub yet."
    elif [ -z "$SOURCE" ]; then
        if have gh && ! gh_ok; then
            die "Couldn't reach CachyPlay on GitHub. While the repository is private, sign in first: gh auth login"
        fi
        die "No CachyPlay release has been published on GitHub yet. A release is built when a version tag (like v0.1.0-beta.7) is pushed; check the Actions tab on GitHub. Until then, download the zip from GitHub, unzip it, and run: sh install.sh"
    fi
fi

[ -n "$SOURCE" ] || die "No ready-made build found and no source code here to build from."
BUILT=$(build_from_source "$SOURCE") || exit 1
finish "$BUILT"
