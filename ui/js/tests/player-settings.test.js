import { describe, expect, it } from 'vitest';
import {
    buildBlocklist,
    buildQueueMigration,
    buildSettingsPatch,
    clampSpeed,
    modeToRepeat,
    repeatToMode,
} from '../player-settings.js';
import { isPlayableTrack } from '../player-mirror.js';

const sources = (over = {}) => ({
    replayGainSettings: { getMode: () => 'album', getPreamp: () => 3 },
    exponentialVolumeSettings: { isEnabled: () => true },
    crossfadeSettings: { isEnabled: () => true, getDuration: () => 7 },
    gaplessPlaybackSettings: { isEnabled: () => false },
    audioEffectsSettings: { getSpeed: () => 9, isPreservePitchEnabled: () => false },
    autoplaySettings: { isEnabled: () => true },
    contentBlockingSettings: {
        getBlockedTracks: () => [{ id: 'aaaaaaaaaaa' }],
        getBlockedArtists: () => [{ id: 3995478 }, { id: 'UCx' }, { id: 3995478 }],
        getBlockedAlbums: () => [],
    },
    ...over,
});

describe('player settings', () => {
    it('builds the PlayerSettings patch from the UI settings', () => {
        expect(buildSettingsPatch(sources(), { quality: 'LOW' })).toEqual({
            volumeCurve: 'cubic',
            quality: 'low',
            crossfade: { enabled: true, seconds: 7 },
            gapless: false,
            speed: 4,
            preservePitch: false,
            replayGain: { mode: 'album', preampDb: 3 },
            autoplay: true,
            blocklist: { trackIds: ['aaaaaaaaaaa'], artistIds: ['3995478', 'UCx'], albumIds: [] },
        });
        expect(buildSettingsPatch(sources(), { quality: 'HI_RES_LOSSLESS' }).quality).toBe('high');
    });

    it('maps repeat modes and clamps speed', () => {
        expect([0, 1, 2].map(modeToRepeat)).toEqual(['off', 'all', 'one']);
        expect(['off', 'all', 'one', 'bogus'].map(repeatToMode)).toEqual([0, 1, 2, 0]);
        expect([0.1, 1.5, 'x', 10].map(clampSpeed)).toEqual([0.25, 1.5, 1, 4]);
        expect(buildBlocklist(sources().contentBlockingSettings).artistIds).toEqual(['3995478', 'UCx']);
    });
});

describe('buildQueueMigration', () => {
    const t = (n) => ({ id: `abcdefghij${n}`, title: `T${n}` });

    it('imports the pre-shuffle order and finds the current track', () => {
        const saved = {
            queue: [t(1), t(2), { id: 'local-file-1.flac', isLocal: true, title: 'local' }, t(3)],
            shuffledQueue: [t(3), t(1), t(2)],
            currentQueueIndex: 0,
            shuffleActive: true,
            repeatMode: 2,
        };
        expect(buildQueueMigration(saved, isPlayableTrack)).toEqual({
            tracks: [t(1), t(2), t(3)],
            startIndex: 2,
            shuffle: true,
            repeat: 'one',
        });
    });

    it('returns null when there is nothing to import', () => {
        expect(buildQueueMigration(null, isPlayableTrack)).toBeNull();
        expect(buildQueueMigration({ queue: [{ id: 'podcast_1' }] }, isPlayableTrack)).toBeNull();
        expect(buildQueueMigration({ queue: [t(1)], currentQueueIndex: -1 }, isPlayableTrack)).toMatchObject({
            startIndex: 0,
            shuffle: false,
            repeat: 'off',
        });
    });
});
