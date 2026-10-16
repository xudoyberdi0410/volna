#!/usr/bin/env bash
# Downloads the xray and yt-dlp sidecars into src-tauri/binaries/ with the
# target-triple names Tauri's externalBin expects, and checks every file
# against the SHA-256 published with the release (and against the hashes
# pinned below, when the default versions are used).
#
#   scripts/fetch-sidecars.sh              # sidecars for this machine
#   scripts/fetch-sidecars.sh linux        # x86_64-unknown-linux-gnu
#   scripts/fetch-sidecars.sh windows      # x86_64-pc-windows-msvc
#   scripts/fetch-sidecars.sh all
#
# Override versions with XRAY_VERSION=v26.3.27 YTDLP_VERSION=2026.08.19
# (the pinned hashes are then skipped; release checksums are still checked).
# Only x86_64 assets are fetched. Works in Linux bash and in Git Bash on Windows (GitHub Actions).

set -euo pipefail

DEFAULT_XRAY_VERSION="v26.3.27"
DEFAULT_YTDLP_VERSION="2026.08.19"
XRAY_VERSION="${XRAY_VERSION:-$DEFAULT_XRAY_VERSION}"
YTDLP_VERSION="${YTDLP_VERSION:-$DEFAULT_YTDLP_VERSION}"

# SHA-256 of the release assets for the default versions.
declare -A PINNED=(
    ["Xray-linux-64.zip"]="23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae"
    ["Xray-windows-64.zip"]="d004c39288ce9ada487c6f398c7c545f7d749e44bdfdd59dbc9f865afba4e1ad"
    ["yt-dlp_linux"]="58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a"
    ["yt-dlp.exe"]="66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a"
)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/src-tauri/binaries"
XRAY_BASE="https://github.com/XTLS/Xray-core/releases/download/$XRAY_VERSION"
YTDLP_BASE="https://github.com/yt-dlp/yt-dlp/releases/download/$YTDLP_VERSION"

die() {
    echo "fetch-sidecars: $*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "'$1' is required"
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

download() { # url dest
    echo "  get $1"
    curl --fail --location --silent --show-error --retry 3 --output "$2" "$1"
}

# verify FILE ASSET_NAME EXPECTED_FROM_RELEASE
verify() {
    local file="$1" name="$2" expected="${3,,}" actual
    actual="$(sha256 "$file")"
    [[ -n "$expected" ]] || die "no published checksum for $name"
    [[ "$actual" == "$expected" ]] || die "$name: sha256 $actual, release says $expected"
    if [[ "$XRAY_VERSION" == "$DEFAULT_XRAY_VERSION" && "$YTDLP_VERSION" == "$DEFAULT_YTDLP_VERSION" ]]; then
        local pinned="${PINNED[$name]:-}"
        if [[ -n "$pinned" && "$actual" != "$pinned" ]]; then
            die "$name: sha256 $actual does not match the pinned $pinned"
        fi
    fi
    echo "  ok  $name $actual"
}

unzip_one() { # zip member dest
    if command -v unzip >/dev/null 2>&1; then
        unzip -p "$1" "$2" >"$3"
    else
        local py
        py="$(command -v python3 || command -v python || true)"
        [[ -n "$py" ]] || die "need unzip or python to extract $1"
        "$py" -c 'import sys, zipfile; sys.stdout.buffer.write(zipfile.ZipFile(sys.argv[1]).read(sys.argv[2]))' "$1" "$2" >"$3"
    fi
}

# fetch_xray OS(linux|windows) TRIPLE
fetch_xray() {
    local os="$1" triple="$2" asset exe="" member="xray"
    asset="Xray-$os-64.zip"
    if [[ "$os" == windows ]]; then
        exe=".exe"
        member="xray.exe"
    fi
    echo "xray $XRAY_VERSION ($triple)"
    download "$XRAY_BASE/$asset" "$TMP/$asset"
    download "$XRAY_BASE/$asset.dgst" "$TMP/$asset.dgst"
    # .dgst lines look like "SHA2-256= <hex>" (older ones "SHA256= <hex>").
    local expected
    expected="$(grep -E '^SHA2?-?256=' "$TMP/$asset.dgst" | head -n1 | sed 's/^[^=]*= *//' | tr -d '\r ')"
    verify "$TMP/$asset" "$asset" "$expected"
    unzip_one "$TMP/$asset" "$member" "$OUT/volna-xray-$triple$exe"
    chmod +x "$OUT/volna-xray-$triple$exe"
}

# fetch_ytdlp ASSET TRIPLE
fetch_ytdlp() {
    local asset="$1" triple="$2" exe=""
    [[ "$asset" == *.exe ]] && exe=".exe"
    echo "yt-dlp $YTDLP_VERSION ($triple)"
    [[ -f "$TMP/SHA2-256SUMS" ]] || download "$YTDLP_BASE/SHA2-256SUMS" "$TMP/SHA2-256SUMS"
    download "$YTDLP_BASE/$asset" "$TMP/$asset"
    local expected
    expected="$(awk -v n="$asset" '$2 == n { print $1 }' "$TMP/SHA2-256SUMS" | tr -d '\r')"
    verify "$TMP/$asset" "$asset" "$expected"
    mv "$TMP/$asset" "$OUT/volna-yt-dlp-$triple$exe"
    chmod +x "$OUT/volna-yt-dlp-$triple$exe"
}

fetch_linux() {
    fetch_xray linux x86_64-unknown-linux-gnu
    fetch_ytdlp yt-dlp_linux x86_64-unknown-linux-gnu
}

fetch_windows() {
    fetch_xray windows x86_64-pc-windows-msvc
    fetch_ytdlp yt-dlp.exe x86_64-pc-windows-msvc
}

host_target() {
    case "$(uname -s)" in
        Linux) echo linux ;;
        MINGW* | MSYS* | CYGWIN* | Windows_NT) echo windows ;;
        *) die "unsupported host $(uname -s); pass linux or windows" ;;
    esac
}

main() {
    need curl
    local target="${1:-$(host_target)}"
    mkdir -p "$OUT"
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    case "$target" in
        linux) fetch_linux ;;
        windows) fetch_windows ;;
        all) fetch_linux && fetch_windows ;;
        *) die "unknown target '$target' (linux, windows, all)" ;;
    esac
    echo "sidecars in $OUT:"
    ls -l "$OUT"
}

main "$@"
