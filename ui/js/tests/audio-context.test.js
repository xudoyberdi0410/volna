import { beforeAll, describe, expect, it, vi } from 'vitest';
import { buildDspConfig, defaultBandQ } from '../audio-context.js';

describe('buildDspConfig', () => {
    it('maps the parametric EQ bands with default Q, type and channel', () => {
        const cfg = buildDspConfig({
            isEQEnabled: true,
            preamp: -3.5,
            bandCount: 4,
            frequencies: [31, 250, 4000, 30000],
            currentGains: [2, -1, 0, 5],
            currentQs: [0.7, 0, undefined, 1.2],
            currentTypes: ['lowshelf', 'peaking', null, 'highshelf'],
            currentChannels: ['stereo', 'mid', undefined, 'side'],
        });
        expect(cfg.eq.enabled).toBe(true);
        expect(cfg.eq.preampDb).toBe(-3.5);
        expect(cfg.eq.bands).toEqual([
            { freq: 31, gainDb: 2, q: 0.7, type: 'lowshelf', channel: 'stereo' },
            { freq: 250, gainDb: -1, q: defaultBandQ(4), type: 'peaking', channel: 'mid' },
            { freq: 4000, gainDb: 0, q: defaultBandQ(4), type: 'peaking', channel: 'stereo' },
            // Clamped below Nyquist of the fixed 48 kHz chain.
            { freq: 23999, gainDb: 5, q: 1.2, type: 'highshelf', channel: 'side' },
        ]);
        expect(defaultBandQ(16)).toBe(2.5);
    });

    it('maps the graphic EQ, mono and binaural', () => {
        const cfg = buildDspConfig({
            frequencies: [],
            isGraphicEQEnabled: true,
            geqPreamp: 40,
            geqBandCount: 4,
            geqFrequencies: [100, 1000],
            geqGains: [50, -2],
            isMonoAudioEnabled: true,
            isBinauralEnabled: true,
            binaural: { crossfeedEnabled: false, crossfeedLevel: 'high', wideningEnabled: true, wideningAmount: 3 },
        });
        expect(cfg.eq).toEqual({ enabled: false, preampDb: 0, bands: [] });
        expect(cfg.graphicEq).toEqual({
            enabled: true,
            preampDb: 20,
            bands: [
                { freq: 100, gainDb: 30, q: 2.5 * Math.sqrt(4) },
                { freq: 1000, gainDb: -2, q: 2.5 * Math.sqrt(4) },
            ],
        });
        expect(cfg.mono).toBe(true);
        expect(cfg.binaural).toEqual({
            enabled: true,
            crossfeed: { enabled: false, level: 'high' },
            widening: { enabled: true, amount: 2 },
        });
    });

    it('has the §3.7 shape for defaults', () => {
        const cfg = buildDspConfig({ frequencies: [] });
        expect(cfg).toEqual({
            eq: { enabled: false, preampDb: 0, bands: [] },
            graphicEq: { enabled: false, preampDb: 0, bands: [] },
            mono: false,
            binaural: {
                enabled: false,
                crossfeed: { enabled: true, level: 'medium' },
                widening: { enabled: true, amount: 1 },
            },
        });
    });
});

describe('audioContextManager facade', () => {
    let acm;
    const pushed = [];

    beforeAll(async () => {
        const store = new Map();
        globalThis.localStorage ??= {
            getItem: (k) => (store.has(k) ? store.get(k) : null),
            setItem: (k, v) => store.set(k, String(v)),
            removeItem: (k) => store.delete(k),
            clear: () => store.clear(),
        };
        globalThis.window ??= globalThis;
        if (typeof globalThis.dispatchEvent !== 'function') {
            const target = new EventTarget();
            globalThis.dispatchEvent = (e) => target.dispatchEvent(e);
            globalThis.addEventListener = (...a) => target.addEventListener(...a);
        }
        globalThis.document ??= { visibilityState: 'hidden' };
        globalThis.CustomEvent ??= class extends Event {
            constructor(type, init) {
                super(type);
                this.detail = init?.detail;
            }
        };
        ({ audioContextManager: acm } = await import('../audio-context.js'));
        acm.setDspSink((cfg) => pushed.push(cfg));
    });

    it('saves each change and pushes one config per frame', async () => {
        vi.useFakeTimers();
        acm.toggleEQ(true);
        acm.setBandCount(5);
        acm.setBandGain(2, 4);
        acm.setPreamp(-2);
        acm.toggleMonoAudio(true);
        expect(pushed).toHaveLength(0);
        await vi.advanceTimersByTimeAsync(20);
        vi.useRealTimers();
        expect(pushed).toHaveLength(1);
        const cfg = pushed[0];
        expect(cfg.eq.enabled).toBe(true);
        expect(cfg.eq.preampDb).toBe(-2);
        expect(cfg.eq.bands).toHaveLength(5);
        expect(cfg.eq.bands[2].gainDb).toBe(4);
        expect(cfg.mono).toBe(true);
        expect(localStorage.getItem('equalizer-enabled')).toBe('true');
        expect(localStorage.getItem('mono-audio-enabled')).toBe('true');
    });

    it('applies AutoEQ bands with M/S channels and a clipping-safe preamp', () => {
        const text = acm.applyAutoEQBands([
            { enabled: true, type: 'lowshelf', freq: 105, gain: 6, q: 0.7, channel: 'mid' },
            { enabled: true, type: 'peaking', freq: 3000, gain: -2, q: 1.4, channel: 'side' },
            { enabled: true, type: 'highshelf', freq: 10000, gain: 3, q: 0.7 },
            { enabled: false, type: 'peaking', freq: 50, gain: 9, q: 1 },
        ]);
        const cfg = acm.buildDspConfig();
        expect(cfg.eq.bands.map((b) => [b.type, b.channel])).toEqual([
            ['lowshelf', 'mid'],
            ['peaking', 'side'],
            ['highshelf', 'stereo'],
        ]);
        expect(cfg.eq.preampDb).toBeLessThan(0);
        expect(text).toMatch(/^Preamp: -/);
        expect(text).toContain('Filter 1: ON LSC Fc 105 Hz Gain 6.0 dB Q 0.70');
    });

    it('lets settings.js switch the EQ through isEQEnabled', () => {
        acm.isEQEnabled = false;
        expect(acm.buildDspConfig().eq.enabled).toBe(false);
        expect(acm.isReady()).toBe(true);
        expect(acm.getSourceNode()).toBeNull();
        expect(acm.getAnalyser().frequencyBinCount).toBe(512);
    });
});
