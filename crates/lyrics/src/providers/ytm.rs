//! YouTube Music's own lyrics tab (plain text) — the last resort.

use volna_ytm::YtmClient;

use crate::error::Result;
use crate::model::{Lyrics, TrackQuery};

pub const LABEL: &str = "YouTube Music";

pub async fn fetch(ytm: &YtmClient, q: &TrackQuery) -> Result<Option<Lyrics>> {
    let Some(id) = q.video_id.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let Some(found) = ytm.lyrics(id).await? else {
        return Ok(None);
    };
    let l = Lyrics::from_plain(LABEL, &found.text, None);
    Ok((!l.is_empty()).then_some(l))
}
