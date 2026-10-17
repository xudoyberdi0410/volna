// js/player-settings.js
// Builds the PlayerSettings patch (docs/native-player.md §3.2) from the
// localStorage-backed settings the UI already has. Pure apart from reading
// those settings objects, which are passed in so tests can fake them.

export const REPEAT_MODES = Object.freeze(['off', 'all', 'one']); // UI REPEAT_MODE 0/1/2

export function repeatToMode(repeat) {
    const i = REPEAT_MODES.indexOf(repeat);
    return i === -1 ? 0 : i;
}

export function modeToRepeat(mode) {
    return REPEAT_MODES[Number(mode)] ?? 'off';
}

export const MIN_SPEED = 0.25;
export const MAX_SPEED = 4;

export function clampSpeed(speed) {
    const n = parseFloat(speed);
    return Math.max(MIN_SPEED, Math.min(MAX_SPEED, Number.isFinite(n) ? n : 1));
}

const ids = (list) => [
    ...new Set(
        (list || [])
            .map((x) => x?.id)
            .filter((id) => id != null)
            .map(String)
    ),
];

/** `settings.blocklist` from contentBlockingSettings (hardcoded ids included). */
export function buildBlocklist(blocking) {
    return {
        trackIds: ids(blocking.getBlockedTracks()),
        artistIds: ids(blocking.getBlockedArtists()),
        albumIds: ids(blocking.getBlockedAlbums()),
    };
}

/**
 * Everything the UI owns in PlayerSettings except volume/muted (those follow
 * the slider and are sent on their own).
 * @param {object} s the storage.js settings objects
 */
export function buildSettingsPatch(s, { quality } = {}) {
    const q = String(quality ?? localStorageGet('playback-quality') ?? '').toUpperCase();
    const mode = s.replayGainSettings.getMode();
    return {
        volumeCurve: s.exponentialVolumeSettings.isEnabled() ? 'cubic' : 'linear',
        quality: q === 'LOW' ? 'low' : 'high',
        crossfade: { enabled: !!s.crossfadeSettings.isEnabled(), seconds: s.crossfadeSettings.getDuration() },
        gapless: !!s.gaplessPlaybackSettings.isEnabled(),
        speed: clampSpeed(s.audioEffectsSettings.getSpeed()),
        preservePitch: !!s.audioEffectsSettings.isPreservePitchEnabled(),
        replayGain: {
            mode: ['off', 'track', 'album'].includes(mode) ? mode : 'track',
            preampDb: Number(s.replayGainSettings.getPreamp()) || 0,
        },
        autoplay: !!s.autoplaySettings.isEnabled(),
        blocklist: buildBlocklist(s.contentBlockingSettings),
    };
}

function localStorageGet(key) {
    try {
        return globalThis.localStorage?.getItem(key) ?? null;
    } catch {
        return null;
    }
}

/** volume/muted as stored by the old `<audio>` player. */
export function storedVolume() {
    const v = parseFloat(localStorageGet('volume'));
    return {
        volume: Number.isFinite(v) ? Math.max(0, Math.min(1, v)) : 0.7,
        muted: localStorageGet('muted') === 'true',
    };
}
