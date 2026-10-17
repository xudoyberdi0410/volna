import { describe, expect, it, vi } from 'vitest';
import { JournalConsumer, SCROBBLE_MAX_AGE_MS, shouldScrobble } from '../player-journal.js';

const NOW = 1_760_700_000_000;
const pt = (id) => ({
    uid: `q${id}`,
    id: `track${String(id).padStart(6, '0')}`,
    title: `T${id}`,
    artists: [{ id: 'UC1', name: 'A' }],
    album: null,
    duration: 200,
    cover: null,
    explicit: false,
    kind: 'track',
    origin: 'user',
    raw: null,
    ui: { id: `track${String(id).padStart(6, '0')}`, title: `T${id}`, fromUi: true },
    unavailable: false,
});
const history = (seq, id, at = NOW - 1000) => ({ seq, at, kind: 'history', track: pt(id) });
const end = (seq, id, over = {}) => ({
    seq,
    at: NOW - 500,
    kind: 'end',
    track: pt(id),
    startedAt: NOW - 200_000,
    position: 150,
    duration: 200,
    ended: false,
    inWave: false,
    ...over,
});

function setup(stored = [], cursorStart = 0) {
    let cursor = cursorStart;
    let journal = [...stored];
    const calls = { history: [], plays: [], scrobbles: [], acks: [] };
    const consumer = new JournalConsumer({
        fetch: vi.fn(async (after) => journal.filter((e) => e.seq > after).slice(0, 2)),
        ack: vi.fn(async (upTo) => {
            calls.acks.push(upTo);
            journal = journal.filter((e) => e.seq > upTo);
        }),
        cursor: { get: () => cursor, set: (n) => (cursor = n) },
        addToHistory: vi.fn(async (track, opts) => calls.history.push([track.id, opts.timestamp, track._uid])),
        recordPlay: vi.fn((track, played, duration, ended) => calls.plays.push([track.id, played, duration, ended])),
        scrobble: vi.fn(async (track, at) => calls.scrobbles.push([track.id, at])),
        scrobblePct: () => 0.5,
    });
    return { consumer, calls, cursor: () => cursor };
}

describe('shouldScrobble', () => {
    it('follows the Last.fm rule and the 14-day window', () => {
        expect(shouldScrobble(end(1, 1), 0.5, NOW)).toBe(true);
        expect(shouldScrobble(end(1, 1, { position: 90 }), 0.5, NOW)).toBe(false);
        // Long tracks need at most 4 minutes.
        expect(shouldScrobble(end(1, 1, { duration: 3600, position: 240 }), 0.5, NOW)).toBe(true);
        expect(shouldScrobble(end(1, 1, { duration: 25, position: 25 }), 0.5, NOW)).toBe(false);
        expect(shouldScrobble(end(1, 1, { duration: null, position: 199 }), 0.5, NOW)).toBe(true);
        expect(shouldScrobble(end(1, 1, { at: NOW - SCROBBLE_MAX_AGE_MS - 1 }), 0.5, NOW)).toBe(false);
    });
});

describe('JournalConsumer', () => {
    it('catches up in pages, in seq order, with original timestamps, then acks', async () => {
        vi.useFakeTimers({ now: NOW });
        const { consumer, calls, cursor } = setup([
            history(4, 1, NOW - 9000),
            end(5, 1, { ended: true, position: 200 }),
            history(6, 2, NOW - 3000),
            end(7, 2, { position: 20 }),
            history(8, 3),
        ]);
        await consumer.catchUp();
        vi.useRealTimers();
        expect(calls.history).toEqual([
            ['track000001', NOW - 9000, 'q1'],
            ['track000002', NOW - 3000, 'q2'],
            ['track000003', NOW - 1000, 'q3'],
        ]);
        expect(calls.plays).toEqual([
            ['track000001', 200, 200, true],
            ['track000002', 20, 200, false],
        ]);
        expect(calls.scrobbles).toEqual([['track000001', (NOW - 200_000) / 1000]]);
        expect(cursor()).toBe(8);
        expect(calls.acks).toEqual([8]);
    });

    it('skips entries at or below the cursor and never counts one twice', async () => {
        vi.useFakeTimers({ now: NOW });
        const { consumer, calls, cursor } = setup([history(3, 1), history(4, 2)], 3);
        // A live event that is also in the stored journal.
        void consumer.handle(history(4, 2));
        await consumer.catchUp();
        await consumer.handle(history(4, 2));
        await consumer.handle(history(2, 9));
        await consumer.handle(end(5, 2));
        expect(calls.history.map((h) => h[0])).toEqual(['track000002']);
        expect(calls.plays).toHaveLength(1);
        expect(cursor()).toBe(5);
        await vi.advanceTimersByTimeAsync(6000);
        vi.useRealTimers();
        expect(calls.acks.at(-1)).toBe(5);
    });

    it('keeps going when one entry fails', async () => {
        const { consumer, calls, cursor } = setup([history(1, 1), history(2, 2)]);
        consumer.deps.addToHistory = vi.fn(async (track) => {
            if (track.id === 'track000001') throw new Error('idb');
            calls.history.push([track.id]);
        });
        await consumer.catchUp();
        expect(calls.history).toEqual([['track000002']]);
        expect(cursor()).toBe(2);
    });
});
