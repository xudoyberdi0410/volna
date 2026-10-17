//! What the user set up, per service. Stored in the scrobble database (one
//! JSON row), so the crate carries its own state into `volna-core`.
//!
//! Everything is off by default: the old webview UI still scrobbles from
//! its localStorage, and the daemon starts only once the user (or the legacy
//! import) turns a service on here.

use serde::{Deserialize, Serialize};

use crate::audioscrobbler::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    LastFm,
    LibreFm,
    ListenBrainz,
    Maloja,
}

impl Service {
    pub const ALL: [Service; 4] = [
        Service::LastFm,
        Service::LibreFm,
        Service::ListenBrainz,
        Service::Maloja,
    ];

    /// The name in commands and in the queue table.
    pub fn as_str(self) -> &'static str {
        match self {
            Service::LastFm => "lastfm",
            Service::LibreFm => "librefm",
            Service::ListenBrainz => "listenbrainz",
            Service::Maloja => "maloja",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.as_str() == s)
    }
}

/// Last.fm's rule: played at least this share of the track (or 4 minutes).
pub const DEFAULT_PERCENTAGE: u8 = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// 1..=100, for every service (as `lastfm-scrobble-percentage` was).
    pub percentage: u8,
    pub lastfm: LastFm,
    pub librefm: LibreFm,
    pub listenbrainz: ListenBrainz,
    pub maloja: Maloja,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            percentage: DEFAULT_PERCENTAGE,
            lastfm: LastFm::default(),
            librefm: LibreFm::default(),
            listenbrainz: ListenBrainz::default(),
            maloja: Maloja::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LastFm {
    pub enabled: bool,
    pub love_on_like: bool,
    pub session: Option<Session>,
    /// Between `scrobble_login_start` and `scrobble_login_finish`.
    pub pending_token: Option<String>,
    pub use_custom_credentials: bool,
    pub custom_api_key: String,
    pub custom_api_secret: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LibreFm {
    pub enabled: bool,
    pub love_on_like: bool,
    pub session: Option<Session>,
    pub pending_token: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ListenBrainz {
    pub enabled: bool,
    pub love_on_like: bool,
    pub token: String,
    /// Own server; empty means api.listenbrainz.org.
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Maloja {
    pub enabled: bool,
    pub token: String,
    pub url: String,
}

impl Config {
    pub fn enabled(&self, s: Service) -> bool {
        match s {
            Service::LastFm => self.lastfm.enabled,
            Service::LibreFm => self.librefm.enabled,
            Service::ListenBrainz => self.listenbrainz.enabled,
            Service::Maloja => self.maloja.enabled,
        }
    }

    /// Enabled and has what it needs to send.
    pub fn active(&self, s: Service) -> bool {
        self.enabled(s)
            && match s {
                Service::LastFm => self.lastfm.session.is_some(),
                Service::LibreFm => self.librefm.session.is_some(),
                Service::ListenBrainz => !self.listenbrainz.token.trim().is_empty(),
                Service::Maloja => {
                    !self.maloja.token.trim().is_empty() && !self.maloja.url.trim().is_empty()
                }
            }
    }

    pub fn love_on_like(&self, s: Service) -> bool {
        match s {
            Service::LastFm => self.lastfm.love_on_like,
            Service::LibreFm => self.librefm.love_on_like,
            Service::ListenBrainz => self.listenbrainz.love_on_like,
            Service::Maloja => false,
        }
    }

    /// The scrobble share as a fraction, clamped like `setScrobblePercentage`.
    pub fn fraction(&self) -> f64 {
        f64::from(self.percentage.clamp(1, 100)) / 100.0
    }
}

/// `scrobble_set` arguments: only the given fields change. `token` is the
/// ListenBrainz / Maloja token; `url` their server.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Patch {
    pub enabled: Option<bool>,
    pub love_on_like: Option<bool>,
    pub token: Option<String>,
    pub url: Option<String>,
    pub percentage: Option<u32>,
    pub use_custom_credentials: Option<bool>,
    pub custom_api_key: Option<String>,
    pub custom_api_secret: Option<String>,
}

impl Config {
    /// Applies `p` to `service` (or only the shared `percentage` when `None`).
    /// Changing Last.fm credentials drops the session: it is bound to the key.
    pub fn apply(&mut self, service: Option<Service>, p: Patch) -> Result<(), String> {
        if let Some(pct) = p.percentage {
            self.percentage = pct.clamp(1, 100) as u8;
        }
        let Some(service) = service else {
            return Ok(());
        };
        let unsupported = |what: &str| Err(format!("{} has no {what}", service.as_str()));
        if service != Service::LastFm
            && (p.use_custom_credentials.is_some()
                || p.custom_api_key.is_some()
                || p.custom_api_secret.is_some())
        {
            return unsupported("custom API credentials");
        }
        match service {
            Service::LastFm => {
                let c = &mut self.lastfm;
                set(&mut c.enabled, p.enabled);
                set(&mut c.love_on_like, p.love_on_like);
                let before = (
                    c.use_custom_credentials,
                    c.custom_api_key.clone(),
                    c.custom_api_secret.clone(),
                );
                set(&mut c.use_custom_credentials, p.use_custom_credentials);
                set(
                    &mut c.custom_api_key,
                    p.custom_api_key.map(|s| s.trim().into()),
                );
                set(
                    &mut c.custom_api_secret,
                    p.custom_api_secret.map(|s| s.trim().into()),
                );
                if before
                    != (
                        c.use_custom_credentials,
                        c.custom_api_key.clone(),
                        c.custom_api_secret.clone(),
                    )
                {
                    c.session = None;
                    c.pending_token = None;
                }
                if p.token.is_some() || p.url.is_some() {
                    return unsupported("token or url (log in instead)");
                }
            }
            Service::LibreFm => {
                set(&mut self.librefm.enabled, p.enabled);
                set(&mut self.librefm.love_on_like, p.love_on_like);
                if p.token.is_some() || p.url.is_some() {
                    return unsupported("token or url (log in instead)");
                }
            }
            Service::ListenBrainz => {
                let c = &mut self.listenbrainz;
                set(&mut c.enabled, p.enabled);
                set(&mut c.love_on_like, p.love_on_like);
                set(&mut c.token, p.token.map(|s| s.trim().into()));
                set(&mut c.url, p.url.map(|s| s.trim().into()));
            }
            Service::Maloja => {
                let c = &mut self.maloja;
                set(&mut c.enabled, p.enabled);
                set(&mut c.token, p.token.map(|s| s.trim().into()));
                set(&mut c.url, p.url.map(|s| s.trim().into()));
                if p.love_on_like == Some(true) {
                    return unsupported("love");
                }
            }
        }
        Ok(())
    }
}

fn set<T>(slot: &mut T, v: Option<T>) {
    if let Some(v) = v {
        *slot = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_by_default() {
        let c = Config::default();
        assert!(Service::ALL.iter().all(|s| !c.active(*s)));
        assert_eq!(c.percentage, 50);
    }

    #[test]
    fn patch_and_active() {
        let mut c = Config::default();
        c.apply(
            Some(Service::Maloja),
            Patch {
                enabled: Some(true),
                token: Some(" k ".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!c.active(Service::Maloja), "needs a url too");
        c.apply(
            Some(Service::Maloja),
            Patch {
                url: Some("https://m".into()),
                percentage: Some(500),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(c.active(Service::Maloja));
        assert_eq!(c.maloja.token, "k");
        assert_eq!(c.percentage, 100);
        assert!(
            c.apply(
                Some(Service::Maloja),
                Patch {
                    love_on_like: Some(true),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn new_lastfm_key_drops_the_session() {
        let mut c = Config::default();
        c.lastfm.session = Some(Session {
            key: "s".into(),
            name: "u".into(),
        });
        c.apply(
            Some(Service::LastFm),
            Patch {
                love_on_like: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(c.lastfm.session.is_some());
        c.apply(
            Some(Service::LastFm),
            Patch {
                custom_api_key: Some("k".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(c.lastfm.session.is_none());
    }

    #[test]
    fn service_names() {
        for s in Service::ALL {
            assert_eq!(Service::parse(s.as_str()), Some(s));
            assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
        }
    }
}
