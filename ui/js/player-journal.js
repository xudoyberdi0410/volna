// js/player-journal.js
// Turns the native player's journal (docs/native-player.md §3.6, §5.7) into
// what the UI used to record from <audio>: Recent (IndexedDB history), the
// listening tracker and scrobbles. Plays made while the window was closed
// are processed when it opens again, with their original timestamps.
//
// One path for everything: the catch-up on boot and the live
// `player:journal` events both go through `process`, in seq order, and the
// cursor (localStorage) makes sure no entry is counted twice.

import { enrichPlayerTrack } from './player-mirror.js';

/** Scrobble services reject (or ignore) older plays. */
export const SCROBBLE_MAX_AGE_MS = 14 * 24 * 3600 * 1000;
/** Live entries are acknowledged in batches. */
const ACK_DELAY_MS = 5000;

/** The Last.fm rule: tracks over 30 s, played min(pct of the length, 4 min). */
export function shouldScrobble(entry, pct, now = Date.now()) {
    const duration = Number(entry.duration ?? entry.track?.duration);
    if (!(duration > 30)) return false;
    if (!(entry.position >= Math.min(duration * pct, 240))) return false;
    return now - entry.at <= SCROBBLE_MAX_AGE_MS;
}

export class JournalConsumer {
    /**
     * @param {object} deps
     * @param {(after: number) => Promise<object[]>} deps.fetch  player_journal
     * @param {(upTo: number) => Promise<void>} deps.ack  player_journal_ack
     * @param {{get(): number, set(n: number): void}} deps.cursor
     * @param {(raw: object) => object} [deps.prepare]  hifi-api raw → rich track
     * @param {(track, opts) => Promise} deps.addToHistory
     * @param {(track, played, duration, ended, at) => void} deps.recordPlay
     * @param {(track, unixSeconds) => Promise} deps.scrobble
     * @param {() => number} deps.scrobblePct  fraction, e.g. 0.5
     * @param {() => void} [deps.onHistory]
     */
    constructor(deps) {
        this.deps = deps;
        this.last = deps.cursor.get();
        this._chain = Promise.resolve();
        this._ackTimer = null;
        this._acked = this.last;
    }

    _enqueue(fn) {
        const p = this._chain.then(fn);
        this._chain = p.catch((e) => console.warn('[journal]', e));
        return p;
    }

    /** §3.6 step 5: everything Rust kept since the cursor, then ack. */
    catchUp() {
        return this._enqueue(async () => {
            for (;;) {
                const entries = await this.deps.fetch(this.last);
                if (!entries?.length) break;
                for (const entry of entries) await this._process(entry);
            }
            await this._ack();
        });
    }

    /** A live `player:journal` entry. */
    handle(entry) {
        return this._enqueue(async () => {
            await this._process(entry);
            this._scheduleAck();
        });
    }

    async _process(entry) {
        if (!entry || !(entry.seq > this.last)) return;
        try {
            const track = enrichPlayerTrack(entry.track, this.deps.prepare);
            if (entry.kind === 'history') {
                await this.deps.addToHistory(track, { timestamp: entry.at });
                this.deps.onHistory?.();
            } else if (entry.kind === 'end') {
                this.deps.recordPlay(track, entry.position, entry.duration, !!entry.ended, entry.at);
                if (shouldScrobble(entry, this.deps.scrobblePct())) {
                    await this.deps.scrobble(track, entry.startedAt / 1000);
                }
            }
        } catch (e) {
            console.warn('[journal] entry failed', entry.seq, e);
        }
        this.last = entry.seq;
        this.deps.cursor.set(this.last);
    }

    _scheduleAck() {
        if (this._ackTimer) return;
        this._ackTimer = setTimeout(() => {
            this._ackTimer = null;
            void this._enqueue(() => this._ack());
        }, ACK_DELAY_MS);
    }

    async _ack() {
        if (this.last <= this._acked) return;
        const upTo = this.last;
        await this.deps.ack(upTo);
        this._acked = upTo;
    }

    /** Before the window closes. */
    flush() {
        clearTimeout(this._ackTimer);
        this._ackTimer = null;
        return this._enqueue(() => this._ack());
    }
}

/** Wires the consumer to the player, the database, the tracker and the scrobblers (app.js). */
export async function installJournal(player, scrobbler, ui) {
    const [{ db }, { listeningTracker }, { journalCursor, lastFMStorage }] = await Promise.all([
        import('./db.js'),
        import('./listening-tracker.js'),
        import('./storage.js'),
    ]);
    const consumer = new JournalConsumer({
        fetch: (after) => player.bridge.call('player_journal', { after }),
        ack: (upTo) => player.bridge.call('player_journal_ack', { upTo }),
        cursor: journalCursor,
        prepare: (raw) => player.api.getAPI().prepareTrack(raw),
        addToHistory: (track, opts) => db.addToHistory(track, opts),
        recordPlay: (...args) => listeningTracker.recordPlay(...args),
        scrobble: (track, at) => scrobbler.scrobbleAt(track, at),
        scrobblePct: () => (Number(lastFMStorage.getScrobblePercentage()) || 75) / 100,
        onHistory: () => {
            const path = window.location.pathname.replace(/\/+$/, '');
            if (path === '/recent' || window.location.hash === '#recent') void ui?.renderRecentPage?.();
        },
    });
    player.on('journal', (entry) => void consumer.handle(entry));
    await consumer.catchUp();
    return consumer;
}
