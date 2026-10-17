//! `volnadsp`: an in-place BaseTransform on F32LE interleaved stereo at
//! 48 kHz (fixed by the capsfilter in front of it) that runs the EQ chain,
//! feeds the visualizer tap, then applies the per-deck fade and the master
//! volume (smoothed, so slider drags do not crackle).
//!
//! The fade lives here instead of a `volume` element so that crossfades,
//! the sleep fade and the de-click on skip/pause are sample-accurate.
//! The element itself needs the `gstreamer` feature; the EQ maths do not.

pub mod biquad;
pub mod chain;
#[cfg(feature = "gstreamer")]
mod element;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwap;

use crate::model::DspConfig;
use chain::Params;

pub use crate::output::Curve;
#[cfg(feature = "gstreamer")]
pub use element::{VolnaDsp, caps, register};

/// Parameters shared by every deck's element.
pub struct Shared {
    params: ArcSwap<Params>,
    /// Target master gain (f64 bits).
    master: AtomicU64,
}

impl Shared {
    pub fn new(config: &DspConfig, master: f64) -> Arc<Self> {
        Arc::new(Self {
            params: ArcSwap::from_pointee(Params::from_config(config)),
            master: AtomicU64::new(master.to_bits()),
        })
    }

    pub fn set_config(&self, config: &DspConfig) {
        self.params.store(Arc::new(Params::from_config(config)));
    }

    pub fn set_master(&self, gain: f64) {
        self.master.store(gain.to_bits(), Ordering::Relaxed);
    }

    #[cfg_attr(not(feature = "gstreamer"), allow(dead_code))]
    fn master(&self) -> f64 {
        f64::from_bits(self.master.load(Ordering::Relaxed))
    }
}
