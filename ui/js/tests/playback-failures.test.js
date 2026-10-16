import { describe, expect, it } from 'vitest';
import { failureNotice, isFatal, MAX_AUTO_SKIPS, SkipGuard } from '../playback-failures.js';

describe('playback-failures', () => {
    it('explains each failure kind', () => {
        expect(failureNotice('bot_check')).toMatch(/не бот/);
        expect(failureNotice('bot_check')).toMatch(/Сеть/);
        expect(failureNotice('unavailable')).toMatch(/недоступен/);
        expect(failureNotice('something new')).toBe(failureNotice('other'));
        expect(isFatal('no_yt_dlp')).toBe(true);
        expect(isFatal('bot_check')).toBe(false);
    });

    it('skips until too many tracks in a row fail', () => {
        const g = new SkipGuard();
        const verdicts = ['a', 'b', 'c', 'd'].map((id) => g.fail(id));
        expect(verdicts).toEqual([true, true, false, false]);
        expect(MAX_AUTO_SKIPS).toBe(3);
    });

    it('counts one track erroring twice once and resets on play', () => {
        const g = new SkipGuard(2);
        expect(g.fail('a')).toBe(true);
        expect(g.fail('a')).toBe(true);
        g.ok();
        expect(g.fail('b')).toBe(true);
        expect(g.fail('c')).toBe(false);
    });
});
