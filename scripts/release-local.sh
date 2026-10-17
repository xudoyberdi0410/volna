#!/usr/bin/env bash
# Windows NSIS installer cross-built on Linux, without CI: runs itself in the
# packaging/docker image (cargo-xwin, makensis, msitools). The installer
# carries GStreamer: its DLLs go next to the executables, the plugins into
# `gstreamer-1.0/` (the daemon points GStreamer there on start).
#
#   scripts/release-local.sh             # -> target-win/.../bundle/nsis/*.exe
#
# Builds go to target-win/ so they do not clash with the native target/.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE=volna-release
GST_VERSION=1.26.7
TRIPLE=x86_64-pc-windows-msvc

# Plugins the player needs (docs/native-player.md §4.6; scaletempo is in
# audiofx) plus level and the fallback audio sinks. Their DLL dependencies are
# found from the PE imports; gstsoup opens libsoup at runtime, so it is added
# by name.
PLUGINS=(coreelements playback typefindfunctions audioconvert audioresample volume
    autodetect wasapi wasapi2 directsound soup matroska isomp4 opus audioparsers
    audiofx level app libav)

if [[ "${1:-}" != --inside ]]; then
    docker build -t "$IMAGE" "$ROOT/packaging/docker"
    exec docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp \
        -v "$HOME/.cargo/registry:/opt/cargo/registry" -v "$ROOT:/src" "$IMAGE" bash /src/scripts/release-local.sh --inside
fi

export CARGO_TARGET_DIR=/src/target-win XWIN_CACHE_DIR=/src/target-win/xwin
WORK="$CARGO_TARGET_DIR/gstreamer"
mkdir -p "$WORK"

# GStreamer SDK: the official MSVC runtime and devel MSIs, unpacked.
if [[ ! -d "$WORK/sdk" ]]; then
    base="https://gstreamer.freedesktop.org/data/pkg/windows/$GST_VERSION/msvc"
    for m in "gstreamer-1.0-msvc-x86_64-$GST_VERSION.msi" "gstreamer-1.0-devel-msvc-x86_64-$GST_VERSION.msi"; do
        [[ -f "$WORK/$m" ]] || curl -fsSL --retry 3 -o "$WORK/$m" "$base/$m"
        mkdir -p "$WORK/sdk.tmp" && msiextract -C "$WORK/sdk.tmp" "$WORK/$m" >/dev/null
    done
    mv "$WORK/sdk.tmp" "$WORK/sdk"
fi
GST="$(dirname "$(dirname "$(dirname "$(find "$WORK/sdk" -name gstreamer-1.0.pc | head -1)")")")"
echo "GStreamer SDK at $GST"

# The .pc files name C:\gstreamer; --define-prefix rebases them onto the SDK.
mkdir -p "$WORK/bin"
printf '#!/bin/sh\nexec pkg-config --define-prefix "$@"\n' >"$WORK/bin/pkg-config"
chmod +x "$WORK/bin/pkg-config"
export PKG_CONFIG="$WORK/bin/pkg-config" PKG_CONFIG_PATH="$GST/lib/pkgconfig" PKG_CONFIG_ALLOW_CROSS=1

cd /src
for name in volna-xray volna-yt-dlp volna-deno volna-bgutil-pot; do
    [[ -f "src-tauri/binaries/$name-$TRIPLE.exe" ]] || { scripts/fetch-sidecars.sh windows; break; }
done

cargo xwin build --release --target "$TRIPLE" -p volna-daemon
cp "$CARGO_TARGET_DIR/$TRIPLE/release/volna.exe" "src-tauri/binaries/volna-$TRIPLE.exe"

# Plugins and every GStreamer DLL they (and the executables) import.
STAGE="$WORK/stage"
rm -rf "$STAGE" && mkdir -p "$STAGE/gstreamer-1.0"
for p in "${PLUGINS[@]}"; do
    if [[ -f "$GST/lib/gstreamer-1.0/gst$p.dll" ]]; then
        cp "$GST/lib/gstreamer-1.0/gst$p.dll" "$STAGE/gstreamer-1.0/"
    else
        echo "warning: no plugin gst$p.dll in the SDK" >&2
    fi
done
cp "$GST"/bin/soup-3.0*.dll "$STAGE/"
queue=("$STAGE"/gstreamer-1.0/*.dll "$STAGE"/soup-3.0*.dll "src-tauri/binaries/volna-$TRIPLE.exe")
while ((${#queue[@]})); do
    file="${queue[0]}" && queue=("${queue[@]:1}")
    while read -r dll; do
        [[ -f "$GST/bin/$dll" && ! -f "$STAGE/$dll" ]] || continue
        cp "$GST/bin/$dll" "$STAGE/"
        queue+=("$STAGE/$dll")
    done < <(llvm-objdump -p "$file" | sed -n 's/^ *DLL Name: //p' | tr -d '\r')
done
# The MSVC runtime GStreamer links against, deployed app-local next to the
# DLLs (the VC++ redistributable licence allows it): Windows does not always
# have it. vc_redist.x64.exe is a Burn bundle; its cabinets are carved out of
# the executable and unpacked twice.
VC="$WORK/vcredist"
if [[ ! -f "$VC/vcruntime140.dll" ]]; then
    rm -rf "$VC" && mkdir -p "$VC/cab"
    curl -fsSL -o "$VC/vc_redist.x64.exe" https://aka.ms/vs/17/release/vc_redist.x64.exe
    python3 -I -c '
import re, sys
b = open(sys.argv[1], "rb").read()
for i, m in enumerate(re.finditer(rb"MSCF\0\0\0\0", b)):
    size = int.from_bytes(b[m.start() + 8:m.start() + 12], "little")
    open(f"{sys.argv[2]}/{i}.cab", "wb").write(b[m.start():m.start() + size])
' "$VC/vc_redist.x64.exe" "$VC/cab"
    for c in "$VC"/cab/*.cab; do 7z x -y -o"$VC/payload" "$c" >/dev/null; done
    for f in "$VC"/payload/*; do 7z x -y -o"$VC/files" "$f" >/dev/null 2>&1 || true; done
    for dll in vcruntime140.dll vcruntime140_1.dll msvcp140.dll; do
        cp "$VC/files/${dll}_amd64" "$VC/$dll"
    done
fi
cp "$VC"/{vcruntime140.dll,vcruntime140_1.dll,msvcp140.dll} "$STAGE/"
echo "GStreamer for the installer: $(find "$STAGE" -name '*.dll' | wc -l) DLLs, $(du -sh "$STAGE" | cut -f1)"

# Each staged file as a resource at the same relative path in the install dir.
(cd "$STAGE" && find . -name '*.dll' | sed 's|^\./||' | sort) | awk -v s="$STAGE" '
    BEGIN { printf "{\"bundle\":{\"resources\":{" }
    { printf "%s\"%s/%s\":\"%s\"", (NR > 1 ? "," : ""), s, $0, $0 }
    END { print "}}}" }' >"$WORK/tauri.gstreamer.json"

cd ui
[[ -d node_modules ]] || bun install --frozen-lockfile
bun run tauri build --runner cargo-xwin --target "$TRIPLE" --bundles nsis \
    --config ../src-tauri/tauri.bundle.json --config "$WORK/tauri.gstreamer.json"
ls -l "$CARGO_TARGET_DIR/$TRIPLE/release/bundle/nsis/"
