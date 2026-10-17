//! The player actor (§4.2): one task owns the queue, the decks, the play
//! tracker, the settings, the sleep timer and the continuation, and applies
//! commands in the order it receives them.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::analysis::{Analyzer, Tap};
use crate::deck::{Deck, DeckId, DeckMsg, NextUp};
use crate::dsp::{self, Curve};
use crate::hooks::{FailureInfo, Hooks, MoreRequest};
use crate::model::*;
use crate::persist::{
    self, Paths, Persisted, SavedContinuation, SavedPosition, SavedQueue, SavedSettings,
};
use crate::queue::{Queue, Removed};
use crate::tracker::{PlayEvent, PlayTracker};
use crate::{Error, PlayerEvent};

const TICK: Duration = Duration::from_millis(250);
/// From load to preroll. Includes the yt-dlp resolve behind the audio
/// server, which through a slow proxy takes well over 20 s at times.
const PREROLL_TIMEOUT: Duration = Duration::from_secs(45);
const SKIP_DELAY: Duration = Duration::from_millis(1200);
const PERSIST_DEBOUNCE: Duration = Duration::from_secs(2);
const POSITION_SAVE_EVERY: u32 = 40; // ticks = 10 s
const DECLICK_S: f64 = 0.03;
/// Pause after the de-click fade reached the speakers.
const PAUSE_DELAY: Duration = Duration::from_millis(130);
const SLEEP_FADE_S: f64 = 3.0;
const MAX_STREAK: u32 = 3;
const TOP_UP_AT: usize = 3;
const BATCH: usize = 5;
const PLAYED_MEMORY: usize = 100;
/// The standby deck starts loading this long before the crossfade.
const CROSSFADE_LEAD_S: f64 = 4.0;

type Reply<T> = oneshot::Sender<Result<T, Error>>;

/// A visualizer subscriber: gets each frame, returns false when gone.
pub type FrameSink = Box<dyn FnMut(Vec<u8>) -> bool + Send>;

/// The light state MPRIS, the tray and the visualizer read without a round trip.
#[derive(Debug, Clone, Default)]
pub struct NowPlaying {
    pub state: PlaybackState,
    pub track: Option<PlayerTrack>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub has_next: bool,
    pub has_prev: bool,
}

impl NowPlaying {
    /// Position interpolated to now.
    pub fn position_now(&self) -> f64 {
        let s = &self.state;
        if s.status != Status::Playing {
            return s.position;
        }
        let p = s.position + (now_ms() - s.at).max(0.0) / 1000.0 * s.rate;
        s.duration.map_or(p, |d| p.min(d))
    }
}

pub enum EnqueueAt {
    Next,
    End,
}

pub enum Command {
    Snapshot(Reply<PlayerSnapshot>),
    Play(Reply<()>),
    Pause(Reply<()>),
    Toggle(Reply<()>),
    Stop(Reply<()>),
    Next(Reply<()>),
    Prev(Reply<()>),
    Seek {
        position: f64,
        resume: bool,
        reply: Reply<()>,
    },
    PlayUid {
        uid: String,
        position: Option<f64>,
        reply: Reply<()>,
    },
    SetQueue {
        tracks: Vec<TrackInput>,
        start: usize,
        play: bool,
        position: Option<f64>,
        reply: Reply<u64>,
    },
    Enqueue {
        tracks: Vec<TrackInput>,
        at: EnqueueAt,
        reply: Reply<u64>,
    },
    Remove {
        uid: String,
        reply: Reply<u64>,
    },
    Move {
        uid: String,
        to: usize,
        reply: Reply<u64>,
    },
    Clear {
        keep_current: bool,
        reply: Reply<u64>,
    },
    SetShuffle {
        on: bool,
        reply: Reply<u64>,
    },
    SetRepeat {
        mode: Repeat,
        reply: Reply<u64>,
    },
    SetSettings {
        patch: serde_json::Map<String, serde_json::Value>,
        external: bool,
        reply: Reply<PlayerSettings>,
    },
    SetDsp {
        config: DspConfig,
        reply: Reply<()>,
    },
    SetSleep {
        minutes: Option<f64>,
        reply: Reply<Option<f64>>,
    },
    SetContinuation {
        kind: ContinuationKind,
        seeds: Vec<TrackInput>,
        reply: Reply<()>,
    },
    WaveStart {
        count: usize,
        reply: Reply<u64>,
    },
    WaveStop(Reply<()>),
    WaveDislike(Reply<()>),
    /// The wave settings changed: replace the queued wave tracks.
    WaveRequeue(Reply<u64>),
    VisualizerStart {
        fps: u32,
        sink: FrameSink,
        reply: Reply<()>,
    },
    VisualizerStop(Reply<()>),
    Shutdown(Reply<()>),
}

enum Timer {
    Preroll(DeckId),
    PauseDeck(DeckId, u64),
    Retire(DeckId),
    CrossfadeDone(DeckId),
    SkipAfterError(String),
    Persist(u64),
    Sleep(u64),
    SleepFadeDone(u64),
}

enum Input {
    Timer(Timer),
    More {
        epoch: u64,
        origin: Origin,
        result: Result<Vec<TrackInput>, String>,
    },
    WaveStarted {
        result: Result<Vec<TrackInput>, String>,
        reply: Reply<u64>,
    },
    Failure {
        uid: String,
        info: Option<FailureInfo>,
        fallback: FailureInfo,
    },
    StreamInfo {
        uid: String,
        info: Option<StreamInfo>,
    },
    RequeueWave,
    VisualizerGone(u64),
}

enum WorkerMsg {
    Event(Box<PlayEvent>),
    Flush(oneshot::Sender<()>),
}

struct Cont {
    kind: ContinuationKind,
    fetching: bool,
    seeds: Vec<PlayerTrack>,
    epoch: u64,
    /// Move on as soon as the fetch lands (the queue ran out).
    advance_after: bool,
}

struct Vis {
    epoch: u64,
    task: tokio::task::JoinHandle<()>,
}

pub struct Engine<H: Hooks> {
    hooks: Arc<H>,
    paths: Paths,
    input_tx: mpsc::UnboundedSender<Input>,
    deck_tx: mpsc::UnboundedSender<DeckMsg>,
    events: broadcast::Sender<PlayerEvent>,
    now_tx: watch::Sender<NowPlaying>,
    worker_tx: mpsc::UnboundedSender<WorkerMsg>,
    queue: Queue,
    cont: Cont,
    settings: PlayerSettings,
    dsp_config: DspConfig,
    dsp: Arc<dsp::Shared>,
    state: PlaybackState,
    published: Option<PlaybackState>,
    want_play: bool,
    active: Option<Deck>,
    standby: Option<Deck>,
    retiring: Vec<Deck>,
    next_deck: DeckId,
    pause_gen: u64,
    tracker: PlayTracker,
    played: VecDeque<String>,
    sleep: Option<(f64, u64)>,
    sleep_gen: u64,
    streak: u32,
    streak_id: Option<String>,
    retried: HashSet<String>,
    persist_gen: u64,
    queue_dirty: bool,
    ticks: u32,
    tap: Arc<Tap>,
    vis: Option<Vis>,
    vis_gen: u64,
    rng: fastrand::Rng,
}

fn bad(msg: impl Into<String>) -> Error {
    Error::new("bad_args", msg)
}

fn validate(tracks: &[TrackInput]) -> Result<(), Error> {
    match tracks.iter().position(|t| !is_video_id(&t.id)) {
        Some(i) => Err(bad(format!(
            "track {i} has no YouTube id ({:?})",
            tracks[i].id
        ))),
        None => Ok(()),
    }
}

/// `Hooks::more` errors are `"<code>: <message>"`.
fn hook_error(e: String) -> Error {
    match e.split_once(": ") {
        Some((code, msg)) if !code.contains(' ') => Error::new_owned(code.to_owned(), msg),
        _ => Error::new("internal", e),
    }
}

pub(crate) struct Channels {
    pub events: broadcast::Sender<PlayerEvent>,
    pub now: watch::Receiver<NowPlaying>,
    pub cmd: mpsc::UnboundedSender<Command>,
}

impl<H: Hooks> Engine<H> {
    pub(crate) fn spawn(hooks: H, paths: Paths) -> Channels {
        let persisted = Persisted::load(&paths);
        let hooks = Arc::new(hooks);
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let (deck_tx, deck_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(256);
        let (now_tx, now_rx) = watch::channel(NowPlaying::default());
        let (worker_tx, worker_rx) = mpsc::unbounded_channel();
        tokio::spawn(worker(hooks.clone(), worker_rx, input_tx.clone()));

        let Persisted {
            queue: saved,
            position,
            settings:
                SavedSettings {
                    settings,
                    dsp: dsp_config,
                    ..
                },
        } = persisted;
        let queue = Queue::from_state(saved.queue);
        let dsp = dsp::Shared::new(&dsp_config, settings.master_gain());
        let mut engine = Engine {
            hooks,
            paths,
            input_tx,
            deck_tx,
            events: events.clone(),
            now_tx,
            worker_tx,
            queue,
            cont: Cont {
                kind: saved.continuation.kind,
                fetching: false,
                seeds: saved.continuation.seeds,
                epoch: 0,
                advance_after: false,
            },
            state: PlaybackState {
                rate: settings.speed,
                volume: settings.volume,
                muted: settings.muted,
                at: now_ms(),
                ..Default::default()
            },
            settings,
            dsp_config,
            dsp,
            published: None,
            want_play: false,
            active: None,
            standby: None,
            retiring: Vec::new(),
            next_deck: 1,
            pause_gen: 0,
            tracker: PlayTracker::new(),
            played: saved.played.into_iter().collect(),
            sleep: None,
            sleep_gen: 0,
            streak: 0,
            streak_id: None,
            retried: HashSet::new(),
            persist_gen: 0,
            queue_dirty: false,
            ticks: 0,
            tap: Tap::new(),
            vis: None,
            vis_gen: 0,
            rng: fastrand::Rng::new(),
        };
        // Restored as paused at the saved position; nothing autoplays.
        if let Some(t) = engine.queue.current().cloned() {
            engine.state.uid = Some(t.uid.clone());
            engine.state.track_id = Some(t.id.clone());
            engine.state.duration = t.duration;
            engine.state.status = Status::Paused;
            if position.uid.as_deref() == Some(t.uid.as_str()) {
                engine.state.position = position.position.max(0.0);
            }
        }
        engine.publish_state();
        tokio::spawn(engine.run(cmd_rx, input_rx, deck_rx));
        Channels {
            events,
            now: now_rx,
            cmd: cmd_tx,
        }
    }

    async fn run(
        mut self,
        mut cmd_rx: mpsc::UnboundedReceiver<Command>,
        mut input_rx: mpsc::UnboundedReceiver<Input>,
        mut deck_rx: mpsc::UnboundedReceiver<DeckMsg>,
    ) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let ticking = self.active.is_some()
                && matches!(self.state.status, Status::Playing | Status::Loading);
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    Some(Command::Shutdown(reply)) => {
                        self.shutdown().await;
                        let _ = reply.send(Ok(()));
                        break;
                    }
                    Some(cmd) => self.command(cmd),
                    None => {
                        self.shutdown().await;
                        break;
                    }
                },
                Some(msg) = deck_rx.recv() => self.deck_msg(msg),
                Some(input) = input_rx.recv() => self.input(input),
                _ = tick.tick(), if ticking => self.on_tick(),
            }
        }
    }

    // ---- helpers -----------------------------------------------------------

    fn emit(&self, ev: PlayerEvent) {
        let _ = self.events.send(ev);
    }

    fn after(&self, d: Duration, t: Timer) {
        let tx = self.input_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(d).await;
            let _ = tx.send(Input::Timer(t));
        });
    }

    fn continuation(&self) -> Continuation {
        Continuation {
            kind: self.cont.kind,
            fetching: self.cont.fetching,
        }
    }

    fn playable(&self) -> impl Fn(&PlayerTrack) -> bool + '_ {
        |t: &PlayerTrack| !t.unavailable && !self.settings.blocklist.blocks(t)
    }

    fn quality(&self) -> Quality {
        self.settings.quality.into()
    }

    fn commit(&mut self) -> u64 {
        let c = self.queue.commit(self.continuation());
        let rev = c.rev;
        self.emit(PlayerEvent::Queue(c));
        self.mark_queue_dirty();
        self.publish_now();
        rev
    }

    fn mark_queue_dirty(&mut self) {
        self.queue_dirty = true;
        self.persist_gen += 1;
        self.after(PERSIST_DEBOUNCE, Timer::Persist(self.persist_gen));
    }

    fn sample_position(&mut self) {
        if let Some(d) = &self.active
            && d.prerolled
            && let Some(p) = d.position()
        {
            self.state.position = p;
        }
        self.state.at = now_ms();
    }

    fn publish_now(&self) {
        let track = self.queue.current().cloned();
        let has_next = self.queue.next_candidate(self.playable()).is_some()
            || self.cont.kind != ContinuationKind::None
            || self.settings.autoplay;
        let has_prev = self.queue.index().is_some();
        self.now_tx.send_replace(NowPlaying {
            state: self.state.clone(),
            track,
            shuffle: self.queue.shuffle(),
            repeat: self.queue.repeat,
            has_next,
            has_prev,
        });
    }

    /// `player:state` when anything but the position changed.
    fn publish_state(&mut self) {
        let strip = |s: &PlaybackState| PlaybackState {
            position: 0.0,
            at: 0.0,
            ..s.clone()
        };
        let changed = self
            .published
            .as_ref()
            .is_none_or(|p| strip(p) != strip(&self.state));
        if changed {
            self.published = Some(self.state.clone());
            self.emit(PlayerEvent::State(self.state.clone()));
        }
        self.publish_now();
    }

    fn emit_position(&mut self) {
        self.publish_now();
        self.emit(PlayerEvent::Position(PositionEvent {
            uid: self.state.uid.clone(),
            position: self.state.position,
            duration: self.state.duration,
            rate: self.state.rate,
            at: self.state.at,
            playing: self.state.status == Status::Playing,
        }));
    }

    fn play_events(&mut self, evs: Vec<PlayEvent>) {
        for ev in evs {
            if let PlayEvent::Start { track, .. } = &ev {
                let track = track.clone();
                self.on_play_started(&track);
            }
            let _ = self.worker_tx.send(WorkerMsg::Event(Box::new(ev)));
        }
    }

    fn in_wave(&self, t: &PlayerTrack) -> bool {
        self.cont.kind == ContinuationKind::Wave && t.origin == Origin::Wave
    }

    // ---- decks ---------------------------------------------------------------

    fn new_deck(&mut self, track: &PlayerTrack) -> Result<Deck, (ErrorKind, String)> {
        let uri = self.hooks.stream_uri(track, self.quality()).ok_or((
            ErrorKind::NoYtDlp,
            "playback is unavailable (no yt-dlp or audio server)".to_owned(),
        ))?;
        let id = self.next_deck;
        self.next_deck += 1;
        let deck = Deck::new(
            id,
            track.uid.clone(),
            &uri,
            self.dsp.clone(),
            self.settings.preserve_pitch,
            self.deck_tx.clone(),
        )
        .map_err(|e| (ErrorKind::Other, e))?;
        tracing::debug!(deck = id, uid = %track.uid, id = %track.id, "deck created");
        Ok(deck)
    }

    /// Fades a deck out (de-click) and drops it shortly after.
    fn retire(&mut self, deck: Deck, declick: bool) {
        deck.set_tap(None);
        if declick && deck.prerolled && deck.is_playing() {
            deck.dsp.fade(None, 0.0, DECLICK_S, Curve::Linear);
            deck.set_next_up(None);
            self.after(PAUSE_DELAY, Timer::Retire(deck.id));
            self.retiring.push(deck);
        }
        // Otherwise dropped here: NULL right away.
    }

    fn drop_decks(&mut self, declick: bool) {
        if let Some(d) = self.standby.take() {
            self.retire(d, false);
        }
        if let Some(d) = self.active.take() {
            self.retire(d, declick);
        }
        self.state.crossfading = false;
    }

    fn sync_tap(&self) {
        let enabled = self.vis.is_some();
        if let Some(d) = &self.active {
            d.set_tap(enabled.then(|| self.tap.clone()));
        }
    }

    /// The gapless successor for `about-to-finish`.
    fn update_next_up(&self) {
        let Some(deck) = &self.active else {
            return;
        };
        let next = if !self.settings.gapless || self.settings.crossfade.enabled {
            None
        } else if self.queue.repeat == Repeat::One {
            self.queue.current().cloned()
        } else {
            self.queue
                .next_candidate(self.playable())
                .and_then(|i| self.queue.at(i).cloned())
        };
        deck.set_next_up(next.and_then(|t| {
            Some(NextUp {
                uri: self.hooks.stream_uri(&t, self.quality())?,
                uid: t.uid,
            })
        }));
    }

    // ---- loading -------------------------------------------------------------

    fn set_index(&mut self, idx: Option<usize>) -> bool {
        if self.queue.index() != idx {
            self.queue.set_index(idx);
            self.commit();
            true
        } else {
            false
        }
    }

    fn set_current_state(&mut self, t: &PlayerTrack, position: f64) {
        if self.state.uid.as_deref() != Some(t.uid.as_str()) {
            self.state.error = None;
            self.state.stream = None;
        }
        self.state.uid = Some(t.uid.clone());
        self.state.track_id = Some(t.id.clone());
        self.state.duration = t.duration;
        self.state.position = position;
        self.state.at = now_ms();
        self.state.buffering = false;
    }

    /// Makes entry `idx` current without loading it (paused, no deck).
    fn cue(&mut self, idx: usize, position: f64) {
        self.set_index(Some(idx));
        let Some(t) = self.queue.at(idx).cloned() else {
            return;
        };
        let evs = self.tracker.switch_to(Some(&t.uid), now_ms());
        self.play_events(evs);
        self.drop_decks(true);
        self.want_play = false;
        self.set_current_state(&t, position);
        self.state.status = Status::Paused;
        self.publish_state();
        self.emit_position();
        self.save_position();
    }

    /// Loads entry `idx` on a fresh deck (or promotes a prerolled standby).
    fn load(&mut self, idx: usize, position: f64, play: bool) {
        self.set_index(Some(idx));
        let Some(t) = self.queue.at(idx).cloned() else {
            return;
        };
        let evs = self.tracker.switch_to(Some(&t.uid), now_ms());
        self.play_events(evs);
        let standby = self.standby.take();
        if let Some(d) = self.active.take() {
            self.retire(d, true);
        }
        self.state.crossfading = false;
        self.want_play = play;
        self.set_current_state(&t, position);

        if let Some(sb) = standby {
            if sb.uid == t.uid && sb.prerolled && position == 0.0 {
                tracing::debug!(deck = sb.id, "promoting the prerolled standby deck");
                sb.dsp.fade(Some(1.0), 1.0, 0.0, Curve::Linear);
                if play {
                    sb.set_state(gst::State::Playing);
                }
                self.state.status = if play {
                    Status::Loading
                } else {
                    Status::Paused
                };
                self.active = Some(sb);
                self.after_load(&t);
                return;
            }
            self.retire(sb, false);
        }

        match self.new_deck(&t) {
            Ok(mut deck) => {
                deck.pending_seek = (position > 0.0).then_some(position);
                deck.play_when_ready = play;
                deck.set_state(gst::State::Paused);
                self.after(PREROLL_TIMEOUT, Timer::Preroll(deck.id));
                self.active = Some(deck);
                self.state.status = if play {
                    Status::Loading
                } else {
                    Status::Paused
                };
            }
            Err((kind, message)) => {
                self.state.status = Status::Paused;
                self.want_play = false;
                self.report_error(&t, kind, message, ErrorAction::Stopped);
            }
        }
        self.after_load(&t);
    }

    fn after_load(&mut self, t: &PlayerTrack) {
        self.sync_tap();
        self.update_next_up();
        self.publish_state();
        self.emit_position();
        self.became_current(t);
    }

    /// At each track start (load or gapless/crossfade switch).
    fn became_current(&mut self, t: &PlayerTrack) {
        self.played.retain(|id| *id != t.id);
        self.played.push_back(t.id.clone());
        while self.played.len() > PLAYED_MEMORY {
            self.played.pop_front();
        }
        if let Some(next) = self
            .queue
            .next_candidate(self.playable())
            .and_then(|i| self.queue.at(i))
        {
            self.hooks.prefetch(next, self.quality());
        }
        let (hooks, tx, uid, id, q) = (
            self.hooks.clone(),
            self.input_tx.clone(),
            t.uid.clone(),
            t.id.clone(),
            self.quality(),
        );
        tokio::spawn(async move {
            // The audio server resolves first; give it a moment.
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let info = hooks.stream_info(&id, q).await;
            let _ = tx.send(Input::StreamInfo { uid, info });
        });
        self.ensure_continuation(true);
        self.save_position();
    }

    // ---- transport -------------------------------------------------------------

    fn play(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        if let Some(d) = &mut self.active {
            self.want_play = true;
            self.pause_gen += 1;
            if d.prerolled {
                d.dsp.fade(None, 1.0, DECLICK_S, Curve::Linear);
                d.set_state(gst::State::Playing);
            } else {
                d.play_when_ready = true;
                self.state.status = Status::Loading;
            }
            self.publish_state();
            return;
        }
        let idx = self.queue.index().unwrap_or(0);
        let pos = if self.state.status == Status::Ended || self.state.uid.is_none() {
            0.0
        } else {
            self.state.position
        };
        self.load(idx, pos, true);
    }

    fn pause(&mut self) {
        self.want_play = false;
        self.sample_position();
        let mut pause_deck = None;
        if let Some(d) = &mut self.active {
            if d.prerolled {
                d.dsp.fade(None, 0.0, DECLICK_S, Curve::Linear);
                pause_deck = Some(d.id);
            } else {
                d.play_when_ready = false;
            }
        }
        if let Some(id) = pause_deck {
            self.pause_gen += 1;
            self.after(PAUSE_DELAY, Timer::PauseDeck(id, self.pause_gen));
        }
        if matches!(self.state.status, Status::Playing | Status::Loading) {
            self.state.status = Status::Paused;
        }
        self.publish_state();
        self.emit_position();
        self.save_position();
    }

    fn seek(&mut self, position: f64) {
        let max = self.state.duration.unwrap_or(f64::MAX);
        let pos = position.clamp(0.0, max.max(0.0));
        match &mut self.active {
            Some(d) if d.prerolled => {
                d.seek(pos, self.settings.speed);
            }
            Some(d) => d.pending_seek = Some(pos),
            None => {}
        }
        self.state.position = pos;
        self.state.at = now_ms();
        self.emit(PlayerEvent::Seeked {
            uid: self.state.uid.clone(),
            position: pos,
        });
        self.emit_position();
    }

    fn stop(&mut self) {
        self.pause();
        self.seek(0.0);
        self.save_position();
    }

    /// `player_next` (explicit: repeat one does not hold it back).
    fn next(&mut self) {
        match self.queue.next_candidate(self.playable()) {
            Some(i) => self.load(i, 0.0, true),
            None if self.can_continue() => {
                self.cont.advance_after = true;
                self.state.status = Status::Loading;
                self.publish_state();
                self.ensure_continuation(false);
            }
            None => self.finish(),
        }
    }

    fn prev(&mut self) {
        self.sample_position();
        if self.state.position > 3.0 {
            self.seek(0.0);
            return;
        }
        match self.queue.prev_candidate(self.playable()) {
            Some(i) => {
                let play = self.want_play || self.active.is_none();
                self.load(i, 0.0, play);
            }
            None => self.seek(0.0),
        }
    }

    /// The queue ran out.
    fn finish(&mut self) {
        let evs = self.tracker.leave(now_ms());
        self.play_events(evs);
        self.drop_decks(false);
        self.want_play = false;
        self.state.status = Status::Ended;
        if let Some(d) = self.state.duration {
            self.state.position = d;
        }
        self.state.at = now_ms();
        self.publish_state();
        self.emit_position();
        self.save_position();
    }

    /// The active deck reached its end on its own.
    fn natural_end(&mut self) {
        if let Some(uid) = self.state.uid.clone() {
            let evs = self.tracker.ended(&uid, now_ms());
            self.play_events(evs);
        }
        if self.queue.repeat == Repeat::One
            && let Some(i) = self.queue.index()
            && self.queue.at(i).is_some_and(self.playable())
        {
            self.load(i, 0.0, true);
            return;
        }
        self.next();
    }

    fn go_idle(&mut self) {
        let evs = self.tracker.leave(now_ms());
        self.play_events(evs);
        self.drop_decks(true);
        self.want_play = false;
        self.state = PlaybackState {
            rate: self.settings.speed,
            volume: self.settings.volume,
            muted: self.settings.muted,
            at: now_ms(),
            ..Default::default()
        };
        self.publish_state();
        self.emit_position();
    }

    // ---- continuation ------------------------------------------------------------

    fn can_continue(&self) -> bool {
        match self.cont.kind {
            ContinuationKind::None => self.settings.autoplay && self.queue.current().is_some(),
            ContinuationKind::Ui => false,
            _ => true,
        }
    }

    fn exclude(&self) -> HashSet<String> {
        self.queue
            .ids()
            .map(str::to_owned)
            .chain(self.played.iter().cloned())
            .collect()
    }

    fn ensure_continuation(&mut self, at_track_start: bool) {
        let remaining = self.queue.remaining_after();
        if remaining >= TOP_UP_AT && !self.cont.advance_after {
            return;
        }
        let (req, origin) = match self.cont.kind {
            ContinuationKind::Ui => {
                if at_track_start {
                    self.emit(PlayerEvent::QueueLow { remaining });
                }
                return;
            }
            ContinuationKind::Wave => (MoreRequest::WaveNext { count: BATCH }, Origin::Wave),
            ContinuationKind::Radio => (
                MoreRequest::Radio {
                    seeds: self.cont.seeds.clone(),
                    exclude: self.exclude(),
                    count: BATCH,
                },
                Origin::Radio,
            ),
            ContinuationKind::None => {
                let Some(current) = self
                    .queue
                    .current()
                    .cloned()
                    .filter(|_| self.settings.autoplay)
                else {
                    return;
                };
                (
                    MoreRequest::Autoplay {
                        current,
                        exclude: self.exclude(),
                        count: BATCH,
                    },
                    Origin::Autoplay,
                )
            }
        };
        self.fetch_more(req, origin);
    }

    fn fetch_more(&mut self, req: MoreRequest, origin: Origin) {
        if self.cont.fetching {
            return;
        }
        self.cont.fetching = true;
        self.commit();
        let (hooks, tx, epoch) = (self.hooks.clone(), self.input_tx.clone(), self.cont.epoch);
        tokio::spawn(async move {
            let result = hooks.more(req).await;
            let _ = tx.send(Input::More {
                epoch,
                origin,
                result,
            });
        });
    }

    fn on_more(&mut self, epoch: u64, origin: Origin, result: Result<Vec<TrackInput>, String>) {
        if epoch != self.cont.epoch {
            return;
        }
        self.cont.fetching = false;
        let mut added = false;
        match result {
            Ok(tracks) => {
                let exclude: HashSet<String> = self.queue.ids().map(str::to_owned).collect();
                let mut seen = HashSet::new();
                let tracks: Vec<TrackInput> = tracks
                    .into_iter()
                    .filter(|t| is_video_id(&t.id))
                    .filter(|t| !exclude.contains(&t.id) && seen.insert(t.id.clone()))
                    .map(|mut t| {
                        t.origin = Some(origin);
                        t
                    })
                    .collect();
                added = !tracks.is_empty();
                self.queue.enqueue_end(tracks);
            }
            Err(e) => tracing::warn!(?origin, "continuation failed: {e}"),
        }
        self.commit();
        self.update_next_up();
        if std::mem::take(&mut self.cont.advance_after) {
            if added {
                self.next();
            } else {
                self.finish();
            }
        }
    }

    fn reset_continuation(&mut self, kind: ContinuationKind, seeds: Vec<PlayerTrack>) {
        let was_wave = self.cont.kind == ContinuationKind::Wave;
        self.cont = Cont {
            kind,
            fetching: false,
            seeds,
            epoch: self.cont.epoch + 1,
            advance_after: false,
        };
        if was_wave && kind != ContinuationKind::Wave {
            let hooks = self.hooks.clone();
            tokio::spawn(async move { hooks.wave_stop().await });
            self.emit(PlayerEvent::Wave { active: false });
        }
    }

    fn on_play_started(&mut self, t: &PlayerTrack) {
        self.streak = 0;
        self.streak_id = None;
        if self.state.error.take().is_some() {
            self.publish_state();
        }
        if self.cont.kind == ContinuationKind::Wave && t.origin != Origin::Wave {
            // The listener played something else: the wave steps aside.
            self.reset_continuation(ContinuationKind::None, Vec::new());
            self.commit();
        }
    }

    // ---- failures ------------------------------------------------------------------

    fn report_error(
        &mut self,
        t: &PlayerTrack,
        kind: ErrorKind,
        message: String,
        action: ErrorAction,
    ) {
        let err = PlayerError {
            uid: Some(t.uid.clone()),
            track_id: Some(t.id.clone()),
            kind,
            message,
            action,
            streak: self.streak,
        };
        tracing::warn!(uid = %t.uid, id = %t.id, ?kind, ?action, "{}", err.message);
        if self.state.uid.as_deref() == Some(t.uid.as_str()) {
            self.state.error = Some(err.clone());
            self.publish_state();
        }
        self.emit(PlayerEvent::Error(err));
    }

    fn on_deck_error(&mut self, deck: DeckId, message: String, decode: bool) {
        if self.standby.as_ref().is_some_and(|d| d.id == deck) {
            tracing::debug!(deck, "standby deck failed: {message}");
            self.standby = None;
            return;
        }
        if self.active.as_ref().is_none_or(|d| d.id != deck) {
            return;
        }
        let Some(t) = self.queue.current().cloned() else {
            return;
        };
        self.sample_position();
        let pos = self.state.position;
        if let Some(d) = self.active.take() {
            self.retire(d, false);
        }
        if pos > 5.0 && self.retried.insert(t.uid.clone()) {
            self.report_error(&t, ErrorKind::Other, message, ErrorAction::Retrying);
            let idx = self.queue.index().unwrap_or(0);
            self.load(idx, pos, true);
            return;
        }
        self.state.status = Status::Loading;
        self.publish_state();
        let fallback = FailureInfo {
            kind: if decode {
                ErrorKind::Decode
            } else {
                ErrorKind::Other
            },
            message,
        };
        let (hooks, tx) = (self.hooks.clone(), self.input_tx.clone());
        tokio::spawn(async move {
            let info = hooks.failure(&t.id).await;
            let _ = tx.send(Input::Failure {
                uid: t.uid,
                info,
                fallback,
            });
        });
    }

    fn on_failure(&mut self, uid: String, info: Option<FailureInfo>, fallback: FailureInfo) {
        let Some(t) = self.queue.get(&uid).cloned() else {
            return;
        };
        let FailureInfo { kind, message } = info.unwrap_or(fallback);
        if self.state.uid.as_deref() != Some(uid.as_str()) {
            self.report_error(&t, kind, message, ErrorAction::None);
            return;
        }
        if kind == ErrorKind::NoYtDlp {
            self.halt();
            self.report_error(&t, kind, message, ErrorAction::Stopped);
            return;
        }
        if self.streak_id.as_deref() != Some(t.id.as_str()) {
            self.streak += 1;
            self.streak_id = Some(t.id.clone());
        }
        if self.streak >= MAX_STREAK {
            self.halt();
            self.report_error(&t, kind, message, ErrorAction::Stopped);
            self.streak = 0;
            self.streak_id = None;
            return;
        }
        if kind == ErrorKind::Unavailable {
            self.queue.mark_unavailable(&uid);
            self.commit();
        }
        self.report_error(&t, kind, message, ErrorAction::Skipped);
        self.after(SKIP_DELAY, Timer::SkipAfterError(uid));
    }

    /// Stops after an error: paused on the failed entry, no deck.
    fn halt(&mut self) {
        self.drop_decks(false);
        self.want_play = false;
        self.state.status = Status::Paused;
        self.publish_state();
    }

    // ---- crossfade -------------------------------------------------------------------

    fn crossfade_check(&mut self, pos: f64) {
        let cf = self.settings.crossfade;
        if !cf.enabled || self.state.crossfading || self.queue.repeat == Repeat::One {
            if let Some(d) = self.standby.take() {
                self.retire(d, false);
            }
            return;
        }
        let Some(dur) = self.state.duration else {
            return;
        };
        let remaining = dur - pos;
        let next = self
            .queue
            .next_candidate(self.playable())
            .and_then(|i| self.queue.at(i).cloned());
        let Some(next) = next.filter(|n| n.uid != self.state.uid.clone().unwrap_or_default())
        else {
            return;
        };
        if self.standby.as_ref().is_some_and(|s| s.uid != next.uid)
            && let Some(d) = self.standby.take()
        {
            self.retire(d, false);
        }
        // An unknown next duration is checked once its deck has prerolled.
        let long_enough =
            dur >= 2.0 * cf.seconds && next.duration.is_none_or(|d| d >= 2.0 * cf.seconds);
        if !long_enough {
            return;
        }
        if self.standby.is_none() && remaining <= cf.seconds + CROSSFADE_LEAD_S && remaining > 1.0 {
            if let Ok(deck) = self.new_deck(&next) {
                deck.set_state(gst::State::Paused);
                self.standby = Some(deck);
            }
            return;
        }
        if self
            .standby
            .as_ref()
            .is_some_and(|s| s.prerolled && s.duration().is_some_and(|d| d < 2.0 * cf.seconds))
            && let Some(d) = self.standby.take()
        {
            self.retire(d, false);
            return;
        }
        if remaining <= cf.seconds && self.standby.as_ref().is_some_and(|s| s.prerolled) {
            self.start_crossfade(remaining.min(cf.seconds).max(0.5));
        }
    }

    fn start_crossfade(&mut self, seconds: f64) {
        let (Some(old), Some(new)) = (self.active.take(), self.standby.take()) else {
            return;
        };
        let Some(idx) = self.queue.position_of(&new.uid) else {
            self.active = Some(old);
            return;
        };
        tracing::debug!(from = old.id, to = new.id, seconds, "crossfade");
        old.set_next_up(None);
        old.set_tap(None);
        old.dsp.fade(None, 0.0, seconds, Curve::EqualPowerOut);
        new.dsp.fade(Some(0.0), 1.0, seconds, Curve::EqualPowerIn);
        new.set_state(gst::State::Playing);
        if let Some(uid) = self.state.uid.clone() {
            let evs = self.tracker.ended(&uid, now_ms());
            self.play_events(evs);
        }
        self.after(
            Duration::from_secs_f64(seconds + 0.3),
            Timer::CrossfadeDone(old.id),
        );
        self.retiring.push(old);
        self.queue.set_index(Some(idx));
        self.commit();
        let t = self.queue.at(idx).cloned().expect("index is valid");
        self.active = Some(new);
        self.set_current_state(&t, 0.0);
        self.state.crossfading = true;
        self.state.status = Status::Playing;
        self.after_load(&t);
    }

    // ---- inputs -------------------------------------------------------------------------

    fn on_tick(&mut self) {
        let Some(d) = &self.active else {
            return;
        };
        if !d.prerolled {
            return;
        }
        let pos = d.position();
        let dur = d.duration();
        let mut changed = false;
        if let Some(dur) = dur
            && self
                .state
                .duration
                .is_none_or(|old| (old - dur).abs() > 0.5)
        {
            self.state.duration = Some(dur);
            changed = true;
        }
        if let Some(p) = pos {
            self.state.position = p;
        }
        self.state.at = now_ms();
        if changed {
            self.publish_state();
        }
        if self.state.status == Status::Playing {
            if let Some(t) = self.queue.current().cloned() {
                let in_wave = self.in_wave(&t);
                let evs = self.tracker.tick(
                    &t,
                    in_wave,
                    self.state.position,
                    self.state.duration,
                    now_ms(),
                );
                self.play_events(evs);
            }
            self.crossfade_check(self.state.position);
            self.ticks += 1;
            if self.ticks.is_multiple_of(POSITION_SAVE_EVERY) {
                self.save_position();
            }
        }
        self.emit_position();
    }

    fn deck_msg(&mut self, msg: DeckMsg) {
        let (id, msg) = match msg {
            DeckMsg::AboutToFinish(id) => {
                tracing::debug!(deck = id, "about-to-finish: next uri set");
                return;
            }
            DeckMsg::Bus(id, msg) => (id, msg),
        };
        use gst::MessageView as V;
        let is_active = self.active.as_ref().is_some_and(|d| d.id == id);
        let is_standby = self.standby.as_ref().is_some_and(|d| d.id == id);
        match msg.view() {
            V::Error(e) => {
                let decode = matches!(
                    e.error().kind::<gst::StreamError>(),
                    Some(
                        gst::StreamError::Decode
                            | gst::StreamError::Demux
                            | gst::StreamError::Format
                            | gst::StreamError::TypeNotFound
                            | gst::StreamError::WrongType
                            | gst::StreamError::CodecNotFound
                    )
                );
                let message = format!("{} ({})", e.error(), e.debug().unwrap_or_default());
                self.on_deck_error(id, message, decode);
            }
            V::AsyncDone(_) if is_standby => {
                if let Some(d) = &mut self.standby {
                    d.prerolled = true;
                }
            }
            V::AsyncDone(_) if is_active => self.on_prerolled(),
            V::StateChanged(s) if is_active => {
                if s.current() == gst::State::Playing && self.want_play {
                    if self.state.status != Status::Playing {
                        self.state.status = Status::Playing;
                        self.sample_position();
                        self.publish_state();
                        self.emit_position();
                    }
                } else if s.current() == gst::State::Paused
                    && !self.want_play
                    && self.state.status == Status::Playing
                {
                    self.state.status = Status::Paused;
                    self.publish_state();
                }
            }
            V::StreamStart(_) if is_active => self.on_stream_start(),
            V::Eos(_) if is_active => {
                tracing::debug!(deck = id, "end of stream");
                self.natural_end();
            }
            V::Eos(_) => self.retiring.retain(|d| d.id != id),
            V::Buffering(b) if is_active => self.on_buffering(b.percent()),
            V::DurationChanged(_) if is_active => {
                if let Some(d) = self.active.as_ref().and_then(Deck::duration) {
                    self.state.duration = Some(d);
                    self.publish_state();
                }
            }
            _ => {}
        }
    }

    fn on_prerolled(&mut self) {
        let rate = self.settings.speed;
        let Some(d) = &mut self.active else {
            return;
        };
        if d.prerolled {
            return;
        }
        d.prerolled = true;
        let seek = d.pending_seek.take();
        if seek.is_some() || (rate - 1.0).abs() > 1e-9 {
            d.seek(seek.unwrap_or(0.0), rate);
        }
        if let Some(dur) = d.duration() {
            self.state.duration = Some(dur);
        }
        if d.play_when_ready && self.want_play {
            d.dsp.fade(Some(0.0), 1.0, DECLICK_S, Curve::Linear);
            d.set_state(gst::State::Playing);
        } else if self.state.status == Status::Loading {
            self.state.status = Status::Paused;
        }
        self.publish_state();
    }

    fn on_stream_start(&mut self) {
        let Some(d) = &mut self.active else {
            return;
        };
        if !d.started {
            d.started = true;
            return;
        }
        let Some(uid) = d.take_switched() else {
            return;
        };
        // Gapless: the next entry began on the same deck.
        d.uid = uid.clone();
        let rate = self.settings.speed;
        if (rate - 1.0).abs() > 1e-9 {
            d.seek(0.0, rate);
        }
        if let Some(old) = self.state.uid.clone() {
            let evs = self.tracker.ended(&old, now_ms());
            self.play_events(evs);
        }
        let Some(idx) = self.queue.position_of(&uid) else {
            return;
        };
        tracing::debug!(%uid, "gapless switch");
        self.set_index(Some(idx));
        let t = self.queue.at(idx).cloned().expect("valid index");
        self.set_current_state(&t, 0.0);
        self.after_load(&t);
    }

    fn on_buffering(&mut self, percent: i32) {
        let buffering = percent < 100;
        if buffering == self.state.buffering {
            return;
        }
        self.state.buffering = buffering;
        if let Some(d) = &self.active
            && d.prerolled
            && self.want_play
        {
            d.set_state(if buffering {
                gst::State::Paused
            } else {
                gst::State::Playing
            });
        }
        self.publish_state();
    }

    fn input(&mut self, input: Input) {
        match input {
            Input::Timer(t) => self.timer(t),
            Input::More {
                epoch,
                origin,
                result,
            } => self.on_more(epoch, origin, result),
            Input::WaveStarted { result, reply } => {
                let r = self.on_wave_started(result);
                let _ = reply.send(r);
            }
            Input::Failure {
                uid,
                info,
                fallback,
            } => self.on_failure(uid, info, fallback),
            Input::StreamInfo { uid, info } => {
                if self.state.uid.as_deref() == Some(uid.as_str()) && info.is_some() {
                    self.state.stream = info;
                    self.publish_state();
                }
            }
            Input::RequeueWave => self.requeue_wave(),
            Input::VisualizerGone(epoch) => {
                if self.vis.as_ref().is_some_and(|v| v.epoch == epoch) {
                    self.vis = None;
                    self.sync_tap();
                }
            }
        }
    }

    fn timer(&mut self, t: Timer) {
        match t {
            Timer::Preroll(id) => {
                if let Some(d) = &self.active
                    && d.id == id
                    && !d.prerolled
                    && let Some(t) = self.queue.current().cloned()
                {
                    if let Some(d) = self.active.take() {
                        self.retire(d, false);
                    }
                    self.state.status = Status::Loading;
                    let fallback = FailureInfo {
                        kind: ErrorKind::Timeout,
                        message: format!(
                            "the stream did not start in {} s",
                            PREROLL_TIMEOUT.as_secs()
                        ),
                    };
                    self.on_failure(t.uid, None, fallback);
                }
            }
            Timer::PauseDeck(id, epoch) => {
                if epoch == self.pause_gen
                    && !self.want_play
                    && let Some(d) = &self.active
                    && d.id == id
                {
                    d.set_state(gst::State::Paused);
                }
            }
            Timer::Retire(id) | Timer::CrossfadeDone(id) => {
                let was_crossfade = matches!(t, Timer::CrossfadeDone(_));
                self.retiring.retain(|d| d.id != id);
                if was_crossfade && self.state.crossfading {
                    self.state.crossfading = false;
                    self.publish_state();
                }
            }
            Timer::SkipAfterError(uid) => {
                if self.state.uid.as_deref() == Some(uid.as_str()) && self.active.is_none() {
                    self.next();
                }
            }
            Timer::Persist(epoch) => {
                if epoch == self.persist_gen {
                    self.save_queue();
                }
            }
            Timer::Sleep(epoch) => {
                if self.sleep.is_some_and(|(_, g)| g == epoch) {
                    if let Some(d) = &self.active
                        && self.want_play
                    {
                        d.dsp.fade(None, 0.0, SLEEP_FADE_S, Curve::Linear);
                        self.after(
                            Duration::from_secs_f64(SLEEP_FADE_S + 0.2),
                            Timer::SleepFadeDone(epoch),
                        );
                    } else {
                        self.timer(Timer::SleepFadeDone(epoch));
                    }
                }
            }
            Timer::SleepFadeDone(epoch) => {
                if self.sleep.is_some_and(|(_, g)| g == epoch) {
                    self.sleep = None;
                    self.pause();
                    self.emit(PlayerEvent::Sleep { ends_at: None });
                }
            }
        }
    }

    fn requeue_wave(&mut self) {
        if self.cont.kind != ContinuationKind::Wave {
            return;
        }
        let dropped = self.queue.remove_upcoming(|t| t.origin == Origin::Wave);
        if !dropped.is_empty() {
            self.commit();
        }
        tracing::debug!(?dropped, "wave re-seeded: requeue");
        // A fetch in flight would append stale tracks: replace it.
        self.cont.epoch += 1;
        self.cont.fetching = false;
        self.fetch_more(MoreRequest::WaveRequeue { dropped, count: 3 }, Origin::Wave);
    }

    fn on_wave_started(&mut self, result: Result<Vec<TrackInput>, String>) -> Result<u64, Error> {
        let tracks = result.map_err(hook_error)?;
        let tracks: Vec<TrackInput> = tracks
            .into_iter()
            .filter(|t| is_video_id(&t.id))
            .map(|mut t| {
                t.origin = Some(Origin::Wave);
                t
            })
            .collect();
        if tracks.is_empty() {
            return Err(Error::new("no_candidates", "the wave has nothing to play"));
        }
        self.reset_continuation(ContinuationKind::Wave, Vec::new());
        self.queue.set(tracks, 0);
        let rev = self.commit();
        self.load(0, 0.0, true);
        self.emit(PlayerEvent::Wave { active: true });
        Ok(self.queue.rev().max(rev))
    }

    // ---- persistence -------------------------------------------------------------------------

    fn save_queue(&mut self) {
        self.queue_dirty = false;
        let saved = SavedQueue {
            version: 1,
            queue: self.queue.state(),
            continuation: SavedContinuation {
                kind: self.cont.kind,
                seeds: self.cont.seeds.clone(),
            },
            played: self.played.iter().cloned().collect(),
        };
        persist::save_queue(&self.paths, &saved);
    }

    fn save_position(&mut self) {
        let p = SavedPosition {
            uid: self.state.uid.clone(),
            position: self.state.position,
        };
        persist::save_position(&self.paths, &p);
    }

    fn save_settings(&self) {
        persist::save_settings(
            &self.paths,
            &SavedSettings {
                version: 1,
                settings: self.settings.clone(),
                dsp: self.dsp_config.clone(),
            },
        );
    }

    async fn shutdown(&mut self) {
        self.sample_position();
        let evs = self.tracker.leave(now_ms());
        self.play_events(evs);
        let (tx, rx) = oneshot::channel();
        let _ = self.worker_tx.send(WorkerMsg::Flush(tx));
        let _ = tokio::time::timeout(Duration::from_secs(3), rx).await;
        self.save_queue();
        self.save_position();
        self.save_settings();
        if let Some(v) = self.vis.take() {
            v.task.abort();
        }
        self.standby = None;
        self.active = None;
        self.retiring.clear();
        tracing::info!("player shut down");
    }

    // ---- commands -----------------------------------------------------------------------------

    fn snapshot(&mut self) -> PlayerSnapshot {
        self.sample_position();
        PlayerSnapshot {
            protocol: 1,
            state: self.state.clone(),
            queue: self.queue.snapshot(self.continuation()),
            settings: self.settings.clone(),
            dsp: self.dsp_config.clone(),
            sleep_timer: self.sleep.map(|(ends_at, _)| SleepTimer { ends_at }),
            wave: WaveInfo {
                active: self.cont.kind == ContinuationKind::Wave,
            },
            journal_seq: self.hooks.journal_seq(),
        }
    }

    fn command(&mut self, cmd: Command) {
        match cmd {
            Command::Snapshot(r) => {
                let _ = r.send(Ok(self.snapshot()));
            }
            Command::Play(r) => {
                self.play();
                let _ = r.send(Ok(()));
            }
            Command::Pause(r) => {
                self.pause();
                let _ = r.send(Ok(()));
            }
            Command::Toggle(r) => {
                if self.want_play {
                    self.pause();
                } else {
                    self.play();
                }
                let _ = r.send(Ok(()));
            }
            Command::Stop(r) => {
                self.stop();
                let _ = r.send(Ok(()));
            }
            Command::Next(r) => {
                if !self.queue.is_empty() {
                    self.next();
                }
                let _ = r.send(Ok(()));
            }
            Command::Prev(r) => {
                if !self.queue.is_empty() {
                    self.prev();
                }
                let _ = r.send(Ok(()));
            }
            Command::Seek {
                position,
                resume,
                reply,
            } => {
                if position.is_finite() {
                    self.seek(position);
                    if resume {
                        self.play();
                    }
                    let _ = reply.send(Ok(()));
                } else {
                    let _ = reply.send(Err(bad("position must be a number")));
                }
            }
            Command::PlayUid {
                uid,
                position,
                reply,
            } => {
                let r = match self.queue.position_of(&uid) {
                    Some(i) => {
                        self.load(
                            i,
                            position.filter(|p| p.is_finite()).unwrap_or(0.0).max(0.0),
                            true,
                        );
                        Ok(())
                    }
                    None => Err(Error::new("unknown_uid", uid)),
                };
                let _ = reply.send(r);
            }
            Command::SetQueue {
                tracks,
                start,
                play,
                position,
                reply,
            } => {
                let _ = reply.send(self.set_queue(tracks, start, play, position));
            }
            Command::Enqueue { tracks, at, reply } => {
                let _ = reply.send(self.enqueue(tracks, at));
            }
            Command::Remove { uid, reply } => {
                let r = match self.queue.remove(&uid) {
                    None => Err(Error::new("unknown_uid", uid)),
                    Some(removed) => {
                        let rev = self.commit();
                        match removed {
                            Removed::Other => self.update_next_up(),
                            Removed::CurrentReplaced => {
                                let idx = self.queue.index().expect("replaced");
                                if self.want_play {
                                    self.load(idx, 0.0, true);
                                } else {
                                    self.cue(idx, 0.0);
                                }
                            }
                            Removed::CurrentGone => self.go_idle(),
                        }
                        Ok(rev.max(self.queue.rev()))
                    }
                };
                let _ = reply.send(r);
            }
            Command::Move { uid, to, reply } => {
                let r = match self.queue.move_to(&uid, to) {
                    Some(()) => {
                        let rev = self.commit();
                        self.update_next_up();
                        Ok(rev)
                    }
                    None if self.queue.position_of(&uid).is_none() => {
                        Err(Error::new("unknown_uid", uid))
                    }
                    None => Err(bad(format!("index {to} is out of range"))),
                };
                let _ = reply.send(r);
            }
            Command::Clear {
                keep_current,
                reply,
            } => {
                if !keep_current {
                    self.go_idle();
                }
                self.queue.clear(keep_current);
                let rev = self.commit();
                self.update_next_up();
                let _ = reply.send(Ok(rev));
            }
            Command::SetShuffle { on, reply } => {
                self.queue.set_shuffle(on, &mut self.rng);
                let rev = self.commit();
                self.update_next_up();
                let _ = reply.send(Ok(rev));
            }
            Command::SetRepeat { mode, reply } => {
                self.queue.repeat = mode;
                let rev = self.commit();
                self.update_next_up();
                let _ = reply.send(Ok(rev));
            }
            Command::SetSettings {
                patch,
                external,
                reply,
            } => {
                let r = self.settings.merged(&patch).map_err(bad).map(|s| {
                    self.apply_settings(s, external);
                    self.settings.clone()
                });
                let _ = reply.send(r);
            }
            Command::SetDsp { config, reply } => {
                self.dsp_config = config.sanitized();
                self.dsp.set_config(&self.dsp_config);
                self.save_settings();
                self.emit(PlayerEvent::Settings {
                    settings: self.settings.clone(),
                    dsp: self.dsp_config.clone(),
                    external: false,
                });
                let _ = reply.send(Ok(()));
            }
            Command::SetSleep { minutes, reply } => {
                self.sleep_gen += 1;
                let r = match minutes {
                    Some(m) if m.is_finite() && m > 0.0 => {
                        let ends_at = (now_ms() + m * 60_000.0).round();
                        self.sleep = Some((ends_at, self.sleep_gen));
                        self.after(
                            Duration::from_secs_f64(m * 60.0),
                            Timer::Sleep(self.sleep_gen),
                        );
                        Some(ends_at)
                    }
                    _ => {
                        self.sleep = None;
                        None
                    }
                };
                self.emit(PlayerEvent::Sleep { ends_at: r });
                let _ = reply.send(Ok(r));
            }
            Command::SetContinuation { kind, seeds, reply } => {
                let _ = reply.send(self.set_continuation(kind, seeds));
            }
            Command::WaveStart { count, reply } => {
                let (hooks, tx) = (self.hooks.clone(), self.input_tx.clone());
                let count = count.clamp(1, 50);
                tokio::spawn(async move {
                    let result = hooks.more(MoreRequest::WaveStart { count }).await;
                    let _ = tx.send(Input::WaveStarted { result, reply });
                });
            }
            Command::WaveStop(r) => {
                if self.cont.kind == ContinuationKind::Wave {
                    self.reset_continuation(ContinuationKind::None, Vec::new());
                    self.commit();
                }
                let _ = r.send(Ok(()));
            }
            Command::WaveRequeue(r) => {
                self.requeue_wave();
                let _ = r.send(Ok(self.queue.rev()));
            }
            Command::WaveDislike(r) => {
                if let Some(t) = self.queue.current().cloned() {
                    let now = now_ms();
                    let mut evs = self.tracker.leave(now);
                    evs.push(PlayEvent::Dislike { track: t, at: now });
                    self.play_events(evs);
                    self.ensure_continuation(false);
                    self.next();
                }
                let _ = r.send(Ok(()));
            }
            Command::VisualizerStart { fps, sink, reply } => {
                self.start_visualizer(fps, sink);
                let _ = reply.send(Ok(()));
            }
            Command::VisualizerStop(r) => {
                if let Some(v) = self.vis.take() {
                    v.task.abort();
                }
                self.sync_tap();
                let _ = r.send(Ok(()));
            }
            Command::Shutdown(_) => unreachable!("handled in run"),
        }
    }

    fn set_queue(
        &mut self,
        tracks: Vec<TrackInput>,
        start: usize,
        play: bool,
        position: Option<f64>,
    ) -> Result<u64, Error> {
        validate(&tracks)?;
        self.reset_continuation(ContinuationKind::None, Vec::new());
        let empty = tracks.is_empty();
        self.queue.set(tracks, start);
        let rev = self.commit();
        let pos = position.filter(|p| p.is_finite()).unwrap_or(0.0).max(0.0);
        if empty {
            self.go_idle();
        } else {
            let idx = self.queue.index().unwrap_or(0);
            if play {
                self.load(idx, pos, true);
            } else {
                self.cue(idx, pos);
            }
        }
        Ok(rev.max(self.queue.rev()))
    }

    fn enqueue(&mut self, tracks: Vec<TrackInput>, at: EnqueueAt) -> Result<u64, Error> {
        validate(&tracks)?;
        if tracks.is_empty() {
            return Ok(self.queue.rev());
        }
        let nothing_loaded = self.queue.index().is_none();
        let uids = match at {
            EnqueueAt::Next => self.queue.enqueue_next(tracks),
            EnqueueAt::End => self.queue.enqueue_end(tracks),
        };
        let rev = self.commit();
        if nothing_loaded && let Some(i) = uids.first().and_then(|u| self.queue.position_of(u)) {
            self.load(i, 0.0, true);
        } else {
            self.update_next_up();
        }
        Ok(rev.max(self.queue.rev()))
    }

    fn set_continuation(
        &mut self,
        kind: ContinuationKind,
        seeds: Vec<TrackInput>,
    ) -> Result<(), Error> {
        let seeds = match kind {
            ContinuationKind::Wave => return Err(bad("use player_wave_start for the wave")),
            ContinuationKind::Radio => {
                validate(&seeds)?;
                if seeds.is_empty() {
                    return Err(bad("radio needs seeds"));
                }
                seeds
                    .into_iter()
                    .take(50)
                    .enumerate()
                    .map(|(i, t)| {
                        PlayerTrack::from_input(format!("seed{i}"), TrackInput { ui: None, ..t })
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        self.reset_continuation(kind, seeds);
        self.commit();
        self.ensure_continuation(false);
        Ok(())
    }

    fn apply_settings(&mut self, new: PlayerSettings, external: bool) {
        let old = std::mem::replace(&mut self.settings, new);
        let s = &self.settings;
        self.dsp.set_master(s.master_gain());
        self.state.volume = s.volume;
        self.state.muted = s.muted;
        self.state.rate = s.speed;
        let rate_changed = (old.speed - s.speed).abs() > 1e-9;
        let pitch_changed = old.preserve_pitch != s.preserve_pitch;
        if pitch_changed && self.active.is_some() {
            // The filter bin is built per deck: reload where we are.
            self.sample_position();
            if let Some(i) = self.queue.index() {
                let (pos, play) = (self.state.position, self.want_play);
                self.load(i, pos, play);
            }
        } else if rate_changed
            && let Some(d) = &self.active
            && d.prerolled
        {
            let pos = d.position().unwrap_or(self.state.position);
            d.seek(pos, self.settings.speed);
        }
        if old.autoplay != self.settings.autoplay {
            self.ensure_continuation(false);
        }
        self.update_next_up();
        self.publish_state();
        self.publish_now();
        self.save_settings();
        self.emit(PlayerEvent::Settings {
            settings: self.settings.clone(),
            dsp: self.dsp_config.clone(),
            external,
        });
    }

    fn start_visualizer(&mut self, fps: u32, mut sink: FrameSink) {
        if let Some(v) = self.vis.take() {
            v.task.abort();
        }
        self.vis_gen += 1;
        let epoch = self.vis_gen;
        let fps = fps.clamp(10, 60);
        let (tap, mut now, tx) = (
            self.tap.clone(),
            self.now_tx.subscribe(),
            self.input_tx.clone(),
        );
        let task = tokio::spawn(async move {
            let mut analyzer = Analyzer::new();
            let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / f64::from(fps)));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut idle_ticks = 0u32;
            loop {
                tick.tick().await;
                let (playing, position) = {
                    let n = now.borrow_and_update();
                    (n.state.status == Status::Playing, n.position_now())
                };
                if !playing {
                    // 2 Hz while paused, zeroed, so the presets decay.
                    idle_ticks += 1;
                    if idle_ticks < fps / 2 {
                        continue;
                    }
                }
                idle_ticks = 0;
                let frame = analyzer.frame(Some(&tap), playing, position);
                if !sink(frame) {
                    let _ = tx.send(Input::VisualizerGone(epoch));
                    break;
                }
            }
        });
        self.vis = Some(Vis { epoch, task });
        self.sync_tap();
    }
}

/// Runs `Hooks::on_play_event` in order, one at a time.
async fn worker<H: Hooks>(
    hooks: Arc<H>,
    mut rx: mpsc::UnboundedReceiver<WorkerMsg>,
    input: mpsc::UnboundedSender<Input>,
) {
    while let Some(msg) = rx.recv().await {
        match msg {
            WorkerMsg::Event(ev) => {
                let in_wave_end = matches!(*ev, PlayEvent::End { in_wave: true, .. });
                let out = hooks.on_play_event(*ev).await;
                if out.requeue_wave && in_wave_end {
                    let _ = input.send(Input::RequeueWave);
                }
            }
            WorkerMsg::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}
