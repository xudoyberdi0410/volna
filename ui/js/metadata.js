import { getMetadataWithTagLib } from './taglib.ts';

/**
 * @typedef {import('./container-classes.ts').Track} Track
 * @typedef {import('./container-classes.ts').EnrichedTrack} EnrichedTrack
 * @typedef {import("./taglib.types.ts").TagLibMetadata} TagLibMetadata
 */

/**
 * Reads metadata from a file
 * @param {Uint8Array | Blob | File | FileSystemFileHandle | FileSystemFileEntry} file
 * @returns {Promise<Object>} Track metadata
 */
export async function readTrackMetadata(file, { filename = file?.name || 'Unknown Title', siblings } = {}) {
    const metadata = {
        title: filename?.replace(/\.[^/.]+$/, ''),
        artists: [],
        artist: { name: 'Unknown Artist' }, // For fallback/compatibility
        album: { title: 'Unknown Album', cover: 'assets/appicon.png', releaseDate: null },
        duration: 0,
        isrc: null,
        copyright: null,
        explicit: false,
        isLocal: true,
        file: file,
        id: `local-${filename}-${file.lastModified}`,
    };

    try {
        const data = await getMetadataWithTagLib(file, filename, true);

        if (data) {
            metadata.title = data.title || metadata.title;
            const artistNames = (data.artist || '')
                .split(';')
                .map((a) => a.trim())
                .filter((a) => a);

            if (artistNames.length > 0) {
                metadata.artists = artistNames.map((name) => ({ name }));
                metadata.artist = metadata.artists[0];
            }

            metadata.album.title = data.albumTitle || metadata.album.title;
            metadata.album.releaseDate = data.releaseDate || metadata.album.releaseDate;

            if (data.albumArtist) {
                metadata.album.artist = { name: data.albumArtist };
            } else if (metadata.artist.name !== 'Unknown Artist') {
                metadata.album.artist = { name: metadata.artist.name };
            }

            if (data.cover) {
                const blob = new Blob([data.cover.data], { type: data.cover.type });
                metadata.album.cover = URL.createObjectURL(blob);
            }

            metadata.duration = data.duration;
            metadata.isrc = data.isrc || metadata.isrc;
            metadata.copyright = data.copyright || metadata.copyright;
            metadata.explicit = !!data.explicit;

            // parse TIDAL_DATA if present
            if (data.extra?.TIDAL_DATA) {
                try {
                    // parse as JSON if it's a string and then filter it
                    const parsed = JSON.parse(data.extra.TIDAL_DATA);
                    metadata.tidalData = filterTidalTrack(parsed);
                } catch {
                    // filter TIDAL_DATA if it's already an object
                    if (typeof data.extra.TIDAL_DATA === 'object') {
                        metadata.tidalData = filterTidalTrack(data.extra.TIDAL_DATA);
                    } else {
                        metadata.tidalData = data.extra.TIDAL_DATA; // raw string fallback
                        console.warn('TIDAL_DATA is not valid JSON or object, storing as raw string');
                    }
                }
            }

            // Reusable filtering function
            // if you want to add more fields in the future, just add them to this function
            // it will be applied to both parsed and raw TIDAL_DATA
            function filterTidalTrack(full) {
                return {
                    id: full.id,
                    title: full.title,
                    duration: full.duration,
                    replayGain: full.replayGain,
                    peak: full.peak,
                    djReady: full.djReady,
                    stemReady: full.stemReady,
                    streamStartDate: full.streamStartDate,
                    trackNumber: full.trackNumber,
                    volumeNumber: full.volumeNumber,
                    version: full.version,
                    popularity: full.popularity,
                    copyright: full.copyright,
                    bpm: full.bpm,
                    key: full.key,
                    keyScale: full.keyScale,
                    url: full.url,
                    isrc: full.isrc,
                    explicit: full.explicit,
                    audioQuality: full.audioQuality,
                    audioModes: full.audioModes,
                    mediaMetadata: full.mediaMetadata,
                    spotlighted: full.spotlighted,
                    ai: full.ai,
                    artist: full.artist,
                    artists: full.artists,
                    mixes: full.mixes,
                    album: full.album
                        ? {
                              id: full.album.id,
                              title: full.album.title,
                              cover: full.album.cover,
                              vibrantColor: full.album.vibrantColor,
                              videoCover: full.album.videoCover,
                              releaseDate: full.album.releaseDate,
                              albumURL: full.album.url,
                          }
                        : undefined,
                };
            }
        }
    } catch (e) {
        console.warn('Error reading metadata for', filename, e);
    }

    if (metadata.album.cover === 'assets/appicon.png' && siblings?.length > 0) {
        const baseName = filename.substring(0, filename.lastIndexOf('.'));
        const imageExtensions = ['.jpg', '.jpeg', '.png', '.webp'];
        const coverFile = siblings.find((f) => {
            const fName = f.name;
            const lastDot = fName.lastIndexOf('.');
            if (lastDot === -1) return false;
            const fBase = fName.substring(0, lastDot);
            const fExt = fName.substring(lastDot).toLowerCase();
            return fBase === baseName && imageExtensions.includes(fExt);
        });

        if (coverFile) {
            metadata.album.cover = URL.createObjectURL(coverFile);
        }
    }

    return metadata;
}
