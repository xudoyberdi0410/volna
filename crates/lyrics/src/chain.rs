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

use std::time::Instant;

use futures_util::future::join_all;
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

    /// The chain as the old UI ran it: stops as soon as it has something
    /// good enough.
    pub async fn first(&self, http: &Client, q: &TrackQuery) -> Fetched {
        let mut acc = Acc::default();
        acc.add(self.run(http, q, Provider::LrcRed).await);
        if !acc.got.is_empty() {
            return acc.done();
        }

        let word = |c: &[Lyrics]| c.iter().any(|l| l.synced == Synced::Word);
        let timed = |c: &[Lyrics]| c.iter().any(|l| l.synced != Synced::None);

        if q.has_title_artist() {
            acc.add(self.run(http, q, Provider::Bini).await);
            for p in [Provider::Unison, Provider::LyricsPlus] {
                if !word(&acc.got) {
                    acc.add(self.run(http, q, p).await);
                }
            }
            if !timed(&acc.got) {
                acc.add(self.run(http, q, Provider::Lrclib).await);
            }
            if acc.got.is_empty() {
                acc.add(self.run(http, q, Provider::Genius).await);
            }
        }
        if acc.got.is_empty() && self.ytm.is_some() {
            acc.add(self.run(http, q, Provider::Ytm).await);
        }
        acc.done()
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
