#!/bin/sh
# Install one nshw from its verified release into NSHW_HOME/bin.
set -eu
repo="${NSHW_REPOSITORY:-YougLin-dev/nshw}"
home="${NSHW_HOME:-$HOME/.nshw}"
version="${NSHW_VERSION:-0.1.1}"
case "${1:-}" in
    '') default=0 ;;
    --set-default) default=1 ;;
    *) printf '%s\n' 'Usage: install.sh [--set-default]' >&2; exit 2 ;;
esac
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
url="${NSHW_BASE_URL:-https://github.com/$repo}/releases/download/v$version/$asset"
work=$(mktemp -d "${TMPDIR:-/tmp}/nshw-install.XXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM
curl_auth=
# Public releases need no authentication. A private repository uses gh's
# existing login (or NSHW_TOKEN), stored in a private curl config rather
# than a process argument, shell history or a URL.
token="${NSHW_TOKEN:-${GH_TOKEN:-${GITHUB_TOKEN:-}}}"
if [ -z "$token" ] && command -v gh >/dev/null 2>&1; then
    token=$(gh auth token 2>/dev/null || true)
fi
if [ -n "$token" ]; then
    umask 077
    printf 'header = "Authorization: Bearer %s"\n' "$token" > "$work/curl.auth"
    curl_auth="--config $work/curl.auth"
else
    printf '%s\n' 'no release token; for a private repository, use gh auth login or NSHW_TOKEN' >&2
fi
gh_assets=0
case "$repo" in
    http://*|https://*) ;;
    */*) gh_assets=1 ;;
esac
if [ -z "${NSHW_BASE_URL:-}" ] && [ -n "$token" ] && [ "$gh_assets" -eq 1 ]; then
    command -v gh >/dev/null 2>&1 || {
        printf '%s\n' 'private GitHub installation requires gh in PATH' >&2
        exit 1
    }
    list="$work/assets.ids"
    # gh obtains exact asset IDs from the API without putting credentials
    # in a URL. curl fetches only the binary/checksum bytes below.
    GH_TOKEN="$token" gh api "repos/$repo/releases/tags/v$version" \
        --jq '.assets[] | [.id, .name] | @tsv' > "$list"
    api="https://api.github.com/repos/$repo/releases/assets"
    printf 'header = "Accept: application/octet-stream"\n' >> "$work/curl.auth"
    for file in "$asset" "$asset.sha256"; do
        id=$(awk -F '\t' -v name="$file" '$2 == name { print $1; exit }' "$list")
        [ -n "$id" ] || { printf 'missing release asset: %s\n' "$file" >&2; exit 1; }
        curl -fSL --retry 2 --connect-timeout 30 --config "$work/curl.auth" "$api/$id" -o "$work/$file"
    done
    curl_ok=1
else
    curl_ok=0
fi
if [ "$curl_ok" -eq 0 ]; then
    # Public downloads and explicitly selected mirrors keep plain URLs.
    # shellcheck disable=SC2086
    curl -fSL --retry 2 --connect-timeout 30 $curl_auth "$url" -o "$work/$asset"
    # shellcheck disable=SC2086
    curl -fSL --retry 2 --connect-timeout 30 $curl_auth "$url.sha256" -o "$work/$asset.sha256"
fi
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$work/$asset" | cut -d ' ' -f 1)
else
    actual=$(shasum -a 256 "$work/$asset" | cut -d ' ' -f 1)
fi
expected=$(cut -d ' ' -f 1 < "$work/$asset.sha256")
expected=$(printf '%s' "$expected" | tr 'A-F' 'a-f')
case "$expected" in
    *[!0-9a-f]*|'') printf 'invalid checksum file\n' >&2; exit 1 ;;
esac
[ "${#expected}" -eq 64 ] && [ "$actual" = "$expected" ] || {
    printf 'SHA-256 mismatch for %s\n' "$url" >&2; exit 1
}
entries=$(tar -tzf "$work/$asset") || exit 1
for entry in $entries; do
    case "$entry" in
        nshw|nshw/|nshw/bin|nshw/bin/|nshw/bin/nshw|./nshw|./nshw/|./nshw/bin|./nshw/bin/|./nshw/bin/nshw) ;;
        *) printf 'unexpected archive entry: %s\n' "$entry" >&2; exit 1 ;;
    esac
done
mkdir -p "$work/unpack"
tar -xzf "$work/$asset" -C "$work/unpack"
chmod 755 "$work/unpack/nshw/bin/nshw"
# Refuse to replace a real nsh before replacing anything already installed.
if [ "$default" -eq 1 ] && [ -e "$home/bin/nsh" ] && [ ! -L "$home/bin/nsh" ]; then
    printf '%s exists and is not a symlink; not replacing it\n' "$home/bin/nsh" >&2
    exit 1
fi
mkdir -p "$home/bin"
# Rename a fully extracted file next to its destination. An interrupted
# download, checksum, extraction or staging step never touches a live nshw.
mv "$work/unpack/nshw/bin/nshw" "$home/bin/.nshw.$$.new"
# mv is atomic on the same filesystem; preserve the old install on failure.
if [ -e "$home/bin/nshw" ]; then
    mv "$home/bin/nshw" "$work/nshw.previous"
    restore=1
else
    restore=0
fi
if mv "$home/bin/.nshw.$$.new" "$home/bin/nshw"; then
    ln -sfn nshw "$home/bin/n"
    if [ "$default" -eq 1 ]; then
        ln -sfn nshw "$home/bin/nsh"
    fi
else
    [ "$restore" -eq 0 ] || mv "$work/nshw.previous" "$home/bin/nshw"
    exit 1
fi
printf 'installed nshw at %s (and n)\n' "$home/bin/nshw"
printf 'add %s/bin to PATH ahead of the distribution bin\n' "$home"
