//! PO tokens ("proof of origin") for googlevideo (`pot=`), the BgUtils way:
//! Rust fetches a BotGuard challenge (`Create`) and trades the VM's snapshot
//! for an integrity token (`GenerateIT`); the [`JsRuntime`] runs the VM and
//! mints tokens. One integrity token serves many mints for ~12 h.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::Value;

use super::js::{
    self, JsRequest, JsRuntime, MintRequest, MintResponse, SnapshotRequest, SnapshotResponse,
};
use crate::error::{Result, StreamError};

/// The web player's WAA request key.
const REQUEST_KEY: &str = "O43z0dpjhgX20SCx4KAo";
const API_KEY: &str = "AIzaSyDyT5W0Jh49F30Pqqtyfdf7pDLFKLJoAnw";
const CREATE_URL: &str = "https://www.youtube.com/api/jnn/v1/Create";
const GENERATE_IT_URL: &str = "https://www.youtube.com/api/jnn/v1/GenerateIT";
/// Refresh at least this long before the integrity token runs out.
const REFRESH_MARGIN: Duration = Duration::from_secs(10 * 60);

/// A BotGuard challenge (`Create`'s answer).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Challenge {
    pub interpreter_js: Option<String>,
    pub interpreter_url: Option<String>,
    pub program: String,
    pub global_name: String,
}

/// A live minter: the runtime's BotGuard session and its integrity token.
#[derive(Debug, Clone)]
struct Minter {
    session: String,
    integrity_token: String,
    refresh_at: Instant,
}

/// Minters per exit (the integrity token is obtained from, and may be bound
/// to, the IP that asked for it).
#[derive(Default)]
pub(crate) struct PotProvider {
    minters: tokio::sync::Mutex<HashMap<Option<String>, Minter>>,
    sessions: AtomicU64,
}

impl PotProvider {
    /// A PO token bound to `binding` (video id, visitor data or data sync id).
    pub async fn mint(
        &self,
        http: &reqwest::Client,
        user_agent: &str,
        proxy: Option<&str>,
        rt: &dyn JsRuntime,
        binding: &str,
    ) -> Result<String> {
        let key = proxy.map(str::to_owned);
        // Held across the attestation, so concurrent resolves share one.
        let mut minters = self.minters.lock().await;
        for attempt in 0..2 {
            let minter = match minters.get(&key) {
                Some(m) if Instant::now() < m.refresh_at => m.clone(),
                _ => {
                    let m = self.attest(http, user_agent, rt).await?;
                    minters.insert(key.clone(), m.clone());
                    m
                }
            };
            let req = JsRequest::BotguardMint(MintRequest {
                session: minter.session.clone(),
                integrity_token: minter.integrity_token.clone(),
                content_bindings: vec![binding.to_owned()],
            });
            match js::request::<MintResponse>(rt, &req).await {
                Ok(r) => {
                    return r
                        .po_tokens
                        .into_iter()
                        .next()
                        .ok_or_else(|| StreamError::JsRuntime("mint returned no token".into()));
                }
                // The runtime lost the VM (WebView reload, Deno restart): attest again.
                Err(e) if e.is("unknown_session") && attempt == 0 => {
                    tracing::debug!("BotGuard session gone, attesting again");
                    minters.remove(&key);
                }
                Err(e) => {
                    minters.remove(&key);
                    return Err(StreamError::JsRuntime(e.to_string()));
                }
            }
        }
        Err(StreamError::JsRuntime("BotGuard session lost twice".into()))
    }

    async fn attest(
        &self,
        http: &reqwest::Client,
        user_agent: &str,
        rt: &dyn JsRuntime,
    ) -> Result<Minter> {
        let started = Instant::now();
        let raw: Value = post(
            http,
            user_agent,
            CREATE_URL,
            &serde_json::json!([REQUEST_KEY]),
        )
        .await?;
        let challenge = parse_challenge(&raw)?;
        let interpreter_js = match (challenge.interpreter_js, challenge.interpreter_url) {
            (Some(js), _) => js,
            (None, Some(url)) => {
                let url = if url.starts_with("//") {
                    format!("https:{url}")
                } else {
                    url
                };
                http.get(&url)
                    .timeout(Duration::from_secs(30))
                    .header(reqwest::header::USER_AGENT, user_agent)
                    .send()
                    .await?
                    .error_for_status()?
                    .text()
                    .await?
            }
            (None, None) => {
                return Err(StreamError::ResolveFailed(
                    "BotGuard challenge without an interpreter".into(),
                ));
            }
        };
        let create_ms = started.elapsed().as_millis() as u64;
        let session = format!("bg{}", self.sessions.fetch_add(1, Ordering::Relaxed) + 1);
        let snap: SnapshotResponse = js::request(
            rt,
            &JsRequest::BotguardSnapshot(SnapshotRequest {
                session: session.clone(),
                interpreter_js,
                program: challenge.program,
                global_name: challenge.global_name,
            }),
        )
        .await
        .map_err(|e| StreamError::JsRuntime(e.to_string()))?;
        let snapshot_ms = started.elapsed().as_millis() as u64 - create_ms;

        let it: Value = post(
            http,
            user_agent,
            GENERATE_IT_URL,
            &serde_json::json!([REQUEST_KEY, snap.botguard_response]),
        )
        .await?;
        let (integrity_token, ttl) = parse_integrity(&it)?;
        let refresh_at = Instant::now()
            + ttl
                .saturating_sub(REFRESH_MARGIN)
                .max(Duration::from_secs(60));
        tracing::debug!(
            ttl_s = ttl.as_secs(),
            create_ms,
            snapshot_ms,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "BotGuard attestation done"
        );
        Ok(Minter {
            session,
            integrity_token,
            refresh_at,
        })
    }
}

async fn post(http: &reqwest::Client, user_agent: &str, url: &str, body: &Value) -> Result<Value> {
    let resp = http
        .post(url)
        .timeout(Duration::from_secs(30))
        .header(reqwest::header::CONTENT_TYPE, "application/json+protobuf")
        .header(reqwest::header::USER_AGENT, user_agent)
        .header("x-goog-api-key", API_KEY)
        .header("x-user-agent", "grpc-web-javascript/0.1")
        .body(body.to_string())
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(StreamError::ResolveFailed(format!(
            "BotGuard {url} answered HTTP {status}"
        )));
    }
    Ok(resp.json().await?)
}

/// `Create`'s answer: `[[messageId, [script], [url], hash, program, globalName, ...]]`,
/// or `[_, "<scrambled base64>"]` whose bytes are shifted by 97.
pub(crate) fn parse_challenge(raw: &Value) -> Result<Challenge> {
    let bad = |what: &str| StreamError::ResolveFailed(format!("BotGuard challenge: {what}"));
    let arr = raw.as_array().ok_or_else(|| bad("not an array"))?;
    let data: Value = match (arr.first(), arr.get(1)) {
        (_, Some(Value::String(scrambled))) => {
            let bytes = decode_base64(scrambled).ok_or_else(|| bad("bad base64"))?;
            let text: Vec<u8> = bytes.iter().map(|b| b.wrapping_add(97)).collect();
            serde_json::from_slice(&text).map_err(|_| bad("bad scrambled JSON"))?
        }
        (Some(first @ Value::Array(_)), _) => first.clone(),
        _ => return Err(bad("unknown shape")),
    };
    let first_string = |v: Option<&Value>| {
        v.and_then(Value::as_array)
            .and_then(|a| a.iter().find_map(|x| x.as_str().filter(|s| !s.is_empty())))
            .map(str::to_owned)
    };
    Ok(Challenge {
        interpreter_js: first_string(data.get(1)),
        interpreter_url: first_string(data.get(2)),
        program: data
            .get(4)
            .and_then(Value::as_str)
            .ok_or_else(|| bad("no program"))?
            .to_owned(),
        global_name: data
            .get(5)
            .and_then(Value::as_str)
            .ok_or_else(|| bad("no global name"))?
            .to_owned(),
    })
}

/// `GenerateIT`'s answer: `[integrityToken, estimatedTtlSecs, mintRefreshThreshold, fallback]`.
fn parse_integrity(raw: &Value) -> Result<(String, Duration)> {
    let token = raw
        .get(0)
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            StreamError::ResolveFailed("BotGuard attestation rejected (no integrity token)".into())
        })?;
    let ttl = raw.get(1).and_then(Value::as_u64).unwrap_or(3600);
    Ok((token.to_owned(), Duration::from_secs(ttl)))
}

fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let std: String = text
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            '.' => '=',
            c => c,
        })
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(std.as_bytes())
        .or_else(|_| {
            base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(std.trim_end_matches('=').as_bytes())
        })
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_plain_and_scrambled_challenges() {
        let plain = json!([[
            "id",
            [null, "SCRIPT"],
            [null, "//u.js"],
            "hash",
            "PROG",
            "glob"
        ]]);
        let c = parse_challenge(&plain).unwrap();
        assert_eq!(c.interpreter_js.as_deref(), Some("SCRIPT"));
        assert_eq!(c.interpreter_url.as_deref(), Some("//u.js"));
        assert_eq!(
            (c.program.as_str(), c.global_name.as_str()),
            ("PROG", "glob")
        );

        let inner = json!(["id", ["S"], [], "h", "P", "g"]).to_string();
        let shifted: Vec<u8> = inner.bytes().map(|b| b.wrapping_sub(97)).collect();
        let b64 = base64::engine::general_purpose::URL_SAFE.encode(shifted);
        let c = parse_challenge(&json!([null, b64])).unwrap();
        assert_eq!(c.interpreter_js.as_deref(), Some("S"));
        assert_eq!(c.global_name, "g");

        assert!(parse_challenge(&json!({})).is_err());
    }

    #[test]
    fn integrity_token_must_be_present() {
        let (t, ttl) = parse_integrity(&json!(["tok", 43200, 100, null])).unwrap();
        assert_eq!((t.as_str(), ttl.as_secs()), ("tok", 43200));
        assert!(parse_integrity(&json!([null, 43200])).is_err());
    }
}
