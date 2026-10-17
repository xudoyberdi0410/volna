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

## Tests

```
cargo test -p volna-streams                                   # unit + fake googlevideo + loopback server
cargo test -p volna-streams --test live -- --ignored --nocapture
cargo test -p volna live_stream_range -- --ignored            # volna:// handler, real track
```
