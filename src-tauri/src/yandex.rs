//! Import from Yandex Music: get a token, then read the account's library with
//! the public music API.
//!
//! The token comes from Yandex's implicit OAuth flow for the music client, which
//! ends on `music.yandex.ru/#access_token=...`. Two ways to get there:
//! - the browser: yt-dlp dumps the cookies of the browser set for YouTube in
//!   Settings → «Сеть», and `oauth.yandex.<tld>/authorize` with its Yandex session
//!   answers with the token straight away (the music client is auto-approved);
//! - a login window, when the browser has no Yandex session. The window keeps its
//!   cookies, so the next import only flashes it. It has no IPC capabilities.
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

use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

const API: &str = "https://api.music.yandex.net";
/// Yandex Music's own OAuth client.
const CLIENT_ID: &str = "23cabbbdc6cd418abb4b39c32c41195d";
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

/// Signs in if needed and returns the whole library. Tries the browser's session
/// first, then the login window; a stale token gets one fresh sign-in.
#[tauri::command]
pub async fn yandex_export(
    app: AppHandle,
    state: State<'_, YandexState>,
    app_state: State<'_, AppState>,
) -> CmdResult<Library> {
    let _busy = state
        .busy
        .try_lock()
        .map_err(|_| "импорт уже идёт".to_owned())?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let (mut tried_browser, mut tried_window) = (false, false);
    loop {
        let token = match state.token() {
            Some(t) => t,
            None => {
                let from_browser = if tried_browser {
                    None
                } else {
                    tried_browser = true;
                    browser_token(&app, &app_state).await
                };
                let t = match from_browser {
                    Some(t) => t,
                    None => {
                        tried_window = true;
                        progress(&app, "login", None);
                        sign_in(&app).await?
                    }
                };
                state.set_token(Some(t.clone()));
                t
            }
        };
        let client = Client {
            http: http.clone(),
            token,
        };
        match export(&client, &|stage, detail| progress(&app, stage, detail)).await {
            Err(ApiError::Unauthorized) if !tried_window => state.set_token(None),
            Err(ApiError::Unauthorized) => {
                state.set_token(None);
                return Err("Яндекс не принял вход, попробуйте ещё раз".to_owned());
            }
            Err(e) => return Err(e.to_string()),
            Ok(library) => return Ok(library),
        }
    }
}

/// A token from the Yandex session of the browser yt-dlp takes cookies from, or
/// `None` (no browser set, not signed in there, cookies unreadable).
async fn browser_token(app: &AppHandle, app_state: &AppState) -> Option<String> {
    let streams = app_state.streams.as_ref()?;
    let dir = app.path().app_cache_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("yandex-cookies.txt");
    progress(app, "browser", None);
    let dumped = streams.backend().dump_cookies(&path).await;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    // The jar holds every site's session; keep it on disk no longer than needed.
    let _ = std::fs::remove_file(&path);
    match dumped {
        Ok(true) => {}
        Ok(false) => return None,
        Err(e) => {
            tracing::warn!("yandex: cannot read the browser cookies: {e}");
            return None;
        }
    }
    token_from_cookies(&text).await
}

/// Trades the Yandex session in a cookies.txt for a token.
async fn token_from_cookies(text: &str) -> Option<String> {
    let jar = parse_cookie_jar(text);
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    for domain in session_domains(&jar) {
        let host = format!("oauth.{domain}");
        let resp = http
            .get(authorize_url(&host))
            .header("Cookie", cookie_header(&jar, &host))
            .send()
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("yandex: {host}: {e}");
                continue;
            }
        };
        // A redirect carries the token in Location; an auto-approve page in a
        // meta refresh.
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let token = match location.as_deref().and_then(token_in_text) {
            Some(t) => Some(t),
            None => resp.text().await.ok().as_deref().and_then(token_in_text),
        };
        if token.is_some() {
            tracing::info!("yandex: signed in with the browser session ({domain})");
            return token;
        }
        tracing::info!("yandex: the browser session on {domain} gave no token");
    }
    None
}

fn authorize_url(host: &str) -> String {
    format!("https://{host}/authorize?response_type=token&client_id={CLIENT_ID}")
}

#[derive(Debug, PartialEq, Eq)]
struct Cookie {
    /// Without the leading dot.
    domain: String,
    name: String,
    value: String,
}

/// Netscape cookies.txt; `#HttpOnly_` lines are cookies too.
fn parse_cookie_jar(text: &str) -> Vec<Cookie> {
    text.lines()
        .filter_map(|line| {
            let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
            if line.starts_with('#') {
                return None;
            }
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 7 {
                return None;
            }
            Some(Cookie {
                domain: f[0].trim_start_matches('.').to_ascii_lowercase(),
                name: f[5].to_owned(),
                value: f[6].trim_end_matches('\r').to_owned(),
            })
        })
        .collect()
}

/// `yandex.ru`, `yandex.uz`, ... wherever the jar has a signed-in session.
fn session_domains(jar: &[Cookie]) -> Vec<String> {
    let mut out: Vec<String> = jar
        .iter()
        .filter(|c| c.name == "Session_id" && c.domain.starts_with("yandex."))
        .map(|c| c.domain.clone())
        .collect();
    out.sort();
    out.dedup();
    out
}

fn cookie_header(jar: &[Cookie], host: &str) -> String {
    jar.iter()
        .filter(|c| host == c.domain || host.ends_with(&format!(".{}", c.domain)))
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The token after `access_token=` in a URL or an HTML page.
fn token_in_text(text: &str) -> Option<String> {
    let start = text.find("access_token=")? + "access_token=".len();
    let token: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .collect();
    (!token.is_empty()).then_some(token)
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
        WebviewUrl::External(authorize_url("oauth.yandex.ru").parse().expect("valid URL")),
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

/// `progress(stage, detail)` reports each step.
async fn export(
    client: &Client,
    progress: &(dyn Fn(&str, Option<String>) + Sync),
) -> Result<Library, ApiError> {
    progress("account", None);
    let status = client.get("/account/status").await?;
    let account = &status["account"];
    let uid = id_string(&account["uid"]).ok_or(ApiError::Unauthorized)?;
    let mut library = Library {
        login: str_field(account, "login"),
        ..Library::default()
    };

    progress("likes", None);
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

    progress("playlists", None);
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
                progress("playlists", Some(p.title.clone()));
                library.playlists.push(p);
            }
            Err(ApiError::Unauthorized) => return Err(ApiError::Unauthorized),
            Err(e) => tracing::warn!("yandex: playlist {owner}/{kind}: {e}"),
        }
    }

    progress("albums", None);
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

    /// The whole import through the real browser session:
    /// `VOLNA_TEST_COOKIES=chrome cargo test -p volna yandex_live -- --ignored --nocapture`.
    /// Prints counts only, never cookies or the token.
    #[tokio::test]
    #[ignore]
    async fn yandex_live_export_from_browser() {
        let browser = std::env::var("VOLNA_TEST_COOKIES").unwrap_or_else(|_| "chrome".into());
        let ytdlp = volna_streams::YtDlp::locate(None).expect("yt-dlp");
        ytdlp
            .set_cookies(volna_streams::Cookies::Browser(browser))
            .unwrap();
        let path = std::env::temp_dir().join(format!("volna-yandex-{}.txt", std::process::id()));
        assert!(ytdlp.dump_cookies(&path).await.unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let token = token_from_cookies(&text)
            .await
            .expect("signed in to Yandex in the browser");
        let client = Client {
            http: reqwest::Client::new(),
            token,
        };
        let library = export(&client, &|stage, detail| {
            println!("{stage} {}", detail.unwrap_or_default())
        })
        .await
        .unwrap();
        let in_playlists: usize = library.playlists.iter().map(|p| p.tracks.len()).sum();
        println!(
            "likes {}, playlists {} ({in_playlists} tracks), albums {}, artists {}",
            library.liked_tracks.len(),
            library.playlists.len(),
            library.albums.len(),
            library.artists.len()
        );
        assert!(!library.liked_tracks.is_empty());
        assert!(
            library
                .liked_tracks
                .iter()
                .all(|t| !t.track.artists.is_empty())
        );
    }

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
    fn finds_the_session_and_its_cookies() {
        let jar = parse_cookie_jar(
            "# Netscape HTTP Cookie File\n\
             .yandex.uz\tTRUE\t/\tTRUE\t0\tyandexuid\t1\n\
             #HttpOnly_.yandex.uz\tTRUE\t/\tTRUE\t0\tSession_id\tabc\n\
             .market.yandex.uz\tTRUE\t/\tTRUE\t0\tskid\tx\n\
             .yandex.ru\tTRUE\t/\tTRUE\t0\tyandexuid\t2\n\
             .youtube.com\tTRUE\t/\tTRUE\t0\tSID\ty\n",
        );
        assert_eq!(session_domains(&jar), ["yandex.uz"]);
        assert_eq!(
            cookie_header(&jar, "oauth.yandex.uz"),
            "yandexuid=1; Session_id=abc"
        );
    }

    #[test]
    fn token_from_a_meta_refresh() {
        let page = r#"<meta http-equiv="refresh" content='2;URL="https://music.yandex.ru/#access_token=y0_Ab-c.d&#x26;token_type=bearer"'>"#;
        assert_eq!(token_in_text(page).as_deref(), Some("y0_Ab-c.d"));
        assert_eq!(token_in_text("<html>no</html>"), None);
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
