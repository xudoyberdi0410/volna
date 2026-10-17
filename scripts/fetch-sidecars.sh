#!/usr/bin/env bash
# Downloads the xray, yt-dlp, Deno and bgutil-pot sidecars into src-tauri/binaries/ with the
# target-triple names Tauri's externalBin expects, and checks every file
# against the SHA-256 published with the release (and against the hashes
# pinned below, when the default versions are used).
#
#   scripts/fetch-sidecars.sh              # sidecars for this machine
#   scripts/fetch-sidecars.sh linux        # x86_64-unknown-linux-gnu
#   scripts/fetch-sidecars.sh windows      # x86_64-pc-windows-msvc
#   scripts/fetch-sidecars.sh all
#
# Deno runs YouTube's player JavaScript for yt-dlp; bgutil-pot is the PO token
# provider (script mode), with its yt-dlp plugin zip in yt-dlp-plugins/.
#
# Override versions with XRAY_VERSION=v26.3.27 YTDLP_VERSION=2026.08.19
# (the pinned hashes are then skipped; release checksums are still checked).
# Deno and bgutil-pot are pinned only: bgutil publishes no checksum file.
# Only x86_64 assets are fetched. Works in Linux bash and in Git Bash on Windows (GitHub Actions).

set -euo pipefail

DEFAULT_XRAY_VERSION="v26.3.27"
DEFAULT_YTDLP_VERSION="2026.08.19"
DENO_VERSION="v2.9.7"
BGUTIL_VERSION="v0.8.1"
XRAY_VERSION="${XRAY_VERSION:-$DEFAULT_XRAY_VERSION}"
YTDLP_VERSION="${YTDLP_VERSION:-$DEFAULT_YTDLP_VERSION}"

# SHA-256 of the release assets for the default versions.
declare -A PINNED=(
    ["Xray-linux-64.zip"]="23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae"
    ["Xray-windows-64.zip"]="d004c39288ce9ada487c6f398c7c545f7d749e44bdfdd59dbc9f865afba4e1ad"
    ["yt-dlp_linux"]="58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a"
    ["yt-dlp.exe"]="66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a"
    ["deno-x86_64-unknown-linux-gnu.zip"]="c6527f24f4b16031d3ae4fa9f658d5f11534c8d84ce7dc8502420280919c3490"
    ["deno-x86_64-pc-windows-msvc.zip"]="a0c3101b4158d1dfb7d6a78a7bf0f3de80c96bb423c152beec8beb22786f2238"
    ["bgutil-pot-linux-x86_64"]="e7c264a574fa2705b6e5dc62283a8a4e80130f27b9d7e9df44e6b09aa6151a87"
    ["bgutil-pot-windows-x86_64.exe"]="25d6b05c79176aa792454c3d1727922ca47e56cf11cb1e866615d751819b14a0"
    ["bgutil-ytdlp-pot-provider-rs.zip"]="99fd83b98fa93b193d6a3b69dc74410d76e7a2b889868c54d16121cac9060344"
)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/src-tauri/binaries"
XRAY_BASE="https://github.com/XTLS/Xray-core/releases/download/$XRAY_VERSION"
YTDLP_BASE="https://github.com/yt-dlp/yt-dlp/releases/download/$YTDLP_VERSION"
DENO_BASE="https://github.com/denoland/deno/releases/download/$DENO_VERSION"
BGUTIL_BASE="https://github.com/jim60105/bgutil-ytdlp-pot-provider-rs/releases/download/$BGUTIL_VERSION"

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

# fetch_deno TRIPLE
fetch_deno() {
    local triple="$1" asset="deno-$1.zip" exe="" expected
    [[ "$triple" == *windows* ]] && exe=".exe"
    echo "deno $DENO_VERSION ($triple)"
    download "$DENO_BASE/$asset" "$TMP/$asset"
    download "$DENO_BASE/$asset.sha256sum" "$TMP/$asset.sha256sum"
    # "<hex>  name" on Linux, a PowerShell Get-FileHash listing on Windows.
    expected="$(grep -oiE '\b[0-9a-f]{64}\b' "$TMP/$asset.sha256sum" | head -n1)"
    verify "$TMP/$asset" "$asset" "$expected"
    unzip_one "$TMP/$asset" "deno$exe" "$OUT/volna-deno-$triple$exe"
    chmod +x "$OUT/volna-deno-$triple$exe"
}

# fetch_pinned URL ASSET DEST: an asset checked against PINNED only.
fetch_pinned() {
    download "$1" "$TMP/$2"
    verify "$TMP/$2" "$2" "${PINNED[$2]}"
    mv "$TMP/$2" "$3"
}

# fetch_bgutil ASSET TRIPLE
fetch_bgutil() {
    local asset="$1" triple="$2" exe=""
    [[ "$asset" == *.exe ]] && exe=".exe"
    echo "bgutil-pot $BGUTIL_VERSION ($triple)"
    fetch_pinned "$BGUTIL_BASE/$asset" "$asset" "$OUT/volna-bgutil-pot-$triple$exe"
    chmod +x "$OUT/volna-bgutil-pot-$triple$exe"
    mkdir -p "$OUT/yt-dlp-plugins"
    local plugin=bgutil-ytdlp-pot-provider-rs.zip
    [[ -f "$OUT/yt-dlp-plugins/$plugin" ]] ||
        fetch_pinned "$BGUTIL_BASE/$plugin" "$plugin" "$OUT/yt-dlp-plugins/$plugin"
}

fetch_linux() {
    fetch_xray linux x86_64-unknown-linux-gnu
    fetch_ytdlp yt-dlp_linux x86_64-unknown-linux-gnu
    fetch_deno x86_64-unknown-linux-gnu
    fetch_bgutil bgutil-pot-linux-x86_64 x86_64-unknown-linux-gnu
}

fetch_windows() {
    fetch_xray windows x86_64-pc-windows-msvc
    fetch_ytdlp yt-dlp.exe x86_64-pc-windows-msvc
    fetch_deno x86_64-pc-windows-msvc
    fetch_bgutil bgutil-pot-windows-x86_64.exe x86_64-pc-windows-msvc
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
