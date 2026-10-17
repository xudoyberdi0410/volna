// swift-tools-version: 6.0

// Volna for iOS: a thin SwiftUI app over the Rust core (`crates/core`,
// C ABI in `include/volna_core.h`). Built on Linux with xtool (`xtool dev`).
//
// - CVolnaCore: the C header + libvolna_core.a from Libs/<platform>/
//   (`scripts/build-core.sh`).
// - VolnaKit: everything that is not UI (core bridge, models, player state,
//   platform-player and JS bridges). Foundation only, so `swift test` runs it
//   on Linux against the real core.
// - Volna: the app (SwiftUI, AVPlayer, WKWebView). Its files compile only for
//   iOS (`#if os(iOS)`), so `swift build` / `swift test` work on Linux too.

import PackageDescription

let libs = Context.packageDirectory + "/Libs"

let package = Package(
    name: "Volna",
    platforms: [
        .iOS(.v17),
        .macOS(.v14),
    ],
    products: [
        // xtool: exactly one library product, the app.
        .library(name: "Volna", targets: ["Volna"]),
    ],
    targets: [
        .target(
            name: "CVolnaCore",
            linkerSettings: [
                .unsafeFlags(["-L\(libs)/ios-arm64"], .when(platforms: [.iOS])),
                .unsafeFlags(["-L\(libs)/linux-x86_64"], .when(platforms: [.linux])),
                .linkedLibrary("volna_core"),
                // What the Rust staticlib needs from the SDK.
                .linkedFramework("Security", .when(platforms: [.iOS])),
                .linkedFramework("CoreFoundation", .when(platforms: [.iOS])),
                .linkedFramework("SystemConfiguration", .when(platforms: [.iOS])),
                .linkedLibrary("iconv", .when(platforms: [.iOS])),
                // ...and from glibc.
                .linkedLibrary("m", .when(platforms: [.linux])),
                .linkedLibrary("dl", .when(platforms: [.linux])),
                .linkedLibrary("pthread", .when(platforms: [.linux])),
                .linkedLibrary("gcc_s", .when(platforms: [.linux])),
            ]
        ),
        .target(
            name: "VolnaKit",
            dependencies: ["CVolnaCore"],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .target(
            name: "Volna",
            dependencies: ["VolnaKit"],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .testTarget(
            name: "VolnaKitTests",
            dependencies: ["VolnaKit"],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
