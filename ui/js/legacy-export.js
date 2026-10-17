// js/legacy-export.js
// Hands the webview library to the daemon before the Flutter window replaces
// this UI (docs/flutter-ui-plan.md 3a): every IndexedDB store of db.js plus
// the scrobblers' localStorage keys, as one JSON dump.
//
// - Once on startup: if the daemon says the library was never imported
//   (`legacy_status`), the dump goes to `legacy_import`.
// - Settings → "Export library to file": the same dump, written by the daemon
//   to ~/Downloads (`legacy_export_file`); Flutter settings can import it.

import { invoke } from '@tauri-apps/api/core';
import { db } from './db.js';

export const DUMP_FORMAT = 'volna-legacy-library';

const STORES = [
    'favorites_tracks',
    'favorites_albums',
    'favorites_artists',
    'favorites_playlists',
    'favorites_mixes',
    'favorites_videos',
    'history_tracks',
    'user_playlists',
    'user_folders',
    'pinned_items',
    'settings',
];

// Last.fm (incl. the session from lastfm.js), Libre.fm, ListenBrainz, Maloja.
const LOCAL_STORAGE_KEY = /^(lastfm|librefm|listenbrainz|maloja)-/;

// Only what JSON can carry (the `settings` store also holds a FileSystemHandle).
function plain(value) {
    try {
        const json = JSON.stringify(value);
        return json === undefined ? null : JSON.parse(json);
    } catch {
        return null;
    }
}

function readStore(idb, name) {
    return new Promise((resolve, reject) => {
        const tx = idb.transaction(name, 'readonly');
        const store = tx.objectStore(name);
        if (store.keyPath !== null) {
            const req = store.getAll();
            req.onsuccess = () => resolve(req.result.map(plain).filter((v) => v !== null));
            req.onerror = () => reject(req.error);
            return;
        }
        // Out-of-line keys (`settings`): keep them as an object.
        const out = {};
        const req = store.openCursor();
        req.onsuccess = () => {
            const cursor = req.result;
            if (!cursor) return resolve(out);
            if (typeof cursor.key === 'string' && cursor.key !== 'local_folder_handle') {
                const v = plain(cursor.value);
                if (v !== null) out[cursor.key] = v;
            }
            cursor.continue();
        };
        req.onerror = () => reject(req.error);
    });
}

/** Everything the daemon needs from this UI's storage. */
export async function dumpLegacyLibrary() {
    const idb = await db.open();
    const stores = {};
    for (const name of STORES) {
        if (!idb.objectStoreNames.contains(name)) continue;
        try {
            stores[name] = await readStore(idb, name);
        } catch (e) {
            console.warn(`[legacy] cannot read ${name}`, e);
        }
    }
    const localStorageKeys = {};
    try {
        for (let i = 0; i < localStorage.length; i++) {
            const key = localStorage.key(i);
            if (key && LOCAL_STORAGE_KEY.test(key)) localStorageKeys[key] = localStorage.getItem(key);
        }
    } catch (e) {
        console.warn('[legacy] cannot read localStorage', e);
    }
    return {
        format: DUMP_FORMAT,
        version: 1,
        exportedAt: Date.now(),
        stores,
        localStorage: localStorageKeys,
    };
}

/** Imports into the daemon once; later runs only ask `legacy_status`. */
export async function importLegacyLibraryOnce() {
    try {
        const status = await invoke('legacy_status');
        if (status?.imported) return null;
        const report = await invoke('legacy_import', { dump: await dumpLegacyLibrary() });
        console.log('[legacy] library handed to the daemon', report);
        return report;
    } catch (e) {
        // An older daemon without these commands: try again next start.
        console.warn('[legacy] import skipped', e);
        return null;
    }
}

/** Writes the dump to a file; resolves to its path. */
export async function exportLegacyLibraryToFile() {
    return invoke('legacy_export_file', { dump: await dumpLegacyLibrary() });
}
