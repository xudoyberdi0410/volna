//! YouTube Music models → the hifi-api (TIDAL-shaped) JSON the Monochrome UI
//! reads. Pure functions; the field rules come from
//! `docs/hifi-api-contract.md` §2 and §9.

use serde_json::{Map, Value, json};
use volna_ytm::{
    Album, AlbumKind, AlbumSummary, Artist, ArtistRef, ArtistSummary, Item, PlaylistSummary,
    Section, Thumbnail, Track,
};

/// `"version"` field every hifi-api response carries.
pub const VERSION: &str = "2.7";
/// Default cover edge in px. The UI asks for 80–1280 and may rewrite `s`.
pub const COVER_SIZE: u32 = 544;

/// `volna://<route><path>?<query>` on Linux/macOS,
/// `http://volna.localhost/<route><path>?<query>` on Windows (WebView2).
/// Mirrors `ui/js/volna-url.js`.
pub fn volna_url(route: &str, path: &str, query: &[(&str, &str)]) -> String {
    let base = if cfg!(windows) {
        format!("http://volna.localhost/{route}")
    } else {
        format!("volna://{route}")
    };
    let mut url = format!("{base}{path}");
    if !query.is_empty() {
        let qs = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(query)
            .finish();
        url.push('?');
        url.push_str(&qs);
    }
    url
}

/// Cover URL served by the Rust image cache (`volna://img/?u=…&s=…`).
pub fn image_url(source: &str, size: u32) -> String {
    volna_url("img", "/", &[("u", source), ("s", &size.to_string())])
}

/// The largest thumbnail (YouTube lists them small → large, but width is
/// sometimes 0, so compare instead of taking the last one blindly).
pub fn best_thumbnail(thumbs: &[Thumbnail]) -> Option<&str> {
    thumbs
        .iter()
        .enumerate()
        .max_by_key(|(i, t)| (u64::from(t.width) * u64::from(t.height), *i))
        .map(|(_, t)| t.url.as_str())
        .filter(|u| u.starts_with("https://"))
}

pub fn cover(thumbs: &[Thumbnail]) -> Value {
    best_thumbnail(thumbs).map_or(Value::Null, |u| json!(image_url(u, COVER_SIZE)))
}

/// Library artist ids come as `MPLAUC…`; pages want the bare `UC…`.
pub fn clean_artist_id(id: &str) -> &str {
    id.strip_prefix("MPLA").unwrap_or(id)
}

fn artist_ref(a: &ArtistRef) -> Value {
    json!({
        "id": a.id.as_deref().map(clean_artist_id),
        "name": a.name,
        "type": "MAIN",
        "picture": Value::Null,
    })
}

/// `artists[]` is required to be non-empty by the renderers.
fn artist_refs(artists: &[ArtistRef]) -> Vec<Value> {
    if artists.is_empty() {
        return vec![
            json!({ "id": null, "name": "Unknown artist", "type": "MAIN", "picture": null }),
        ];
    }
    artists.iter().map(artist_ref).collect()
}

fn release_date(year: Option<u16>) -> Option<String> {
    year.map(|y| format!("{y:04}-01-01"))
}

/// Album kind → hifi-api `type`.
fn album_type(kind: AlbumKind) -> &'static str {
    match kind {
        AlbumKind::Album => "ALBUM",
        AlbumKind::Single => "SINGLE",
        AlbumKind::Ep => "EP",
    }
}

/// Context a track gets from the page it was listed on.
#[derive(Default, Clone, Copy)]
pub struct TrackCtx<'a> {
    /// Parent album (album pages), used when the row has no album link.
    pub album: Option<&'a Album>,
    /// 1-based position, for album pages.
    pub track_number: Option<usize>,
    /// `popularity` value; the artist page sorts top tracks by it.
    pub popularity: Option<u32>,
}

/// A playable item (§2.1). Every YouTube video becomes `type: "track"`.
pub fn track(t: &Track, ctx: TrackCtx<'_>) -> Value {
    let artists = if t.artists.is_empty() {
        ctx.album.map(|a| a.artists.as_slice()).unwrap_or_default()
    } else {
        t.artists.as_slice()
    };
    let artists = artist_refs(artists);
    let year = t.year.or(ctx.album.and_then(|a| a.year));

    // Thumbnails of album rows are often empty: fall back to the album cover.
    let mut cover = cover(&t.thumbnails);
    if cover.is_null()
        && let Some(album) = ctx.album
    {
        cover = self::cover(&album.thumbnails);
    }

    // TrackAlbumRef must stay slim (no numberOfTracks / type), see §2.1.
    let mut album = Map::new();
    match (&t.album, ctx.album) {
        (Some(a), _) if a.id.is_some() => {
            album.insert("id".into(), json!(a.id));
            album.insert("title".into(), json!(a.name));
        }
        (_, Some(parent)) => {
            album.insert("id".into(), json!(parent.browse_id));
            album.insert("title".into(), json!(parent.title));
        }
        (Some(a), None) => {
            album.insert("id".into(), Value::Null);
            album.insert("title".into(), json!(a.name));
        }
        (None, None) => {
            album.insert("id".into(), Value::Null);
            album.insert("title".into(), json!(""));
        }
    }
    album.insert("cover".into(), cover);
    if let Some(d) = release_date(year) {
        album.insert("releaseDate".into(), json!(d));
    }

    let mut v = json!({
        "id": t.video_id,
        "title": t.title,
        "version": null,
        "duration": t.duration_s.unwrap_or(0),
        "artist": artists[0],
        "artists": artists,
        "album": album,
        "type": "track",
        "explicit": t.explicit,
        "volumeNumber": 1,
        "mixes": { "TRACK_MIX": format!("RDAMVM{}", t.video_id) },
        "audioQuality": "HIGH",
        "isrc": null,
        "copyright": null,
        "mediaMetadata": { "tags": [] },
    });
    let obj = v.as_object_mut().expect("a JSON object literal");
    if let Some(n) = ctx.track_number {
        obj.insert("trackNumber".into(), json!(n));
    }
    if let Some(p) = ctx.popularity {
        obj.insert("popularity".into(), json!(p));
    }
    if let Some(d) = release_date(year) {
        obj.insert(
            "streamStartDate".into(),
            json!(format!("{d}T00:00:00.000+0000")),
        );
    }
    if !t.playable {
        obj.insert("allowStreaming".into(), json!(false));
        obj.insert("streamReady".into(), json!(false));
    }
    v
}

/// `{ "item": Track, "type": "track" }`, the element shape of paged lists.
pub fn track_item(t: &Track, ctx: TrackCtx<'_>) -> Value {
    json!({ "item": track(t, ctx), "type": "track" })
}

/// Album as listed in search, artist shelves and carousels (§2.2).
/// `owner` replaces the artist when the listing does not repeat it (artist
/// pages), so the UI's "belongs to this artist" filter keeps it.
pub fn album_summary(a: &AlbumSummary, owner: Option<(&str, &str)>) -> Value {
    let artists: Vec<Value> = match owner {
        Some((id, name)) if a.artists.iter().all(|r| r.id.as_deref() != Some(id)) => {
            let mut v = vec![json!({ "id": id, "name": name, "type": "MAIN", "picture": null })];
            v.extend(a.artists.iter().map(artist_ref));
            v
        }
        _ => artist_refs(&a.artists),
    };
    let mut v = json!({
        "id": a.browse_id,
        "title": a.title,
        "cover": cover(&a.thumbnails),
        "artist": { "id": artists[0]["id"], "name": artists[0]["name"] },
        "artists": artists,
        "type": album_type(a.kind),
        "explicit": a.explicit,
        // The key must exist: the artist-page scanner recognises albums by
        // it. Shelves do not say how many tracks there are.
        "numberOfTracks": null,
        "numberOfVolumes": 1,
        "copyright": null,
    });
    if let Some(d) = release_date(a.year) {
        v["releaseDate"] = json!(d);
    }
    v
}

/// The album page body (§3.4) with `items` sliced to `[offset, offset+limit)`.
pub fn album_page(a: &Album, offset: usize, limit: usize) -> Value {
    let tracks: Vec<(usize, &Track)> = a
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| (i + 1, t))
        .collect();
    let items: Vec<Value> = tracks
        .iter()
        .skip(offset)
        .take(limit)
        .map(|(n, t)| {
            track_item(
                t,
                TrackCtx {
                    album: Some(a),
                    track_number: Some(*n),
                    popularity: None,
                },
            )
        })
        .collect();
    let artists = artist_refs(&a.artists);
    let mut v = json!({
        "id": a.browse_id,
        "title": a.title,
        "cover": cover(&a.thumbnails),
        "artist": { "id": artists[0]["id"], "name": artists[0]["name"] },
        "artists": artists,
        "type": album_type(a.kind),
        "numberOfTracks": a.tracks.len(),
        "numberOfVolumes": 1,
        "duration": a.duration_s,
        "explicit": a.explicit,
        "copyright": null,
        "description": a.description,
        "items": items,
    });
    if let Some(d) = release_date(a.year) {
        v["releaseDate"] = json!(d);
    }
    v
}

/// Artist in lists (search, related). `picture` is required in search
/// results or the UI fetches every artist page for it (§2.3).
pub fn artist_summary(a: &ArtistSummary) -> Value {
    json!({
        "id": clean_artist_id(&a.id),
        "name": a.name,
        "picture": cover(&a.thumbnails),
        "type": "ARTIST",
    })
}

/// The artist profile object of `/artist/` (§2.3).
pub fn artist_profile(a: &Artist) -> Value {
    let mut v = json!({
        "id": clean_artist_id(&a.id),
        "name": a.name,
        "picture": cover(&a.thumbnails),
    });
    let mix = a
        .radio_playlist_id
        .clone()
        .or_else(|| a.top_songs.first().map(|t| format!("RDAMVM{}", t.video_id)));
    if let Some(mix) = mix {
        v["mixes"] = json!({ "ARTIST_MIX": mix });
    }
    if let Some(text) = a.description.as_deref().filter(|t| !t.trim().is_empty()) {
        v["biography"] = json!({ "text": text, "source": "YouTube Music" });
    }
    v
}

/// Radio queues (`RD…`) are not browseable playlists, except YouTube Music's
/// own curated `RDCLAK…` lists. Those open the mix page instead (§9.4).
pub fn is_radio_id(id: &str) -> bool {
    id.starts_with("RD") && !id.starts_with("RDCLAK")
}

/// Playlist in lists and as the `/playlist/` header (§2.4). `uuid` never
/// carries the `VL` browse prefix.
pub fn playlist_summary(p: &PlaylistSummary) -> Value {
    playlist_header(
        &p.id,
        &p.title,
        p.author.as_ref(),
        None,
        &p.thumbnails,
        p.track_count,
    )
}

pub fn playlist_header(
    id: &str,
    title: &str,
    author: Option<&ArtistRef>,
    description: Option<&str>,
    thumbnails: &[Thumbnail],
    track_count: Option<u32>,
) -> Value {
    let id = id.strip_prefix("VL").unwrap_or(id);
    let image = cover(thumbnails);
    let editorial = author.is_none_or(|a| a.name == "YouTube Music");
    let mut v = json!({
        "uuid": id,
        "id": id,
        "title": title,
        "numberOfTracks": track_count.unwrap_or(0),
        "squareImage": image,
        "image": image,
        "description": description.unwrap_or_default(),
        "type": if editorial { "EDITORIAL" } else { "USER" },
        "creator": author.map(|a| json!({ "id": a.id, "name": a.name })),
    });
    if is_radio_id(id) {
        v["_href"] = json!(format!("/mix/{id}"));
    }
    v
}

/// The `mix` header of `/mix/` (§2.5).
pub fn mix_header(id: &str, title: &str, subtitle: &str, cover: Value) -> Value {
    let mix_type = if id.starts_with("RDAMVM") {
        "TRACK_MIX"
    } else if id.starts_with("RDEM") || id.starts_with("RDAO") {
        "ARTIST_MIX"
    } else {
        "DISCOVERY_MIX"
    };
    json!({
        "id": id,
        "title": title,
        // Inserted with innerHTML by the unpatched UI; escape to be safe.
        "subTitle": escape_html(subtitle),
        "description": "",
        "mixType": mix_type,
        "images": { "LARGE": { "url": cover } },
    })
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A search bucket: `{items, limit, offset, totalNumberOfItems}`.
pub fn bucket(items: Vec<Value>) -> Value {
    let n = items.len();
    json!({ "items": items, "limit": n, "offset": 0, "totalNumberOfItems": n })
}

/// One explore/home shelf in the `{type, title, items}` shape the explore
/// renderer understands. Mixed YTM shelves keep their dominant item type.
pub fn section(s: &Section) -> Option<Value> {
    let count = |f: fn(&Item) -> bool| s.items.iter().filter(|i| f(i)).count();
    let kinds = [
        ("TRACK_LIST", count(|i| matches!(i, Item::Track(_)))),
        ("ALBUM_LIST", count(|i| matches!(i, Item::Album(_)))),
        ("PLAYLIST_LIST", count(|i| matches!(i, Item::Playlist(_)))),
        ("ARTIST_LIST", count(|i| matches!(i, Item::Artist(_)))),
    ];
    let (kind, n) = kinds.into_iter().max_by_key(|(_, n)| *n)?;
    if n == 0 {
        return None;
    }
    let items: Vec<Value> = s
        .items
        .iter()
        .filter_map(|i| match (kind, i) {
            ("TRACK_LIST", Item::Track(t)) => Some(track(t, TrackCtx::default())),
            ("ALBUM_LIST", Item::Album(a)) => Some(album_summary(a, None)),
            ("PLAYLIST_LIST", Item::Playlist(p)) => Some(playlist_summary(p)),
            ("ARTIST_LIST", Item::Artist(a)) => Some(artist_summary(a)),
            _ => None,
        })
        .collect();
    Some(json!({ "type": kind, "title": s.title, "subtitle": s.strapline, "items": items }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use volna_ytm::{AlbumRef, TrackKind};

    fn thumb(url: &str, w: u32) -> Thumbnail {
        Thumbnail {
            url: url.into(),
            width: w,
            height: w,
        }
    }

    fn song(id: &str) -> Track {
        Track {
            video_id: id.into(),
            title: "Song".into(),
            artists: vec![ArtistRef {
                id: Some("MPLAUCabc".into()),
                name: "Artist".into(),
            }],
            album: Some(AlbumRef {
                id: Some("MPREb_x".into()),
                name: "Album".into(),
            }),
            duration_s: Some(200),
            thumbnails: vec![
                thumb("https://lh3.googleusercontent.com/a=w60-h60", 60),
                thumb("https://lh3.googleusercontent.com/a=w544-h544", 544),
                thumb("https://lh3.googleusercontent.com/a=w120-h120", 120),
            ],
            explicit: true,
            kind: TrackKind::Song,
            year: Some(2019),
            views: None,
            playable: true,
        }
    }

    #[test]
    fn volna_urls_match_the_js_helper() {
        let u = image_url("https://i.ytimg.com/vi/x/hq.jpg", 100);
        if cfg!(windows) {
            assert!(u.starts_with("http://volna.localhost/img/?u="));
        } else {
            assert_eq!(
                u,
                "volna://img/?u=https%3A%2F%2Fi.ytimg.com%2Fvi%2Fx%2Fhq.jpg&s=100"
            );
        }
    }

    #[test]
    fn picks_the_largest_https_thumbnail() {
        let t = song("a");
        assert_eq!(
            best_thumbnail(&t.thumbnails),
            Some("https://lh3.googleusercontent.com/a=w544-h544")
        );
        assert_eq!(best_thumbnail(&[]), None);
        // Unknown sizes: the last one wins.
        let no_size = [thumb("https://a/1", 0), thumb("https://a/2", 0)];
        assert_eq!(best_thumbnail(&no_size), Some("https://a/2"));
        assert_eq!(best_thumbnail(&[thumb("http://insecure", 10)]), None);
    }

    #[test]
    fn track_has_the_required_fields() {
        let v = track(&song("dQw4w9WgXcQ"), TrackCtx::default());
        assert_eq!(v["id"], "dQw4w9WgXcQ");
        assert_eq!(v["type"], "track");
        assert_eq!(v["duration"], 200);
        assert_eq!(v["artists"][0]["id"], "UCabc", "MPLA prefix stripped");
        assert_eq!(v["artist"]["name"], "Artist");
        assert_eq!(v["album"]["id"], "MPREb_x");
        assert!(v["album"]["cover"].as_str().unwrap().contains("s=544"));
        assert_eq!(v["album"]["releaseDate"], "2019-01-01");
        assert_eq!(v["mixes"]["TRACK_MIX"], "RDAMVMdQw4w9WgXcQ");
        assert_eq!(v["explicit"], true);
        assert!(v.get("audioUrl").is_none());
        assert!(v.get("allowStreaming").is_none());
        // The slim album ref must not look like an album to the scanner.
        let album = v["album"].as_object().unwrap();
        assert!(!album.contains_key("numberOfTracks"));
        assert!(!album.contains_key("type"));
    }

    #[test]
    fn track_without_album_or_artist_still_has_both() {
        let mut t = song("a");
        t.album = None;
        t.artists.clear();
        t.thumbnails.clear();
        t.duration_s = None;
        t.playable = false;
        let v = track(&t, TrackCtx::default());
        assert_eq!(v["album"]["id"], Value::Null);
        assert_eq!(v["album"]["title"], "");
        assert_eq!(v["album"]["cover"], Value::Null);
        assert_eq!(v["artists"].as_array().unwrap().len(), 1);
        assert_eq!(v["duration"], 0);
        assert_eq!(v["allowStreaming"], false);
    }

    fn album() -> Album {
        let mut t1 = song("aaaaaaaaaaa");
        t1.album = None;
        t1.thumbnails.clear();
        t1.artists.clear();
        let t2 = song("bbbbbbbbbbb");
        Album {
            browse_id: "MPREb_album".into(),
            playlist_id: Some("OLAK5uy_x".into()),
            title: "Album".into(),
            kind: AlbumKind::Ep,
            artists: vec![ArtistRef {
                id: Some("UCowner".into()),
                name: "Owner".into(),
            }],
            year: Some(2020),
            description: None,
            thumbnails: vec![thumb("https://lh3.googleusercontent.com/c=w544-h544", 544)],
            explicit: false,
            track_count: Some(2),
            duration_s: 400,
            tracks: vec![t1, t2],
            related: vec![],
        }
    }

    #[test]
    fn album_page_fills_tracks_from_the_parent() {
        let a = album();
        let v = album_page(&a, 0, 500);
        assert_eq!(v["numberOfTracks"], 2);
        assert_eq!(v["type"], "EP");
        assert_eq!(v["artist"]["id"], "UCowner");
        assert_eq!(v["releaseDate"], "2020-01-01");
        let first = &v["items"][0]["item"];
        assert_eq!(first["trackNumber"], 1);
        assert_eq!(first["album"]["id"], "MPREb_album");
        assert_eq!(first["album"]["cover"], v["cover"]);
        assert_eq!(first["artists"][0]["id"], "UCowner");
        assert_eq!(v["items"][1]["item"]["trackNumber"], 2);

        let page = album_page(&a, 1, 500);
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(page["items"][0]["item"]["trackNumber"], 2);
        assert!(
            album_page(&a, 5, 500)["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn artist_shelf_albums_belong_to_the_page_artist() {
        let s = AlbumSummary {
            browse_id: "MPREb_s".into(),
            playlist_id: None,
            title: "Single".into(),
            kind: AlbumKind::Single,
            artists: vec![],
            year: None,
            thumbnails: vec![],
            explicit: false,
        };
        let v = album_summary(&s, Some(("UCowner", "Owner")));
        assert_eq!(v["artist"]["id"], "UCowner");
        assert_eq!(v["type"], "SINGLE");
        assert!(v.as_object().unwrap().contains_key("numberOfTracks"));
        assert!(v.get("releaseDate").is_none());
    }

    #[test]
    fn playlists_drop_vl_and_radios_link_to_mix() {
        let v = playlist_header("VLPLabc", "x", None, None, &[], Some(30));
        assert_eq!(v["uuid"], "PLabc");
        assert_eq!(v["numberOfTracks"], 30);
        assert_eq!(v["type"], "EDITORIAL");
        assert!(v.get("_href").is_none());
        let r = playlist_header("RDEMabc", "x", None, None, &[], None);
        assert_eq!(r["_href"], "/mix/RDEMabc");
        assert!(!is_radio_id("RDCLAK5uy_abc"));
        assert!(is_radio_id("RDAMVMabc"));
    }

    #[test]
    fn mix_types_and_escaping() {
        let m = mix_header("RDAMVMx", "Radio", "<b>A & B</b>", json!("c"));
        assert_eq!(m["mixType"], "TRACK_MIX");
        assert_eq!(m["subTitle"], "&lt;b&gt;A &amp; B&lt;/b&gt;");
        assert_eq!(m["images"]["LARGE"]["url"], "c");
        assert_eq!(
            mix_header("RDEMx", "", "", Value::Null)["mixType"],
            "ARTIST_MIX"
        );
    }

    #[test]
    fn sections_keep_the_dominant_type() {
        let s = Section {
            title: "Quick picks".into(),
            strapline: None,
            items: vec![
                Item::Track(song("a")),
                Item::Track(song("b")),
                Item::Artist(ArtistSummary {
                    id: "UCx".into(),
                    name: "X".into(),
                    thumbnails: vec![],
                    audience: None,
                }),
            ],
        };
        let v = section(&s).unwrap();
        assert_eq!(v["type"], "TRACK_LIST");
        assert_eq!(v["items"].as_array().unwrap().len(), 2);
        let empty = Section {
            title: "e".into(),
            strapline: None,
            items: vec![],
        };
        assert!(section(&empty).is_none());
    }
}
