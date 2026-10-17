// js/wave-logic.js
// Pure helpers for «Моя волна» (js/wave.js): labels, settings edits and the
// slim track Rust reads. No DOM, no Tauri: unit-tested. Queue top-ups and
// play tracking moved to the native player (crates/player).

export const CHARACTERS = [
    { value: 'favorite', label: 'Любимое' },
    { value: 'unfamiliar', label: 'Незнакомое' },
    { value: 'popular', label: 'Популярное' },
];

export const MOODS = [
    { value: 'energetic', label: 'Бодрое' },
    { value: 'happy', label: 'Весёлое' },
    { value: 'calm', label: 'Спокойное' },
    { value: 'sad', label: 'Грустное' },
];

export const LANGUAGES = [
    { value: 'any', label: 'Любой' },
    { value: 'russian', label: 'Русский' },
    { value: 'uzbek', label: 'Узбекский' },
    { value: 'english', label: 'Английский' },
];

export const DEFAULT_SETTINGS = Object.freeze({
    character: 'default',
    mood: null,
    activity: null,
    language: 'any',
});

/** Settings as the engine expects them, whatever came in. */
export function normalizeSettings(raw) {
    const s = { ...DEFAULT_SETTINGS, ...(raw || {}) };
    if (!['default', ...CHARACTERS.map((c) => c.value)].includes(s.character)) s.character = 'default';
    if (s.mood !== null && !MOODS.some((m) => m.value === s.mood)) s.mood = null;
    if (!LANGUAGES.some((l) => l.value === s.language)) s.language = 'any';
    if (s.activity && !(s.activity.title && s.activity.params)) s.activity = null;
    return s;
}

/**
 * One click in the settings popover. Character and mood act as toggles
 * (clicking the chosen chip clears it, as in Yandex Music); language is a
 * plain choice; activity toggles by title.
 */
export function applyChoice(settings, group, value) {
    const s = normalizeSettings(settings);
    switch (group) {
        case 'character':
            s.character = s.character === value ? 'default' : value;
            break;
        case 'mood':
            s.mood = s.mood === value ? null : value;
            break;
        case 'language':
            s.language = value;
            break;
        case 'activity':
            s.activity =
                value && s.activity?.title !== value.title ? { title: value.title, params: value.params } : null;
            break;
        default:
            throw new Error(`unknown wave setting ${group}`);
    }
    return s;
}

const labelOf = (list, value) => list.find((x) => x.value === value)?.label;

/** "Незнакомое · Спокойное · Тренировка · Русский", or the default line. */
export function summarize(settings, activityLabel = null) {
    const s = normalizeSettings(settings);
    const parts = [
        labelOf(CHARACTERS, s.character),
        labelOf(MOODS, s.mood),
        s.activity ? activityLabel || s.activity.title : null,
        s.language !== 'any' ? labelOf(LANGUAGES, s.language) : null,
    ].filter(Boolean);
    return parts.length ? parts.join(' · ') : 'Под ваш вкус';
}

/** The slim track object Rust reads (see src-tauri/src/wave.rs `UiTrack`). */
export function trackForRust(track) {
    if (!track) return null;
    const artists = (Array.isArray(track.artists) && track.artists.length ? track.artists : [track.artist])
        .filter(Boolean)
        .map((a) => ({ id: a.id == null ? null : String(a.id), name: a.name || '' }));
    return {
        id: String(track.id ?? ''),
        title: track.title || '',
        artists,
        duration: Number.isFinite(track.duration) ? track.duration : null,
        explicit: !!track.explicit,
        type: track.type === 'video' ? 'video' : 'track',
    };
}
