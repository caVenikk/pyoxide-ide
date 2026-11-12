#!/bin/sh
# Installs the IDE from a GitHub Release (macOS and Linux). docs/release.md.
#
#   curl -fsSL https://github.com/caVenikk/pyoxide-ide/releases/latest/download/install.sh | sh
#   sh install.sh [--version X.Y.Z] [--prefix DIR] [--bin-dir DIR] [--uninstall]
#
# What it does:
#   1. finds the release (the latest one, or --version),
#   2. downloads the archive for this OS and CPU and the release's SHA256SUMS,
#   3. checks the archive's SHA-256 against SHA256SUMS and stops on a mismatch,
#   4. installs it: macOS  <prefix>/<Product>.app   (default prefix ~/Applications)
#                   Linux  <prefix>/<slug>/          (default prefix ~/.local/share)
#      and puts launchers for the command-line programs in --bin-dir
#      (default ~/.local/bin),
#   5. macOS only: removes the quarantine attribute from the installed app if
#      the download carried one (see below).
# It writes nowhere else and never asks for administrator rights.
#
# Why the quarantine step: macOS marks files that a browser or another
# "quarantining" program downloaded, and Gatekeeper then refuses to open an
# app that is not notarized by Apple. These builds are not notarized (that
# needs a paid Apple Developer account), they are signed with a fixed
# identifier only. Files downloaded by curl are not marked, so normally there
# is nothing to remove; if the mark is there (the archive came through a
# browser, or a file:// copy of one), the script clears it from the installed
# app only, after the checksum matched. You can read every command below.
#
# For testing, the environment overrides the source:
#   PYOXIDE_VERSION      the version to install (same as --version)
#   PYOXIDE_BASE_URL     where the archive and SHA256SUMS are, instead of the
#                        release's download folder; file:// works
#   PYOXIDE_OS, PYOXIDE_ARCH   macos|linux, aarch64|x86_64 instead of detection
set -eu

# The brand module is the source of these (scripts/package.py check compares).
PRODUCT="Pyoxide"
SLUG="pyoxide"
REPOSITORY="caVenikk/pyoxide-ide"
CLI="pyoxide"
DB_CLI="pyoxide-db"
IDE="pyoxide-ide"

say() { printf '%s\n' "$*"; }
die() {
    printf 'install.sh: %s\n' "$*" >&2
    exit 1
}

version="${PYOXIDE_VERSION:-}"
prefix=""
bin_dir="${HOME:?HOME is not set}/.local/bin"
uninstall=0

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            [ $# -ge 2 ] || die "--version needs a value"
            version="$2"
            shift 2
            ;;
        --prefix)
            [ $# -ge 2 ] || die "--prefix needs a value"
            prefix="$2"
            shift 2
            ;;
        --bin-dir)
            [ $# -ge 2 ] || die "--bin-dir needs a value"
            bin_dir="$2"
            shift 2
            ;;
        --uninstall)
            uninstall=1
            shift
            ;;
        -h | --help)
            sed -n '2,/^set -eu/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *) die "unknown argument '$1' (see --help)" ;;
    esac
done
version="${version#v}"

os="${PYOXIDE_OS:-}"
if [ -z "$os" ]; then
    case "$(uname -s)" in
        Darwin) os=macos ;;
        Linux) os=linux ;;
        MINGW* | MSYS* | CYGWIN*) die "on Windows download the .zip from https://github.com/$REPOSITORY/releases and unpack it" ;;
        *) die "no release for $(uname -s)" ;;
    esac
fi
arch="${PYOXIDE_ARCH:-}"
if [ -z "$arch" ]; then
    case "$(uname -m)" in
        arm64 | aarch64) arch=aarch64 ;;
        x86_64 | amd64) arch=x86_64 ;;
        *) die "no release for the CPU $(uname -m)" ;;
    esac
fi

if [ -z "$prefix" ]; then
    case "$os" in
        macos) prefix="$HOME/Applications" ;;
        *) prefix="$HOME/.local/share" ;;
    esac
fi

case "$os" in
    macos) app_dir="$prefix/$PRODUCT.app" ;;
    *) app_dir="$prefix/$SLUG" ;;
esac
case "$os" in
    macos) program_rel="/Contents/MacOS" ;;
    *) program_rel="" ;;
esac
program_dir="$app_dir$program_rel"

launcher() {
    # A small script instead of a symlink: the program finds its own files
    # (the debugger helper) from the path it was started by.
    name="$1"
    target="$program_dir/$name"
    printf '#!/bin/sh\nexec "%s" "$@"\n' "$target" >"$bin_dir/$name"
    chmod 755 "$bin_dir/$name"
}

if [ "$uninstall" -eq 1 ]; then
    rm -rf "$app_dir"
    for name in "$IDE" "$DB_CLI" "$CLI"; do
        # Only launchers this script wrote.
        if [ -f "$bin_dir/$name" ] && grep -qF "$program_dir" "$bin_dir/$name" 2>/dev/null; then
            rm -f "$bin_dir/$name"
        fi
    done
    say "Removed $app_dir and its launchers in $bin_dir."
    say "Settings and caches stay; they are in the folders named $SLUG under your configuration and cache folders."
    exit 0
fi

command -v curl >/dev/null 2>&1 || die "curl is needed"
command -v tar >/dev/null 2>&1 || die "tar is needed"
if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
    die "sha256sum or shasum is needed to check the download"
fi

if [ -z "$version" ] && [ -z "${PYOXIDE_BASE_URL:-}" ]; then
    # The newest non-prerelease: GitHub redirects /releases/latest to its tag.
    say "Looking for the latest release..."
    final=$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/$REPOSITORY/releases/latest") ||
        die "could not reach GitHub"
    version="${final##*/}"
    version="${version#v}"
    case "$version" in
        latest | "") die "no release found; pass --version for a prerelease" ;;
    esac
fi
[ -n "$version" ] || die "PYOXIDE_BASE_URL needs a version: pass --version"

archive="$SLUG-$version-$os-$arch.tar.gz"
base="${PYOXIDE_BASE_URL:-https://github.com/$REPOSITORY/releases/download/v$version}"

work=$(mktemp -d "${TMPDIR:-/tmp}/$SLUG-install.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM

say "Installing $PRODUCT $version for $os $arch."
say "Downloading $archive"
curl -fSL --progress-bar -o "$work/$archive" "$base/$archive" || die "download failed: $base/$archive"
curl -fsSL -o "$work/SHA256SUMS" "$base/SHA256SUMS" || die "download failed: $base/SHA256SUMS"

expected=$(awk -v name="$archive" '{ f = $2; sub(/^\*/, "", f); if (f == name) print $1 }' "$work/SHA256SUMS")
[ -n "$expected" ] || die "SHA256SUMS has no entry for $archive"
actual=$(sha256 "$work/$archive")
if [ "$actual" != "$expected" ]; then
    die "checksum mismatch for $archive: expected $expected, got $actual; nothing was installed"
fi
say "SHA-256 matches ($actual)."

mkdir "$work/unpacked"
tar -xzf "$work/$archive" -C "$work/unpacked"
case "$os" in
    macos) unpacked="$work/unpacked/$PRODUCT.app" ;;
    *) unpacked="$work/unpacked/$SLUG-$version-$os-$arch" ;;
esac
[ -d "$unpacked" ] || die "the archive does not hold $(basename "$unpacked")"
[ -x "$unpacked$program_rel/$IDE" ] || die "the archive has no $IDE"

mkdir -p "$prefix" "$bin_dir"
if [ -e "$app_dir" ]; then
    # Replace in two renames, so a failure leaves the old copy in place.
    old="$app_dir.old.$$"
    mv "$app_dir" "$old"
    if mv "$unpacked" "$app_dir"; then
        rm -rf "$old"
    else
        mv "$old" "$app_dir"
        die "could not install into $app_dir"
    fi
else
    mv "$unpacked" "$app_dir"
fi

if [ "$os" = macos ]; then
    if xattr -r "$app_dir" 2>/dev/null | grep -q 'com.apple.quarantine'; then
        say "The app carries macOS's quarantine mark. It is not notarized by Apple (that needs a paid"
        say "developer account), so Gatekeeper would refuse to open it. The checksum matched, so the"
        say "mark is removed from $app_dir:  xattr -dr com.apple.quarantine"
        xattr -dr com.apple.quarantine "$app_dir"
    else
        say "No quarantine mark on $app_dir (curl does not set one): nothing to remove."
    fi
fi

for name in "$IDE" "$DB_CLI" "$CLI"; do
    launcher "$name"
done

say ""
say "Installed: $app_dir"
say "Command-line programs: $bin_dir/$CLI, $bin_dir/$DB_CLI, $bin_dir/$IDE"
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) say "Add $bin_dir to your PATH to run them by name." ;;
esac
if [ "$os" = macos ]; then
    say "Start the IDE from $prefix (Finder, Launchpad, Spotlight) or with:  open \"$app_dir\""
fi
say "Uninstall:  sh install.sh --uninstall"
