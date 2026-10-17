import { describe, expect, it } from 'vitest';
import { applyChoice, DEFAULT_SETTINGS, normalizeSettings, summarize, trackForRust } from '../wave-logic.js';

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

describe('trackForRust', () => {
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
