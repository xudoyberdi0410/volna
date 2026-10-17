// js/audio-context.js
// The EQ / DSP settings facade. Audio is processed in Rust (volnadsp in
// crates/player, docs/native-player.md §4.3); this keeps the API settings.js
// and the visualizers were written against. Every change is saved to
// localStorage as before and then sent to Rust as a full DspConfig (§3.2),
// at most once per animation frame.

import { equalizerSettings, monoAudioSettings, binauralDspSettings } from './storage.js';
import { AnalyserShim } from './analyser-shim.js';

/** The native chain runs at a fixed rate (the capsfilter in each deck). */
export const SAMPLE_RATE = 48000;

// Generate frequency array for given number of bands using logarithmic spacing
function generateFrequencies(bandCount, minFreq = 20, maxFreq = 20000) {
    const frequencies = [];
    const safeMin = Math.max(10, minFreq);
    const safeMax = Math.min(96000, maxFreq);

    if (bandCount <= 1) {
        // Single band: use geometric mean of range
        frequencies.push(Math.round(Math.sqrt(safeMin * safeMax)));
        return frequencies;
    }

    for (let i = 0; i < bandCount; i++) {
        // Logarithmic interpolation
        const t = i / (bandCount - 1);
        const freq = safeMin * Math.pow(safeMax / safeMin, t);
        frequencies.push(Math.round(freq));
    }

    return frequencies;
}

// Generate frequency labels for display
function generateFrequencyLabels(frequencies) {
    return frequencies.map((freq) => {
        if (freq < 1000) {
            return freq.toString();
        } else if (freq < 10000) {
            return (freq / 1000).toFixed(freq % 1000 === 0 ? 0 : 1) + 'K';
        } else {
            return (freq / 1000).toFixed(0) + 'K';
        }
    });
}

// EQ Presets (16-band default)
const EQ_PRESETS_16 = {
    flat: { name: 'Flat', gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
    bass_boost: { name: 'Bass Boost', gains: [6, 5, 4.5, 4, 3, 2, 1, 0.5, 0, 0, 0, 0, 0, 0, 0, 0] },
    bass_reducer: { name: 'Bass Reducer', gains: [-6, -5, -4, -3, -2, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
    treble_boost: { name: 'Treble Boost', gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 5.5, 6] },
    treble_reducer: { name: 'Treble Reducer', gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, -1, -2, -3, -4, -5, -5.5, -6] },
    vocal_boost: { name: 'Vocal Boost', gains: [-2, -1, 0, 0, 1, 2, 3, 4, 4, 3, 2, 1, 0, 0, -1, -2] },
    loudness: { name: 'Loudness', gains: [5, 4, 3, 1, 0, -1, -1, 0, 0, 1, 2, 3, 4, 4.5, 4, 3] },
    rock: { name: 'Rock', gains: [4, 3.5, 3, 2, -1, -2, -1, 1, 2, 3, 3.5, 4, 4, 3, 2, 1] },
    pop: { name: 'Pop', gains: [-1, 0, 1, 2, 3, 3, 2, 1, 0, 1, 2, 2, 2, 2, 1, 0] },
    classical: { name: 'Classical', gains: [3, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 3, 2] },
    jazz: { name: 'Jazz', gains: [3, 2, 1, 1, -1, -1, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2] },
    electronic: { name: 'Electronic', gains: [4, 3.5, 3, 1, 0, -1, 0, 1, 2, 3, 3, 2, 2, 3, 4, 3.5] },
    hip_hop: { name: 'Hip-Hop', gains: [5, 4.5, 4, 3, 1, 0, 0, 1, 1, 1, 1, 1, 1, 2, 2, 2] },
    r_and_b: { name: 'R&B', gains: [3, 5, 4, 2, 1, 0, 1, 1, 1, 1, 2, 2, 2, 1, 1, 1] },
    acoustic: { name: 'Acoustic', gains: [3, 2, 1, 1, 2, 2, 1, 0, 0, 1, 1, 2, 3, 3, 2, 1] },
    podcast: { name: 'Podcast / Speech', gains: [-3, -2, -1, 0, 1, 2, 3, 4, 4, 3, 2, 1, 0, -1, -2, -3] },
};

// Interpolate 16-band preset to target band count
function interpolatePreset(preset16, targetBands) {
    if (targetBands === 16) return [...preset16];

    const result = [];
    for (let i = 0; i < targetBands; i++) {
        const sourceIndex = (i / (targetBands - 1)) * (preset16.length - 1);
        const indexLow = Math.floor(sourceIndex);
        const indexHigh = Math.min(Math.ceil(sourceIndex), preset16.length - 1);
        const fraction = sourceIndex - indexLow;

        const lowValue = preset16[indexLow] || 0;
        const highValue = preset16[indexHigh] || 0;
        const interpolated = lowValue + (highValue - lowValue) * fraction;
        result.push(Math.round(interpolated * 10) / 10);
    }
    return result;
}

// Get presets for given band count
function getPresetsForBandCount(bandCount) {
    const presets = {};
    for (const [key, preset] of Object.entries(EQ_PRESETS_16)) {
        presets[key] = {
            name: preset.name,
            gains: interpolatePreset(preset.gains, bandCount),
        };
    }
    return presets;
}

// Default export for backwards compatibility (16 bands)
const EQ_PRESETS = EQ_PRESETS_16;

const clampNum = (v, lo, hi, fallback) => {
    const n = parseFloat(v);
    return Math.max(lo, Math.min(hi, Number.isFinite(n) ? n : fallback));
};

/** The default Q of a parametric band: tighter as the band count grows. */
export function defaultBandQ(bandCount) {
    return 2.5 * Math.sqrt(16 / bandCount);
}

/**
 * The DspConfig (docs/native-player.md §3.2) for an EQ state. Pure.
 * `s` has the AudioContextManager field names.
 */
export function buildDspConfig(s) {
    const nyquist = SAMPLE_RATE / 2 - 1;
    const eqBands = (s.frequencies || []).slice(0, 32).map((freq, i) => ({
        freq: Math.min(Number(freq) || 1000, nyquist),
        gainDb: Number(s.currentGains?.[i]) || 0,
        q: s.currentQs?.[i] > 0 ? s.currentQs[i] : defaultBandQ(s.bandCount || s.frequencies.length),
        type: ['peaking', 'lowshelf', 'highshelf'].includes(s.currentTypes?.[i]) ? s.currentTypes[i] : 'peaking',
        channel: ['stereo', 'mid', 'side'].includes(s.currentChannels?.[i]) ? s.currentChannels[i] : 'stereo',
    }));
    const geqQ = defaultBandQ(s.geqBandCount || (s.geqFrequencies || []).length || 16);
    const geqBands = (s.geqFrequencies || []).slice(0, 32).map((freq, i) => ({
        freq: Math.min(Number(freq) || 1000, nyquist),
        gainDb: clampNum(s.geqGains?.[i], -30, 30, 0),
        q: geqQ,
    }));
    const b = s.binaural || {};
    return {
        eq: { enabled: !!s.isEQEnabled, preampDb: clampNum(s.preamp, -20, 20, 0), bands: eqBands },
        graphicEq: { enabled: !!s.isGraphicEQEnabled, preampDb: clampNum(s.geqPreamp, -20, 20, 0), bands: geqBands },
        mono: !!s.isMonoAudioEnabled,
        binaural: {
            enabled: !!s.isBinauralEnabled,
            crossfeed: {
                enabled: b.crossfeedEnabled !== false,
                level: ['low', 'medium', 'high'].includes(b.crossfeedLevel) ? b.crossfeedLevel : 'medium',
            },
            widening: { enabled: b.wideningEnabled !== false, amount: clampNum(b.wideningAmount, 0, 2, 1) },
        },
    };
}

class AudioContextManager {
    constructor() {
        this._sink = null;
        this._pushScheduled = false;
        this._analyser = null;
        this._offlineContext = null;
        this._eqEnabled = false;
        this.isInitialized = true;
        this.msEnabled = false;

        this.bandCount = equalizerSettings.getBandCount();
        this.freqRange = equalizerSettings.getFreqRange();
        this.frequencies = generateFrequencies(this.bandCount, this.freqRange.min, this.freqRange.max);
        this.currentGains = new Array(this.bandCount).fill(0);
        this.currentChannels = new Array(this.bandCount).fill('stereo');

        this.isGraphicEQEnabled = equalizerSettings.isGraphicEqEnabled();
        this.geqBandCount = equalizerSettings.getGraphicEqBandCount();
        this.geqFreqRange = equalizerSettings.getGraphicEqFreqRange();
        this.geqFrequencies = generateFrequencies(this.geqBandCount, this.geqFreqRange.min, this.geqFreqRange.max);
        this.geqGains = equalizerSettings.getGraphicEqGains(this.geqBandCount);
        this.geqPreamp = equalizerSettings.getGraphicEqPreamp();

        this._loadSettings();
    }

    // ---- the Rust side ---------------------------------------------------------

    /** Where configs go (set by js/player.js). Called with a DspConfig. */
    setDspSink(fn) {
        this._sink = fn;
    }

    /** The current DspConfig. */
    buildDspConfig() {
        return buildDspConfig({
            ...this,
            isEQEnabled: this._eqEnabled,
            binaural: {
                crossfeedEnabled: binauralDspSettings.getCrossfeedEnabled(),
                crossfeedLevel: binauralDspSettings.getCrossfeedLevel(),
                wideningEnabled: binauralDspSettings.getWideningEnabled(),
                wideningAmount: binauralDspSettings.getWideningAmount(),
            },
        });
    }

    /** Sends the config once per frame however many sliders moved. */
    scheduleDspPush() {
        if (this._pushScheduled) return;
        this._pushScheduled = true;
        const flush = () => {
            this._pushScheduled = false;
            this.pushDsp();
        };
        if (typeof requestAnimationFrame === 'function' && document.visibilityState !== 'hidden') {
            requestAnimationFrame(flush);
        } else {
            setTimeout(flush, 16);
        }
    }

    pushDsp() {
        try {
            this._sink?.(this.buildDspConfig());
        } catch (e) {
            console.warn('[AudioContext] DSP push failed:', e);
        }
    }

    get isEQEnabled() {
        return this._eqEnabled;
    }

    /** settings.js flips this directly when switching between the two EQs. */
    set isEQEnabled(enabled) {
        this._eqEnabled = !!enabled;
        this.scheduleDspPush();
    }

    // ---- Web Audio compatibility (no graph any more) --------------------------

    init() {}
    changeSource() {}
    async resume() {}
    notifyBinauralChannelCount() {}

    isReady() {
        return true;
    }

    /** Fed by the native visualizer frames (js/visualizer.js subscribes). */
    getAnalyser() {
        this._analyser ??= new AnalyserShim();
        return this._analyser;
    }

    /** Only butterchurn needs a context, to build its own analysers. */
    getAudioContext() {
        if (!this._offlineContext && typeof OfflineAudioContext === 'function') {
            this._offlineContext = new OfflineAudioContext(2, 2, SAMPLE_RATE);
        }
        return this._offlineContext;
    }

    getSourceNode() {
        return null;
    }

    onGraphChange() {
        return () => {};
    }

    // ---- band layout -----------------------------------------------------------

    setBandCount(count) {
        const newCount = Math.max(
            equalizerSettings.MIN_BANDS,
            Math.min(equalizerSettings.MAX_BANDS, parseInt(count, 10) || 16)
        );
        if (newCount === this.bandCount) return;

        equalizerSettings.setBandCount(newCount);
        this.bandCount = newCount;
        this.frequencies = generateFrequencies(newCount, this.freqRange.min, this.freqRange.max);
        this.currentGains = equalizerSettings.interpolateGains(this.currentGains, newCount);
        equalizerSettings.setGains(this.currentGains);
        this.scheduleDspPush();

        window.dispatchEvent(
            new CustomEvent('equalizer-band-count-changed', {
                detail: { bandCount: newCount, frequencies: this.frequencies },
            })
        );
    }

    setFreqRange(minFreq, maxFreq) {
        const newMin = Math.max(10, Math.min(96000, parseInt(minFreq, 10) || 20));
        const newMax = Math.max(10, Math.min(96000, parseInt(maxFreq, 10) || 20000));
        if (newMin >= newMax) {
            console.warn('[AudioContext] Invalid frequency range: min must be less than max');
            return false;
        }
        if (newMin === this.freqRange.min && newMax === this.freqRange.max) return true;

        equalizerSettings.setFreqRange(newMin, newMax);
        this.freqRange = { min: newMin, max: newMax };
        this.frequencies = generateFrequencies(this.bandCount, newMin, newMax);
        this.scheduleDspPush();

        window.dispatchEvent(
            new CustomEvent('equalizer-freq-range-changed', {
                detail: { min: newMin, max: newMax, frequencies: this.frequencies },
            })
        );
        return true;
    }

    _calculateQ(_index) {
        return defaultBandQ(this.bandCount);
    }

    // ---- switches ----------------------------------------------------------------

    toggleEQ(enabled) {
        equalizerSettings.setEnabled(enabled);
        this.isEQEnabled = enabled;
        return this._eqEnabled;
    }

    isEQActive() {
        return this._eqEnabled;
    }

    toggleMonoAudio(enabled) {
        this.isMonoAudioEnabled = enabled;
        monoAudioSettings.setEnabled(enabled);
        this.scheduleDspPush();
        return this.isMonoAudioEnabled;
    }

    isMonoAudioActive() {
        return this.isMonoAudioEnabled;
    }

    // ---- binaural (crossfeed + widening; HRTF is gone) -------------------------

    async toggleBinaural(enabled) {
        this.isBinauralEnabled = enabled;
        binauralDspSettings.setEnabled(enabled);
        this.scheduleDspPush();
        return this.isBinauralEnabled;
    }

    isBinauralActive() {
        return this.isBinauralEnabled;
    }

    async setBinauralCrossfeedEnabled(enabled) {
        binauralDspSettings.setCrossfeedEnabled(enabled);
        this.scheduleDspPush();
    }

    setBinauralCrossfeedLevel(level) {
        binauralDspSettings.setCrossfeedLevel(level);
        this.scheduleDspPush();
    }

    /** Stored only: multichannel HRTF is not part of the native chain. */
    async setBinauralHrtfPreset(preset) {
        binauralDspSettings.setHrtfPreset(preset);
    }

    async setBinauralWideningEnabled(enabled) {
        binauralDspSettings.setWideningEnabled(enabled);
        this.scheduleDspPush();
    }

    setBinauralWidening(amount) {
        binauralDspSettings.setWideningAmount(amount);
        this.scheduleDspPush();
    }

    getBinauralStatus() {
        return {
            enabled: this.isBinauralEnabled,
            crossfeedEnabled: binauralDspSettings.getCrossfeedEnabled(),
            crossfeedLevel: binauralDspSettings.getCrossfeedLevel(),
            wideningEnabled: binauralDspSettings.getWideningEnabled(),
            wideningAmount: binauralDspSettings.getWideningAmount(),
        };
    }

    // ---- gains ---------------------------------------------------------------------

    getRange() {
        return equalizerSettings.getRange();
    }

    /** Biquad magnitude response in dB (RBJ; shelves use Q, as the native chain does). */
    _biquadResponseDb(f, band, sr) {
        if (!band.enabled || !band.type) return 0;
        const w = (2 * Math.PI * band.freq) / sr;
        const p = (2 * Math.PI * f) / sr;
        const s = Math.sin(w) / (2 * band.q);
        const A = Math.pow(10, band.gain / 40);
        const c = Math.cos(w);
        let b0, b1, b2, a0, a1, a2;
        const t = band.type[0];
        if (t === 'p') {
            b0 = 1 + s * A;
            b1 = -2 * c;
            b2 = 1 - s * A;
            a0 = 1 + s / A;
            a1 = -2 * c;
            a2 = 1 - s / A;
        } else if (t === 'l') {
            const sq = 2 * Math.sqrt(A) * s;
            b0 = A * (A + 1 - (A - 1) * c + sq);
            b1 = 2 * A * (A - 1 - (A + 1) * c);
            b2 = A * (A + 1 - (A - 1) * c - sq);
            a0 = A + 1 + (A - 1) * c + sq;
            a1 = -2 * (A - 1 + (A + 1) * c);
            a2 = A + 1 + (A - 1) * c - sq;
        } else if (t === 'h') {
            const sq = 2 * Math.sqrt(A) * s;
            b0 = A * (A + 1 + (A - 1) * c + sq);
            b1 = -2 * A * (A - 1 + (A + 1) * c);
            b2 = A * (A + 1 + (A - 1) * c - sq);
            a0 = A + 1 - (A - 1) * c + sq;
            a1 = 2 * (A - 1 - (A + 1) * c);
            a2 = A + 1 - (A - 1) * c - sq;
        } else {
            return 0;
        }
        const _a0 = 1 / a0;
        const b0n = b0 * _a0,
            b1n = b1 * _a0,
            b2n = b2 * _a0;
        const a1n = a1 * _a0,
            a2n = a2 * _a0;
        const cp = Math.cos(p),
            c2p = Math.cos(2 * p);
        const n = b0n * b0n + b1n * b1n + b2n * b2n + 2 * (b0n * b1n + b1n * b2n) * cp + 2 * b0n * b2n * c2p;
        const d = 1 + a1n * a1n + a2n * a2n + 2 * (a1n + a1n * a2n) * cp + 2 * a2n * c2p;
        return 10 * Math.log10(n / d);
    }

    _clampGain(gainDb) {
        const range = this.getRange();
        return Math.max(range.min, Math.min(range.max, gainDb));
    }

    setBandGain(bandIndex, gainDb) {
        if (bandIndex < 0 || bandIndex >= this.bandCount) return;
        this.currentGains[bandIndex] = this._clampGain(gainDb);
        equalizerSettings.setGains(this.currentGains);
        this.scheduleDspPush();
    }

    setAllGains(gains) {
        if (!Array.isArray(gains)) return;
        const adjusted =
            gains.length !== this.bandCount ? equalizerSettings.interpolateGains(gains, this.bandCount) : gains;
        adjusted.forEach((gain, i) => {
            this.currentGains[i] = this._clampGain(gain);
        });
        equalizerSettings.setGains(this.currentGains);
        this.scheduleDspPush();
    }

    applyPreset(presetKey) {
        const preset = getPresetsForBandCount(this.bandCount)[presetKey];
        if (!preset) return;
        this.setAllGains(preset.gains);
        equalizerSettings.setPreset(presetKey);
    }

    reset() {
        this.setAllGains(new Array(this.bandCount).fill(0));
        equalizerSettings.setPreset('flat');
    }

    getGains() {
        return [...this.currentGains];
    }

    getBandCount() {
        return this.bandCount;
    }

    _loadSettings() {
        this._eqEnabled = equalizerSettings.isEnabled();
        this.bandCount = equalizerSettings.getBandCount();
        this.freqRange = equalizerSettings.getFreqRange();
        const customFreqs = equalizerSettings.getCustomFrequencies(this.bandCount);
        this.frequencies = customFreqs || generateFrequencies(this.bandCount, this.freqRange.min, this.freqRange.max);
        this.currentGains = equalizerSettings.getGains(this.bandCount);
        this.currentTypes = equalizerSettings.getBandTypes(this.bandCount);
        this.currentQs = equalizerSettings.getBandQs(this.bandCount);
        this.currentChannels = equalizerSettings.getBandChannels(this.bandCount);
        this.msEnabled = this.currentChannels.some((ch) => ch === 'mid' || ch === 'side');
        this.isMonoAudioEnabled = monoAudioSettings.isEnabled();
        this.preamp = equalizerSettings.getPreamp();
        this.isBinauralEnabled = binauralDspSettings.isEnabled();
    }

    setPreamp(db) {
        const clampedDb = Math.max(-20, Math.min(20, parseFloat(db) || 0));
        this.preamp = clampedDb;
        equalizerSettings.setPreamp(clampedDb);
        this.scheduleDspPush();
    }

    getPreamp() {
        return this.preamp || 0;
    }

    /**
     * Applies AutoEQ bands (specific frequencies, gains and Qs).
     * @returns {string} the applied EQ as text
     */
    applyAutoEQBands(bands, skipPreamp = false) {
        if (!bands || bands.length === 0) return '';

        const enabledBands = bands.filter((b) => b.enabled);
        if (enabledBands.length === 0) return '';
        const count = Math.max(equalizerSettings.MIN_BANDS, Math.min(equalizerSettings.MAX_BANDS, enabledBands.length));

        // Preamp: the negative of the summed peak gain, so nothing clips.
        let cumulativePeak = 0;
        if (!skipPreamp) {
            for (let f = 20; f <= 20000; f *= Math.pow(2, 1 / 24)) {
                let sum = 0;
                for (const b of enabledBands) sum += this._biquadResponseDb(f, b, SAMPLE_RATE);
                if (sum > cumulativePeak) cumulativePeak = sum;
            }
        }
        const preamp = skipPreamp
            ? equalizerSettings.getPreamp()
            : cumulativePeak > 0
              ? -Math.round(cumulativePeak * 10) / 10
              : 0;

        const sortedBands = [...enabledBands].sort((a, b) => a.freq - b.freq);
        const maxFreq = SAMPLE_RATE / 2 - 1;
        const slicedBands = sortedBands.slice(0, count);
        const newFrequencies = slicedBands.map((b) => Math.round(Math.min(b.freq, maxFreq)));
        const newTypes = slicedBands.map((b) => b.type || 'peaking');
        const newQs = slicedBands.map((b) => b.q);
        const newGains = slicedBands.map((b) => this._clampGain(b.gain));
        const newChannels = slicedBands.map((b) => b.channel || 'stereo');
        while (newFrequencies.length < count) {
            const lastFreq = newFrequencies[newFrequencies.length - 1] || 1000;
            newFrequencies.push(Math.round(Math.min(lastFreq * 2, maxFreq)));
            newTypes.push('peaking');
            newQs.push(1.0);
            newGains.push(0);
            newChannels.push('stereo');
        }

        // Through the setter, so the band-count event fires for the UI.
        if (count !== this.bandCount) this.setBandCount(count);

        this.frequencies = newFrequencies;
        this.currentTypes = newTypes;
        this.currentQs = newQs;
        this.currentGains = newGains;
        this.currentChannels = newChannels;
        this.msEnabled = newChannels.some((ch) => ch === 'mid' || ch === 'side');

        if (!skipPreamp) this.setPreamp(preamp);

        equalizerSettings.setCustomFrequencies(this.frequencies);
        equalizerSettings.setGains(this.currentGains);
        equalizerSettings.setBandTypes(this.currentTypes);
        equalizerSettings.setBandQs(this.currentQs);
        equalizerSettings.setBandChannels(this.currentChannels);
        this.scheduleDspPush();

        const lines = [`Preamp: ${this.preamp.toFixed(1)} dB`];
        sortedBands.forEach((band, index) => {
            if (index >= count) return;
            const filterType = band.type === 'lowshelf' ? 'LSC' : band.type === 'highshelf' ? 'HSC' : 'PK';
            lines.push(
                `Filter ${index + 1}: ON ${filterType} Fc ${newFrequencies[index]} Hz Gain ${newGains[index].toFixed(1)} dB Q ${newQs[index].toFixed(2)}`
            );
        });
        return lines.join('\n');
    }

    exportEQToText() {
        const lines = [`Preamp: ${this.getPreamp().toFixed(1)} dB`];
        this.frequencies.forEach((freq, index) => {
            const gain = this.currentGains[index] || 0;
            const type = (this.currentTypes && this.currentTypes[index]) || 'peaking';
            const filterType = type === 'lowshelf' ? 'LSC' : type === 'highshelf' ? 'HSC' : 'PK';
            const q = this.currentQs && this.currentQs[index] > 0 ? this.currentQs[index] : this._calculateQ(index);
            lines.push(
                `Filter ${index + 1}: ON ${filterType} Fc ${freq} Hz Gain ${gain.toFixed(1)} dB Q ${q.toFixed(2)}`
            );
        });
        return lines.join('\n');
    }

    /** @returns {boolean} true if the text held at least one filter */
    importEQFromText(text) {
        try {
            const lines = text
                .split('\n')
                .map((line) => line.trim())
                .filter((line) => line);
            const filters = [];
            let preamp = 0;

            for (const line of lines) {
                const preampMatch = line.match(/^Preamp:\s*([+-]?\d+\.?\d*)\s*dB$/i);
                if (preampMatch) {
                    preamp = parseFloat(preampMatch[1]);
                    continue;
                }
                const filterMatch = line.match(
                    /^Filter\s*\d*:\s*ON\s+(\w+)\s+Fc\s+(\d+)\s+Hz\s+Gain\s*([+-]?\d+\.?\d*)\s*dB(?:\s+Q\s+(\d+\.?\d*))?/i
                );
                if (filterMatch) {
                    filters.push({
                        type: filterMatch[1].toUpperCase(),
                        freq: parseInt(filterMatch[2], 10),
                        gain: parseFloat(filterMatch[3]),
                        q: filterMatch[4] ? parseFloat(filterMatch[4]) : Math.SQRT1_2,
                    });
                }
            }

            if (filters.length === 0) {
                console.warn('[AudioContext] No valid filters found in import text');
                return false;
            }

            this.setPreamp(preamp);
            const newCount = Math.max(
                equalizerSettings.MIN_BANDS,
                Math.min(equalizerSettings.MAX_BANDS, filters.length)
            );
            if (newCount !== this.bandCount) this.setBandCount(newCount);

            const sliced = filters.slice(0, this.bandCount);
            const typeMap = {
                PK: 'peaking',
                LS: 'lowshelf',
                LSC: 'lowshelf',
                LSF: 'lowshelf',
                HS: 'highshelf',
                HSC: 'highshelf',
                HSF: 'highshelf',
            };
            const padCount = this.bandCount - sliced.length;
            const freqs = sliced.map((f) => f.freq);
            const types = sliced.map((f) => typeMap[f.type] || 'peaking');
            const qs = sliced.map((f) => f.q);
            const gains = sliced.map((f) => this._clampGain(f.gain));
            if (padCount > 0) {
                const lastFreq = freqs[freqs.length - 1] || 1000;
                const maxFreq = SAMPLE_RATE / 2 - 1;
                for (let p = 0; p < padCount; p++) {
                    freqs.push(Math.round(Math.min(lastFreq * Math.pow(2, p + 1), maxFreq)));
                    types.push('peaking');
                    qs.push(this._calculateQ(freqs.length - 1));
                    gains.push(0);
                }
            }

            this.frequencies = freqs;
            this.currentTypes = types;
            this.currentQs = qs;
            this.currentGains = gains;
            // Imported text has no channel info.
            this.currentChannels = new Array(this.bandCount).fill('stereo');
            this.msEnabled = false;

            equalizerSettings.setCustomFrequencies(this.frequencies);
            equalizerSettings.setGains(this.currentGains);
            equalizerSettings.setBandTypes(this.currentTypes);
            equalizerSettings.setBandQs(this.currentQs);
            equalizerSettings.setBandChannels(this.currentChannels);
            this.scheduleDspPush();
            return true;
        } catch (e) {
            console.warn('[AudioContext] Failed to import EQ settings:', e);
            return false;
        }
    }

    // ---- graphic EQ (its own chain after the parametric one) -------------------

    toggleGraphicEQ(enabled) {
        this.isGraphicEQEnabled = enabled;
        equalizerSettings.setGraphicEqEnabled(enabled);
        this.scheduleDspPush();
    }

    setGraphicEqBandGain(bandIndex, gainDb) {
        if (bandIndex < 0 || bandIndex >= this.geqBandCount) return;
        this.geqGains[bandIndex] = Math.max(-30, Math.min(30, gainDb));
        equalizerSettings.setGraphicEqGains([...this.geqGains]);
        this.scheduleDspPush();
    }

    setGraphicEqAllGains(gains) {
        if (!Array.isArray(gains)) return;
        gains.forEach((g, i) => {
            if (i < this.geqBandCount) this.geqGains[i] = Math.max(-30, Math.min(30, g));
        });
        equalizerSettings.setGraphicEqGains([...this.geqGains]);
        this.scheduleDspPush();
    }

    setGraphicEqBandCount(count) {
        const newCount = Math.max(3, Math.min(32, parseInt(count, 10) || 16));
        if (newCount === this.geqBandCount) return;
        this.geqGains = equalizerSettings.interpolateGains(this.geqGains, newCount);
        this.geqBandCount = newCount;
        this.geqFrequencies = generateFrequencies(newCount, this.geqFreqRange.min, this.geqFreqRange.max);
        equalizerSettings.setGraphicEqBandCount(newCount);
        equalizerSettings.setGraphicEqGains(this.geqGains);
        this.scheduleDspPush();
    }

    setGraphicEqFreqRange(minFreq, maxFreq) {
        const newMin = Math.max(10, Math.min(96000, parseInt(minFreq, 10) || 25));
        const newMax = Math.max(10, Math.min(96000, parseInt(maxFreq, 10) || 20000));
        if (newMin >= newMax) return;
        if (newMin === this.geqFreqRange.min && newMax === this.geqFreqRange.max) return;
        this.geqFreqRange = { min: newMin, max: newMax };
        this.geqFrequencies = generateFrequencies(this.geqBandCount, newMin, newMax);
        equalizerSettings.setGraphicEqFreqRange(newMin, newMax);
        this.scheduleDspPush();
    }

    getGraphicEqFrequencies() {
        return this.geqFrequencies;
    }

    getGraphicEqBandCount() {
        return this.geqBandCount;
    }

    setGraphicEqPreamp(db) {
        this.geqPreamp = Math.max(-20, Math.min(20, parseFloat(db) || 0));
        equalizerSettings.setGraphicEqPreamp(this.geqPreamp);
        this.scheduleDspPush();
    }
}

export const audioContextManager = new AudioContextManager();

// Presets and helpers for the settings UI
export {
    EQ_PRESETS,
    generateFrequencies,
    generateFrequencyLabels,
    getPresetsForBandCount,
    interpolatePreset,
    EQ_PRESETS_16,
};
