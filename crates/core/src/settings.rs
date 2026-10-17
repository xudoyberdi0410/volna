//! Persistent app settings owned by Rust (as opposed to the UI's localStorage).
//!
//! Lives in `<app config dir>/settings.json`: the network setting and where
//! yt-dlp takes YouTube cookies from. Missing fields fall back to defaults
//! (`serde(default)`), so older and newer files both load.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use volna_net::ProxyConfig;
#[cfg(feature = "ytdlp")]
use volna_streams::Cookies;

pub const FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// How the app reaches the internet; applied on startup.
    pub network: ProxyConfig,
    /// Cookies for yt-dlp (helps against YouTube's bot check); applied on startup.
    #[cfg(feature = "ytdlp")]
    pub ytdlp_cookies: Cookies,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("{path} is not valid settings JSON: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
}

/// Reads and writes [`Settings`] at one path.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    /// `config_dir` is the app config directory; it is created on first save.
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join(FILE_NAME),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Missing file means defaults; a broken file is an error so the caller
    /// can report it instead of silently overwriting the user's link.
    pub fn load(&self) -> Result<Settings, SettingsError> {
        let bytes = match fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(source) => {
                return Err(SettingsError::Read {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        serde_json::from_slice(&bytes).map_err(|source| SettingsError::Parse {
            path: self.path.clone(),
            source,
        })
    }

    /// Writes atomically (temp file + rename). The file can hold a vless UUID
    /// or proxy password, so on Unix it is readable by the owner only.
    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        let err = |source| SettingsError::Write {
            path: self.path.clone(),
            source,
        };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(err)?;
        }
        let json = serde_json::to_vec_pretty(settings).expect("settings serialise");
        let tmp = self.path.with_extension("json.tmp");
        {
            let mut file = open_private(&tmp).map_err(err)?;
            file.write_all(&json).map_err(err)?;
            file.sync_all().map_err(err)?;
        }
        fs::rename(&tmp, &self.path).map_err(err)
    }
}

fn open_private(path: &Path) -> io::Result<fs::File> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("volna-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_file_gives_defaults() {
        let store = SettingsStore::new(&temp_dir("missing"));
        assert_eq!(store.load().unwrap(), Settings::default());
    }

    #[test]
    fn round_trips_every_mode() {
        let dir = temp_dir("roundtrip");
        let store = SettingsStore::new(&dir.join("nested"));
        for network in [
            ProxyConfig::Direct,
            ProxyConfig::Url("socks5h://u:p@127.0.0.1:1080".into()),
            ProxyConfig::Vless("vless://id@example.com:443?security=reality&pbk=x".into()),
        ] {
            #[cfg_attr(not(feature = "ytdlp"), allow(unused_mut, clippy::needless_update))]
            let mut settings = Settings {
                network,
                ..Settings::default()
            };
            #[cfg(feature = "ytdlp")]
            {
                settings.ytdlp_cookies = Cookies::File("/home/u/cookies.txt".into());
            }
            store.save(&settings).unwrap();
            assert_eq!(store.load().unwrap(), settings);
        }
        assert!(!store.path().with_extension("json.tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_format_is_stable() {
        let dir = temp_dir("format");
        fs::create_dir_all(&dir).unwrap();
        let store = SettingsStore::new(&dir);
        fs::write(
            store.path(),
            r#"{"network":{"mode":"url","value":"http://127.0.0.1:8080"},"future":1}"#,
        )
        .unwrap();
        assert_eq!(
            store.load().unwrap().network,
            ProxyConfig::Url("http://127.0.0.1:8080".into())
        );
        // Files from before the cookie setting load with no cookies.
        #[cfg(feature = "ytdlp")]
        assert_eq!(store.load().unwrap().ytdlp_cookies, Cookies::None);
        fs::write(store.path(), "{}").unwrap();
        assert_eq!(store.load().unwrap().network, ProxyConfig::Direct);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn broken_file_is_an_error() {
        let dir = temp_dir("broken");
        fs::create_dir_all(&dir).unwrap();
        let store = SettingsStore::new(&dir);
        fs::write(store.path(), "{not json").unwrap();
        assert!(matches!(store.load(), Err(SettingsError::Parse { .. })));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("private");
        let store = SettingsStore::new(&dir);
        store.save(&Settings::default()).unwrap();
        let mode = fs::metadata(store.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        fs::remove_dir_all(dir).unwrap();
    }
}
