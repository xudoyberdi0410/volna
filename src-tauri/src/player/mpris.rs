//! MPRIS (`org.mpris.MediaPlayer2.volna`) over the player, so media keys,
//! KDE/GNOME widgets and `playerctl -p volna` work with the window closed.

use std::sync::Arc;

use mpris_server::zbus::fdo;
use mpris_server::{
    LoopStatus, Metadata, PlaybackRate, PlaybackStatus, PlayerInterface, Property, RootInterface,
    Server, Signal, Time, TrackId, Volume,
};
use tauri::AppHandle;
use tokio::sync::broadcast::error::RecvError;
use volna_player::{NowPlaying, PlayerEvent, PlayerHandle, PlayerTrack, Repeat, Status};

use super::convert::cover_source;
use super::window;
use crate::state::Images;

struct Mpris {
    app: AppHandle,
    player: PlayerHandle,
    /// `(uid, file:// URL)` of the cover once it is on disk.
    art: std::sync::Mutex<Option<(String, String)>>,
}

impl Mpris {
    fn art_for(&self, uid: Option<&str>) -> Option<String> {
        let art = self.art.lock().ok()?;
        art.as_ref()
            .filter(|(u, _)| Some(u.as_str()) == uid)
            .map(|(_, a)| a.clone())
    }
}

fn internal(e: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::Failed(e.to_string())
}

fn track_id(uid: &str) -> TrackId {
    // uids are "q<base36>": valid object path elements.
    TrackId::try_from(format!("/uz/khudoberdi/volna/track/{uid}")).unwrap_or(TrackId::NO_TRACK)
}

fn status(n: &NowPlaying) -> PlaybackStatus {
    match n.state.status {
        Status::Playing | Status::Loading => PlaybackStatus::Playing,
        Status::Paused => PlaybackStatus::Paused,
        Status::Idle | Status::Ended => PlaybackStatus::Stopped,
    }
}

fn loop_status(r: Repeat) -> LoopStatus {
    match r {
        Repeat::Off => LoopStatus::None,
        Repeat::One => LoopStatus::Track,
        Repeat::All => LoopStatus::Playlist,
    }
}

fn metadata(t: Option<&PlayerTrack>, duration: Option<f64>, art: Option<&str>) -> Metadata {
    let Some(t) = t else {
        return Metadata::builder().trackid(TrackId::NO_TRACK).build();
    };
    let mut m = Metadata::builder()
        .trackid(track_id(&t.uid))
        .title(t.title.clone())
        .artist(
            t.artists
                .iter()
                .map(|a| a.name.clone())
                .filter(|n| !n.is_empty()),
        )
        .url(format!("https://music.youtube.com/watch?v={}", t.id));
    if let Some(d) = duration.or(t.duration) {
        m = m.length(Time::from_micros((d * 1e6) as i64));
    }
    if let Some(a) = t.album.as_ref().filter(|a| !a.title.is_empty()) {
        m = m.album(a.title.clone());
    }
    if let Some(art) = art {
        m = m.art_url(art.to_owned());
    }
    m.build()
}

impl RootInterface for Mpris {
    async fn raise(&self) -> fdo::Result<()> {
        window::show_or_create(&self.app);
        Ok(())
    }
    async fn quit(&self) -> fdo::Result<()> {
        window::quit(&self.app);
        Ok(())
    }
    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn set_fullscreen(&self, _: bool) -> mpris_server::zbus::Result<()> {
        Err(mpris_server::zbus::Error::from(fdo::Error::NotSupported(
            "no fullscreen".into(),
        )))
    }
    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn identity(&self) -> fdo::Result<String> {
        Ok("Volna".into())
    }
    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok("volna".into())
    }
    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![])
    }
    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![])
    }
}

impl PlayerInterface for Mpris {
    async fn next(&self) -> fdo::Result<()> {
        self.player.next().await.map_err(internal)
    }
    async fn previous(&self) -> fdo::Result<()> {
        self.player.prev().await.map_err(internal)
    }
    async fn pause(&self) -> fdo::Result<()> {
        self.player.pause().await.map_err(internal)
    }
    async fn play_pause(&self) -> fdo::Result<()> {
        self.player.toggle().await.map_err(internal)
    }
    async fn stop(&self) -> fdo::Result<()> {
        self.player.stop().await.map_err(internal)
    }
    async fn play(&self) -> fdo::Result<()> {
        self.player.play().await.map_err(internal)
    }
    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        let n = self.player.now();
        if n.track.is_none() {
            return Ok(());
        }
        let pos = n.position_now() + offset.as_micros() as f64 / 1e6;
        if n.state.duration.is_some_and(|d| pos >= d) {
            return self.player.next().await.map_err(internal);
        }
        self.player
            .seek(pos.max(0.0), false)
            .await
            .map_err(internal)
    }
    async fn set_position(&self, id: TrackId, position: Time) -> fdo::Result<()> {
        let n = self.player.now();
        let Some(t) = &n.track else {
            return Ok(());
        };
        let pos = position.as_micros() as f64 / 1e6;
        if id != track_id(&t.uid) || pos < 0.0 || n.state.duration.is_some_and(|d| pos > d) {
            return Ok(());
        }
        self.player.seek(pos, false).await.map_err(internal)
    }
    async fn open_uri(&self, _: String) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "Volna plays YouTube Music only".into(),
        ))
    }
    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(status(&self.player.now()))
    }
    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(loop_status(self.player.now().repeat))
    }
    async fn set_loop_status(&self, l: LoopStatus) -> mpris_server::zbus::Result<()> {
        let mode = match l {
            LoopStatus::None => Repeat::Off,
            LoopStatus::Track => Repeat::One,
            LoopStatus::Playlist => Repeat::All,
        };
        self.player
            .set_repeat(mode)
            .await
            .map(|_| ())
            .map_err(|e| internal(e).into())
    }
    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(self.player.now().state.rate)
    }
    async fn set_rate(&self, rate: PlaybackRate) -> mpris_server::zbus::Result<()> {
        // A rate of 0 means pause in MPRIS.
        if rate <= 0.0 {
            return self.player.pause().await.map_err(|e| internal(e).into());
        }
        let patch = serde_json::json!({ "speed": rate.clamp(0.25, 4.0) });
        self.player
            .set_settings(patch.as_object().cloned().unwrap_or_default(), true)
            .await
            .map(|_| ())
            .map_err(|e| internal(e).into())
    }
    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(self.player.now().shuffle)
    }
    async fn set_shuffle(&self, on: bool) -> mpris_server::zbus::Result<()> {
        self.player
            .set_shuffle(on)
            .await
            .map(|_| ())
            .map_err(|e| internal(e).into())
    }
    async fn metadata(&self) -> fdo::Result<Metadata> {
        let n = self.player.now();
        let art = self.art_for(n.track.as_ref().map(|t| t.uid.as_str()));
        Ok(metadata(n.track.as_ref(), n.state.duration, art.as_deref()))
    }
    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(self.player.now().state.volume)
    }
    async fn set_volume(&self, v: Volume) -> mpris_server::zbus::Result<()> {
        let patch = serde_json::json!({ "volume": v.clamp(0.0, 1.0) });
        self.player
            .set_settings(patch.as_object().cloned().unwrap_or_default(), true)
            .await
            .map(|_| ())
            .map_err(|e| internal(e).into())
    }
    async fn position(&self) -> fdo::Result<Time> {
        Ok(Time::from_micros(
            (self.player.now().position_now() * 1e6) as i64,
        ))
    }
    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(0.25)
    }
    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(4.0)
    }
    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(self.player.now().has_next)
    }
    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(self.player.now().has_prev)
    }
    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(self.player.now().track.is_some())
    }
    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(self.player.now().track.is_some())
    }
    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(self.player.now().track.is_some())
    }
    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

/// What was last announced, to send only what changed.
#[derive(Default, PartialEq)]
struct Shown {
    status: Option<PlaybackStatus>,
    uid: Option<String>,
    duration: Option<i64>,
    art: Option<String>,
    repeat: Option<Repeat>,
    shuffle: Option<bool>,
    volume: Option<u64>,
    rate: Option<u64>,
    next: Option<bool>,
    prev: Option<bool>,
}

pub fn start(app: &AppHandle, player: PlayerHandle, images: Arc<Images>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let imp = || Mpris {
            app: app.clone(),
            player: player.clone(),
            art: std::sync::Mutex::new(None),
        };
        // Another copy (a dev build) may hold the plain name.
        let server = match Server::new("volna", imp()).await {
            Ok(s) => s,
            Err(_) => {
                match Server::new(&format!("volna.instance{}", std::process::id()), imp()).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("no MPRIS: {e}");
                        return;
                    }
                }
            }
        };
        let (art_tx, mut art_rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
        let mut events = player.subscribe();
        let mut now = player.watch_now();
        let mut shown = Shown::default();
        loop {
            tokio::select! {
                changed = now.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let n = now.borrow_and_update().clone();
                    announce(&server, &n, &mut shown, &images, &art_tx).await;
                }
                Some((uid, art)) = art_rx.recv() => {
                    let n = player.now();
                    if n.track.as_ref().is_some_and(|t| t.uid == uid) {
                        shown.art = Some(art.clone());
                        if let Ok(mut a) = server.imp().art.lock() {
                            *a = Some((uid, art.clone()));
                        }
                        let m = metadata(n.track.as_ref(), n.state.duration, Some(&art));
                        let _ = server.properties_changed([Property::Metadata(m)]).await;
                    }
                }
                ev = events.recv() => match ev {
                    Ok(PlayerEvent::Seeked { position, .. }) => {
                        let _ = server
                            .emit(Signal::Seeked { position: Time::from_micros((position * 1e6) as i64) })
                            .await;
                    }
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                },
            }
        }
    });
}

async fn announce(
    server: &Server<Mpris>,
    n: &NowPlaying,
    shown: &mut Shown,
    images: &Arc<Images>,
    art_tx: &tokio::sync::mpsc::UnboundedSender<(String, String)>,
) {
    let mut props = Vec::new();
    let st = status(n);
    if shown.status != Some(st) {
        shown.status = Some(st);
        props.push(Property::PlaybackStatus(st));
    }
    let uid = n.track.as_ref().map(|t| t.uid.clone());
    let duration = n.state.duration.map(|d| d.round() as i64);
    if shown.uid != uid || shown.duration != duration {
        let new_track = shown.uid != uid;
        shown.uid = uid.clone();
        shown.duration = duration;
        if new_track {
            shown.art = None;
        }
        props.push(Property::Metadata(metadata(
            n.track.as_ref(),
            n.state.duration,
            shown.art.as_deref(),
        )));
        let can = n.track.is_some();
        props.extend([
            Property::CanPlay(can),
            Property::CanPause(can),
            Property::CanSeek(can),
        ]);
        if new_track
            && let Some((uid, src)) = n
                .track
                .as_ref()
                .and_then(|t| Some((t.uid.clone(), cover_source(t.cover.as_deref()?)?)))
        {
            let (images, tx) = (images.clone(), art_tx.clone());
            tokio::spawn(async move {
                match images.get_path(&src, Some(544)).await {
                    Ok(path) => {
                        let _ = tx.send((uid, format!("file://{}", path.display())));
                    }
                    Err(e) => tracing::debug!("no cover for MPRIS: {e}"),
                }
            });
        }
    }
    if shown.repeat != Some(n.repeat) {
        shown.repeat = Some(n.repeat);
        props.push(Property::LoopStatus(loop_status(n.repeat)));
    }
    if shown.shuffle != Some(n.shuffle) {
        shown.shuffle = Some(n.shuffle);
        props.push(Property::Shuffle(n.shuffle));
    }
    let vol = n.state.volume.to_bits();
    if shown.volume != Some(vol) {
        shown.volume = Some(vol);
        props.push(Property::Volume(n.state.volume));
    }
    let rate = n.state.rate.to_bits();
    if shown.rate != Some(rate) {
        shown.rate = Some(rate);
        props.push(Property::Rate(n.state.rate));
    }
    if shown.next != Some(n.has_next) {
        shown.next = Some(n.has_next);
        props.push(Property::CanGoNext(n.has_next));
    }
    if shown.prev != Some(n.has_prev) {
        shown.prev = Some(n.has_prev);
        props.push(Property::CanGoPrevious(n.has_prev));
    }
    if !props.is_empty()
        && let Err(e) = server.properties_changed(props).await
    {
        tracing::debug!("MPRIS update failed: {e}");
    }
}
