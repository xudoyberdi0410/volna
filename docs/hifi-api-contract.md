# hifi-api contract expected by the Monochrome UI

This document is the spec for two pieces of phase 3:

- **api_emu** (Rust, `volna://api/...`): answers the endpoints below with JSON in the shapes
  below, filled from the `ytm` crate.
- **Frontend wiring** (`ui/`): the patches in [section 10](#10-frontend-patches) that point the UI
  at `volna://api`, make it play `volna://stream/<videoId>`, and stop it from assuming TIDAL ids
  and TIDAL cover UUIDs.

Everything here was reverse-engineered from the code in `ui/js` at commit `2b67a32`: the
`LosslessAPI` normalizers in `api.js`, the typed TIDAL shapes and the `query()` router in `HiFi.ts`
(which show what a real hifi-api server returns), `music-api.js`, `player.js`, `ui.js`,
`events.js`, `db.js`, `storage.js`. Line numbers refer to that commit. If a number is stale,
search for the quoted identifier.

Conventions in the tables: **R** means required (the UI breaks or shows garbage without it).
**O** means optional (the UI reads it if it is there). **—** means "leave it out" (the UI does
something unwanted if it is present).

---

## 0. Decisions in one screen

1. **Every id is a JSON string, never a number.** `videoId`, `MPREb_…`, `UC…`, playlistId and
   radio ids all go into `id` / `uuid` unchanged. The UI compares ids with `===` in many places
   (`player.currentTrack.id === track.id`, `ui.js:623`), so numbers and strings must not mix.
2. **Envelope**: copy the real hifi-api shapes exactly, including which endpoints wrap the payload
   in `data` and which don't (see the per-endpoint sections). Always add `"version": "2.7"`.
3. **Images are absolute URLs.** api_emu never emits TIDAL-style UUIDs. Every `cover`,
   `picture`, `squareImage`, `image` and mix `images.*.url` is a full URL that points back
   into Rust, e.g. `volna://api/image/?u=<urlencoded source>&s=544` (on Windows it is
   `http://volna.localhost/api/image/?u=…`). The frontend has to pass these through untouched
   (patch F3).
4. **Playback doesn't use the hifi-api manifest path.** `MusicAPI.getStreamUrl` is replaced (patch
   F2) so it returns `{ url: volnaUrl('stream', '/<videoId>'), playbackType: 'direct', … }` with
   no API round trip. `<audio>` then plays `volna://stream/<videoId>` natively. The Rust stream
   route resolves through yt-dlp when the first Range request arrives. `/track/` and
   `/trackManifests/` are documented in §4.4 for completeness, but they aren't on the hot path.
5. **No TIDAL "videos" in v1.** Every YouTube videoId, including OMV and UGC music videos, is
   emitted as a `type: "track"` audio item. The `videos` search buckets are always empty, and
   `/video/` can stay a 404.
6. **Radio = mix.** Every track carries `mixes.TRACK_MIX = "RDAMVM<videoId>"`, and every artist
   carries `mixes.ARTIST_MIX = <the artist page's radio playlistId>`. `/mix/?id=RD…` is served
   from InnerTube `next`. This is what turns on the UI's "Track mix", "Artist mix" and
   "Start mix" buttons.
7. **Recommendations go through api_emu.** `getTrackRecommendations` currently calls
   `tidal.com/v1` straight from the webview (`api.js:1704`). It is re-pointed at
   `/recommendations/?id=<videoId>`, which is an existing hifi-api route (`HiFi.ts:2533`) (patch F6).

---

## 1. Transport

### 1.1 How the UI makes a request

All hifi-api traffic goes through `LosslessAPI.fetchWithRetry(relativePath, options)` (`api.js:60`).
`relativePath` always starts with the endpoint and keeps the trailing slash, e.g.
`/search/?s=daft%20punk`, `/album/?id=MPREb_abc`, `/artist/similar/?id=UCxyz`. After patch F1
it becomes exactly:

```
GET volnaUrl('api', relativePath)      // volna://api/search/?s=...   (Linux)
                                       // http://volna.localhost/api/search/?s=...  (Windows)
```

- Method: always `GET`. No body and no custom headers.
- Ids are interpolated **without** `encodeURIComponent` (`api.js:846,869,1015,1135,1231,1439,1466,
  1492,1687`). YouTube ids only use `[A-Za-z0-9_-]`, so that is safe. Search terms are encoded.
- Rust has to accept the path both with and without the trailing slash (`/album/` and `/album`),
  the way `HiFi.ts:2514` strips it.
- Any non-2xx status throws in JS. Almost every caller catches the error and renders an empty
  section (search, similar, bio, recommendations). Album, playlist, mix, artist and track pages
  show "Could not load …". Use **404** for unknown ids, **502** for upstream InnerTube failures
  and **200** for an empty-but-valid result. Errors use the existing protocol shape:
  `{"status":404,"error":"..."}`.
- `Content-Type: application/json`. CORS headers are already added by `protocol.rs`.
- The JS side caches results by id for 30 minutes in IndexedDB (`monochrome-cache`, `cache.js`).
  Rust should keep its own cache as well (InnerTube browse is the slow part).

### 1.2 Query parameters the UI actually sends

| Endpoint | Params sent by the UI | Call site |
|---|---|---|
| `/search/` | exactly one of `q` (combined), `s` (tracks), `a` (artists), `al` (albums), `p` (playlists), `v` (videos), `i` (ISRC) | `api.js:608,690,720,746,772,797,821` |
| `/info/` | `id` | `api.js:1687` |
| `/album/` | `id`; pagination: `id&offset=N&limit=500` | `api.js:869,934` |
| `/album/similar/` | `id` | `api.js:1492` |
| `/artist/` | `id`; fallback `f=<id>&skip_tracks=true`; top tracks `f=<id>&skip_tracks=true&offset=N&limit=15` | `api.js:1231,1284,1365` |
| `/artist/similar/` | `id` | `api.js:1439` |
| `/artist/bio/` | `id` | `api.js:1466` |
| `/playlist/` | `id`; pagination: `id&offset=N` (no limit) | `api.js:1015,1074` |
| `/mix/` | `id` | `api.js:1135` |
| `/video/` | `id` | `api.js:846` |
| `/track/` | `id`, `quality`, optional `immersiveAudio=true` | `api.js:1809` |
| `/trackManifests/` | `id`, `quality`, `adaptive`, repeated `formats` | `api.js:1798` |
| `/recommendations/` (after F6) | `id` | new |
| `/explore/`, `/explore/genre/` (after F11, optional) | `id` (genre) | new |

The UI never sends `offset` or `limit` to `/search/`. Return the first page (about 20 items).

---

## 2. Shared object shapes

These objects appear inside many responses. The required/optional status comes from what the
renderers and normalizers actually read.

### 2.1 Track

Read by `prepareTrack` (`api.js:287`), `createTrackItemHTML` (`ui.js:566`), the player, the
library minifier `db._minifyItem` (`db.js:282`) and the recommenders.

| Field | Type | | Notes |
|---|---|---|---|
| `id` | string | **R** | videoId. |
| `title` | string | **R** | A title of `"Unavailable"` marks the track unavailable (`utils.js:515`). |
| `version` | string\|null | O | Rendered as `title (version)` (`utils.js:620`). Use null. |
| `duration` | number | **R** | **Seconds** (an integer is fine). Drives the list duration, queue totals and LRCLIB lookup. |
| `artists` | ArtistRef[] | **R** | At least one entry. The display string is `artists.map(a=>a.name)` (`utils.js:625`), and each one becomes a `data-artist-id` link. |
| `artist` | ArtistRef | O | `prepareTrack` fills it from `artists[0]`. Send it anyway, since some code reads `track.artist.id` before preparation (smart recs, `db.js` minify). |
| `album` | TrackAlbumRef | **R** | Must be present on **every** track, including singles and UGC videos with no YTM album. It is the only cover source that survives the library (`db.js:298-309` keeps `album.cover` and drops top-level `cover`/`image`). If the cover is missing, `enrichTracksWithAlbumCover` fires one `/album/` request per track (`api.js:395`). |
| `type` | `"track"` | **R** | Anything containing `video` makes the UI use the video player path (§4). |
| `explicit` | bool | O | Explicit badge. |
| `trackNumber` | number | O | Album pages show it. 1-based. |
| `volumeNumber` | number | O | Disc number. Use 1. |
| `mixes` | `{TRACK_MIX: string}` | O (strongly recommended) | `"RDAMVM"+id`. Enables "Track mix" and "Start mix" (`events.js:1399,1532,1463`). Persisted in favorites. |
| `popularity` | number | O | Artist top tracks are **sorted by it, descending** (`api.js:1333,1412`). Emit `100 - index` to keep YTM's order. |
| `streamStartDate` | ISO string | O | Year display fallback (`utils.js:71`). |
| `audioQuality` | string | O | Use `"HIGH"`. Never emit a lossless or Atmos token, or the UI shows wrong badges. |
| `allowStreaming`, `streamReady` | bool | O | Only `false` matters: it greys the track out (`utils.js:519`). Omit both, or send `true`. |
| `isrc` | string\|null | O | Null. ISRC is not available from YTM. |
| `copyright` | string\|null | O | Null. |
| `replayGain`, `peak` | number | O | Not used by playback (see §4.3). |
| `mediaMetadata` | `{tags:[]}` | O | Empty tags. |
| `audioUrl`, `remoteUrl` | — | **—** | These switch the player to a raw-URL path (`player.js:1367`) that skips preload and crossfade, and `db.js:329` persists them. Don't emit them (see §4.2 path B). |
| `isLocal`, `isTracker`, `isPodcast` | — | **—** | Special player branches. An `id` must also never start with `tracker-` or `podcast_`. |

**ArtistRef**: `{ "id": "UC…" | null, "name": string, "type": "MAIN", "picture": url|null }`.
`id` and `name` are read; `picture` is optional. Featured artists without a browseId get
`id: null` (the link then goes nowhere, which is acceptable).

**TrackAlbumRef**: `{ "id": "MPREb_…" | null, "title": string, "cover": url, "releaseDate"?: "YYYY-MM-DD" }`.
Keep it **slim**. It must not contain `numberOfTracks`, `numberOfItems` or `type`, because the
artist-page scanner (`api.js:1251`) would then pick the embedded object up as a separate album. For
tracks with no album, use `{ "id": null, "title": "", "cover": <thumbnail> }`. Don't emit `album.id`
values the `/album/` endpoint can't serve, because `enrichTracksWithAlbumDates` /
`enrichTracksWithAlbumCover` fetch `/album/?id=<album.id>` for any track lacking a date or cover
(`api.js:352,395`; up to 20 per list, enabled by default via `use-album-release-year`).

### 2.2 Album

Read by `prepareAlbum`, `createAlbumCardHTML` (`ui.js:921`), `renderAlbumPage` (`ui.js:5214`) and
`db._minifyItem`.

| Field | Type | | Notes |
|---|---|---|---|
| `id` | string | **R** | browseId `MPREb_…`. |
| `title` | string | **R** | |
| `cover` | url | **R** | |
| `artist` | `{id, name}` | **R** | The album page dereferences `album.artist.id` and `.name` without a guard (`ui.js:5364,5528`). Cards fall back to `artists`. |
| `artists` | ArtistRef[] | O | |
| `releaseDate` | `"YYYY-MM-DD"` | O | YTM only gives a year, so emit `"YYYY-01-01"`. Discography is sorted by this, descending (`api.js:1325`). |
| `type` | `"ALBUM"`\|`"EP"`\|`"SINGLE"` | O | `EP` and `SINGLE` go to the "EPs and Singles" shelf (`api.js:1328`) and get a card label (`ui.js:933-934`). |
| `numberOfTracks` | number | **R on `/album/`** | It drives pagination (§3.4). It also marks the object as an album for the artist scanner. Search and similar results may omit it (but see patch F9). |
| `duration` | number | O | Seconds. |
| `explicit` | bool | O | |
| `copyright` | string\|null | O | Null. |
| `numberOfVolumes` | number | O | 1. |
| `_href` | string | O | Overrides the card link. Not needed. |

### 2.3 Artist profile

Read by `getArtist` (`api.js:1224`), `renderArtistPage` (`ui.js:6209`) and `createArtistCardHTML`
(`ui.js:1018`).

| Field | Type | | Notes |
|---|---|---|---|
| `id` | string | **R** | `UC…`. |
| `name` | string | **R** | |
| `picture` | url\|null | **R in search results** | If it is missing, `enrichArtistsWithPicture` fires `/artist/?id=` per artist (`api.js:432`). |
| `popularity` | number | O | Rendered verbatim as `"<n>% Popularity"` (`ui.js:6522`). Patch F10 hides it when the value isn't a number. Otherwise emit something like `min(100, round(log10(subscribers)*12))`. |
| `mixes` | `{ARTIST_MIX: string}` | O | The artist radio playlistId. It shows the "Artist mix" button (`ui.js:6467`, `ui.js:5532`, `events.js:1463`). |
| `biography` | `{text, source}` | O | If present, `/artist/bio/` is never called (`ui.js:6454`). Plain text; `\n` becomes `<br>`. Use `source: "YouTube Music"`. |
| `artistRoles` | `[{category}]` | O | Tags under the name. Omit. |
| `artistTypes` | string[] | O | `prepareArtist` copies `[0]` into `type`. Omit. |

### 2.4 Playlist

Read by `createPlaylistCardHTML` (`ui.js:791`), `renderPlaylistPage` (`ui.js:5736`), favorites
(store keyPath `uuid`, `db.js:72`) and `recentActivityManager`.

| Field | Type | | Notes |
|---|---|---|---|
| `uuid` | string | **R** | playlistId **without** the `VL` prefix (see §9). It is the playlist's id everywhere. |
| `id` | string | O | Same value as `uuid`. |
| `title` | string | **R** | Not escaped on cards (`ui.js:799`); see patch F12. |
| `numberOfTracks` | number | **R** | Pagination (§3.6) plus the "N tracks" label. |
| `squareImage` | url | **R** (or `image`) | The UI uses `squareImage \|\| image`. |
| `image` | url | O | |
| `description` | string | O | Set with `textContent`. |
| `duration` | number | O | |
| `type` | string | O | `"EDITORIAL"` / `"USER"`. |
| `creator` | object | O | |
| `_href` | string | O | Card link override. Use `"/mix/<id>"` for `RD…` radio playlists (§9.4). |

### 2.5 Mix header (inside `/mix/`)

`getMix` (`api.js:1131`) copies these fields: `id`, `title`, `subTitle`, `description`, `mixType`,
`images.{LARGE|MEDIUM|SMALL}.url` (the first one present becomes `mix.cover`). `mix.cover` goes
straight into `img.src` (`ui.js:6122`, `ui.js:828`), so it must be a full URL. `subTitle` is
inserted with **`innerHTML`** (`ui.js:6179`); patch F12 changes that to `textContent`. Until the
patch lands, api_emu must HTML-escape `subTitle`.

---

## 3. Endpoints

### 3.1 `/search/`

The normalizer `normalizeSearchResponse(data, key)` (`api.js:282`) walks the JSON for the first
object under `key` that has an `items` array. **If the requested bucket is missing, it returns
whichever other bucket it finds first**, so every bucket a call can ask for must be present,
even if it is empty.

Response, same shape for every variant:

```json
{
  "version": "2.7",
  "data": {
    "tracks":    { "items": [Track…],    "limit": 20, "offset": 0, "totalNumberOfItems": 20 },
    "artists":   { "items": [Artist…],   "limit": 20, "offset": 0, "totalNumberOfItems": 5 },
    "albums":    { "items": [Album…],    "limit": 20, "offset": 0, "totalNumberOfItems": 8 },
    "playlists": { "items": [Playlist…], "limit": 20, "offset": 0, "totalNumberOfItems": 6 },
    "videos":    { "items": [],          "limit": 0,  "offset": 0, "totalNumberOfItems": 0 }
  }
}
```

- `?q=` (combined; YTM search without a filter): fill all five buckets, with `videos` empty.
  If this call fails once, the UI sets `_combinedSearchUnavailable` for the rest of the session
  and switches to five scoped calls (`api.js:660`). So make `?q=` reliable, or make the scoped
  ones cheap.
- `?s=` uses the YTM *songs* filter and fills `tracks`. Optionally merge in the *videos* filter
  results as tracks.
- `?a=` uses the *artists* filter, `?al=` the *albums* filter, and `?p=` the *community playlists*
  filter (plus featured playlists).
- `?v=` always returns empty buckets. `?i=` (ISRC) always returns empty buckets too.
- The other buckets may be omitted on a scoped call, but sending all five empty except the
  requested one is the safest choice.
- `limit`/`offset`/`totalNumberOfItems` are informational only. Set them to `items.length` / `0` /
  `items.length`.
- Albums are deduplicated by `[title, numberOfTracks || 0]` (`api.js:573`). See patch F9, otherwise
  two different artists' "Greatest Hits" collapse into one.
- Artist search items need `picture` (§2.3). Track items need `album.cover` (§2.1).

### 3.2 `/info/?id=<videoId>`

```json
{ "version": "2.7", "data": Track }
```

`getTrackMetadata` (`api.js:1682`) takes `json.data || json` and finds the element whose `id == id`
(loose equality; the id is a string). It is used by the track page (`ui.js:7497`), editor's picks,
`app.js:2371` and smart features. Source: InnerTube `next` with `videoId` (its first
`playlistPanelVideoRenderer` has title, artists, album, thumbnail and duration) or `player`
(videoDetails, which has no album). `album` is still required (§2.1).

### 3.3 `/recommendations/?id=<videoId>` (new caller, existing hifi-api route)

```json
{ "version": "2.7", "data": { "items": [ { "item": Track, "type": "track" } … ], "totalNumberOfItems": 24 } }
```

Same shape as `HiFi.ts:1520`. Each element may also be a bare Track; the patched reader takes
`entry.item || entry.track || entry`. **Exclude the seed track** (the native client drops item 0
when it equals the seed). Source: `next` with `playlistId=RDAMVM<id>`, then drop the first entry.
About 20–25 items.

This one endpoint feeds: autoplay after a single track (`events.js:2367`, `player.js:2226,2388`),
home "Recommended songs" (`ui.js:4202`), playlist "Recommended" (`ui.js:5649`) and track-page
recommendations (`ui.js:7622`), all through `getRecommendedTracksForPlaylist` (`api.js:1512`). That
function takes the first 3 seeds, merges their recommendations, **shuffles** them and slices to `limit`.

### 3.4 `/album/?id=<MPREb_…>[&offset=N&limit=500]`

```json
{
  "version": "2.7",
  "data": {
    …Album fields (id, title, cover, artist, artists, releaseDate, type, numberOfTracks, duration, explicit),
    "items": [ { "item": Track, "type": "track" }, … ]
  }
}
```

- Detection (`api.js:879`): the album is `data` itself if it has `numberOfTracks` or `title`.
  Tracks come from `data.items`, and each element is `i.item || i`.
- **Pagination**: while `album.numberOfTracks > tracks.length`, the UI calls
  `/album/?id=X&offset=<loaded>&limit=500` and appends `data.items`. It stops on an empty page,
  or when the first new id equals the first id it already has (`api.js:928-976`). Either return
  everything at once with `numberOfTracks == items.length`, or honour `offset`/`limit`. For
  `offset >= total`, return `{"data":{"items":[]}}`, not an error.
- YTM album tracks that have no `videoId` (greyed out) are skipped. `numberOfTracks` counts only
  the emitted ones.
- Every track's `album` must be filled. JS fills `album.cover` from the parent when it is missing
  (`api.js:994`), but only if `track.album` exists.
- `trackNumber` = 1-based index, `volumeNumber` = 1, `artists` from the track row, falling back to
  the album artists.
- If the id is an `OLAK5uy_…` audio playlist id (some YTM surfaces only expose that), resolve it to
  the album browseId first. ytmusicapi's `get_album_browse_id` shows how: browse `VL<id>` and find
  the `MPREb_` in the response.

### 3.5 `/artist/?id=<UC…>` and `/artist/?f=<UC…>&skip_tracks=true[&offset&limit]`

Response for **both** forms (this matches the native client's v2 path at `HiFi.ts:1884`):

```json
{
  "version": "2.7",
  "artist": ArtistProfile,
  "cover": { "id": "UC…", "name": "…", "750": url } ,
  "albums": { "items": [Album…] },
  "tracks": [Track…]
}
```

How `getArtist` (`api.js:1224`) consumes it:

1. `rawArtist = (json.data || json).artist`. Then it recursively **scans the whole payload**. Any
   object with `id` and (`numberOfTracks` | `numberOfItems` | `type == album/ALBUM`) becomes an
   album. Any object with `id` and (`duration` | `trackNumber != null` | `type == track`) that is
   not an album or a video becomes a track (`api.js:1250-1277`). This is why TrackAlbumRef has to
   stay slim (§2.1).
2. If no album was found, it calls `/artist/?f=<id>&skip_tracks=true` and scans that too. So `f`
   must be accepted as a synonym of `id`.
3. **Albums and tracks are kept only if they "belong" to the artist**: one of `item.artistId`,
   `item.artist.id`, `item.artists[].id` must match the page artist (`api.js:1292`). It currently
   compares with `Number()`, and `Number("UC…")` is `NaN`, so **everything is filtered out until
   patch F5 lands**. Either way, api_emu must set `album.artist = {id: <page artist id>, name}` on
   every discography album (YTM artist shelves don't repeat the artist), and every top track must
   list the page artist in `artists`.
4. Releases are deduplicated, sorted by `releaseDate` descending, and split by
   `type ∈ {EP, SINGLE}` into `eps`; the rest go to `albums`.
5. Top tracks: filtered (step 3), sorted by `popularity` descending, and the first **15** are kept.
6. Unless `lightweight`, it also calls `/search/?v=<artist name>` for videos (empty in v1; patch
   F5 removes the call).
7. With the default settings it then calls `/album/` for up to 20 top-track albums that lack
   `releaseDate` (`enrichTracksWithAlbumDates`). Put `album.releaseDate` on top tracks when the
   year is known, or accept the extra `/album/` calls (cache them).

Sources: the YTM artist page (`browse` UC…). "Songs" shelf → `tracks` (first 15, `popularity =
100 - i`). "Albums" + "Singles" shelves → `albums.items`. If a shelf has a "See all"
browse endpoint (params), fetch it so the full discography is listed. Header thumbnail →
`picture`. Header `radioEndpoint.watchPlaylistEndpoint.playlistId` → `mixes.ARTIST_MIX`.
Description → `biography`. Subscriber count → `popularity`.

**Top tracks paging** (`getArtistTopTracks`, `api.js:1351`, used by `player.js:2655` to extend
artist "popular" playback): `/artist/?f=<id>&skip_tracks=true&offset=N&limit=15` → the UI reads
`(json.data || json).tracks` and sets `hasMore = tracks.length === limit`. Serve the slice
`[offset, offset+limit)` of the artist's full "Songs" playlist (the shelf's `VL…` browse with
continuations). The plain `id=` call can ignore `offset`/`limit`.

`cover` is not read by `LosslessAPI` and can be null.

### 3.6 `/playlist/?id=<playlistId>[&offset=N]`

```json
{
  "version": "2.7",
  "playlist": Playlist,
  "items": [ { "item": Track, "type": "track", "cut": null } … ]
}
```

- `getPlaylist` (`api.js:1011`) unwraps `data` if present, then reads `.playlist` and `.items`.
  Either top-level or `data`-wrapped works; use top-level, like `HiFi.ts:2406`.
- **Pagination**: while `playlist.numberOfTracks > tracks.length`, it calls
  `/playlist/?id=X&offset=<loaded>` (no `limit`) and appends `.items` until an empty page arrives
  (`api.js:1068-1109`). For large YTM playlists, return the first page (about 100) with the true
  total in `numberOfTracks`, and serve later pages through continuations. Rust keeps the
  continuation token per (id, offset) or simply re-walks. The total comes from the header's
  "N songs" text. If it's unknown, set `numberOfTracks = items.length` once there is no
  continuation left.
- Skip rows that have no videoId.
- Callers: the playlist page (`ui.js:5955`), the card play button (`events.js:1427`, which uses
  `item.uuid`), and header-like (`events.js:2205`).
- **Never** return a playlist whose `uuid` starts with `VL`: `MusicAPI.getPlaylist` sends `VL…`
  ids to the community-playlist worker (`music-api.js:323`; patch F4 removes that).
- An id that looks like a UUID is treated as a local user playlist first (`ui.js:5762`). YTM ids
  never look like UUIDs.

### 3.7 `/mix/?id=<RD…>`

```json
{
  "version": "2.7",
  "mix": {
    "id": "RDAMVMdQw4w9WgXcQ",
    "title": "Radio: Never Gonna Give You Up",
    "subTitle": "Rick Astley",
    "description": "",
    "mixType": "TRACK_MIX",
    "images": { "LARGE": { "url": "volna://api/image/?u=…&s=544" } }
  },
  "items": [ Track | { "item": Track } … ]
}
```

- **No `data` envelope.** `getMix` reads `json.mix` and `json.items` at the top level (`api.js:1138`).
  Without `mix` it throws "Mix metadata not found".
- `mixType`: `TRACK_MIX` for `RDAMVM…`, `ARTIST_MIX` for the artist radio id.
- Source: InnerTube `next` with `playlistId=<id>` (for `RDAMVM<v>`, also send `videoId=<v>`).
  Include the seed track as item 0 (it is a radio "starting from" the seed). Use 25–50 items:
  the first `next` page plus one continuation.
- Up to 10 album-date enrichments follow (`api.js:1149`).
- Mix ids appear in URLs (`/mix/<id>`), favorites (`favorites_mixes`, keyPath `id`) and "Recent".
  A radio is not a stable list, so a re-opened mix may contain different tracks. That is expected.

### 3.8 `/album/similar/?id=<MPREb_…>`

```json
{ "version": "2.7", "albums": [Album…] }
```

Top-level `albums` (`api.js:1498`: `data.items || data.albums || data.data || array`). **Don't wrap in
`data`**, because `data.data` being an object breaks `.map`. YTM has no "similar albums" surface.
Recommended source: the radio of the album's first track (`next` `RDAMVM<firstVideoId>`); collect
the distinct `album`s of the radio tracks, excluding the source album (and optionally the same
artist), up to 12. Each needs `id`, `title`, `cover`, `artist{id,name}`, and `releaseDate`/`type`
when known.

Consumers: the album page "Similar albums" (`ui.js:5604`) and **home "Recommended albums"**
(`ui.js:4255`, seeded by `album.id` of a random library track, so it must work for any album id the
UI has stored).

### 3.9 `/artist/similar/?id=<UC…>`

```json
{ "version": "2.7", "artists": [ { "id": "UC…", "name": "…", "picture": url }, … ] }
```

Top-level `artists` (`api.js:1446`). Source: the artist page "Fans might also like" shelf.
Consumers: the artist page (`ui.js:6478`), the album page (`ui.js:5579`) and **home "Recommended
artists"** (`ui.js:4534`).

### 3.10 `/artist/bio/?id=<UC…>`

```json
{ "version": "2.7", "data": { "text": "…", "source": "YouTube Music" } }
```

`getArtistBiography` destructures `{ data }` and needs `data.text` (`api.js:1469`). It is only
called when the artist response had no `biography`, so putting `biography` on `/artist/` makes
this endpoint nearly dead. Implement it anyway; it is trivial. Bio markup: plain text. TIDAL's
`[wimpLink …]` tags are parsed but not needed.

### 3.11 `/video/?id=`

Not used in v1 (decision 5). For reference: `getVideoStreamUrl` (`api.js:2769`) searches the JSON
for `manifest`, then `OriginalTrackUrl`, `url`, `streamUrl` and `manifestUrl`, and sets a plain URL
directly on the `<video>` element (`player.js:1475`). A later implementation could return
`{"version":"2.7","video":{"OriginalTrackUrl":"volna://stream/<id>?kind=video"}}` with a muxed
format.

### 3.12 `/` root

Not called by the UI. Optionally return `{"version":"2.7","Repo":"volna"}`.

---

## 4. Playback

### 4.1 What the player consumes

The player asks `MusicAPI.getStreamUrl(track.id, quality, { track })` (`player.js:670,1506,1844`)
for a **stream descriptor** and branches on it (`player.js:1565-1660`):

| Descriptor | Path |
|---|---|
| `playbackType: 'hls'`, `delivery: 'hls'`, mime containing `mpegurl`, or url containing `.m3u8` | Shaka/hls.js |
| `playbackType: 'dash'`, mime containing `dash`, url `data:`/`.mpd`, or a `blob:` url that isn't `direct` | Shaka (MSE) |
| anything else | **native**: `prepareNativePlayback` → `audio.src = url` (`player.js:831`) |

Fields read from the descriptor: `url` (**R**), `playbackType` (**R**: `'direct'`), `mimeType` /
`mediaMimeType`, `provider`, `quality`, `qualityDisplay`, `codec`, `lossless`, `bitDepth`,
`sampleRate(Hz)`, `bitrateKbps`, `channels`, `rgInfo`, `rgInfoFallback`, `waveform`, `singleUse`
and `preloadManager`. The quality badge text comes from `codec` and `bitrateKbps` (`utils.js:440`):
`codec: 'opus'` shows "Opus", and adding `bitrateKbps` shows "Opus 160k".

The `<audio>` element has `crossorigin="anonymous"` (`index.html:86`) and goes through Web Audio
(EQ, visualizers). **`volna://stream/` responses must therefore carry
`Access-Control-Allow-Origin: *`**, or the audio graph outputs silence. The element issues Range
requests: answer `206` with `Content-Range`, `Accept-Ranges: bytes`, the correct `Content-Type`
(`audio/webm` for opus/251, `audio/mp4` for m4a/140) and
`Access-Control-Expose-Headers: Content-Range, Content-Length, Accept-Ranges`.

The next track is preloaded by setting `preloader.src = streamUrl` on a second `<audio>`
(`player.js:773`). That triggers Rust's yt-dlp resolve ahead of time, which is good. Crossfade
works with direct streams (`player.js:1810`).

### 4.2 The path to satisfy (recommended): local descriptor, no API call

**Path A (use this).** Patch F2 rewrites `MusicAPI.getStreamUrl` (`music-api.js:345-362`). Right
now it goes only to tracks.monochrome.st and never reaches `LosslessAPI`:

```js
async getStreamUrl(id, quality, options = {}) {
    const low = normalizeQualityToken(quality) === 'LOW';
    return {
        url: volnaUrl('stream', `/${encodeURIComponent(String(id))}`, low ? { q: 'low' } : undefined),
        playbackType: 'direct',
        provider: 'youtube',
        quality: low ? 'LOW' : 'HIGH',
        qualityDisplay: 'Opus',
        codec: 'opus',
        lossless: false,
        mimeType: 'audio/webm',
        mediaMimeType: 'audio/webm',
        rgInfo: null,
        waveform: null,
    };
}
```

The stream URL must not contain `.mpd` or `.m3u8`. Quality mapping: the UI default is
`HI_RES_LOSSLESS` (`app.js:358`). Everything except `LOW` maps to the best audio (itag 251/140),
and `LOW` maps to 249/250. Rust may fall back to m4a and send `Content-Type: audio/mp4`; the
`<audio>` element doesn't care that the descriptor said webm.

If playback fails while quality is `HI_RES_LOSSLESS`, the player retries once with `LOSSLESS`
(`player.js:1670`). That retry is harmless.

**Path B (zero-patch fallback, not recommended).** Put `audioUrl: "volna://stream/<id>"` on every
Track. `player.js:1367` then sets `audio.src = track.audioUrl` directly. It skips preload
(`player.js:665`) and crossfade, and `db.js:329` persists the URL into favorites. Mentioned only
for completeness.

### 4.3 ReplayGain

With `rgInfo: null` the player applies no gain (`backfillReplayGainFromTrack` is a no-op,
`player.js:809`). Optional later step: YTM `player` responses carry `playerConfig.audioConfig.loudnessDb`.
api_emu could expose it as `Track.replayGain`, and the F2 descriptor could build
`rgInfo = { trackReplayGain: -loudnessDb, trackPeakAmplitude: 1, albumReplayGain: -loudnessDb, albumPeakAmplitude: 1 }`.
Not needed for v1.

### 4.4 `/track/` and `/trackManifests/` (only if something still calls them)

After F2 the only remaining caller is `MusicAPI.getTrack` → `LosslessAPI.getTrack` (`api.js:1777`),
used by the header "like" fallback on the track page (`events.js:2212`, which patch F7 switches
to `getTrackMetadata`). If api_emu implements them anyway:

- `/trackManifests/`: return **404**. `getTrack` catches the error and falls back to `/track/`.
  (A real response is JSON:API `{data:{data:{id, attributes:{uri, formats, hash,
  trackPresentation}}}}`, and the UI **fetches `attributes.uri`** and base64-encodes the body,
  `api.js:1627`. That path isn't worth emulating.)
- `/track/?id=X&quality=Q` → TIDAL "BTS" playback info:

  ```json
  { "version": "2.7", "data": {
      "trackId": "X", "assetPresentation": "FULL", "audioMode": "STEREO", "audioQuality": "HIGH",
      "manifestMimeType": "application/vnd.tidal.bts",
      "manifest": "<base64 of {\"mimeType\":\"audio/webm\",\"codecs\":\"opus\",\"encryptionType\":\"NONE\",\"urls\":[\"volna://stream/X\"]}>"
  } }
  ```

  `parseTrackLookup` requires a `manifest` key (`api.js:481`). `extractStreamUrlFromManifest`
  base64-decodes it, and `JSON.parse(...).urls[0]` gives the URL (`api.js:540`). Its raw-text regex
  fallback only matches `https?://`, so the JSON form is mandatory for `volna://`.

---

## 5. Images

### 5.1 How the UI builds image URLs today

The builders are `LosslessAPI.getCoverUrl / getCoverSrcset / getArtistPictureUrl /
getArtistPictureSrcset / getVideoCoverUrl` (`api.js:2816-2879`), wrapped by `MusicAPI` (`music-api.js:373-415`).
A TIDAL image is a UUID; `-` is replaced with `/`, giving
`https://resources.tidal.com/images/<a>/<b>/<c>/<d>/<e>/<size>x<size>.jpg` with sizes 80/160/320/640/750/1080/1280.
**Pass-through** (the value is used as-is) happens only when the string starts with:

| Builder | Passes through |
|---|---|
| `MusicAPI.getCoverUrl`, `getArtistPictureUrl` | `http:`, `https:`, `blob:`, `data:`, `assets/` |
| `LosslessAPI.getCoverUrl`, `getArtistPictureUrl`, `getVideoCoverUrl` | `http`, `blob:`, `assets/` |
| `LosslessAPI.getCoverSrcset` | returns `''` for `http`, `blob:`, `assets/` |
| `LosslessAPI.getArtistPictureSrcset` | returns `''` only for `blob:`, `assets/` (not even `http`) |
| `MusicAPI.getVideoCoverUrl` | `blob:` (everything else goes to the TIDAL builder) |

So on Windows, `http://volna.localhost/api/image/…` already passes `getCoverUrl`. On Linux,
`volna://…` does **not**: it turns into `https://resources.tidal.com/images/volna:/…`. Worse, on
both platforms the now-playing bar's `srcset` (`player.js:466,1281`) gets a TIDAL srcset built
from the URL, which **overrides `src`** and breaks the image. Patch F3 fixes all five builders.

Other image sinks that take the value raw (they need a full URL and never go through a builder):
`mix.cover` (`ui.js:828,6122`), user playlist `cover`, `suggestion.image` (`ui.js:5157`), and
MediaSession artwork via `getCoverUrl(cover,'1280')` (`player.js:3088`).

Every `<img>` has `crossorigin="anonymous"`, and the vibrant-colour extractor reads pixels
through a canvas (`ui.js:362`), so image responses need `Access-Control-Allow-Origin: *`
(`protocol.rs` adds it to everything).

### 5.2 Contract for api_emu

- Emit absolute URLs in the platform's form: `volna://api/image/?u=<urlencoded source>&s=<px>` on
  Linux/macOS and `http://volna.localhost/api/image/?u=…&s=<px>` on Windows (`cfg!(windows)`).
  This mirrors `ui/js/volna-url.js`; put one `volna_url(route, path, query)` helper in Rust.
- Default `s=544` for covers and artists. (The UI asks for 80–1280; F3's optional size rewrite
  replaces `s` when it sees a `volna` image URL, so lists can load 120px thumbnails.)
- The Rust image route fetches the source through the shared proxied client, applies a host
  allowlist (`lh3.googleusercontent.com`, `yt3.ggpht.com`, `yt3.googleusercontent.com`,
  `i.ytimg.com`, `i9.ytimg.com`; anything else → 400, so it is not an open proxy) and caches on
  disk. It sends the upstream `Content-Type` and `Cache-Control: max-age=…`.
- Size rewriting on the Rust side: googleusercontent/ggpht URLs end in `=w<N>-h<N>-…` or `=s<N>…`;
  replace that suffix with `=w<s>-h<s>-l90-rj`. For `i.ytimg.com/vi/<id>/<name>.jpg`, pick
  `hqdefault` (or `maxresdefault` when `s > 480`). Optional `crop=1`: center-crop 16:9 video
  thumbnails to a square, since every cover slot in the UI is square.
- Pick the **largest** entry of an InnerTube `thumbnails[]` array as the source.

---

## 6. Id format assumptions found in the UI

| Where | Assumption | Effect with YTM ids |
|---|---|---|
| `api.js:1305` `matchesArtistId` | `Number(id) === Number(artistId)` | `NaN !== NaN`: the artist page has **no albums and no top tracks**. Must patch (F5). |
| `api.js:1658,1676,1812` | `Number(raw.id)` in manifest normalization | id becomes `NaN`/`null`. Only on the unused manifest path; patch it to `String` anyway (F8). |
| `music-api.js:348,485`, `tracks-api.js:834` | `/^\d{17,20}$/` = tracks.monochrome.st id | Never matches an 11-char videoId. Harmless; removed with F2/F4. |
| `music-api.js:323` | `id.startsWith('VL')` = community playlist | Collides with YTM browse ids. api_emu strips `VL`, and F4 removes the branch. |
| `music-api.js:463-503`, `router.js:33-45` | `t:`/`q:`/`tracks:`/`mono:` prefixes, `/t/`, `/tracks/`, `/mono/`, `/monochrome/` path prefixes | YTM ids contain no `:` or `/`. Harmless. |
| `ui.js:5762` | UUID regex = local user playlist | YTM ids never match. Fine. |
| `db.js:567-577` (`importData`) | numeric-looking strings become `parseInt` | A videoId made only of digits (possible, very rare) would become a number on library import. Patch F8: stop converting. |
| `router.js:82`, `player.js:664,1340`, `db.js:323-325` | ids starting with `tracker-` / `podcast_` are special | YTM ids can't contain those prefixes. Fine. |
| DOM | ids go into `data-*-id="${id}"` and `querySelector('[data-album-id="${id}"]')` unescaped | `[A-Za-z0-9_-]` is safe. Fine. |
| `storage.js:3238` | hard-coded blocked artist `3995478` | Compared with `String()`. Never matches. Fine. |
| `ui.js:6410` | bio links navigate to `/<type>/t/<id>` | The router strips `t/`. Fine. |

## 7. How recommendations, mixes, "similar" and the home page are populated

- **Home "For you"** (`renderHomePage`, `ui.js:2854`) runs only when the local library is
  non-empty: history, favorite tracks or playlists. Otherwise the welcome screen shows. Seeds
  (`getSeeds`, `ui.js:4135`) come from smart-recs (`listeningTracker`, localStorage) or a random
  mix of playlist tracks, favorites and history. The sections:
  - Recommended songs = `getRecommendedTracksForPlaylist(seeds, 20)` → `/recommendations/` (§3.3) for 3 seeds.
  - Recommended albums = `/album/similar/?id=<first seed with album.id>` (§3.8).
  - Recommended artists = `/artist/similar/?id=<first seed's artist id>` (§3.9).
  - Recent = `recentActivityManager` (localStorage).
  There is **no server-side home feed**. The YTM home (`FEmusic_home`) can be added later as a
  new endpoint. "My Wave" (phase 4) is a separate feature and isn't part of this contract.
- **Hot & New tab** fetches `https://hot.monochrome.tf/` straight from the webview (`ui.js:2962`) and
  the genre pages fetch `…/explore/genre/?id=` (`ui.js:4095`). Optional F11: re-point them to
  `/explore/` and `/explore/genre/?id=` with the same JSON shape:
  `{ top_albums: Album[], top_tracks: Track[], featured_playlists: Playlist[],
  sections: [{ type: 'ALBUM_LIST'|'TRACK_LIST'|'PLAYLIST_LIST', title, items }] }`, built from YTM
  charts, new releases and moods & genres. Those tracks go to `renderListWithTracks` **without**
  `prepareTrack`, so emit them already fully shaped (`type: 'track'`, `artist` set).
- **Editor's Picks tab** reads `public/editors-picks*.json` (TIDAL ids). Hide it (F11).
  **AOTY tab** and the album-page AOTY ratings (`ui.js:5367`) are third-party; hide them.
- **Autoplay / infinite queue**: `player.js:2226,2388` → `/recommendations/`.
- **Mixes**: `TRACK_MIX`/`ARTIST_MIX` ids (§2.1, §2.3) → `/mix/`.
- **Search suggestions**: currently the tracks.monochrome.st search (`music-api.js:190`). F4
  either returns `[]` or calls a new optional `/search/suggest/?q=` that returns
  `[{kind:'term', searchTerm, displayTerm}]` (YTM `music/get_search_suggestions`). Don't emit
  `kind:'song'`, because selecting one navigates via `track.tracksTrackId` (`app.js:2057`).

## 8. Where the local library lives (all in the webview, nothing in Rust)

- IndexedDB **`MonochromeDB`** v11 (`db.js:13`): `favorites_tracks`, `favorites_videos`,
  `favorites_albums`, `favorites_artists` (keyPath `id`), `favorites_playlists` (keyPath `uuid`),
  `favorites_mixes` (keyPath `id`), `history_tracks` (keyPath `timestamp`), `user_playlists`
  and `user_folders` (keyPath `id`, locally generated UUIDs), `pinned_items`, `settings`. Items are
  stored **minified** by `db._minifyItem` (`db.js:282`). Track fields kept: `id, title, duration,
  explicit, artist, artists[{id,name}], album{id,title,cover,releaseDate,vibrantColor,artist,
  numberOfTracks,mediaMetadata}, copyright, isrc, trackNumber, streamStartDate, version, mixes,
  audioUrl, audioQuality, mediaMetadata` (note: not `type`, `cover` or `image`). Liked tracks
  therefore keep working only if the cover lives in `album.cover` and the id stays playable
  through `/stream`.
- IndexedDB `monochrome-cache`: the API response cache (`cache.js`).
- localStorage: `monochrome-listening-data` (ListeningTracker: per-track play/skip signals and
  per-artist affinity; hooks in `events.js:519,529,567,646`, `player.js:165,2441`). These hooks are
  the natural place for phase 4 to report signals to the Rust `wave` engine. Also
  `monochrome-recent-activity`, `playback-quality`, `use-album-release-year`, `dev-mode-*`, and
  every other setting in `storage.js`.

---

## 9. Mapping YouTube Music to the contract

### 9.1 Ids

| Contract id | YTM source | Format | Notes |
|---|---|---|---|
| `Track.id` | `videoId` (`playlistItemData.videoId`, overlay `watchEndpoint.videoId`, `playlistPanelVideoRenderer.videoId`) | 11 chars `[A-Za-z0-9_-]` | Songs (ATV), official videos (OMV) and UGC all become `type:'track'`. |
| `Album.id`, `TrackAlbumRef.id` | browseId with `pageType = MUSIC_PAGE_TYPE_ALBUM` | `MPREb_` + 11 chars | `/album/` also accepts `OLAK5uy_…` (resolve it, §3.4). |
| `Artist.id`, `ArtistRef.id` | browseId with `pageType = MUSIC_PAGE_TYPE_ARTIST` (or `USER_CHANNEL`) | `UC` + 22 chars | Strip a `MPLA` prefix (library artist ids `MPLAUC…`). |
| `Playlist.uuid` | `playlistId` / the browseId **without the leading `VL`** | `PL…`, `RDCLAK5uy_…`, `OLAK5uy_…`, `VLPL…` → `PL…` | `/playlist/` re-adds `VL` for `browse`. |
| mix id (`TRACK_MIX`) | `"RDAMVM" + videoId` | | `/mix/`: `next(videoId, playlistId)`. |
| mix id (`ARTIST_MIX`) | artist header `radioEndpoint.watchPlaylistEndpoint.playlistId` | usually `RDEM…` | `/mix/`: `next(playlistId)`. If the header has none, use the `RDAMVM` of the first top track. |

### 9.2 Field mapping (the main ones)

| Contract | YTM |
|---|---|
| `title` | flex column 0 text / `title.runs[0].text` |
| `artists[]` | runs whose `navigationEndpoint.browseEndpoint.browseEndpointContextSupportedConfigs…pageType` is `ARTIST` (`id` = browseId); text runs without an endpoint become `{id:null,name}`. Separators (" • ", " & ", ", ") are dropped. |
| `album` | the run with pageType `ALBUM` (`id`, `title`) + the row thumbnail → `cover` |
| `duration` | `"m:ss"` / `"h:mm:ss"` text → seconds (`lengthText`, the fixed column, or `lengthSeconds` from `player`) |
| `explicit` | the `badges[].musicInlineBadgeRenderer` icon `MUSIC_EXPLICIT_BADGE` |
| `releaseDate` | the year in the album subtitle runs → `"YYYY-01-01"` |
| `Album.type` | album subtitle first run: "Album" → `ALBUM`, "EP" → `EP`, "Single" → `SINGLE` (localized; set `hl=en` on InnerTube requests to keep this stable) |
| `numberOfTracks` | album page: count of rows with a videoId; playlist: header "N songs" |
| `Playlist.squareImage` | header thumbnail (largest) |
| `Artist.picture` | header `musicImmersiveHeaderRenderer.thumbnail` (wide; use `crop=1`) or `musicVisualHeaderRenderer` foreground thumbnail |

### 9.3 Thumbnails

Take the largest `thumbnails[].url`, wrap it as described in §5.2, and pass `crop=1` for
`i.ytimg.com` video thumbnails and wide artist banners.

### 9.4 Radio playlists in search and shelves

Playlists whose id starts with `RD` aren't browseable as playlists. Emit them as Playlist objects
with `_href: "/mix/<id>"`. Card clicks then open the mix page, and `events.js:1427`'s card play
(which calls `getPlaylist(item.uuid)`) should also accept them, so `/playlist/` must forward `RD…`
ids to the mix implementation and return the playlist shape.

---

## 10. Frontend patches

All paths are under `ui/js/`. "Required" means search, browse and play are broken without it.

**F1 (required): route every hifi-api call to `volna://api`.** `api.js:60-243`
`fetchWithRetry`: replace the body with
`const r = await fetch(volnaUrl('api', relativePath), { signal: options.signal }); if (!r.ok) throw new Error(\`volna api ${r.status} for ${relativePath}\`); return r;`.
This drops the dev-mode branch, `HiFiClient.instance.query` and `tryInstances`. Also drop
`HiFiClient.initialize` at `app.js:333-351` (it fetches TIDAL tokens). Keep the
`instanceof TidalResponse` checks or delete them; they are always false now.

**F2 (required): play `volna://stream/<id>`.** `music-api.js:345-362`: rewrite `getStreamUrl` as in
§4.2 (import `volnaUrl` and `normalizeQualityToken`). `api.js:2712` `LosslessAPI.getStreamUrl` is
then dead; make it delegate to the same builder or delete it.

**F3 (required): pass volna and other absolute image URLs through.** Add one helper, e.g.
`isDirectImage = (v) => typeof v === 'string' && /^(?:[a-z][a-z0-9+.-]*:|assets\/|images\/|\/)/i.test(v)`,
and use it in `music-api.js:373-415` (`getCoverUrl`, `getCoverSrcset` → `''`, `getVideoCoverUrl`,
`getArtistPictureUrl`, `getArtistPictureSrcset` → `''`) and `api.js:2816-2879` (all five). Optionally,
when the URL is a volna image URL, replace its `s` query param with the requested size. Also
`ui.js:717-736` `getCoverHTML`: the editor's-pick branch builds resources.tidal.com/wsrv.nl URLs
for non-`http` strings, so guard it with the same helper.

**F4 (required): remove tracks.monochrome.st and the community worker from the facade.**
`music-api.js`: `search` (`93-112`), `searchTracks` (`133-146`), `searchArtists` (`148-163`) and
`searchAlbums` (`165-180`) call `this.getAPI()` directly. `searchSuggestions` (`190-199`) returns
`[]` (or uses `/search/suggest/`, §7). `getTrack` / `getTrackMetadata` / `getAlbum` /
`getArtist` / `getArtistBiography` / `getArtistTopTracks` (`202-302, 560-574`) drop the
`isTracksId` / `tracksArtistIds` branches. `getPlaylist` (`322-328`) drops the `VL` community
branch. Optional: `ui.js:5066-5083` / `ui.js:40` (community playlists search tab), `app.js:67`
(contributors) and `ui.js:2637` (donation goal) also call third parties.

**F5 (required): artist page with string ids.** `api.js:1305`: compare with
`String(id) === String(artist.id) || String(id) === String(artistId)`. `api.js:1308-1321`: drop the
`searchVideos(artist.name)` call (v1 has no videos).

**F6 (required for recommendations): stop calling tidal.com directly.** `api.js:1704-1751`
`getTrackRecommendations`: `fetchWithRetry(\`/recommendations/?id=${encodeURIComponent(id)}\`)`, then
`(json.data?.items ?? json.items ?? []).map(e => e.item || e.track || e).filter(t => t?.id).map(t => this.prepareTrack(t))`,
keeping the seed-exclusion and the cache.

**F7: header "like" on the track page.** `events.js:2212`: `api.getTrack(id)` returns a stub
`{id: NaN, duration: 0}` (`api.js:1812`). Use `item = await api.getTrackMetadata(id)` instead.

**F8: numeric coercions.** `api.js:1658,1676`: `trackId: raw.id ?? null`, `id: raw.id ?? null`.
`api.js:1812`: `{ id: String(id), duration: 0 }`. `db.js:567-577`: drop the `parseInt`
conversions, or only convert when the whole library is from the legacy TIDAL format (there is
none in Volna, so drop them).

**F9: album dedup across artists.** `api.js:573`: key on `[album.title, album.numberOfTracks || 0, album.artist?.id ?? album.artists?.[0]?.id ?? '']`.

**F10: popularity label.** `ui.js:6522`: render `<span>…% Popularity</span>` only when
`typeof artist.popularity === 'number'`.

**F11: remove TIDAL/third-party home tabs.** `index.html:1106-1107` (Editor's Picks, AOTY): remove
them or hide by default (`homePageSettings.shouldShowEditorsPicks` defaults to true,
`storage.js:2298`). `ui.js:2962` and `ui.js:4095`: point Hot & New at `volnaUrl('api','/explore/')` /
`'/explore/genre/'` if api_emu implements §7, otherwise hide the tab. `ui.js:5367` (album-page
AOTY fetch): remove. `music-api.js:417` `getArtistBanner` (samidy worker): return null.

**F12: HTML injection from YTM user content.** `ui.js:799,801` (`createPlaylistCardHTML`
title/alt) and `ui.js:836,838` (`createMixCardHTML`): wrap them in `escapeHtml`. `ui.js:6179`: replace
`descEl.innerHTML = mix.subTitle` with `textContent`. YTM community playlist titles are
user-controlled.

**F13: paste-a-link.** `app.js:2080-2097` `handleExternalLink`: add `music.youtube.com/` and
`youtube.com/`/`youtu.be/`. Map `watch?v=<id>` to `/track/<id>`, `playlist?list=<id>` to
`/playlist/<id>` (or `/mix/` for `RD…`), `browse/MPREb_…` to `/album/…` and `channel/UC…` to `/artist/…`.

Not needed: ids in `router.js`, DOM selectors, favorites keys, `stripProviderPrefix` and
`isTracksId` all work with YTM strings as they are (§6).

## 11. Open questions and risks

- **WebKitGTK media over a custom scheme**: `<audio src="volna://stream/…">` with Range and
  `crossorigin` has to be checked in the real webview early in phase 2 (Tauri has a streaming
  example for this). If WebKitGTK refuses CORS for media on a custom scheme, the fallback is a
  loopback `http://127.0.0.1:<port>` stream server. It would still go through the same Rust
  resolver and proxy.
- **MediaSession / MPRIS artwork** gets a `volna://` URL that the desktop shell can't load
  (`player.js:3088`). Low priority; serving a `data:` URL would fix it.
- **Album date enrichment** (§3.5 step 7) can cost up to 20 `/album/` browses per artist page or
  mix. Emit `album.releaseDate` on tracks whenever YTM shows a year, and cache `/album/`
  aggressively.
- **`/album/similar/`** has no native YTM surface. The radio-based derivation (§3.8) is a
  heuristic; adjust it if the results look wrong.
- **Localized strings** (album type, "N songs") need `hl=en` on InnerTube requests, or
  parsing that doesn't depend on locale.
- Artist "popularity" isn't a YTM concept. F10 hides it rather than inventing a number.
