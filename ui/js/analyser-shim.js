// js/analyser-shim.js
// An AnalyserNode look-alike fed by the native player's visualizer frames
// (docs/native-player.md §3.5), so the visualizer presets keep calling
// getByteFrequencyData/getByteTimeDomainData as they did with Web Audio.

export const FRAME_BYTES = 3608;
export const FREQ_BINS = 512;
export const TIME_LEN = 1024;
/** Frames older than this read as silence. */
export const STALE_MS = 250;

const OFF_FREQ = 24;
const OFF_TIME = OFF_FREQ + FREQ_BINS;
const OFF_TIME_L = OFF_TIME + TIME_LEN;
const OFF_TIME_R = OFF_TIME_L + TIME_LEN;

/** Parses a frame; null when it is not a version-1 "VVIS" frame. */
export function parseFrame(buffer) {
    if (!(buffer instanceof ArrayBuffer) || buffer.byteLength < FRAME_BYTES) return null;
    const bytes = new Uint8Array(buffer);
    if (bytes[0] !== 0x56 || bytes[1] !== 0x56 || bytes[2] !== 0x49 || bytes[3] !== 0x53) return null;
    const view = new DataView(buffer);
    if (view.getUint16(4, true) !== 1) return null;
    if (view.getUint16(6, true) !== FREQ_BINS || view.getUint16(8, true) !== TIME_LEN) return null;
    return {
        playing: (view.getUint16(10, true) & 1) === 1,
        sampleRate: view.getUint32(12, true),
        position: view.getFloat64(16, true),
        freq: bytes.subarray(OFF_FREQ, OFF_FREQ + FREQ_BINS),
        time: bytes.subarray(OFF_TIME, OFF_TIME + TIME_LEN),
        timeL: bytes.subarray(OFF_TIME_L, OFF_TIME_L + TIME_LEN),
        timeR: bytes.subarray(OFF_TIME_R, OFF_TIME_R + TIME_LEN),
    };
}

export class AnalyserShim {
    constructor({ now = () => performance.now() } = {}) {
        this.fftSize = TIME_LEN;
        this.frequencyBinCount = FREQ_BINS;
        this.smoothingTimeConstant = 0.7;
        this.minDecibels = -100;
        this.maxDecibels = -30;
        this.context = { sampleRate: 48000, state: 'running' };
        this._now = now;
        this._frame = null;
        this._at = 0;
        this._silentTime = new Uint8Array(TIME_LEN).fill(128);
    }

    /** Stores a frame from the channel. */
    push(buffer) {
        const frame = parseFrame(buffer);
        if (!frame) return false;
        this._frame = frame;
        this._at = this._now();
        this.context.sampleRate = frame.sampleRate || 48000;
        return true;
    }

    /** The newest frame, or null if none arrived recently. */
    get latest() {
        if (!this._frame || this._now() - this._at > STALE_MS) return null;
        return this._frame;
    }

    getByteFrequencyData(array) {
        const f = this.latest;
        if (!f) return array.fill(0);
        const n = Math.min(array.length, FREQ_BINS);
        array.set(f.freq.subarray(0, n));
        if (array.length > n) array.fill(0, n);
    }

    getByteTimeDomainData(array) {
        this._copyTime(array, this.latest?.time);
    }

    /** Left/right time-domain data for butterchurn. */
    getByteTimeDomainDataL(array) {
        this._copyTime(array, this.latest?.timeL);
    }

    getByteTimeDomainDataR(array) {
        this._copyTime(array, this.latest?.timeR);
    }

    _copyTime(array, src) {
        const data = src || this._silentTime;
        const n = Math.min(array.length, TIME_LEN);
        array.set(data.subarray(0, n));
        if (array.length > n) array.fill(128, n);
    }

    connect() {}
    disconnect() {}
}
