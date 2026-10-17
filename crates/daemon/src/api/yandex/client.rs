//! Yandex Music: the token and the account's library through the public music API.
//!
//! The token comes from Yandex's implicit OAuth flow for the music client, which
//! ends on `music.yandex.ru/#access_token=...`. A Yandex session (browser
//! cookies, see `browser.rs`) gets it from `oauth.yandex.<tld>/authorize`
//! straight away, since the music client is auto-approved; a login webview gets
//! it from the redirect.
//!
//! The API is reached directly, not through the app proxy: it answers from
//! the listener's own country, while the proxy's exit country may not have the
//! service. Only `reqwest` and `serde_json` here, so it moves to the mobile core
//! as is.

use std::collections::HashMap;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use url::Url;

const API: &str = "https://api.music.yandex.net";
/// Yandex Music's own OAuth client.
pub const CLIENT_ID: &str = "23cabbbdc6cd418abb4b39c32c41195d";
/// The id of the "Мне нравится" playlist; its tracks come from the likes instead.
const LIKES_KIND: i64 = 3;
/// How many ids one `POST /tracks` gets.
const TRACKS_BATCH: usize = 250;
/// 429 retries per request: pauses of 2, 4, 8, 16 and 32 s.
const RATE_LIMIT_RETRIES: usize = 5;

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

impl Track {
    /// "Title (Version)", as Yandex shows it.
    pub fn full_title(&self) -> String {
        match &self.version {
            Some(v) => format!("{} ({v})", self.title),
            None => self.title.clone(),
        }
    }
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

/// Which parts of the library to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub likes: bool,
    pub playlists: bool,
    pub albums: bool,
    pub artists: bool,
}

impl Default for Parts {
    fn default() -> Self {
        Self {
            likes: true,
            playlists: true,
            albums: true,
            artists: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Яндекс не принял токен")]
    Unauthorized,
    #[error("Яндекс Музыка недоступна: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Яндекс Музыка ответила {status} на {path}")]
    Status { status: u16, path: String },
    #[error("непонятный ответ Яндекс Музыки на {0}")]
    Shape(String),
}

/// The signed-in account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Account {
    pub uid: String,
    pub login: Option<String>,
}

/// The music API with one token. Direct connection, never the app proxy.
pub struct Client {
    http: reqwest::Client,
    token: String,
}

impl Client {
    pub fn new(token: String) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { http, token })
    }

    /// Waits out 429s (the API rate-limits bursts) with growing pauses.
    async fn send(&self, path: &str, form: Option<String>) -> Result<Value, ApiError> {
        let mut pause = Duration::from_secs(2);
        for _ in 0..RATE_LIMIT_RETRIES {
            match self.send_once(path, form.clone()).await {
                Err(ApiError::Status { status: 429, .. }) => {
                    tracing::info!("yandex: rate-limited on {path}, waiting {pause:?}");
                    tokio::time::sleep(pause).await;
                    pause *= 2;
                }
                other => return other,
            }
        }
        self.send_once(path, form).await
    }

    async fn send_once(&self, path: &str, form: Option<String>) -> Result<Value, ApiError> {
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

    /// Who the token belongs to; [`ApiError::Unauthorized`] for a dead token.
    pub async fn account(&self) -> Result<Account, ApiError> {
        let status = self.get("/account/status").await?;
        let account = &status["account"];
        Ok(Account {
            uid: id_string(&account["uid"]).ok_or(ApiError::Unauthorized)?,
            login: str_field(account, "login"),
        })
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

/// The whole library; `progress(stage, detail)` reports each step
/// (`account`, `likes`, `playlists`, `albums`).
pub async fn export(
    client: &Client,
    progress: &(dyn Fn(&str, Option<String>) + Sync),
) -> Result<Library, ApiError> {
    export_parts(client, Parts::default(), progress).await
}

/// [`export`] limited to `parts`.
pub async fn export_parts(
    client: &Client,
    parts: Parts,
    progress: &(dyn Fn(&str, Option<String>) + Sync),
) -> Result<Library, ApiError> {
    progress("account", None);
    let Account { uid, login } = client.account().await?;
    let mut library = Library {
        login,
        ..Library::default()
    };

    if parts.likes {
        progress("likes", None);
        let likes = client
            .get(&format!(
                "/users/{uid}/likes/tracks?if-modified-since-revision=0"
            ))
            .await?;
        let liked = parse_likes(&likes);
        let ids: Vec<String> = liked.iter().map(|(id, _)| id.clone()).collect();
        let mut details: HashMap<String, Track> = client
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
    }

    if parts.playlists {
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
    }

    if parts.albums || parts.artists {
        progress("albums", None);
    }
    if parts.albums {
        match client
            .get(&format!("/users/{uid}/likes/albums?rich=true"))
            .await
        {
            Ok(v) => library.albums = parse_albums(&v),
            Err(ApiError::Unauthorized) => return Err(ApiError::Unauthorized),
            Err(e) => tracing::warn!("yandex: liked albums: {e}"),
        }
    }
    if parts.artists {
        match client
            .get(&format!("/users/{uid}/likes/artists?with-timestamps=false"))
            .await
        {
            Ok(v) => library.artists = parse_artists(&v),
            Err(ApiError::Unauthorized) => return Err(ApiError::Unauthorized),
            Err(e) => tracing::warn!("yandex: liked artists: {e}"),
        }
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

// ---- the token ------------------------------------------------------------

/// The page that hands out the token for a Yandex session on `host`
/// (`oauth.yandex.ru`, `oauth.yandex.uz`, ...).
pub fn authorize_url(host: &str) -> String {
    format!("https://{host}/authorize?response_type=token&client_id={CLIENT_ID}")
}

/// Trades the Yandex session in a Netscape cookies.txt for a token.
pub async fn token_from_cookies(text: &str) -> Option<String> {
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

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// The token after `access_token=` in a URL or an HTML page.
pub fn token_in_text(text: &str) -> Option<String> {
    let start = text.find("access_token=")? + "access_token=".len();
    let token: String = text[start..]
        .chars()
        .take_while(|&c| is_token_char(c))
        .collect();
    (!token.is_empty()).then_some(token)
}

/// `https://music.yandex.ru/#access_token=...&token_type=bearer` → the token.
pub fn token_from_url(url: &Url) -> Option<String> {
    let fragment = url.fragment()?;
    url::form_urlencoded::parse(fragment.as_bytes())
        .find(|(k, _)| k == "access_token")
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty())
}

/// What a user pastes: the bare token, or the redirect URL that carries it.
pub fn token_from_input(input: &str) -> Option<String> {
    let input = input.trim();
    if input.contains("access_token=") {
        return token_in_text(input);
    }
    (!input.is_empty() && input.chars().all(is_token_char)).then(|| input.to_owned())
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
    fn pasted_token_or_redirect() {
        assert_eq!(
            token_from_input(" https://music.yandex.ru/#access_token=y0_AbC-1&token_type=bearer\n")
                .as_deref(),
            Some("y0_AbC-1")
        );
        assert_eq!(
            token_from_input("y0_AbC-1.x").as_deref(),
            Some("y0_AbC-1.x")
        );
        assert_eq!(token_from_input("not a token"), None);
        assert_eq!(token_from_input("  "), None);
        assert_eq!(token_from_input("https://music.yandex.ru/"), None);
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
        assert_eq!(t.full_title(), "Song (Remix)");
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
