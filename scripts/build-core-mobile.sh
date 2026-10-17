#!/usr/bin/env bash
# volna-core for the mobile apps, cross-built on Linux (docs/mobile-build.md).
#
#   scripts/build-core-mobile.sh android [--release]   # libvolna_core.so: arm64-v8a, x86_64
#   scripts/build-core-mobile.sh ios [--release]       # libvolna_core.a: aarch64-apple-ios
#   scripts/build-core-mobile.sh header                # crates/core/include/volna_core.h
#
# Android needs the NDK (ANDROID_NDK_HOME, default ~/Android/Sdk/ndk/25.2.9519653)
# and cargo-ndk. iOS needs the Darwin SDK that `xtool sdk install` puts in
# ~/.swiftpm/swift-sdks/darwin.artifactbundle (its ld64.lld understands the
# SDK's .tbd stubs; the system lld may be too old).
#
# Mobile builds drop yt-dlp: --no-default-features --features ffi.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
what="${1:-}"
shift || true
features=(-p volna-core --no-default-features --features ffi)

case "$what" in
android)
    export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/25.2.9519653}"
    cd "$ROOT"
    exec cargo ndk -t arm64-v8a -t x86_64 build "${features[@]}" "$@"
    ;;
ios)
    bundle="${DARWIN_SDK_BUNDLE:-$HOME/.swiftpm/swift-sdks/darwin.artifactbundle}"
    sdk="$bundle/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk"
    [ -d "$sdk" ] || { echo "no iPhoneOS.sdk at $sdk (run: xtool sdk install <Xcode.xip>)" >&2; exit 1; }
    target=arm64-apple-ios15.0
    export SDKROOT="$sdk"
    export IPHONEOS_DEPLOYMENT_TARGET=15.0
    # C parts (SQLite, ring) with the host clang against the iOS SDK.
    export CC_aarch64_apple_ios=clang
    export CXX_aarch64_apple_ios=clang++
    export AR_aarch64_apple_ios=llvm-ar
    export CFLAGS_aarch64_apple_ios="--target=$target -isysroot $sdk"
    export CXXFLAGS_aarch64_apple_ios="--target=$target -isysroot $sdk"
    # Linking (the cdylib, and checking that the staticlib resolves): clang
    # driving xtool's ld64.lld.
    export CARGO_TARGET_AARCH64_APPLE_IOS_LINKER=clang
    export CARGO_TARGET_AARCH64_APPLE_IOS_RUSTFLAGS="-C link-arg=--target=$target -C link-arg=-isysroot -C link-arg=$sdk -C link-arg=-B$bundle/toolset/bin -C link-arg=-fuse-ld=lld -C link-arg=-L$sdk/usr/lib -C link-arg=-lSystem -C link-arg=-F$sdk/System/Library/Frameworks -C link-arg=-framework -C link-arg=Security -C link-arg=-framework -C link-arg=CoreFoundation -C link-arg=-framework -C link-arg=SystemConfiguration"
    cd "$ROOT"
    exec cargo build --target aarch64-apple-ios "${features[@]}" "$@"
    ;;
header)
    cd "$ROOT/crates/core"
    exec cbindgen --config cbindgen.toml --crate volna-core --output include/volna_core.h
    ;;
*)
    sed -n '2,13p' "$0"
    exit 2
    ;;
esac
