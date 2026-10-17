#!/usr/bin/env bash
# libvolna_core.a for the iOS app and for `swift test` on Linux, copied into
# ios/Libs/<platform>/ where Package.swift links it from.
#
#   ios/scripts/build-core.sh            # both
#   ios/scripts/build-core.sh ios        # aarch64-apple-ios, release (what goes on the phone)
#   ios/scripts/build-core.sh linux      # host, debug (devProxy works only in debug builds)
#
# IOS_PROFILE=debug builds the iOS library unoptimised (much larger, slower).
# Respects CARGO_TARGET_DIR.

set -euo pipefail

IOS="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT="$(cd "$IOS/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
what="${1:-all}"

ios() {
    local profile="${IOS_PROFILE:-release}" flag=()
    [ "$profile" = release ] && flag=(--release)
    "$ROOT/scripts/build-core-mobile.sh" ios "${flag[@]}"
    mkdir -p "$IOS/Libs/ios-arm64"
    cp "$TARGET_DIR/aarch64-apple-ios/$profile/libvolna_core.a" "$IOS/Libs/ios-arm64/"
}

linux() {
    (cd "$ROOT" && cargo build -p volna-core --no-default-features --features ffi)
    mkdir -p "$IOS/Libs/linux-x86_64"
    # Only the static library: with a .so next to it the linker would pick that.
    cp "$TARGET_DIR/debug/libvolna_core.a" "$IOS/Libs/linux-x86_64/"
}

case "$what" in
all) ios; linux ;;
ios) ios ;;
linux) linux ;;
*) sed -n '2,11p' "$0"; exit 2 ;;
esac
