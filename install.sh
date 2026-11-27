#!/bin/sh
# CachyPlay installer.
#
#   From the downloaded folder (zip from GitHub, or a release download):
#       sh install.sh
#   Without downloading anything (needs the GitHub CLI signed in while the repository is private):
#       gh api repos/Matt-B-007/CachyPlay/contents/install.sh -H "Accept: application/vnd.github.raw" | sh
#
# Options:  --beta         newest beta instead of the newest stable release
#           --version X    install version X (like 0.1.0-beta.16) instead of the newest
#           --from-source  build on this PC instead of downloading a ready-made build
#           --no-launch    don't open CachyPlay at the end
#           --uninstall    put every game back the way it was and remove CachyPlay
#
# What it does: finds a ready-made build (in this folder, or on GitHub Releases), checks its
# SHA-256 checksum (and its signature, if minisign is installed), and runs `cachyplay install`, which copies CachyPlay to ~/.local/bin and adds it
# to your app menu. If no ready-made build exists for this version, it builds one from the source
# code in this folder (installing the build tools with pacman, after asking). Nothing goes outside
# your home folder except those pacman packages. Safe to run again: it just reinstalls.

set -eu

REPO="Matt-B-007/CachyPlay"
CHANNEL=stable
FROM_SOURCE=0
LAUNCH=1
VERSION=""
UNINSTALL=0
# The public keys CachyPlay releases are signed with (same as packaging/minisign.pub).
KEYS="RWQcs3jtP2jTiAMafwQX8UwciSjOH4o9tQcCVvYl2NkdnOeVupKPlhvC RWRObC9qlP598z1kmTFw7W+TKYKk3w2/63qWyRWkvRSkqGwGEgjmOlGe"

# Messages go to stderr, so functions can hand back a path on stdout.
say() { printf '\033[1;36m==>\033[0m %s\n' "$*" >&2; }
warn() { printf '\033[1;33m==>\033[0m %s\n' "$*" >&2; }
die() {
    printf '\033[1;31mError:\033[0m %s\n' "$*" >&2
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

while [ $# -gt 0 ]; do
    case "$1" in
    --beta) CHANNEL=beta ;;
    --from-source) FROM_SOURCE=1 ;;
    --no-launch) LAUNCH=0 ;;
    --uninstall) UNINSTALL=1 ;;
    --version)
        [ $# -ge 2 ] || die "--version needs a version number, like --version 0.1.0-beta.16"
        VERSION="v${2#v}"
        shift
        ;;
    --version=*) VERSION="v$(echo "${1#--version=}" | sed 's/^v//')" ;;
    -h | --help)
        sed -n '2,20p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    *) die "Unknown option: $1 (try --help)" ;;
    esac
    shift
done

[ "$(id -u)" -ne 0 ] || die "Run this as your normal user, without sudo. CachyPlay installs into your home folder."

if [ "$UNINSTALL" = 1 ]; then
    bin="$HOME/.local/bin/cachyplay"
    [ -x "$bin" ] || die "CachyPlay isn't installed in ~/.local/bin. (Installed from the AUR? Use: sudo pacman -Rns cachyplay)"
    # The uninstaller asks before changing anything; read the answer from the keyboard even
    # when this script was piped into sh.
    if [ -r /dev/tty ]; then
        exec "$bin" uninstall </dev/tty
    fi
    exec "$bin" uninstall
fi
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

# Every published release that has a Linux download attached, one per line: "<tag> <true|false>"
# (the second word says whether it's a beta). Releases without a download are skipped.
releases_with_builds() {
    if gh_ok; then
        gh api "repos/$REPO/releases?per_page=50" --jq \
            '.[] | select(.draft | not) | select([.assets[].name] | any(endswith("-x86_64-linux.tar.zst"))) | "\(.tag_name) \(.prerelease)"' \
            2>/dev/null || true
    elif have curl; then
        # Without jq: read tag_name, draft, prerelease and asset names release by release.
        curl -fsSL -A cachyplay-installer "https://api.github.com/repos/$REPO/releases?per_page=50" 2>/dev/null |
            awk '
                function flush() { if (tag != "" && build && draft != "true") print tag, pre; tag = ""; build = 0; draft = ""; pre = "false" }
                /"tag_name":/   { flush(); match($0, /"tag_name": *"[^"]*"/); t = substr($0, RSTART, RLENGTH); sub(/.*"tag_name": *"/, "", t); sub(/"$/, "", t); tag = t }
                /"draft":/      { draft = ($0 ~ /true/) ? "true" : "false" }
                /"prerelease":/ { pre = ($0 ~ /true/) ? "true" : "false" }
                /"name": *"[^"]*-x86_64-linux\.tar\.zst"/ { build = 1 }
                END { flush() }' || true
    fi
}

# Print the newest release tag on a channel ("stable" or "beta") that has a download; empty if
# there is none. Compares version numbers instead of trusting GitHub's order, and ranks a finished
# version above its betas (0.1.0 > 0.1.0-beta.10 > 0.1.0-beta.9).
newest_tag() {
    releases_with_builds |
        while read -r tag pre; do
            [ "$1" = beta ] || [ "$pre" = false ] || continue
            case "$tag" in
            *-*) echo "$tag $tag" ;;
            *) echo "$tag-zzz $tag" ;; # "zzz" sorts after "beta" and "rc", so finished versions win
            esac
        done |
        sort -V -k1,1 | tail -n 1 | cut -d' ' -f2
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

# Check the checksum file's signature with minisign, if it's installed. Releases from beta.16 on are
# signed; the app itself always checks signatures before it updates.
check_signature() {
    sums="$1"
    if ! have minisign; then
        say "Checked the download's checksum. (Install minisign to check its signature too.)"
        return 0
    fi
    if [ ! -f "$sums.minisig" ]; then
        warn "This release isn't signed (releases before beta.16 weren't). Checked its checksum only."
        return 0
    fi
    for key in $KEYS; do
        if minisign -V -q -P "$key" -m "$sums" 2>/dev/null; then
            say "Checked the download's checksum and signature"
            return 0
        fi
    done
    die "The download's signature doesn't match CachyPlay's signing keys. It may have been tampered with; nothing was installed."
}

# Download one release file into $TMP (GitHub CLI first, then curl). Fails quietly.
fetch() {
    tag="$1"
    file="$2"
    if gh_ok && gh release download "$tag" -R "$REPO" -p "$file" -D "$TMP" --clobber >/dev/null 2>&1; then
        return 0
    fi
    have curl && curl -fsSL -o "$TMP/$file" "https://github.com/$REPO/releases/download/$tag/$file" 2>/dev/null
}

# Download and unpack a release build into $TMP. Prints the unpacked folder; fails quietly.
download_release() {
    tag="$1"
    name="cachyplay-$tag-x86_64-linux"
    fetch "$tag" "$name.tar.zst" || return 3
    # SHA256SUMS from beta.16 on; SHA256SUMS.txt before that.
    sums=""
    for f in SHA256SUMS SHA256SUMS.txt; do
        if fetch "$tag" "$f"; then
            sums="$f"
            fetch "$tag" "$f.minisig" || true
            break
        fi
    done
    [ -n "$sums" ] || return 3
    check_signature "$TMP/$sums"
    (cd "$TMP" && grep " $name.tar.zst\$" "$sums" | sha256sum -c --status) ||
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
if [ "$FROM_SOURCE" = 0 ] && [ -z "$VERSION" ] && [ -n "$HERE" ] && [ -x "$HERE/cachyplay" ] && [ -x "$HERE/cachyplay-gui" ]; then
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

# --version wins over the source folder's version.
[ -z "$VERSION" ] || WANT="$VERSION"

if [ "$FROM_SOURCE" = 0 ]; then
    TAG=$(pick_tag "$WANT")
    if [ -n "$TAG" ]; then
        say "Downloading CachyPlay $TAG from GitHub"
        rc=0
        DIR=$(download_release "$TAG") || rc=$?
        [ "$rc" = 0 ] && finish "$DIR"
        [ "$rc" = 3 ] || exit 1 # checksum or unpack problem: already explained
        [ -z "$VERSION" ] || die "There's no ready-made build of $VERSION on GitHub. Check the version number on the Releases page."
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
