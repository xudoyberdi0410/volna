// js/lyrics-proxy.js
// Routes the lyrics providers' requests (made by the am-lyrics component and
// the Genius/LRCLIB helpers) through `volna://api/proxy/`, so they take the
// app's network route like everything else and are not subject to CORS.
// Must match ALLOWED_HOSTS in src-tauri/src/api_emu/proxy.rs.

import { volnaUrl } from './volna-url.js';

export const LYRICS_PROXY_HOSTS = Object.freeze([
    'lrclib.net',
    'lrc.red',
    'lyrics-api.binimum.org',
    'lyricsplus.binimum.org',
    'lyricsplus.prjktla.workers.dev',
    'lyricsplus-seven.vercel.app',
    'lyrics-plus-backend.vercel.app',
    'unison.boidu.dev',
    'fetch-genius.samidy.workers.dev',
    'api.genius.com',
    'translate.googleapis.com',
]);

/** The proxied URL for an allowlisted https GET target, else null. */
export function proxiedLyricsUrl(raw, options = undefined) {
    let url;
    try {
        url = new URL(String(raw));
    } catch {
        return null;
    }
    if (url.protocol !== 'https:' || url.port || url.username || url.password) return null;
    if (!LYRICS_PROXY_HOSTS.includes(url.hostname)) return null;
    return volnaUrl('api', '/proxy/', { u: url.href }, options);
}

/** Wraps `win.fetch` once; other requests pass through untouched. */
export function installLyricsFetchProxy(win = globalThis.window) {
    if (!win?.fetch || win.__volnaLyricsProxyInstalled) return;
    const original = win.fetch.bind(win);
    win.fetch = (input, init) => {
        const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input?.url;
        const method = (init?.method || (typeof input === 'object' && input?.method) || 'GET').toUpperCase();
        const target = method === 'GET' && raw ? proxiedLyricsUrl(raw) : null;
        if (target) {
            return original(target, {
                signal: init?.signal ?? (typeof input === 'object' ? input?.signal : undefined),
            });
        }
        return original(input, init);
    };
    win.__volnaLyricsProxyInstalled = true;
}
