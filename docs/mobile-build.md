# Building volna-core for Android and iOS

`crates/core` (`volna-core`) is everything the UI calls, minus desktop hosting
(`docs/flutter-ui-plan.md`, phase 6). The desktop daemon links it as an rlib; the
mobile apps link it through the C ABI in `crates/core/src/ffi.rs`
(`crates/core/include/volna_core.h`). Everything below runs on Linux and was
checked on 2026-10-17.

## Features

| feature | default | what |
|---|---|---|
| `ytdlp` | yes | stream resolution, cookies and the Yandex browser session through yt-dlp (desktop) |
| `ffi` | yes | the C ABI |
| `innertube` | no | in-process stream resolution (volna-streams InnerTube); its JavaScript runs in the platform WebView through the JS callback |

Mobile builds use `--no-default-features --features ffi,innertube`. The stream
backend is injected at init (`CoreConfig::backend`, `StreamBackend::injected`):
with `innertube`, `volna_core_init` builds `volna_streams::innertube::InnerTube`
over the FFI JS callback (players cached in `<cacheDir>/players`, cookies from
`youtubeCookies`). Without it `backend` is `None` and playback answers `no_ytdlp`.

`volna-player` builds without GStreamer when its `gstreamer` feature is off,
and `volna-streams` without yt-dlp when its `ytdlp` feature is off; both are
workspace dependencies with `default-features = false`, and `volna-core/ytdlp`
turns `volna-streams/ytdlp` on (the daemon asks for it directly).

## One-time setup

```sh
rustup target add aarch64-linux-android x86_64-linux-android aarch64-apple-ios
cargo install cargo-ndk cbindgen
```

- Android NDK: `~/Android/Sdk/ndk/25.2.9519653` (`ANDROID_NDK_HOME` overrides).
- iOS: the Darwin SDK from `xtool sdk install Xcode_27.1.xip`, in
  `~/.swiftpm/swift-sdks/darwin.artifactbundle`. Its `toolset/bin/ld64.lld` is
  needed: the system lld (LLD 23.1) rejects the iOS 27 SDK's `.tbd` stubs
  ("unknown target").

## Commands

```sh
scripts/build-core-mobile.sh android [--release]   # target/{aarch64,x86_64}-linux-android/*/libvolna_core.so
scripts/build-core-mobile.sh ios [--release]       # target/aarch64-apple-ios/*/libvolna_core.a (+ .dylib)
scripts/build-core-mobile.sh header                # regenerate include/volna_core.h
```

Android is plain `cargo ndk -t arm64-v8a -t x86_64 build -p volna-core
--no-default-features --features ffi,innertube`. The `.so` needs only `libc`, `libm`, `libdl`.

iOS, spelled out (what the script sets):

```sh
SDK=~/.swiftpm/swift-sdks/darwin.artifactbundle/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk
export SDKROOT=$SDK IPHONEOS_DEPLOYMENT_TARGET=15.0
export CC_aarch64_apple_ios=clang CXX_aarch64_apple_ios=clang++ AR_aarch64_apple_ios=llvm-ar
export CFLAGS_aarch64_apple_ios="--target=arm64-apple-ios15.0 -isysroot $SDK"
export CARGO_TARGET_AARCH64_APPLE_IOS_LINKER=clang
export CARGO_TARGET_AARCH64_APPLE_IOS_RUSTFLAGS="-C link-arg=--target=arm64-apple-ios15.0 \
  -C link-arg=-isysroot -C link-arg=$SDK \
  -C link-arg=-B$HOME/.swiftpm/swift-sdks/darwin.artifactbundle/toolset/bin -C link-arg=-fuse-ld=lld \
  -C link-arg=-L$SDK/usr/lib -C link-arg=-lSystem -C link-arg=-F$SDK/System/Library/Frameworks \
  -C link-arg=-framework -C link-arg=Security -C link-arg=-framework -C link-arg=CoreFoundation \
  -C link-arg=-framework -C link-arg=SystemConfiguration"
cargo build -p volna-core --no-default-features --features ffi,innertube --target aarch64-apple-ios
```

The linker settings only matter for the `.dylib` the crate type list also
produces; it links, which shows the static library's symbols resolve against
the SDK (libSystem, libiconv, Security, CoreFoundation, SystemConfiguration).
ld64.lld warns "does not support linking for platform iOS. Proceed at your own
risk"; harmless here. The app itself links `libvolna_core.a` through the SwiftPM
C-target pattern in `docs/flutter-ui-plan.md` ("Facts verified").

What was expected to break and did not: rusqlite `bundled` (SQLite C) and
rustls with `ring` (reqwest `rustls-tls`, webpki roots) cross-compile for both
targets with the host clang / the NDK clang.

## The C ABI in short

```c
VolnaCore *volna_core_init(const char *config_json); // {"dataDir":…,"cacheDir"?,"audioServer"?,"log"?,"youtubeCookies"?}; NULL → volna_core_last_error()
void volna_core_set_reply_callback(VolnaCore *, VolnaReplyCallback, void *user_data);   // (user, id, ok, json)
void volna_core_set_event_callback(VolnaCore *, VolnaEventCallback, void *user_data);   // (user, name, payload_json)
void volna_core_set_proto_callback(VolnaCore *, VolnaProtoCallback, void *user_data);   // (user, id, status, headers_json, body, len)
void volna_core_set_player_callback(VolnaCore *, VolnaPlayerCallback, void *user_data); // (user, command_json)
void volna_core_set_js_callback(VolnaCore *, VolnaJsCallback, void *user_data);         // (user, request_id, request_json)
void volna_core_call(VolnaCore *, uint64_t id, const char *cmd, const char *args_json);
void volna_core_proto(VolnaCore *, uint64_t id, const char *uri);
bool volna_core_player_report(VolnaCore *, const char *event_json);
bool volna_core_js_result(VolnaCore *, uint64_t request_id, bool ok, const char *result_json);
const char *volna_core_js_prelude(void);                                               // static, do not free
void volna_core_free_string(char *);
void volna_core_free_bytes(uint8_t *, size_t);
void volna_core_shutdown(VolnaCore *);
```

- Commands, args and results are the daemon IPC ones (`dispatch.rs`,
  `player/commands.rs`, `api/*`); an error reply is a JSON string
  `"<kind>: <message>"`. Mobile networking is direct only: `net_apply` with a
  proxy answers `unsupported: …`.
- Every string or body given to a callback is owned by the receiver: free it with
  `volna_core_free_string` / `volna_core_free_bytes`. Callbacks come from core
  threads (fine for Dart `NativeCallable.listener`).
- Platform player commands: `{"op":"load","url","startMs"}`, `play`, `pause`,
  `{"op":"seek","positionMs"}`, `stop`, `{"op":"setVolume","volume"}`. The URL is
  always the loopback audio server. Reports: `{"type":"ready","durationMs"?}`
  after a load, `playing`, `paused`, `{"type":"position","positionMs","durationMs"?}`
  every few hundred ms while playing, `{"type":"buffering","on"}`, `ended`,
  `{"type":"error","message","decode"?}`. Reports are about the latest load.
  The core sends `stop` before loading the next item.
- JS jobs (for the mobile stream resolver): load `volna_core_js_prelude()` once
  into a WebView page that stays alive (BotGuard needs a real DOM); then for each
  job evaluate `volnaJs(<request_json>)` and pass the string it resolves to,
  unchanged, to `volna_core_js_result(…, ok=true, …)`. `ok=false` only when the
  job could not run (no WebView). The requests and answers are in
  `docs/streams.md` ("JsRuntime contract"); the trait is
  `volna_streams::js::JsRuntime`, re-exported by `volna_core::js`.

## Not done

- The InnerTube resolver ran only against Deno + jsdom on desktop
  (`docs/streams.md`); no Android WebView / WKWebView host exists yet. Cookies are
  set once at init (`youtubeCookies`); there is no command to change them later.
- On the platform player there is no crossfade, gapless handoff, EQ or
  visualizer (the output is single-deck; `player_set_dsp` is stored but not
  applied; visualizer frames are never sent). Fades jump to their target volume.
- Release builds and app-side linking (Gradle `jniLibs`, the SwiftPM target) are
  phases 7 and 8.
