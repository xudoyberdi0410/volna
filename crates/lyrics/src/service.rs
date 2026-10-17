//! Chain + cache: what the daemon (and later `volna-core`) calls.

use std::future::Future;
use std::path::Path;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use volna_ytm::YtmClient;

use crate::cache::{Cache, Entry};
use crate::chain::Fetcher;
use crate::error::Result;
use crate::model::{Lyrics, Synced, TrackQuery};
use crate::rank::source_key;

/// One available source, for a switcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub source: String,
    pub synced: Synced,
    pub lines: usize,
}

/// Which video and how to describe it to the providers.
#[derive(Debug, Clone, Copy)]
pub struct Key<'a> {
    pub video_id: &'a str,
    pub isrc: Option<&'a str>,
}

pub struct LyricsService {
    fetcher: Fetcher,
    cache: Cache,
}

impl LyricsService {
    pub fn new(cache: Cache, ytm: Option<YtmClient>) -> Self {
        Self {
            fetcher: Fetcher::new(ytm),
            cache,
        }
    }

    /// Opens (or creates) the cache at `path`; expired rows are dropped.
    pub fn open(path: &Path, ytm: Option<YtmClient>) -> Result<Self> {
        let cache = Cache::open(path)?;
        if let Err(e) = cache.prune() {
            tracing::warn!("lyrics cache prune: {e}");
        }
        Ok(Self::new(cache, ytm))
    }

    pub fn fetcher(&self) -> &Fetcher {
        &self.fetcher
    }

    /// The cached entry, or a chain run. `query` describes the track; it is
    /// only called on a miss. With `complete`, every provider is asked.
    async fn entry<F, Fut>(
        &self,
        http: &Client,
        key: Key<'_>,
        complete: bool,
        query: F,
    ) -> Result<Entry>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<TrackQuery>>,
    {
        match self.cache.get(key.video_id, key.isrc) {
            Ok(Some(e)) if e.complete || !complete => return Ok(e),
            Ok(_) => {}
            Err(e) => tracing::warn!("lyrics cache read: {e}"),
        }
        let mut q = query().await?;
        q.video_id.get_or_insert_with(|| key.video_id.to_owned());
        if q.isrc.is_none() {
            q.isrc = key.isrc.map(str::to_owned);
        }
        let fetched = if complete {
            self.fetcher.all(http, &q).await
        } else {
            self.fetcher.first(http, &q).await
        };
        let entry = Entry {
            sources: fetched.sources.clone(),
            complete,
        };
        if fetched.all_failed() {
            tracing::info!(
                video_id = key.video_id,
                "every lyrics provider failed; not caching"
            );
        } else if let Err(e) = self.cache.put(key.video_id, q.isrc.as_deref(), &entry) {
            tracing::warn!("lyrics cache write: {e}");
        }
        Ok(entry)
    }

    /// The best lyrics, `None` when no provider has the track.
    pub async fn get<F, Fut>(&self, http: &Client, key: Key<'_>, query: F) -> Result<Option<Lyrics>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<TrackQuery>>,
    {
        Ok(self
            .entry(http, key, false, query)
            .await?
            .sources
            .into_iter()
            .next())
    }

    /// Every source that has the track, best first (asks all providers once).
    pub async fn sources<F, Fut>(
        &self,
        http: &Client,
        key: Key<'_>,
        query: F,
    ) -> Result<Vec<SourceInfo>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<TrackQuery>>,
    {
        let e = self.entry(http, key, true, query).await?;
        Ok(e.sources
            .iter()
            .map(|l| SourceInfo {
                source: l.source.clone(),
                synced: l.synced,
                lines: l.lines.len(),
            })
            .collect())
    }

    /// Lyrics from one source (a label from [`Self::sources`]).
    pub async fn get_source<F, Fut>(
        &self,
        http: &Client,
        key: Key<'_>,
        source: &str,
        query: F,
    ) -> Result<Option<Lyrics>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<TrackQuery>>,
    {
        let want = source_key(source);
        let pick = |e: Entry| {
            e.sources
                .into_iter()
                .find(|l| source_key(&l.source) == want)
        };
        // The short chain may already hold it.
        if let Ok(Some(e)) = self.cache.get(key.video_id, key.isrc)
            && let Some(l) = pick(e)
        {
            return Ok(Some(l));
        }
        Ok(pick(self.entry(http, key, true, query).await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serves_from_cache_without_query() {
        let svc = LyricsService::new(Cache::in_memory().unwrap(), None);
        let key = Key {
            video_id: "vid",
            isrc: None,
        };
        let cached = Entry {
            sources: vec![
                Lyrics::from_plain("Genius", "a", None),
                Lyrics::from_plain("YouTube Music", "b", None),
            ],
            complete: true,
        };
        svc.cache.put("vid", None, &cached).unwrap();
        let http = Client::new();
        let never = || async { panic!("query must not run on a hit") };
        let best = svc.get(&http, key, never).await.unwrap().unwrap();
        assert_eq!(best.source, "Genius");
        let srcs = svc.sources(&http, key, never).await.unwrap();
        assert_eq!(srcs.len(), 2);
        let yt = svc
            .get_source(&http, key, "youtube music", never)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(yt.lines[0].text, "b");
    }
}
