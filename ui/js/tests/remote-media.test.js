import { describe, expect, it, vi } from 'vitest';
import { RemoteMediaElement } from '../remote-media.js';

const base = {
    status: 'paused',
    uid: 'q1',
    trackId: 'aaaaaaaaaaa',
    position: 10,
    at: 1000,
    duration: 200,
    rate: 1,
    buffering: false,
    crossfading: false,
    volume: 0.5,
    muted: false,
    stream: null,
    error: null,
};

function setup() {
    let now = 1000;
    const controller = {
        seek: vi.fn(() => Promise.resolve()),
        play: vi.fn(() => Promise.resolve()),
        pause: vi.fn(() => Promise.resolve()),
        setVolume: vi.fn(),
        setMuted: vi.fn(),
        setRate: vi.fn(),
    };
    const el = new RemoteMediaElement(controller, { now: () => now });
    const events = [];
    for (const type of [
        'emptied',
        'loadstart',
        'durationchange',
        'loadedmetadata',
        'play',
        'playing',
        'waiting',
        'pause',
        'ended',
        'error',
        'volumechange',
        'ratechange',
        'seeking',
        'seeked',
    ]) {
        el.addEventListener(type, () => events.push(type));
    }
    return { el, controller, events, tick: (ms) => (now += ms), setNow: (v) => (now = v) };
}

describe('RemoteMediaElement', () => {
    it('starts empty', () => {
        const { el } = setup();
        expect(el.paused).toBe(true);
        expect(el.src).toBe('');
        expect(Number.isNaN(el.duration)).toBe(true);
        expect(el.readyState).toBe(0);
        expect(el.currentTime).toBe(0);
        expect(el.error).toBeNull();
        expect(el.id).toBe('audio-player');
    });

    it('interpolates currentTime only while playing, capped at duration', () => {
        const { el, tick } = setup();
        el.applyState({ ...base, status: 'playing', rate: 2 });
        tick(1500);
        expect(el.currentTime).toBeCloseTo(13);
        tick(1_000_000);
        expect(el.currentTime).toBe(200);

        el.applyState({ ...base, status: 'paused', at: 2000, position: 50 });
        tick(5000);
        expect(el.currentTime).toBe(50);
    });

    it('follows position samples for the loaded uid only', () => {
        const { el, setNow } = setup();
        el.applyState({ ...base, status: 'playing' });
        const times = [];
        el.addEventListener('timeupdate', () => times.push(el.currentTime));
        setNow(5000);
        el.applyPosition({ uid: 'q1', position: 80, duration: 200, rate: 1, at: 5000, playing: true });
        el.applyPosition({ uid: 'other', position: 3, duration: 10, rate: 1, at: 5000, playing: true });
        expect(times).toEqual([80]);
        expect(el.currentTime).toBe(80);
    });

    it('dispatches load and play events in media order for a new track', () => {
        const { el, events } = setup();
        el.applyState({ ...base, status: 'idle', uid: null, duration: null });
        events.length = 0;
        el.applyState({ ...base, status: 'loading', position: 0, duration: null });
        expect(events).toEqual(['emptied', 'loadstart', 'play', 'waiting']);
        events.length = 0;
        el.applyState({ ...base, status: 'playing', position: 0 });
        expect(events).toEqual(['durationchange', 'loadedmetadata', 'playing']);
        expect(el.paused).toBe(false);
        expect(el.readyState).toBe(4);
        expect(el.src).toBe('volna-player:q1');
    });

    it('fires play again when the track changes while playing (gapless)', () => {
        const { el, events } = setup();
        el.applyState({ ...base, status: 'playing' });
        events.length = 0;
        el.applyState({ ...base, status: 'playing', uid: 'q2', position: 0 });
        expect(events).toEqual(['emptied', 'loadstart', 'durationchange', 'loadedmetadata', 'play', 'playing']);
    });

    it('dispatches pause, then ended when the queue finishes', () => {
        const { el, events } = setup();
        el.applyState({ ...base, status: 'playing' });
        events.length = 0;
        el.applyState({ ...base, status: 'paused' });
        expect(events).toEqual(['pause']);
        el.applyState({ ...base, status: 'playing' });
        events.length = 0;
        el.applyState({ ...base, status: 'ended', position: 200 });
        expect(events).toEqual(['pause', 'ended']);
        expect(el.ended).toBe(true);
    });

    it('reports errors, volume and rate changes', () => {
        const { el, events } = setup();
        el.applyState(base);
        events.length = 0;
        const error = {
            uid: 'q1',
            trackId: 'aaaaaaaaaaa',
            kind: 'bot_check',
            message: 'x',
            action: 'skipped',
            streak: 1,
        };
        el.applyState({ ...base, error, volume: 0.6, rate: 1.5 });
        expect(events).toEqual(['error', 'volumechange', 'ratechange']);
        expect(el.error).toEqual({ code: 4, message: 'x' });
        events.length = 0;
        el.applyState({ ...base, error, volume: 0.6, rate: 1.5 });
        expect(events).toEqual([]);
    });

    it('sends setters and play/pause to the controller', async () => {
        const { el, controller, events } = setup();
        el.applyState({ ...base, status: 'playing' });
        events.length = 0;
        el.currentTime = 500;
        expect(el.currentTime).toBe(200);
        expect(events).toEqual(['seeking']);
        await new Promise((r) => setTimeout(r, 0));
        expect(controller.seek).toHaveBeenCalledWith(200);
        expect(events).toEqual(['seeking', 'seeked']);

        el.volume = 2;
        el.muted = true;
        el.playbackRate = 1.25;
        expect(controller.setVolume).toHaveBeenCalledWith(1);
        expect(controller.setMuted).toHaveBeenCalledWith(true);
        expect(controller.setRate).toHaveBeenCalledWith(1.25);

        await el.play();
        el.pause();
        expect(controller.play).toHaveBeenCalled();
        expect(controller.pause).toHaveBeenCalled();
    });

    it('lets the lyrics seek through the default action of exact-seek-request', () => {
        const { el, controller } = setup();
        el.applyState(base);
        const req = new CustomEvent('exact-seek-request', { cancelable: true, detail: { time: 30 } });
        if (el.dispatchEvent(req)) el.currentTime = 30;
        return new Promise((r) => setTimeout(r, 0)).then(() => expect(controller.seek).toHaveBeenCalledWith(30));
    });
});
