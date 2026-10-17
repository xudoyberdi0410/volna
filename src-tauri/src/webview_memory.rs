//! Keeps the WebKitGTK webview lean.
//!
//! By default WebKitGTK uses the "web browser" cache model: hundreds of MB of
//! memory cache with dead (off-screen) resources kept around, plus a disk cache
//! that can grow to a gigabyte. Volna is one page whose bundle ships in the
//! binary and whose covers already sit in the Rust image cache
//! (`volna_streams::ImageCache`), so both WebKit caches only duplicate data.
//! The "document viewer" model turns the disk cache off and drops decoded
//! resources as soon as nothing on screen uses them.

use tauri::{AppHandle, Manager, Runtime};

/// Applies the settings to the `main` window. Call once from `setup`.
pub fn apply<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Err(e) = window.with_webview(configure) {
        tracing::warn!("cannot configure the webview caches: {e}");
    }
}

#[cfg(target_os = "linux")]
fn configure(webview: tauri::webview::PlatformWebview) {
    use webkit2gtk::{
        CacheModel, WebContextExt, WebViewExt, WebsiteDataManagerExtManual, WebsiteDataTypes,
    };

    let Some(context) = webview.inner().context() else {
        return;
    };
    context.set_cache_model(CacheModel::DocumentViewer);
    // The disk cache is off from now on; drop what earlier versions stored
    // (Google Fonts, dev-server modules, covers: hundreds of MB).
    if let Some(manager) = context.website_data_manager() {
        manager.clear(
            WebsiteDataTypes::DISK_CACHE,
            webkit2gtk::glib::TimeSpan::from_seconds(0),
            None::<&webkit2gtk::gio::Cancellable>,
            |res| {
                if let Err(e) = res {
                    tracing::warn!("cannot clear the WebKit disk cache: {e}");
                }
            },
        );
    }
}

/// WebView2 manages its own caches; nothing to tune there yet.
#[cfg(not(target_os = "linux"))]
fn configure(_webview: tauri::webview::PlatformWebview) {}
