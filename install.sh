#!/bin/sh
# Install nshw from its own release into NSHW_HOME/bin (default ~/.nshw/bin).
# Neither modifies an existing nsh nor reads the caller's application files.
set -eu

repo=YougLin-dev/nshw
home="${NSHW_HOME:-$HOME/.nshw}"
version="${NSHW_VERSION:-0.1.0}"
set_default=0
if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
    printf '%s\n' 'Usage: install.sh [--set-default]' >&2
    exit 0
fi
if [ "${1:-}" = "--set-default" ]; then
    set_default=1
elif [ $# -ne 0 ]; then
    printf 'unknown option: %s\n' "$1" >&2
    exit 2
fi

os=$(uname -s)
arch=$(uname -m)
case "$os:$arch" in
    Darwin:arm64) host=macos-arm64 ;;
    Darwin:x86_64) host=macos-x86_64 ;;
    Linux:x86_64) host=linux-x86_64 ;;
    Linux:aarch64|Linux:arm64) host=linux-arm64 ;;
    *) printf 'unsupported platform: %s\n' "$os $arch" >&2; exit 1 ;;
esac
asset="nshw-$host.tar.gz"
url="https://github.com/$repo/releases/download/v$version/$asset"
work=$(mktemp -d "${TMPDIR:-/tmp}/nshw-install.XXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM
curl -fSL --retry 2 --connect-timeout 30 "$url" -o "$work/$asset"
curl -fSL --retry 2 --connect-timeout 30 "$url.sha256" -o "$work/$asset.sha256"
if command -v sha256sum >/dev/null 2>&1; then
    sum() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
    sum() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
    printf 'sha256sum or shasum is required\n' >&2
    exit 1
fi
expected=$(tr -d '[:space:]' < "$work/$asset.sha256")
actual=$(sum "$work/$asset")
[ "$actual" = "$expected" ] || {
    printf 'SHA-256 mismatch for %s\n' "$url" >&2
    exit 1
}
# Archives must have exactly the expected structure.
tar -tzf "$work/$asset" | while IFS= read -r entry; do
    case "$entry" in
        nshw/|nshw/bin/|nshw/bin/nshw|./nshw/|./nshw/bin/|./nshw/bin/nshw) ;;
        *) printf 'unexpected archive entry: %s\n' "$entry" >&2; exit 1 ;;
    esac
done
mkdir -p "$home/bin"
tar -xzf "$work/$asset" -C "$home"
chmod 755 "$home/bin/nshw"
ln -sfn nshw "$home/bin/n"
if [ "$set_default" -eq 1 ]; then
    if [ -e "$home/bin/nsh" ] && [ ! -L "$home/bin/nsh" ]; then
        printf '%s exists and is not a symlink; not replacing it\n' "$home/bin/nsh" >&2
        exit 1
    fi
    ln -sfn nshw "$home/bin/nsh"
fi
printf 'installed nshw at %s (and n)\n' "$home/bin/nshw"
printf 'add %s/bin to PATH ahead of the distribution bin if using --set-default\n' "$home"
