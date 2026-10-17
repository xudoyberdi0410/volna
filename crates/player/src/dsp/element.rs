//! The `volnadsp` GStreamer element (feature `gstreamer`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwapOption;
use gst::glib;
use gst::prelude::*;
use gst::subclass::prelude::*;
use gst_base::prelude::*;
use gst_base::subclass::prelude::*;

use super::Shared;
use super::chain::{Runtime, SAMPLE_RATE};
use crate::analysis::Tap;
use crate::output::Curve;

#[derive(Debug, Clone, Copy)]
struct FadeCmd {
    from: Option<f64>,
    to: f64,
    samples: usize,
    curve: Curve,
}

#[derive(Debug, Clone, Copy)]
struct Ramp {
    from: f64,
    to: f64,
    pos: usize,
    len: usize,
    curve: Curve,
}

/// Master volume smoothing: about a 10 ms time constant.
const MASTER_K: f64 = 1.0 / (0.010 * SAMPLE_RATE);

struct Rt {
    runtime: Option<Runtime>,
    fade: f64,
    ramp: Option<Ramp>,
    master: Option<f64>,
}

mod imp {
    use super::*;

    pub struct VolnaDsp {
        pub(super) shared: ArcSwapOption<Shared>,
        pub(super) tap: ArcSwapOption<Tap>,
        pub(super) pending: Mutex<Option<FadeCmd>>,
        pub(super) has_pending: AtomicBool,
        pub(super) rt: Mutex<Rt>,
    }

    impl Default for VolnaDsp {
        fn default() -> Self {
            Self {
                shared: ArcSwapOption::empty(),
                tap: ArcSwapOption::empty(),
                pending: Mutex::new(None),
                has_pending: AtomicBool::new(false),
                rt: Mutex::new(Rt {
                    runtime: None,
                    fade: 1.0,
                    ramp: None,
                    master: None,
                }),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VolnaDsp {
        const NAME: &'static str = "VolnaDsp";
        type Type = super::VolnaDsp;
        type ParentType = gst_base::BaseTransform;
    }

    impl ObjectImpl for VolnaDsp {}
    impl GstObjectImpl for VolnaDsp {}

    impl ElementImpl for VolnaDsp {
        fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
            static META: std::sync::OnceLock<gst::subclass::ElementMetadata> =
                std::sync::OnceLock::new();
            Some(META.get_or_init(|| {
                gst::subclass::ElementMetadata::new(
                    "Volna DSP",
                    "Filter/Effect/Audio",
                    "EQ, crossfeed, widening, fades and volume of the Volna player",
                    "Volna",
                )
            }))
        }

        fn pad_templates() -> &'static [gst::PadTemplate] {
            static TEMPLATES: std::sync::OnceLock<Vec<gst::PadTemplate>> =
                std::sync::OnceLock::new();
            TEMPLATES.get_or_init(|| {
                let caps = super::caps();
                ["src", "sink"]
                    .into_iter()
                    .map(|name| {
                        let dir = if name == "src" {
                            gst::PadDirection::Src
                        } else {
                            gst::PadDirection::Sink
                        };
                        gst::PadTemplate::new(name, dir, gst::PadPresence::Always, &caps)
                            .expect("static pad template")
                    })
                    .collect()
            })
        }
    }

    impl BaseTransformImpl for VolnaDsp {
        const MODE: gst_base::subclass::BaseTransformMode =
            gst_base::subclass::BaseTransformMode::AlwaysInPlace;
        const PASSTHROUGH_ON_SAME_CAPS: bool = false;
        const TRANSFORM_IP_ON_PASSTHROUGH: bool = false;

        fn transform_ip(
            &self,
            buf: &mut gst::BufferRef,
        ) -> Result<gst::FlowSuccess, gst::FlowError> {
            let end_time = self.end_time(buf);
            let mut map = buf.map_writable().map_err(|_| gst::FlowError::Error)?;
            // SAFETY: any bit pattern is a valid f32.
            let (pre, samples, _) = unsafe { map.as_mut_slice().align_to_mut::<f32>() };
            if !pre.is_empty() {
                // GStreamer allocates audio aligned; refuse anything else.
                return Err(gst::FlowError::Error);
            }
            self.process(samples, end_time);
            Ok(gst::FlowSuccess::Ok)
        }
    }

    impl VolnaDsp {
        fn end_time(&self, buf: &gst::BufferRef) -> Option<f64> {
            let pts = buf.pts()?;
            let dur = buf.duration().unwrap_or(gst::ClockTime::ZERO);
            let seg = self.obj().segment();
            let seg = seg.downcast_ref::<gst::ClockTime>()?;
            let st = seg.to_stream_time(pts + dur)?;
            Some(st.nseconds() as f64 / 1e9)
        }

        fn process(&self, samples: &mut [f32], end_time: Option<f64>) {
            let Ok(mut rt) = self.rt.lock() else {
                return;
            };
            let rt = &mut *rt;
            let shared = self.shared.load_full();
            if let Some(shared) = &shared {
                let params = shared.params.load_full();
                match &mut rt.runtime {
                    Some(r) => r.set(params),
                    None => rt.runtime = Some(Runtime::new(params)),
                }
            }
            let frames = samples.len() / 2;
            let samples = &mut samples[..frames * 2];
            let tap = self.tap.load_full();
            let mut ring = tap.as_ref().and_then(|t| t.try_ring());
            if let Some(r) = ring.as_mut() {
                r.set_end_time(end_time);
            }
            match &mut rt.runtime {
                Some(runtime) => runtime.process(samples, |l, r| {
                    if let Some(ring) = ring.as_mut() {
                        ring.push(l, r);
                    }
                }),
                None => {
                    if let Some(ring) = ring.as_mut() {
                        for f in samples.as_chunks::<2>().0 {
                            ring.push(f[0], f[1]);
                        }
                    }
                }
            }
            drop(ring);

            if self.has_pending.swap(false, Ordering::AcqRel)
                && let Ok(mut p) = self.pending.lock()
                && let Some(cmd) = p.take()
            {
                let from = cmd.from.unwrap_or(rt.fade);
                if cmd.samples == 0 {
                    rt.fade = cmd.to;
                    rt.ramp = None;
                } else {
                    rt.fade = from;
                    rt.ramp = Some(Ramp {
                        from,
                        to: cmd.to,
                        pos: 0,
                        len: cmd.samples,
                        curve: cmd.curve,
                    });
                }
            }
            let target = shared.as_ref().map_or(1.0, |s| s.master());
            let mut master = rt.master.unwrap_or(target);
            let mut fade = rt.fade;
            for f in samples.as_chunks_mut::<2>().0 {
                if let Some(r) = &mut rt.ramp {
                    r.pos += 1;
                    if r.pos >= r.len {
                        fade = r.to;
                        rt.ramp = None;
                    } else {
                        fade =
                            r.from + (r.to - r.from) * r.curve.shape(r.pos as f64 / r.len as f64);
                    }
                }
                master += (target - master) * MASTER_K;
                let g = (fade * master) as f32;
                f[0] *= g;
                f[1] *= g;
            }
            rt.fade = fade;
            rt.master = Some(if (master - target).abs() < 1e-6 {
                target
            } else {
                master
            });
        }
    }
}

glib::wrapper! {
    pub struct VolnaDsp(ObjectSubclass<imp::VolnaDsp>)
        @extends gst_base::BaseTransform, gst::Element, gst::Object;
}

/// The only format the element takes.
pub fn caps() -> gst::Caps {
    gst::Caps::builder("audio/x-raw")
        .field("format", "F32LE")
        .field("layout", "interleaved")
        .field("channels", 2i32)
        .field("rate", SAMPLE_RATE as i32)
        .build()
}

impl VolnaDsp {
    pub fn new(shared: Arc<Shared>) -> Self {
        let el: Self = glib::Object::new();
        el.imp().shared.store(Some(shared));
        el
    }

    /// Ramps the fade gain to `to` over `seconds` (0 = right away), from the
    /// current gain or from `from`.
    pub fn fade(&self, from: Option<f64>, to: f64, seconds: f64, curve: Curve) {
        let imp = self.imp();
        if let Ok(mut p) = imp.pending.lock() {
            *p = Some(FadeCmd {
                from,
                to,
                samples: (seconds.max(0.0) * SAMPLE_RATE) as usize,
                curve,
            });
        }
        imp.has_pending.store(true, Ordering::Release);
    }

    pub fn set_tap(&self, tap: Option<Arc<Tap>>) {
        if let Some(t) = &tap {
            t.clear();
        }
        self.imp().tap.store(tap);
    }
}

/// Registers `volnadsp` (static, no plugin file).
pub fn register() -> Result<(), glib::BoolError> {
    gst::Element::register(None, "volnadsp", gst::Rank::NONE, VolnaDsp::static_type())
}
