//! The YouTube session the in-process stream resolver uses (feature
//! `innertube`): cookies from the app's Google sign-in WebView, kept in
//! `<data dir>/youtube-cookies.txt` (Netscape format, mode 0600) and swapped
//! into the running [`InnerTube`] backend without a restart.
//!
//! Commands: `youtube_set_cookies {cookies}` (empty string signs out) and
//! `youtube_auth_status`, both answering `{signedIn, account?}`.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use volna_streams::innertube::{CookieJar, InnerTube};

use crate::core::Core;

/// File name under the data directory.
pub const COOKIES_FILE: &str = "youtube-cookies.txt";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub signed_in: bool,
    /// The account's display name, when YouTube Music tells it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

pub fn cookies_path(data_dir: &Path) -> PathBuf {
    data_dir.join(COOKIES_FILE)
}

/// The saved session, if there is one.
pub fn load_cookies(data_dir: &Path) -> Option<CookieJar> {
    let text = std::fs::read_to_string(cookies_path(data_dir)).ok()?;
    let jar = CookieJar::from_netscape(&text);
    (!jar.is_empty()).then_some(jar)
}

/// Saves `text` (a Netscape cookies.txt) readable by the owner only; an
/// empty `text` deletes the file.
pub fn save_cookies(data_dir: &Path, text: &str) -> std::io::Result<()> {
    let path = cookies_path(data_dir);
    if text.trim().is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(data_dir)?;
    let tmp = path.with_extension("txt.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&tmp)?;
    #[cfg(unix)]
    {
        // `mode` only applies when the file is created.
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(tmp, path)
}

fn backend(core: &Core) -> Result<&InnerTube, String> {
    core.app
        .streams
        .as_ref()
        .and_then(|s| s.backend().innertube())
        .ok_or_else(|| "unsupported: streams are not resolved in-process here".to_owned())
}

/// `youtube_set_cookies {cookies}`: saves and applies a new session.
pub async fn youtube_set_cookies(core: &Core, cookies: String) -> Result<AuthStatus, String> {
    let backend = backend(core)?;
    let jar = CookieJar::from_netscape(&cookies);
    if !cookies.trim().is_empty() && jar.is_empty() {
        return Err("invalid args `cookies`: no cookies in Netscape format".into());
    }
    save_cookies(&core.dirs.data, if jar.is_empty() { "" } else { &cookies })
        .map_err(|e| format!("cannot save the YouTube cookies: {e}"))?;
    backend.set_cookies(Some(jar));
    // URLs resolved with the old session are bound to it.
    if let Some(streams) = &core.app.streams {
        streams.clear_cache();
    }
    youtube_auth_status(core).await
}

/// `youtube_auth_status`: whether a signed-in session is set, and whose.
pub async fn youtube_auth_status(core: &Core) -> Result<AuthStatus, String> {
    let backend = backend(core)?;
    let signed_in = backend.cookies().is_some_and(|c| c.is_logged_in());
    let account = if signed_in {
        match backend
            .account_name(core.app.net.proxy_url().as_deref())
            .await
        {
            Ok(name) => name,
            Err(e) => {
                tracing::debug!("YouTube account name: {e}");
                None
            }
        }
    } else {
        None
    };
    Ok(AuthStatus { signed_in, account })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use volna_streams::innertube::{InnerTubeOptions, JsCallback, JsError};

    const SIGNED_OUT: &str = ".youtube.com\tTRUE\t/\tTRUE\t0\tVISITOR_INFO1_LIVE\tabc\n.google.com\tTRUE\t/\tTRUE\t0\tNID\tx\n";

    #[test]
    fn saves_privately_and_signs_out() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_cookies(dir.path()).is_none());
        save_cookies(dir.path(), SIGNED_OUT).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(cookies_path(dir.path()))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(load_cookies(dir.path()).unwrap().len(), 2);
        save_cookies(dir.path(), "").unwrap();
        assert!(!cookies_path(dir.path()).exists());
        save_cookies(dir.path(), "").unwrap(); // nothing to delete is fine
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commands_swap_the_session_without_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let js = JsCallback(|_req: String| async { Err::<String, _>(JsError::runtime("unused")) });
        let innertube = InnerTube::new(Arc::new(js), InnerTubeOptions::default());
        let config = crate::CoreConfig {
            dirs: crate::Dirs::under(dir.path().to_owned(), None),
            proxy: false,
            audio_server: false,
            backend: Some(crate::StreamBackend::InnerTube(innertube.clone())),
            codec: volna_streams::Codec::default(),
        };
        let output = crate::player::platform::PlatformOutput::new();
        let core = Core::start(config, Some(output)).await.unwrap();

        let status = core
            .call("youtube_auth_status", &serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(status, serde_json::json!({"signedIn": false}));

        let status = core
            .call(
                "youtube_set_cookies",
                &serde_json::json!({"cookies": SIGNED_OUT}),
            )
            .await
            .unwrap();
        assert_eq!(status, serde_json::json!({"signedIn": false}));
        assert_eq!(
            innertube.cookies().unwrap().len(),
            2,
            "applied to the running backend"
        );
        assert!(load_cookies(&core.dirs.data).is_some(), "persisted");

        let err = core
            .call(
                "youtube_set_cookies",
                &serde_json::json!({"cookies": "not cookies"}),
            )
            .await
            .unwrap_err();
        assert!(err.contains("Netscape"), "{err}");

        core.call("youtube_set_cookies", &serde_json::json!({"cookies": ""}))
            .await
            .unwrap();
        assert!(innertube.cookies().is_none());
        assert!(load_cookies(&core.dirs.data).is_none());
    }
}
