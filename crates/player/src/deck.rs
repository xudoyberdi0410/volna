//! One `playbin3` with the Volna audio-filter bin and the system sink.
//!
//! A deck lives for one continuous playback: an explicit load makes a new
//! one (so stale bus messages are told apart by deck id) and the old one
//! goes to NULL when dropped. Gapless transitions stay on the same deck.

use std::sync::{Arc, Mutex};

use gst::prelude::*;
use tokio::sync::mpsc::UnboundedSender;

use crate::analysis::Tap;
use crate::dsp::{self, VolnaDsp};

pub type DeckId = u64;

/// souphttpsrc `timeout` (s) for the loopback audio server.
const HTTP_TIMEOUT_S: u32 = 60;

/// From GStreamer threads to the engine.
#[derive(Debug)]
pub enum DeckMsg {
    Bus(DeckId, gst::Message),
    /// `about-to-finish` handed the next URI to the deck.
    AboutToFinish(DeckId),
}

/// What `about-to-finish` switches to (gapless).
#[derive(Debug, Clone)]
pub struct NextUp {
    pub uid: String,
    pub uri: String,
}

pub struct Deck {
    pub id: DeckId,
    playbin: gst::Element,
    pub dsp: VolnaDsp,
    /// The entry the listener hears (updated on gapless transitions).
    pub uid: String,
    /// Reached PAUSED once (`AsyncDone`).
    pub prerolled: bool,
    pub pending_seek: Option<f64>,
    pub play_when_ready: bool,
    /// A first `StreamStart` was seen.
    pub started: bool,
    next_up: Arc<Mutex<Option<NextUp>>>,
    switched_to: Arc<Mutex<Option<String>>>,
}

fn sink_props() -> gst::Structure {
    gst::Structure::builder("props")
        .field("media.role", "Music")
        .field("application.name", "Volna")
        .field("application.icon-name", "volna")
        .field("node.name", "volna")
        .build()
}

fn make(name: &str) -> Result<gst::Element, String> {
    gst::ElementFactory::make(name)
        .build()
        .map_err(|e| format!("GStreamer element {name}: {e}"))
}

fn filter_bin(
    shared: Arc<dsp::Shared>,
    preserve_pitch: bool,
) -> Result<(gst::Bin, VolnaDsp), String> {
    let bin = gst::Bin::new();
    let mut chain = vec![make("audioconvert")?];
    if preserve_pitch {
        chain.push(make("scaletempo")?);
        chain.push(make("audioconvert")?);
    }
    chain.push(make("audioresample")?);
    chain.push(
        gst::ElementFactory::make("capsfilter")
            .property("caps", dsp::caps())
            .build()
            .map_err(|e| e.to_string())?,
    );
    let dsp = VolnaDsp::new(shared);
    chain.push(dsp.clone().upcast());
    bin.add_many(&chain).map_err(|e| e.to_string())?;
    gst::Element::link_many(&chain).map_err(|e| e.to_string())?;
    let first = chain.first().expect("non-empty");
    let last = chain.last().expect("non-empty");
    let sink = gst::GhostPad::with_target(&first.static_pad("sink").expect("sink pad"))
        .map_err(|e| e.to_string())?;
    let src = gst::GhostPad::with_target(&last.static_pad("src").expect("src pad"))
        .map_err(|e| e.to_string())?;
    bin.add_pad(&sink).map_err(|e| e.to_string())?;
    bin.add_pad(&src).map_err(|e| e.to_string())?;
    Ok((bin, dsp))
}

fn audio_sink() -> Result<gst::Element, String> {
    // Tests and headless checks: e.g. `VOLNA_PLAYER_SINK="fakesink sync=true"`.
    if let Ok(desc) = std::env::var("VOLNA_PLAYER_SINK")
        && !desc.trim().is_empty()
    {
        return gst::parse::bin_from_description(&desc, true)
            .map(|b| b.upcast())
            .map_err(|e| format!("VOLNA_PLAYER_SINK: {e}"));
    }
    let sink = make("autoaudiosink")?;
    // The real sink (pulsesink / pipewiresink) appears inside on READY.
    if let Some(bin) = sink.downcast_ref::<gst::Bin>() {
        bin.connect_deep_element_added(|_, _, el| {
            if el.find_property("stream-properties").is_some() {
                el.set_property("stream-properties", sink_props());
            }
            if el.find_property("client-name").is_some() {
                el.set_property("client-name", "Volna");
            }
        });
    }
    Ok(sink)
}

impl Deck {
    pub fn new(
        id: DeckId,
        uid: String,
        uri: &str,
        shared: Arc<dsp::Shared>,
        preserve_pitch: bool,
        tx: UnboundedSender<DeckMsg>,
    ) -> Result<Self, String> {
        let name = format!("volna-deck{id}");
        let playbin = gst::ElementFactory::make("playbin3")
            .name(&name)
            .property("uri", uri)
            .build()
            .map_err(|e| format!("GStreamer playbin3: {e}"))?;
        playbin.set_property_from_str("flags", "audio");
        let (filter, dsp) = filter_bin(shared, preserve_pitch)?;
        playbin.set_property("audio-filter", &filter);
        playbin.set_property("audio-sink", audio_sink()?);

        let bus = playbin.bus().ok_or("playbin3 without a bus")?;
        let bus_tx = tx.clone();
        bus.set_sync_handler(move |_, msg| {
            use gst::MessageView as V;
            let forward = match msg.view() {
                V::Eos(_)
                | V::Error(_)
                | V::AsyncDone(_)
                | V::StreamStart(_)
                | V::Buffering(_)
                | V::DurationChanged(_) => true,
                V::StateChanged(_) => msg.src().is_some_and(|s| s.name() == name),
                V::Warning(w) => {
                    tracing::debug!(deck = id, "gst warning: {} ({:?})", w.error(), w.debug());
                    false
                }
                _ => false,
            };
            if forward {
                let _ = bus_tx.send(DeckMsg::Bus(id, msg.clone()));
            }
            gst::BusSyncReply::Drop
        });

        // The audio server answers only once yt-dlp resolved the stream,
        // which through a proxy can take longer than souphttpsrc's 15 s.
        playbin.connect("source-setup", false, |args| {
            if let Ok(src) = args[1].get::<gst::Element>()
                && src.find_property("timeout").is_some()
            {
                src.set_property("timeout", HTTP_TIMEOUT_S);
            }
            None
        });

        let next_up: Arc<Mutex<Option<NextUp>>> = Arc::default();
        let switched_to: Arc<Mutex<Option<String>>> = Arc::default();
        {
            let next_up = next_up.clone();
            let switched_to = switched_to.clone();
            playbin.connect("about-to-finish", false, move |args| {
                let Ok(pb) = args[0].get::<gst::Element>() else {
                    return None;
                };
                let next = next_up.lock().ok().and_then(|mut n| n.take());
                if let Some(next) = next {
                    pb.set_property("uri", &next.uri);
                    if let Ok(mut s) = switched_to.lock() {
                        *s = Some(next.uid);
                    }
                    let _ = tx.send(DeckMsg::AboutToFinish(id));
                }
                None
            });
        }

        Ok(Self {
            id,
            playbin,
            dsp,
            uid,
            prerolled: false,
            pending_seek: None,
            play_when_ready: false,
            started: false,
            next_up,
            switched_to,
        })
    }

    pub fn set_state(&self, state: gst::State) {
        if let Err(e) = self.playbin.set_state(state) {
            tracing::warn!(deck = self.id, "cannot set {state:?}: {e}");
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playbin.current_state() == gst::State::Playing
    }

    pub fn position(&self) -> Option<f64> {
        self.playbin
            .query_position::<gst::ClockTime>()
            .map(|t| t.nseconds() as f64 / 1e9)
    }

    pub fn duration(&self) -> Option<f64> {
        self.playbin
            .query_duration::<gst::ClockTime>()
            .map(|t| t.nseconds() as f64 / 1e9)
            .filter(|d| *d > 0.0)
    }

    /// Flushing, accurate seek; also how the rate changes.
    pub fn seek(&self, position: f64, rate: f64) -> bool {
        let pos = gst::ClockTime::from_nseconds((position.max(0.0) * 1e9) as u64);
        self.playbin
            .seek(
                rate,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                pos,
                gst::SeekType::None,
                gst::ClockTime::NONE,
            )
            .inspect_err(|e| tracing::debug!(deck = self.id, "seek failed: {e}"))
            .is_ok()
    }

    pub fn set_next_up(&self, next: Option<NextUp>) {
        if let Ok(mut n) = self.next_up.lock() {
            *n = next;
        }
    }

    /// The uid `about-to-finish` switched to, if a gapless switch is pending.
    pub fn take_switched(&self) -> Option<String> {
        self.switched_to.lock().ok().and_then(|mut s| s.take())
    }

    pub fn set_tap(&self, tap: Option<Arc<Tap>>) {
        self.dsp.set_tap(tap);
    }
}

impl Drop for Deck {
    fn drop(&mut self) {
        if let Some(bus) = self.playbin.bus() {
            bus.unset_sync_handler();
        }
        let _ = self.playbin.set_state(gst::State::Null);
    }
}
