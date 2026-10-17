import { describe, expect, it } from 'vitest';
import { SKIP, TauriBridge, parseError } from '../player-bridge.js';

function deferred() {
    let resolve, reject;
    const promise = new Promise((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
}

function fakeTauri() {
    const calls = [];
    const pending = [];
    const invoke = (cmd, args) => {
        const d = deferred();
        calls.push({ cmd, args });
        pending.push(d);
        return d.promise;
    };
    const listeners = new Map();
    const listen = async (name, fn) => {
        listeners.set(name, fn);
        return () => listeners.delete(name);
    };
    class Channel {
        constructor(onmessage) {
            this.onmessage = onmessage;
        }
    }
    return { calls, pending, listeners, bridge: new TauriBridge({ invoke, listen, Channel }) };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('TauriBridge FIFO', () => {
    it('sends the next command only after the previous one answered', async () => {
        const { calls, pending, bridge } = fakeTauri();
        const a = bridge.call('player_set_queue', { tracks: [] });
        const b = bridge.call('player_play_uid', { uid: 'q1' });
        const c = bridge.call('player_enqueue', { tracks: [], at: 'next' });
        await flush();
        expect(calls.map((c) => c.cmd)).toEqual(['player_set_queue']);
        pending[0].resolve({ rev: 1 });
        await flush();
        expect(calls.map((c) => c.cmd)).toEqual(['player_set_queue', 'player_play_uid']);
        pending[1].resolve(null);
        await flush();
        pending[2].resolve({ rev: 2 });
        expect(await a).toEqual({ rev: 1 });
        expect(await b).toBeNull();
        expect(await c).toEqual({ rev: 2 });
        expect(calls.map((c) => c.cmd)).toEqual(['player_set_queue', 'player_play_uid', 'player_enqueue']);
    });

    it('keeps going after a failed command', async () => {
        const { calls, pending, bridge } = fakeTauri();
        const a = bridge.call('player_remove', { uid: 'x' });
        const b = bridge.call('player_next');
        await flush();
        pending[0].reject('unknown_uid: x');
        await expect(a).rejects.toBe('unknown_uid: x');
        await flush();
        expect(calls.map((c) => c.cmd)).toEqual(['player_remove', 'player_next']);
        pending[1].resolve(null);
        await expect(b).resolves.toBeNull();
        expect(calls[1].args).toEqual({});
    });

    it('evaluates lazy arguments when the command is sent, and SKIP drops it', async () => {
        const { calls, pending, bridge } = fakeTauri();
        let uid = null;
        void bridge.call('player_set_queue', { tracks: [] });
        const p = bridge.call('player_play_uid', async () => ({ uid }));
        const s = bridge.call('player_move', () => SKIP);
        const n = bridge.call('player_next');
        await flush();
        uid = 'q7';
        pending[0].resolve({ rev: 1 });
        await flush();
        expect(calls[1]).toEqual({ cmd: 'player_play_uid', args: { uid: 'q7' } });
        pending[1].resolve(null);
        await p;
        expect(await s).toBeNull();
        await flush();
        expect(calls.map((c) => c.cmd)).toEqual(['player_set_queue', 'player_play_uid', 'player_next']);
        pending[2].resolve(null);
        await n;
    });

    it('unwraps event payloads and builds channels', async () => {
        const { listeners, bridge } = fakeTauri();
        const got = [];
        const unlisten = await bridge.on('player:state', (p) => got.push(p));
        listeners.get('player:state')({ event: 'player:state', payload: { status: 'idle' } });
        expect(got).toEqual([{ status: 'idle' }]);
        unlisten();
        expect(listeners.has('player:state')).toBe(false);
        const fn = () => {};
        expect(bridge.channel(fn).onmessage).toBe(fn);
    });
});

describe('parseError', () => {
    it('splits code and message', () => {
        expect(parseError('no_candidates: cold wave')).toEqual({ code: 'no_candidates', message: 'cold wave' });
        expect(parseError(new Error('unavailable: no yt-dlp'))).toEqual({ code: 'unavailable', message: 'no yt-dlp' });
        expect(parseError('boom')).toEqual({ code: 'internal', message: 'boom' });
    });
});
