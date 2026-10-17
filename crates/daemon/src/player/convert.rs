//! Between the player's tracks and `volna_ytm::Track` (the wave and radio).

use volna_player::{AlbumRef, ArtistRef, Origin, PlayerTrack, TrackInput, TrackKind};
use volna_ytm::{Track, TrackKind as YtmKind};

use crate::api_emu::shape::{self, TrackCtx};

/// A YouTube Music track as a queue entry: `raw` is the hifi-api shape the UI
/// runs through `prepareTrack`, `cover` the `volna://img` URL.
pub fn track_input(t: &Track, origin: Origin) -> TrackInput {
    TrackInput {
        id: t.video_id.clone(),
        title: t.title.clone(),
        artists: t
            .artists
            .iter()
            .map(|a| ArtistRef {
                id: a
                    .id
                    .as_deref()
                    .map(|id| shape::clean_artist_id(id).to_owned()),
                name: a.name.clone(),
            })
            .collect(),
        album: t.album.as_ref().map(|a| AlbumRef {
            id: a.id.clone(),
            title: a.name.clone(),
        }),
        duration: t.duration_s.map(f64::from),
        cover: shape::cover(&t.thumbnails).as_str().map(str::to_owned),
        explicit: t.explicit,
        kind: TrackKind::Track,
        origin: Some(origin),
        ui: None,
        raw: Some(shape::track(t, TrackCtx::default())),
    }
}

/// A queue entry as the scrobblers see it.
pub fn scrobble_track(t: &PlayerTrack) -> volna_scrobble::Track {
    volna_scrobble::Track {
        title: t.title.clone(),
        artists: t
            .artists
            .iter()
            .map(|a| a.name.clone())
            .filter(|n| !n.is_empty() && n != "Unknown artist")
            .collect(),
        album: t
            .album
            .as_ref()
            .map(|a| a.title.clone())
            .filter(|a| !a.is_empty()),
        duration: t.duration,
        track_number: None,
        video_id: Some(t.id.clone()).filter(|v| !v.is_empty()),
    }
}

/// A queue entry as the wave sees it (the same rules as `wave::UiTrack`).
pub fn ytm_track(t: &PlayerTrack) -> Track {
    Track {
        video_id: t.id.clone(),
        title: t.title.clone(),
        artists: t
            .artists
            .iter()
            .filter(|a| !a.name.is_empty() && a.name != "Unknown artist")
            .map(|a| volna_ytm::ArtistRef {
                id: a.id.clone().filter(|id| !id.is_empty()),
                name: a.name.clone(),
            })
            .collect(),
        album: None,
        duration_s: t.duration.filter(|d| *d > 0.0).map(|d| d.round() as u32),
        thumbnails: vec![],
        explicit: t.explicit,
        kind: if t.kind == TrackKind::Video {
            YtmKind::Video
        } else {
            YtmKind::Song
        },
        year: None,
        views: None,
        playable: true,
    }
}

/// The `https://` source behind a cover URL the UI holds
/// (`volna://img/?u=…&s=…` or `http://volna.localhost/img/?u=…`).
pub fn cover_source(cover: &str) -> Option<String> {
    if cover.starts_with("https://") {
        return Some(cover.to_owned());
    }
    let url = url::Url::parse(cover).ok()?;
    url.query_pairs()
        .find(|(k, _)| k == "u")
        .map(|(_, v)| v.into_owned())
        .filter(|u| u.starts_with("https://"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use volna_ytm::{AlbumRef as YAlbum, ArtistRef as YArtist, Thumbnail};

    fn ytm() -> Track {
        Track {
            video_id: "dQw4w9WgXcQ".into(),
            title: "Never Gonna Give You Up".into(),
            artists: vec![YArtist {
                id: Some("MPLAUCuAXFkgsw1L7xaCfnd5JJOw".into()),
                name: "Rick Astley".into(),
            }],
            album: Some(YAlbum {
                id: Some("MPREb_x".into()),
                name: "Whenever You Need Somebody".into(),
            }),
            duration_s: Some(213),
            thumbnails: vec![Thumbnail {
                url: "https://lh3.googleusercontent.com/abc=w60-h60".into(),
                width: 60,
                height: 60,
            }],
            explicit: false,
            kind: YtmKind::Song,
            year: None,
            views: None,
            playable: true,
        }
    }

    #[test]
    fn ytm_to_input_and_back() {
        let input = track_input(&ytm(), Origin::Wave);
        assert_eq!(
            input.artists[0].id.as_deref(),
            Some("UCuAXFkgsw1L7xaCfnd5JJOw")
        );
        assert_eq!(input.duration, Some(213.0));
        assert_eq!(input.origin, Some(Origin::Wave));
        let raw = input.raw.as_ref().unwrap();
        assert_eq!(raw["id"], "dQw4w9WgXcQ");
        assert_eq!(raw["album"]["id"], "MPREb_x");
        let cover = input.cover.clone().unwrap();
        assert!(cover.contains("img"), "{cover}");
        assert_eq!(
            cover_source(&cover).as_deref(),
            Some("https://lh3.googleusercontent.com/abc=w60-h60")
        );
        let back = ytm_track(&PlayerTrack::from_input("q1".into(), input));
        assert_eq!(back.video_id, "dQw4w9WgXcQ");
        assert_eq!(back.duration_s, Some(213));
        assert_eq!(back.artists[0].name, "Rick Astley");
    }

    #[test]
    fn cover_sources() {
        assert_eq!(
            cover_source(
                "http://volna.localhost/img/?u=https%3A%2F%2Fi.ytimg.com%2Fvi%2Fx%2Fhq.jpg&s=544"
            )
            .as_deref(),
            Some("https://i.ytimg.com/vi/x/hq.jpg")
        );
        assert_eq!(cover_source("volna://img/?s=544"), None);
        assert_eq!(cover_source("data:image/png;base64,xx"), None);
    }
}
