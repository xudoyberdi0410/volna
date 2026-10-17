//! The processing chain of `volnadsp`, in the order of `_connectGraph` in
//! `ui/js/audio-context.js`: mono → binaural (crossfeed, widening) →
//! [EQ preamp → EQ, stereo or M/S] → [GEQ preamp → GEQ].
//!
//! [`Params`] is built from a [`DspConfig`] in the actor and swapped in;
//! each element instance keeps its own [`Runtime`] (filter state). A change
//! with the same topology ramps the parameters over 20 ms, recomputing the
//! coefficients every 64 samples, and keeps the filter state; a topology
//! change (band count, M/S, a stage on/off) crossfades the old and new
//! chains over 20 ms.

use std::sync::Arc;

use super::biquad::{Coefs, State};
use crate::model::{BandChannel, BandType, CrossfeedLevel, DspConfig};

pub const SAMPLE_RATE: f64 = 48000.0;
/// Parameter ramps and chain crossfades last this many samples (20 ms).
const RAMP: usize = 960;
/// Coefficients are recomputed this often during a ramp.
const STEP: usize = 64;
const MAX_DELAY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    pub kind: BandType,
    pub freq: f64,
    pub gain_db: f64,
    pub q: f64,
}

impl Band {
    fn coefs(&self) -> Coefs {
        Coefs::eq(self.kind, self.freq, self.gain_db, self.q, SAMPLE_RATE)
    }

    fn lerp(a: &Band, b: &Band, t: f64) -> Band {
        Band {
            kind: b.kind,
            // Frequencies move on a log scale.
            freq: (a.freq.ln() + (b.freq.ln() - a.freq.ln()) * t).exp(),
            gain_db: a.gain_db + (b.gain_db - a.gain_db) * t,
            q: a.q + (b.q - a.q) * t,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crossfeed {
    pub cutoff: f64,
    pub cross_gain: f64,
    pub direct_gain: f64,
    pub delay: usize,
}

impl Crossfeed {
    /// Presets of `binaural-dsp.js` (cutoff Hz, cross gain dB, delay ms).
    pub fn preset(level: CrossfeedLevel) -> Self {
        let (cutoff, db, ms) = match level {
            CrossfeedLevel::Low => (500.0, -6.0, 0.2),
            CrossfeedLevel::Medium => (700.0, -4.5, 0.3),
            CrossfeedLevel::High => (1000.0, -3.0, 0.4),
        };
        let cross_gain = 10f64.powf(db / 20.0);
        Self {
            cutoff,
            cross_gain,
            direct_gain: 1.0 - cross_gain * 0.5,
            delay: ((ms / 1000.0 * SAMPLE_RATE).round() as usize).clamp(1, MAX_DELAY - 1),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Eq {
    pub preamp: f64,
    /// M/S topology: `mid` and `side` chains instead of one stereo chain.
    pub ms: bool,
    /// Stereo chain, or the mid chain when `ms`.
    pub main: Vec<Band>,
    pub side: Vec<Band>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    pub mono: bool,
    pub crossfeed: Option<Crossfeed>,
    /// (mid gain, side gain).
    pub widening: Option<(f64, f64)>,
    pub eq: Option<Eq>,
    pub geq: Option<Eq>,
}

impl Default for Params {
    fn default() -> Self {
        Self::from_config(&DspConfig::default())
    }
}

fn db_to_lin(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

impl Params {
    pub fn from_config(c: &DspConfig) -> Self {
        let binaural = c.binaural.enabled;
        let eq = (c.eq.enabled && !c.eq.bands.is_empty()).then(|| {
            let band = |b: &crate::model::EqBand| Band {
                kind: b.kind,
                freq: b.freq,
                gain_db: b.gain_db,
                q: b.q,
            };
            let ms = c.eq.bands.iter().any(|b| b.channel != BandChannel::Stereo);
            if ms {
                Eq {
                    preamp: db_to_lin(c.eq.preamp_db),
                    ms,
                    main: c
                        .eq
                        .bands
                        .iter()
                        .filter(|b| b.channel != BandChannel::Side)
                        .map(band)
                        .collect(),
                    side: c
                        .eq
                        .bands
                        .iter()
                        .filter(|b| b.channel != BandChannel::Mid)
                        .map(band)
                        .collect(),
                }
            } else {
                Eq {
                    preamp: db_to_lin(c.eq.preamp_db),
                    ms,
                    main: c.eq.bands.iter().map(band).collect(),
                    side: vec![],
                }
            }
        });
        let geq = (c.graphic_eq.enabled && !c.graphic_eq.bands.is_empty()).then(|| Eq {
            preamp: db_to_lin(c.graphic_eq.preamp_db),
            ms: false,
            main: c
                .graphic_eq
                .bands
                .iter()
                .map(|b| Band {
                    kind: BandType::Peaking,
                    freq: b.freq,
                    gain_db: b.gain_db,
                    q: b.q,
                })
                .collect(),
            side: vec![],
        });
        Self {
            mono: c.mono,
            crossfeed: (binaural && c.binaural.crossfeed.enabled)
                .then(|| Crossfeed::preset(c.binaural.crossfeed.level)),
            widening: (binaural && c.binaural.widening.enabled).then(|| {
                let a = c.binaural.widening.amount;
                (2.0 - a, a)
            }),
            eq,
            geq,
        }
    }

    /// Nothing to do: the element only applies its gains.
    pub fn is_bypass(&self) -> bool {
        !self.mono
            && self.crossfeed.is_none()
            && self.widening.is_none()
            && self.eq.is_none()
            && self.geq.is_none()
    }

    fn same_topology(&self, o: &Params) -> bool {
        let eq_shape = |e: &Option<Eq>| {
            e.as_ref().map(|e| {
                (
                    e.ms,
                    e.main.iter().map(|b| b.kind).collect::<Vec<_>>(),
                    e.side.iter().map(|b| b.kind).collect::<Vec<_>>(),
                )
            })
        };
        self.mono == o.mono
            && self.crossfeed.map(|c| c.delay) == o.crossfeed.map(|c| c.delay)
            && self.widening.is_some() == o.widening.is_some()
            && eq_shape(&self.eq) == eq_shape(&o.eq)
            && eq_shape(&self.geq) == eq_shape(&o.geq)
    }

    /// Parameters `t` of the way from `a` to `b` (same topology).
    fn lerp(a: &Params, b: &Params, t: f64) -> Params {
        let l = |x: f64, y: f64| x + (y - x) * t;
        let eq = |a: &Option<Eq>, b: &Option<Eq>| match (a, b) {
            (Some(a), Some(b)) => Some(Eq {
                preamp: l(a.preamp, b.preamp),
                ms: b.ms,
                main: a
                    .main
                    .iter()
                    .zip(&b.main)
                    .map(|(x, y)| Band::lerp(x, y, t))
                    .collect(),
                side: a
                    .side
                    .iter()
                    .zip(&b.side)
                    .map(|(x, y)| Band::lerp(x, y, t))
                    .collect(),
            }),
            _ => b.clone(),
        };
        Params {
            mono: b.mono,
            crossfeed: match (a.crossfeed, b.crossfeed) {
                (Some(x), Some(y)) => Some(Crossfeed {
                    cutoff: (x.cutoff.ln() + (y.cutoff.ln() - x.cutoff.ln()) * t).exp(),
                    cross_gain: l(x.cross_gain, y.cross_gain),
                    direct_gain: l(x.direct_gain, y.direct_gain),
                    delay: y.delay,
                }),
                (_, y) => y,
            },
            widening: match (a.widening, b.widening) {
                (Some(x), Some(y)) => Some((l(x.0, y.0), l(x.1, y.1))),
                (_, y) => y,
            },
            eq: eq(&a.eq, &b.eq),
            geq: eq(&a.geq, &b.geq),
        }
    }
}

/// Derived coefficients of one [`Params`].
#[derive(Debug, Clone)]
struct Coefficients {
    cf_lp: Coefs,
    eq_main: Vec<Coefs>,
    eq_side: Vec<Coefs>,
    geq: Vec<Coefs>,
}

impl Coefficients {
    fn of(p: &Params) -> Self {
        let bands = |e: &Option<Eq>, side: bool| {
            e.as_ref()
                .map(|e| {
                    if side { &e.side } else { &e.main }
                        .iter()
                        .map(Band::coefs)
                        .collect()
                })
                .unwrap_or_default()
        };
        Self {
            cf_lp: p.crossfeed.map_or(Coefs::IDENTITY, |c| {
                Coefs::lowpass(c.cutoff, std::f64::consts::FRAC_1_SQRT_2, SAMPLE_RATE)
            }),
            eq_main: bands(&p.eq, false),
            eq_side: bands(&p.eq, true),
            geq: bands(&p.geq, false),
        }
    }
}

/// Filter state of one chain topology.
#[derive(Debug, Clone)]
struct Filters {
    cf_lp: [State; 2],
    cf_delay: [[f64; MAX_DELAY]; 2],
    cf_pos: usize,
    /// [band][channel]: L/R for stereo, M/S for M/S.
    eq_main: Vec<[State; 2]>,
    eq_side: Vec<State>,
    geq: Vec<[State; 2]>,
}

impl Filters {
    fn new(c: &Coefficients) -> Self {
        Self {
            cf_lp: Default::default(),
            cf_delay: [[0.0; MAX_DELAY]; 2],
            cf_pos: 0,
            eq_main: vec![Default::default(); c.eq_main.len()],
            eq_side: vec![State::default(); c.eq_side.len()],
            geq: vec![Default::default(); c.geq.len()],
        }
    }
}

/// One chain with its filter state and the ramp toward its target.
#[derive(Debug, Clone)]
struct Stage {
    target: Arc<Params>,
    current: Params,
    coefs: Coefficients,
    filters: Filters,
    ramp: Option<(Params, usize)>,
}

impl Stage {
    fn new(p: Arc<Params>) -> Self {
        let coefs = Coefficients::of(&p);
        Self {
            filters: Filters::new(&coefs),
            current: (*p).clone(),
            coefs,
            target: p,
            ramp: None,
        }
    }

    fn retarget(&mut self, p: Arc<Params>) {
        self.ramp = Some((self.current.clone(), 0));
        self.target = p;
    }

    /// Advances the ramp by `n` samples.
    fn step(&mut self, n: usize) {
        let Some((from, pos)) = &mut self.ramp else {
            return;
        };
        *pos += n;
        if *pos >= RAMP {
            self.current = (*self.target).clone();
            self.ramp = None;
        } else {
            self.current = Params::lerp(from, &self.target, *pos as f64 / RAMP as f64);
        }
        self.coefs = Coefficients::of(&self.current);
    }

    #[inline]
    fn process(&mut self, mut l: f64, mut r: f64) -> (f64, f64) {
        let p = &self.current;
        let c = &self.coefs;
        let f = &mut self.filters;
        if p.mono {
            let m = 0.5 * (l + r);
            l = m;
            r = m;
        }
        if let Some(cf) = p.crossfeed {
            let lp_l = f.cf_lp[0].process(&c.cf_lp, l);
            let lp_r = f.cf_lp[1].process(&c.cf_lp, r);
            let read = (f.cf_pos + MAX_DELAY - cf.delay) % MAX_DELAY;
            let (d_l, d_r) = (f.cf_delay[0][read], f.cf_delay[1][read]);
            f.cf_delay[0][f.cf_pos] = lp_l;
            f.cf_delay[1][f.cf_pos] = lp_r;
            f.cf_pos = (f.cf_pos + 1) % MAX_DELAY;
            let (nl, nr) = (
                cf.direct_gain * l + cf.cross_gain * d_r,
                cf.direct_gain * r + cf.cross_gain * d_l,
            );
            l = nl;
            r = nr;
        }
        if let Some((mid_gain, side_gain)) = p.widening {
            let m = 0.5 * (l + r) * mid_gain;
            let s = 0.5 * (l - r) * side_gain;
            l = m + s;
            r = m - s;
        }
        if let Some(eq) = &p.eq {
            l *= eq.preamp;
            r *= eq.preamp;
            if eq.ms {
                let mut m = 0.5 * (l + r);
                let mut s = 0.5 * (l - r);
                for (st, co) in f.eq_main.iter_mut().zip(&c.eq_main) {
                    m = st[0].process(co, m);
                }
                for (st, co) in f.eq_side.iter_mut().zip(&c.eq_side) {
                    s = st.process(co, s);
                }
                l = m + s;
                r = m - s;
            } else {
                for (st, co) in f.eq_main.iter_mut().zip(&c.eq_main) {
                    l = st[0].process(co, l);
                    r = st[1].process(co, r);
                }
            }
        }
        if let Some(geq) = &p.geq {
            l *= geq.preamp;
            r *= geq.preamp;
            for (st, co) in f.geq.iter_mut().zip(&c.geq) {
                l = st[0].process(co, l);
                r = st[1].process(co, r);
            }
        }
        (l, r)
    }
}

/// Per-element processing state.
#[derive(Debug)]
pub struct Runtime {
    stage: Stage,
    /// The chain being faded out after a topology change, and samples done.
    old: Option<(Stage, usize)>,
}

impl Runtime {
    pub fn new(p: Arc<Params>) -> Self {
        Self {
            stage: Stage::new(p),
            old: None,
        }
    }

    pub fn target(&self) -> &Arc<Params> {
        &self.stage.target
    }

    /// Moves toward new parameters, glitch-free.
    pub fn set(&mut self, p: Arc<Params>) {
        if Arc::ptr_eq(&p, &self.stage.target) {
            return;
        }
        if self.stage.current.same_topology(&p) {
            self.stage.retarget(p);
        } else {
            let old = std::mem::replace(&mut self.stage, Stage::new(p));
            self.old = Some((old, 0));
        }
    }

    /// True when the chain does nothing and no transition is running.
    pub fn idle(&self) -> bool {
        self.old.is_none() && self.stage.ramp.is_none() && self.stage.current.is_bypass()
    }

    /// Processes interleaved stereo in place. `tap` sees the output.
    pub fn process(&mut self, frames: &mut [f32], mut tap: impl FnMut(f32, f32)) {
        if self.idle() {
            for fr in frames.as_chunks::<2>().0 {
                tap(fr[0], fr[1]);
            }
            return;
        }
        for block in frames.chunks_mut(STEP * 2) {
            for fr in block.as_chunks_mut::<2>().0 {
                let (l, r) = (f64::from(fr[0]), f64::from(fr[1]));
                let (mut ol, mut or) = self.stage.process(l, r);
                if let Some((old, pos)) = &mut self.old {
                    let (xl, xr) = old.process(l, r);
                    let t = *pos as f64 / RAMP as f64;
                    ol = ol * t + xl * (1.0 - t);
                    or = or * t + xr * (1.0 - t);
                    *pos += 1;
                }
                fr[0] = ol as f32;
                fr[1] = or as f32;
                tap(fr[0], fr[1]);
            }
            let n = block.len() / 2;
            self.stage.step(n);
            if let Some((old, pos)) = &mut self.old {
                old.step(n);
                if *pos >= RAMP {
                    self.old = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BinauralConfig, CrossfeedConfig, EqBand, EqConfig, WideningConfig};

    fn tone(f: f64, n: usize, amp: f64) -> Vec<f32> {
        (0..n)
            .flat_map(|i| {
                let x =
                    (amp * (2.0 * std::f64::consts::PI * f * i as f64 / SAMPLE_RATE).sin()) as f32;
                [x, x]
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    fn eq_config(gain: f64, channel: BandChannel) -> DspConfig {
        DspConfig {
            eq: EqConfig {
                enabled: true,
                preamp_db: 0.0,
                bands: vec![EqBand {
                    freq: 1000.0,
                    gain_db: gain,
                    q: 1.0,
                    kind: BandType::Peaking,
                    channel,
                }],
            },
            ..Default::default()
        }
    }

    #[test]
    fn default_config_is_bypass() {
        let p = Params::default();
        assert!(p.is_bypass());
        let mut rt = Runtime::new(Arc::new(p));
        let mut buf = tone(440.0, 1000, 0.5);
        let orig = buf.clone();
        let mut taps = 0;
        rt.process(&mut buf, |_, _| taps += 1);
        assert_eq!(buf, orig);
        assert_eq!(taps, 1000);
    }

    #[test]
    fn peaking_band_boosts_its_frequency() {
        let mut rt = Runtime::new(Arc::new(Params::from_config(&eq_config(
            6.0,
            BandChannel::Stereo,
        ))));
        let mut buf = tone(1000.0, 48000, 0.25);
        rt.process(&mut buf, |_, _| {});
        let gain = 20.0 * (rms(&buf[48000..]) / rms(&tone(1000.0, 24000, 0.25))).log10();
        assert!((gain - 6.0).abs() < 0.1, "{gain}");
    }

    #[test]
    fn side_band_leaves_mono_content_alone() {
        let mut rt = Runtime::new(Arc::new(Params::from_config(&eq_config(
            12.0,
            BandChannel::Side,
        ))));
        let mut buf = tone(1000.0, 24000, 0.25);
        let orig = buf.clone();
        rt.process(&mut buf, |_, _| {});
        let err: f64 = buf
            .iter()
            .zip(&orig)
            .map(|(a, b)| f64::from(a - b).abs())
            .fold(0.0, f64::max);
        assert!(err < 1e-5, "{err}");
    }

    #[test]
    fn mono_and_widening() {
        let mut c = DspConfig {
            mono: true,
            ..Default::default()
        };
        let mut rt = Runtime::new(Arc::new(Params::from_config(&c)));
        let mut buf = vec![1.0f32, 0.0, 0.5, -0.5];
        rt.process(&mut buf, |_, _| {});
        assert_eq!(buf, [0.5, 0.5, 0.0, 0.0]);

        c.mono = false;
        c.binaural = BinauralConfig {
            enabled: true,
            crossfeed: CrossfeedConfig {
                enabled: false,
                ..Default::default()
            },
            widening: WideningConfig {
                enabled: true,
                amount: 1.0,
            },
        };
        let p = Params::from_config(&c);
        assert_eq!(p.widening, Some((1.0, 1.0)));
        let mut rt = Runtime::new(Arc::new(p));
        let mut buf = vec![0.3f32, -0.1];
        rt.process(&mut buf, |_, _| {});
        assert!((buf[0] - 0.3).abs() < 1e-6 && (buf[1] + 0.1).abs() < 1e-6);
        c.binaural.widening.amount = 2.0;
        let mut rt = Runtime::new(Arc::new(Params::from_config(&c)));
        let mut buf = vec![1.0f32, 0.0];
        rt.process(&mut buf, |_, _| {});
        // mid 0.5 × 0, side 0.5 × 2: L = 1, R = -1.
        assert_eq!(buf, [1.0, -1.0]);
    }

    #[test]
    fn crossfeed_leaks_a_delayed_lowpassed_copy() {
        let c = DspConfig {
            binaural: BinauralConfig {
                enabled: true,
                crossfeed: CrossfeedConfig {
                    enabled: true,
                    level: CrossfeedLevel::Medium,
                },
                widening: WideningConfig {
                    enabled: false,
                    amount: 1.0,
                },
            },
            ..Default::default()
        };
        let p = Params::from_config(&c);
        let cf = p.crossfeed.unwrap();
        assert_eq!(cf.delay, 14);
        let mut rt = Runtime::new(Arc::new(p));
        // Left-only low tone: the right channel gets ~cross_gain of it.
        let mut buf: Vec<f32> = (0..48000)
            .flat_map(|i| {
                let x = (0.5 * (2.0 * std::f64::consts::PI * 100.0 * i as f64 / SAMPLE_RATE).sin())
                    as f32;
                [x, 0.0]
            })
            .collect();
        rt.process(&mut buf, |_, _| {});
        let tail = &buf[48000..];
        let l: Vec<f32> = tail.iter().step_by(2).copied().collect();
        let r: Vec<f32> = tail.iter().skip(1).step_by(2).copied().collect();
        let ratio = rms(&r) / rms(&l);
        let want = cf.cross_gain / cf.direct_gain;
        assert!((ratio - want).abs() < 0.02, "{ratio} vs {want}");
    }

    #[test]
    fn transitions_are_smooth() {
        let a = Arc::new(Params::from_config(&eq_config(0.0, BandChannel::Stereo)));
        let b = Arc::new(Params::from_config(&eq_config(12.0, BandChannel::Stereo)));
        let mut rt = Runtime::new(a);
        let mut buf = tone(1000.0, 4800, 0.25);
        rt.process(&mut buf[..4800], |_, _| {});
        rt.set(b.clone());
        assert!(rt.old.is_none(), "same topology ramps");
        rt.process(&mut buf[4800..], |_, _| {});
        // No sample-to-sample jump bigger than the tone itself makes.
        let max_jump = buf
            .as_chunks::<2>()
            .0
            .iter()
            .map(|f| f[0])
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_jump < 0.25, "{max_jump}");
        // Topology change crossfades.
        let c = Arc::new(Params::from_config(&eq_config(3.0, BandChannel::Mid)));
        rt.set(c);
        assert!(rt.old.is_some());
        let mut more = tone(1000.0, 2000, 0.25);
        rt.process(&mut more, |_, _| {});
        assert!(rt.old.is_none());
        assert!(more.iter().all(|v| v.is_finite()));
    }
}
