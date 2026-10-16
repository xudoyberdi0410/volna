# Volna: repository structure

The spec is `PLAN.md` (in Russian). This file says where things live and lists the spots in
`ui/` that talk to TIDAL or assume TIDAL data. Phases 1 to 5 replace those spots.

Line numbers are for the commit that added this file. They drift as code changes, so search
for the quoted identifier if a number is stale.

## Layout

```
Cargo.toml            workspace; all members are registered here, so don't edit the members list
src-tauri/            Tauri v2 app, crate `volna` (lib `volna_lib`)
  src/main.rs         entry, calls volna_lib::run()
  src/lib.rs          Builder; registers the async `volna` URI scheme
  src/protocol.rs     volna:// router (Route::Api / Route::Stream), CORS, 501 placeholder
  tauri.conf.json     devUrl http://localhost:5173, frontendDist ../ui/dist, window "main"
  capabilities/       default capability (core:default)
  icons/              generated from ui/public/assets/1024.png (`tauri icon`)
crates/ytm/           InnerTube WEB_REMIX client (phase 1)
crates/net/           shared reqwest client + proxy / xray manager (phase 1, 5)
crates/streams/       yt-dlp resolution, stream cache, Range serving (phase 2)
crates/wave/          "My Wave" engine on SQLite (phase 4)
ui/                   working fork of Monochrome (Apache-2.0, see ui/NOTICE)
ui-upstream/          pristine upstream copy, read-only (commit in UPSTREAM_COMMIT)
docs/                 this file
```

### Commands

```sh
cargo build --workspace && cargo test --workspace
cd ui && bun install
bun run build                            # vite build -> ui/dist
bun run test:node                        # browser-free vitest (pure logic only)
bun run tauri dev                        # Vite on :5173 + cargo run
bun run tauri build --debug --no-bundle  # -> target/debug/volna with embedded UI
```

The `tauri` script in `ui/package.json` sets `TAURI_APP_PATH=../src-tauri`, so the CLI runs
from `ui/`. The tauri crate and `@tauri-apps/api` have to share a minor version (currently 2.12),
or the CLI refuses to build.

A plain `cargo build -p volna` binary is a dev build: it loads `http://localhost:5173`, so start
Vite first. `tauri build` turns on `custom-protocol` and embeds `ui/dist`.

### The volna:// protocol

- Rust: `src-tauri/src/protocol.rs`. `Target::parse` handles `volna://api/...` (Linux/macOS)
  and `http://volna.localhost/api/...` (Windows WebView2). Every response carries
  `Access-Control-Allow-Origin: *`, and OPTIONS returns 204. Right now every known route answers
  501 JSON `{"status":501,"error":"..."}` and unknown routes answer 404.
- JS: `ui/js/volna-url.js`, with `volnaBase('api'|'stream')` and `volnaUrl(route, path, query)`.
  Use it everywhere instead of hard-coding the scheme.
- Checked in a real webview: `fetch('volna://api/info/')` returns 501 both from
  `http://localhost:5173` (dev) and from `tauri://localhost` (built app).

### ui/ entry points

- `index.html` holds all page and modal markup. `js/app.js` is the module entry: it boots
  `HiFiClient`, `MusicAPI`, `Player`, `UIRenderer` and the router.
- `js/music-api.js`: the `MusicAPI` facade that pages call. It delegates to
  `tracksStreamerAPI` (tracks.monochrome.st) first and to `LosslessAPI` (`js/api.js`) second.
- `js/api.js`: `LosslessAPI`, the hifi-api client and normalizers. Phase 3 emulates this format.
- `js/HiFi.ts`: `HiFiClient`, which calls TIDAL directly in-browser using the TIDAL web client id.
- `js/router.js`: path routing (`/album/:id`, `/artist/:id`, `/playlist/:id`, `/mix/:id`,
  `/track/:id`, `/search/:q`, `/userplaylist/:id`, `/folder/:id`, `/library`, ...).
- `js/db.js`: IndexedDB (favorites, history, user playlists/folders). The library is local only.
- `js/player.js`, `js/audio-context.js`, `js/equalizer.js` (EQ), `js/visualizer.js` with
  `js/visualizers/*` (butterchurn and others), `js/lyrics.js`, `js/settings.js`, `js/storage.js`
  (every localStorage-backed setting), `js/themeStore.js` (applies saved custom CSS only).
- `js/notifications.js`: `showNotification()` toasts, moved out of the removed downloads module.

### Removed from upstream in phase 0

Accounts and cloud sync (better-auth, PocketBase, profiles, public playlists, community theme
store, custom DB settings), Plausible analytics, Sentry, listening parties, every kind of
download (tracks, albums, playlists, discography, queue, ffmpeg, tag writing, the Downloads
settings tab), the Apple Music provider, podcasts, Cloudflare `functions/`, `docker/`, lhci, the
auth-gate and upload Vite plugins, the PWA/service worker, and the SEO/web-deploy files.
Playlist cover "upload" now stores a local data URL.

The dead CSS for those features is still in `styles.css`. It does no harm, so it was left there.

## TIDAL coupling in ui/ (work list for phase 3)

### 1. Where API requests go

- `js/api.js:60` `fetchWithRetry(relativePath, options)` is the single choke point for
  hifi-api calls. Order of attempts:
  1. `devModeSettings` (`js/storage.js:3181`): when enabled, `GET <devUrl><relativePath>`.
     This is the cheapest hook. Point it at `volnaBase('api')` permanently and drop the rest.
  2. `HiFiClient.instance.query(relativePath)` (`js/api.js:198`), native TIDAL calls from the
     browser.
  3. `tryInstances()` (`js/api.js:109`) over `apiSettings` instances (`js/storage.js:6`,
     defaults at `js/storage.js:84,110`, `refreshInstances` at `js/storage.js:180`); TIDAL
     URLs get rewritten by `wrapTidalUrl` (`js/proxy-utils.js:11-13`, tidal-proxy.monochrome.tf).
- Endpoints requested through `fetchWithRetry` (the shapes api_emu must serve):
  `/search/` (7 call sites: tracks/artists/albums/playlists/videos/combined/ISRC),
  `/track/`, `/info/`, `/trackManifests/`, `/album/`, `/album/similar/`, `/artist/`,
  `/artist/similar/`, `/artist/bio/`, `/playlist/`, `/mix/`, `/video/`.
  Get the exact query params with `grep -n "fetchWithRetry(" ui/js/api.js`.
- Normalizers that define the expected JSON: `prepareTrack` `js/api.js:287`, `prepareAlbum`
  `:324`, `preparePlaylist` `:331`, `prepareVideo` `:335`, `prepareArtist` `:345`,
  `parseTrackLookup` `:469`, `normalizeTrackManifestResponse` `:1627`. Route handlers and
  response envelopes (`{version, data}`) are in `js/HiFi.ts:2510-2630` (`query()` switch).
- Streams: `LosslessAPI.getTrack` `js/api.js:1777`, `getStreamUrl` `:2712`, unified playback
  with a Cloudflare Turnstile captcha `:1967-2108`, `:2394`, `:2589`, Deezer fallback `:1821-1848`
  (settings `js/storage.js:3049`, UI `index.html:4507-4525`, `js/settings.js:760-772`), and
  `MusicAPI.getStreamUrl` `js/music-api.js:345-362`. That last one goes to tracks.monochrome.st
  **before** LosslessAPI. Playback consumes `{url, playbackType: 'direct'|'dash'|'hls', mimeType}`
  in `js/player.js` (DASH via shaka-player, HLS via hls.js).
- tracks.monochrome.st (a Monochrome backend, not TIDAL, but it has to go):
  `js/tracks-api.js:8`; `MusicAPI.search*` try it first (`js/music-api.js:93-185`); also
  `js/app.js:67` (contributors) and `js/ui.js:2637` (donation goal).
- Direct TIDAL calls outside api.js: `js/api.js:1709-1724` (track/mix recommendations via
  `tidal.com/v1`), and `js/HiFi.ts` as a whole (api.tidal.com, openapi.tidal.com,
  auth.tidal.com:1237). `HiFiClient.initialize` is in `js/app.js:334-350`.

### 2. Cover and image URLs (resources.tidal.com, `uuid` with `-` replaced by `/`)

- URL builders: `js/api.js:2816` `getCoverUrl`, `:2829` `getCoverSrcset`, `:2842`
  `getArtistPictureUrl`, `:2855` `getArtistPictureSrcset`, `:2865` `getVideoCoverUrl`.
  Wrappers that already pass through `http(s):`, `blob:`, `data:` and `assets/`:
  `js/music-api.js:373-414`. The easiest fix is to make api_emu return full image URLs
  (proxied through `volna://api/image?...`) so the wrappers pass them through.
- Hard-coded resources.tidal.com plus wsrv.nl for editor's picks: `js/ui.js:732-734`.
- Cover slugs built inside HiFi.ts: `js/HiFi.ts:1805-1809`, `:2018-2024`.
- Call sites of the builders (they pass `album.cover`, `artist.picture`, `track.album.cover`,
  `video.imageId`): `js/ui.js:442,449,450,546,582,589,715,801,882,886,979,1437,1458,1490,1510,
  5298,5333,5812,5961,5964,6132,6162,6507,6516,7553,7587`, `js/player.js:465,466,1280,1281,3091`,
  `js/commandPalette.js:783,798,812`, `js/ui-interactions.js:284`, `js/utils.js:759,770`
  (`getCoverBlob` uses `coverArtSizeSettings` and corsproxy.io at `:748`).

### 3. Numeric-id assumptions

- `js/db.js:567-576`: library import runs `parseInt` on string ids that look numeric. YouTube
  ids are not numeric, so they pass through, but numeric-looking playlist/browse ids would get
  mangled.
- `js/music-api.js:348,485` and `js/tracks-api.js:834`: `/^\d{17,20}$/` means "tracks id".
- `js/api.js:1305` (artist id compare via `Number`), `:1658,:1676` (`trackId: Number(raw.id)`),
  `:1812` (`Number(id)`).
- `js/HiFi.ts:2528-2617`: every route handler does `Number(qp.id)`, and the normalizers at
  `:1610-2175` do `Number(x.id)`.
- `js/router.js` `extractProviderAndId`: `/track/t/<id>` means TIDAL, `tracks/` and `mono/`
  mean tracks.monochrome.st. `js/music-api.js` `stripProviderPrefix` / `getProviderFromId`
  (`t:`, `q:`, `tracks:`, `mono:` prefixes).

### 4. Other TIDAL-shaped data

- Mixes: `item.mixes.TRACK_MIX` / `ARTIST_MIX` drive "track mix", "artist mix" and radio
  buttons: `js/events.js:1400,1465,1471,1533,2092`, `js/ui-interactions.js:366`,
  `js/ui.js:5532,6467`, stored in `js/db.js:322`. YTM equivalent: `RDAMVM<videoId>` radio
  playlists, so api_emu can fill `mixes.TRACK_MIX` with that id and serve `/mix/`.
- Playlists are addressed by `uuid`: `js/events.js:1381,1427,1474,1554,1819,1833,2059,2080,2086`,
  `js/ui.js:792,797,798,2756,2759,4056,4063,4606,4614,5058,5061,5077,5080,5933,5936,6006,7197,
  7216`, `js/db.js:228,378,406,585`, `js/playlist-generator.js:240,319`.
- Quality model (HI_RES_LOSSLESS / LOSSLESS / HIGH / LOW, Dolby Atmos): `js/utils.js:43-53`,
  the streaming-quality select in settings (`js/settings.js:777`, `streaming-quality-setting`),
  `js/player.js` quality fallback and the Deezer hi-res warning (`:927-934`, `:1535-1542`,
  `:2883`). YTM serves opus/AAC only, so this UI needs simplifying.
- ISRC matching (TIDAL/Deezer/tracks lookups): `js/api.js` (9 places), `js/playlist-importer.js`
  (13), `js/lyrics.js` (2), `js/tracks-api.js` (4).
- Paste-a-link detection for `tidal.com/` and `monochrome.tf/` URLs: `js/app.js:2081-2083`.
  Add `music.youtube.com` here.
- MusicBrainz/Harmony links built from TIDAL album ids: `js/events.js:1838`,
  `js/api.js:1181-1194`.

### 5. Other third-party calls made directly from the webview

These break the "everything goes through Rust and the proxy" rule in PLAN.md. They should be
proxied via `volna://api` or removed in later phases.

- Home "Hot & New" and genre explore: `js/ui.js:2962,4095` (hot.monochrome.tf). AOTY tab:
  `js/ui.js:104` (aoty.edideaur.works). Editor's picks: `public/editors-picks*.json`, which
  hold TIDAL ids (`js/ui.js:4322`).
- Artist banner: `js/music-api.js:424` (samidy workers). Community playlists:
  `js/community-playlists.js:1` (panora-api). YouTube Music playlist import:
  `js/app.js:1161` (ytmimport.samidy.workers.dev). Our own `ytm` crate can replace that.
- Lyrics: LRCLIB `js/lyrics.js:491`, Genius through allorigins `js/lyrics.js:104-140`,
  kuroshiro dictionaries from jsdelivr `js/lyrics.js:310-340`, plus the `am-lyrics` component.
- Unreleased tracker (ArtistGrid sheets): `js/tracker.js:87-193,737`.
- Scrobblers (Last.fm, Libre.fm, ListenBrainz, Maloja): `js/lastfm.js`, `js/librefm.js`,
  `js/listenbrainz.js`, `js/maloja.js`. These are fine to keep but go direct.
- Google Fonts: `index.html` head, `js/storage.js:2727`, `js/themeStore.js:138`. Bundle the
  fonts locally to avoid Google from the webview.
- AutoEQ data: `js/autoeq-importer.js:128,238,239` (GitHub, jsdelivr).
- Branding left in place: "Monochrome" texts, Discord/GitHub/Donate sidebar links, the About
  page, `js/utils.js:849` share URLs (`monochrome.tf`).
