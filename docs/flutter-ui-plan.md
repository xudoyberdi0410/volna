# Flutter UI migration plan (agent-facing)

Replace the webview window (`src-tauri/` + `ui/`, Monochrome fork) with a Flutter window.
Goals: light, good-looking, fast to build. Later the same Flutter app targets Android; iOS gets
a SwiftUI app over the same Rust core (see "Mobile").

## Fixed decisions (do not revisit)

- UI: Flutter (Dart). Everything else stays Rust. No business logic, networking or YouTube
  access in Dart.
- Desktop keeps the daemon (`crates/daemon`, binary `volna`). The Flutter app is a drop-in
  replacement for `volna-window`: it connects to the daemon socket and speaks `ipc.rs` frames.
  No FFI / flutter_rust_bridge on desktop.
- `api_emu` (TIDAL hifi-api emulation) is dropped. The daemon gets a native API returning
  `crates/ytm` models directly. `docs/hifi-api-contract.md` becomes obsolete.
- First target: Linux. Then Windows. Then mobile (phases 5–8).
- New daemon commands (catalog, library, lyrics, scrobblers, yandex, color) must not depend on
  GStreamer, the socket or desktop-only crates: they move into `volna-core` for mobile (phase 6).
  Keep them in modules that only take `ytm`/`net`/SQLite handles.
- Kept from the old UI: wave, player (mini + fullscreen + queue), search, album, artist,
  playlist, library, synced lyrics (all providers), settings (network, theme, quality),
  dynamic color from cover, scrobbling (Last.fm, Libre.fm, ListenBrainz, Maloja),
  Yandex Music import.
- Dropped: AutoEQ / parametric / graphic EQ / binaural DSP / mono / silence removal,
  butterchurn and other visualizers, waveform, CD cover, tilt,
  local files (taglib), folders, "unreleased", donate, PWA, command palette,
  playlist file import, playlist generator, home feed sections (editor's picks etc.),
  the ~90 interface toggles. Do not port them.
- Track downloading stays removed (CLAUDE.md).

## Current state (facts to rely on)

- IPC: `crates/daemon/src/ipc.rs`. Unix socket `$XDG_RUNTIME_DIR/volna/daemon.sock`
  (`VOLNA_SOCKET` overrides; Windows: loopback TCP + token from port file, see `connect`).
  Frame = `u32 LE header len, u32 LE body len, JSON header, raw body`. Header is `Msg`
  (serde tag `t`, snake_case): `hello{role:"window",token?}` → `welcome{stream_base}`;
  `call{id,cmd,args}` → `reply{id,ok|err}`; `proto{id,method,uri,headers}` + body →
  `proto_reply{id,status,headers}` + body; daemon → window: `event{name,payload}`, `vis`,
  `show`, `quit`.
- Commands: `crates/core/src/dispatch.rs` (`call`) + `crates/core/src/player/commands.rs`
  (`player_*`, incl. `player_wave_*`, `player_snapshot`, `player_journal`). Args use camelCase keys.
- Playback is GStreamer in the daemon (`crates/player`); the window only shows state.
  MPRIS and tray are in the daemon (`player/mpris.rs`, `player/tray.rs`).
- `volna://` handler: `crates/daemon/src/protocol.rs`; images and the lyrics relay
  (`src-tauri/src/api_emu/proxy.rs`, allowlist mirrored in `ui/js/lyrics-proxy.js`) go
  through it, so they already use the app proxy.
- YTM client API: `crates/ytm/src/api.rs` (`search`, `search_more`, `search_suggestions`,
  `track`, `radio`, `watch_playlist`, `queue_more`, `album`, `artist`, `playlist`,
  `playlist_more`, `playlist_all`, `related`, `lyrics`, `moods_and_genres`, `home`).
- User library lives in the webview's IndexedDB (`ui/js/db.js`: `favorites_tracks`,
  `favorites_albums`, `favorites_artists`, `favorites_playlists`, `favorites_mixes`,
  `favorites_videos`, `history_tracks`, `user_playlists`, `user_folders`, `pinned_items`,
  `settings`) and localStorage (`ui/js/storage.js`, Last.fm session in `ui/js/lastfm.js`).
  It must be migrated, not lost.
- Scrobblers: `ui/js/multi-scrobbler.js` fans out to `lastfm.js`, `librefm.js`,
  `listenbrainz.js`, `maloja.js`; finished plays come from the player journal (`scrobbleAt`).
  Settings keys in `ui/js/storage.js`: `lastfm-*` (incl. custom API key/secret, scrobble
  percentage, love on like), `librefm-session`, `librefm-enabled`, `librefm-love-on-like`,
  `listenbrainz-enabled/-token/-custom-url/-love-on-like`, `maloja-enabled/-token/-custom-url`.
- Lyrics chain (in `@uimaxbai/am-lyrics` `src/AmLyrics.ts` `fetchLyrics`, plus
  `ui/js/lyrics.js`): lrc.red (TTML by ISRC, word sync) → BiniLyrics (TTML) → Unison →
  YouLyPlus/lyricsplus (Apple Music, Musixmatch, Spotify, QQ backends) → LRCLIB → Genius →
  YouTube Music (`ytm::lyrics`, plain). Hosts: `ui/js/lyrics-proxy.js`.
- Yandex: `src-tauri/src/yandex.rs` (token from browser cookies via yt-dlp, else a Tauri
  login webview window catching `#access_token=`), matching to YTM in `ui/js/yandex-import.js`.
- Wave: `crates/core/src/wave.rs` + `crates/wave`.

## Target layout

```
app/                      Flutter project (linux, windows; android in phase 7; no ios)
  lib/ipc/                transport interface (call/reply, events, proto) + socket transport
                          (desktop); FFI transport added in phase 7
  lib/api/                typed wrappers over daemon commands + models (mirror Rust serde)
  lib/state/              app state (pick one: riverpod; no other state libs)
  lib/ui/                 screens + widgets
crates/daemon/src/api/    new native API handlers (catalog, library, lyrics, lastfm, yandex, color)
crates/lyrics/            new crate: provider chain, TTML/LRC parsing, cache
crates/scrobble/          new crate: Last.fm, Libre.fm, ListenBrainz, Maloja, offline queue
crates/core/              phase 6: `volna-core`, everything the UI calls, minus desktop hosting
ios/                      phase 8: SwiftUI app (xtool SwiftPM project)
```

`lib/api` must only talk to the transport interface, never to sockets directly, so Android
reuses it unchanged over FFI.

Window process contract stays the same as `volna-window`: daemon spawns/finds it, `show`/`quit`
messages, single instance. Check how the daemon launches the window (`daemon.rs`, `main.rs`) and
point it at the Flutter binary (`volna-window` name can be kept for the Flutter binary to avoid
touching the launcher).

## Phases

Each phase ends with: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`,
`cd app && flutter analyze && flutter test`, an atomic commit per logical change with
`Co-Authored-By`. The old UI must keep working until phase 4 (both windows can coexist; only one
connects at a time).

### Phase 0. Toolchain
- Install Flutter SDK (AUR `flutter-bin` or official tarball; ask user before installing).
- `flutter create --platforms=linux,windows --org uz.khudoberdi app`.
- Done when `cd app && flutter run -d linux` shows an empty window.

### Phase 1. IPC client + native API skeleton (~1–2 days)
Dart:
- `lib/ipc/frame.dart`: encode/decode frames exactly as `ipc.rs` (`read_frame`/`write_frame`),
  64 MB part limit. Unit tests with byte fixtures produced by a Rust test (add a test in
  `ipc.rs` that writes fixtures to `app/test/fixtures/` or hardcode bytes in both).
- `lib/ipc/connection.dart`: socket path resolution identical to Rust (`VOLNA_SOCKET`,
  `$XDG_RUNTIME_DIR/volna/daemon.sock`; Windows TCP + token), `hello{role:window}`,
  `call()` with id counter and futures, `events` broadcast stream, `proto()` for binary,
  reconnect with backoff, handle `show` (focus window) and `quit` (exit).
Rust:
- New commands in `dispatch.rs` (snake_case names, camelCase args, return `ytm` models via serde):
  `catalog_search{query,filter}`, `catalog_search_more{continuation}`,
  `catalog_suggestions{input}`, `catalog_album{id}`, `catalog_artist{id}`,
  `catalog_playlist{id}`, `catalog_playlist_more{continuation}`, `catalog_track{videoId}`,
  `catalog_related{videoId}`. Make sure `ytm` models derive `Serialize` with stable field names.
- Images: keep fetching through `volna://` (`proto`) so the proxy applies; Dart side: an
  `ImageProvider` that loads via `proto()` with an in-memory + disk LRU cache.
- Done when a Dart integration test (skipped unless daemon running) does search → album and
  loads one cover.

### Phase 2. Prototype: wave + player (~2–3 days)
- Rust: `color_palette{imageUri}` → dominant/accent colors computed in the daemon (crate
  `image` + simple k-means or median cut), cached per URL. Keeps Dart thin and portable.
- Dart: player state from `player_snapshot` + player events (inspect `player/forward.rs`,
  `player/hooks.rs`, `player/journal.rs` for event names and payloads; document them in
  `docs/native-player.md`).
- Screens: Wave (big cover, title/artist, like/dislike/skip, start/stop via `player_wave_*`),
  mini player bar, fullscreen player (seek, prev/next, shuffle/repeat, queue sheet with
  reorder/remove via `player_move`/`player_remove`).
- Animated background gradient from `color_palette`; cover transitions (hero + crossfade).
- Done when the user can listen to the wave for 10 minutes with the Flutter window, and
  closing/reopening the window keeps playback (daemon owns it).

### Phase 3. MVP (~1–2 weeks)
3a. Library to the daemon:
- Rust: SQLite tables (reuse the wave DB connection or a sibling DB in the same data dir):
  liked tracks/albums/artists/playlists, user playlists (+ items, order), history, pinned.
  Commands: `library_get{kind}`, `library_like{kind,item,on}`, `library_is_liked{kind,ids}`,
  `playlist_create/rename/delete/add/remove/move`, `history_add` (or record from player hooks
  in the daemon, preferred), `history_get`. Keep `wave_sync_likes` semantics: likes feed the wave.
- Legacy migration: add a one-shot export in the OLD UI (`ui/js`) that dumps all IndexedDB
  stores + relevant localStorage keys to a new daemon command `legacy_import{dump}`;
  daemon writes it into SQLite and marks `legacy_imported`. Ship this in a release before
  removing the old UI. Also accept the same JSON from a file (Settings → import) as fallback.
3b. Lyrics (`crates/lyrics`):
- Port the provider chain in the same order as `AmLyrics.fetchLyrics` + YTM fallback.
  Read `node_modules/@uimaxbai/am-lyrics/src/AmLyrics.ts` for exact endpoints, params,
  response shapes and TTML parsing; port `ui/js/lyrics.js` Genius/LRCLIB helpers.
- All requests via `crates/net` client (proxy applies). Hosts = current allowlist.
- Model: `Lyrics{source, synced: None|Line|Word, lines:[{startMs,endMs,text,words?:[{startMs,endMs,text}]}], plain?}`.
- SQLite cache keyed by videoId (+ ISRC when known); negative cache with TTL.
- Command `lyrics_get{videoId}`; optional `lyrics_sources{videoId}` to let the user switch.
- Unit tests per provider with recorded fixtures; live test `#[ignore]` like `api_live`.
- Dart: karaoke view with word highlight, auto-scroll, tap line to seek.
3c. Screens: search (suggestions, filters, infinite scroll), album, artist, playlist,
  library tabs, history, settings.
3d. Settings: network (`net_settings`, `net_apply`, `net_validate`, `net_health`,
  `net_set_cookies`, `net_audio_check`, `net_xray_logs`), wave (`wave_set_settings`,
  `wave_activities`, cold start artist picker), quality, theme (dark/light/system),
  yt-dlp version/update. UI-only prefs (theme) stored by Flutter (`shared_preferences`);
  everything else in daemon settings (`crates/core/src/settings.rs`).
3e. Scrobblers in the daemon (`crates/scrobble`), all four services:
- One `Scrobbler` trait (`now_playing`, `scrobble(track, started_at)`, `love(track, on)`),
  a fan-out like `multi-scrobbler.js`: each service independent, one failing never blocks others.
- Driven by player hooks in the daemon (≥ the configured percentage, Last.fm default 50%, or
  4 min), not by the window, so scrobbling works with the window closed.
- Offline queue in SQLite per service, retried with backoff; batch where the API allows
  (Last.fm/Libre.fm up to 50, ListenBrainz `import`).
- Last.fm: desktop auth flow (token → open browser → session), optional custom API
  key/secret. Port from `ui/js/lastfm.js`.
- Libre.fm: Last.fm-compatible 2.0 API at `https://libre.fm/2.0/`, auth via
  `https://libre.fm/api/auth/`. Port from `ui/js/librefm.js`; share the signing code with Last.fm.
- ListenBrainz: user token, custom server URL, `1/submit-listens` (`playing_now`/`single`),
  MBID lookup via `1/metadata/lookup/`, love via `1/feedback/recording-feedback`.
  Port from `ui/js/listenbrainz.js`.
- Maloja: server URL + key, `apis/mlj_1/newscrobble` with fallback to `apis/native/newscrobble`
  (as `ui/js/maloja.js`). No love.
- Love on like: per service toggle (Last.fm, Libre.fm, ListenBrainz).
- Requests go through `crates/net` like everything else.
- Commands: `scrobble_status` (all services), `scrobble_set{service, ...}` (enable, token, url,
  love on like, percentage), `scrobble_login_start{service}` / `scrobble_login_finish{service}`
  (Last.fm, Libre.fm), `scrobble_logout{service}`.
- Legacy import (3a) must carry all keys listed in "Current state → Scrobblers".
- Tests: request building/signing per service with fixtures; live tests `#[ignore]`.
- Dart: settings → Scrobbling, one card per service.
3f. Yandex import:
- Move `src-tauri/src/yandex.rs` to the daemon. Browser-cookie token path moves as is.
- Login-window fallback: Flutter opens a webview window (`desktop_webview_window` or similar;
  verify Linux support at implementation time) on `oauth.yandex.<tld>/authorize`, catches the
  `#access_token=` redirect, passes the token to the daemon (`yandex_set_token`). Last-resort
  fallback: paste token / redirect URL manually.
- Move YTM matching from `ui/js/yandex-import.js` into Rust (daemon), report progress via events,
  result goes into library tables.

### Phase 4. Replace the old window (~2–3 days)
- Daemon launches the Flutter binary. Remove `src-tauri/` (keep nothing Tauri), `ui/`,
  `ui-upstream/`, `api_emu` (`#[path]` include in daemon), `docs/hifi-api-contract.md`.
  Remove `volna://api/*` routes from `protocol.rs` that only served api_emu.
- `packaging/arch/PKGBUILD`: build Flutter linux release bundle, install to
  `/usr/lib/volna/`, bump `pkgrel`. Flutter Linux depends on GTK3 (no WebKitGTK anymore).
- Windows: update `.github/workflows/build.yml` and the local cross-build; Flutter Windows
  build must run on Windows (no cross-compile), so CI is the source of truth.
- Update `CLAUDE.md`, `docs/STRUCTURE.md`, `docs/native-player.md` §9, `PLAN.md`.
- Done when `makepkg -f` package installs and the full MVP works on Linux.

## Mobile

### Fixed decisions (mobile)

- Android: the same Flutter app (`app/`), built on Linux.
- iOS: no Mac and no macOS CI runners. Flutter cannot build iOS on Linux (the iOS
  `gen_snapshot`/App.framework toolchain exists only for macOS hosts), so iOS is a separate
  SwiftUI app built on Linux with xtool. Both apps sit on the same Rust core; the iOS UI is
  the only duplicated part, so keep it thin.
- No proxy on mobile: `crates/net` is used in Direct mode only, no xray/VLESS. If YTM is
  unavailable in the user's country, a system VPN is the user's business.
- No yt-dlp on mobile: stream resolution in Rust (phase 5).
- Native players: Media3/ExoPlayer on Android, AVPlayer on iOS. No GStreamer on mobile.
- The players play `http://127.0.0.1:<port>/…` from `streams::LoopbackServer`, never
  googlevideo directly: the resolver and the downloader must share the same exit IP, and Range
  handling stays in Rust.
- Distribution: Android APK (sideload / GitHub releases). iOS: sideload via xtool only
  (free Apple ID: signature valid 7 days; paid account: 1 year). No App Store (YouTube ToS).

### Facts verified on 2026-10-17 (iOS toolchain)

- xtool 1.22.2 (`~/.local/bin/xtool`) built a SwiftPM app on Linux, signed it with a free
  Apple ID and installed it over USB on an iPhone 12 mini (iOS 27.0.1). It launched.
- Needs in PATH: Swift 6.4 (`swiftc`) and `zip`; Darwin SDK installed by
  `xtool sdk install Xcode_27.1.xip` into `~/.swiftpm/swift-sdks/darwin.artifactbundle`.
- Rust `staticlib` for `aarch64-apple-ios` builds on Linux and links into the app via a C
  target: `Sources/CVolnaCore/include/volna_core.h` + a dummy `.c`, with
  `linkerSettings: [.unsafeFlags(["-L<dir>"]), .linkedLibrary("volna_core")]`.
- Verified both directions: Swift → Rust, and WKWebView JS → `WKScriptMessageHandler` →
  Rust → `evaluateJavaScript`. Only a dependency-free crate was tested; crates with C code
  (rusqlite `bundled`, TLS) are not yet tested for iOS.
- If the phone drops off USB, `xtool dev run` silently waits at "Waiting for device".

### Phase 5. Streams without yt-dlp (can start any time, independent of the UI)

- `crates/streams` already has the seam: trait `Backend` (`resolver.rs`). Cache, Range,
  `StreamService`, `LoopbackServer` do not depend on yt-dlp.
- Move `VideoId`, `Quality`, `ResolvedStream` out of `ytdlp.rs` into a neutral module.
  Put `ytdlp` behind a default feature `ytdlp`; add feature `rustypipe` with
  `impl Backend for RustyPipeBackend`.
- Spike first: live test `rustypipe_live` (`#[ignore]`) on ~10 YTM video ids, on desktop:
  1) which clients return a playable audio URL without PO token (check: Range request to
  googlevideo returns 206); 2) formats/bitrates, incl. m4a (itag 140) for AVPlayer;
  3) behaviour without cookies. Decide from the result.
- PO token, if needed: BotGuard run in the platform WebView (Android WebView / WKWebView),
  token handed to Rust through a callback; desktop keeps bgutil. Do not ship the
  `rustypipe-botguard` binary on mobile.
- `Quality` gets a codec preference: iOS → AAC/m4a only; Android → opus preferred.
- SABR: YouTube is moving clients to streams without a plain URL. Shape the backend API as
  "open a stream" rather than "give me a URL" so `LoopbackServer` can later assemble SABR
  (rustypipe works on it). Not a blocker now.
- Done when the live test plays tracks on desktop via `RustyPipeBackend` with the existing
  `<audio>`/GStreamer path.

### Phase 6. `volna-core`

Status (2026-10-17): done except the mobile stream backend. Builds, ABI and what is
missing: `docs/mobile-build.md`.

- New crate `crates/core`: owns `ytm` client, wave, library, lyrics, scrobblers, yandex
  matching, color palette, `streams` service + `LoopbackServer`, settings. Exposes one
  JSON command surface: `call(cmd, args_json) -> result_json`, an event callback, and
  `proto(uri) -> bytes` for images — the same commands and payloads as the daemon IPC.
- The daemon becomes a host: IPC socket, GStreamer player, MPRIS, tray, `volna://` — and
  forwards `call` into `volna-core`. Desktop behaviour must not change.
- Player control on mobile: core keeps the queue/wave logic and emits "play this URL /
  pause / seek" to a platform player through a small trait; the platform reports position,
  end-of-track and errors back. Same events as `player_snapshot`/journal so the UI is shared.
- C ABI: `volna_core_init(config_json)`, `volna_core_call(id, cmd, args)` with async reply
  via callback, `volna_core_set_event_callback`, `volna_core_free`. Header generated by
  cbindgen. Dart uses it through `dart:ffi` (or flutter_rust_bridge if it saves work);
  Swift through the C target pattern above.
- Build check: `cargo build -p volna-core --target aarch64-linux-android` (NDK) and
  `--target aarch64-apple-ios` (clang with `-isysroot` pointing at the xtool iPhoneOS SDK).
  rusqlite `bundled` and the TLS backend are the likely failures — check first.
- Mobile config: no xray, `NetMode::Direct`, data dir from the platform, no yt-dlp.

### Phase 7. Android (Flutter)

- `flutter create --platforms=android` in `app/`; `cargo-ndk` builds `libvolna_core.so`
  into `jniLibs`.
- `lib/ipc`: add `FfiTransport` implementing the transport interface; `lib/api` and screens
  unchanged. Layout pass for narrow screens (bottom nav, full-screen player as a sheet).
- Player: Kotlin plugin with Media3 `MediaSessionService` (foreground service, notification,
  headset buttons, Android Auto for free), playing from `LoopbackServer`.
- Google sign-in (for library/likes from the account): WebView login, cookies passed to core.
  Google may block embedded WebView sign-in — check early; the wave must work without login.
- Yandex import: WebView login catching `#access_token=` (same flow as desktop fallback).
- Done when the wave plays 30 minutes in the background with the screen off.

### Phase 8. iOS (SwiftUI + xtool)

- `ios/` SwiftPM project (`xtool new`), bundle id `uz.khudoberdi.volna`, links
  `libvolna_core.a`. Build/install: `cd ios && xtool dev`.
- Swift side is thin: a typed wrapper over `volna_core_call` (Codable models mirroring
  the Rust serde types), screens for wave, player, search, album, artist, playlist,
  library, lyrics, settings. Same commands as Flutter; no logic in Swift.
- Player: AVPlayer from `LoopbackServer` (`NSAllowsLocalNetworking` in Info.plist),
  `UIBackgroundModes: audio`, `MPNowPlayingInfoCenter` + `MPRemoteCommandCenter`.
- m4a only (phase 5 codec preference).
- Done when the wave plays in the background with the screen locked, controls on the lock
  screen work, and the app survives a reinstall after the 7-day signature expiry with its
  data intact (data dir in the app container is kept across `xtool` reinstalls — verify).

## Risks / check early
- Flutter Linux window under Hyprland/Wayland: verify focus on `show`, no CSD glitches.
- `desktop_webview_window` on Linux needs WebKitGTK at runtime only for the Yandex fallback;
  make it optional (feature-detect, else paste-token fallback).
- Lyrics third-party endpoints change often; keep each provider isolated and failure-tolerant
  (timeout per provider, chain continues).
- Legacy library migration must ship before the old UI is removed.
- Windows has never been run (CLAUDE.md "Открытые вопросы").
- Mobile stream resolution without yt-dlp (phase 5) decides whether mobile is viable at all;
  run its spike before investing in phases 6–8.
- iOS builds depend on xtool (one maintainer, private Apple APIs for password login).
  If it breaks, iOS stops until it is fixed; Android does not depend on it.
- C dependencies of the core cross-compiled for iOS from Linux (rusqlite, TLS) — unverified.

## Estimates
Phase 1: 1–2 days. Phase 2: 2–3 days. Phase 3: 1–2 weeks (+2–3 days for the three extra
scrobblers). Phase 4: 2–3 days.
Total ~2–3 weeks to replace the old window on Linux and Windows.
Mobile: phase 5 spike 1–2 days, full 3–5 days; phase 6: 1 week; phase 7: 1–2 weeks;
phase 8: 2–3 weeks (a second UI). Mobile total ~5–7 weeks after phase 4.
