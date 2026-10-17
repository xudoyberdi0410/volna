import { describe, expect, it } from 'vitest';
import { AnalyserShim, STALE_MS, parseFrame } from '../analyser-shim.js';
import { syntheticFrame } from '../player-mock.js';

describe('AnalyserShim', () => {
    it('parses §3.5 frames and rejects others', () => {
        const f = parseFrame(syntheticFrame(1000, true, 7.25));
        expect(f).toMatchObject({ playing: true, sampleRate: 48000, position: 7.25 });
        expect(f.freq).toHaveLength(512);
        expect(f.time).toHaveLength(1024);
        expect(f.timeR).toHaveLength(1024);
        expect(parseFrame(new ArrayBuffer(3608))).toBeNull();
        expect(parseFrame(new ArrayBuffer(10))).toBeNull();
        expect(parseFrame('nope')).toBeNull();
    });

    it('serves the newest frame and silence once it is stale', () => {
        let now = 0;
        const a = new AnalyserShim({ now: () => now });
        const freq = new Uint8Array(a.frequencyBinCount);
        const time = new Uint8Array(a.fftSize);

        a.getByteFrequencyData(freq);
        a.getByteTimeDomainData(time);
        expect(freq.every((v) => v === 0)).toBe(true);
        expect(time.every((v) => v === 128)).toBe(true);

        const buf = syntheticFrame(1234, true, 1);
        expect(a.push(buf)).toBe(true);
        a.getByteFrequencyData(freq);
        a.getByteTimeDomainData(time);
        expect(Array.from(freq)).toEqual(Array.from(new Uint8Array(buf, 24, 512)));
        expect(Array.from(time)).toEqual(Array.from(new Uint8Array(buf, 536, 1024)));
        const right = new Uint8Array(1024);
        a.getByteTimeDomainDataR(right);
        expect(Array.from(right)).toEqual(Array.from(new Uint8Array(buf, 2584, 1024)));

        // Shorter arrays get a prefix (kawarp reads 512 time samples).
        const short = new Uint8Array(512);
        a.getByteTimeDomainData(short);
        expect(Array.from(short)).toEqual(Array.from(new Uint8Array(buf, 536, 512)));

        now += STALE_MS + 1;
        a.getByteFrequencyData(freq);
        expect(freq.every((v) => v === 0)).toBe(true);
        expect(a.context.sampleRate).toBe(48000);
    });
});
