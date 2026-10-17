# Native player: playback in Rust, the webview as a remote

Status: design, ready to implement. Two work packages: **RUST** (§6.1) and **UI** (§6.2),
built in parallel against the contract in §3. This document is the contract. Anything a side
needs from the other is written down here, and nothing else should be assumed.

## 0. Goal

Closing the window destroys the webview (WebKitWebProcess, about 800 MB). Volna stays in the
tray at 30–60 MB RSS, music keeps playing, «Моя волна» keeps filling the queue, and MPRIS and
the tray menu still work. Opening the window again (tray "Show", MPRIS Raise, second launch)
creates a fresh webview that attaches to the running player and shows the current track,
position, queue and settings.

Decisions already made: GStreamer through gstreamer-rs (`playbin3`), all audio processing in
Rust (EQ, crossfade and the visualizer analysis), a Tauri v2 tray, and native MPRIS with cover
art.

Non-goals for v1: video playback (music videos already play as audio, contract decision 5),
local files, podcasts, tracker entries, multichannel HRTF, and silence trimming based on
waveforms.

## 1. Architecture

```
                    ┌──────────────────────── volna process ────────────────────────┐
 tray (Tauri) ─────►│ src-tauri/src/player/*  (glue)                                 │
 MPRIS (zbus) ─────►│   commands ─► PlayerHandle ─► crates/player Engine (actor) ────┼─► Deck A playbin3 ─┐
 webview (opt.) ───►│   events  ◄── broadcast  ◄──┘   │  queue / tracker / sleep    │   Deck B playbin3 ─┤► autoaudiosink
                    │   Hooks impl: streams, wave, ytm radio, journal                │   (volnadsp in    │  (PipeWire)
                    │   journal.jsonl, player.json, player-queue.json               │    each deck)     │
                    │ crates/streams loopback server 127.0.0.1 ◄── souphttpsrc ──────┼───────────────────┘
                    └────────────────────────────────────────────────────────────────┘
```

- Audio still comes from the loopback server (`docs/streams.md`), so there is no change to
  resolving, Range or re-resolve. A deck's URI is `"{stream_base}/{videoId}"`, plus `?q=low`
  for LOW quality.
- The webview is optional. Without it, events are dropped and visualizer frames are not computed.

## 2. Inventory: what exists today and who owns it afterwards

R = Rust (`crates/player` or the glue), U = UI, × = dropped.

| Feature | Today | After | Notes |
|---|---|---|---|
| Queue: set, play at index, add next, add to end, remove, move, clear (keep current), wipe | player.js `setQueue/addNextToQueue/addToQueue/removeFromQueue/moveInQueue/clearQueue/wipeQueue` | R | Ops are keyed by `uid` (one per queue entry), see §3. |
| Shuffle (current first, Fisher–Yates rest, restore original order on off) | `toggleShuffle` | R | Port exactly, including add-next/remove keeping both orders in sync. |
| Repeat off/all/one | `toggleRepeat`, `REPEAT_MODE` 0/1/2 | R | Natural end with *one* replays the track. The Next button advances (behaviour change, see §8). |
| Next/prev (skip unavailable/blocked; prev restarts when position > 3 s) | `playNext/playPrev` | R | Blocklist pushed by the UI (`settings.blocklist`). |
| Seek, ±10 s | `seekTo/seekForward/seekBackward` | R | Safari seek correction dropped. |
| Volume, mute, cubic curve | `setVolume`, localStorage `volume`/`muted`, `exponentialVolumeSettings` | R | Software `volume` element in the chain, not the sink volume (WirePlumber stores the stream volume). |
| ReplayGain / normalization | `replayGainSettings`, `rgInfo` (always null for YouTube) | R (stored, no-op) | No loudness data yet; see §8. |
| Playback speed, preserve pitch | `audioEffectsSettings`, `playbackRate`, `preservesPitch` | R | Clamped to 0.25–4. Pitch preserved with `scaletempo`. |
| Gapless | `gaplessPlaybackSettings` (effectively native `<audio>`) | R | `playbin3` `about-to-finish`. |
| Crossfade (on, 1–12 s, equal-power) | events.js `timeupdate` trigger + `crossfadeToNext` | R | Two decks. |
| Silence removal / waveform-aware crossfade | `silenceRemovalSettings`, waveform PNG (SoundCloud) | × | YouTube has no waveform; this is a no-op today. |
| Waveform seekbar | events.js `updateWaveform` | U (unchanged, no-op) | No data. |
| Preload next / prefetch | `_executePreloadNextTracks`, Shaka preload | R | `streams.prefetch(next)` when a track starts. |
| Shaka/DASH/HLS, cross-codec ABR, `forceQuality` | player.js | × | YouTube is progressive opus/m4a. |
| Stream quality HIGH/LOW | localStorage `playback-quality` | R | `settings.quality`. |
| Quality badge (`currentStreamInfo`) | player.js, ui-interactions.js | U reads R state | `state.stream`. |
| Parametric EQ 3–32 bands, type/Q/channel (stereo/mid/side), freq range, preamp, presets, AutoEQ, import/export | audio-context.js + settings.js + storage `equalizerSettings` | DSP in R; editing, presets, AutoEQ, text import/export stay in U | The UI sends explicit band arrays (§3.2 `DspConfig`). |
| Graphic EQ (separate chain) | audio-context.js `geq*` | R (DSP), U (UI) | Same. |
| Mono downmix | `monoAudioSettings` | R | |
| Binaural: crossfeed (bs2b-style), stereo widening | binaural-dsp.js | R | HRTF multichannel ×. |
| Visualizer analysis (AnalyserNode fft 1024, smoothing 0.7) | audio-context.js analyser | R | Binary frames over a Channel (§3.5). |
| Visualizer rendering, presets | visualizer.js, visualizers/* | U | Fed by an `AnalyserShim`. |
| Sleep timer | player.js `setSleepTimer` | R | Survives a closed window. |
| MediaSession / MPRIS | `@capgo/capacitor-media-session` → WebKit MPRIS | R | Native `mpris-server`. |
| Now-playing bar DOM, title, `document.title`, playing indicator | player.js | U | Re-rendered from events. |
| Play-start/end tracking for the wave | wave-logic.js `PlayTracker` + wave.js capture listeners | R | Ported to `crates/player/src/tracker.rs`; calls `Wave::record`. |
| Wave start/top-up/requeue/stop/dislike | wave.js | R (logic), U (card, popover, cold start, buttons) | `player_wave_*`. |
| Wave likes / like sync | wave.js → `wave_event{like}`, `wave_sync_likes` | U (unchanged) | Favourites live in IndexedDB. |
| Monochrome infinite radio (seeds from history/favourites) | `enableRadio/fetchRadioRecommendations` | Seeds picked in U, refill in R | `player_set_continuation{radio, seeds}`; Rust uses `ytm.radio`. |
| Autoplay at queue end | `autoplaySettings`, `fetchAutoplayRecommendations` | R | `settings.autoplay`; `ytm.radio(current)`. |
| Artist popular tracks extension | `artistPopularTracksState` | U (window open only) | `continuation: ui` + `player:queue-low`. |
| History (`db.addToHistory` after 10 s) | events.js timeupdate | R detects, U writes IndexedDB | Through the journal (§3.6). |
| Listening tracker (affinity in localStorage) | events.js + listening-tracker.js | U fed from the journal | |
| Scrobbling (Last.fm, Libre.fm, ListenBrainz, Maloja) | scrobbler timers after `updateNowPlaying` | Now-playing: U live. Scrobble: U from the journal with the original timestamp | §5.7. |
| Error explanation, auto-skip ≤ 3 | playback-failures.js + `stream_failure` | R (skip logic), U (notification text) | `player:error`. |
| Queue persistence | localStorage `monochrome-queue` | R `player-queue.json` | Also stores the position (new). |
| Lyrics sync | lyrics.js on media events | U (unchanged API) | Uses the shim's interpolated `currentTime`. |
| Keyboard shortcuts, command palette | app.js, commandPalette.js | U | Through `Player` methods. |
| Local files / podcasts / tracker / blob URLs | player.js branches | × | The native player rejects tracks without a YouTube id. |

## 3. Contract

### 3.1 Conventions

- JSON is camelCase. Rust structs use `#[serde(rename_all = "camelCase")]`. Tauri maps
  camelCase invoke keys to snake_case Rust parameters, so call `invoke('player_seek', { position, resume })`.
- Times: positions and durations are **seconds** (f64). Wall-clock values are **unix ms** (f64).
- Errors cross IPC as strings `"<code>: <message>"`. Codes: `bad_args`, `unknown_uid`,
  `no_candidates` (wave cold start), `unavailable` (engine not running, e.g. no yt-dlp), `internal`.
- **Ordering:** the engine is one actor that applies commands in the order it receives them. The UI
  bridge **serializes every `player_*` invoke** (one FIFO promise chain) so the order is preserved
  end to end. Commands reply as soon as the state is updated, not when audio starts, so the chain
  never waits on the network.
- Every queue mutation bumps `rev` by exactly 1, and every `player:queue` event carries it.

### 3.2 Types (TypeScript notation; Rust mirrors them in `crates/player/src/model.rs`)

```ts
type Origin = 'user' | 'wave' | 'radio' | 'autoplay';
type Repeat = 'off' | 'all' | 'one';           // UI REPEAT_MODE 0/1/2 ↔ off/all/one
type Status = 'idle' | 'loading' | 'playing' | 'paused' | 'ended';

/** What the UI sends to enqueue a track. */
interface TrackInput {
  id: string;                                  // YouTube videoId (required, [A-Za-z0-9_-]{11})
  title: string;
  artists: { id: string | null; name: string }[];
  album: { id: string | null; title: string } | null;
  duration: number | null;                     // seconds, from metadata
  cover: string | null;                        // as the UI holds it: volna://img/?u=<https>&s=.. or http://volna.localhost/img/?u=..
  explicit: boolean;
  kind: 'track' | 'video';
  origin?: Origin;                             // default 'user'
  ui: object | null;                           // opaque full prepared UI track. Rust stores and returns it verbatim and never reads it
}

/** A queue entry as Rust reports it. */
interface PlayerTrack extends Omit<TrackInput, 'origin'> {
  uid: string;                                 // unique per queue entry (e.g. "q" + u64 counter in base36), stable across reorder/shuffle
  origin: Origin;
  raw: object | null;                          // for Rust-originated tracks (wave/radio/autoplay): hifi-api shaped JSON
                                               // (src-tauri api_emu::shape::track), to run through LosslessAPI.prepareTrack. null otherwise
  unavailable: boolean;                        // set after a failure of kind 'unavailable'
}

interface StreamInfo { codec: string | null; bitrateKbps: number | null; quality: 'HIGH' | 'LOW'; mime: string | null }

interface PlayerError {
  uid: string | null; trackId: string | null;
  kind: 'bot_check' | 'unavailable' | 'no_yt_dlp' | 'decode' | 'timeout' | 'other';
  message: string;
  action: 'skipped' | 'stopped' | 'retrying' | 'none';
  streak: number;                              // consecutive failures
}

interface PlaybackState {
  status: Status;
  uid: string | null;                          // loaded entry
  trackId: string | null;
  position: number;                            // s, sampled at `at`
  at: number;                                  // unix ms of the sample
  duration: number | null;                     // s (GStreamer, else track.duration)
  rate: number;                                // configured rate; UI interpolates only when status === 'playing'
  buffering: boolean;
  crossfading: boolean;
  volume: number;                              // 0..1 slider value (before curve)
  muted: boolean;
  stream: StreamInfo | null;
  error: PlayerError | null;                   // last error for `uid`, cleared on next successful start
}

type ContinuationKind = 'none' | 'wave' | 'radio' | 'ui';
interface Continuation { kind: ContinuationKind; fetching: boolean }   // autoplay applies when kind==='none' && settings.autoplay

interface QueueChanged {
  rev: number;
  order: string[];                             // uids in play order (shuffled order when shuffle is on)
  original: string[] | null;                   // pre-shuffle order, only when shuffle is on
  index: number;                               // index into `order`, -1 = none
  shuffle: boolean;
  repeat: Repeat;
  continuation: Continuation;
  added: PlayerTrack[];                        // full objects for uids not present at rev-1
  updated: PlayerTrack[];                      // existing uids whose fields changed (e.g. unavailable)
}

interface QueueSnapshot {
  rev: number; items: PlayerTrack[];           // play order, full objects
  original: string[] | null; index: number; shuffle: boolean; repeat: Repeat; continuation: Continuation;
}

interface PlayerSettings {
  volume: number;                              // 0..1
  muted: boolean;
  volumeCurve: 'linear' | 'cubic';             // exponentialVolumeSettings: effective = v^3
  quality: 'high' | 'low';
  crossfade: { enabled: boolean; seconds: number };   // 1..12, default false/5
  gapless: boolean;                            // default true
  speed: number;                               // 0.25..4, default 1
  preservePitch: boolean;                      // default true
  replayGain: { mode: 'off' | 'track' | 'album'; preampDb: number };  // stored, no-op in v1
  autoplay: boolean;                           // default false (autoplaySettings)
  blocklist: { trackIds: string[]; artistIds: string[]; albumIds: string[] };
  migrated: boolean;                           // UI imported localStorage once (§5.8)
}

type BandType = 'peaking' | 'lowshelf' | 'highshelf';
interface DspConfig {
  eq: { enabled: boolean; preampDb: number;    // -20..20
        bands: { freq: number; gainDb: number; q: number; type: BandType; channel: 'stereo' | 'mid' | 'side' }[] };   // 0..32 bands
  graphicEq: { enabled: boolean; preampDb: number; bands: { freq: number; gainDb: number; q: number }[] };      // peaking only, gain -30..30
  mono: boolean;
  binaural: { enabled: boolean;
              crossfeed: { enabled: boolean; level: 'low' | 'medium' | 'high' };
              widening: { enabled: boolean; amount: number } };   // 0..2, 1 = neutral
}

interface PlayerSnapshot {
  protocol: 1;
  state: PlaybackState;
  queue: QueueSnapshot;
  settings: PlayerSettings;
  dsp: DspConfig;
  sleepTimer: { endsAt: number } | null;
  wave: { active: boolean };
  journalSeq: number;                          // highest journal seq written
}

type JournalEntry =
  | { seq: number; at: number; kind: 'history'; track: PlayerTrack }          // played ≥ 10 s
  | { seq: number; at: number; kind: 'end'; track: PlayerTrack; startedAt: number;
      position: number; duration: number | null; ended: boolean; inWave: boolean };   // a started play closed
```

### 3.3 Commands (all in `src-tauri/src/player/commands.rs`; all `async`)

| Command | Args | Returns | Semantics |
|---|---|---|---|
| `player_snapshot` | – | `PlayerSnapshot` | Full state for a new window. |
| `player_play` | – | `null` | Resume, or load the current entry at its saved position. No-op when the queue is empty. |
| `player_pause` | – | `null` | |
| `player_toggle` | – | `null` | |
| `player_stop` | – | `null` | Pause and seek to 0 (MPRIS Stop). |
| `player_next` | – | `null` | Next playable entry (skips unavailable/blocked); wraps with repeat all; fetches the continuation if needed; at the end → `ended`. |
| `player_prev` | – | `null` | position > 3 s → seek 0; else previous playable; at index 0 → seek 0. |
| `player_seek` | `{position: number, resume?: boolean}` | `null` | Clamped to [0, duration]. |
| `player_play_uid` | `{uid: string, position?: number}` | `null` | Jump to an entry and play. `unknown_uid` if missing. |
| `player_set_queue` | `{tracks: TrackInput[], startIndex: number, play: boolean, position?: number}` | `{rev}` | Replaces the queue, shuffle off, keeps repeat. Stops the wave and resets the continuation to `none` (the UI calls `player_set_continuation` afterwards for radio). |
| `player_enqueue` | `{tracks: TrackInput[], at: 'next' \| 'end'}` | `{rev}` | Same rules as `addNextToQueue`/`addToQueue`. If nothing is loaded, starts the first added entry. |
| `player_remove` | `{uid}` | `{rev}` | Removing the current entry starts the next one, or goes idle if it was the last. |
| `player_move` | `{uid, to: number}` | `{rev}` | `to` = target index in play order. |
| `player_clear` | `{keepCurrent: boolean}` | `{rev}` | `true` = clearQueue (keep the current track); `false` = wipe (stop, empty, idle). |
| `player_set_shuffle` | `{on: boolean}` | `{rev}` | |
| `player_set_repeat` | `{mode: Repeat}` | `{rev}` | |
| `player_set_settings` | `{patch: Partial<PlayerSettings>}` | `PlayerSettings` | Merged one level deep (nested objects replaced whole). Persisted. |
| `player_set_dsp` | `{config: DspConfig}` | `null` | Replaces the config; applied glitch-free (§4.3). Persisted. |
| `player_set_sleep_timer` | `{minutes: number \| null}` | `{endsAt: number \| null}` | On expiry: 3 s fade-out, pause, timer cleared, `player:sleep {endsAt:null}`. |
| `player_set_continuation` | `{kind: 'none' \| 'radio' \| 'ui', seeds?: TrackInput[]}` | `null` | `radio` needs seeds (the UI picks up to 50 as `pickRadioSeeds` does). Stops the wave if it was active. |
| `player_wave_start` | `{count?: number}` (default 5) | `{rev}` | `Wave::start`, replaces the queue (origin `wave`), plays index 0, continuation `wave`. `no_candidates: …` → the UI opens cold start. |
| `player_wave_stop` | – | `null` | Continuation → `none`, `Wave::stop`. |
| `player_wave_dislike` | – | `null` | Records Dislike for the current track (closing the play first so the skip is not counted twice), tops up, then `next`. |
| `player_wave_requeue` | – | `{rev}` | The UI changed the wave settings (`wave_set_settings`) while the wave is active: same as a re-seed outcome (`requeue_wave`): drop the upcoming wave-origin entries and append `WaveRequeue{dropped, count:3}`. No-op when the wave is not active. |
| `player_visualizer_start` | `{channel: Channel<ArrayBuffer>, fps: number}` | `null` | fps clamped 10..60. Replaces any previous subscription. |
| `player_visualizer_stop` | – | `null` | |
| `player_journal` | `{after: number}` | `JournalEntry[]` | Entries with seq > after, ascending, at most 1000 per call (call again until empty). |
| `player_journal_ack` | `{upTo: number}` | `null` | Rust may drop entries ≤ upTo. |
| `app_ready_to_close` | – | `null` | The UI finished its flush after `app:will-close`. |
| `app_quit` | – | `null` | Real exit (same path as tray Quit). |

The existing commands stay: `wave_status`, `wave_set_settings`, `wave_activities`,
`wave_artist_search`, `wave_related_artists`, `wave_cold_start`, `wave_event` (still used for
`like`), `wave_sync_likes`, `stream_failure`, and the network/yt-dlp ones. The UI stops calling
`wave_start`/`wave_next`/`wave_requeue`/`wave_stop`/`stream_prefetch` (they remain registered
for compatibility).

### 3.4 Events (`app.emit_to("main", …)`, sent only while the `main` webview exists)

| Event | Payload | When |
|---|---|---|
| `player:state` | `PlaybackState` | Any change of status, uid, duration, rate, volume, muted, buffering, crossfading, stream, error. Not emitted for position changes alone. |
| `player:position` | `{uid, position, duration, rate, at, playing: boolean}` | 4 Hz while playing; immediately after a seek, a state change or a track start. |
| `player:queue` | `QueueChanged` | Every queue mutation, including Rust top-ups. |
| `player:error` | `PlayerError` | Every failure (also when skipped). |
| `player:journal` | `JournalEntry` | Live mirror of each journal append. |
| `player:queue-low` | `{remaining: number}` | Continuation `ui` and fewer than 3 entries after the current one, at each track start. |
| `player:settings` | `{settings: PlayerSettings, dsp: DspConfig}` | Settings changed outside the UI (MPRIS volume/rate/shuffle/loop, tray). |
| `player:sleep` | `{endsAt: number \| null}` | Timer set, cleared or expired. |
| `player:wave` | `{active: boolean}` | Wave session started or stopped (including "the listener played something else"). |
| `app:will-close` | `{}` | The window close button was pressed; the UI has about 700 ms to flush and call `app_ready_to_close`. |

### 3.5 Visualizer frames (Channel, raw binary)

`player_visualizer_start` receives a `tauri::ipc::Channel`. Rust sends `InvokeResponseBody::Raw`,
which arrives in JS as an `ArrayBuffer`. Frames are sent at `fps` while playing (and at 2 Hz while
paused, with zeroed data, so the presets can decay). Layout, little-endian:

```
off  size  field
0    4     magic "VVIS"
4    2     version = 1
6    2     freqBins  = 512
8    2     timeLen   = 1024
10   2     flags     bit0 = playing
12   4     sampleRate (48000)
16   8     position (f64 s)
24   512   freq   u8  — AnalyserNode.getByteFrequencyData semantics, mono (L+R)/2
536  1024  time   u8  — getByteTimeDomainData, mono, 128 = 0
1560 1024  timeL  u8
2584 1024  timeR  u8
total 3608 bytes
```

Analysis follows the Web Audio spec so the presets need no retuning: fftSize 1024, Blackman
window, magnitude/N, smoothing `0.7*prev + 0.3*cur`, dB = 20·log10, mapped to bytes over
minDecibels -100 … maxDecibels -30. It is computed on the post-DSP signal of the deck that is
currently "active", before the master volume (so the visualizer does not follow the volume knob;
`visualizer.js` already divides by volume, so the UI passes 1 there).

### 3.6 Rehydration and journal protocol (UI boot, also after the window is recreated)

1. Register `listen()` for every `player:*` event first, buffering payloads.
2. `snap = await invoke('player_snapshot')`, then build the mirror from it.
3. Replay buffered `player:queue` events with `rev > snap.queue.rev`, and buffered
   state/position events with `at > snap.state.at`.
4. For any later `player:queue` with `rev !== mirror.rev + 1`, take a new snapshot (resync).
5. Journal: `last = +localStorage['volna-journal-seq'] || 0`. Loop `player_journal({after:last})`
   until it returns empty. Process the entries (§5.7), store `last`, then call `player_journal_ack({upTo:last})`.
   After that, handle live `player:journal` entries with `seq > last` the same way.

### 3.7 Example payloads (used as fixtures by both sides' tests)

```json
{"protocol":1,
 "state":{"status":"playing","uid":"q1a","trackId":"dQw4w9WgXcQ","position":42.1,"at":1760700000000,
          "duration":213.0,"rate":1,"buffering":false,"crossfading":false,"volume":0.7,"muted":false,
          "stream":{"codec":"opus","bitrateKbps":160,"quality":"HIGH","mime":"audio/webm"},"error":null},
 "queue":{"rev":12,"index":0,"shuffle":false,"repeat":"off","original":null,
          "continuation":{"kind":"wave","fetching":false},
          "items":[{"uid":"q1a","id":"dQw4w9WgXcQ","title":"Never Gonna Give You Up",
                    "artists":[{"id":"UCuAXFkgsw1L7xaCfnd5JJOw","name":"Rick Astley"}],
                    "album":{"id":"MPREb_x","title":"Whenever You Need Somebody"},"duration":213,
                    "cover":"volna://img/?u=https%3A%2F%2Flh3.googleusercontent.com%2Fabc&s=640",
                    "explicit":false,"kind":"track","origin":"wave","raw":{"id":"dQw4w9WgXcQ"},"ui":null,
                    "unavailable":false}]},
 "settings":{"volume":0.7,"muted":false,"volumeCurve":"linear","quality":"high",
             "crossfade":{"enabled":false,"seconds":5},"gapless":true,"speed":1,"preservePitch":true,
             "replayGain":{"mode":"track","preampDb":3},"autoplay":false,
             "blocklist":{"trackIds":[],"artistIds":["3995478"],"albumIds":[]},"migrated":true},
 "dsp":{"eq":{"enabled":false,"preampDb":0,"bands":[]},"graphicEq":{"enabled":false,"preampDb":0,"bands":[]},
        "mono":false,"binaural":{"enabled":false,"crossfeed":{"enabled":true,"level":"medium"},
        "widening":{"enabled":true,"amount":1}}},
 "sleepTimer":null,"wave":{"active":true},"journalSeq":311}
```

```json
{"rev":13,"order":["q1a","q1b","q1c"],"original":null,"index":0,"shuffle":false,"repeat":"off",
 "continuation":{"kind":"wave","fetching":false},"added":[{"uid":"q1b","…":"…"},{"uid":"q1c","…":"…"}],"updated":[]}
```

## 4. Rust design (work package RUST)

### 4.1 `crates/player` (new; no dependency on Tauri, ytm or wave)

```
crates/player/
  Cargo.toml        gstreamer, gstreamer-base, gstreamer-audio (gstreamer-rs 0.24.x, feature v1_22),
                    tokio, futures-util, serde, serde_json, thiserror, tracing, realfft, rand, arc-swap
  src/lib.rs        Player::spawn(hooks, persisted) -> PlayerHandle; PlayerHandle (Clone): async methods
                    mirroring §3.3 + subscribe() -> broadcast::Receiver<PlayerEvent>; init_gstreamer()
  src/model.rs      §3.2 types, serde camelCase (THE contract) + fixture tests from §3.7
  src/queue.rs      pure Queue: entries by uid, original/shuffled orders, index, repeat, all ops,
                    next_candidate(pred)/prev_candidate(pred), remaining_after(); unit tests port
                    toggleShuffle/addNext/remove/move semantics from ui/js/player.js
  src/tracker.rs    PlayTracker port of ui/js/wave-logic.js (start once trusted position > 0.5 s;
                    end on switch/stop/end; history once ≥ 10 s) emitting PlayEvent; unit tests
  src/engine.rs     the actor (§4.2)
  src/deck.rs       one playbin3 + audio-filter bin + sink (§4.2)
  src/dsp/mod.rs    `volnadsp` GstBaseTransform subclass, registered statically by init_gstreamer()
  src/dsp/biquad.rs RBJ biquads, TDF-II, f64 state; response tests vs ui `_biquadResponseDb`
  src/dsp/chain.rs  preamp → EQ (stereo or M/S) → GEQ → mono → crossfeed → widening; ArcSwap params
  src/analysis.rs   tap ring buffer + FFT → VisFrame bytes (§3.5); runs only when subscribed
  src/persist.rs    load/save of queue state (JSON) — paths supplied by the caller
  src/hooks.rs      Hooks trait (§4.4)
  examples/play.rs  `cargo run -p volna-player --example play -- <uri>...` (EQ/crossfade by flags)
```

### 4.2 Engine

- **Actor:** one tokio task owns `Queue`, two `Deck`s, `PlayTracker`, settings, the sleep
  timer and the continuation. Inputs: `mpsc::UnboundedReceiver<Command>` (each with a oneshot
  reply), the deck bus streams (`bus.stream()`, never `add_watch`, because Windows has no GLib
  main loop), and tokio timers (position tick 250 ms, crossfade scheduler, sleep, persistence
  debounce). Output: `broadcast::Sender<PlayerEvent>` (State, Position, Queue, Error, Settings,
  Sleep, Wave, QueueLow, Played(PlayEvent)) and `watch` for the latest Snapshot.
- **Deck:** a `playbin3` with `audio-sink` = `autoaudiosink`. On Linux the child
  pulsesink/pipewiresink gets `stream-properties` `media.role=Music,application.name=Volna,application.icon-name=volna,node.name=volna`.
  The `audio-filter` bin is
  `scaletempo (only when preservePitch && speed≠1) ! audioconvert ! audioresample ! capsfilter audio/x-raw,format=F32LE,layout=interleaved,channels=2,rate=48000 ! volnadsp ! volume name=fade ! volume name=master`.
  As built: `audioconvert ! scaletempo (whenever preservePitch; passthrough at 1×) ! audioconvert ! audioresample ! capsfilter ! volnadsp`,
  with the fade (sample-accurate ramps) and the smoothed master gain inside `volnadsp`, after the analysis tap.
  `master` = curve(volume) × (muted?0:1). `fade` is for crossfade, sleep fade and a 30 ms
  de-click on manual skip/pause. Changing preservePitch rebuilds the bin when the next track
  loads, or right away by reloading at the current position.
- **Load:** `uri = hooks.stream_uri(track, quality)`, deck → PAUSED (preroll, 45 s timeout →
  failure `timeout`), then seek if a start position is set, → PLAYING. Position = `query_position`
  of the playbin (sink-latency aware, good for lyrics). Duration = `query_duration`, else
  `track.duration`.
- **Rate:** a seek with rate (`SeekFlags::FLUSH|ACCURATE`) at the current position.
- **Gapless (crossfade off):** on `about-to-finish`, set `uri` to the next candidate.
  `StreamStart` on the bus means the next track began: advance the index and emit
  Queue/State/Position, and feed the tracker. With repeat one, set the same uri.
- **Crossfade:** when `remaining ≤ seconds` and a next candidate exists, repeat ≠ one, and both
  durations are ≥ 2·seconds: load the standby deck to PAUSED. If prerolled in time, PLAYING
  with fade=0, then ramp over `seconds` (10 ms steps): old `cos(πt/2)`, new `sin(πt/2)`. At t=1,
  stop the old deck (→ NULL), the standby becomes active, advance. If preroll fails or is late,
  give up and let gapless/EOS handle the transition. `state.crossfading` is true during the
  ramp. Explicit next/prev never crossfades.
- **EOS** on the active deck without a pending transition → advance per §3.3 `player_next`
  rules (continuation awaited when 0 entries remain; if that fails → status `ended`).
- **Prefetch:** at each track start, `hooks.prefetch(next_candidate)`.
- **Continuation:** at each track start, and after queue mutations, if `kind != none` (or
  autoplay) and `remaining_after < 3` and not already fetching → `hooks.more(...)` for 5 tracks,
  appended with the matching origin (dedup against the queue and the last 100 played ids). For
  `ui`, emit `QueueLow` instead.
- **Wave rules** (port of wave.js): a track with origin ≠ wave starting while the wave is active
  → wave stops (`hooks.wave_stop()`, event Wave{active:false}). `PlayEvent` outcome
  `requeue_wave` → remove upcoming wave-origin entries after the current one, then
  `hooks.more(WaveRequeue{dropped, count:3})`, then append.
- **Failures:** bus Error on a deck → if the error came at position > 5 s and is the first for
  this uid, reload at that position once (`action: 'retrying'`). Otherwise
  `hooks.failure(id)` → kind. `no_yt_dlp` → stop (`stopped`). Else the SkipGuard port (streak < 3)
  → mark `unavailable` if kind is `unavailable`, wait 1.2 s, top up the continuation, then next
  (`skipped`). Streak 3 → stop (`stopped`, streak reset). A successful start resets the streak.
- **Blocklist:** `next_candidate` skips entries whose id, any artist id or album id is in
  `settings.blocklist` (same as `contentBlockingSettings.shouldHideTrack`), and entries marked
  `unavailable`.
- **Persistence:** queue state (items with `ui`/`raw` blobs, orders, index, repeat, continuation
  incl. radio seeds, current position) saved 2 s after a change, every 10 s while playing, on
  pause and on shutdown. On startup it is restored as `paused` at the saved position; nothing
  autoplays.
- **Shutdown:** `PlayerHandle::shutdown()` saves state, sets decks to NULL and closes the tracker
  (emits End).

### 4.3 `volnadsp` element

- BaseTransform, in-place, F32LE interleaved stereo at 48 kHz (fixed by the capsfilter). Params
  are an `ArcSwap<Arc<Chain>>`, rebuilt from `DspConfig` in the actor and swapped in. Coefficient
  changes are interpolated over 20 ms (per-64-sample recompute during ramps); band-count or M/S
  topology changes crossfade old/new chains over 20 ms. Filter state is kept on a pure gain change.
- Chain order matches audio-context.js `_connectGraph`: mono → binaural (crossfeed, widening) →
  [EQ preamp → EQ (stereo, or M/S: M=(L+R)/2, S=(L−R)/2, mid bands skip channel 'side', side bands
  skip 'mid', decode L=M+S, R=M−S)] → [GEQ preamp → GEQ] → tap for analysis → out.
- Biquads: RBJ cookbook; **shelves use Q** (alpha = sin w0 / 2Q), exactly like
  `_biquadResponseDb` in audio-context.js and the AutoEQ engine (WebAudio ignores Q for shelves;
  this is deliberate, §8). Freq clamped to < sr/2 − 1.
- Crossfeed presets from binaural-dsp.js (`low 500 Hz/-6 dB/0.2 ms`, `medium 700/-4.5/0.3`,
  `high 1000/-3/0.4`): 1-pole lowpass on the cross path + delay + gain. Widening: M/S with side
  gain = amount, mid gain compensation as `_calcMidGain/_calcSideGain` in binaural-dsp.js.
- Analysis tap: when `analysis_enabled` (AtomicBool) is set, copy the samples into an SPSC ring.
  Otherwise there is zero cost.

### 4.4 `Hooks` trait (implemented in src-tauri)

```rust
pub trait Hooks: Send + Sync + 'static {
    fn stream_uri(&self, track: &PlayerTrack, quality: Quality) -> Option<String>;
    fn prefetch(&self, track: &PlayerTrack, quality: Quality);
    fn failure(&self, track_id: &str) -> impl Future<Output = FailureInfo> + Send;          // kind + message
    fn stream_info(&self, track_id: &str, quality: Quality) -> impl Future<Output = Option<StreamInfo>> + Send;
    fn more(&self, req: MoreRequest) -> impl Future<Output = Result<Vec<TrackInput>, String>> + Send;
    fn on_play_event(&self, ev: PlayEvent) -> impl Future<Output = PlayEventOutcome> + Send; // wave.record + journal
    fn wave_stop(&self) -> impl Future<Output = ()> + Send;
}
pub enum MoreRequest { WaveStart{count}, WaveNext{count}, WaveRequeue{dropped: Vec<String>, count},
                       Radio{seeds: Vec<PlayerTrack>, exclude: HashSet<String>, count},
                       Autoplay{current: PlayerTrack, exclude: HashSet<String>, count} }
pub enum PlayEvent { Start{track, in_wave, at}, History{track, at},
                     End{track, started_at, position, duration, ended, in_wave} }
pub struct PlayEventOutcome { pub requeue_wave: bool }
```

`Engine<H: Hooks>` is generic (same style as `volna_wave::CandidateSource`). Tests use a fake
`Hooks` plus `audiotestsrc`/file URIs.

### 4.5 src-tauri glue

```
src-tauri/src/player/mod.rs        PlayerState {handle, journal, store}; setup(app, &AppState, &WaveState)
src-tauri/src/player/commands.rs   §3.3 commands (+ app_quit, app_ready_to_close)
src-tauri/src/player/forward.rs    broadcast → app.emit_to("main", name, payload) if the window exists
src-tauri/src/player/hooks.rs      Hooks: stream_uri = format!("{stream_base}/{id}{q}"), prefetch/failure/stream_info via
                                   AppState.streams (resolve() is cached → codec/bitrate), more(): wave.start/next/requeue,
                                   ytm.radio(seed|current) + filtering; on_play_event: Start/End → Wave::record (PlayerTrack →
                                   volna_ytm::Track as UiTrack::from does), History/End → journal
src-tauri/src/player/convert.rs    volna_ytm::Track → TrackInput {origin, raw: shape::track(..), cover: shape::cover(..)}
src-tauri/src/player/journal.rs    JSONL at <app_data_dir>/player-journal.jsonl; seq monotonic; ack rewrites; cap 5000
src-tauri/src/player/store.rs      <app_config_dir>/player.json {version:1, settings, dsp}; atomic write like settings.rs;
                                   queue file <app_data_dir>/player-queue.json (via crates/player persist)
src-tauri/src/player/window.rs     show_or_create(), close flow, quit flow (below)
src-tauri/src/player/tray.rs       Tauri tray (feature "tray-icon"): items «Сейчас: Title — Artist» (disabled), «Пауза»/«Играть»,
                                   «Следующий», «Предыдущий», separator, «Показать Volna», «Выход». Tooltip = now playing.
                                   Left click (Windows) = show. Labels/tooltip updated from player events.
src-tauri/src/player/mpris.rs      #[cfg(target_os="linux")] mpris-server `Server` with RootInterface + PlayerInterface impl over
                                   PlayerHandle; bus name org.mpris.MediaPlayer2.volna; Identity "Volna"; DesktopEntry "volna";
                                   CanRaise (→ show_or_create) / CanQuit; LoopStatus None/Track/Playlist ↔ off/one/all; Shuffle;
                                   Volume; Rate (0.25..4); Position on demand; Seeked signal on seek/jump.
                                   Metadata: mpris:trackid=/uz/khudoberdi/volna/track/<uid>, mpris:length (µs), xesam:title/artist/album,
                                   xesam:url=https://music.youtube.com/watch?v=<id>, mpris:artUrl=file://<path>
src-tauri/src/player/visualizer.rs Channel subscription: enables the analysis flag, drops it on send error/stop
```

- **Cover art:** take `u` from `track.cover` (`volna://img/?u=…` or `http://volna.localhost/img/?u=…`)
  and call the new `ImageCache::get_path(url, Some(544)) -> Result<PathBuf>` in
  `crates/streams/src/image.rs` (fetch if missing, return the cache file path) → `file://…`.
  If a desktop shell refuses an extensionless file, copy it to `<cache>/mpris/<hash>.jpg`.
- **wave.rs:** add `pub fn engine(&self) -> Arc<Wave<YtmSource>>` and `pub fn ytm(&self)` (make
  the existing private helper public). Commands stay.
- **lib.rs:** `mod player;` set up after `WaveState`; register the commands; `PlayerState`
  managed. Window handling:
  - `on_window_event`: `WindowEvent::CloseRequested{api}` on "main" → `api.prevent_close()`,
    emit `app:will-close`, wait for `app_ready_to_close` or 700 ms, then `window.destroy()`.
    Keep no `WebviewWindow` clones alive.
  - `RunEvent::ExitRequested{code: None, api}` while not quitting → `api.prevent_exit()`.
  - Quit (tray, MPRIS, `app_quit`): set `QUITTING`, `player.shutdown().await`, journal flush,
    `app.exit(0)`. The existing `RunEvent::Exit` still stops xray.
  - `show_or_create`: existing window → `show/unminimize/set_focus`; else
    `WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?.build()`. The label stays
    `main`, so the capability and the `volna-stream` init script apply. The page boots and runs §3.6.
- **Single instance:** `tauri-plugin-single-instance` → `show_or_create` on a second launch.
  `VOLNA_MULTI_INSTANCE=1` skips it so dev and test copies can run next to an installed
  Volna; MPRIS then falls back to `org.mpris.MediaPlayer2.volna.instance<pid>`.
- **WebKit processes (as built):** WebKitGTK keeps a closed page's WebProcess in its process
  cache and the NetworkProcess with the web context. Close therefore calls
  `webkit_web_view_terminate_web_process` before `destroy()`, and 2 s later, with no window
  left, ends our `WebKitNetworkProcess` child; WebKit starts new ones for the next window.
- **GStreamer init:** `volna_player::init_gstreamer()` early in `setup`. On failure, log and run
  without a player (commands return `unavailable:`).

### 4.6 Dependencies, packaging, Windows

- Workspace `Cargo.toml`: add `crates/player` to `members` and `volna-player` to
  `[workspace.dependencies]` (the only edit to that list).
- `src-tauri/Cargo.toml`: `tauri` features `["tray-icon"]`, `volna-player`, `mpris-server` (Linux
  target only), `tauri-plugin-single-instance`.
- Linux runtime: GStreamer base (playback, audioconvert, audioresample, volume, typefind, opus),
  good (soup, matroska, isomp4, autodetect, pulse, audiofx/scaletempo), libav (AAC fallback).
  The deb already depends on base/good and recommends libav; PKGBUILD already depends on
  base/good/libav. The Linux tray is `ksni` (pure D-Bus), so no `libayatana-appindicator` is needed;
  PKGBUILD adds `gstreamer` and bumps `pkgrel`.
  AppImage keeps `bundleMediaFramework: true`.
- Windows (secondary, must not block Linux): CI installs the GStreamer MSVC runtime + devel MSIs
  (pkg-config for gstreamer-rs). The installer either chains the official runtime MSI silently
  or bundles DLLs + `lib/gstreamer-1.0` with `GST_PLUGIN_SYSTEM_PATH`/`GST_PLUGIN_PATH` set
  before `gst::init`. Plugins: coreelements, playback, typefindfunctions, audioconvert,
  audioresample, volume, autodetect, wasapi2, soup (+libsoup-3; plain HTTP loopback, so no
  glib-networking needed), matroska, isomp4, opus, audioparsers, scaletempo, libav (AAC). No MPRIS
  there: implement SMTC later behind the same `OsMediaControls` seam (it needs an HWND; use a
  hidden message-only window because the main window can be destroyed). The tray works as-is.

## 5. Frontend design (work package UI)

### 5.1 New modules

- `ui/js/player-bridge.js`: wraps `invoke`/`listen`/`Channel`. Serial FIFO for all mutating
  `player_*` calls. Exports `createBridge()` → `TauriBridge`, or `MockBridge` when
  `!window.__TAURI_INTERNALS__` or `localStorage['volna-mock-player']==='1'`. MockBridge
  implements §3.3/§3.4 in memory (queue via player-mirror, fake 4 Hz ticks, synthetic visualizer
  frames) so the UI can be developed in `bun run dev` and vitest without Rust.
- `ui/js/player-mirror.js` (pure, unit-tested): applies `QueueSnapshot`/`QueueChanged` (rev
  rule), keeps `uid → rich track`. For each `PlayerTrack`: `rich = pt.ui ?? api.prepareTrack(pt.raw)`;
  attaches `rich._uid`, `rich._origin`, `rich.isUnavailable = pt.unavailable`. Exposes
  `order`, `index`, `currentTrack`, `playQueue` (rich, play order), `originalQueue`.
  Also `toTrackInput(rich)` (id, title, artists, album, duration, cover, explicit, kind, ui: rich).
- `ui/js/remote-media.js`: `class RemoteMediaElement extends EventTarget`, an HTMLMediaElement
  look-alike (`id='audio-player'`):
  - getters: `currentTime` (interpolated `position + (Date.now()-at)/1000*rate` while playing,
    capped at duration), `duration` (NaN if null), `paused` (status not in playing|loading),
    `ended`, `volume`, `muted`, `playbackRate`, `readyState` (4 when uid && duration else 0),
    `src`/`currentSrc` (`'volna-player:'+uid` or `''`), `error` (`{code: 4, message}` | null);
  - setters: `currentTime` → seek, `volume` → `player.setVolume`, `muted` → settings patch,
    `playbackRate` → speed;
  - `play()` → Promise (player_play), `pause()`, `load()` no-op;
  - dispatches: uid change → `emptied`, `loadstart`; duration known → `durationchange`, `loadedmetadata`;
    loading → `waiting`; playing → `play` + `playing`; paused → `pause`; each position event → `timeupdate`;
    seek → `seeking` + `seeked`; status ended → `ended` (queue finished only); error → `error`;
    volume/muted → `volumechange`; rate → `ratechange`;
  - `exact-seek-request` (lyrics) is handled by the default action: `dispatchEvent` returns true,
    then the lyrics code sets `currentTime`, which seeks.

### 5.2 `Player` (ui/js/player.js rewritten as a remote; same import and class name)

Kept and implemented through the bridge/mirror. Behaviour stays the same unless noted:

| Member | Implementation |
|---|---|
| `static initialize(audioEl, api, quality)`, `static instance` | audioEl ignored; runs §3.6 |
| `audio`, `activeElement` | the single `RemoteMediaElement`; `audioElements = [it]`; `video = null` |
| `currentTrack`, `currentQueueIndex`, `shuffleActive`, `repeatMode` (0/1/2), `queue`, `shuffledQueue`, `getCurrentQueue()`, `getNextTrack()` | mirror getters |
| `userVolume`, `quality`, `isLoadingTrack` (status loading), `currentStreamInfo` (`state.stream` mapped to `{codec, bitrateKbps, quality, qualityDisplay}`) | state getters |
| `handlePlayPause`, `playNext`, `playPrev`, `seekTo(t,{resume})`, `seekForward/Backward` | commands |
| `playAtIndex(i)` | `player_play_uid(order[i])` |
| `playTrackFromQueue(startTime=0)` | `player_play_uid(currentUid, startTime)` |
| `setQueue(tracks, startIndex, isRadio)` | `player_set_queue({tracks: map(toTrackInput), startIndex, play:false})`. Call sites follow it with playAtIndex/playTrackFromQueue, so keep `play:false`. `!isRadio` → also `disableRadio()` locally |
| `addToQueue`, `addNextToQueue`, `removeFromQueue(i)`, `moveInQueue(from,to)`, `clearQueue`, `wipeQueue` | enqueue end/next, remove/move by `order[i]`, clear keepCurrent true/false |
| `toggleShuffle`, `toggleRepeat` (returns the new 0/1/2) | `player_set_shuffle/repeat` |
| `setVolume(v)`, `setPlaybackSpeed`, `setPreservePitch`, `setQuality`, `applyReplayGain` | `player_set_settings` patches (+ keep the localStorage mirrors written as today) |
| `enableRadio(seeds)`, `disableRadio`, `radioEnabled` | seeds via the existing `pickRadioSeeds` (IndexedDB), then set_queue + `player_set_continuation({kind:'radio', seeds})`; disable → `{kind:'none'}` |
| `enableAutoplay/disableAutoplay`, `autoplayEnabled` | settings patch `autoplay` |
| `setArtistPopularTracksContext/clearArtistPopularTracksContext` | local state + `player_set_continuation({kind:'ui'})`; on `player:queue-low` fetch `getArtistTopTracks` and `addToQueue` |
| `setSleepTimer(min)`, `clearSleepTimer`, `getSleepTimerRemaining`, `isSleepTimerActive`, `updateSleepTimerUI` | command + local 1 s countdown from `endsAt` |
| `playVideo(v)` | as setQueue([v]) + play (plays as audio) |
| `updatePlayingTrackIndicator`, `updateNowPlayingTitle`, `setLoadingState`, `syncDurationUI`, `resetProgressUI`, `loadAlbumYear` | kept (DOM), driven by events (§5.3) |
| `forceQuality`, `preloadNextTracks`, `checkPreloadConditions`, `saveQueueState`, `updateMediaSession*`, `evaluateCrossCodecAbr` | no-ops kept for callers; settings.js references to `preloadCache`/`getNextCrossfadeCandidate`/`isCrossfadeShakaStream` are removed |

### 5.3 Now-playing rendering

When a new uid arrives (`player:state`) and during boot: render cover/title/album/artist/year,
`document.title`, the playing indicator, `UIRenderer.instance.setCurrentTrack(track)`, and
`resetProgressUI` + `syncDurationUI`. These are the same DOM writes as today's
`playTrackFromQueue`/`loadQueueState`, moved into `renderNowPlaying(track)`. Play/pause icon from
status; spinner while `loading`.

### 5.4 Per-file changes

- **events.js `initializePlayerEvents`:** keep the progress bar/time on `timeupdate`, the icons,
  the scrobbler `updateNowPlaying` on `play` when the uid changes, and `updateWaveform`. Delete:
  the `ended` → playNext handler, the crossfade/silence logic, `db.addToHistory`, the
  listeningTracker calls (moved to §5.7), the `audioContextManager.init/resume` calls and the
  error-code handling. Seekbar drag: do not pause during the drag; seek on release.
  Mute: `player.audio.muted = !muted` (no localStorage write; Rust persists it).
- **app.js:** `audioPlayer` = `Player.instance.activeElement` (remove `getElementById('audio-player')`);
  `handleActiveAudioPlay` stays (shim `play` events). Handle `app:will-close`: flush
  listeningTracker, call `app_ready_to_close`.
- **ui.js:** fullscreen/visualizer code keeps using `this.player.activeElement`; remove
  `audioContextManager.changeSource/init/resume` calls.
- **wave.js:** delete `hookPlayer`, `topUp`, `requeue`, `waveIds`, the `PlayTracker` use and
  `setBeforeAutoSkip`. `start()` → `player_wave_start({count:5})` (`no_candidates` → cold start).
  `deactivate()` → `player_wave_stop`. `active` comes from `snapshot.wave.active` + `player:wave`.
  `isPlaying()` from the shim. Dislike shown when `currentTrack._origin === 'wave' && active`.
  `dislikeCurrent()` → unfavourite in IndexedDB (as today) + `player_wave_dislike`. Keep likes,
  `wave_sync_likes`, card, popover, cold start.
- **wave-logic.js:** remove `PlayTracker`, `needsTopUp`, `remainingAfter`, `upcomingWaveTracks`
  (+ their tests); keep the rest.
- **playback-failures.js:** delete the DOM listeners, `SkipGuard`, `setBeforeAutoSkip`;
  `installPlaybackFailureHandler(player)` listens to `player:error` and shows
  `failureNotice(kind)` (15 s cooldown, the "several in a row… stopped" text when
  `action==='stopped' && streak>=3`). Keep the `failureNotice`/`isFatal` tests.
- **audio-context.js → facade** (keep the export `audioContextManager` and every method used by
  settings.js/ui.js, plus the helpers `generateFrequencies`, `getPresetsForBandCount`,
  `interpolatePreset`, `EQ_PRESETS*`). State lives in plain fields loaded from
  `equalizerSettings`/`monoAudioSettings`/`binauralDspSettings` as now. Every mutator updates the
  fields + localStorage exactly as now, then `scheduleDspPush()` (rAF-debounced
  `player_set_dsp(buildDspConfig())`). `isEQEnabled` becomes a getter/setter that also pushes.
  `buildDspConfig()`: `eq.bands[i] = {freq: frequencies[i], gainDb: currentGains[i], q: currentQs[i] > 0 ? currentQs[i] : _calculateQ(i), type: currentTypes[i] || 'peaking', channel: currentChannels[i] || 'stereo'}`;
  `graphicEq.bands[i] = {freq: geqFrequencies[i], gainDb: geqGains[i], q: 2.5*sqrt(16/geqBandCount)}`;
  binaural from binauralDspSettings; mono. `applyAutoEQBands`, `_biquadResponseDb`, import/export
  unchanged (sample rate fixed at 48000). `getAnalyser()` → `AnalyserShim`;
  `getAudioContext()` → lazily `new OfflineAudioContext(2, 2, 48000)` (butterchurn construction
  only); `getSourceNode()` → null; `isReady()` → true; `init/resume/changeSource/notifyBinauralChannelCount` → no-ops;
  `onGraphChange` → returns a no-op unregister; `setVolume` removed (volume is in Rust);
  `setBinauralHrtfPreset` stores only.
- **visualizer.js:** `init()` subscribes via `player_visualizer_start({channel, fps:60})` in
  `start()` and stops in `stop()` and on `visibilitychange` hidden. `AnalyserShim {fftSize:1024, frequencyBinCount:512, context:{sampleRate:48000}, getByteFrequencyData(a), getByteTimeDomainData(a), latest}`
  fills from the newest frame (zeros if none or older than 250 ms). The volume factor uses 1.
- **visualizers/butterchurn.js:** create with the OfflineAudioContext, never `connectAudio`;
  in `draw` call `this.visualizer.render({ audioLevels: { timeByteArray, timeByteArrayL, timeByteArrayR } })`
  from the latest frame (verify against butterchurn 2.6.7's `render(opts)`). Drop `onGraphChange`.
- **visualizers/kawarp.js:** `audioElement = Player.instance.activeElement`; the analyser comes
  from the shim. lcd/particles/unknown-pleasures need no change.
- **lyrics.js, side-panel.js, commandPalette.js:** replace `getElementById('audio-player')` with
  `Player.instance.activeElement`; otherwise unchanged.
- **settings.js:** remove the crossfade preload poking (≈ lines 5473–5483); crossfade/gapless/
  speed/pitch/quality/replay-gain/exponential-volume/autoplay/content-blocking handlers also
  patch Rust settings (`player.setX` or `player.pushSettings()`); clamp the speed input to 0.25–4;
  hide the HRTF preset select, silence-removal and "native OS Atmos" options.
- **db.js:** `addToHistory(track, {timestamp} = {})`: use the given timestamp (bumped by +1 ms
  while it collides) instead of `Date.now()`; same dedupe.
- **listening-tracker.js:** add `recordPlay(track, playedSeconds, durationSeconds, ended)` used by
  the journal consumer (wraps onTrackStart/onTimeUpdate/onTrackEnd/updateArtistAffinity semantics).
- **lastfm.js, librefm.js, listenbrainz.js, maloja.js, multi-scrobbler.js:**
  `scrobbleAt(track, unixSeconds)`; disable the timer-based scrobble (keep now-playing).
- **storage.js:** `queueManager` used only for the one-time migration; add `journalCursor` get/set.
- **index.html:** remove `<audio id="audio-player">` (and `#video-player` uses in player code).
- **package.json:** drop `shaka-player`, `hls.js`, `@capgo/capacitor-media-session`.

### 5.5 Delete

`ui/js/equalizer.js` (already unused), `ui/js/binaural-dsp.js`, `ui/js/hrtf-generator.js`, all
Shaka/HLS/crossfade/preload/MediaSession/Web Audio code in player.js, and `replay-gain.js` once
nothing imports it (keep its test only if the module stays).

### 5.6 EQ UI mapping (summary)

UI action → facade method (unchanged names) → localStorage → `player_set_dsp(full config)`.
Presets, AutoEQ fitting, band count/range changes, text import/export, M/S channel picks: all
computed in UI; Rust only sees explicit bands. Graphic EQ vs parametric EQ exclusivity
(settings.js 3733/5176) is just the two `enabled` flags.

### 5.7 Journal consumer (`ui/js/player-journal.js`)

For each entry in seq order:
- `history` → `db.addToHistory(rich(track), {timestamp: entry.at})`; refresh Recent if open.
- `end` → `listeningTracker.recordPlay(...)`; scrobble when `duration > 30 && position >= min(duration*pct, 240)`
  (pct from `lastFMStorage.getScrobblePercentage()`) → `scrobbler.scrobbleAt(track, startedAt/1000)`,
  only when `entry.at` is within 14 days.
Then advance the cursor and ack (§3.6). The live window uses the same path, so there is one code
path and no double counting.

### 5.8 Migration (once, when `snapshot.settings.migrated === false`)

Build a settings patch from localStorage (`volume`, `muted`, `exponential-volume-enabled`,
`playback-quality`, `crossfade-*`, `gapless-playback-enabled`, `audio-effects-*`, `replay-gain-*`,
`autoplay-enabled`, `blocked-*` with the hardcoded ids, `migrated:true`) and a `DspConfig` from the
EQ keys. If `snapshot.queue.items` is empty and `monochrome-queue` exists: `player_set_queue`
(current order, `startIndex`, `play:false`) + shuffle/repeat. Then remove `monochrome-queue`.

## 6. Work packages and file ownership (no file is touched by both)

### 6.1 RUST

Owns: `Cargo.toml` (members + workspace deps), `crates/player/**` (new), `crates/streams/src/image.rs`
(`get_path`), `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`, `src-tauri/src/player/**` (new),
`src-tauri/src/wave.rs` (accessors only), `src-tauri/tauri.conf.json` (only if needed),
`src-tauri/capabilities/default.json` (only if needed), `packaging/arch/PKGBUILD`,
`.github/workflows/build.yml`, `docs/streams.md` (a cross-reference only). Do not touch the
uncommitted files `src-tauri/src/api_emu/*`, `src-tauri/src/yandex.rs`.

Order: R1 model/queue/tracker + tests (fixtures from §3.7) → R2 deck/engine without DSP, commands,
events, persistence, journal → R3 window lifecycle, tray, MPRIS, cover path, single instance →
R4 hooks: wave/radio/autoplay, failures/skip → R5 crossfade, volnadsp, analysis + Channel →
R6 packaging/CI. Until UI lands, test with `examples/play.rs`, `playerctl`, the tray, and
`window.__TAURI_INTERNALS__.invoke` from devtools.

### 6.2 UI

Owns everything under `ui/` except the uncommitted `ui/js/api.js`, `ui/js/playlist-importer.js`,
`ui/js/yandex-import.js` (read them, don't edit them). New: `player-bridge.js`,
`player-mirror.js`, `remote-media.js`, `player-journal.js` + tests. Changed: §5.4 files.

Order: U1 bridge + mirror + shim + MockBridge + tests → U2 Player remote + now-playing render →
U3 events/app/ui/lyrics/side-panel/commandPalette → U4 wave.js/playback-failures → U5 audio-context
facade + settings.js → U6 visualizer shim + butterchurn/kawarp → U7 journal consumer, db, scrobblers,
listening tracker → U8 migration, deletions, package.json.

### 6.3 Capabilities

App-defined commands need no capability entries (default `tauri_build::build()`); `core:default`
covers event listen/unlisten and channels. Keep the window label `main` on recreation so
`default.json` (`"windows": ["main"]`) still applies. The tray, MPRIS and single-instance are
Rust-only (no JS permissions). Add `window-state:default` only if `tauri-plugin-window-state`'s JS
API gets used.

## 7. Integration checklist

1. Command and event names: `grep -o "player_[a-z_]*\|player:[a-z-]*\|app[:_][a-z_-]*"` on both
   sides matches §3.3/§3.4.
2. The §3.7 fixtures deserialize in `crates/player` tests and are applied by `player-mirror` tests.
3. Rapid `setQueue` → `playAtIndex` → `addNextToQueue` keeps order (FIFO + actor).
4. Close the window: WebKit processes exit; `ps -o rss= -C volna` ≤ 60 MB after 30 s; no
   `player:position`/visualizer traffic.
5. Tray "Show" and `playerctl -p volna open`/Raise recreate the window; it shows the current track,
   position within 0.3 s, identical queue, EQ state; Recent contains plays from the closed period.
6. Wave runs ≥ 30 min with the window closed; the queue refills; a reseed (2 early skips) replaces
   queued wave tracks.
7. `playerctl metadata` shows `mpris:artUrl` `file://…` that exists; play-pause/next/previous/
   position/volume/loop/shuffle work; KDE/GNOME widgets show the cover.
8. Gapless on an album; crossfade 5 s audible with no double advance; explicit Next is immediate.
9. AutoEQ profile + M/S bands, mono, crossfeed, widening audible and glitch-free while dragging
   sliders.
10. All five visualizer presets react; frames stop when the visualizer closes or the window hides.
11. Lyrics line highlighting in sync; clicking a line seeks.
12. bot_check → notification + skip, 3 in a row → stop; no_yt_dlp → stop.
13. Tray Quit / `app_quit` exits the process (xray stopped, queue saved, position restored on next start).
14. Old localStorage queue/EQ migrate once.
15. `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`;
    `cd ui && bun run test:node && bun run build`; `makepkg -f` with the bumped `pkgrel`.

## 8. Decisions taken here (flag if you disagree) and open questions

- Linux tray: Tauri's tray (libayatana-appindicator) delivers no click events on Linux.
  Left-click opens the menu and "Show" is the first item. Alternative: `ksni` for left-click =
  show. **Open.**
- Scrobbles for plays made while the window is closed are submitted when it reopens, with the
  original timestamps (§5.7). **Open:** or skip them.
- Dropped: multichannel HRTF, silence removal, waveform data, local files/podcasts/tracker
  playback. Speed limited to 0.25–4. ReplayGain stored but no-op until loudness data exists
  (possible source: InnerTube `playerConfig.audioConfig.loudnessDb`). **Open.**
- Monochrome radio/autoplay refill in Rust via YouTube Music radio. This loses
  smart-recommendations filtering and the IndexedDB "known tracks" exclusion. **Open.**
- With repeat one, Next advances (today it replays). Shelf filters honour Q (today WebAudio
  ignores it). **Open.**
- Close always goes to the tray; quit via tray/MPRIS/`app_quit`.

### 8.1 Resolved (2026-10-17)

- Linux tray: use `ksni` (StatusNotifierItem) so that left click shows/creates the window and
  the menu stays on right click. Tauri's `tray-icon` is still used on Windows. Hide this behind
  one `tray` module with `#[cfg]`.
- Scrobbles from plays made while the window was closed: submit them when the window reopens,
  with the original timestamps (§5.7).
- Dropped features: accepted as listed (HRTF multichannel, silence removal, waveform,
  local files/podcasts/tracker, speed 0.25–4, ReplayGain stored as a no-op).
- Radio/autoplay refill through YouTube Music radio in Rust: accepted.
- Repeat one + Next advances; shelf bands honour Q: accepted.
- `docs/STRUCTURE.md`: adding `crates/player` to the workspace members is allowed.
