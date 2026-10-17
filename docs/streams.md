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
- Feature `innertube`: `InnerTube`, our own resolver for mobile (see "InnerTube backend"
  below). No binary, no Python; JavaScript runs in a host-provided `JsRuntime`.
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

## InnerTube backend (feature `innertube`)

`volna_streams::innertube::InnerTube` resolves the way yt-dlp's `web_music` client does
(yt-dlp 2026.08.19 is the reference), with no yt-dlp and no Python. All HTTP is Rust, on
the exit the stream is fetched from (the `proxy` the `Backend` gets). Only JavaScript runs
elsewhere, through `JsRuntime`.

1. `GET https://music.youtube.com/` with the session's cookies; every `ytcfg.set({...})` is
   merged. It gives `INNERTUBE_CONTEXT` (pinned to `hl=en`, UTC), client version, visitor
   data, `DATASYNC_ID`, `SESSION_INDEX`, `LOGGED_IN`, the player URL and the
   `html5_generate_content_po_token` experiment. Cached per exit for an hour. Without a
   page it falls back to the built-in `WEB_REMIX` context and `iframe_api` for the player.
2. `base.js` of that player, always the `main` variant
   (`/s/player/<id>/player_ias.vflset/en_US/base.js`, as yt-dlp forces), kept in memory
   (two versions) and in `InnerTubeOptions::cache_dir`. `signatureTimestamp` comes from the
   page's `STS` or the player.
3. `POST music.youtube.com/youtubei/v1/player` with `X-YouTube-Client-Name: 67`, version,
   `X-Goog-Visitor-Id`, `Cookie`, `Authorization: SAPISIDHASH … SAPISID1PHASH …
   SAPISID3PHASH …` (with `_u` when the user session id is known), `X-Goog-AuthUser`,
   `X-Origin`. A 400/403/429 refreshes the ytcfg and retries twice (0.5 s, 3 s); then it is
   reported as `BotCheck` ("this exit is rate limited").
4. Format choice: audio-only, no DRC, default audio track; `PreferOpus` takes the
   best/worst Opus (251/250/249) or falls back to AAC; `AacOnly` takes 140/139. Formats
   without a URL or cipher are SABR-only and give `NoAudioStream`.
5. `s` (from `signatureCipher`, written back under `sp`) and `n` are solved by the runtime.
   As in yt-dlp, a signature is solved once per length (the challenge is the chars
   `0..len`, the answer is a permutation); `n` answers are cached per challenge.
6. GVS PO token: bound to the video id under the content-binding experiment (on now), else
   to `DATASYNC_ID` when logged in, else to the visitor data, as yt-dlp's
   `get_webpo_content_binding`. BotGuard as in BgUtils: Rust posts `[requestKey]` to
   `www.youtube.com/api/jnn/v1/Create` (descrambling the base64 answer, bytes + 97), the
   runtime runs the VM and takes a snapshot, Rust trades it at `GenerateIT` for an
   integrity token (TTL 12 h here), the runtime mints `pot` with it. One attestation per exit,
   refreshed 10 min before expiry or when the runtime has lost the session. If minting fails
   the URL is returned without `pot` (logged), like yt-dlp.

`ResolvedStream::user_agent` is the Chrome UA the requests used; googlevideo is fetched with
it. Pre-roll ads (`adPlacements`) are counted; `InnerTubeOptions::wait_for_ads` would sleep
them out as yt-dlp's downloader does, off by default (no 403 seen without waiting).

### YouTube session on mobile

The apps sign in with Google in a WebView and hand its cookies to the core:
`youtube_set_cookies {cookies}` (Netscape cookies.txt text; `""` signs out) and
`youtube_auth_status`, both answering `{signedIn, account?}` (`account` is the display
name from YTM's `account/account_menu`, looked up only when signed in). The core saves the
text as `<dataDir>/youtube-cookies.txt` with mode 0600, loads it at init (the optional
`youtubeCookies` init field replaces it), and swaps it into the running `InnerTube`
backend: the next resolve fetches a fresh ytcfg with the new session, and cached stream
URLs are dropped. `signedIn` means `LOGIN_INFO` plus one of the SAPISID cookies, as yt-dlp
checks.

What the WebView must export:

- After the Google sign-in finishes, load `https://music.youtube.com/` once in the same
  WebView. Google sets its session on `.google.com`; YouTube gets its own copy on
  `.youtube.com` only through that redirect.
- Export the cookies of `.youtube.com` (and host-only `music.youtube.com` /
  `www.youtube.com`). The resolver sends only cookies that match `music.youtube.com` and
  `www.youtube.com`; `.google.com` cookies (`SID`, `__Secure-1PSID`, `NID`, ...) may be
  included but are never sent. Export every cookie of those domains rather than a list:
  YouTube adds and rotates names.
- The ones that matter, all on `.youtube.com`:
  - `SAPISID`, `__Secure-1PAPISID`, `__Secure-3PAPISID`: the SAPISIDHASH /
    SAPISID1PHASH / SAPISID3PHASH `Authorization` header is built from them (yt-dlp falls
    back to `__Secure-3PAPISID` when `SAPISID` is missing, and so does Volna);
  - `LOGIN_INFO`: required for `signedIn`;
  - `SID`, `HSID`, `SSID`, `APISID`, `__Secure-1PSID`, `__Secure-3PSID`: the session itself;
  - `__Secure-1PSIDTS`, `__Secure-3PSIDTS`, `SIDCC`, `__Secure-1PSIDCC`,
    `__Secure-3PSIDCC`: short-lived and rotated by Google; re-export after the WebView has
    been used, or the session may stop working;
  - `VISITOR_INFO1_LIVE`, `VISITOR_PRIVACY_METADATA`, `PREF`, `YSC`: visitor state, harmless.
- Netscape lines: `domain<TAB>includeSubdomains<TAB>path<TAB>secure<TAB>expiry<TAB>name<TAB>value`,
  with `TRUE` for a domain cookie (leading dot) and `0` expiry for session cookies. The
  cookie stores give these fields (`WKHTTPCookieStore.getAllCookies`; on Android
  `CookieManager.getCookie(url)` returns only `name=value`, so write each pair with the
  URL's domain, `.youtube.com`, and expiry `0`).

### JsRuntime contract

The trait is `volna_streams::js::JsRuntime` (built without any feature);
`volna_core::js` re-exports it, and its `CallbackJs` (the `volna_core_set_js_callback` /
`volna_core_js_result` pair, see `docs/mobile-build.md`) implements it. One request is one
`evaluateJavaScript` round trip:

```rust
pub trait JsRuntime: Send + Sync {
    /// request: a JsRequest as JSON. Evaluate `volnaJs(request)` (PRELUDE loaded) and
    /// return the string the promise resolves to. Err only for transport failures.
    fn call(&self, request: String) -> Pin<Box<dyn Future<Output = Result<String, JsError>> + Send + '_>>;
}
```

The host loads `volna_streams::js::PRELUDE` (over FFI: `volna_core_js_prelude()`) once per
JS context (yt-dlp's EJS solver plus `js/volna.js`; it defines `globalThis.volnaJs`), then
for each request
evaluates `volnaJs(<request JSON>)` and hands back the resolved string unchanged.
`JsRequest::to_script()` builds that expression. `volnaJs` never rejects and makes no
network requests. The context must persist between calls (it keeps the preprocessed player
and the BotGuard VM); if it is lost, the scripts answer `need_player` / `unknown_session`
and Rust resends the player or attests again by itself. BotGuard fingerprints the DOM, so
the context needs a real document: a WebView page (Android `WebView`, `WKWebView`) is fine;
Deno uses jsdom.

- WKWebView: `callAsyncJavaScript("return await volnaJs(r)", arguments: ["r": json])`.
- Android: `evaluateJavascript` does not await promises; evaluate
  `volnaJs(<json>).then(s => VolnaBridge.reply(<id>, s))` with a `@JavascriptInterface`.

Every answer is a JSON string: `{"ok": <result>}` or
`{"error": {"code": "...", "message": "..."}}`. Codes: `need_player`, `unknown_session`,
`solver`, `botguard`, `no_solver`, `bad_request`, `js_error`; Rust adds `runtime`
(transport) and `bad_response`.

| Request | `ok` result |
|---|---|
| `{"op":"solve","player_id":"0229cd64","player":"<base.js, only when needed>","n":["dl0R5lucCIx30g"],"sig":["\u0000\u0001…"]}` | `{"n":{"dl0R5lucCIx30g":"jhWOwsSWQ6pa"},"sig":{"\u0000\u0001…":"*fedcba…"}}` |
| `{"op":"botguard_snapshot","session":"bg1","interpreter_js":"…","program":"…","global_name":"trayride"}` | `{"botguard_response":"$20Q5RCBR…"}` |
| `{"op":"botguard_mint","session":"bg1","integrity_token":"<base64>","content_bindings":["dQw4w9WgXcQ"]}` | `{"po_tokens":["MlqWVZKJ…"]}` (base64url) |
| `{"op":"version"}` | `{"version":1}` |

`player` is sent only the first time per player id (or after `need_player`): it is 3 MB,
and the preprocessed player the solver derives from it is 4 MB, so the runtime keeps it
instead of round-tripping it. A first solve takes ~0.6 s in Deno, later ones ~30 ms.

`JsCallback(|request| async { … })` turns a closure into a runtime.
`DenoRuntime` (feature `innertube-deno`, dev and tests only) runs `js/deno_host.js` in one
long-lived `deno run` (found as `$VOLNA_DENO`, the `volna-deno` sidecar, or `deno` on
`PATH`), JSON lines over stdio, `--allow-env` only; jsdom (`npm:jsdom@26.1.0`) is
fetched by Deno into `DENO_DIR` on first use.

### Vendored JavaScript and licenses

`crates/streams/js/`:

| File | Source | License |
|---|---|---|
| `ejs/yt.solver.lib.min.js`, `ejs/yt.solver.core.min.js` | yt-dlp-ejs 0.8.0 (`yt_dlp_ejs/yt/solver/`), byte-identical | Unlicense (`ejs/LICENSE`); the lib bundles meriyah 6.1.4 (ISC) and astring 1.9.0 (MIT), notices in its header |
| `volna.js` | ours; the BotGuard part follows BgUtils 4.0.3 by LuanRT | Apache-2.0 and MIT (`LICENSE-bgutils`) |
| `deno_host.js` | ours | Apache-2.0 |

Nothing from bgutil-ytdlp-pot-provider is used: it is GPL-3.0 (both the TypeScript
server and the Rust `bgutil-pot` installed here). jsdom (MIT) is only loaded by the Deno
host at run time, never shipped.

### Live results (2026-10-17)

`innertube_live` through `socks5h://127.0.0.1:1081` (exit 46.62.225.48, the datacenter VPS;
googlevideo URLs carry the exit's IPv6 `2a01:4f9:c010:b658::1`), Chrome cookies exported with
yt-dlp, the RustyPipe spike's ten tracks, each in Opus and AAC. A cell counts when
googlevideo answers 206 with 64 KiB both at byte 0 and at the middle of the file (past the
first MiBs that are served without a PO token).

| Pass | Played | Notes |
|---|---|---|
| Cookies + PO token | 20/20 | 251 (124–137 kbps) and 140 (130 kbps) for every track. Resolve ~0.4–0.5 s warm; ~3 s for the first track (ytcfg, 3 MB player, Deno start, attestation). About one player request in three got Google's "Sorry..." 403 page and needed the retry (+8–11 s). |
| No cookies + PO token | 2/20 | "Sign in to confirm you're not a bot" for the rest, the same as yt-dlp and RustyPipe from this exit. |
| Cookies, no PO token | 18/20 | Also served past the middle: this account gets streams without `pot` (yt-dlp skips the GVS token for Premium accounts; the account is likely Premium). The 2 misses were rate-limit 403s. |

`innertube_live_plays_through_loopback` (NetManager → `StreamService<InnerTube>` →
`LoopbackServer`, start + seek to the middle): 19/20, WebM starts with the EBML magic, MP4
with `ftyp`. The one miss (Queen, Opus) was the rate-limit 403 after both retries.

BotGuard attestation usually takes 0.5–1.6 s (`Create` ~0.4 s, snapshot 30 ms–1.1 s, `GenerateIT`).
Twice the first snapshot of a test process took 43 s and 77 s; the same snapshot sent to the
host by hand takes 0.03 s, and other agents' builds were running at the time, so CPU
starvation of the VM is the likely cause (not proven).

### Verdict for mobile

Viable with a logged-in session: the whole yt-dlp path (web_music, n/sig, BotGuard PO token)
works in-process with all JavaScript behind a WebView-shaped `JsRuntime`, and plays both
Opus and AAC. Unverified, and what decides it:

- The WebView itself: only Deno+jsdom ran here. BotGuard should accept a real WebView more
  readily than jsdom, but Android WebView / WKWebView were not tried.
- Anonymous use from a datacenter IP fails (bot check). A phone on a residential/mobile IP
  in a YTM country has not been measured; plan for a Google sign-in (cookies from a login
  WebView, `InnerTube::set_cookies`).
- The PO token path was not proven necessary for this (likely Premium) account; a
  non-Premium account is the case it exists for and has not been measured.
- Google's rate limiting on the exit IP shows up as 403 "Sorry..." pages; on a phone's own
  IP this should be rarer.
- Upstream churn: the EJS solver must follow player changes (update the vendored files
  from yt-dlp-ejs releases); SABR-only responses would need a new `Delivery`.

## Tests

```
cargo test -p volna-streams                                   # unit + fake googlevideo + loopback server
cargo test -p volna-streams --test live -- --ignored --nocapture
cargo test -p volna live_stream_range -- --ignored            # volna:// handler, real track
cargo test -p volna-streams --no-default-features              # builds without yt-dlp
cargo test -p volna-streams --no-default-features --features innertube
VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081 VOLNA_TEST_COOKIES=chrome \
  cargo test -p volna-streams --features innertube-deno --test innertube_live \
  -- --ignored --nocapture --test-threads=1
VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081 cargo test \
  --manifest-path crates/streams-rustypipe/Cargo.toml \
  --test rustypipe_live -- --ignored --nocapture --test-threads=1
```

`rustypipe_live` also reads `VOLNA_TEST_CLIENTS=AndroidVr,Ios` (narrow the matrix),
`VOLNA_TEST_PAUSE_MS` (pause between player requests, default 1500) and
`VOLNA_TEST_COOKIES_TXT` (a Netscape cookies.txt for a logged-in pass).

`innertube_live` reads `VOLNA_TEST_COOKIES=<browser>` or `VOLNA_TEST_COOKIES_TXT`,
`VOLNA_TEST_PAUSE_MS` (default 1000) and `VOLNA_TEST_MAIN_ONLY=1` (skip the anonymous and
no-PO-token passes); `RUST_LOG` overrides its `volna_streams=debug` log.
