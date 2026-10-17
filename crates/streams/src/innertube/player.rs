//! The player's `base.js`: fetched once per player id (memory, optionally a
//! disk cache), its `signatureTimestamp`, and the `n`/signature challenges
//! solved through the [`JsRuntime`].
//!
//! Like yt-dlp, a signature is solved once per length: the challenge is the
//! string of chars `0..len`, and the answer is the permutation to apply to
//! any signature of that length. `n` answers are cached per challenge.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::js::{self, JsError, JsRequest, JsRuntime, SolveRequest, SolveResponse};
use crate::error::{Result, StreamError};

/// Players kept in memory (each is ~3 MB of JavaScript).
const MAX_PLAYERS: usize = 2;
/// `n` answers kept per player.
const MAX_N: usize = 512;

/// One player version.
pub(crate) struct Player {
    pub id: String,
    pub code: Arc<str>,
    pub sts: Option<u32>,
    /// Whether the runtime was given `code` (it keeps the preprocessed player).
    sent: AtomicBool,
    sig_specs: Mutex<HashMap<usize, Vec<usize>>>,
    n: Mutex<HashMap<String, String>>,
}

impl Player {
    pub fn new(id: String, code: String) -> Self {
        let sts = find_sts(&code);
        Self {
            id,
            code: code.into(),
            sts,
            sent: AtomicBool::new(false),
            sig_specs: Mutex::default(),
            n: Mutex::default(),
        }
    }

    /// Solves `n` (a URL parameter) and `sig` (an encrypted signature).
    pub async fn solve(
        &self,
        rt: &dyn JsRuntime,
        n: Option<&str>,
        sig: Option<&str>,
    ) -> Result<(Option<String>, Option<String>)> {
        let cached_n = n.and_then(|n| lock(&self.n).get(n).cloned());
        let spec = sig.and_then(|s| lock(&self.sig_specs).get(&s.chars().count()).cloned());

        let want_n = n.filter(|_| cached_n.is_none());
        let want_sig_len = sig.filter(|_| spec.is_none()).map(|s| s.chars().count());
        let (mut n_out, mut spec) = (cached_n, spec);
        if want_n.is_some() || want_sig_len.is_some() {
            let probe = want_sig_len.map(sig_probe);
            let answer = self
                .request(rt, want_n.map(str::to_owned), probe.clone())
                .await?;
            if let Some(n) = want_n {
                let solved = answer
                    .n
                    .get(n)
                    .cloned()
                    .ok_or_else(|| js_failure("no answer for the n challenge"))?;
                let mut cache = lock(&self.n);
                if cache.len() >= MAX_N {
                    cache.clear();
                }
                cache.insert(n.to_owned(), solved.clone());
                n_out = Some(solved);
            }
            if let (Some(len), Some(probe)) = (want_sig_len, probe) {
                let solved = answer
                    .sig
                    .get(&probe)
                    .ok_or_else(|| js_failure("no answer for the signature challenge"))?;
                let perm: Vec<usize> = solved.chars().map(|c| c as usize).collect();
                if perm.iter().any(|&i| i >= len) {
                    return Err(js_failure("signature permutation out of range"));
                }
                lock(&self.sig_specs).insert(len, perm.clone());
                spec = Some(perm);
            }
        }
        let sig_out = match (sig, spec) {
            (Some(s), Some(spec)) => Some(apply_spec(s, &spec)),
            _ => None,
        };
        Ok((n_out, sig_out))
    }

    async fn request(
        &self,
        rt: &dyn JsRuntime,
        n: Option<String>,
        sig: Option<String>,
    ) -> Result<SolveResponse> {
        let mut req = SolveRequest {
            player_id: self.id.clone(),
            player: None,
            n: n.into_iter().collect(),
            sig: sig.into_iter().collect(),
        };
        if !self.sent.load(Ordering::Acquire) {
            req.player = Some(self.code.clone());
        }
        let first = js::request::<SolveResponse>(rt, &JsRequest::Solve(req.clone())).await;
        let answer = match first {
            // The runtime lost its context (WebView reload, Deno restart).
            Err(e) if e.is("need_player") && req.player.is_none() => {
                tracing::debug!(player = %self.id, "runtime lost the player, sending it again");
                req.player = Some(self.code.clone());
                js::request::<SolveResponse>(rt, &JsRequest::Solve(req.clone())).await
            }
            other => other,
        }
        .map_err(js_error)?;
        if req.player.is_some() {
            self.sent.store(true, Ordering::Release);
        }
        Ok(answer)
    }
}

/// Players by id, with an optional directory that keeps `base.js` across runs.
#[derive(Default)]
pub(crate) struct PlayerCache {
    players: Mutex<Vec<Arc<Player>>>,
    dir: Option<PathBuf>,
}

impl PlayerCache {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            players: Mutex::default(),
            dir,
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<Player>> {
        lock(&self.players).iter().find(|p| p.id == id).cloned()
    }

    /// The player from disk, if the cache directory has it.
    pub fn load(&self, id: &str) -> Option<Arc<Player>> {
        let path = self.dir.as_ref()?.join(format!("player-{id}.js"));
        let code = std::fs::read_to_string(path).ok()?;
        Some(self.insert(id, code, false))
    }

    pub fn insert(&self, id: &str, code: String, persist: bool) -> Arc<Player> {
        if persist && let Some(dir) = &self.dir {
            let path = dir.join(format!("player-{id}.js"));
            if let Err(e) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&path, &code))
            {
                tracing::debug!("cannot cache player {id}: {e}");
            }
        }
        let player = Arc::new(Player::new(id.to_owned(), code));
        let mut players = lock(&self.players);
        players.retain(|p| p.id != id);
        if players.len() >= MAX_PLAYERS {
            players.remove(0);
        }
        players.push(player.clone());
        player
    }
}

/// The player URL to fetch: always the `main` variant, as yt-dlp forces it
/// (the solver is tested against it).
pub(crate) fn player_js_url(id: &str) -> String {
    format!("https://www.youtube.com/s/player/{id}/player_ias.vflset/en_US/base.js")
}

/// `signatureTimestamp:12345` or `sts:12345` in the player.
pub(crate) fn find_sts(code: &str) -> Option<u32> {
    for key in ["signatureTimestamp", "sts"] {
        let mut rest = code;
        while let Some(pos) = rest.find(key) {
            rest = &rest[pos + key.len()..];
            let after = rest.trim_start();
            let Some(after) = after.strip_prefix(':') else {
                continue;
            };
            let after = after.trim_start();
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            if digits.len() == 5 {
                return digits.parse().ok();
            }
        }
    }
    None
}

/// The challenge whose answer is the permutation for signatures of `len` chars.
fn sig_probe(len: usize) -> String {
    (0..len as u32).filter_map(char::from_u32).collect()
}

fn apply_spec(sig: &str, spec: &[usize]) -> String {
    let chars: Vec<char> = sig.chars().collect();
    spec.iter().filter_map(|&i| chars.get(i)).collect()
}

fn js_error(e: JsError) -> StreamError {
    StreamError::JsRuntime(e.to_string())
}

fn js_failure(message: &str) -> StreamError {
    StreamError::JsRuntime(message.to_owned())
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::innertube::js::JsCallback;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn finds_sts() {
        assert_eq!(find_sts("a={signatureTimestamp:20739,b:1}"), Some(20739));
        assert_eq!(find_sts("x.sts : 19999;"), Some(19999));
        assert_eq!(find_sts("sts:1234;signatureTimestamp:12"), None);
    }

    #[test]
    fn signature_permutation() {
        assert_eq!(sig_probe(3), "\u{0}\u{1}\u{2}");
        assert_eq!(apply_spec("abc", &[2, 0, 1]), "cab");
    }

    /// A runtime that reverses every challenge and forgets its player once.
    fn fake(
        calls: Arc<AtomicUsize>,
        forget_once: Arc<AtomicBool>,
    ) -> JsCallback<
        impl Fn(String) -> futures_util::future::Ready<std::result::Result<String, JsError>>
        + Send
        + Sync,
    > {
        JsCallback(move |raw: String| {
            calls.fetch_add(1, Ordering::SeqCst);
            let JsRequest::Solve(req) = serde_json::from_str(&raw).unwrap() else {
                panic!("unexpected request {raw}");
            };
            if req.player.is_none() && forget_once.swap(false, Ordering::SeqCst) {
                return futures_util::future::ready(Ok(
                    r#"{"error":{"code":"need_player","message":"gone"}}"#.to_owned(),
                ));
            }
            let rev = |s: &String| s.chars().rev().collect::<String>();
            let resp = SolveResponse {
                n: req.n.iter().map(|c| (c.clone(), rev(c))).collect(),
                sig: req.sig.iter().map(|c| (c.clone(), rev(c))).collect(),
            };
            futures_util::future::ready(Ok(serde_json::json!({ "ok": resp }).to_string()))
        })
    }

    #[tokio::test]
    async fn solves_and_caches() {
        let calls = Arc::new(AtomicUsize::new(0));
        let forget = Arc::new(AtomicBool::new(false));
        let rt = fake(calls.clone(), forget.clone());
        let p = Player::new("0229cd64".into(), "var sts:20739".into());
        assert_eq!(p.sts, Some(20739));

        let (n, s) = p.solve(&rt, Some("abc"), Some("wxyz")).await.unwrap();
        assert_eq!(n.as_deref(), Some("cba"));
        assert_eq!(s.as_deref(), Some("zyxw"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Same n, another signature of the same length: all cached.
        let (n, s) = p.solve(&rt, Some("abc"), Some("1234")).await.unwrap();
        assert_eq!((n.as_deref(), s.as_deref()), (Some("cba"), Some("4321")));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // The runtime lost the player: it is sent again.
        forget.store(true, Ordering::SeqCst);
        let (n, _) = p.solve(&rt, Some("new"), None).await.unwrap();
        assert_eq!(n.as_deref(), Some("wen"));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}
