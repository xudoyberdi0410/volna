// js/playback-failures.js
// When a track cannot be played, say why and move on.
//
// `<audio>` only sees a bare HTTP error from the loopback audio server, so
// the reason (YouTube's bot check, a removed video, a missing yt-dlp) is
// asked from Rust (`stream_failure`). The player is not modified: errors are
// observed with a capture-phase listener on the document, like wave.js does.

import { invoke } from '@tauri-apps/api/core';
import { showNotification } from './notifications.js';

/** Stop skipping after this many tracks in a row failed to play. */
export const MAX_AUTO_SKIPS = 3;
/** The same explanation is not repeated more often than this. */
const NOTICE_COOLDOWN_MS = 15_000;
/** Give the player a moment to finish its own error handling. */
const SKIP_DELAY_MS = 1200;

const NOTICES = {
    bot_check:
        'YouTube не отдаёт звук через это подключение («подтвердите, что вы не бот»). Смените подключение в Настройки → Сеть',
    unavailable: 'Трек недоступен на YouTube, пропускаю',
    no_yt_dlp: 'Не найден yt-dlp, воспроизведение невозможно',
    other: 'Не удалось воспроизвести трек, пропускаю',
};

/** The message shown for a failure kind (`volna_streams::FailureKind`). */
export function failureNotice(kind) {
    return NOTICES[kind] ?? NOTICES.other;
}

/** Kinds where trying the next track cannot help. */
export function isFatal(kind) {
    return kind === 'no_yt_dlp';
}

/**
 * Counts consecutive failures. `fail(id)` says whether to skip to the next
 * track; a track that starts playing resets the count.
 */
export class SkipGuard {
    constructor(max = MAX_AUTO_SKIPS) {
        this.max = max;
        this.streak = 0;
        this.lastId = null;
    }

    /** Returns true if the player should move on to the next track. */
    fail(id) {
        // The same track erroring twice (e.g. src reset) is one failure.
        if (id !== this.lastId) {
            this.streak += 1;
            this.lastId = id;
        }
        return this.streak < this.max;
    }

    ok() {
        this.streak = 0;
        this.lastId = null;
    }
}

let beforeAutoSkip = null;

/** Lets «Моя волна» fill the queue before the player moves past its end. */
export function setBeforeAutoSkip(fn) {
    beforeAutoSkip = fn;
}

export function installPlaybackFailureHandler(player) {
    if (!window.__TAURI_INTERNALS__) return;
    const guard = new SkipGuard();
    const lastNotice = new Map();
    const notify = (kind, text = failureNotice(kind)) => {
        const now = Date.now();
        if (now - (lastNotice.get(text) ?? 0) < NOTICE_COOLDOWN_MS) return;
        lastNotice.set(text, now);
        showNotification(text, { type: 'error' });
    };

    document.addEventListener(
        'playing',
        (e) => {
            if (e.target === player.activeElement) guard.ok();
        },
        true
    );

    document.addEventListener(
        'error',
        (e) => {
            const el = e.target;
            if (!(el instanceof HTMLMediaElement) || el !== player.activeElement || !el.error) return;
            const track = player.currentTrack;
            if (!track?.id) return;
            void handleFailure(String(track.id));
        },
        true
    );

    async function handleFailure(id) {
        let kind = 'other';
        try {
            const failure = await invoke('stream_failure', { videoId: id });
            if (failure?.kind) kind = failure.kind;
        } catch {
            // Not a YouTube id (local file) or the command failed: generic text.
        }
        if (isFatal(kind)) {
            notify(kind);
            return;
        }
        if (!guard.fail(id)) {
            notify(kind, 'Несколько треков подряд не воспроизводятся, остановлено. ' + failureNotice(kind));
            guard.ok();
            return;
        }
        notify(kind);
        await new Promise((r) => setTimeout(r, SKIP_DELAY_MS));
        // The listener picked another track meanwhile: leave it alone.
        if (String(player.currentTrack?.id) !== id) return;
        try {
            await beforeAutoSkip?.();
        } catch (err) {
            console.warn('[playback] before-skip hook failed', err);
        }
        if (String(player.currentTrack?.id) !== id) return;
        await player.playNext();
    }
}
