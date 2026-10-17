//! `volna://` custom protocol: the webview's only door to the outside world.
//! Images and audio are `volna_core::protocol`; the daemon adds the hifi-api
//! emulation (`volna://api/`) the old webview UI reads.

use http::{Request, Response};

pub use volna_core::protocol::{Route, Target, json_error, with_cors};

use crate::api_emu::ApiEmu;
use crate::state::AppState;

/// Dispatches a protocol request.
pub async fn handle(state: &AppState, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    handle_with(&state.core, &state.api, request).await
}

/// [`handle`] over the core state and the emulator held separately.
pub async fn handle_with(
    core: &volna_core::AppState,
    api: &ApiEmu,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    if let Some(res) = volna_core::protocol::handle_core(core, &request).await {
        return res;
    }
    match Target::parse(request.uri()) {
        Some(target) => api.handle(&target).await,
        None => json_error(http::StatusCode::NOT_FOUND, "unknown volna:// route"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use http::{StatusCode, header};

    /// App state with a throwaway cache dir, as `setup` would build it.
    pub(crate) fn test_state() -> AppState {
        let dir = std::env::temp_dir().join(format!("volna-test-cache-{}", std::process::id()));
        AppState::new(&dir).unwrap()
    }

    #[tokio::test]
    async fn api_errors_are_json_with_cors() {
        // Fails before any network I/O: the id is missing.
        let req = Request::get("volna://api/info/").body(Vec::new()).unwrap();
        let res = handle(&test_state(), req).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(res.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        let body: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(body["status"], 400);
    }

    #[tokio::test]
    async fn api_root_answers_without_network() {
        let req = Request::get("volna://api/").body(Vec::new()).unwrap();
        let res = handle(&test_state(), req).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(body["Repo"], "volna");
    }

    #[tokio::test]
    async fn unknown_route_answers_404() {
        let req = Request::get("volna://nope/").body(Vec::new()).unwrap();
        assert_eq!(
            handle(&test_state(), req).await.status(),
            StatusCode::NOT_FOUND
        );
    }
}
