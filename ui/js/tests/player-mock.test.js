import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MockBridge, syntheticFrame } from '../player-mock.js';
import { PlayerMirror } from '../player-mirror.js';

const track = (n) => ({
    id: `trk${String(n).padStart(8, '0')}`,
    title: `Track ${n}`,
    artists: [{ id: 'UCa', name: 'Artist' }],
    album: null,
    duration: 30,
    cover: null,
    explicit: false,
    kind: 'track',
    ui: { id: `trk${String(n).padStart(8, '0')}`, title: `Track ${n}` },
});

describe('MockBridge', () => {
    let bridge, mirror, states, journal;

    beforeEach(async () => {
        vi.useFakeTimers({ now: 1_000_000 });
        bridge = new MockBridge();
        mirror = new PlayerMirror();
        states = [];
        journal = [];
        await bridge.on('player:queue', (q) => expect(mirror.apply(q)).toBe('applied'));
        await bridge.on('player:state', (s) => states.push(s));
        await bridge.on('player:journal', (e) => journal.push(e));
    });

    afterEach(() => vi.useRealTimers());

    it('keeps a mirror in sync through queue operations', async () => {
        await bridge.call('player_set_queue', { tracks: [1, 2, 3].map(track), startIndex: 1, play: false });
        await bridge.call('player_enqueue', { tracks: [track(4)], at: 'next' });
        await vi.advanceTimersByTimeAsync(0);
        expect(mirror.playQueue.map((t) => t.title)).toEqual(['Track 1', 'Track 2', 'Track 4', 'Track 3']);
        expect(mirror.index).toBe(1);

        await bridge.call('player_set_shuffle', { on: true });
        await vi.advanceTimersByTimeAsync(0);
        expect(mirror.shuffle).toBe(true);
        expect(mirror.currentTrack.title).toBe('Track 2');
        expect(mirror.index).toBe(0);
        expect(mirror.originalQueue.map((t) => t.title)).toEqual(['Track 1', 'Track 2', 'Track 4', 'Track 3']);

        await bridge.call('player_set_shuffle', { on: false });
        await bridge.call('player_move', { uid: mirror.uidAt(0), to: 3 });
        await vi.advanceTimersByTimeAsync(0);
        expect(mirror.playQueue.map((t) => t.title)).toEqual(['Track 2', 'Track 4', 'Track 3', 'Track 1']);
        expect(mirror.index).toBe(0);
        await bridge.call('player_remove', { uid: mirror.uidAt(3) });
        await vi.advanceTimersByTimeAsync(0);
        expect(mirror.playQueue.map((t) => t.title)).toEqual(['Track 2', 'Track 4', 'Track 3']);
        expect(mirror.currentTrack.title).toBe('Track 2');

        const snap = await bridge.call('player_snapshot');
        const fresh = new PlayerMirror();
        fresh.reset(snap.queue);
        expect(fresh.rev).toBe(mirror.rev);
        expect(fresh.order).toEqual(mirror.order);
    });

    it('plays, writes the journal and advances at the end', async () => {
        await bridge.call('player_set_queue', { tracks: [track(1), track(2)], startIndex: 0, play: true });
        await vi.advanceTimersByTimeAsync(300);
        expect(states.at(-1)).toMatchObject({ status: 'playing', trackId: 'trk00000001' });
        await vi.advanceTimersByTimeAsync(31_000);
        expect(journal.map((e) => e.kind)).toEqual(['history', 'end']);
        expect(journal[1]).toMatchObject({ ended: true, track: { id: 'trk00000001' }, startedAt: 1_000_000 });
        expect(states.at(-1).trackId).toBe('trk00000002');
        expect(mirror.index).toBe(1);
        await vi.advanceTimersByTimeAsync(31_000);
        expect(states.at(-1).status).toBe('ended');
        const entries = await bridge.call('player_journal', { after: 0 });
        expect(entries.map((e) => e.seq)).toEqual([1, 2, 3, 4]);
        await bridge.call('player_journal_ack', { upTo: 2 });
        expect((await bridge.call('player_journal', { after: 0 })).map((e) => e.seq)).toEqual([3, 4]);
    });

    it('rejects unknown uids with the error code', async () => {
        await expect(bridge.call('player_play_uid', { uid: 'nope' })).rejects.toThrow(/^unknown_uid:/);
        await expect(bridge.call('wave_start', {})).rejects.toThrow(/^unavailable:/);
    });

    it('builds visualizer frames in the §3.5 layout', () => {
        const buf = syntheticFrame(1000, true, 12.5);
        expect(buf.byteLength).toBe(3608);
        const v = new DataView(buf);
        expect(String.fromCharCode(...new Uint8Array(buf, 0, 4))).toBe('VVIS');
        expect(v.getUint16(6, true)).toBe(512);
        expect(v.getUint16(8, true)).toBe(1024);
        expect(v.getUint16(10, true) & 1).toBe(1);
        expect(v.getUint32(12, true)).toBe(48000);
        expect(v.getFloat64(16, true)).toBe(12.5);
    });
});
