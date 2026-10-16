#!/usr/bin/env bash
# Release build: fetches the sidecars if they are missing, then runs
# `tauri build` with src-tauri/tauri.bundle.json merged in (it adds the
# xray and yt-dlp externalBin entries, which plain dev builds do without).
#
#   scripts/build.sh                    # Linux: AppImage + deb; Windows: NSIS
#   scripts/build.sh --bundles deb      # extra args go to `tauri build`

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

case "$(uname -s)" in
    Linux) triple=x86_64-unknown-linux-gnu exe="" ;;
    MINGW* | MSYS* | CYGWIN* | Windows_NT) triple=x86_64-pc-windows-msvc exe=".exe" ;;
    *) echo "build: unsupported host $(uname -s)" >&2 && exit 1 ;;
esac

for name in volna-xray volna-yt-dlp; do
    if [[ ! -f "$ROOT/src-tauri/binaries/$name-$triple$exe" ]]; then
        "$ROOT/scripts/fetch-sidecars.sh"
        break
    fi
done

cd "$ROOT/ui"
[[ -d node_modules ]] || bun install --frozen-lockfile
bun run tauri build --config ../src-tauri/tauri.bundle.json "$@"
