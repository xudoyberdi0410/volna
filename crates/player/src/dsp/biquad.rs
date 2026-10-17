//! RBJ cookbook biquads in transposed direct form II with f64 state.
//!
//! Shelves use Q (alpha = sin w0 / 2Q), exactly like `_biquadResponseDb` in
//! `ui/js/audio-context.js` and the AutoEQ fitter, so what the EQ graph draws
//! is what is heard (WebAudio ignored Q for shelves; §8 of the design).

use std::f64::consts::PI;

use crate::model::BandType;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coefs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coefs {
    pub const IDENTITY: Coefs = Coefs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        let k = 1.0 / a0;
        Self {
            b0: b0 * k,
            b1: b1 * k,
            b2: b2 * k,
            a1: a1 * k,
            a2: a2 * k,
        }
    }

    fn clamp_freq(freq: f64, sr: f64) -> f64 {
        freq.clamp(1.0, sr / 2.0 - 1.0)
    }

    pub fn eq(kind: BandType, freq: f64, gain_db: f64, q: f64, sr: f64) -> Self {
        let w = 2.0 * PI * Self::clamp_freq(freq, sr) / sr;
        let s = w.sin() / (2.0 * q.max(1e-3));
        let a = 10f64.powf(gain_db / 40.0);
        let c = w.cos();
        match kind {
            BandType::Peaking => Self::normalized(
                1.0 + s * a,
                -2.0 * c,
                1.0 - s * a,
                1.0 + s / a,
                -2.0 * c,
                1.0 - s / a,
            ),
            BandType::Lowshelf => {
                let sq = 2.0 * a.sqrt() * s;
                Self::normalized(
                    a * (a + 1.0 - (a - 1.0) * c + sq),
                    2.0 * a * (a - 1.0 - (a + 1.0) * c),
                    a * (a + 1.0 - (a - 1.0) * c - sq),
                    a + 1.0 + (a - 1.0) * c + sq,
                    -2.0 * (a - 1.0 + (a + 1.0) * c),
                    a + 1.0 + (a - 1.0) * c - sq,
                )
            }
            BandType::Highshelf => {
                let sq = 2.0 * a.sqrt() * s;
                Self::normalized(
                    a * (a + 1.0 + (a - 1.0) * c + sq),
                    -2.0 * a * (a - 1.0 + (a + 1.0) * c),
                    a * (a + 1.0 + (a - 1.0) * c - sq),
                    a + 1.0 - (a - 1.0) * c + sq,
                    2.0 * (a - 1.0 - (a + 1.0) * c),
                    a + 1.0 - (a - 1.0) * c - sq,
                )
            }
        }
    }

    pub fn lowpass(freq: f64, q: f64, sr: f64) -> Self {
        let w = 2.0 * PI * Self::clamp_freq(freq, sr) / sr;
        let s = w.sin() / (2.0 * q.max(1e-3));
        let c = w.cos();
        Self::normalized(
            (1.0 - c) / 2.0,
            1.0 - c,
            (1.0 - c) / 2.0,
            1.0 + s,
            -2.0 * c,
            1.0 - s,
        )
    }

    /// Magnitude response at `f` Hz, in dB.
    pub fn response_db(&self, f: f64, sr: f64) -> f64 {
        let p = 2.0 * PI * f / sr;
        let (cp, c2p) = (p.cos(), (2.0 * p).cos());
        let Coefs { b0, b1, b2, a1, a2 } = *self;
        let n = b0 * b0 + b1 * b1 + b2 * b2 + 2.0 * (b0 * b1 + b1 * b2) * cp + 2.0 * b0 * b2 * c2p;
        let d = 1.0 + a1 * a1 + a2 * a2 + 2.0 * (a1 + a1 * a2) * cp + 2.0 * a2 * c2p;
        10.0 * (n / d).log10()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct State {
    s1: f64,
    s2: f64,
}

impl State {
    #[inline]
    pub fn process(&mut self, c: &Coefs, x: f64) -> f64 {
        let y = c.b0 * x + self.s1;
        self.s1 = c.b1 * x - c.a1 * y + self.s2;
        self.s2 = c.b2 * x - c.a2 * y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48000.0;

    /// Values printed by `_biquadResponseDb` of ui/js/audio-context.js
    /// (sr 48000) for the same bands.
    const JS_REFERENCE: &[(BandType, f64, f64, f64, f64, f64)] = &[
        // (type, freq, gain, q, at f, dB)
        (BandType::Peaking, 1000.0, 6.0, 1.0, 1000.0, 6.000000),
        (BandType::Peaking, 1000.0, 6.0, 1.0, 500.0, 1.879381),
        (BandType::Peaking, 100.0, -9.0, 4.0, 110.0, -5.594797),
        (BandType::Lowshelf, 105.0, 5.5, 0.7, 20.0, 5.488164),
        (BandType::Lowshelf, 105.0, 5.5, 0.7, 1000.0, 0.001964),
        (BandType::Highshelf, 8000.0, -4.0, 0.7, 16000.0, -3.940787),
        (BandType::Highshelf, 8000.0, -4.0, 2.0, 6000.0, 1.985916),
    ];

    #[test]
    fn matches_the_ui_response_formula() {
        for &(kind, freq, gain, q, f, want) in JS_REFERENCE {
            let got = Coefs::eq(kind, freq, gain, q, SR).response_db(f, SR);
            assert!(
                (got - want).abs() < 1e-3,
                "{kind:?} {freq} Hz {gain} dB q {q} at {f}: {got} != {want}"
            );
        }
    }

    /// Runs a sine through the filter and measures the gain.
    fn measured_db(c: &Coefs, f: f64) -> f64 {
        let mut st = State::default();
        let n = 48000;
        let (mut e_in, mut e_out) = (0.0, 0.0);
        for i in 0..n {
            let x = (2.0 * PI * f * i as f64 / SR).sin();
            let y = st.process(c, x);
            if i > n / 2 {
                e_in += x * x;
                e_out += y * y;
            }
        }
        10.0 * (e_out / e_in).log10()
    }

    #[test]
    fn time_domain_agrees_with_response() {
        for &(kind, freq, gain, q, f, _) in JS_REFERENCE {
            let c = Coefs::eq(kind, freq, gain, q, SR);
            let want = c.response_db(f, SR);
            let got = measured_db(&c, f);
            assert!((got - want).abs() < 0.05, "{kind:?} at {f}: {got} vs {want}");
        }
    }

    #[test]
    fn lowpass_and_identity() {
        let lp = Coefs::lowpass(700.0, std::f64::consts::FRAC_1_SQRT_2, SR);
        assert!(lp.response_db(50.0, SR).abs() < 0.01);
        assert!((lp.response_db(700.0, SR) + 3.01).abs() < 0.05);
        assert!(lp.response_db(10000.0, SR) < -40.0);
        assert!(Coefs::IDENTITY.response_db(1234.0, SR).abs() < 1e-12);
        // Frequencies at or past Nyquist are clamped, not NaN.
        let c = Coefs::eq(BandType::Peaking, 30000.0, 3.0, 1.0, SR);
        assert!(c.response_db(1000.0, SR).is_finite());
    }
}
