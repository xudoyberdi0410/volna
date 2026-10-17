// js/player-mirror.js
// The UI's copy of the native player's queue (docs/native-player.md §3.2,
// §3.6, §5.1). Pure: no DOM, no Tauri, unit-tested.
//
// Rust owns the queue. The mirror applies a full QueueSnapshot or a
// QueueChanged delta and keeps, for every queue entry (`uid`), the rich
// track object the rest of the UI renders. A delta whose `rev` is not
// exactly `rev + 1` cannot be applied: the caller takes a new snapshot.

export const EMPTY_CONTINUATION = Object.freeze({ kind: 'none', fetching: false });

export const EMPTY_QUEUE = Object.freeze({
    rev: 0,
    items: [],
    original: null,
    index: -1,
    shuffle: false,
    repeat: 'off',
    continuation: EMPTY_CONTINUATION,
});

const YOUTUBE_ID = /^[A-Za-z0-9_-]{11}$/;

/** The native player only plays YouTube ids (no local files, podcasts, blobs). */
export function isPlayableTrack(track) {
    return !!track && !track.isLocal && YOUTUBE_ID.test(String(track.id ?? ''));
}

function pickCover(track) {
    const candidates = [track.album?.cover, track.cover, track.image];
    for (const c of candidates) {
        if (typeof c === 'string' && /^(?:[a-z][a-z0-9+.-]*:|\/)/i.test(c)) return c;
    }
    return null;
}

/** A JSON copy without the mirror's own bookkeeping fields. */
function uiBlob(track) {
    try {
        return JSON.parse(JSON.stringify(track, (key, value) => (key.startsWith('_') ? undefined : value)));
    } catch {
        return null;
    }
}

/** The TrackInput (§3.2) for a rich UI track. */
export function toTrackInput(track, origin) {
    const list = Array.isArray(track.artists) && track.artists.length ? track.artists : [track.artist];
    const artists = list
        .filter(Boolean)
        .map((a) => (typeof a === 'string' ? { id: null, name: a } : a))
        .map((a) => ({ id: a.id == null ? null : String(a.id), name: a.name || '' }));
    const album = track.album
        ? { id: track.album.id == null ? null : String(track.album.id), title: track.album.title || '' }
        : null;
    const d = Number(track.duration);
    const input = {
        id: String(track.id ?? ''),
        title: track.title || '',
        artists,
        album,
        duration: Number.isFinite(d) && d > 0 ? d : null,
        cover: pickCover(track),
        explicit: !!track.explicit,
        kind: track.type === 'video' ? 'video' : 'track',
        ui: uiBlob(track),
    };
    if (origin) input.origin = origin;
    return input;
}

/** A rich track for an entry that has neither `ui` nor `raw`. */
function fromPlayerTrack(pt) {
    const artists = (pt.artists || []).map((a) => ({ ...a }));
    return {
        id: pt.id,
        title: pt.title,
        artists,
        artist: artists[0] || null,
        album: pt.album || pt.cover ? { ...(pt.album || {}), cover: pt.cover ?? undefined } : null,
        duration: pt.duration,
        explicit: pt.explicit,
        type: pt.kind === 'video' ? 'video' : 'track',
    };
}

export class PlayerMirror {
    /**
     * @param {{prepare?: (raw: object) => object}} opts `prepare` turns a
     *   hifi-api shaped `raw` track into a rich one (LosslessAPI.prepareTrack).
     */
    constructor({ prepare } = {}) {
        this.prepare = prepare || ((raw) => ({ ...raw }));
        this.reset(EMPTY_QUEUE);
    }

    /** Replaces everything with a QueueSnapshot. */
    reset(snapshot) {
        const q = snapshot || EMPTY_QUEUE;
        this.rev = q.rev ?? 0;
        this.entries = new Map();
        this.rich = new Map();
        for (const pt of q.items || []) this._put(pt);
        this.order = (q.items || []).map((pt) => pt.uid);
        this.original = q.original ? [...q.original] : null;
        this.index = q.index ?? -1;
        this.shuffle = !!q.shuffle;
        this.repeat = q.repeat || 'off';
        this.continuation = { ...EMPTY_CONTINUATION, ...(q.continuation || {}) };
    }

    /**
     * Applies a QueueChanged. Returns 'applied', 'stale' (already seen) or
     * 'gap' (a delta was missed or is inconsistent: take a new snapshot).
     */
    apply(change) {
        if (!change || typeof change.rev !== 'number') return 'gap';
        if (change.rev <= this.rev) return 'stale';
        if (change.rev !== this.rev + 1) return 'gap';

        const added = new Map((change.added || []).map((pt) => [pt.uid, pt]));
        const known = (uid) => this.entries.has(uid) || added.has(uid);
        if (!change.order.every(known) || (change.original && !change.original.every(known))) return 'gap';

        for (const pt of added.values()) this._put(pt);
        for (const pt of change.updated || []) this._update(pt);

        this.rev = change.rev;
        this.order = [...change.order];
        this.original = change.original ? [...change.original] : null;
        this.index = change.index;
        this.shuffle = !!change.shuffle;
        this.repeat = change.repeat || 'off';
        this.continuation = { ...EMPTY_CONTINUATION, ...(change.continuation || {}) };
        this._collect();
        return 'applied';
    }

    _put(pt) {
        this.entries.set(pt.uid, pt);
        this.rich.set(pt.uid, this._enrich(pt));
    }

    _update(pt) {
        const old = this.rich.get(pt.uid);
        this.entries.set(pt.uid, pt);
        if (!old) {
            this.rich.set(pt.uid, this._enrich(pt));
            return;
        }
        // Rust only changes its own fields (e.g. `unavailable`); keep the
        // object the UI already holds.
        old.isUnavailable = !!pt.unavailable;
        old._origin = pt.origin;
    }

    _enrich(pt) {
        let rich;
        if (pt.ui && typeof pt.ui === 'object') rich = { ...pt.ui };
        else if (pt.raw && typeof pt.raw === 'object') rich = this.prepare(pt.raw);
        else rich = fromPlayerTrack(pt);
        rich._uid = pt.uid;
        rich._origin = pt.origin || 'user';
        rich.isUnavailable = !!pt.unavailable;
        return rich;
    }

    _collect() {
        const live = new Set(this.order);
        if (this.original) for (const uid of this.original) live.add(uid);
        for (const uid of [...this.entries.keys()]) {
            if (!live.has(uid)) {
                this.entries.delete(uid);
                this.rich.delete(uid);
            }
        }
    }

    track(uid) {
        return uid == null ? null : (this.rich.get(uid) ?? null);
    }

    entry(uid) {
        return uid == null ? null : (this.entries.get(uid) ?? null);
    }

    uidAt(index) {
        return this.order[index] ?? null;
    }

    indexOf(uid) {
        return this.order.indexOf(uid);
    }

    get length() {
        return this.order.length;
    }

    get currentUid() {
        return this.uidAt(this.index);
    }

    get currentTrack() {
        return this.track(this.currentUid);
    }

    /** Rich tracks in play order (shuffled order while shuffle is on). */
    get playQueue() {
        return this.order.map((uid) => this.rich.get(uid));
    }

    /** Rich tracks in the pre-shuffle order (the play order when shuffle is off). */
    get originalQueue() {
        return this.original ? this.original.map((uid) => this.rich.get(uid)) : this.playQueue;
    }
}
