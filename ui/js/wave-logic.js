// js/wave-logic.js
// Pure helpers for «Моя волна» (js/wave.js): labels, settings edits, queue
// arithmetic and the play tracker that turns <audio> time updates into the
// start/end events the Rust engine wants. No DOM, no Tauri: unit-tested.

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

/** Tracks left after the current one. */
export function remainingAfter(queueLength, index) {
    return Math.max(0, queueLength - 1 - Math.max(index, -1));
}

/** The infinite queue tops up when fewer than `threshold` tracks are left. */
export function needsTopUp(queueLength, index, threshold = 3) {
    return remainingAfter(queueLength, index) < threshold;
}

/** Positions (descending, safe to remove in order) and ids of queued wave tracks after `index`. */
export function upcomingWaveTracks(queue, index, waveIds) {
    const out = [];
    for (let i = queue.length - 1; i > index; i--) {
        const id = queue[i]?.id;
        if (id != null && waveIds.has(String(id))) out.push({ index: i, id: String(id) });
    }
    return out;
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

/**
 * Follows what the player is doing and reports plays:
 * `{kind:'start', track}` once a track has really started (time > 0.5 s),
 * `{kind:'end', track, position, duration, ended}` when the player moves on.
 * Tracks that never started produce nothing.
 */
export class PlayTracker {
    constructor() {
        this.current = null;
    }

    /** A `timeupdate` of the active media element for `track`. */
    tick(track, position, duration) {
        const out = [];
        if (!track || track.id == null) return out;
        const id = String(track.id);
        if (!this.current || this.current.id !== id) {
            out.push(...this.leave());
            this.current = { id, track, position: 0, duration: null, started: false, trusted: false, ticks: 0 };
        }
        const c = this.current;
        c.ticks += 1;
        if (Number.isFinite(duration) && duration > 0) c.duration = duration;
        if (Number.isFinite(position)) {
            // Right after a switch the element may still report the old
            // track's time; trust positions once one near the start was seen
            // (or after a few ticks, for tracks resumed mid-way).
            if (!c.trusted && (position < 3 || c.ticks > 4)) c.trusted = true;
            if (c.trusted) c.position = position;
        }
        if (!c.started && c.trusted && c.position > 0.5) {
            c.started = true;
            out.push({ kind: 'start', track: c.track });
        }
        return out;
    }

    /** The element fired `ended` for `track`. */
    ended(track) {
        if (!this.current || track == null || this.current.id !== String(track.id)) return [];
        this.current.ended = true;
        if (this.current.duration) this.current.position = this.current.duration;
        return this.leave();
    }

    /**
     * The player moved to `track` (it may not play yet, e.g. while loading
     * or when its stream fails): closes the previous play if it was another.
     */
    switchTo(track) {
        if (!this.current || (track && this.current.id === String(track.id))) return [];
        return this.leave();
    }

    /** Closes the current play (track switch, app hiding). */
    leave() {
        const c = this.current;
        this.current = null;
        if (!c || !c.started) return [];
        return [
            {
                kind: 'end',
                track: c.track,
                position: c.position,
                duration: c.duration,
                ended: !!c.ended,
            },
        ];
    }
}
