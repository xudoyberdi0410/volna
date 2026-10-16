import { describe, expect, it } from 'vitest';
import {
    applyChoice,
    DEFAULT_SETTINGS,
    needsTopUp,
    normalizeSettings,
    PlayTracker,
    summarize,
    trackForRust,
    upcomingWaveTracks,
} from '../wave-logic.js';

const t = (id) => ({ id, title: `T${id}`, artists: [{ id: 'UC1', name: 'A' }], duration: 200, type: 'track' });

describe('wave settings', () => {
    it('toggles character and mood like Yandex Music', () => {
        let s = applyChoice(DEFAULT_SETTINGS, 'character', 'unfamiliar');
        expect(s.character).toBe('unfamiliar');
        s = applyChoice(s, 'character', 'unfamiliar');
        expect(s.character).toBe('default');
        s = applyChoice(s, 'mood', 'calm');
        s = applyChoice(s, 'mood', 'sad');
        expect(s.mood).toBe('sad');
        s = applyChoice(s, 'activity', { title: 'Workout', params: 'p' });
        expect(s.activity).toEqual({ title: 'Workout', params: 'p' });
        expect(applyChoice(s, 'activity', { title: 'Workout', params: 'p' }).activity).toBeNull();
        expect(() => applyChoice(s, 'nope', 1)).toThrow();
    });

    it('cleans up unknown values', () => {
        expect(normalizeSettings({ character: 'weird', mood: 'angry', language: 'fr', activity: {} })).toEqual(
            DEFAULT_SETTINGS
        );
    });

    it('summarizes in Russian', () => {
        expect(summarize(DEFAULT_SETTINGS)).toBe('Под ваш вкус');
        expect(
            summarize(
                { character: 'popular', mood: 'calm', language: 'uzbek', activity: { title: 'Workout', params: 'x' } },
                'Тренировка'
            )
        ).toBe('Популярное · Спокойное · Тренировка · Узбекский');
    });
});

describe('wave queue', () => {
    it('tops up when fewer than three tracks are left', () => {
        expect(needsTopUp(5, 0)).toBe(false);
        expect(needsTopUp(5, 2)).toBe(true);
        expect(needsTopUp(0, -1)).toBe(true);
    });

    it('lists upcoming wave tracks from the end', () => {
        const queue = [t('a'), t('b'), t('x'), t('c')];
        expect(upcomingWaveTracks(queue, 0, new Set(['a', 'b', 'c']))).toEqual([
            { index: 3, id: 'c' },
            { index: 1, id: 'b' },
        ]);
    });

    it('slims tracks for Rust', () => {
        expect(trackForRust({ id: 'v', title: 'x', artist: { id: null, name: 'N' }, duration: NaN })).toEqual({
            id: 'v',
            title: 'x',
            artists: [{ id: null, name: 'N' }],
            duration: null,
            explicit: false,
            type: 'track',
        });
    });
});

describe('PlayTracker', () => {
    it('reports start once and end with the last position', () => {
        const p = new PlayTracker();
        expect(p.tick(t('a'), 0.1, 200)).toEqual([]);
        expect(p.tick(t('a'), 1, 200).map((e) => e.kind)).toEqual(['start']);
        expect(p.tick(t('a'), 12, 200)).toEqual([]);
        const events = p.tick(t('b'), 0, 180);
        expect(events).toHaveLength(1);
        expect(events[0]).toMatchObject({ kind: 'end', position: 12, duration: 200, ended: false });
        expect(events[0].track.id).toBe('a');
    });

    it('marks natural ends and ignores tracks that never started', () => {
        const p = new PlayTracker();
        p.tick(t('a'), 0.2, 200);
        expect(p.tick(t('b'), 0.0, 200)).toEqual([]);
        p.tick(t('b'), 2, 200);
        const [end] = p.ended(t('b'));
        expect(end).toMatchObject({ kind: 'end', ended: true, position: 200 });
        expect(p.ended(t('b'))).toEqual([]);
    });

    it('closes a play when the player moves on before the next track plays', () => {
        const p = new PlayTracker();
        p.tick(t('a'), 1, 200);
        p.tick(t('a'), 9, 200);
        expect(p.switchTo(t('a'))).toEqual([]);
        const [end] = p.switchTo(t('b'));
        expect(end).toMatchObject({ kind: 'end', position: 9, ended: false });
        expect(p.switchTo(t('c'))).toEqual([]);
    });

    it('does not trust a stale position right after a switch', () => {
        const p = new PlayTracker();
        p.tick(t('a'), 1, 200);
        p.tick(t('a'), 150, 200);
        // The element still reports the old time once with the new track.
        const out = p.tick(t('b'), 150, 200);
        expect(out.map((e) => e.kind)).toEqual(['end']);
        expect(p.tick(t('b'), 0.2, 180)).toEqual([]);
        expect(p.tick(t('b'), 0.9, 180).map((e) => e.kind)).toEqual(['start']);
        expect(p.current.position).toBe(0.9);
    });
});
