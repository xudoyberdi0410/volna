//! Import from Yandex Music: sign in through a separate window, then read the
//! account's library with the public music API.
//!
//! The token comes from Yandex's implicit OAuth flow for the music client: after
//! sign-in the login window lands on `music.yandex.ru/#access_token=...`, and we
//! take the token from that URL. The window keeps its cookies, so the next import
//! only flashes it. It has no IPC capabilities (only "main" does).
//!
//! The API is reached directly, not through the app proxy: it answers from
//! Uzbekistan, while the proxy's exit country may not have the service.
//! Matching the tracks to YouTube Music is the UI's job (`ui/js/yandex-import.js`).

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::oneshot;
use url::Url;

type CmdResult<T> = Result<T, String>;

const API: &str = "https://api.music.yandex.net";
const AUTHORIZE_URL: &str = "https://oauth.yandex.ru/authorize?response_type=token&client_id=23cabbbdc6cd418abb4b39c32c41195d";
const LOGIN_WINDOW: &str = "yandex-login";
/// The id of the "Мне нравится" playlist; its tracks come from the likes instead.
const LIKES_KIND: i64 = 3;
/// How many ids one `POST /tracks` gets.
const TRACKS_BATCH: usize = 250;

/// Managed state: the token of the last sign-in, kept for the app's lifetime.
#[derive(Default)]
pub struct YandexState {
    token: Mutex<Option<String>>,
    /// One import at a time; a second click gets an error instead of a second window.
    busy: tokio::sync::Mutex<()>,
}

impl YandexState {
    fn token(&self) -> Option<String> {
        self.token.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_token(&self, token: Option<String>) {
        *self.token.lock().unwrap_or_else(|e| e.into_inner()) = token;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub title: String,
    /// "Remix", "Live" and the like; Yandex keeps it apart from the title.
    pub version: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LikedTrack {
    #[serde(flatten)]
    pub track: Track,
    /// When it was liked (ISO 8601), so the library keeps the order.
    pub liked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub title: String,
    pub description: Option<String>,
    /// False for someone else's playlist the user liked.
    pub own: bool,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub title: String,
    pub artists: Vec<String>,
    pub year: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub login: Option<String>,
    pub liked_tracks: Vec<LikedTrack>,
    pub playlists: Vec<Playlist>,
    pub albums: Vec<Album>,
    pub artists: Vec<String>,
}

/// Progress for the UI, emitted as the `yandex-progress` event.
#[derive(Clone, Serialize)]
struct Progress<'a> {
    stage: &'a str,
    detail: Option<String>,
}

fn progress(app: &AppHandle, stage: &str, detail: Option<String>) {
    let _ = app.emit("yandex-progress", Progress { stage, detail });
}

/// Signs in if needed and returns the whole library. A stale token gets one
/// fresh sign-in.
#[tauri::command]
pub async fn yandex_export(app: AppHandle, state: State<'_, YandexState>) -> CmdResult<Library> {
    let _busy = state
        .busy
        .try_lock()
        .map_err(|_| "импорт уже идёт".to_owned())?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut fresh = false;
    loop {
        let token = match state.token() {
            Some(t) => t,
            None => {
                progress(&app, "login", None);
                fresh = true;
                let t = sign_in(&app).await?;
                state.set_token(Some(t.clone()));
                t
            }
        };
        let client = Client {
            http: http.clone(),
            token,
        };
        match export(&app, &client).await {
            Err(ApiError::Unauthorized) if !fresh => state.set_token(None),
            Err(ApiError::Unauthorized) => {
                state.set_token(None);
                return Err("Яндекс не принял вход, попробуйте ещё раз".to_owned());
            }
            Err(e) => return Err(e.to_string()),
            Ok(library) => return Ok(library),
        }
    }
}

/// Opens the login window and waits for the redirect with the token.
async fn sign_in(app: &AppHandle) -> CmdResult<String> {
    if let Some(old) = app.get_webview_window(LOGIN_WINDOW) {
        let _ = old.destroy();
    }
    let (tx, rx) = oneshot::channel::<Option<String>>();
    let tx = std::sync::Arc::new(Mutex::new(Some(tx)));
    let send = {
        let tx = tx.clone();
        move |value: Option<String>| {
            if let Some(tx) = tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = tx.send(value);
            }
        }
    };

    let on_nav = send.clone();
    let window = WebviewWindowBuilder::new(
        app,
        LOGIN_WINDOW,
        WebviewUrl::External(AUTHORIZE_URL.parse().expect("valid URL")),
    )
    .title("Вход в Яндекс")
    .inner_size(480.0, 720.0)
    // Some engines report the redirect with its fragment here; page loads and
    // the poll below cover the ones that do not.
    .on_navigation(move |url| match token_from_url(url) {
        Some(token) => {
            on_nav(Some(token));
            false
        }
        None => true,
    })
    .on_page_load({
        let send = send.clone();
        move |_, payload| {
            if let Some(token) = token_from_url(payload.url()) {
                send(Some(token));
            }
        }
    })
    .build()
    .map_err(|e| format!("не удалось открыть окно входа: {e}"))?;

    let on_close = send.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            on_close(None);
        }
    });

    let poll = {
        let window = window.clone();
        let send = send.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(300)).await;
                match window.url() {
                    Ok(url) => {
                        if let Some(token) = token_from_url(&url) {
                            send(Some(token));
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
        })
    };

    let token = rx.await.ok().flatten();
    poll.abort();
    let _ = window.destroy();
    token.ok_or_else(|| "вход в Яндекс отменён".to_owned())
}

/// `https://music.yandex.ru/#access_token=...&token_type=bearer` → the token.
fn token_from_url(url: &Url) -> Option<String> {
    let fragment = url.fragment()?;
    url::form_urlencoded::parse(fragment.as_bytes())
        .find(|(k, _)| k == "access_token")
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty())
}

#[derive(Debug, thiserror::Error)]
enum ApiError {
    #[error("Яндекс не принял токен")]
    Unauthorized,
    #[error("Яндекс Музыка недоступна: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Яндекс Музыка ответила {status} на {path}")]
    Status { status: u16, path: String },
    #[error("непонятный ответ Яндекс Музыки на {0}")]
    Shape(String),
}

struct Client {
    http: reqwest::Client,
    token: String,
}

impl Client {
    async fn send(&self, path: &str, form: Option<String>) -> Result<Value, ApiError> {
        let url = format!("{API}{path}");
        let req = match form {
            Some(body) => self
                .http
                .post(url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body),
            None => self.http.get(url),
        };
        let resp = req
            .header("Authorization", format!("OAuth {}", self.token))
            .header("Accept-Language", "ru")
            .send()
            .await?;
        let status = resp.status();
        // 403 is per resource (a private playlist), not a bad token.
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }
        if !status.is_success() {
            return Err(ApiError::Status {
                status: status.as_u16(),
                path: path.to_owned(),
            });
        }
        let mut body: Value = resp.json().await?;
        body.get_mut("result")
            .map(Value::take)
            .ok_or_else(|| ApiError::Shape(path.to_owned()))
    }

    async fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.send(path, None).await
    }

    /// Full track objects for `ids`, in batches.
    async fn tracks(&self, ids: &[String]) -> Result<Vec<Track>, ApiError> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(TRACKS_BATCH) {
            let body = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("track-ids", &chunk.join(","))
                .append_pair("with-positions", "false")
                .finish();
            let result = self.send("/tracks", Some(body)).await?;
            out.extend(as_array(&result).iter().filter_map(parse_track));
        }
        Ok(out)
    }

    /// One playlist with its tracks; tracks the response left bare are fetched.
    async fn playlist(&self, owner: &str, kind: i64, own: bool) -> Result<Playlist, ApiError> {
        let result = self
            .get(&format!("/users/{owner}/playlists/{kind}"))
            .await?;
        let mut tracks = Vec::new();
        let mut bare = Vec::new();
        for item in as_array(&result["tracks"]) {
            match item.get("track").and_then(parse_track) {
                Some(t) => tracks.push(t),
                None => bare.extend(id_string(&item["id"])),
            }
        }
        if !bare.is_empty() {
            tracks.extend(self.tracks(&bare).await?);
        }
        Ok(Playlist {
            title: str_field(&result, "title").unwrap_or_else(|| "Плейлист".to_owned()),
            description: str_field(&result, "description").filter(|d| !d.is_empty()),
            own,
            tracks,
        })
    }
}

async fn export(app: &AppHandle, client: &Client) -> Result<Library, ApiError> {
    progress(app, "account", None);
    let status = client.get("/account/status").await?;
    let account = &status["account"];
    let uid = id_string(&account["uid"]).ok_or(ApiError::Unauthorized)?;
    let mut library = Library {
        login: str_field(account, "login"),
        ..Library::default()
    };

    progress(app, "likes", None);
    let likes = client
        .get(&format!(
            "/users/{uid}/likes/tracks?if-modified-since-revision=0"
        ))
        .await?;
    let liked = parse_likes(&likes);
    let ids: Vec<String> = liked.iter().map(|(id, _)| id.clone()).collect();
    let mut details: std::collections::HashMap<String, Track> = client
        .tracks(&ids)
        .await?
        .into_iter()
        .map(|t| (t.id.clone(), t))
        .collect();
    library.liked_tracks = liked
        .into_iter()
        .filter_map(|(id, liked_at)| {
            details
                .remove(&id)
                .map(|track| LikedTrack { track, liked_at })
        })
        .collect();

    progress(app, "playlists", None);
    let own = client.get(&format!("/users/{uid}/playlists/list")).await?;
    let mut refs: Vec<(String, i64, bool)> = as_array(&own)
        .iter()
        .filter_map(|p| p["kind"].as_i64())
        .filter(|&kind| kind != LIKES_KIND)
        .map(|kind| (uid.clone(), kind, true))
        .collect();
    // Someone else's playlists the user liked; their absence is not fatal.
    match client.get(&format!("/users/{uid}/likes/playlists")).await {
        Ok(v) => refs.extend(as_array(&v).iter().filter_map(|item| {
            let p = &item["playlist"];
            let owner = id_string(&p["uid"]).or_else(|| id_string(&p["owner"]["uid"]))?;
            Some((owner, p["kind"].as_i64()?, false))
        })),
        Err(ApiError::Unauthorized) => return Err(ApiError::Unauthorized),
        Err(e) => tracing::warn!("yandex: liked playlists: {e}"),
    }
    for (owner, kind, own) in refs {
        match client.playlist(&owner, kind, own).await {
            Ok(p) => {
                progress(app, "playlists", Some(p.title.clone()));
                library.playlists.push(p);
            }
            Err(ApiError::Unauthorized) => return Err(ApiError::Unauthorized),
            Err(e) => tracing::warn!("yandex: playlist {owner}/{kind}: {e}"),
        }
    }

    progress(app, "albums", None);
    match client
        .get(&format!("/users/{uid}/likes/albums?rich=true"))
        .await
    {
        Ok(v) => library.albums = parse_albums(&v),
        Err(ApiError::Unauthorized) => return Err(ApiError::Unauthorized),
        Err(e) => tracing::warn!("yandex: liked albums: {e}"),
    }
    match client
        .get(&format!("/users/{uid}/likes/artists?with-timestamps=false"))
        .await
    {
        Ok(v) => library.artists = parse_artists(&v),
        Err(ApiError::Unauthorized) => return Err(ApiError::Unauthorized),
        Err(e) => tracing::warn!("yandex: liked artists: {e}"),
    }

    tracing::info!(
        tracks = library.liked_tracks.len(),
        playlists = library.playlists.len(),
        albums = library.albums.len(),
        artists = library.artists.len(),
        "yandex library exported"
    );
    Ok(library)
}

// ---- response parsing ------------------------------------------------------

fn as_array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_owned)
}

/// Yandex sends ids as numbers or strings, depending on the endpoint.
fn id_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn names(v: &Value) -> Vec<String> {
    as_array(v)
        .iter()
        .filter_map(|a| str_field(a, "name"))
        .collect()
}

fn parse_track(v: &Value) -> Option<Track> {
    let title = str_field(v, "title")?;
    Some(Track {
        id: id_string(&v["id"])?,
        title,
        version: str_field(v, "version").filter(|s| !s.is_empty()),
        artists: names(&v["artists"]),
        album: as_array(&v["albums"])
            .first()
            .and_then(|a| str_field(a, "title")),
        duration_ms: v["durationMs"].as_u64(),
    })
}

/// `(track id, liked at)` from `/likes/tracks`, newest first as Yandex sends them.
fn parse_likes(v: &Value) -> Vec<(String, Option<String>)> {
    as_array(&v["library"]["tracks"])
        .iter()
        .filter_map(|t| Some((id_string(&t["id"])?, str_field(t, "timestamp"))))
        .collect()
}

fn parse_albums(v: &Value) -> Vec<Album> {
    as_array(v)
        .iter()
        .filter_map(|item| {
            let a = item.get("album").unwrap_or(item);
            Some(Album {
                title: str_field(a, "title")?,
                artists: names(&a["artists"]),
                year: a["year"].as_i64(),
            })
        })
        .collect()
}

/// Accepts both `[{artist: {...}}]` and `[{name: ...}]`.
fn parse_artists(v: &Value) -> Vec<String> {
    as_array(v)
        .iter()
        .filter_map(|item| str_field(item.get("artist").unwrap_or(item), "name"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn token_comes_from_the_fragment() {
        let url: Url =
            "https://music.yandex.ru/#access_token=y0_AbC-1&token_type=bearer&expires_in=31536000"
                .parse()
                .unwrap();
        assert_eq!(token_from_url(&url).as_deref(), Some("y0_AbC-1"));
        let no_token: Url = "https://passport.yandex.ru/auth?retpath=x#foo=bar"
            .parse()
            .unwrap();
        assert_eq!(token_from_url(&no_token), None);
    }

    #[test]
    fn parses_a_track_with_numeric_id() {
        let t = parse_track(&json!({
            "id": 12345, "title": "Song", "version": "Remix", "durationMs": 201000,
            "artists": [{"name": "A"}, {"name": "B"}],
            "albums": [{"id": 1, "title": "Album"}]
        }))
        .unwrap();
        assert_eq!(t.id, "12345");
        assert_eq!(t.version.as_deref(), Some("Remix"));
        assert_eq!(t.artists, ["A", "B"]);
        assert_eq!(t.album.as_deref(), Some("Album"));
        assert_eq!(t.duration_ms, Some(201000));
    }

    #[test]
    fn parses_likes_albums_and_artists() {
        let likes = json!({"library": {"uid": 1, "tracks": [
            {"id": "10", "albumId": "2", "timestamp": "2024-01-01T00:00:00+00:00"},
            {"id": "11"}
        ]}});
        assert_eq!(
            parse_likes(&likes),
            [
                (
                    "10".to_owned(),
                    Some("2024-01-01T00:00:00+00:00".to_owned())
                ),
                ("11".to_owned(), None)
            ]
        );
        let albums = parse_albums(
            &json!([{"album": {"title": "X", "year": 2001, "artists": [{"name": "Y"}]}}]),
        );
        assert_eq!(albums[0].title, "X");
        assert_eq!(albums[0].artists, ["Y"]);
        assert_eq!(
            parse_artists(&json!([{"name": "One"}, {"artist": {"name": "Two"}}])),
            ["One", "Two"]
        );
    }
}
