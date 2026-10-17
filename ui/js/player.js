// js/player.js
// The player as a remote control for the native player in Rust
// (docs/native-player.md §5.2). Audio, the queue, shuffle/repeat, crossfade,
// the sleep timer and the wave/radio refill all live in crates/player; this
// class keeps the API the rest of the UI was written against, mirrors the
// native state (player-mirror.js, remote-media.js) and renders the
// now-playing bar.

import {
    REPEAT_MODE,
    formatTime,
    getTrackArtists,
    getTrackTitle,
    getTrackArtistsHTML,
    getTrackYearDisplay,
    createQualityBadgeHTML,
    escapeHtml,
    formatQualityBadgeText,
    normalizeQualityToken,
} from './utils.js';
import {
    trackDateSettings,
    radioSettings,
    autoplaySettings,
    audioEffectsSettings,
    replayGainSettings,
    exponentialVolumeSettings,
    crossfadeSettings,
    gaplessPlaybackSettings,
    contentBlockingSettings,
    queueManager,
} from './storage.js';
import { db } from './db.js';
import { SVG_CLOCK, SVG_PLAY, SVG_PAUSE } from './icons.js';
import { UIRenderer } from './ui.js';
import { createBridge, parseError, PLAYER_EVENTS, SKIP } from './player-bridge.js';
import { PlayerMirror, isPlayableTrack, toTrackInput } from './player-mirror.js';
import { RemoteMediaElement } from './remote-media.js';
import { audioContextManager } from './audio-context.js';
import {
    buildQueueMigration,
    buildSettingsPatch,
    clampSpeed,
    modeToRepeat,
    repeatToMode,
    storedVolume,
} from './player-settings.js';

const SETTINGS_SOURCES = {
    replayGainSettings,
    exponentialVolumeSettings,
    crossfadeSettings,
    gaplessPlaybackSettings,
    audioEffectsSettings,
    autoplaySettings,
    contentBlockingSettings,
};

/** Rust replies `{rev}`; the matching player:queue event should follow within this time. */
const REV_WAIT_MS = 1500;
/** State events keep the slider's volume for a moment after the last change was sent. */
const VOLUME_QUIET_MS = 400;
const RADIO_SEEDS = 50;

const SPINNER_20 =
    '<svg class="animate-spin" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 12a9 9 0 1 1-6.219-8.56"/></svg>';
const SPINNER_32 =
    '<svg class="animate-spin" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 12a9 9 0 1 1-6.219-8.56"/></svg>';

const IDLE_STATE = Object.freeze({
    status: 'idle',
    uid: null,
    trackId: null,
    position: 0,
    at: 0,
    duration: null,
    rate: 1,
    buffering: false,
    crossfading: false,
    volume: 0.7,
    muted: false,
    stream: null,
    error: null,
});

function uiRenderer() {
    try {
        return UIRenderer.instance;
    } catch {
        return null;
    }
}

function warn(what) {
    return (e) => console.warn(`[player] ${what} failed:`, e);
}

export class Player {
    static #instance = null;

    static get instance() {
        if (!Player.#instance) {
            throw new Error('Player is not initialized. Call Player.initialize(audioElement, api) first.');
        }
        return Player.#instance;
    }

    /** @private */
    constructor(api, quality = 'HIGH', { bridge = null } = {}) {
        this.api = api;
        this.quality = normalizeQualityToken(quality) || quality;
        // Read before anything mirrors the native volume into localStorage (§5.8).
        this._legacyVolume = storedVolume();
        this.bridge = bridge;
        this.mirror = new PlayerMirror({ prepare: (raw) => this.api.getAPI().prepareTrack(raw) });
        this.media = new RemoteMediaElement(this);
        this.audio = this.media;
        this.audioElements = [this.media];
        this.video = null;

        this.state = { ...IDLE_STATE };
        this.settings = null;
        this.dsp = null;
        this.waveActive = false;
        this.journalSeq = 0;
        this.sleepTimerEndTime = null;
        this.sleepTimerInterval = null;

        this._listeners = new Map();
        this._syncing = false;
        this._syncPromise = null;
        this._buffer = [];
        this._expectRev = 0;
        this._revWaiters = [];
        this._deferredState = null;
        this._renderedUid = undefined;
        this._optimistic = {};
        this._volume = this.state.volume;
        this._muted = false;
        this._volumeQueued = false;
        this._volumeInFlight = 0;
        this._volumeQuietUntil = 0;
        this._radioWasOn = false;
        this._fetchingShown = false;

        this.artistPopularTracksState = {
            artistId: null,
            offset: 0,
            initialTracks: [],
            isFetching: false,
            hasMore: false,
        };
    }

    /** The first argument (the old `<audio>` element) is ignored. */
    static async initialize(_audioElement, api, quality, options = {}) {
        if (Player.#instance) {
            throw new Error('Player is already initialized');
        }
        const player = new Player(api, quality, options);
        await player.init();
        Player.#instance = player;
        return player;
    }

    async init() {
        this.bridge ??= await createBridge();
        this.isNative = this.bridge.kind === 'tauri';
        await this._attach();
        // EQ, mono and binaural edits in settings go to Rust as DspConfig.
        audioContextManager.setDspSink((config) => this.setDsp(config));
        if (this.settings && this.settings.migrated === false) await this._migrate();

        window.addEventListener('content-blocking-changed', () => this.pushSettings({ blocklist: true }));
        if (import.meta.env?.DEV) window.volnaPlayer = this;
    }

    // ---- events from the native player (§3.4, §3.6) --------------------------

    /** Subscribe to `state`, `queue`, `error`, `journal`, `wave`, `settings`, `sleep`, `queue-low`, `will-close`, `snapshot`. */
    on(name, fn) {
        if (!this._listeners.has(name)) this._listeners.set(name, new Set());
        this._listeners.get(name).add(fn);
        return () => this._listeners.get(name)?.delete(fn);
    }

    _emit(name, payload) {
        for (const fn of this._listeners.get(name) || []) {
            try {
                fn(payload);
            } catch (e) {
                console.error(`[player] ${name} listener failed`, e);
            }
        }
    }

    async _attach() {
        // Rust answers {rev} for queue mutations; record it before the next
        // call in the FIFO evaluates its arguments.
        this.bridge.onReply = (_cmd, r) => {
            if (r && typeof r.rev === 'number') this._expectRev = Math.max(this._expectRev, r.rev);
        };
        // §3.6 step 1: listen first and buffer until the snapshot is applied.
        this._syncing = true;
        await Promise.all(PLAYER_EVENTS.map((name) => this.bridge.on(name, (p) => this._receive(name, p))));
        await this.resync();
    }

    _receive(name, payload) {
        if (this._syncing) {
            this._buffer.push([name, payload]);
            return;
        }
        this._handle(name, payload);
    }

    _handle(name, p) {
        switch (name) {
            case 'player:state':
                return this._onState(p);
            case 'player:position':
                return this._onPosition(p);
            case 'player:queue':
                return this._onQueue(p);
            case 'player:error':
                return this._emit('error', p);
            case 'player:journal':
                return this._emit('journal', p);
            case 'player:queue-low':
                return this._onQueueLow(p);
            case 'player:settings':
                return this._onSettings(p);
            case 'player:sleep':
                return this._onSleep(p?.endsAt ?? null);
            case 'player:wave':
                return this._onWave(!!p?.active);
            case 'app:will-close':
                return this._emit('will-close', p);
        }
    }

    /** Takes a new snapshot and replays what arrived meanwhile (§3.6 steps 2–4). */
    resync() {
        if (this._syncPromise) return this._syncPromise;
        this._syncing = true;
        this._syncPromise = (async () => {
            let snap = null;
            try {
                snap = await this.bridge.direct('player_snapshot');
            } catch (e) {
                console.error('[player] native player unavailable:', e);
                this.unavailable = parseError(e);
            }
            if (snap) this._applySnapshot(snap);
            const buffered = this._buffer;
            this._buffer = [];
            this._syncing = false;
            this._syncPromise = null;
            const at = snap?.state?.at ?? 0;
            for (const [name, p] of buffered) {
                if (name === 'player:queue' && p.rev <= this.mirror.rev) continue;
                if ((name === 'player:state' || name === 'player:position') && p.at <= at) continue;
                // Latest-value events: the snapshot already has them.
                if (snap && ['player:settings', 'player:sleep', 'player:wave'].includes(name)) continue;
                this._handle(name, p);
            }
            this._resolveRevWaiters();
        })();
        return this._syncPromise;
    }

    _applySnapshot(snap) {
        this.settings = snap.settings;
        this.dsp = snap.dsp;
        this.journalSeq = snap.journalSeq ?? 0;
        this.waveActive = !!snap.wave?.active;
        this.mirror.reset(snap.queue);
        this._expectRev = Math.max(this._expectRev, this.mirror.rev);
        this._deferredState = null;
        this._applyState(snap.state, { force: true });
        this._onSleep(snap.sleepTimer?.endsAt ?? null);
        this._afterQueueChange();
        this._emit('snapshot', snap);
    }

    _onState(state) {
        if (state.uid && !this.mirror.entry(state.uid)) {
            // The queue event naming this entry has not arrived yet.
            this._deferredState = state;
            clearTimeout(this._deferTimer);
            this._deferTimer = setTimeout(() => {
                if (this._deferredState) void this.resync();
            }, REV_WAIT_MS);
            return;
        }
        this._deferredState = null;
        this._applyState(state);
    }

    _applyState(state, { force = false } = {}) {
        const prev = this.state;
        this.state = { ...IDLE_STATE, ...state };
        const quiet = this._volumeInFlight > 0 || Date.now() < this._volumeQuietUntil;
        if (!quiet || force) {
            this._volume = this.state.volume;
            this._muted = this.state.muted;
            try {
                localStorage.setItem('volume', String(this._volume));
            } catch {
                /* storage full or unavailable */
            }
        }
        const uidChanged = force || prev.uid !== this.state.uid;
        this._renderCurrent();
        this.media.applyState({ ...this.state, volume: this._volume, muted: this._muted });
        if (force || (prev.status === 'loading') !== (this.state.status === 'loading')) {
            this.setLoadingState(this.state.status === 'loading');
        }
        if (uidChanged || JSON.stringify(prev.stream) !== JSON.stringify(this.state.stream)) {
            this.updateAdaptiveQualityBadge();
        }
        this._emit('state', this.state);
    }

    _renderCurrent() {
        const track = this.currentTrack;
        const key = track?._uid ?? null;
        if (key === this._renderedUid) return;
        this._renderedUid = key;
        this.renderNowPlaying(track);
    }

    _onPosition(p) {
        if (p.uid !== this.state.uid) return;
        this.state = { ...this.state, position: p.position, at: p.at, duration: p.duration ?? this.state.duration };
        this.media.applyPosition(p);
    }

    _onQueue(change) {
        const result = this.mirror.apply(change);
        if (result === 'gap') {
            void this.resync();
            return;
        }
        if (result !== 'applied') return;
        if (this._deferredState && this.mirror.entry(this._deferredState.uid)) {
            const s = this._deferredState;
            this._deferredState = null;
            this._applyState(s);
        }
        this._afterQueueChange();
        this._resolveRevWaiters();
        this._emit('queue', change);
    }

    _afterQueueChange() {
        const { shuffle, repeat } = this.mirror;
        if (this._optimistic.shuffle === shuffle) delete this._optimistic.shuffle;
        if (this._optimistic.repeat === repeat) delete this._optimistic.repeat;
        this._syncControls();
        this.updatePlayingTrackIndicator();

        const radioOn = this.radioEnabled;
        if (radioOn !== this._radioWasOn) {
            this._radioWasOn = radioOn;
            radioSettings.setEnabled(radioOn);
            window.dispatchEvent(new CustomEvent('radio-state-changed', { detail: { enabled: radioOn } }));
        }
        const fetching = !!this.mirror.continuation.fetching && this.mirror.continuation.kind !== 'none';
        if (fetching !== this._fetchingShown) {
            this._fetchingShown = fetching;
            this.showRadioLoading(fetching);
        }

        // With nothing loaded the bar shows the current queue entry (as after a restart).
        this._renderCurrent();
        if (window.renderQueueFunction) void Promise.resolve(window.renderQueueFunction()).catch(warn('queue render'));
    }

    /** Shuffle/repeat buttons follow the native state (MPRIS can change them too). */
    _syncControls() {
        const shuffle = this.shuffleActive;
        const mode = this.repeatMode;
        for (const id of ['shuffle-btn', 'fs-shuffle-btn']) {
            document.getElementById(id)?.classList.toggle('active', shuffle);
        }
        const repeatBtn = document.getElementById('repeat-btn');
        if (repeatBtn) {
            repeatBtn.classList.toggle('active', mode !== REPEAT_MODE.OFF);
            repeatBtn.classList.toggle('repeat-one', mode === REPEAT_MODE.ONE);
            repeatBtn.title =
                mode === REPEAT_MODE.OFF ? 'Repeat' : mode === REPEAT_MODE.ALL ? 'Repeat Queue' : 'Repeat One';
        }
        document.getElementById('fs-repeat-btn')?.classList.toggle('active', mode !== REPEAT_MODE.OFF);
    }

    _onQueueLow() {
        const s = this.artistPopularTracksState;
        if (!s.artistId || !s.hasMore || s.isFetching) return;
        void this.fetchMoreArtistPopularTracks()
            .then((tracks) => (tracks?.length ? this.addToQueue(tracks) : undefined))
            .catch(warn('artist tracks'));
    }

    _onSettings({ settings, dsp } = {}) {
        if (settings) {
            this.settings = settings;
            const quiet = this._volumeInFlight > 0 || Date.now() < this._volumeQuietUntil;
            if (!quiet) {
                this._volume = settings.volume;
                this._muted = settings.muted;
                this.media.setLocalVolume(this._volume, this._muted);
            }
            try {
                localStorage.setItem('volume', String(settings.volume));
                audioEffectsSettings.setSpeed(settings.speed);
                audioEffectsSettings.setPreservePitch(settings.preservePitch);
                autoplaySettings.setEnabled(settings.autoplay);
            } catch {
                /* storage unavailable */
            }
        }
        if (dsp) this.dsp = dsp;
        this._emit('settings', { settings, dsp });
    }

    _onWave(active) {
        if (this.waveActive === active) return;
        this.waveActive = active;
        this._emit('wave', { active });
    }

    /**
     * First start with the native player (§5.8): hand it the settings, the
     * EQ and the queue the old `<audio>` player kept in localStorage.
     * `migrated: true` goes last, so an interrupted import runs again.
     */
    async _migrate() {
        try {
            await this.bridge.call('player_set_dsp', { config: audioContextManager.buildDspConfig() });
            if (!this.mirror.length) {
                const m = buildQueueMigration(queueManager.getQueue(), isPlayableTrack);
                if (m) {
                    await this.bridge.call('player_set_queue', {
                        tracks: m.tracks.map((t) => toTrackInput(t)),
                        startIndex: m.startIndex,
                        play: false,
                    });
                    if (m.shuffle) await this.bridge.call('player_set_shuffle', { on: true });
                    if (m.repeat !== 'off') await this.bridge.call('player_set_repeat', { mode: m.repeat });
                }
            }
            try {
                localStorage.removeItem(queueManager.STORAGE_KEY);
            } catch {
                /* storage unavailable */
            }
            const patch = {
                ...buildSettingsPatch(SETTINGS_SOURCES, { quality: this.quality }),
                ...this._legacyVolume,
                migrated: true,
            };
            const settings = await this.bridge.call('player_set_settings', { patch });
            if (settings) this.settings = settings;
            console.info('[player] imported the old player state');
        } catch (e) {
            console.error('[player] migration failed', e);
        }
    }

    // ---- commands -------------------------------------------------------------

    /** Sends a command through the FIFO; errors are logged, never thrown to callers. */
    _cmd(cmd, args) {
        return this.bridge.call(cmd, args).catch((e) => {
            warn(cmd)(e);
            return null;
        });
    }

    /** A queue mutation (the {rev} it answers is recorded by bridge.onReply). */
    _mutate(cmd, args) {
        return this._cmd(cmd, args);
    }

    /** Resolves when the mirror has seen every queue change Rust has acknowledged. */
    _caughtUp() {
        if (this.mirror.rev >= this._expectRev) return Promise.resolve();
        return new Promise((resolve) => {
            const waiter = { rev: this._expectRev, resolve };
            waiter.timer = setTimeout(() => {
                void this.resync().then(() => this._resolveRevWaiters(true));
            }, REV_WAIT_MS);
            this._revWaiters.push(waiter);
        });
    }

    _resolveRevWaiters(all = false) {
        this._revWaiters = this._revWaiters.filter((w) => {
            if (!all && this.mirror.rev < w.rev) return true;
            clearTimeout(w.timer);
            w.resolve();
            return false;
        });
    }

    /** Lazy args for an entry chosen by index, resolved once the mirror caught up. */
    _atIndex(index, build) {
        return async () => {
            await this._caughtUp();
            const uid = typeof index === 'function' ? index() : this.mirror.uidAt(index);
            return uid ? build(uid) : SKIP;
        };
    }

    _inputs(trackOrTracks, origin) {
        const list = (Array.isArray(trackOrTracks) ? trackOrTracks : [trackOrTracks]).filter(Boolean);
        return list.filter(isPlayableTrack).map((t) => toTrackInput(t, origin));
    }

    // RemoteMediaElement controller
    seek(position, { resume = false } = {}) {
        return this._cmd('player_seek', { position, resume });
    }

    play() {
        return this._cmd('player_play');
    }

    pause() {
        return this._cmd('player_pause');
    }

    setMuted(muted) {
        this._muted = !!muted;
        this.media.setLocalVolume(this._volume, this._muted);
        this._pushVolume();
    }

    setRate(rate) {
        void this.setPlaybackSpeed(rate);
    }

    // ---- playback -------------------------------------------------------------

    get activeElement() {
        return this.media;
    }

    /** The loaded entry, or the current queue entry while nothing is loaded. */
    get currentTrack() {
        if (this.state.uid) return this.mirror.track(this.state.uid) ?? this.mirror.currentTrack;
        return this.mirror.currentTrack;
    }

    get isLoadingTrack() {
        return this.state.status === 'loading';
    }

    set isLoadingTrack(_v) {
        // Derived from the native state.
    }

    get currentStreamInfo() {
        const s = this.state.stream;
        if (!s) return null;
        const codec = s.codec ? String(s.codec) : null;
        const kbps = s.bitrateKbps ?? null;
        return {
            codec,
            bitrateKbps: kbps,
            quality: s.quality,
            qualityDisplay: [codec ? codec[0].toUpperCase() + codec.slice(1) : null, kbps ? `~${kbps}k` : null]
                .filter(Boolean)
                .join(' '),
            mimeType: s.mime,
            lossless: false,
            provider: 'youtube',
        };
    }

    async handlePlayPause() {
        if (!this.mirror.length) return;
        await this._cmd('player_toggle');
    }

    async playNext() {
        await this._cmd('player_next');
    }

    async playPrev() {
        await this._cmd('player_prev');
    }

    clampSeekTime(time) {
        const requested = Number(time);
        const safe = Number.isFinite(requested) ? Math.max(0, requested) : 0;
        const d = this.media.duration;
        return Number.isFinite(d) && d > 0 ? Math.min(safe, d) : safe;
    }

    async seekTo(time, { resume = false } = {}) {
        await this.media.seek(this.clampSeekTime(time), { resume });
    }

    seekBackward(seconds = 10) {
        void this.seekTo(Math.max(0, this.media.currentTime - seconds));
    }

    seekForward(seconds = 10) {
        const d = this.media.duration || 0;
        void this.seekTo(Math.min(d, this.media.currentTime + seconds));
    }

    async playAtIndex(index) {
        await this._cmd(
            'player_play_uid',
            this._atIndex(index, (uid) => ({ uid }))
        );
    }

    /** Plays the current queue entry from `startTime` (callers use it right after setQueue). */
    async playTrackFromQueue(startTime = 0) {
        await this._cmd(
            'player_play_uid',
            this._atIndex(
                () => this.mirror.currentUid,
                (uid) => ({ uid, position: Math.max(0, Number(startTime) || 0) })
            )
        );
    }

    async playVideo(video) {
        await this.setQueue([video], 0);
        await this.playAtIndex(0);
    }

    // ---- queue ------------------------------------------------------------------

    get currentQueueIndex() {
        return this.mirror.index;
    }

    get shuffleActive() {
        return this._optimistic.shuffle ?? this.mirror.shuffle;
    }

    set shuffleActive(_v) {
        // Callers set this right before setQueue, which turns shuffle off anyway.
    }

    get repeatMode() {
        return repeatToMode(this._optimistic.repeat ?? this.mirror.repeat);
    }

    /** The pre-shuffle order (the play order when shuffle is off). */
    get queue() {
        return this.mirror.originalQueue;
    }

    get shuffledQueue() {
        return this.mirror.shuffle ? this.mirror.playQueue : [];
    }

    get originalQueueBeforeShuffle() {
        return this.mirror.shuffle ? this.mirror.originalQueue : [];
    }

    getCurrentQueue() {
        return this.mirror.playQueue;
    }

    getNextTrack() {
        const q = this.mirror;
        if (q.index === -1 || !q.length) return null;
        const next = q.index + 1;
        if (next < q.length) return q.track(q.uidAt(next));
        return this.repeatMode === REPEAT_MODE.ALL ? q.track(q.uidAt(0)) : null;
    }

    async setQueue(tracks, startIndex = 0, isRadio = false) {
        if (!isRadio) this._radioSeeds = null;
        const list = (tracks || []).filter(Boolean);
        const kept = list.filter(isPlayableTrack);
        // Keep startIndex pointing at the same track after dropping unplayable ones.
        let start = list.slice(0, Math.max(0, startIndex)).filter(isPlayableTrack).length;
        if (start >= kept.length) start = Math.max(0, kept.length - 1);
        if (kept.length < list.length) console.warn(`[player] ${list.length - kept.length} tracks cannot be played`);
        return this._mutate('player_set_queue', {
            tracks: kept.map((t) => toTrackInput(t)),
            startIndex: start,
            play: false,
        });
    }

    async addToQueue(trackOrTracks) {
        const tracks = this._inputs(trackOrTracks);
        if (tracks.length) await this._mutate('player_enqueue', { tracks, at: 'end' });
    }

    async addNextToQueue(trackOrTracks) {
        const tracks = this._inputs(trackOrTracks);
        if (tracks.length) await this._mutate('player_enqueue', { tracks, at: 'next' });
    }

    /** By index in the queue the user sees (play order). */
    async removeFromQueue(index) {
        const uid = this.mirror.uidAt(index);
        if (!uid) return;
        await this._mutate('player_remove', { uid });
    }

    async moveInQueue(fromIndex, toIndex) {
        const uid = this.mirror.uidAt(fromIndex);
        if (!uid || toIndex < 0 || toIndex >= this.mirror.length) return;
        await this._mutate('player_move', { uid, to: toIndex });
    }

    async clearQueue() {
        await this._mutate('player_clear', { keepCurrent: true });
    }

    async wipeQueue() {
        await this._mutate('player_clear', { keepCurrent: false });
    }

    async toggleShuffle() {
        const on = !this.shuffleActive;
        this._optimistic.shuffle = on;
        this._syncControls();
        await this._mutate('player_set_shuffle', { on });
    }

    async toggleRepeat() {
        const mode = (this.repeatMode + 1) % 3;
        this._optimistic.repeat = modeToRepeat(mode);
        this._syncControls();
        await this._mutate('player_set_repeat', { mode: modeToRepeat(mode) });
        return mode;
    }

    // ---- radio, autoplay, artist popular tracks ---------------------------------

    get radioEnabled() {
        return this.mirror.continuation.kind === 'radio';
    }

    async enableRadio(seeds = []) {
        let picked = Array.isArray(seeds) ? seeds : [seeds];
        let initial;
        if (picked.length === 0) {
            picked = await this.pickRadioSeeds();
            if (!picked.length) return;
            initial = [...picked].sort(() => 0.5 - Math.random()).slice(0, 5);
        } else {
            initial = picked.slice(0, 5);
        }
        const inputs = this._inputs(picked.slice(0, RADIO_SEEDS));
        if (!inputs.length) return;
        await this.setQueue(initial, 0, true);
        await this.playAtIndex(0);
        await this._cmd('player_set_continuation', { kind: 'radio', seeds: inputs });
    }

    disableRadio() {
        if (!this.radioEnabled) return;
        void this._cmd('player_set_continuation', { kind: 'none' });
    }

    async pickRadioSeeds() {
        try {
            const { smartRecommendations } = await import('./smart-recommendations.js');
            const smartSeeds = await smartRecommendations.getSmartSeeds(RADIO_SEEDS);
            if (smartSeeds.length > 0) return smartSeeds;
        } catch (e) {
            console.warn('Smart seeds failed, falling back to basic seed selection:', e);
        }

        try {
            const [history, favorites, userPlaylists] = await Promise.all([
                db.getHistory(),
                db.getFavorites('track'),
                db.getAll('user_playlists'),
            ]);

            const potentialSeeds = [];
            if (history?.length) {
                const frequency = new Map();
                history.forEach((t) => frequency.set(t.id, (frequency.get(t.id) || 0) + 1));
                const historyTracks = Array.from(new Set(history.map((t) => t.id)))
                    .map((id) => history.find((t) => t.id === id))
                    .sort((a, b) => frequency.get(b.id) - frequency.get(a.id));
                potentialSeeds.push(...historyTracks.slice(0, 20));
            }
            if (favorites?.length) potentialSeeds.push(...favorites);
            (userPlaylists || []).forEach((p) => {
                if (p.tracks?.length) {
                    potentialSeeds.push(...[...p.tracks].sort(() => 0.5 - Math.random()).slice(0, 5));
                }
            });
            if (!potentialSeeds.length) return [];

            const unique = Array.from(new Set(potentialSeeds.map((s) => s.id))).map((id) =>
                potentialSeeds.find((s) => s.id === id)
            );
            return unique.sort(() => 0.5 - Math.random()).slice(0, RADIO_SEEDS);
        } catch (error) {
            console.error('Failed to pick radio seeds:', error);
            return this.currentTrack ? [this.currentTrack] : [];
        }
    }

    showRadioLoading(show) {
        const el = document.getElementById('radio-loading-indicator');
        if (el) el.style.display = show ? 'flex' : 'none';
    }

    get autoplayEnabled() {
        return this.settings ? !!this.settings.autoplay : autoplaySettings.isEnabled();
    }

    enableAutoplay() {
        autoplaySettings.setEnabled(true);
        return this.pushSettings({ autoplay: true });
    }

    disableAutoplay() {
        autoplaySettings.setEnabled(false);
        return this.pushSettings({ autoplay: true });
    }

    setArtistPopularTracksContext(artistId, initialTracks, offset = 15, hasMore = true) {
        this.artistPopularTracksState = { artistId, offset, initialTracks, isFetching: false, hasMore };
        void this._cmd('player_set_continuation', { kind: 'ui' });
    }

    clearArtistPopularTracksContext() {
        this.artistPopularTracksState = {
            artistId: null,
            offset: 0,
            initialTracks: [],
            isFetching: false,
            hasMore: false,
        };
        if (this.mirror.continuation.kind === 'ui') void this._cmd('player_set_continuation', { kind: 'none' });
    }

    async fetchMoreArtistPopularTracks() {
        const state = this.artistPopularTracksState;
        if (!state.artistId || state.isFetching || !state.hasMore) return [];
        state.isFetching = true;
        try {
            const result = await this.api.getArtistTopTracks(state.artistId, {
                offset: state.offset,
                limit: 15,
                firstTrackId: state.initialTracks[0]?.id,
            });
            if (result.tracks?.length) {
                state.offset += result.tracks.length;
                state.hasMore = result.hasMore;
                return result.tracks;
            }
            state.hasMore = false;
            return [];
        } catch (error) {
            console.warn('Failed to fetch more artist popular tracks:', error);
            state.hasMore = false;
            return [];
        } finally {
            state.isFetching = false;
        }
    }

    // ---- «Моя волна» (the logic is in Rust; js/wave.js draws the card) ----------

    /** Replaces the queue with a wave batch and plays it. Throws `no_candidates: …` on a cold wave. */
    async waveStart(count = 5) {
        return this.bridge.call('player_wave_start', { count });
    }

    waveStop() {
        return this._cmd('player_wave_stop');
    }

    waveDislike() {
        return this._cmd('player_wave_dislike');
    }

    /** The wave settings changed: Rust swaps the queued wave entries for fresh ones. */
    waveRequeue() {
        return this._mutate('player_wave_requeue');
    }

    // ---- volume, effects, quality, settings ------------------------------------

    get userVolume() {
        return this._volume;
    }

    setVolume(value) {
        this._volume = Math.max(0, Math.min(1, Number(value) || 0));
        try {
            localStorage.setItem('volume', String(this._volume));
        } catch {
            /* storage unavailable */
        }
        this.media.setLocalVolume(this._volume, this._muted);
        this._pushVolume();
    }

    /** Coalesces slider drags: only the latest value is sent once the FIFO gets to it. */
    _pushVolume() {
        this._volumeQuietUntil = Date.now() + VOLUME_QUIET_MS;
        if (this._volumeQueued) return;
        this._volumeQueued = true;
        this._volumeInFlight += 1;
        void this.bridge
            .call('player_set_settings', () => {
                this._volumeQueued = false;
                return { patch: { volume: this._volume, muted: this._muted } };
            })
            .catch(warn('volume'))
            .finally(() => {
                this._volumeInFlight -= 1;
                this._volumeQuietUntil = Date.now() + VOLUME_QUIET_MS;
            });
    }

    getEffectiveVolume() {
        return exponentialVolumeSettings.applyCurve(this._volume);
    }

    /**
     * Sends the settings the UI owns (§3.2). Called by the settings handlers
     * after they updated localStorage. `only` limits the patch to some keys.
     */
    pushSettings(only = null) {
        const full = buildSettingsPatch(SETTINGS_SOURCES, { quality: this.quality });
        const patch = only ? Object.fromEntries(Object.keys(only).map((k) => [k, full[k]])) : full;
        // Merged with anything not sent yet, so a dragged slider sends one patch per round trip.
        this._pendingPatch = { ...this._pendingPatch, ...patch };
        this._patchPromise ??= this._cmd('player_set_settings', () => {
            const next = this._pendingPatch;
            this._pendingPatch = null;
            this._patchPromise = null;
            return { patch: next };
        }).then((settings) => {
            if (settings) this.settings = settings;
            return settings;
        });
        return this._patchPromise;
    }

    applyReplayGain() {
        return this.pushSettings({ replayGain: true, volumeCurve: true });
    }

    applyAudioEffects() {
        return this.pushSettings({ speed: true, preservePitch: true });
    }

    setPlaybackSpeed(speed) {
        audioEffectsSettings.setSpeed(clampSpeed(speed));
        return this.applyAudioEffects();
    }

    setPreservePitch(enabled) {
        audioEffectsSettings.setPreservePitch(enabled);
        return this.applyAudioEffects();
    }

    setQuality(quality) {
        this.quality = normalizeQualityToken(quality) || quality;
        return this.pushSettings({ quality: true });
    }

    /** Sends the DSP config; coalesced like the volume. */
    setDsp(config) {
        this._dspPending = config;
        if (this._dspQueued) return;
        this._dspQueued = true;
        void this.bridge
            .call('player_set_dsp', () => {
                this._dspQueued = false;
                this.dsp = this._dspPending;
                return { config: this._dspPending };
            })
            .catch(warn('dsp'));
    }

    // ---- visualizer (§3.5) ---------------------------------------------------------

    startVisualizer(onFrame, fps = 60) {
        const channel = this.bridge.channel((msg) => {
            const buf =
                msg instanceof ArrayBuffer ? msg : Array.isArray(msg) ? Uint8Array.from(msg).buffer : msg?.buffer;
            if (buf) onFrame(buf);
        });
        return this._cmd('player_visualizer_start', { channel, fps });
    }

    stopVisualizer() {
        return this._cmd('player_visualizer_stop');
    }

    // ---- sleep timer --------------------------------------------------------------

    async setSleepTimer(minutes) {
        const r = await this._cmd('player_set_sleep_timer', { minutes });
        if (r) this._onSleep(r.endsAt ?? null);
    }

    async clearSleepTimer() {
        this._onSleep(null);
        await this._cmd('player_set_sleep_timer', { minutes: null });
    }

    _onSleep(endsAt) {
        this.sleepTimerEndTime = endsAt;
        clearInterval(this.sleepTimerInterval);
        this.sleepTimerInterval = endsAt ? setInterval(() => this.updateSleepTimerUI(), 1000) : null;
        this.updateSleepTimerUI();
        this._emit('sleep', { endsAt });
    }

    getSleepTimerRemaining() {
        if (!this.sleepTimerEndTime) return null;
        return Math.ceil(Math.max(0, this.sleepTimerEndTime - Date.now()) / 1000);
    }

    isSleepTimerActive() {
        return this.sleepTimerEndTime != null;
    }

    updateSleepTimerUI() {
        const update = (btn) => {
            if (!btn) return;
            const remaining = this.isSleepTimerActive() ? this.getSleepTimerRemaining() : 0;
            if (remaining > 0) {
                const minutes = Math.floor(remaining / 60);
                const seconds = remaining % 60;
                const text = `${minutes}:${seconds.toString().padStart(2, '0')}`;
                btn.innerHTML = `<span style="font-size: 12px; font-weight: bold;">${text}</span>`;
                btn.title = `Sleep Timer: ${text} remaining`;
                btn.classList.add('active');
                btn.style.color = 'var(--primary)';
            } else {
                btn.innerHTML = SVG_CLOCK(20);
                btn.title = 'Sleep Timer';
                btn.classList.remove('active');
                btn.style.color = '';
            }
        };
        update(document.getElementById('sleep-timer-btn'));
        update(document.getElementById('sleep-timer-btn-desktop'));
    }

    // ---- now-playing rendering (§5.3) ------------------------------------------------

    renderNowPlaying(track) {
        this.resetProgressUI();
        if (!track) {
            document.querySelector('.now-playing-bar .title')?.replaceChildren();
            const albumEl = document.querySelector('.now-playing-bar .album');
            if (albumEl) {
                albumEl.textContent = '';
                albumEl.style.display = 'none';
            }
            const artistEl = document.querySelector('.now-playing-bar .artist');
            if (artistEl) artistEl.innerHTML = '';
            document.title = 'Volna';
            this.updatePlayingTrackIndicator();
            void uiRenderer()?.setCurrentTrack(null);
            return;
        }

        const trackTitle = getTrackTitle(track);
        const trackArtistsHTML = getTrackArtistsHTML(track);
        const yearDisplay = getTrackYearDisplay(track);

        const coverEl = document.querySelector('.now-playing-bar .cover');
        if (coverEl) {
            const coverId = track.image || track.cover || track.album?.cover;
            const coverUrl = this.api.getCoverUrl(coverId);
            let img = coverEl;
            if (coverEl.tagName === 'VIDEO') {
                img = document.createElement('img');
                img.crossOrigin = 'anonymous';
                img.referrerPolicy = 'no-referrer';
                img.className = coverEl.className;
                img.id = coverEl.id;
                coverEl.replaceWith(img);
            }
            img.style.display = 'block';
            if (img.getAttribute('src') !== coverUrl) img.src = coverUrl;
            img.removeAttribute('srcset');
            img.removeAttribute('sizes');
        }

        this.updateNowPlayingTitle(track);
        const albumEl = document.querySelector('.now-playing-bar .album');
        if (albumEl) {
            const albumTitle = track.album?.title || '';
            const show = albumTitle && albumTitle !== trackTitle;
            albumEl.textContent = show ? albumTitle : '';
            albumEl.style.display = show ? 'block' : 'none';
        }
        const artistEl = document.querySelector('.now-playing-bar .artist');
        if (artistEl) artistEl.innerHTML = trackArtistsHTML + yearDisplay;
        if (!yearDisplay && track.album?.id) this.loadAlbumYear(track, trackArtistsHTML, artistEl);

        document.title = `${trackTitle} • ${getTrackArtists(track)}`;
        this.updatePlayingTrackIndicator();
        this.syncDurationUI();
        void uiRenderer()?.setCurrentTrack(track);
    }

    setLoadingState(isLoading) {
        const playPauseBtn = document.querySelector('.now-playing-bar .play-pause-btn');
        const fsBtn = document.getElementById('fs-play-pause-btn');
        if (isLoading) {
            if (playPauseBtn) playPauseBtn.innerHTML = SPINNER_20;
            if (fsBtn) fsBtn.innerHTML = SPINNER_32;
        } else {
            const paused = this.media.paused;
            if (playPauseBtn) playPauseBtn.innerHTML = paused ? SVG_PLAY(20) : SVG_PAUSE(20);
            if (fsBtn && fsBtn.querySelector('.animate-spin')) fsBtn.innerHTML = paused ? SVG_PLAY(32) : SVG_PAUSE(32);
        }
    }

    resetProgressUI() {
        document.querySelectorAll('#progress-fill, #fs-progress-fill').forEach((el) => (el.style.width = '0%'));
        document.querySelectorAll('#current-time, #fs-current-time').forEach((el) => (el.textContent = '0:00'));
        document.querySelectorAll('#total-duration, #fs-total-duration').forEach((el) => (el.textContent = '0:00'));
    }

    syncDurationUI(element = this.media, fallbackDuration = this.currentTrack?.duration) {
        let duration = Number(element?.duration);
        if (!Number.isFinite(duration) || duration <= 0) {
            duration = Number(fallbackDuration);
            if (duration > 10000) duration /= 1000;
        }
        if (!Number.isFinite(duration) || duration <= 0) return false;
        document
            .querySelectorAll('#total-duration, #fs-total-duration')
            .forEach((el) => (el.textContent = formatTime(duration)));
        return true;
    }

    loadAlbumYear(track, trackArtistsHTML, artistEl) {
        if (!trackDateSettings.useAlbumYear()) return;
        this.api
            .getAlbum(track.album.id)
            .then(({ album }) => {
                if (album?.releaseDate && this.currentTrack?.id === track.id) {
                    track.album.releaseDate = album.releaseDate;
                    const year = new Date(album.releaseDate).getFullYear();
                    if (!isNaN(year) && artistEl) artistEl.innerHTML = `${trackArtistsHTML} • ${year}`;
                }
            })
            .catch(() => {});
    }

    updatePlayingTrackIndicator() {
        const current = this.currentTrack;
        const index = current ? this.mirror.indexOf(current._uid) : -1;
        document.querySelectorAll('.track-item').forEach((item) => {
            item.classList.toggle('playing', !!current && item.dataset.trackId == current.id);
        });
        document.querySelectorAll('.queue-track-item').forEach((item) => {
            item.classList.toggle('playing', parseInt(item.dataset.queueIndex) === index);
        });
    }

    updateNowPlayingTitle(track = this.currentTrack) {
        if (!track) return;
        const titleEl = document.querySelector('.now-playing-bar .title');
        if (!titleEl) return;
        titleEl.innerHTML = `${escapeHtml(getTrackTitle(track))} ${createQualityBadgeHTML(track)}`;
        this.updateAdaptiveQualityBadge();
    }

    /** The stream badge next to the title, from `state.stream`. */
    updateAdaptiveQualityBadge() {
        const titleEl = document.querySelector('.now-playing-bar .title');
        if (!titleEl || !this.currentTrack) return;
        const text = formatQualityBadgeText(this.currentStreamInfo, null, this.quality);
        let badge = titleEl.querySelector('.shaka-quality-badge');
        if (!text) {
            if (badge) badge.style.display = 'none';
            return;
        }
        if (!badge) {
            badge = document.createElement('span');
            badge.className = 'quality-badge quality-hires shaka-quality-badge';
            badge.title = 'Stream Quality';
            titleEl.appendChild(badge);
            const staticBadge = titleEl.querySelector('.quality-badge:not(.shaka-quality-badge)');
            if (staticBadge) staticBadge.style.display = 'none';
        }
        badge.textContent = text;
        badge.style.display = 'inline-flex';
    }

    // ---- window lifecycle ------------------------------------------------------------

    /** After `app:will-close`: the UI has flushed, Rust may destroy the window. */
    readyToClose() {
        return this.bridge.direct('app_ready_to_close').catch(warn('app_ready_to_close'));
    }

    /** Really exits (the close button only hides to the tray). */
    quit() {
        return this.bridge.direct('app_quit').catch(warn('app_quit'));
    }

    // ---- kept for callers; the native player does these itself ----------------------

    forceQuality() {}
    preloadNextTracks() {}
    checkPreloadConditions() {}
    evaluateCrossCodecAbr() {}
    saveQueueState() {}
    updateMediaSession() {}
    updateMediaSessionPlaybackState() {}
    updateMediaSessionPositionState() {}
}
