//! The provider chain of `AmLyrics.fetchLyrics`, plus YouTube Music as the
//! last resort:
//!
//! 1. lrc.red — a hit ends the search.
//! 2. BiniLyrics; Unison if nothing word-synced yet; lyricsplus likewise.
//! 3. LRCLIB if nothing is line-synced yet.
//! 4. Genius if nothing at all; then YouTube Music.
//!
//! Every provider runs under its own timeout; an error is logged and the
//! chain goes on.

use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::future::join_all;
use futures_util::stream::FuturesUnordered;
use reqwest::Client;
use volna_ytm::YtmClient;

use crate::error::{Error, Result};
use crate::model::{Lyrics, Synced, TrackQuery};
use crate::providers::{self, Provider};
use crate::rank::merge_and_sort;

/// What a chain run found.
#[derive(Debug, Clone, Default)]
pub struct Fetched {
    /// Best first.
    pub sources: Vec<Lyrics>,
    /// Providers asked, and how many of them failed (error or timeout).
    pub asked: usize,
    pub failed: usize,
}

impl Fetched {
    /// Every provider failed: probably offline, not "no lyrics".
    pub fn all_failed(&self) -> bool {
        self.sources.is_empty() && self.asked > 0 && self.failed == self.asked
    }
}

/// Accumulates provider runs.
#[derive(Default)]
struct Acc {
    got: Vec<Lyrics>,
    asked: usize,
    failed: usize,
}

impl Acc {
    fn add(&mut self, r: Option<Vec<Lyrics>>) {
        self.asked += 1;
        match r {
            Some(found) => self.got.extend(found),
            None => self.failed += 1,
        }
    }

    fn done(self) -> Fetched {
        Fetched {
            sources: merge_and_sort(self.got),
            asked: self.asked,
            failed: self.failed,
        }
    }
}

/// How long [`Fetcher::first`] waits for the providers the sequential chain
/// would consult before settling for whatever it already has.
pub const SOFT_DEADLINE: Duration = Duration::from_secs(4);

/// Provider results so far: `None` = failed.
type Done = HashMap<Provider, Option<Vec<Lyrics>>>;

/// Replays the sequential chain over the results in `done`. `None` while a
/// provider it needs is still running.
fn decide(q: &TrackQuery, with_ytm: bool, done: &Done) -> Option<Fetched> {
    let mut acc = Acc::default();
    let need = |acc: &mut Acc, p: Provider| -> Option<()> {
        acc.add(done.get(&p)?.clone());
        Some(())
    };
    need(&mut acc, Provider::LrcRed)?;
    if !acc.got.is_empty() {
        return Some(acc.done());
    }
    let word = |c: &[Lyrics]| c.iter().any(|l| l.synced == Synced::Word);
    let timed = |c: &[Lyrics]| c.iter().any(|l| l.synced != Synced::None);
    if q.has_title_artist() {
        need(&mut acc, Provider::Bini)?;
        for p in [Provider::Unison, Provider::LyricsPlus] {
            if !word(&acc.got) {
                need(&mut acc, p)?;
            }
        }
        if !timed(&acc.got) {
            need(&mut acc, Provider::Lrclib)?;
        }
        if acc.got.is_empty() {
            need(&mut acc, Provider::Genius)?;
        }
    }
    if acc.got.is_empty() && with_ytm {
        need(&mut acc, Provider::Ytm)?;
    }
    Some(acc.done())
}

/// Everything in `done` that found something, for the soft deadline.
fn found_so_far(done: &Done) -> Option<Fetched> {
    let mut acc = Acc::default();
    for r in done.values() {
        acc.add(r.clone());
    }
    (!acc.got.is_empty()).then(|| acc.done())
}

/// [`Fetcher::first`] over any provider runner (tests pass fakes).
async fn first_with<F, Fut>(
    q: &TrackQuery,
    with_ytm: bool,
    soft: Duration,
    run: F,
) -> Fetched
where
    F: Fn(Provider) -> Fut,
    Fut: Future<Output = Option<Vec<Lyrics>>>,
{
    let wanted = Provider::ALL.into_iter().filter(|&p| match p {
        Provider::LrcRed => true,
        Provider::Ytm => with_ytm,
        _ => q.has_title_artist(),
    });
    let mut running: FuturesUnordered<_> = wanted
        .map(|p| {
            let f = run(p);
            async move { (p, f.await) }
        })
        .collect();
    let mut done = Done::new();
    let deadline = tokio::time::sleep(soft);
    tokio::pin!(deadline);
    let mut late = false;
    loop {
        tokio::select! {
            next = running.next() => {
                let Some((p, r)) = next else { break };
                done.insert(p, r);
                if let Some(f) = decide(q, with_ytm, &done) {
                    return f;
                }
                if late && let Some(f) = found_so_far(&done) {
                    return f;
                }
            }
            () = &mut deadline, if !late => {
                late = true;
                if let Some(f) = found_so_far(&done) {
                    tracing::debug!(have = done.len(), "lyrics: soft deadline, settling");
                    return f;
                }
            }
        }
    }
    decide(q, with_ytm, &done).unwrap_or_default()
}

#[derive(Clone)]
pub struct Fetcher {
    ytm: Option<YtmClient>,
}

impl Fetcher {
    /// `ytm` enables the YouTube Music fallback.
    pub fn new(ytm: Option<YtmClient>) -> Self {
        Self { ytm }
    }

    /// One provider, without the timeout or error swallowing.
    pub async fn provider(
        &self,
        http: &Client,
        q: &TrackQuery,
        p: Provider,
    ) -> Result<Vec<Lyrics>> {
        let one = |r: Result<Option<Lyrics>>| r.map(|l| l.into_iter().collect());
        match p {
            Provider::LrcRed => one(providers::lrcred::fetch(http, q).await),
            Provider::Bini => one(providers::bini::fetch(http, q).await),
            Provider::Unison => one(providers::unison::fetch(http, q).await),
            Provider::LyricsPlus => providers::lyricsplus::fetch(http, q).await,
            Provider::Lrclib => one(providers::lrclib::fetch(http, q).await),
            Provider::Genius => one(providers::genius::fetch(http, q).await),
            Provider::Ytm => match &self.ytm {
                Some(ytm) => one(providers::ytm::fetch(ytm, q).await),
                None => Ok(Vec::new()),
            },
        }
    }

    /// One provider under its timeout. `None` when it failed (logged).
    pub async fn run(&self, http: &Client, q: &TrackQuery, p: Provider) -> Option<Vec<Lyrics>> {
        let t0 = Instant::now();
        let r = tokio::time::timeout(p.timeout(), self.provider(http, q, p))
            .await
            .unwrap_or(Err(Error::Timeout));
        let ms = t0.elapsed().as_millis();
        match r {
            Ok(found) => {
                tracing::debug!(
                    provider = p.name(),
                    ms,
                    found = found.len(),
                    "lyrics provider"
                );
                Some(found.into_iter().filter(|l| !l.is_empty()).collect())
            }
            Err(e) => {
                tracing::info!(provider = p.name(), ms, "lyrics provider failed: {e}");
                None
            }
        }
    }

    /// The chain as the old UI ran it (see the module doc), but with every
    /// provider it may need asked at once: the answer is the one the
    /// sequential chain would give, as soon as the providers it depends on
    /// are in. After [`SOFT_DEADLINE`], anything found is good enough.
    pub async fn first(&self, http: &Client, q: &TrackQuery) -> Fetched {
        first_with(q, self.ytm.is_some(), SOFT_DEADLINE, |p| self.run(http, q, p)).await
    }

    /// Every provider at once, for the source switcher.
    pub async fn all(&self, http: &Client, q: &TrackQuery) -> Fetched {
        let runs = Provider::ALL.map(|p| self.run(http, q, p));
        let mut acc = Acc::default();
        for r in join_all(runs).await {
            acc.add(r);
        }
        acc.done()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Line;

    fn q() -> TrackQuery {
        TrackQuery {
            title: "Song".into(),
            artist: "Artist".into(),
            ..Default::default()
        }
    }

    fn line(source: &str) -> Lyrics {
        let mut l = Line::plain("la");
        l.start_ms = 1000;
        l.end_ms = 2000;
        Lyrics::new(source, vec![l])
    }

    /// A fake provider: answers after `secs` (virtual time).
    async fn after(secs: u64, r: Option<Vec<Lyrics>>) -> Option<Vec<Lyrics>> {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        r
    }

    #[tokio::test(start_paused = true)]
    async fn providers_run_at_once_not_one_after_another() {
        // Sequentially this is 10 s (every provider takes 10 s); in parallel
        // the answer comes when the slowest needed one is in.
        let t0 = tokio::time::Instant::now();
        let got = first_with(&q(), true, Duration::from_secs(60), |p| {
            after(
                10,
                Some(if p == Provider::Lrclib {
                    vec![line("LRCLIB")]
                } else {
                    vec![]
                }),
            )
        })
        .await;
        assert_eq!(got.sources[0].source, "LRCLIB");
        assert_eq!(t0.elapsed(), Duration::from_secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn lrc_red_hit_does_not_wait_for_the_rest() {
        let t0 = tokio::time::Instant::now();
        let got = first_with(&q(), true, Duration::from_secs(60), |p| match p {
            Provider::LrcRed => after(1, Some(vec![line("lrc.red")])),
            _ => after(30, None),
        })
        .await;
        assert_eq!(got.sources.len(), 1);
        assert_eq!(got.sources[0].source, "lrc.red");
        assert_eq!(t0.elapsed(), Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn a_hanging_provider_does_not_hold_up_a_found_text() {
        // lyricsplus hangs; LRCLIB answered fast. The chain would wait for
        // lyricsplus (nothing word-synced yet); the soft deadline cuts it.
        let t0 = tokio::time::Instant::now();
        let got = first_with(&q(), true, Duration::from_secs(4), |p| match p {
            Provider::LyricsPlus => after(25, None),
            Provider::Lrclib => after(1, Some(vec![line("LRCLIB")])),
            _ => after(1, Some(vec![])),
        })
        .await;
        assert_eq!(got.sources[0].source, "LRCLIB");
        assert_eq!(t0.elapsed(), Duration::from_secs(4));
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_found_waits_for_every_needed_provider() {
        let got = first_with(&q(), true, Duration::from_secs(4), |p| match p {
            Provider::Ytm => after(6, Some(vec![Lyrics::from_plain("YouTube Music", "x", None)])),
            _ => after(2, Some(vec![])),
        })
        .await;
        assert_eq!(got.sources[0].source, "YouTube Music");
    }

    #[tokio::test(start_paused = true)]
    async fn all_failed_is_reported() {
        let got = first_with(&q(), true, Duration::from_secs(4), |_| after(1, None)).await;
        assert!(got.all_failed());
    }
}
