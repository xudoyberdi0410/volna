# Playback path (phase 2)

> Since the native player (`docs/native-player.md`), the consumer of the loopback server is
> a GStreamer `playbin3` deck in Rust (`crates/player`), not `<audio>`; the URL format and the
> resolve/Range/re-resolve behaviour below are unchanged.

## What `<audio>` must use

WebKitGTK (checked on 2.54.1) plays media through GStreamer, and its source element only
accepts `http(s)` and `blob` URIs. `<audio src="volna://stream/<id>">` fails with
`MEDIA_ERR_SRC_NOT_SUPPORTED` before any request is sent, while `fetch()` to the same URL
works. Audio therefore comes from a loopback HTTP server inside the app:

```
http://127.0.0.1:<random port>/<random token>/stream/<videoId>[?q=low]
```

The page gets the prefix as `window.__VOLNA__.streamBase`, set by an init script before any
page script runs. The `stream_base` command returns the same value. For patch F2 in
`hifi-api-contract.md`, use:

```js
url: `${window.__VOLNA__.streamBase}/${encodeURIComponent(id)}${low ? '?q=low' : ''}`
```

and not `volnaUrl('stream', …)`. This was checked in the real webview from both
`http://localhost:5173` (dev) and `tauri://localhost` (built app), with
`crossOrigin = 'anonymous'`:

- playback starts about 0.3 s after the resolve;
- seeking to 150 s works;
- an `AnalyserNode` on `createMediaElementSource` gets non-zero data, so CORS does not taint
  the element and the EQ and visualizers keep working.

The server answers `GET`/`HEAD`/`OPTIONS`:

- `200` with the whole track when there is no `Range`, `206` with `Content-Range` when there is
  one, and `416` with `bytes */<total>` past the end;
- `Access-Control-Allow-Origin: *` and `Accept-Ranges: bytes`;
- the body is streamed from googlevideo in 8 MiB upstream windows and never buffered whole.

A URL that expires in the middle of the body is re-resolved once, transparently. The server
binds to loopback only, and the token stops other local programs from using it as a proxy.

`volna://stream/<id>` is still available for `fetch` use: a capped answer of up to 2 MiB,
always `206`.

## Covers

`volna://img/?u=<urlencoded https URL>[&s=<px>]` (Windows:
`http://volna.localhost/img/?u=…`) only accepts `*.googleusercontent.com`, `*.ytimg.com` and
`*.ggpht.com`.

- It rewrites `=w…-h…` size suffixes on lh3/ggpht URLs to `s`.
- It caches covers forever in `<app cache dir>/images/`, sniffs the content type from the
  bytes, and answers with `Cache-Control: immutable` and CORS headers.

`<img>` loads custom schemes fine; only media elements have the problem above. The
contract's `volna://api/image/` route can delegate to `AppState::images`.

## Resolving

- yt-dlp is looked up as `$VOLNA_YTDLP`, then a sidecar next to the executable
  (`volna-yt-dlp` once installed, or `volna-yt-dlp-<triple>`/`yt-dlp-<triple>`), then `PATH`.
- It runs as `--ignore-config --no-playlist -J -f 'bestaudio[acodec=opus]/bestaudio[ext=m4a]/bestaudio'`,
  plus `--proxy <NetManager::proxy_url>` when not in direct mode.
- Results are cached per (videoId, quality) until `expire − 5 min`. The cache also tracks the
  proxy, because googlevideo URLs are bound to the exit IP: changing the network setting makes
  the next play resolve again.
- Concurrent requests for the same track share one yt-dlp run. A 403 or 410 from googlevideo
  invalidates the cached URL and resolves once more.
- On this machine a resolve takes about 3 s, and the first 64 KiB arrive about 0.4 s later.

Commands:

| Command | What it does |
|---|---|
| `stream_prefetch({videoId, quality?})` | Resolves the next track in the background. |
| `ytdlp_version()` | Returns `{binary, origin, version}`. |
| `ytdlp_update()` | Runs `yt-dlp -U` through the current proxy for a sidecar or configured binary. For a system (PATH) binary it only reports the version. |

## Backends

`Backend` (`resolver.rs`) turns `(videoId, Quality, proxy)` into a `ResolvedStream`. The
cache, Range handling, `StreamService` and `LoopbackServer` do not know which backend runs.

- Feature `ytdlp` (default): `YtDlp`, the sidecar described above.
- `RustyPipeBackend` in `crates/streams-rustypipe`: in-process InnerTube player requests
  through [RustyPipe](https://codeberg.org/ThetaDev/rustypipe), no binary and no PO token.
  It uses the Codeberg branch `feat/deobf-extractor` (pinned rev in its `Cargo.toml`),
  because the crates.io release 0.11.4 can no longer deobfuscate the current player. This
  crate is not part of the workspace (see "License" below).

Any other `Backend` (a future resolver of our own, for example) plugs into
`StreamService::new` the same way; nothing in the crate names a particular backend.

`Quality` is `{tier, codec}`. `Codec::PreferOpus` takes Opus and falls back to AAC;
`Codec::AacOnly` takes m4a only (itag 140/139), which is what AVPlayer on iOS needs. The
loopback URL takes `?codec=aac`. `Quality::High`/`Low` and their JSON form `"high"`/`"low"`
are unchanged.

`ResolvedStream::delivery` says how bytes are fetched. Only `Delivery::Url` exists. SABR
(streams without a byte-addressable URL) will be another variant, assembled inside
`StreamService::request`, so the loopback server and its callers stay as they are.

### `crates/streams-rustypipe`

The crate is listed in `[workspace] exclude` and has its own `[workspace]` table and
`Cargo.lock`. The app's `Cargo.lock` references no RustyPipe, no Codeberg and none of its
dependencies; workspace builds, CI and the Arch package never fetch it. The app cannot
select it: the desktop always resolves through yt-dlp. Build and test it on its own:

```sh
cargo clippy --manifest-path crates/streams-rustypipe/Cargo.toml --all-targets -- -D warnings
```

RustyPipe brings reqwest 0.13 (the app uses 0.12), so it gets its own client. That client
uses the proxy URL `NetManager` gives yt-dlp, which is the same exit. One RustyPipe instance
is kept per proxy URL.

Fetching it: Codeberg was slow from here, and the clone only went through the proxy with
`CARGO_NET_GIT_FETCH_WITH_CLI=true ALL_PROXY=socks5h://127.0.0.1:1081`.

Other costs, if it were ever used: reqwest 0.13 with `rustls` pulls in `aws-lc-sys`, which
needs cmake and a C toolchain (an extra hurdle for Android NDK and iOS cross builds). The
deobfuscator adds oxc and QuickJS (`rquickjs`).

### License

RustyPipe is GPL-3.0, and so is `crates/streams-rustypipe` (its `Cargo.toml` says so).
Volna is Apache-2.0. Linking the crate into the app would make the distributed binary
GPL-3.0. It is therefore never shipped: it stays outside the workspace, no app crate depends
on it, and it is used only to re-run the spike below.

## Phase 5 spike: RustyPipe without yt-dlp (2026-10-17)

`crates/streams-rustypipe/tests/rustypipe_live.rs`, run on the dev machine:

```sh
VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081 \
  cargo test --manifest-path crates/streams-rustypipe/Cargo.toml \
  --test rustypipe_live -- --ignored --nocapture --test-threads=1
```

`rustypipe_live_clients` asks every client for the player of ten YTM tracks (four official
videos and six "Topic" art tracks, several of them region-licensed). It then requests
`bytes=0-65535` of the best Opus and of itag 140 from googlevideo, from the same exit. A
cell counts only if the answer is 206 with 65536 bytes. `rustypipe_live_plays_through_loopback`
runs the app path (`NetManager` → `StreamService<RustyPipeBackend>` → `LoopbackServer`), in
Opus and in AAC, and seeks to the middle.

No botguard and no cookies were used. RustyPipe only applies cookies to the web clients, and
those fail before the player request (see below).

Exit via proxy (46.62.225.48, a datacenter VPS, the app's usual route):

| Client | Result on 10 tracks | Notes |
|---|---|---|
| Desktop, DesktopMusic, Mobile (web) | 0 | `deobfuscation error: could not extract deobf data` on today's player JS, even on the June 2026 branch. They also need a PO token. |
| Tv | 0 | Same deobfuscation failure. |
| Android | 0 | Once returned formats 599/600/139/249/140/251, all without a URL (SABR only). Otherwise "Sign in to confirm you're not a bot" (`ip-ban`) or HTTP 403 on the player request. |
| AndroidVr | 1 (first request only) | `dQw4w9WgXcQ`: plain URLs for 139, 249, 140 (130 kbps AAC), 251 (136 kbps Opus). Both 251 and 140 answered 206 with real bytes, URLs valid for 6 h, and the loopback test played it in AAC. Every later request got `ip-ban` or 403, also when paced at 6 s. |
| Ios | 0 | `ip-ban` or HTTP 403 on the player request. |

Direct exit (188.113.198.7, home ISP): 0 for every client. YouTube Music is not offered in this
country. RustyPipe's visitor-data request to music.youtube.com gets the "not available in your
area" page, then 302, so no player request is made.

For comparison, yt-dlp 2026.08.19 on the same proxy without cookies also gets "Sign in to confirm
you're not a bot". With the Chrome cookies, Deno and the bgutil PO token provider it resolves
format 251 (the setup in `CLAUDE.md`). The datacenter exit is flagged for anonymous requests.
Only a logged-in web client gets through, and that needs a JS challenge solver and a PO token.

What the spike shows:

1. Without a PO token, only AndroidVr returned a plain URL that googlevideo served (206). The
   web and TV clients depend on RustyPipe's deobfuscator, which does not handle the current
   player. Android is already SABR-only.
2. When a client works, both Opus 251 and m4a 140 (AAC 130 kbps) are there, so `AacOnly` is
   enough for AVPlayer.
3. Without cookies, a flagged IP gets the bot check from every client, with RustyPipe and with
   yt-dlp alike. This could not be tested from a clean residential IP in a YTM country.

Verdict for mobile: not viable today with RustyPipe as the only resolver. One client
(AndroidVr) works without a PO token, and only from an unflagged IP. The web/TV path is broken
upstream; RustyPipe's last release is from April 2025 and the working code sits on an
unreleased branch. Then there is the GPL. A phone on a residential or mobile IP in a country
where YTM is available may do much better than this VPS, but that is unverified. Before
building phase 7/8 on this backend, run the same test on a phone's network (hotspot +
`VOLNA_TEST_PROXY` unset) from a supported country. If it stays like this, mobile needs the
logged-in web client path that works for yt-dlp: cookies or OAuth, a PO token from BotGuard
in the platform WebView (as the plan says), and a JS challenge solver (the WebView's JS engine
can run the player's `n`/signature code). That is a resolver of our own, not RustyPipe.

## Tests

```
cargo test -p volna-streams                                   # unit + fake googlevideo + loopback server
cargo test -p volna-streams --test live -- --ignored --nocapture
cargo test -p volna live_stream_range -- --ignored            # volna:// handler, real track
cargo test -p volna-streams --no-default-features              # builds without yt-dlp
VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081 cargo test \
  --manifest-path crates/streams-rustypipe/Cargo.toml \
  --test rustypipe_live -- --ignored --nocapture --test-threads=1
```

`rustypipe_live` also reads `VOLNA_TEST_CLIENTS=AndroidVr,Ios` (narrow the matrix),
`VOLNA_TEST_PAUSE_MS` (pause between player requests, default 1500) and
`VOLNA_TEST_COOKIES_TXT` (a Netscape cookies.txt for a logged-in pass).
