import { describe, expect, it } from 'vitest';
import { failureNotice, isFatal, MAX_AUTO_SKIPS, noticeFor } from '../playback-failures.js';

const error = (over) => ({
    uid: 'q1',
    trackId: 'aaaaaaaaaaa',
    kind: 'bot_check',
    message: 'Sign in to confirm',
    action: 'skipped',
    streak: 1,
    ...over,
});

describe('playback-failures', () => {
    it('explains each failure kind', () => {
        expect(failureNotice('bot_check')).toMatch(/не бот/);
        expect(failureNotice('bot_check')).toMatch(/Сеть/);
        expect(failureNotice('unavailable')).toMatch(/недоступен/);
        expect(failureNotice('something new')).toBe(failureNotice('other'));
        expect(isFatal('no_yt_dlp')).toBe(true);
        expect(isFatal('bot_check')).toBe(false);
    });

    it('turns player:error into a notice', () => {
        expect(noticeFor(error())).toBe(failureNotice('bot_check'));
        expect(noticeFor(error({ kind: 'timeout' }))).toBe(failureNotice('other'));
        expect(noticeFor(error({ kind: 'no_yt_dlp', action: 'stopped', streak: 1 }))).toBe(failureNotice('no_yt_dlp'));
        expect(noticeFor(error({ action: 'retrying' }))).toBeNull();
        expect(noticeFor(null)).toBeNull();
    });

    it('says so when several tracks in a row failed', () => {
        const text = noticeFor(error({ action: 'stopped', streak: MAX_AUTO_SKIPS }));
        expect(text).toMatch(/^Несколько треков подряд/);
        expect(text).toContain(failureNotice('bot_check'));
    });
});
