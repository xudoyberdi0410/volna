// js/playback-failures.js
// When a track cannot be played, say why. The native player decides what
// to do (retry, skip up to three in a row, stop) and reports each failure
// as `player:error` (docs/native-player.md §3.4); this module only turns
// that into a notification.

import { showNotification } from './notifications.js';

/** The native player stops after this many tracks in a row failed to play. */
export const MAX_AUTO_SKIPS = 3;
/** The same explanation is not repeated more often than this. */
const NOTICE_COOLDOWN_MS = 15_000;

const NOTICES = {
    bot_check:
        'YouTube не отдаёт звук через это подключение («подтвердите, что вы не бот»). Смените подключение или включите cookies в Настройки → Сеть',
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

/** The text for a PlayerError, or null when nothing should be shown. */
export function noticeFor(error) {
    if (!error || error.action === 'retrying' || error.action === 'none') return null;
    const kind = error.kind === 'decode' || error.kind === 'timeout' ? 'other' : error.kind;
    if (error.action === 'stopped' && error.streak >= MAX_AUTO_SKIPS) {
        return 'Несколько треков подряд не воспроизводятся, остановлено. ' + failureNotice(kind);
    }
    return failureNotice(kind);
}

export function installPlaybackFailureHandler(player) {
    const lastNotice = new Map();
    player.on('error', (error) => {
        const text = noticeFor(error);
        if (!text) return;
        const now = Date.now();
        if (now - (lastNotice.get(text) ?? 0) < NOTICE_COOLDOWN_MS) return;
        lastNotice.set(text, now);
        showNotification(text, { type: 'error' });
    });
}
