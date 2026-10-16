// js/volna-media.js
// Small pure helpers for the YouTube Music backend: stream descriptors,
// image URL handling and link parsing. Kept free of DOM/storage access so
// they run under `bun run test:node`.

import { volnaUrl } from './volna-url.js';

/**
 * Prefix of the loopback audio server (`http://127.0.0.1:<port>/<token>/stream`),
 * injected by the Rust side before any page script runs. `<audio>` cannot
 * play `volna://` on WebKitGTK, so media elements must use this.
 */
export function streamBase(win = globalThis.window) {
    return win?.__VOLNA__?.streamBase || null;
}

/** Quality tokens the UI still passes around; only LOW changes anything. */
export function isLowQuality(quality) {
    return String(quality || '').toUpperCase() === 'LOW';
}

/**
 * The descriptor `Player` consumes for native (`<audio src>`) playback.
 * See docs/hifi-api-contract.md §4.2 and docs/streams.md.
 */
export function buildStreamDescriptor(id, quality, { base = streamBase() } = {}) {
    const low = isLowQuality(quality);
    const encoded = encodeURIComponent(String(id));
    const url = base
        ? `${base}/${encoded}${low ? '?q=low' : ''}`
        : volnaUrl('stream', `/${encoded}`, low ? { q: 'low' } : undefined);
    return {
        url,
        playbackType: 'direct',
        provider: 'youtube',
        quality: low ? 'LOW' : 'HIGH',
        qualityDisplay: low ? 'Opus ~70k' : 'Opus ~160k',
        codec: 'opus',
        bitrateKbps: low ? 70 : 160,
        lossless: false,
        mimeType: 'audio/webm',
        mediaMimeType: 'audio/webm',
        rgInfo: null,
        waveform: null,
    };
}

/**
 * True for values that already are a usable image URL (any scheme, data/blob,
 * or a bundled asset path), as opposed to a TIDAL image UUID.
 */
export function isDirectImage(value) {
    return typeof value === 'string' && /^(?:[a-z][a-z0-9+.-]*:|assets\/|images\/|\/)/i.test(value);
}

const VOLNA_IMG_RE = /^(?:volna:\/\/img\/|http:\/\/volna\.localhost\/img\/)/i;

/** True for covers served by the Rust image cache. */
export function isVolnaImage(value) {
    return typeof value === 'string' && VOLNA_IMG_RE.test(value);
}

/**
 * Rewrites the `s` (edge in px) parameter of a volna image URL so lists can
 * load small thumbnails. Other URLs come back unchanged.
 */
export function resizeImage(value, size) {
    if (!isVolnaImage(value) || !size) return value;
    const px = parseInt(size, 10);
    if (!Number.isFinite(px) || px <= 0) return value;
    const [head, query = ''] = value.split('?');
    const params = new URLSearchParams(query);
    params.set('s', String(px));
    return `${head}?${params.toString()}`;
}

/** Radio queues open as mixes; YouTube Music's own RDCLAK lists are playlists. */
export function isRadioId(id) {
    return typeof id === 'string' && id.startsWith('RD') && !id.startsWith('RDCLAK');
}

/**
 * Maps a pasted YouTube / YouTube Music link to an in-app route, or null.
 *   watch?v=<id>          -> /track/<id>
 *   playlist?list=<id>    -> /playlist/<id>  (or /mix/<id> for radios)
 *   browse/MPREb_…        -> /album/MPREb_…
 *   channel/UC…           -> /artist/UC…
 *   youtu.be/<id>         -> /track/<id>
 */
export function routeForYoutubeLink(text) {
    let url;
    try {
        url = new URL(String(text).trim());
    } catch {
        return null;
    }
    const host = url.hostname.replace(/^(?:www|m)\./, '');
    const id = /^[A-Za-z0-9_-]+$/;
    if (host === 'youtu.be') {
        const v = url.pathname.slice(1).split('/')[0];
        return id.test(v) ? `/track/${v}` : null;
    }
    if (host !== 'youtube.com' && host !== 'music.youtube.com') return null;

    const [first, second] = url.pathname.split('/').filter(Boolean);
    const v = url.searchParams.get('v');
    const list = url.searchParams.get('list');
    if (first === 'watch' && v && id.test(v)) return `/track/${v}`;
    if (first === 'playlist' && list && id.test(list)) {
        const clean = list.replace(/^VL/, '');
        if (clean.startsWith('OLAK5uy_')) return `/album/${clean}`;
        return isRadioId(clean) ? `/mix/${clean}` : `/playlist/${clean}`;
    }
    if (first === 'browse' && second && id.test(second)) {
        if (second.startsWith('MPRE')) return `/album/${second}`;
        if (second.startsWith('VL')) return `/playlist/${second.slice(2)}`;
        if (second.startsWith('UC')) return `/artist/${second}`;
    }
    if (first === 'channel' && second && /^UC[A-Za-z0-9_-]+$/.test(second)) return `/artist/${second}`;
    return null;
}
