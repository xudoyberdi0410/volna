// js/remote-media.js
// An HTMLMediaElement look-alike for the native player (docs/native-player.md
// §5.1). Audio plays in Rust; this object keeps the old `<audio>` API and
// events alive for the code that still listens to them (progress bar, lyrics,
// scrobbler now-playing, fullscreen, visualizers).
//
// It is fed `PlaybackState` (player:state) and position samples
// (player:position) and interpolates `currentTime` between samples while
// playing. Setters and play()/pause() go to the controller (js/player.js).

const ACTIVE = new Set(['playing', 'loading']);

const INITIAL_STATE = Object.freeze({
    status: 'idle',
    uid: null,
    trackId: null,
    position: 0,
    at: 0,
    duration: null,
    rate: 1,
    buffering: false,
    crossfading: false,
    volume: 1,
    muted: false,
    stream: null,
    error: null,
});

function sameError(a, b) {
    if (!a || !b) return a === b;
    return a.uid === b.uid && a.kind === b.kind && a.message === b.message && a.streak === b.streak;
}

export class RemoteMediaElement extends EventTarget {
    /**
     * @param {object} controller `{seek(t), play(), pause(), setVolume(v), setMuted(m), setRate(r)}`
     * @param {{now?: () => number}} opts clock (unix ms), injectable for tests
     */
    constructor(controller, { now = () => Date.now() } = {}) {
        super();
        this.id = 'audio-player';
        this.tagName = 'AUDIO';
        this.nodeName = 'AUDIO';
        this.style = {};
        this.dataset = {};
        this.crossOrigin = 'anonymous';
        this.preload = 'auto';
        this.preservesPitch = true;
        this.parentElement = null;
        this._c = controller;
        this._now = now;
        this._s = { ...INITIAL_STATE };
        this._p = { position: 0, at: now(), rate: 1, playing: false };
        this._metaFor = null;
    }

    // ---- HTMLMediaElement surface ------------------------------------------

    get currentTime() {
        const p = this._p;
        let t = p.position;
        if (p.playing) t += (Math.max(0, this._now() - p.at) / 1000) * p.rate;
        const d = this._s.duration;
        if (d > 0 && t > d) t = d;
        return Math.max(0, t);
    }

    set currentTime(value) {
        const t = Math.max(0, Number(value) || 0);
        const d = this._s.duration;
        const target = d > 0 ? Math.min(t, d) : t;
        this._p = { ...this._p, position: target, at: this._now() };
        this._fire('seeking');
        this._fire('timeupdate');
        Promise.resolve()
            .then(() => this._c.seek(target))
            .catch((e) => console.warn('[player] seek failed', e))
            .finally(() => this._fire('seeked'));
    }

    get duration() {
        const d = this._s.duration;
        return d > 0 ? d : NaN;
    }

    get paused() {
        return !ACTIVE.has(this._s.status);
    }

    get ended() {
        return this._s.status === 'ended';
    }

    get seeking() {
        return false;
    }

    get volume() {
        return this._s.volume;
    }

    set volume(v) {
        this._c.setVolume(Math.max(0, Math.min(1, Number(v) || 0)));
    }

    get muted() {
        return this._s.muted;
    }

    set muted(m) {
        this._c.setMuted(!!m);
    }

    get playbackRate() {
        return this._s.rate || 1;
    }

    set playbackRate(r) {
        this._c.setRate(Number(r) || 1);
    }

    get readyState() {
        return this._s.uid && this._s.duration > 0 ? 4 : 0;
    }

    get networkState() {
        return this._s.uid ? 1 : 0;
    }

    get src() {
        return this._s.uid ? `volna-player:${this._s.uid}` : '';
    }

    set src(_v) {
        // Sources are chosen by the native player.
    }

    get currentSrc() {
        return this.src;
    }

    get error() {
        const e = this._s.error;
        return e && e.uid === this._s.uid ? { code: 4, message: e.message } : null;
    }

    play() {
        return Promise.resolve(this._c.play()).then(() => undefined);
    }

    pause() {
        void Promise.resolve(this._c.pause()).catch((e) => console.warn('[player] pause failed', e));
    }

    load() {}
    setAttribute() {}
    removeAttribute() {}
    getAttribute() {
        return null;
    }
    hasAttribute() {
        return false;
    }

    // ---- feeding from the native player ------------------------------------

    /** The last PlaybackState applied. */
    get state() {
        return this._s;
    }

    /** Applies a PlaybackState and dispatches the matching media events. */
    applyState(next) {
        const prev = this._s;
        const s = { ...INITIAL_STATE, ...next };
        this._s = s;
        this._p = {
            position: s.position || 0,
            at: s.at || this._now(),
            rate: s.rate || 1,
            playing: s.status === 'playing',
        };

        const uidChanged = s.uid !== prev.uid;
        const wasActive = ACTIVE.has(prev.status);
        const isActive = ACTIVE.has(s.status);

        if (uidChanged) {
            this._metaFor = null;
            this._fire('emptied');
            if (s.uid) this._fire('loadstart');
        }
        if (s.uid && s.duration > 0 && (this._metaFor !== s.uid || s.duration !== prev.duration)) {
            this._fire('durationchange');
            if (this._metaFor !== s.uid) {
                this._metaFor = s.uid;
                this._fire('loadedmetadata');
            }
        }

        if (isActive && (!wasActive || (uidChanged && s.uid))) this._fire('play');
        if (s.status === 'loading' && prev.status !== 'loading') this._fire('waiting');
        if (s.status === 'playing' && (prev.status !== 'playing' || uidChanged)) this._fire('playing');
        if (!isActive && wasActive) this._fire('pause');
        if (s.status === 'ended' && prev.status !== 'ended') this._fire('ended');

        if (s.error && !sameError(s.error, prev.error)) this._fire('error');
        if (s.volume !== prev.volume || s.muted !== prev.muted) this._fire('volumechange');
        if ((s.rate || 1) !== (prev.rate || 1)) this._fire('ratechange');
        this._fire('timeupdate');
    }

    /** Applies a `player:position` sample. Samples for another uid are ignored. */
    applyPosition(p) {
        if (!p || p.uid !== this._s.uid) return;
        this._p = { position: p.position || 0, at: p.at || this._now(), rate: p.rate || 1, playing: !!p.playing };
        if (p.duration > 0 && p.duration !== this._s.duration) {
            this._s = { ...this._s, duration: p.duration };
            this._fire('durationchange');
            if (this._metaFor !== this._s.uid) {
                this._metaFor = this._s.uid;
                this._fire('loadedmetadata');
            }
        }
        this._fire('timeupdate');
    }

    /** Local volume echo while a change is on its way to Rust. */
    setLocalVolume(volume, muted = this._s.muted) {
        if (volume === this._s.volume && muted === this._s.muted) return;
        this._s = { ...this._s, volume, muted };
        this._fire('volumechange');
    }

    _fire(type) {
        this.dispatchEvent(new Event(type));
    }
}
