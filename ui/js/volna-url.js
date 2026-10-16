// js/volna-url.js
// Base URLs for the `volna://` custom protocol served by the Rust side.
//
// Linux/macOS webviews (WebKitGTK/WKWebView) load custom schemes as-is:
//   volna://api/search/?s=x      volna://stream/<videoId>
// Windows (WebView2) cannot, and Tauri maps them to a fixed localhost host:
//   http://volna.localhost/api/search/?s=x
// The Rust handler (src-tauri/src/protocol.rs) accepts both shapes.

/** Routes served by the protocol handler. */
export const VOLNA_ROUTES = Object.freeze(['api', 'stream', 'img']);

/** True when running inside WebView2 (Windows). */
export function isWindowsWebview(userAgent = globalThis.navigator?.userAgent ?? '') {
    return /\bWindows\b/i.test(userAgent);
}

/**
 * Returns the base URL for a route, without a trailing slash.
 * @param {'api' | 'stream' | 'img'} route
 * @param {{ windows?: boolean }} [options] override platform detection (tests)
 */
export function volnaBase(route, { windows = isWindowsWebview() } = {}) {
    if (!VOLNA_ROUTES.includes(route)) {
        throw new Error(`Unknown volna route: ${route}`);
    }
    return windows ? `http://volna.localhost/${route}` : `volna://${route}`;
}

/**
 * Builds a full URL: `volnaUrl('api', '/search/', { s: 'daft punk' })`.
 * @param {'api' | 'stream' | 'img'} route
 * @param {string} [path]
 * @param {Record<string, string | number | boolean | null | undefined>} [query]
 * @param {{ windows?: boolean }} [options]
 */
export function volnaUrl(route, path = '/', query = undefined, options = undefined) {
    const normalizedPath = path.startsWith('/') ? path : `/${path}`;
    let url = volnaBase(route, options) + normalizedPath;
    if (query) {
        const params = new URLSearchParams();
        for (const [key, value] of Object.entries(query)) {
            if (value !== undefined && value !== null) params.append(key, String(value));
        }
        const qs = params.toString();
        if (qs) url += `?${qs}`;
    }
    return url;
}
