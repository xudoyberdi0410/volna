// js/player-mock.js
// An in-memory stand-in for the native player (docs/native-player.md §5.1):
// the commands of §3.3 and the events of §3.4 with the same queue rules as
// crates/player, fake 4 Hz position ticks and synthetic visualizer frames.
// No sound. Used by `bun run dev` in a plain browser and by vitest; loaded
// lazily, so it is not part of the production entry chunk.

import { SerialBridge } from './player-bridge.js';

const TICK_MS = 250;
const LOAD_MS = 200;
const DEFAULT_DURATION = 180;

export const DEFAULT_SETTINGS = Object.freeze({
    volume: 0.7,
    muted: false,
    volumeCurve: 'linear',
    quality: 'high',
    crossfade: { enabled: false, seconds: 5 },
    gapless: true,
    speed: 1,
    preservePitch: true,
    replayGain: { mode: 'track', preampDb: 3 },
    autoplay: false,
    blocklist: { trackIds: [], artistIds: [], albumIds: [] },
    migrated: false,
});

export const DEFAULT_DSP = Object.freeze({
    eq: { enabled: false, preampDb: 0, bands: [] },
    graphicEq: { enabled: false, preampDb: 0, bands: [] },
    mono: false,
    binaural: {
        enabled: false,
        crossfeed: { enabled: true, level: 'medium' },
        widening: { enabled: true, amount: 1 },
    },
});

const clone = (v) => (v === undefined ? v : JSON.parse(JSON.stringify(v)));

function err(code, message) {
    return new Error(`${code}: ${message}`);
}

/** The engine. Separate from the bridge so tests can drive it with a fake clock. */
export class MockEngine {
    constructor({
        now = () => Date.now(),
        setTimer = (fn, ms) => setTimeout(() => fn(), ms),
        clearTimer = (id) => clearTimeout(id),
        emit = () => {},
    } = {}) {
        this.now = now;
        this.setTimer = setTimer;
        this.clearTimer = clearTimer;
        this.emit = emit;
        this.counter = 0;
        this.rev = 0;
        this.entries = new Map();
        this.order = [];
        this.original = null;
        this.index = -1;
        this.repeat = 'off';
        this.continuation = { kind: 'none', fetching: false };
        this.seeds = [];
        this.settings = clone(DEFAULT_SETTINGS);
        this.dsp = clone(DEFAULT_DSP);
        this.sleep = null;
        this.sleepTimer = null;
        this.wave = false;
        this.journal = [];
        this.journalSeq = 0;
        this.known = new Set();
        this.dirty = new Set();
        this.play = null; // the current play for the journal: {uid, startedAt, started, history}
        this.state = {
            status: 'idle',
            uid: null,
            trackId: null,
            position: 0,
            at: now(),
            duration: null,
            rate: 1,
            buffering: false,
            crossfading: false,
            volume: this.settings.volume,
            muted: false,
            stream: null,
            error: null,
        };
        this.tickTimer = null;
        this.loadTimer = null;
        this.vis = null;
    }

    // ---- helpers ---------------------------------------------------------------

    uid() {
        this.counter += 1;
        return `q${this.counter.toString(36)}`;
    }

    entry(input) {
        const { origin, ...rest } = input;
        return { ...clone(rest), uid: this.uid(), origin: origin || 'user', raw: null, unavailable: false };
    }

    position() {
        const s = this.state;
        if (s.status !== 'playing') return s.position;
        const t = s.position + ((this.now() - s.at) / 1000) * s.rate;
        return s.duration ? Math.min(t, s.duration) : t;
    }

    blocked(e) {
        const b = this.settings.blocklist;
        if (b.trackIds.includes(e.id)) return true;
        if (e.album?.id && b.albumIds.includes(e.album.id)) return true;
        return e.artists.some((a) => a.id && b.artistIds.includes(String(a.id)));
    }

    playable(uid) {
        const e = this.entries.get(uid);
        return !!e && !e.unavailable && !this.blocked(e);
    }

    emitQueue() {
        this.rev += 1;
        const live = new Set([...this.order, ...(this.original || [])]);
        const added = [...live].filter((u) => !this.known.has(u)).map((u) => clone(this.entries.get(u)));
        const updated = [...this.dirty]
            .filter((u) => live.has(u) && this.known.has(u))
            .map((u) => clone(this.entries.get(u)));
        for (const u of [...this.entries.keys()]) if (!live.has(u)) this.entries.delete(u);
        this.known = live;
        this.dirty.clear();
        this.emit('player:queue', {
            rev: this.rev,
            order: [...this.order],
            original: this.original ? [...this.original] : null,
            index: this.index,
            shuffle: !!this.original,
            repeat: this.repeat,
            continuation: { ...this.continuation },
            added,
            updated,
        });
        return { rev: this.rev };
    }

    setState(patch, { position = true } = {}) {
        const pos = this.position();
        this.state = { ...this.state, position: pos, at: this.now(), ...patch };
        this.emit('player:state', clone(this.state));
        if (position) this.emitPosition();
    }

    emitPosition() {
        const s = this.state;
        if (!s.uid) return;
        this.emit('player:position', {
            uid: s.uid,
            position: this.position(),
            duration: s.duration,
            rate: s.rate,
            at: this.now(),
            playing: s.status === 'playing',
        });
    }

    journalAppend(entry) {
        this.journalSeq += 1;
        const e = { seq: this.journalSeq, at: this.now(), ...entry };
        this.journal.push(e);
        this.emit('player:journal', clone(e));
    }

    closePlay(ended = false) {
        const p = this.play;
        this.play = null;
        if (!p || !p.started) return;
        const e = this.entries.get(p.uid) || p.track;
        this.journalAppend({
            kind: 'end',
            track: clone(e),
            startedAt: p.startedAt,
            position: ended ? (this.state.duration ?? this.position()) : this.position(),
            duration: this.state.duration,
            ended,
            inWave: this.wave && e.origin === 'wave',
        });
    }

    // ---- playback ----------------------------------------------------------------

    load(index, { position = 0, play = true } = {}) {
        this.clearTimer(this.loadTimer);
        this.closePlay(false);
        const uid = this.order[index];
        if (!uid) return this.goIdle();
        this.index = index;
        const e = this.entries.get(uid);
        this.play = { uid, track: clone(e), startedAt: this.now(), started: false, history: false };
        if (this.wave && e.origin !== 'wave') this.setWave(false);
        this.setState({
            status: play ? 'loading' : 'paused',
            uid,
            trackId: e.id,
            position,
            duration: e.duration || DEFAULT_DURATION,
            error: null,
            stream: {
                codec: 'opus',
                bitrateKbps: this.settings.quality === 'low' ? 70 : 160,
                quality: this.settings.quality === 'low' ? 'LOW' : 'HIGH',
                mime: 'audio/webm',
            },
        });
        if (play) {
            this.loadTimer = this.setTimer(() => {
                this.setState({ status: 'playing' });
                this.trackStarted();
            }, LOAD_MS);
        }
    }

    trackStarted() {
        this.startTicking();
        if (this.continuation.kind === 'ui') {
            const remaining = this.order.length - 1 - this.index;
            if (remaining < 3) this.emit('player:queue-low', { remaining });
        }
    }

    goIdle() {
        this.clearTimer(this.loadTimer);
        this.stopTicking();
        this.closePlay(false);
        this.index = this.order.length ? Math.min(this.index, this.order.length - 1) : -1;
        this.setState({ status: 'idle', uid: null, trackId: null, position: 0, duration: null, stream: null });
    }

    startTicking() {
        if (this.tickTimer) return;
        const loop = () => {
            this.tickTimer = this.setTimer(loop, TICK_MS);
            this.tick();
        };
        this.tickTimer = this.setTimer(loop, TICK_MS);
    }

    stopTicking() {
        this.clearTimer(this.tickTimer);
        this.tickTimer = null;
    }

    tick() {
        const s = this.state;
        if (s.status !== 'playing') return;
        const pos = this.position();
        const p = this.play;
        if (p && !p.started && pos > 0.5) p.started = true;
        if (p && !p.history && pos >= 10) {
            p.history = true;
            this.journalAppend({ kind: 'history', track: clone(this.entries.get(p.uid) || p.track) });
        }
        if (s.duration && pos >= s.duration) {
            this.naturalEnd();
            return;
        }
        this.emitPosition();
    }

    naturalEnd() {
        this.closePlay(true);
        if (this.repeat === 'one') {
            this.load(this.index);
            return;
        }
        const next = this.nextIndex();
        if (next === -1) {
            this.stopTicking();
            this.state = { ...this.state, position: this.state.duration ?? this.position() };
            this.setState({ status: 'ended' });
            return;
        }
        this.load(next);
        this.emitQueue();
    }

    nextIndex() {
        const n = this.order.length;
        for (let step = 1; step <= n; step++) {
            let i = this.index + step;
            if (i >= n) {
                if (this.repeat !== 'all') return -1;
                i -= n;
            }
            if (this.playable(this.order[i])) return i;
        }
        return -1;
    }

    prevIndex() {
        for (let i = this.index - 1; i >= 0; i--) if (this.playable(this.order[i])) return i;
        return -1;
    }

    setWave(active) {
        if (this.wave === active) return;
        this.wave = active;
        if (!active && this.continuation.kind === 'wave') this.continuation = { kind: 'none', fetching: false };
        this.emit('player:wave', { active });
    }

    // ---- commands (§3.3) ---------------------------------------------------------

    player_snapshot() {
        return {
            protocol: 1,
            state: { ...clone(this.state), position: this.position(), at: this.now() },
            queue: {
                rev: this.rev,
                items: this.order.map((u) => clone(this.entries.get(u))),
                original: this.original ? [...this.original] : null,
                index: this.index,
                shuffle: !!this.original,
                repeat: this.repeat,
                continuation: { ...this.continuation },
            },
            settings: clone(this.settings),
            dsp: clone(this.dsp),
            sleepTimer: this.sleep ? { endsAt: this.sleep } : null,
            wave: { active: this.wave },
            journalSeq: this.journalSeq,
        };
    }

    player_play() {
        const s = this.state;
        if (!this.order.length) return null;
        if (s.status === 'paused' && s.uid) {
            this.setState({ status: 'playing' });
            this.trackStarted();
        } else if (s.status === 'idle' || s.status === 'ended' || !s.uid) {
            const at = s.status === 'ended' ? 0 : s.position;
            this.load(Math.max(0, this.index), { position: s.uid ? at : 0 });
        }
        return null;
    }

    player_pause() {
        if (this.state.status === 'playing' || this.state.status === 'loading') {
            this.clearTimer(this.loadTimer);
            this.stopTicking();
            this.setState({ status: 'paused' });
        }
        return null;
    }

    player_toggle() {
        return this.state.status === 'playing' || this.state.status === 'loading'
            ? this.player_pause()
            : this.player_play();
    }

    player_stop() {
        this.player_pause();
        this.state = { ...this.state, position: 0 };
        this.setState({});
        return null;
    }

    player_next() {
        if (!this.order.length) return null;
        const next = this.nextIndex();
        if (next === -1) {
            this.closePlay(false);
            this.stopTicking();
            this.setState({ status: 'ended' });
            return null;
        }
        this.load(next);
        this.emitQueue();
        return null;
    }

    player_prev() {
        if (this.position() > 3 || this.index <= 0) return this.player_seek({ position: 0 });
        const prev = this.prevIndex();
        if (prev === -1) return this.player_seek({ position: 0 });
        this.load(prev);
        this.emitQueue();
        return null;
    }

    player_seek({ position, resume = false }) {
        const d = this.state.duration ?? Infinity;
        const t = Math.max(0, Math.min(Number(position) || 0, d));
        this.state = { ...this.state, position: t, at: this.now() };
        if (resume && this.state.status === 'paused') {
            this.setState({ status: 'playing' });
            this.trackStarted();
        } else this.setState({});
        return null;
    }

    player_play_uid({ uid, position = 0 }) {
        const i = this.order.indexOf(uid);
        if (i === -1) throw err('unknown_uid', uid);
        this.load(i, { position });
        this.emitQueue();
        return null;
    }

    player_set_queue({ tracks, startIndex = 0, play = false, position = 0 }) {
        if (!Array.isArray(tracks)) throw err('bad_args', 'tracks');
        this.closePlay(false);
        this.entries = new Map();
        this.order = tracks.map((t) => {
            const e = this.entry(t);
            this.entries.set(e.uid, e);
            return e.uid;
        });
        this.original = null;
        this.continuation = { kind: 'none', fetching: false };
        this.setWave(false);
        this.index = this.order.length ? Math.max(0, Math.min(startIndex, this.order.length - 1)) : -1;
        if (play && this.index >= 0) this.load(this.index, { position });
        else if (this.state.uid && !this.entries.has(this.state.uid)) {
            this.clearTimer(this.loadTimer);
            this.stopTicking();
            this.setState({ status: 'idle', uid: null, trackId: null, position: 0, duration: null, stream: null });
        }
        return this.emitQueue();
    }

    player_enqueue({ tracks, at = 'end' }) {
        const added = tracks.map((t) => {
            const e = this.entry(t);
            this.entries.set(e.uid, e);
            return e.uid;
        });
        if (!added.length) return { rev: this.rev };
        const wasEmpty = this.index === -1;
        const current = this.order[this.index];
        if (at === 'next') {
            this.order.splice(this.index + 1, 0, ...added);
            if (this.original) {
                const oi = this.original.indexOf(current);
                if (oi === -1) this.original.push(...added);
                else this.original.splice(oi + 1, 0, ...added);
            }
        } else {
            this.order.push(...added);
            if (this.original) this.original.push(...added);
        }
        // Like addToQueue: adding to an empty queue starts playing.
        if (wasEmpty) this.load(this.order.indexOf(added[0]));
        return this.emitQueue();
    }

    player_remove({ uid }) {
        const i = this.order.indexOf(uid);
        if (i === -1) throw err('unknown_uid', uid);
        const removingCurrent = i === this.index;
        if (i < this.index) this.index -= 1;
        this.order.splice(i, 1);
        if (this.original) this.original = this.original.filter((u) => u !== uid);
        if (removingCurrent) {
            if (this.index < this.order.length) this.load(this.index);
            else {
                this.index = -1;
                this.goIdle();
            }
        }
        return this.emitQueue();
    }

    player_move({ uid, to }) {
        const from = this.order.indexOf(uid);
        if (from === -1) throw err('unknown_uid', uid);
        if (to < 0 || to >= this.order.length) throw err('bad_args', 'to');
        this.order.splice(from, 1);
        this.order.splice(to, 0, uid);
        if (this.index === from) this.index = to;
        else if (from < this.index && to >= this.index) this.index -= 1;
        else if (from > this.index && to <= this.index) this.index += 1;
        return this.emitQueue();
    }

    player_clear({ keepCurrent }) {
        const current = this.state.uid && this.entries.has(this.state.uid) ? this.state.uid : null;
        if (keepCurrent && current) {
            this.order = [current];
            this.original = this.original ? [current] : null;
            this.index = 0;
        } else {
            this.order = [];
            this.original = this.original ? [] : null;
            this.index = -1;
            if (!keepCurrent) this.goIdle();
        }
        return this.emitQueue();
    }

    player_set_shuffle({ on }) {
        if (on && !this.original) {
            this.original = [...this.order];
            const current = this.order[this.index];
            const rest = this.order.filter((_, i) => i !== this.index);
            for (let i = rest.length - 1; i > 0; i--) {
                const j = Math.floor(Math.random() * (i + 1));
                [rest[i], rest[j]] = [rest[j], rest[i]];
            }
            this.order = current ? [current, ...rest] : rest;
            this.index = current ? 0 : -1;
        } else if (!on && this.original) {
            const current = this.order[this.index];
            this.order = this.original;
            this.original = null;
            this.index = current ? this.order.indexOf(current) : -1;
        }
        return this.emitQueue();
    }

    player_set_repeat({ mode }) {
        if (!['off', 'all', 'one'].includes(mode)) throw err('bad_args', 'mode');
        this.repeat = mode;
        return this.emitQueue();
    }

    player_set_settings({ patch }) {
        const before = this.settings;
        this.settings = { ...before, ...clone(patch) };
        const s = this.settings;
        s.speed = Math.max(0.25, Math.min(4, Number(s.speed) || 1));
        if (s.volume !== this.state.volume || s.muted !== this.state.muted || s.speed !== this.state.rate) {
            this.setState({ volume: s.volume, muted: s.muted, rate: s.speed });
        }
        return clone(this.settings);
    }

    player_set_dsp({ config }) {
        this.dsp = clone(config);
        return null;
    }

    player_set_sleep_timer({ minutes }) {
        this.clearTimer(this.sleepTimer);
        this.sleepTimer = null;
        this.sleep = null;
        if (minutes != null && minutes > 0) {
            this.sleep = this.now() + minutes * 60000;
            this.sleepTimer = this.setTimer(() => {
                this.sleep = null;
                this.player_pause();
                this.emit('player:sleep', { endsAt: null });
            }, minutes * 60000);
        }
        this.emit('player:sleep', { endsAt: this.sleep });
        return { endsAt: this.sleep };
    }

    player_set_continuation({ kind, seeds = [] }) {
        if (!['none', 'radio', 'ui'].includes(kind)) throw err('bad_args', 'kind');
        if (kind === 'radio' && !seeds.length) throw err('bad_args', 'radio needs seeds');
        this.setWave(false);
        this.continuation = { kind, fetching: false };
        this.seeds = clone(seeds);
        this.emitQueue();
        if (kind === 'ui' && this.state.uid) {
            const remaining = this.order.length - 1 - this.index;
            if (remaining < 3) this.emit('player:queue-low', { remaining });
        }
        return null;
    }

    player_wave_start() {
        throw err('unavailable', '«Моя волна» needs the native player');
    }

    player_wave_stop() {
        this.setWave(false);
        this.emitQueue();
        return null;
    }

    player_wave_dislike() {
        return this.player_next();
    }

    player_wave_requeue() {
        return { rev: this.rev };
    }

    player_visualizer_start({ channel, fps = 60 }) {
        this.player_visualizer_stop();
        const rate = Math.max(10, Math.min(60, fps));
        const send = () => {
            const playing = this.state.status === 'playing';
            this.vis = this.setTimer(send, playing ? 1000 / rate : 500);
            try {
                channel.onmessage?.(syntheticFrame(this.now(), playing, this.position()));
            } catch {
                this.player_visualizer_stop();
            }
        };
        this.vis = this.setTimer(send, 1000 / rate);
        return null;
    }

    player_visualizer_stop() {
        this.clearTimer(this.vis);
        this.vis = null;
        return null;
    }

    player_journal({ after = 0 }) {
        return clone(this.journal.filter((e) => e.seq > after).slice(0, 1000));
    }

    player_journal_ack({ upTo }) {
        this.journal = this.journal.filter((e) => e.seq > upTo);
        return null;
    }

    app_ready_to_close() {
        return null;
    }

    app_quit() {
        return null;
    }

    handle(cmd, args) {
        const fn = this[cmd];
        if (typeof fn !== 'function' || !/^(player|app)_/.test(cmd)) {
            throw err('unavailable', `${cmd} is not available without the native player`);
        }
        return fn.call(this, args || {});
    }
}

/** A §3.5 frame with a slowly moving fake spectrum. */
export function syntheticFrame(nowMs, playing, position) {
    const buf = new ArrayBuffer(3608);
    const view = new DataView(buf);
    const bytes = new Uint8Array(buf);
    bytes.set([0x56, 0x56, 0x49, 0x53], 0); // "VVIS"
    view.setUint16(4, 1, true);
    view.setUint16(6, 512, true);
    view.setUint16(8, 1024, true);
    view.setUint16(10, playing ? 1 : 0, true);
    view.setUint32(12, 48000, true);
    view.setFloat64(16, position, true);
    if (playing) {
        const t = nowMs / 1000;
        const beat = Math.max(0, Math.sin(t * Math.PI * 4)) ** 4;
        for (let i = 0; i < 512; i++) {
            const fall = 1 - i / 512;
            const wobble = 0.5 + 0.5 * Math.sin(t * 3 + i * 0.07);
            const bass = i < 12 ? beat : 0;
            bytes[24 + i] = Math.min(255, Math.round(255 * (fall * fall * 0.7 * wobble + bass * 0.6)));
        }
        for (let i = 0; i < 1024; i++) {
            const s = Math.sin(t * 40 + i * 0.05) * (0.3 + 0.5 * beat);
            const v = Math.round(128 + 127 * s);
            bytes[536 + i] = v;
            bytes[1560 + i] = v;
            bytes[2584 + i] = Math.round(128 + 127 * Math.sin(t * 40 + i * 0.05 + 0.3) * (0.3 + 0.5 * beat));
        }
    } else {
        bytes.fill(128, 536);
    }
    return buf;
}

export class MockBridge extends SerialBridge {
    constructor(opts = {}) {
        super();
        this.kind = 'mock';
        this.listeners = new Map();
        this.engine = new MockEngine({ ...opts, emit: (name, payload) => this._emit(name, payload) });
    }

    async _invoke(cmd, args) {
        return clone(this.engine.handle(cmd, args));
    }

    _emit(name, payload) {
        const set = this.listeners.get(name);
        if (!set?.size) return;
        // Tauri delivers events asynchronously.
        const copy = clone(payload);
        queueMicrotask(() => set.forEach((fn) => fn(copy)));
    }

    on(event, handler) {
        if (!this.listeners.has(event)) this.listeners.set(event, new Set());
        this.listeners.get(event).add(handler);
        return Promise.resolve(() => this.listeners.get(event)?.delete(handler));
    }

    channel(onmessage) {
        return { onmessage };
    }

    /** For tests and the devtools console: fire an event as Rust would. */
    emit(name, payload) {
        this._emit(name, payload);
    }
}
