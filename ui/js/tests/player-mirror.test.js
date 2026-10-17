import { describe, expect, it } from 'vitest';
import { EMPTY_QUEUE, PlayerMirror, isPlayableTrack, toTrackInput } from '../player-mirror.js';
import { SNAPSHOT } from './fixtures/native-player.js';

const entry = (uid, id, extra = {}) => ({
    uid,
    id,
    title: `T ${id}`,
    artists: [{ id: null, name: 'A' }],
    album: null,
    duration: 100,
    cover: null,
    explicit: false,
    kind: 'track',
    origin: 'user',
    raw: null,
    ui: { id, title: `T ${id}`, fromUi: true },
    unavailable: false,
    ...extra,
});

// §3.7 second fixture, with the elided fields filled in.
const CHANGE_13 = {
    rev: 13,
    order: ['q1a', 'q1b', 'q1c'],
    original: null,
    index: 0,
    shuffle: false,
    repeat: 'off',
    continuation: { kind: 'wave', fetching: false },
    added: [entry('q1b', 'bbbbbbbbbbb'), entry('q1c', 'ccccccccccc')],
    updated: [],
};

const prepare = (raw) => ({ ...raw, prepared: true });

describe('PlayerMirror', () => {
    it('applies the §3.7 snapshot and builds rich tracks from raw', () => {
        const m = new PlayerMirror({ prepare });
        m.reset(SNAPSHOT.queue);
        expect(m.rev).toBe(12);
        expect(m.order).toEqual(['q1a']);
        expect(m.index).toBe(0);
        expect(m.continuation).toEqual({ kind: 'wave', fetching: false });
        const t = m.currentTrack;
        expect(t).toMatchObject({
            id: 'dQw4w9WgXcQ',
            prepared: true,
            _uid: 'q1a',
            _origin: 'wave',
            isUnavailable: false,
        });
        expect(m.playQueue).toEqual([t]);
        expect(m.originalQueue).toEqual([t]);
    });

    it('applies the §3.7 delta (rev + 1) and prefers the ui blob', () => {
        const m = new PlayerMirror({ prepare });
        m.reset(SNAPSHOT.queue);
        const first = m.currentTrack;
        expect(m.apply(CHANGE_13)).toBe('applied');
        expect(m.rev).toBe(13);
        expect(m.playQueue.map((t) => t.id)).toEqual(['dQw4w9WgXcQ', 'bbbbbbbbbbb', 'ccccccccccc']);
        expect(m.track('q1b')).toMatchObject({ fromUi: true, _uid: 'q1b', _origin: 'user' });
        expect(m.currentTrack).toBe(first);
    });

    it('ignores stale deltas and reports gaps', () => {
        const m = new PlayerMirror({ prepare });
        m.reset(SNAPSHOT.queue);
        expect(m.apply({ ...CHANGE_13, rev: 12 })).toBe('stale');
        expect(m.apply({ ...CHANGE_13, rev: 11 })).toBe('stale');
        expect(m.apply({ ...CHANGE_13, rev: 14 })).toBe('gap');
        expect(m.rev).toBe(12);
        // An order naming a uid the mirror never saw is a gap too.
        expect(m.apply({ ...CHANGE_13, added: [] })).toBe('gap');
        expect(m.rev).toBe(12);
    });

    it('updates entries in place and drops removed ones', () => {
        const m = new PlayerMirror({ prepare });
        m.reset(SNAPSHOT.queue);
        m.apply(CHANGE_13);
        const b = m.track('q1b');
        expect(
            m.apply({
                ...CHANGE_13,
                rev: 14,
                order: ['q1a', 'q1b'],
                added: [],
                updated: [entry('q1b', 'bbbbbbbbbbb', { unavailable: true })],
            })
        ).toBe('applied');
        expect(m.track('q1b')).toBe(b);
        expect(b.isUnavailable).toBe(true);
        expect(m.track('q1c')).toBeNull();
        expect(m.entries.has('q1c')).toBe(false);
    });

    it('keeps the pre-shuffle order while shuffle is on', () => {
        const m = new PlayerMirror({ prepare });
        m.reset(SNAPSHOT.queue);
        m.apply(CHANGE_13);
        m.apply({
            ...CHANGE_13,
            rev: 14,
            order: ['q1b', 'q1c', 'q1a'],
            original: ['q1a', 'q1b', 'q1c'],
            shuffle: true,
            index: 0,
            added: [],
        });
        expect(m.shuffle).toBe(true);
        expect(m.currentUid).toBe('q1b');
        expect(m.originalQueue.map((t) => t._uid)).toEqual(['q1a', 'q1b', 'q1c']);
        expect(m.playQueue.map((t) => t._uid)).toEqual(['q1b', 'q1c', 'q1a']);
    });

    it('builds a rich track from the entry when there is no ui or raw', () => {
        const m = new PlayerMirror({ prepare });
        m.reset({ ...EMPTY_QUEUE, rev: 1, index: 0, items: [{ ...SNAPSHOT.queue.items[0], raw: null, ui: null }] });
        expect(m.currentTrack).toMatchObject({
            id: 'dQw4w9WgXcQ',
            title: 'Never Gonna Give You Up',
            artist: { name: 'Rick Astley' },
            album: { id: 'MPREb_x', cover: SNAPSHOT.queue.items[0].cover },
            type: 'track',
        });
    });
});

describe('toTrackInput', () => {
    it('maps a rich UI track and strips mirror fields from the ui blob', () => {
        const rich = {
            id: 'dQw4w9WgXcQ',
            title: 'Never Gonna Give You Up',
            artist: { id: 'UCx', name: 'Rick Astley' },
            artists: [{ id: 'UCx', name: 'Rick Astley', picture: 'p' }],
            album: { id: 'MPREb_x', title: 'Album', cover: 'volna://img/?u=x&s=640' },
            duration: 213,
            explicit: 1,
            type: 'video',
            _uid: 'q1',
            _origin: 'wave',
            _originalIndex: 3,
        };
        const input = toTrackInput(rich, 'user');
        expect(input).toMatchObject({
            id: 'dQw4w9WgXcQ',
            title: 'Never Gonna Give You Up',
            artists: [{ id: 'UCx', name: 'Rick Astley' }],
            album: { id: 'MPREb_x', title: 'Album' },
            duration: 213,
            cover: 'volna://img/?u=x&s=640',
            explicit: true,
            kind: 'video',
            origin: 'user',
        });
        expect(input.ui._uid).toBeUndefined();
        expect(input.ui._origin).toBeUndefined();
        expect(input.ui.album.cover).toBe('volna://img/?u=x&s=640');
        expect(JSON.parse(JSON.stringify(input))).toEqual(input);
    });

    it('handles missing fields', () => {
        expect(toTrackInput({ id: 'aaaaaaaaaaa', artist: 'Someone', cover: 'not a url' })).toMatchObject({
            artists: [{ id: null, name: 'Someone' }],
            album: null,
            duration: null,
            cover: null,
            kind: 'track',
        });
    });

    it('accepts only YouTube ids', () => {
        expect(isPlayableTrack({ id: 'dQw4w9WgXcQ' })).toBe(true);
        expect(isPlayableTrack({ id: 'dQw4w9WgXcQ', isLocal: true })).toBe(false);
        expect(isPlayableTrack({ id: 'podcast_1' })).toBe(false);
        expect(isPlayableTrack(null)).toBe(false);
    });
});
