//! Visualizer analysis (§3.5): the DSP element copies its output into a
//! [`Tap`] while someone is subscribed, and [`Analyzer`] turns the newest
//! 1024 samples into a frame with Web Audio `AnalyserNode` semantics
//! (fftSize 1024, Blackman window, smoothing 0.7, -100…-30 dB → bytes).

use std::sync::Arc;
use std::sync::Mutex;

use realfft::{RealFftPlanner, RealToComplex};

pub const FFT_SIZE: usize = 1024;
pub const FREQ_BINS: usize = FFT_SIZE / 2;
pub const FRAME_BYTES: usize = 24 + FREQ_BINS + 3 * FFT_SIZE;
const SAMPLE_RATE: u32 = 48000;
/// Samples kept: enough to look back over the sink's buffer.
const RING: usize = 16384;
const SMOOTHING: f64 = 0.7;
const MIN_DB: f64 = -100.0;
const MAX_DB: f64 = -30.0;

pub struct Ring {
    buf: Vec<[f32; 2]>,
    write: usize,
    filled: usize,
    /// Stream time (s) just after the newest sample, when known.
    end_time: Option<f64>,
}

impl Ring {
    #[inline]
    pub fn push(&mut self, l: f32, r: f32) {
        self.buf[self.write] = [l, r];
        self.write = (self.write + 1) % RING;
        self.filled = (self.filled + 1).min(RING);
    }

    pub fn set_end_time(&mut self, t: Option<f64>) {
        self.end_time = t;
    }

    pub fn clear(&mut self) {
        self.filled = 0;
        self.end_time = None;
    }
}

/// Shared between the streaming thread (writer) and the analyzer.
pub struct Tap {
    ring: Mutex<Ring>,
}

impl Default for Tap {
    fn default() -> Self {
        Self {
            ring: Mutex::new(Ring {
                buf: vec![[0.0; 2]; RING],
                write: 0,
                filled: 0,
                end_time: None,
            }),
        }
    }
}

impl Tap {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// For the streaming thread: never blocks; `None` if the analyzer is reading.
    pub fn try_ring(&self) -> Option<std::sync::MutexGuard<'_, Ring>> {
        self.ring.try_lock().ok()
    }

    pub fn clear(&self) {
        if let Ok(mut r) = self.ring.lock() {
            r.clear();
        }
    }

    /// Copies the `FFT_SIZE` samples that are audible at `position` (the
    /// ring runs ahead of the sink by its buffer) into `out`.
    fn window(&self, position: Option<f64>, out: &mut [[f32; 2]]) -> bool {
        let Ok(r) = self.ring.lock() else {
            return false;
        };
        if r.filled < out.len() {
            return false;
        }
        let ahead = match (r.end_time, position) {
            (Some(end), Some(pos)) => ((end - pos).max(0.0) * f64::from(SAMPLE_RATE)) as usize,
            _ => 0,
        };
        let ahead = ahead.min(r.filled - out.len());
        let end = (r.write + RING - ahead) % RING;
        let start = (end + RING - out.len()) % RING;
        for (i, o) in out.iter_mut().enumerate() {
            *o = r.buf[(start + i) % RING];
        }
        true
    }
}

pub struct Analyzer {
    fft: Arc<dyn RealToComplex<f64>>,
    window: Vec<f64>,
    smoothed: Vec<f64>,
    samples: Vec<[f32; 2]>,
    input: Vec<f64>,
    spectrum: Vec<realfft::num_complex::Complex<f64>>,
    scratch: Vec<realfft::num_complex::Complex<f64>>,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Self {
        let fft = RealFftPlanner::<f64>::new().plan_fft_forward(FFT_SIZE);
        let n = FFT_SIZE as f64;
        // Blackman with alpha 0.16, as the Web Audio spec defines it.
        let window = (0..FFT_SIZE)
            .map(|i| {
                let x = 2.0 * std::f64::consts::PI * i as f64 / n;
                0.42 - 0.5 * x.cos() + 0.08 * (2.0 * x).cos()
            })
            .collect();
        Self {
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            input: fft.make_input_vec(),
            fft,
            window,
            smoothed: vec![0.0; FREQ_BINS],
            samples: vec![[0.0; 2]; FFT_SIZE],
        }
    }

    /// One §3.5 frame. Without data (paused, no tap yet) the data is zeroed
    /// (time domain at 128) and the smoothing decays.
    pub fn frame(&mut self, tap: Option<&Tap>, playing: bool, position: f64) -> Vec<u8> {
        let have = playing && tap.is_some_and(|t| t.window(Some(position), &mut self.samples));
        if !have {
            self.samples.iter_mut().for_each(|s| *s = [0.0; 2]);
        }
        let mut out = Vec::with_capacity(FRAME_BYTES);
        out.extend_from_slice(b"VVIS");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(FREQ_BINS as u16).to_le_bytes());
        out.extend_from_slice(&(FFT_SIZE as u16).to_le_bytes());
        out.extend_from_slice(&u16::from(playing).to_le_bytes());
        out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        out.extend_from_slice(&position.to_le_bytes());

        for (i, (x, s)) in self.input.iter_mut().zip(&self.samples).enumerate() {
            *x = 0.5 * (f64::from(s[0]) + f64::from(s[1])) * self.window[i];
        }
        let ok = self
            .fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .is_ok();
        let n = FFT_SIZE as f64;
        for k in 0..FREQ_BINS {
            let mag = if ok { self.spectrum[k].norm() / n } else { 0.0 };
            let v = SMOOTHING * self.smoothed[k] + (1.0 - SMOOTHING) * mag;
            self.smoothed[k] = if v.is_finite() { v } else { 0.0 };
            out.push(db_byte(self.smoothed[k]));
        }
        let time_byte = |x: f64| (128.0 * (1.0 + x)).floor().clamp(0.0, 255.0) as u8;
        out.extend(
            self.samples
                .iter()
                .map(|s| time_byte(0.5 * (f64::from(s[0]) + f64::from(s[1])))),
        );
        out.extend(self.samples.iter().map(|s| time_byte(f64::from(s[0]))));
        out.extend(self.samples.iter().map(|s| time_byte(f64::from(s[1]))));
        debug_assert_eq!(out.len(), FRAME_BYTES);
        out
    }
}

fn db_byte(mag: f64) -> u8 {
    if mag <= 0.0 {
        return 0;
    }
    let db = 20.0 * mag.log10();
    let scaled = 255.0 / (MAX_DB - MIN_DB) * (db - MIN_DB);
    scaled.floor().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_layout() {
        let mut a = Analyzer::new();
        let f = a.frame(None, false, 12.5);
        assert_eq!(f.len(), 3608);
        assert_eq!(&f[0..4], b"VVIS");
        assert_eq!(u16::from_le_bytes([f[4], f[5]]), 1);
        assert_eq!(u16::from_le_bytes([f[6], f[7]]), 512);
        assert_eq!(u16::from_le_bytes([f[8], f[9]]), 1024);
        assert_eq!(u16::from_le_bytes([f[10], f[11]]), 0);
        assert_eq!(u32::from_le_bytes(f[12..16].try_into().unwrap()), 48000);
        assert_eq!(f64::from_le_bytes(f[16..24].try_into().unwrap()), 12.5);
        assert!(f[24..536].iter().all(|b| *b == 0));
        assert!(f[536..].iter().all(|b| *b == 128));
    }

    #[test]
    fn sine_peaks_in_its_bin() {
        let tap = Tap::new();
        {
            let mut r = tap.try_ring().unwrap();
            // 1500 Hz = bin 32 at 48 kHz / 1024.
            for i in 0..4096 {
                let x = (0.5 * (2.0 * std::f64::consts::PI * 1500.0 * f64::from(i) / 48000.0).sin())
                    as f32;
                r.push(x, x);
            }
        }
        let mut a = Analyzer::new();
        let mut f = Vec::new();
        for _ in 0..20 {
            f = a.frame(Some(&tap), true, 0.0);
        }
        assert_eq!(u16::from_le_bytes([f[10], f[11]]), 1);
        let freq = &f[24..536];
        // The window's main lobe saturates the bytes around bin 32.
        let first_peak = freq.iter().position(|v| *v == 255).unwrap();
        assert!((30..=32).contains(&first_peak), "{first_peak}");
        // A 0.5 sine windowed: 20·log10(0.5·0.42/2) ≈ -19.6 dB, above -30 → 255.
        assert_eq!(freq[32], 255);
        assert!(freq[200] < 50);
        let time = &f[536..1560];
        assert!(time.iter().any(|b| *b > 180) && time.iter().any(|b| *b < 76));
    }

    #[test]
    fn looks_back_by_the_sink_delay() {
        let tap = Tap::new();
        {
            let mut r = tap.try_ring().unwrap();
            for _ in 0..2048 {
                r.push(0.5, 0.5);
            }
            for _ in 0..2048 {
                r.push(0.0, 0.0);
            }
            // Newest sample is at 10 s; the listener hears 9.95 s.
            r.set_end_time(Some(10.0));
        }
        let mut out = vec![[0.0f32; 2]; FFT_SIZE];
        assert!(tap.window(Some(10.0), &mut out));
        assert!(out.iter().all(|s| s[0] == 0.0));
        // 2048 samples ≈ 42.7 ms back: the window is in the loud part.
        assert!(tap.window(Some(10.0 - 2048.0 / 48000.0 - 0.001), &mut out));
        assert!(out.iter().all(|s| s[0] == 0.5));
    }
}
