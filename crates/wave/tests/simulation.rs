//! A scripted listener against a fake catalogue: the wave must learn.
//!
//! Six artists with 100 songs each. Every radio mixes all of them, every
//! artist page lists its top songs and the other artists as related. The
//! listener picked four artists on the cold-start screen, then skips every
//! track by one of them ("Hated") within seconds and listens to the rest.
//! After a few skips that artist must vanish from the wave.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use volna_wave::{
    ArtistBrief, ArtistInfo, CandidateSource, MoodQuery, PlayerEvent, Result, SeedArtist, Store,
    Wave,
};
use volna_ytm::{ArtistRef, Track, TrackKind};

const ARTISTS: [&str; 6] = ["Loved", "Hated", "Fine", "Okay", "Fresh", "Other"];

fn song(artist: &str, n: usize) -> Track {
    Track {
        video_id: format!("{artist}-{n:02}"),
        title: format!("{artist} song {n}"),
        artists: vec![ArtistRef {
            id: Some(format!("UC{artist}")),
            name: artist.into(),
        }],
        album: None,
        duration_s: Some(180),
        thumbnails: vec![],
        explicit: false,
        kind: TrackKind::Song,
        year: None,
        views: Some(format!("{}K plays", 100 + n)),
        playable: true,
    }
}

/// Cheap deterministic hash so each seed gets its own radio order.
fn mix(s: &str) -> usize {
    s.bytes().fold(1469598103934665603u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(1099511628211)
    }) as usize
}

struct Catalogue;

impl CandidateSource for Catalogue {
    async fn radio(&self, seed: &str) -> Result<Vec<Track>> {
        let h = mix(seed);
        Ok((0..48)
            .map(|i| song(ARTISTS[(i + h) % ARTISTS.len()], (i * 7 + h) % 100))
            .collect())
    }

    async fn related(&self, seed: &str) -> Result<Vec<Track>> {
        let h = mix(seed) / 7;
        Ok((0..12)
            .map(|i| song(ARTISTS[(i + h) % ARTISTS.len()], (i * 3 + h) % 100))
            .collect())
    }

    async fn artist(&self, id: &str) -> Result<ArtistInfo> {
        let name = id.trim_start_matches("UC");
        Ok(ArtistInfo {
            top_tracks: (0..10).map(|n| song(name, n)).collect(),
            related: ARTISTS
                .iter()
                .filter(|a| **a != name)
                .map(|a| ArtistBrief {
                    id: format!("UC{a}"),
                    name: (*a).into(),
                    thumbnail: None,
                })
                .collect(),
        })
    }

    async fn mood_tracks(&self, _: &MoodQuery) -> Result<Vec<Track>> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn skipped_artist_disappears_from_the_wave() {
    let now = Arc::new(AtomicI64::new(1_800_000_000));
    let clock = now.clone();
    let wave = Wave::new(Store::open_in_memory().unwrap(), Catalogue)
        .with_test_hooks(42, Arc::new(move || clock.load(Ordering::SeqCst)));

    assert!(wave.status().await.unwrap().cold);
    let picks: Vec<SeedArtist> = ["Loved", "Hated", "Fine", "Okay"]
        .iter()
        .map(|a| SeedArtist {
            id: format!("UC{a}"),
            name: (*a).into(),
        })
        .collect();
    wave.add_seed_artists(&picks).unwrap();
    assert!(!wave.status().await.unwrap().cold);

    // The UI's loop: a queue that is topped up when fewer than 3 are left.
    let mut queue: std::collections::VecDeque<Track> = wave.start(5).await.unwrap().into();
    let mut heard: Vec<String> = Vec::new();
    let mut ids = std::collections::HashSet::new();
    while heard.len() < 80 {
        if queue.len() < 3 {
            queue.extend(wave.next(5).await.unwrap());
        }
        let track = queue.pop_front().unwrap();
        assert!(
            ids.insert(track.video_id.clone()),
            "repeat: {}",
            track.video_id
        );
        let artist = track.artists[0].name.clone();
        let position = if artist == "Hated" { 4.0 } else { 175.0 };

        wave.record(PlayerEvent::Start {
            track: track.clone(),
            in_wave: true,
        })
        .await
        .unwrap();
        now.fetch_add(position as i64 + 1, Ordering::SeqCst);
        let out = wave
            .record(PlayerEvent::End {
                track,
                position_s: position,
                duration_s: Some(180.0),
                ended: false,
                in_wave: true,
            })
            .await
            .unwrap();
        if out.reseeded {
            // What the UI does: give back the stale upcoming tracks.
            let dropped: Vec<String> = queue.drain(..).map(|t| t.video_id).collect();
            queue.extend(wave.requeue(&dropped, 3).await.unwrap());
        }
        heard.push(artist);
    }

    let hated_early = heard[..20].iter().filter(|a| *a == "Hated").count();
    let hated_late = heard[30..].iter().filter(|a| *a == "Hated").count();
    eprintln!("heard: {heard:?}");
    assert!(
        hated_early >= 1,
        "the hated artist never came up: {heard:?}"
    );
    assert_eq!(hated_late, 0, "still playing the skipped artist: {heard:?}");
    // No artist twice in a row.
    for pair in heard.windows(2) {
        assert_ne!(pair[0], pair[1], "{heard:?}");
    }
    // The wave did not collapse to one artist.
    let distinct: std::collections::HashSet<_> = heard[30..].iter().collect();
    assert!(distinct.len() >= 4, "{distinct:?}");
}
