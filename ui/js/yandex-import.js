// Library → «Импорт из Яндекс Музыки». Rust (src-tauri/src/yandex.rs) signs in
// (with the browser's Yandex session, else a login window) and exports the
// Yandex library; here every track is looked up on YouTube Music
// and the matches go into the local library: likes, playlists, albums, artists.
// Running it again adds only what is new.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { db } from './db.js';
import { showNotification } from './notifications.js';
import { findBestMatch, stringSimilarity, toLatin } from './playlist-importer.js';

const PARALLEL = 4;
// Candidates further off than this in length are another recording.
const DURATION_SLACK_S = 15;
const IMPORTED_FROM = 'yandex';

const STAGES = {
    browser: 'Беру вход в Яндекс из браузера…',
    login: 'Войдите в Яндекс в открывшемся окне…',
    account: 'Читаю аккаунт…',
    likes: 'Забираю «Мне нравится»…',
    playlists: 'Забираю плейлисты…',
    albums: 'Забираю альбомы и артистов…',
};

function errorText(e) {
    return typeof e === 'string' ? e : (e?.message ?? String(e));
}

function fullTitle(t) {
    return t.version ? `${t.title} (${t.version})` : t.title;
}

// The progress box of the CSV import, with our own heading.
function progressBox() {
    const box = document.getElementById('csv-import-progress');
    const heading = box.querySelector('.progress-header h4');
    const warning = box.querySelector('.progress-warning');
    const saved = [heading.textContent, warning.textContent];
    heading.textContent = 'Импорт из Яндекс Музыки';
    warning.textContent = 'Каждый трек ищется на YouTube Music, на большую библиотеку уйдёт несколько минут.';
    const line = box.querySelector('.current-track');
    const sub = box.querySelector('.current-artist');
    const fill = document.getElementById('csv-progress-fill');
    const current = document.getElementById('csv-progress-current');
    const total = document.getElementById('csv-progress-total');
    const counter = box.querySelector('.progress-text');
    box.style.display = 'block';
    counter.style.visibility = 'hidden';
    fill.style.width = '0%';
    return {
        say(text, detail = '') {
            line.textContent = text;
            sub.textContent = detail;
        },
        count(done, of) {
            counter.style.visibility = 'visible';
            current.textContent = String(done);
            total.textContent = String(of);
            fill.style.width = `${of ? Math.min((done / of) * 100, 100) : 0}%`;
        },
        close() {
            box.style.display = 'none';
            counter.style.visibility = 'visible';
            [heading.textContent, warning.textContent] = saved;
        },
    };
}

// Stricter than the CSV import: Yandex gives exact titles, and a loose match
// puts a stranger's song into the likes.
const MATCH = { strictArtistMatch: true, minTitleScore: 0.7, minArtistScore: 0.7 };
// "(Official Music Video)", "[HD]", "| Music video" and the like.
const VIDEO_NOISE =
    /\s*[([](?:official|music video|audio|lyrics?|mood video|hd|mv|клип|премьера)[^)\]]*[)\]]|\s+\|.*$/gi;

// A video as the song it carries. Uploads by other channels put the song into
// the title ("DVN - Можно я с тобой"), sometimes the other way round, so both
// readings are offered; each candidate points back at the video.
function videoCandidates(video) {
    const title = video.title.replace(VIDEO_NOISE, '').trim();
    const out = [{ ...video, title, source: video }];
    const parts = title.match(/^(.+?)\s+[-–—]\s+(.+)$/);
    if (parts) {
        const [, left, right] = parts;
        out.push({ ...video, title: right, artists: [{ name: left }, ...(video.artists ?? [])], source: video });
        out.push({ ...video, title: left, artists: [{ name: right }, ...(video.artists ?? [])], source: video });
    }
    return out;
}

// Yandex sometimes has a Japanese release title («フェイデッド» for "Faded"),
// which no spelling rule maps back. For such titles the same artist and a
// near-identical length are taken as the song.
const NO_LATIN_OR_CYRILLIC = /^[^\p{Script=Latin}\p{Script=Cyrillic}]+$/u;
const EXACT_LENGTH_S = 2;

function sameArtistAndLength(items, t) {
    if (!t.durationMs || !NO_LATIN_OR_CYRILLIC.test(t.title)) return null;
    return (
        items.find(
            (item) =>
                item.duration &&
                Math.abs(item.duration - t.durationMs / 1000) <= EXACT_LENGTH_S &&
                (item.artists ?? []).some((a) => t.artists.some((name) => stringSimilarity(a.name, name) >= 0.9))
        ) ?? null
    );
}

export async function matchTrack(api, t) {
    const artist = t.artists.join(', ');
    const durationOk = (item) =>
        !t.durationMs || !item.duration || Math.abs(item.duration - t.durationMs / 1000) <= DURATION_SLACK_S;
    const plain = `${t.artists[0] ?? ''} ${t.title}`.trim();
    // YouTube Music may only know the release under its Latin spelling.
    const queries = toLatin(plain) === plain ? [plain] : [plain, toLatin(plain)];
    const titles = t.version ? [fullTitle(t), t.title] : [t.title];

    for (const query of t.version ? [`${t.artists[0] ?? ''} ${fullTitle(t)}`.trim(), ...queries] : queries) {
        const items = ((await api.searchTracks(query))?.items ?? []).filter(durationOk);
        for (const title of titles) {
            const found = findBestMatch(items, artist, t.album, MATCH, title);
            if (found) return found;
        }
        const byLength = sameArtistAndLength(items, t);
        if (byLength) return byLength;
    }
    // Songs that exist on YouTube only as videos. Uploads differ in length
    // (intros, compilations), so only close lengths count here.
    for (const query of queries) {
        const videos = (await api.searchTracks(query, { videos: true }))?.items ?? [];
        const candidates = videos.filter(durationOk).flatMap(videoCandidates);
        for (const title of titles) {
            const found = findBestMatch(candidates, artist, null, MATCH, title);
            if (found) return found.source;
        }
    }
    return null;
}

// Runs `fn` over `items` with a few requests in flight; `null` where it threw.
async function pool(items, fn, onDone) {
    const out = new Array(items.length).fill(null);
    let next = 0;
    let done = 0;
    const worker = async () => {
        while (next < items.length) {
            const i = next++;
            try {
                out[i] = await fn(items[i]);
            } catch (e) {
                console.warn('[yandex] lookup failed', items[i], e);
            }
            onDone(++done, items[i]);
        }
    };
    await Promise.all(Array.from({ length: Math.min(PARALLEL, items.length) }, worker));
    return out;
}

async function savePlaylists(playlists, matched) {
    const existing = (await db.getPlaylists(false)).filter((p) => p.importedFrom === IMPORTED_FROM);
    let created = 0;
    for (const p of playlists) {
        const tracks = p.tracks.map((t) => matched.get(t.id)).filter(Boolean);
        if (tracks.length === 0) continue;
        const same = existing.find((e) => e.name === p.title);
        if (same) {
            // Tracks found this time go where Yandex has them, not at the end;
            // tracks added by hand in Volna stay after the imported ones.
            await db.addTracksToPlaylist(same.id, tracks);
            const order = new Map(tracks.map((t, i) => [t.id, i]).reverse());
            const current = (await db.getPlaylist(same.id))?.tracks ?? [];
            const rank = (t) => order.get(t.id) ?? tracks.length;
            await db.updatePlaylistTracks(
                same.id,
                current
                    .map((t, i) => ({ t, i }))
                    .sort((a, b) => rank(a.t) - rank(b.t) || a.i - b.i)
                    .map(({ t }) => t)
            );
            continue;
        }
        const playlist = await db.createPlaylist(p.title, tracks, '', p.description || '');
        await db.updatePlaylist({ ...playlist, importedFrom: IMPORTED_FROM });
        created++;
    }
    return created;
}

async function bestByName(search, query, name, minScore) {
    const items = (await search(query))?.items ?? [];
    let best = null;
    let bestScore = minScore;
    for (const item of items) {
        const score = stringSimilarity(item.title ?? item.name, name);
        if (score >= bestScore) {
            best = item;
            bestScore = score;
        }
    }
    return best;
}

async function runImport({ api, ui, showMissing }) {
    const box = progressBox();
    const unlisten = await listen('yandex-progress', ({ payload }) => {
        box.say(STAGES[payload.stage] ?? 'Читаю Яндекс Музыку…', payload.detail ?? '');
    });
    try {
        box.say('Вхожу в Яндекс…');
        let library;
        try {
            library = await invoke('yandex_export');
        } finally {
            unlisten();
        }

        // Each Yandex track once, however many playlists it is in.
        const byId = new Map();
        for (const t of library.likedTracks) byId.set(t.id, t);
        for (const p of library.playlists) for (const t of p.tracks) if (!byId.has(t.id)) byId.set(t.id, t);
        const unique = [...byId.values()];
        const totalSteps = unique.length + library.albums.length + library.artists.length;

        box.say('Ищу треки на YouTube Music…');
        box.count(0, totalSteps);
        const found = await pool(
            unique,
            (t) => matchTrack(api, t),
            (done, t) => {
                box.count(done, totalSteps);
                box.say('Ищу треки на YouTube Music…', `${t.artists.join(', ')} — ${fullTitle(t)}`);
            }
        );
        const matched = new Map();
        const missing = [];
        unique.forEach((t, i) => {
            if (found[i]) matched.set(t.id, found[i]);
            else missing.push({ title: fullTitle(t), artist: t.artists.join(', ') });
        });

        const albums = await pool(
            library.albums,
            (a) => bestByName((q) => api.searchAlbums(q), `${a.artists[0] ?? ''} ${a.title}`, a.title, 0.6),
            (done, a) => {
                box.count(unique.length + done, totalSteps);
                box.say('Ищу альбомы…', a.title);
            }
        );
        const artists = await pool(
            library.artists,
            (name) => bestByName((q) => api.searchArtists(q), name, name, 0.85),
            (done, name) => {
                box.count(unique.length + library.albums.length + done, totalSteps);
                box.say('Ищу артистов…', name);
            }
        );

        box.say('Сохраняю в библиотеку…');
        // Yandex lists likes newest first; addedAt keeps that order here.
        const now = Date.now();
        const likes = library.likedTracks
            .map((t, i) => {
                const track = matched.get(t.id);
                if (!track) return null;
                const at = Date.parse(t.likedAt ?? '');
                return { ...track, addedAt: Number.isFinite(at) ? at : now - i };
            })
            .filter(Boolean);
        const likedCount = await db.addFavorites('track', likes);
        const albumCount = await db.addFavorites('album', albums.filter(Boolean));
        const artistCount = await db.addFavorites('artist', artists.filter(Boolean));
        const playlistCount = await savePlaylists(library.playlists, matched);
        window.dispatchEvent(new CustomEvent('playlist-tracks-changed'));

        box.close();
        const parts = [
            `${likedCount} новых лайков`,
            `${playlistCount} плейлистов`,
            `${albumCount} альбомов`,
            `${artistCount} артистов`,
        ];
        showNotification(
            `Импорт из Яндекс Музыки: ${parts.join(', ')}. Найдено ${matched.size} из ${unique.length} треков.`
        );
        if (ui?.renderLibraryPage && window.location.pathname.startsWith('/library')) await ui.renderLibraryPage();
        if (missing.length > 0) showMissing(missing, 'Яндекс Музыка');
    } catch (e) {
        box.close();
        console.error('[yandex] import failed', e);
        showNotification(`Импорт из Яндекс Музыки не удался: ${errorText(e)}`, { type: 'error' });
    }
}

export function initYandexImport(deps) {
    let running = false;
    document.getElementById('library-import-yandex-card')?.addEventListener('click', async () => {
        if (running) return;
        running = true;
        try {
            await runImport(deps);
        } finally {
            running = false;
        }
    });
}
