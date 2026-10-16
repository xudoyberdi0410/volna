//js/api.js
// hifi-api client. In Volna every request goes to the Rust emulator at
// `volna://api` (http://volna.localhost/api on Windows), which answers from
// YouTube Music in the shapes documented in docs/hifi-api-contract.md.
import { deriveTrackQuality, isTrackUnavailable } from './utils.js';
import { trackDateSettings } from './storage.js';
import { APICache } from './cache.js';
import { TrackAlbum, Track, Album, PreparedVideo, PreparedTrack } from './container-classes.js';
import { volnaUrl } from './volna-url.js';
import { buildStreamDescriptor, isDirectImage, resizeImage } from './volna-media.js';

export const DASH_MANIFEST_UNAVAILABLE_CODE = 'DASH_MANIFEST_UNAVAILABLE';
export const UNSUPPORTED_PLAYBACK_CODEC_CODE = 'UNSUPPORTED_PLAYBACK_CODEC';
export const STRICT_QUALITY_UNAVAILABLE_CODE = 'STRICT_QUALITY_UNAVAILABLE';

export class LosslessAPI {
    constructor(settings) {
        this.settings = settings;
        this.cache = new APICache({
            maxSize: 200,
            ttl: 1000 * 60 * 30,
        });
        this._combinedSearchUnavailable = false;

        setInterval(
            async () => {
                await this.cache.clearExpired();
            },
            1000 * 60 * 5
        );
    }

    usesSingleUsePlaybackUrls() {
        return false;
    }

    /**
     * GET `volna://api<relativePath>`. Throws on any non-2xx status; callers
     * render an empty section or a "could not load" message.
     */
    async fetchWithRetry(relativePath, options = {}) {
        const path = relativePath.startsWith('/') ? relativePath : `/${relativePath}`;
        const response = await fetch(volnaUrl('api', path), { signal: options.signal });
        if (!response.ok) {
            let detail = '';
            try {
                detail = (await response.clone().json())?.error || '';
            } catch {
                // not JSON
            }
            throw new Error(`volna api ${response.status} for ${path}${detail ? `: ${detail}` : ''}`);
        }
        return response;
    }

    findSearchSection(source, key, visited) {
        if (!source || typeof source !== 'object') return;

        if (Array.isArray(source)) {
            for (const e of source) {
                const f = this.findSearchSection(e, key, visited);
                if (f) return f;
            }
            return;
        }

        if (visited.has(source)) return;
        visited.add(source);

        if ('items' in source && Array.isArray(source.items)) return source;

        if (key in source) {
            const f = this.findSearchSection(source[key], key, visited);
            if (f) return f;
        }

        for (const v of Object.values(source)) {
            const f = this.findSearchSection(v, key, visited);
            if (f) return f;
        }
    }

    buildSearchResponse(section) {
        const items = section?.items ?? [];
        return {
            items,
            limit: section?.limit ?? items.length,
            offset: section?.offset ?? 0,
            totalNumberOfItems: section?.totalNumberOfItems ?? items.length,
        };
    }

    normalizeSearchResponse(data, key) {
        const section = this.findSearchSection(data, key, new Set());
        return this.buildSearchResponse(section);
    }

    prepareTrack(track) {
        let normalized = { ...track };

        if (track.type && typeof track.type === 'string') {
            const lowType = track.type.toLowerCase();
            if (lowType.includes('video')) {
                normalized.type = 'video';
            } else if (lowType.includes('track')) {
                normalized.type = 'track';
            } else {
                normalized.type = lowType;
            }
        }

        if (!normalized.artist && Array.isArray(normalized.artists) && normalized.artists.length > 0) {
            normalized.artist = normalized.artists[0];
        } else if (normalized.artist && !normalized.artists) {
            normalized.artists = [normalized.artist];
        }

        if (track.album) {
            normalized.album = { ...track.album };
            if (track.album.releaseDate) {
                normalized.album.releaseDate = track.album.releaseDate;
            }
        }

        const derivedQuality = deriveTrackQuality(normalized);
        if (derivedQuality && normalized.audioQuality !== derivedQuality) {
            normalized.audioQuality = derivedQuality;
        }

        normalized.isUnavailable = isTrackUnavailable(normalized);

        return normalized.type == 'video' ? new PreparedVideo(normalized) : new PreparedTrack(normalized);
    }

    prepareAlbum(album) {
        if (!album.artist && Array.isArray(album.artists) && album.artists.length > 0) {
            return { ...album, artist: album.artists[0] };
        }
        return album;
    }

    preparePlaylist(playlist) {
        return playlist;
    }

    prepareVideo(video) {
        let normalized = { ...video, type: 'video' };

        if (!video.artist && Array.isArray(video.artists) && video.artists.length > 0) {
            normalized.artist = video.artists[0];
        }

        return normalized;
    }

    prepareArtist(artist) {
        if (!artist.type && Array.isArray(artist.artistTypes) && artist.artistTypes.length > 0) {
            return { ...artist, type: artist.artistTypes[0] };
        }
        return artist;
    }

    async enrichTracksWithAlbumDates(tracks, maxRequests = 20) {
        if (!trackDateSettings.useAlbumYear()) return tracks;

        const albumIdsToFetch = [];
        for (const track of tracks) {
            if (!track.album?.releaseDate && track.album?.id && !albumIdsToFetch.includes(track.album.id)) {
                albumIdsToFetch.push(track.album.id);
            }
        }

        if (albumIdsToFetch.length === 0) return tracks;

        // Limit the number of albums to fetch to prevent spamming
        const limitedIds = albumIdsToFetch.slice(0, maxRequests);
        if (albumIdsToFetch.length > maxRequests) {
            console.warn(`[Enrich] Too many albums to fetch (${albumIdsToFetch.length}). limiting to ${maxRequests}.`);
        }

        const albumDateMap = new Map();

        // Chunk requests to avoid spamming
        const chunkSize = 5;
        for (let i = 0; i < limitedIds.length; i += chunkSize) {
            const chunk = limitedIds.slice(i, i + chunkSize);
            const results = await Promise.allSettled(chunk.map((id) => this.getAlbum(id)));

            for (let j = 0; j < results.length; j++) {
                const result = results[j];
                const id = chunk[j];
                if (result.status === 'fulfilled' && result.value.album?.releaseDate) {
                    albumDateMap.set(id, result.value.album.releaseDate);
                }
            }
        }

        return tracks.map((track) => {
            if (!track.album?.releaseDate && track.album?.id && albumDateMap.has(track.album.id)) {
                return { ...track, album: { ...track.album, releaseDate: albumDateMap.get(track.album.id) } };
            }
            return track;
        });
    }

    async enrichTracksWithAlbumCover(tracks, maxRequests = 20) {
        if (!Array.isArray(tracks) || tracks.length === 0) return tracks;

        const albumIdsToFetch = [];
        for (const track of tracks) {
            if (!track?.album?.cover && track?.album?.id && !albumIdsToFetch.includes(track.album.id)) {
                albumIdsToFetch.push(track.album.id);
            }
        }

        if (albumIdsToFetch.length === 0) return tracks;

        const limitedIds = albumIdsToFetch.slice(0, maxRequests);

        const coverMap = new Map();
        const chunkSize = 5;
        for (let i = 0; i < limitedIds.length; i += chunkSize) {
            const chunk = limitedIds.slice(i, i + chunkSize);
            const results = await Promise.allSettled(chunk.map((id) => this.getAlbum(id)));
            for (let j = 0; j < results.length; j++) {
                const r = results[j];
                if (r.status === 'fulfilled' && r.value?.album?.cover) {
                    coverMap.set(chunk[j], r.value.album.cover);
                }
            }
        }

        if (coverMap.size === 0) return tracks;

        return tracks.map((track) => {
            if (!track?.album?.cover && track?.album?.id && coverMap.has(track.album.id)) {
                return { ...track, album: { ...track.album, cover: coverMap.get(track.album.id) } };
            }
            return track;
        });
    }

    async enrichArtistsWithPicture(artists, maxRequests = 10) {
        if (!Array.isArray(artists) || artists.length === 0) return artists;

        const idsToFetch = [];
        for (const artist of artists) {
            if (!artist?.picture && artist?.id && !idsToFetch.includes(artist.id)) {
                idsToFetch.push(artist.id);
            }
        }

        if (idsToFetch.length === 0) return artists;

        const limitedIds = idsToFetch.slice(0, maxRequests);

        const pictureMap = new Map();
        const chunkSize = 5;
        for (let i = 0; i < limitedIds.length; i += chunkSize) {
            const chunk = limitedIds.slice(i, i + chunkSize);
            const results = await Promise.allSettled(chunk.map((id) => this.getArtist(id, { lightweight: true })));
            for (let j = 0; j < results.length; j++) {
                const r = results[j];
                if (r.status === 'fulfilled' && r.value?.picture) {
                    pictureMap.set(chunk[j], r.value.picture);
                }
            }
        }

        if (pictureMap.size === 0) return artists;

        return artists.map((artist) => {
            if (!artist?.picture && artist?.id && pictureMap.has(artist.id)) {
                return { ...artist, picture: pictureMap.get(artist.id) };
            }
            return artist;
        });
    }

    parseTrackLookup(data) {
        const entries = Array.isArray(data) ? data : [data];
        let track, info, originalTrackUrl;

        for (const entry of entries) {
            if (!entry || typeof entry !== 'object') continue;

            if (!track && 'duration' in entry) {
                track = entry;
                continue;
            }

            if (!info && 'manifest' in entry) {
                info = entry;
                continue;
            }

            if (!originalTrackUrl && 'OriginalTrackUrl' in entry) {
                const candidate = entry.OriginalTrackUrl;
                if (typeof candidate === 'string') {
                    originalTrackUrl = candidate;
                }
            }
        }

        if (!track || !info) {
            throw new Error('Malformed track response');
        }

        return { track, info, originalTrackUrl };
    }

    extractStreamUrlFromManifest(manifest) {
        if (!manifest) return null;

        try {
            let decoded;
            if (typeof manifest === 'string') {
                try {
                    decoded = atob(manifest);
                } catch {
                    decoded = manifest;
                }
            } else if (typeof manifest === 'object') {
                if (manifest.urls && Array.isArray(manifest.urls)) {
                    const priorityKeywords = ['flac', 'lossless', 'hi-res', 'high'];
                    const sortedUrls = [...manifest.urls].sort((a, b) => {
                        const aLow = String(a ?? '').toLowerCase();
                        const bLow = String(b ?? '').toLowerCase();
                        const aScore = priorityKeywords.findIndex((k) => aLow.includes(k));
                        const bScore = priorityKeywords.findIndex((k) => bLow.includes(k));

                        const finalAScore = aScore === -1 ? 999 : aScore;
                        const finalBScore = bScore === -1 ? 999 : bScore;

                        return finalAScore - finalBScore;
                    });
                    return sortedUrls[0];
                }
                if (manifest.urls?.[0]) return manifest.urls[0];
                return null;
            } else {
                return null;
            }

            // Check if it's a DASH manifest (XML)
            if (decoded.includes('<MPD')) {
                const blob = new Blob([decoded], { type: 'application/dash+xml' });
                return URL.createObjectURL(blob);
            }

            try {
                const parsed = JSON.parse(decoded);
                if (parsed?.urls && Array.isArray(parsed.urls)) {
                    const priorityKeywords = ['flac', 'lossless', 'hi-res', 'high'];
                    const sortedUrls = [...parsed.urls].sort((a, b) => {
                        const aLow = String(a ?? '').toLowerCase();
                        const bLow = String(b ?? '').toLowerCase();
                        const aScore = priorityKeywords.findIndex((k) => aLow.includes(k));
                        const bScore = priorityKeywords.findIndex((k) => bLow.includes(k));
                        const finalAScore = aScore === -1 ? 999 : aScore;
                        const finalBScore = bScore === -1 ? 999 : bScore;
                        return finalAScore - finalBScore;
                    });
                    return sortedUrls[0];
                }
                if (parsed?.urls?.[0]) {
                    return parsed.urls[0];
                }
            } catch {
                const match = decoded.match(/https?:\/\/[\w\-.~:?#[@!$&'()*+,;=%/]+/);
                return match ? match[0] : null;
            }
        } catch (error) {
            console.error('Failed to decode manifest:', error);
            return null;
        }
    }

    deduplicateAlbums(albums) {
        const unique = new Map();

        for (const album of albums) {
            // Key based on title and numberOfTracks (excluding duration and explicit)
            const key = JSON.stringify([
                album.title,
                album.numberOfTracks || 0,
                album.artist?.id ?? album.artists?.[0]?.id ?? '',
            ]);

            if (unique.has(key)) {
                const existing = unique.get(key);

                // Priority 1: Explicit
                if (album.explicit && !existing.explicit) {
                    unique.set(key, album);
                    continue;
                }
                if (!album.explicit && existing.explicit) {
                    continue;
                }

                // Priority 2: More Metadata Tags (if explicit status is same)
                const existingTags = existing.mediaMetadata?.tags?.length || 0;
                const newTags = album.mediaMetadata?.tags?.length || 0;

                if (newTags > existingTags) {
                    unique.set(key, album);
                }
            } else {
                unique.set(key, album);
            }
        }

        return Array.from(unique.values());
    }

    async search(query, options = {}) {
        const cached = await this.cache.get('search_all', query);
        if (cached) return cached;

        if (!this._combinedSearchUnavailable) {
            try {
                const response = await this.fetchWithRetry(`/search/?q=${encodeURIComponent(query)}`, options);
                const data = await response.json();

                const extractSection = (key) => this.normalizeSearchResponse(data, key);

                const tracksData = extractSection('tracks');
                const artistsData = extractSection('artists');
                const albumsData = extractSection('albums');
                const playlistsData = extractSection('playlists');
                const videosData = extractSection('videos');

                const preparedTracks = tracksData.items.map((t) => this.prepareTrack(t));
                const preparedArtists = artistsData.items.map((a) => this.prepareArtist(a));

                const [enrichedTracks, enrichedArtists] = await Promise.all([
                    this.enrichTracksWithAlbumCover(preparedTracks),
                    options.enrichArtists === false
                        ? Promise.resolve(preparedArtists)
                        : this.enrichArtistsWithPicture(preparedArtists),
                ]);

                const results = {
                    tracks: {
                        ...tracksData,
                        items: enrichedTracks,
                    },
                    artists: {
                        ...artistsData,
                        items: enrichedArtists,
                    },
                    albums: {
                        ...albumsData,
                        items: albumsData.items.map((a) => this.prepareAlbum(a)),
                    },
                    playlists: playlistsData
                        ? {
                              ...playlistsData,
                              items: playlistsData.items.map((p) => this.preparePlaylist(p)),
                          }
                        : { items: [], limit: 0, offset: 0, totalNumberOfItems: 0 },
                    videos: {
                        ...videosData,
                        items: videosData.items.map((v) => this.prepareTrack(v)),
                    },
                };

                await this.cache.set('search_all', query, results);

                return results;
            } catch (error) {
                if (error.name === 'AbortError') throw error;

                this._combinedSearchUnavailable = true;

                if (import.meta.env.DEV) {
                    console.warn('[search] combined search failed, using HiFi scoped fallback', error);
                }
            }
        }

        const [tracks, videos, artists, albums, playlists] = await Promise.all([
            this.searchTracks(query, options).catch(() => ({ items: [] })),
            this.searchVideos(query, options).catch(() => ({ items: [] })),
            this.searchArtists(query, options).catch(() => ({ items: [] })),
            this.searchAlbums(query, options).catch(() => ({ items: [] })),
            this.searchPlaylists(query, options).catch(() => ({ items: [] })),
        ]);

        return {
            tracks,
            videos,
            artists,
            albums,
            playlists,
        };
    }

    async searchTracks(query, options = {}) {
        const cached = await this.cache.get('search_tracks', query);
        if (cached) return cached;

        try {
            const response = await this.fetchWithRetry(`/search/?s=${encodeURIComponent(query)}`, options);
            const data = await response.json();
            const normalized = this.normalizeSearchResponse(data, 'tracks');
            const preparedTracks = normalized.items.map((t) => this.prepareTrack(t));
            const enrichedTracks = await this.enrichTracksWithAlbumCover(preparedTracks);
            const result = {
                ...normalized,
                items: enrichedTracks,
            };

            await this.cache.set('search_tracks', query, result);
            return result;
        } catch (error) {
            if (error.name === 'AbortError') throw error;
            console.error('Track search failed:', error);
            return { items: [], limit: 0, offset: 0, totalNumberOfItems: 0 };
        }
    }

    async searchTracksByIsrc(isrc, options = {}) {
        const normalizedIsrc = String(isrc || '').trim();
        if (!normalizedIsrc) return { items: [], limit: 0, offset: 0, totalNumberOfItems: 0 };

        const cacheKey = `isrc:${normalizedIsrc}`;
        const cached = await this.cache.get('search_tracks', cacheKey);
        if (cached) return cached;

        try {
            const response = await this.fetchWithRetry(`/search/?i=${encodeURIComponent(normalizedIsrc)}`, options);
            const data = await response.json();
            const normalized = this.normalizeSearchResponse(data, 'tracks');
            const preparedTracks = normalized.items.map((t) => this.prepareTrack(t));
            const enrichedTracks = await this.enrichTracksWithAlbumCover(preparedTracks);
            const result = {
                ...normalized,
                items: enrichedTracks,
            };

            await this.cache.set('search_tracks', cacheKey, result);
            return result;
        } catch (error) {
            if (error.name === 'AbortError') throw error;
            console.error('ISRC track search failed:', error);
            return { items: [], limit: 0, offset: 0, totalNumberOfItems: 0 };
        }
    }

    async searchArtists(query, options = {}) {
        const cached = await this.cache.get('search_artists', query);
        if (cached) return cached;

        try {
            const response = await this.fetchWithRetry(`/search/?a=${encodeURIComponent(query)}`, options);
            const data = await response.json();
            const normalized = this.normalizeSearchResponse(data, 'artists');
            const preparedArtists = normalized.items.map((a) => this.prepareArtist(a));
            const enrichedArtists = await this.enrichArtistsWithPicture(preparedArtists);
            const result = {
                ...normalized,
                items: enrichedArtists,
            };

            await this.cache.set('search_artists', query, result);
            return result;
        } catch (error) {
            if (error.name === 'AbortError') throw error;
            console.error('Artist search failed:', error);
            return { items: [], limit: 0, offset: 0, totalNumberOfItems: 0 };
        }
    }

    async searchAlbums(query, options = {}) {
        const cached = await this.cache.get('search_albums', query);
        if (cached) return cached;

        try {
            const response = await this.fetchWithRetry(`/search/?al=${encodeURIComponent(query)}`, options);
            const data = await response.json();
            const normalized = this.normalizeSearchResponse(data, 'albums');
            const preparedItems = normalized.items.map((a) => this.prepareAlbum(a));
            const result = {
                ...normalized,
                items: this.deduplicateAlbums(preparedItems),
            };

            await this.cache.set('search_albums', query, result);
            return result;
        } catch (error) {
            if (error.name === 'AbortError') throw error;
            console.error('Album search failed:', error);
            return { items: [], limit: 0, offset: 0, totalNumberOfItems: 0 };
        }
    }

    async searchPlaylists(query, options = {}) {
        const cached = await this.cache.get('search_playlists', query);
        if (cached) return cached;

        try {
            const response = await this.fetchWithRetry(`/search/?p=${encodeURIComponent(query)}`, options);
            const data = await response.json();
            const normalized = this.normalizeSearchResponse(data, 'playlists');
            const result = {
                ...normalized,
                items: normalized.items.map((p) => this.preparePlaylist(p)),
            };

            await this.cache.set('search_playlists', query, result);
            return result;
        } catch (error) {
            if (error.name === 'AbortError') throw error;
            console.error('Playlist search failed:', error);
            return { items: [], limit: 0, offset: 0, totalNumberOfItems: 0 };
        }
    }

    async searchVideos(query, options = {}) {
        const cached = await this.cache.get('search_videos', query);
        if (cached) return cached;

        try {
            const response = await this.fetchWithRetry(`/search/?v=${encodeURIComponent(query)}`, {
                ...options,
            });
            const data = await response.json();
            const normalized = this.normalizeSearchResponse(data, 'videos');
            const result = {
                ...normalized,
                items: normalized.items.map((v) => this.prepareVideo(v)),
            };

            await this.cache.set('search_videos', query, result);
            return result;
        } catch (error) {
            if (error.name === 'AbortError') throw error;
            console.error('Video search failed:', error);
            return { items: [], limit: 0, offset: 0, totalNumberOfItems: 0 };
        }
    }

    async getVideo(id) {
        const cached = await this.cache.get('video', id);
        if (cached) return cached;

        const response = await this.fetchWithRetry(`/video/?id=${id}`, {
            type: 'streaming',
        });
        const jsonResponse = await response.json();

        const data = jsonResponse.data || jsonResponse;

        const result = {
            track: data,
            info: data,
            originalTrackUrl: data.OriginalTrackUrl || null,
        };

        await this.cache.set('video', id, result);
        return result;
    }

    async getAlbum(id) {
        const cached = await this.cache.get('album', id);
        if (cached) return cached;

        const response = await this.fetchWithRetry(`/album/?id=${id}`);
        const jsonData = await response.json();

        // Unwrap the data property if it exists
        const data = jsonData.data || jsonData;

        let album, tracksSection;

        if (data && typeof data === 'object' && !Array.isArray(data)) {
            // Check for album metadata at root level
            if ('numberOfTracks' in data || 'title' in data) {
                album = this.prepareAlbum(data);
            }

            // Set tracksSection if items exist
            if ('items' in data) {
                tracksSection = data;

                // If we still don't have album but have items with tracks, try to extract album from first track
                if (!album && data.items && data.items.length > 0) {
                    const firstItem = data.items[0];
                    const track = firstItem.item || firstItem;

                    // Check if track has album property
                    if (track && track.album) {
                        album = this.prepareAlbum(track.album);
                    }
                }
            }
        }

        if (!album) throw new Error('Album not found');

        // If album exists but has no artist, try to extract from tracks
        if (!album.artist && tracksSection?.items && tracksSection.items.length > 0) {
            const firstTrack = tracksSection.items[0];
            const track = firstTrack.item || firstTrack;
            if (track && track.artist) {
                album = { ...album, artist: track.artist };
            }
        }

        // If album exists but has no releaseDate, try to extract from tracks
        if (!album.releaseDate && tracksSection?.items && tracksSection.items.length > 0) {
            const firstTrack = tracksSection.items[0];
            const track = firstTrack.item || firstTrack;

            if (track) {
                if (track.album && track.album.releaseDate) {
                    album = { ...album, releaseDate: track.album.releaseDate };
                } else if (track.streamStartDate) {
                    album = { ...album, releaseDate: track.streamStartDate.split('T')[0] };
                }
            }
        }

        let tracks = (tracksSection?.items || []).map((i) => this.prepareTrack(i.item || i));

        // Handle pagination if there are more tracks
        if (album && album.numberOfTracks > tracks.length) {
            let offset = tracks.length;
            const SAFE_MAX_TRACKS = 10000;

            while (tracks.length < album.numberOfTracks && tracks.length < SAFE_MAX_TRACKS) {
                try {
                    const nextResponse = await this.fetchWithRetry(`/album/?id=${id}&offset=${offset}&limit=500`);
                    const nextJson = await nextResponse.json();
                    const nextData = nextJson.data || nextJson;

                    let nextItems = [];

                    if (nextData.items) {
                        nextItems = nextData.items;
                    } else if (Array.isArray(nextData)) {
                        for (const entry of nextData) {
                            if (entry && typeof entry === 'object' && 'items' in entry && Array.isArray(entry.items)) {
                                nextItems = entry.items;
                                break;
                            }
                        }
                    }

                    if (!nextItems || nextItems.length === 0) break;

                    const preparedItems = nextItems.map((i) => this.prepareTrack(i.item || i));
                    if (preparedItems.length === 0) break;

                    // Safeguard: If API ignores offset, it returns the first page again.
                    // Check if the first new item matches the very first track we have.
                    if (tracks.length > 0 && preparedItems[0].id === tracks[0].id) {
                        break;
                    }

                    // Also check if the first new item matches the last track we have (overlap check)
                    if (tracks.length > 0 && preparedItems[0].id === tracks[tracks.length - 1].id) {
                        // If it's just one overlap, maybe we should skip it?
                        // But usually offset should be precise.
                        // If we see exact same id as first track, it's definitely a loop.
                    }

                    tracks = tracks.concat(preparedItems);
                    offset += preparedItems.length;
                } catch (error) {
                    console.error(`Error fetching album tracks at offset ${offset}:`, error);
                    break;
                }
            }
        }

        // Enrich tracks with album releaseDate if available
        if (album?.releaseDate) {
            tracks = tracks.map((track) => {
                if (track.album && !track.album.releaseDate) {
                    return { ...track, album: { ...track.album, releaseDate: album.releaseDate } };
                }
                return track;
            });
        }

        tracks = tracks.map((t) => {
            if (t.album) {
                // Propagate the parent album's cover to each track's album sub-object when
                // the API omits it in the per-track album object (common for album endpoints).
                t.album = new TrackAlbum({
                    ...t.album,
                    cover: t.album.cover || album.cover,
                });
            }

            return new Track(t);
        });

        album = new Album(album);

        const result = { album, tracks };

        await this.cache.set('album', id, result);
        return result;
    }

    async getPlaylist(id) {
        const cached = await this.cache.get('playlist', id);
        if (cached) return cached;

        const response = await this.fetchWithRetry(`/playlist/?id=${id}`);
        const jsonData = await response.json();

        // Unwrap the data property if it exists
        const data = jsonData.data || jsonData;

        let playlist = null;
        let tracksSection = null;

        // Check for direct playlist property (common in v2 responses)
        if (data.playlist) {
            playlist = data.playlist;
        }

        // Check for direct items property
        if (data.items) {
            tracksSection = { items: data.items };
        }

        // Fallback: iterate if we still missed something or if structure is flat array
        if (!playlist || !tracksSection) {
            const entries = Array.isArray(data) ? data : [data];
            for (const entry of entries) {
                if (!entry || typeof entry !== 'object') continue;

                if (
                    !playlist &&
                    ('uuid' in entry || 'numberOfTracks' in entry || ('title' in entry && 'id' in entry))
                ) {
                    playlist = entry;
                }

                if (!tracksSection && 'items' in entry) {
                    tracksSection = entry;
                }
            }
        }

        // Fallback 2: If we have a list of entries but no explicit playlist object, try to find one that looks like a playlist
        if (!playlist && Array.isArray(data)) {
            for (const entry of data) {
                if (entry && typeof entry === 'object' && ('uuid' in entry || 'numberOfTracks' in entry)) {
                    playlist = entry;
                    break;
                }
            }
        }

        if (!playlist) throw new Error('Playlist not found');

        let tracks = (tracksSection?.items || []).map((i) => this.prepareTrack(i.item || i));

        // Handle pagination if there are more tracks
        if (playlist.numberOfTracks > tracks.length) {
            let offset = tracks.length;
            const SAFE_MAX_TRACKS = 10000;

            while (tracks.length < playlist.numberOfTracks && tracks.length < SAFE_MAX_TRACKS) {
                try {
                    const nextResponse = await this.fetchWithRetry(`/playlist/?id=${id}&offset=${offset}`);
                    const nextJson = await nextResponse.json();
                    const nextData = nextJson.data || nextJson;

                    let nextItems = [];

                    if (nextData.items) {
                        nextItems = nextData.items;
                    } else if (Array.isArray(nextData)) {
                        for (const entry of nextData) {
                            if (entry && typeof entry === 'object' && 'items' in entry && Array.isArray(entry.items)) {
                                nextItems = entry.items;
                                break;
                            }
                        }
                    }

                    if (!nextItems || nextItems.length === 0) break;

                    const preparedItems = nextItems.map((i) => this.prepareTrack(i.item || i));
                    if (preparedItems.length === 0) break;

                    // Safeguard: If API ignores offset, it returns the first page again.
                    // Check if the first new item matches the very first track we have.
                    if (tracks.length > 0 && preparedItems[0].id === tracks[0].id) {
                        break;
                    }

                    tracks = tracks.concat(preparedItems);
                    offset += preparedItems.length;
                } catch (error) {
                    console.error(`Error fetching playlist tracks at offset ${offset}:`, error);
                    break;
                }
            }
        }

        // Enrich tracks with album release dates
        // Removed to reduce API load. Playlists can be very large.
        // tracks = await this.enrichTracksWithAlbumDates(tracks);

        tracks = tracks.map((t) => {
            if (t.album) {
                t.album = new TrackAlbum(t.album);
            }

            return new Track(t);
        });

        const result = { playlist, tracks };

        await this.cache.set('playlist', id, result);
        return result;
    }

    async getMix(id) {
        const cached = await this.cache.get('mix', id);
        if (cached) return cached;

        const response = await this.fetchWithRetry(`/mix/?id=${id}`, { type: 'api', minVersion: '2.3' });
        const data = await response.json();

        const mixData = data.mix;
        const items = data.items || [];

        if (!mixData) {
            throw new Error('Mix metadata not found');
        }

        let tracks = items.map((i) => this.prepareTrack(i.item || i));

        // Enrich tracks with album release dates
        // Limited to reduce API load
        tracks = await this.enrichTracksWithAlbumDates(tracks, 10);

        tracks = tracks.map((t) => {
            if (t.album) {
                t.album = new TrackAlbum(t.album);
            }

            return new Track(t);
        });

        const mix = {
            id: mixData.id,
            title: mixData.title,
            subTitle: mixData.subTitle,
            description: mixData.description,
            mixType: mixData.mixType,
            cover: mixData.images?.LARGE?.url || mixData.images?.MEDIUM?.url || mixData.images?.SMALL?.url || null,
        };

        const result = { mix, tracks };
        await this.cache.set('mix', id, result);
        return result;
    }

    /**
     * Artist links came from MusicBrainz, called straight from the webview.
     * Volna sends all traffic through Rust (and its proxy), so there are none.
     */
    async getArtistSocials(_artistName) {
        return [];
    }

    async getArtist(artistId, options = {}) {
        const cacheKey = options.lightweight ? `artist_${artistId}_light` : `artist_${artistId}`;
        if (!options.skipCache) {
            const cached = await this.cache.get('artist', cacheKey);
            if (cached) return cached;
        }

        const primaryResponse = await this.fetchWithRetry(`/artist/?id=${artistId}`);
        const primaryJsonData = await primaryResponse.json();

        // Unwrap data property if it exists, then unwrap artist property if it exists
        let primaryData = primaryJsonData.data || primaryJsonData;
        const rawArtist = primaryData.artist || (Array.isArray(primaryData) ? primaryData[0] : primaryData);

        if (!rawArtist) throw new Error('Primary artist details not found.');

        const artist = {
            ...this.prepareArtist(rawArtist),
            picture: rawArtist.picture || null,
            name: rawArtist.name || 'Unknown Artist',
        };

        const albumMap = new Map();
        const trackMap = new Map();
        const videoMap = new Map();

        const isTrack = (v) => v?.id && (v.duration || v.trackNumber != null || v.type === 'track');
        const isAlbum = (v) =>
            v?.id && ('numberOfTracks' in v || 'numberOfItems' in v || v.type === 'album' || v.type === 'ALBUM');
        const isVideo = (v) => v?.id && (!!v.type?.toLowerCase().includes('video') || v.type === 'VIDEO');

        const scan = (value, visited) => {
            if (!value || typeof value !== 'object' || visited.has(value)) return;
            visited.add(value);

            if (Array.isArray(value)) {
                value.forEach((item) => scan(item, visited));
                return;
            }

            const item = value.item || value;
            const type = (item.type || '').toLowerCase();

            if (isAlbum(item) || type === 'album') albumMap.set(item.id, this.prepareAlbum(item));
            if ((isTrack(item) || type === 'track') && !isAlbum(item) && !isVideo(item)) {
                trackMap.set(item.id, this.prepareTrack(item));
            }
            if (isVideo(item) || type === 'video') videoMap.set(item.id, this.prepareVideo(item));

            Object.values(value).forEach((nested) => scan(nested, visited));
        };

        const visited = new Set();
        scan(primaryData, visited);

        if (albumMap.size === 0) {
            try {
                if (import.meta.env.DEV) {
                    console.log('No albums in primary response, trying fallback fetch');
                }
                const albumsResponse = await this.fetchWithRetry(`/artist/?f=${artistId}&skip_tracks=true`);
                const albumsData = await albumsResponse.json();
                scan(albumsData, visited);
            } catch (e) {
                console.warn('Fallback album fetch failed:', e);
            }
        }

        const matchesArtistId = (item) => {
            const candidateIds = [
                item.artistId,
                item.artist_id,
                item.artist?.id,
                ...(Array.isArray(item.artists) ? item.artists.map((a) => a.id) : []),
                ...(Array.isArray(item.artistRoles) ? item.artistRoles.map((r) => r.artist?.id) : []),
            ].filter((id) => id != null);

            if (item.artist && (typeof item.artist === 'number' || typeof item.artist === 'string')) {
                candidateIds.push(item.artist);
            }

            return candidateIds.some((id) => String(id) === String(artist.id) || String(id) === String(artistId));
        };

        const rawReleases = Array.from(albumMap.values()).filter(matchesArtistId);
        const allReleases = this.deduplicateAlbums(rawReleases).sort(
            (a, b) => new Date(b.releaseDate || 0) - new Date(a.releaseDate || 0)
        );

        const eps = allReleases.filter((a) => a.type === 'EP' || a.type === 'SINGLE');
        const albums = allReleases.filter((a) => !eps.includes(a));

        const topTracks = Array.from(trackMap.values())
            .filter(matchesArtistId)
            .sort((a, b) => (b.popularity || 0) - (a.popularity || 0))
            .slice(0, 15);

        const videos = Array.from(videoMap.values()).sort(
            (a, b) => new Date(b.releaseDate || 0) - new Date(a.releaseDate || 0)
        );

        // Enrich tracks with album release dates
        const tracks = options.lightweight ? topTracks : await this.enrichTracksWithAlbumDates(topTracks);

        const result = { ...artist, albums, eps, tracks, videos };

        await this.cache.set('artist', cacheKey, result);
        return result;
    }

    async getArtistTopTracks(artistId, options = {}) {
        const offset = options.offset || 0;
        const limit = options.limit || 15;
        console.log('[getArtistTopTracks] Called:', { artistId, offset, limit, options });

        const cacheKey = `artist_tracks_${artistId}_${offset}_${limit}`;
        if (!options.skipCache) {
            const cached = await this.cache.get('artist', cacheKey);
            if (cached) return cached;
        }

        try {
            // Use f parameter with skip_tracks=true to get toptracks from the dedicated endpoint
            const response = await this.fetchWithRetry(
                `/artist/?f=${artistId}&skip_tracks=true&offset=${offset}&limit=${limit}`
            );
            const jsonData = await response.json();

            let data = jsonData.data || jsonData;
            console.log(
                '[getArtistTopTracks] Raw response data keys:',
                Object.keys(data),
                'tracks:',
                data.tracks?.length
            );

            // Extract tracks from the response
            let tracks = [];

            // Check for tracks array directly (from toptracks endpoint)
            if (Array.isArray(data.tracks)) {
                tracks = data.tracks;
            }

            // Also scan for tracks in the data structure
            if (tracks.length === 0) {
                const trackMap = new Map();
                const isTrack = (v) => v?.id && v.duration;

                const scan = (value, visited) => {
                    if (!value || typeof value !== 'object' || visited.has(value)) return;
                    visited.add(value);

                    if (Array.isArray(value)) {
                        value.forEach((item) => scan(item, visited));
                        return;
                    }

                    const item = value.item || value;
                    if (isTrack(item)) {
                        trackMap.set(item.id, this.prepareTrack(item));
                    }

                    Object.values(value).forEach((nested) => scan(nested, visited));
                };

                const visited = new Set();
                scan(data, visited);
                tracks = Array.from(trackMap.values());
            }

            tracks = tracks.map((t) => this.prepareTrack(t)).sort((a, b) => (b.popularity || 0) - (a.popularity || 0));
            tracks = await this.enrichTracksWithAlbumDates(tracks);

            // Safeguard: If API ignores offset, it returns the same first tracks
            const hasMore = tracks.length === limit && (offset === 0 || tracks[0]?.id !== options.firstTrackId);
            const result = {
                tracks,
                offset,
                limit,
                hasMore,
            };

            await this.cache.set('artist', cacheKey, result);
            return result;
        } catch (e) {
            console.warn('Failed to fetch artist top tracks:', e);
            return { tracks: [], offset, limit, hasMore: false };
        }
    }

    async getSimilarArtists(artistId) {
        const cached = await this.cache.get('similar_artists', artistId);
        if (cached) return cached;

        try {
            const response = await this.fetchWithRetry(`/artist/similar/?id=${artistId}`, {
                type: 'api',
                minVersion: '2.3',
            });
            const data = await response.json();

            // Handle various response structures
            const items = data.artists || data.items || data.data || (Array.isArray(data) ? data : []);

            const result = items.map((artist) => this.prepareArtist(artist));

            await this.cache.set('similar_artists', artistId, result);
            return result;
        } catch (e) {
            console.warn('Failed to fetch similar artists:', e);
            return [];
        }
    }

    async getArtistBiography(artistId) {
        const cacheKey = `artist_bio_v1_${artistId}`;
        const cached = await this.cache.get('artist', cacheKey);
        if (cached) return cached;

        try {
            const response = await this.fetchWithRetry(`/artist/bio/?id=${artistId}`, { type: 'api' });

            if (response.ok) {
                const { data } = await response.json();
                if (data && data.text) {
                    const bio = {
                        text: data.text,
                        source: data.source || 'YouTube Music',
                    };
                    await this.cache.set('artist', cacheKey, bio);
                    return bio;
                }
            }
        } catch (e) {
            console.warn('Failed to fetch artist biography:', e);
        }
        return null;
    }

    async getSimilarAlbums(albumId) {
        const cached = await this.cache.get('similar_albums', albumId);
        if (cached) return cached;

        try {
            const response = await this.fetchWithRetry(`/album/similar/?id=${albumId}`, {
                type: 'api',
                minVersion: '2.3',
            });
            const data = await response.json();

            const items = data.items || data.albums || data.data || (Array.isArray(data) ? data : []);

            const result = items.map((album) => this.prepareAlbum(album));

            await this.cache.set('similar_albums', albumId, result);
            return result;
        } catch (e) {
            console.warn('Failed to fetch similar albums:', e);
            return [];
        }
    }

    async getRecommendedTracksForPlaylist(tracks, limit = 20, options = {}) {
        if (!tracks || tracks.length === 0) return [];

        const seenTrackIds = new Set(tracks.map((t) => t.id));
        const recommendedTracks = [];

        const seedTracks = tracks.slice(0, 3);
        for (const seed of seedTracks) {
            if (!seed?.id) continue;
            try {
                const recs = await this.getTrackRecommendations(seed.id);
                for (const t of recs) {
                    if (t?.id && !seenTrackIds.has(t.id)) {
                        seenTrackIds.add(t.id);
                        recommendedTracks.push(t);
                    }
                }
            } catch (e) {
                console.warn(`Failed to get mix recommendations for track ${seed.id}:`, e);
            }
        }

        if (recommendedTracks.length === 0) {
            const artistMap = new Map();
            for (const track of tracks) {
                const artists = track.artists || (track.artist ? [track.artist] : []);
                for (const artist of artists) {
                    if (artist?.id) artistMap.set(artist.id, artist);
                }
            }
            const artistsToProcess = Array.from(artistMap.values()).slice(0, 10);
            for (const artist of artistsToProcess) {
                try {
                    const artistData = await this.getArtist(artist.id, {
                        lightweight: true,
                        skipCache: options.refresh,
                    });
                    if (artistData?.tracks) {
                        for (const t of artistData.tracks) {
                            if (t?.id && !seenTrackIds.has(t.id)) {
                                seenTrackIds.add(t.id);
                                recommendedTracks.push(this.prepareTrack(t));
                            }
                        }
                    }
                } catch (e) {}
            }
        }

        const shuffled = recommendedTracks.sort(() => 0.5 - Math.random());
        const sliced = shuffled.slice(0, limit);
        return this.enrichTracksWithAlbumCover(sliced);
    }

    async getTrackMetadata(id) {
        const cacheKey = `meta_${id}`;
        const cached = await this.cache.get('track', cacheKey);
        if (cached) return cached;

        const response = await this.fetchWithRetry(`/info/?id=${id}`, { type: 'api' });
        const json = await response.json();
        const data = json.data || json;

        let track;
        const items = Array.isArray(data) ? data : [data];
        const found = items.find((i) => i.id == id || (i.item && i.item.id == id));

        if (found) {
            track = this.prepareTrack(found.item || found);
            await this.cache.set('track', cacheKey, track);
            return track;
        }

        throw new Error('Track metadata not found');
    }

    /** Song radio without the seed (`/recommendations/`, contract §3.3). */
    async getTrackRecommendations(id) {
        const cached = await this.cache.get('recommendations', id);
        if (cached) return cached;

        try {
            const response = await this.fetchWithRetry(`/recommendations/?id=${encodeURIComponent(id)}`);
            const json = await response.json();
            const tracks = (json.data?.items ?? json.items ?? [])
                .map((entry) => entry.item || entry.track || entry)
                .filter((item) => item?.id && String(item.id) !== String(id))
                .map((item) => this.prepareTrack(item));

            await this.cache.set('recommendations', id, tracks);
            return tracks;
        } catch (error) {
            console.error('Failed to fetch recommendations:', error);
            return [];
        }
    }

    /** Metadata only: playback no longer goes through TIDAL manifests. */
    async getTrack(id) {
        const track = await this.getTrackMetadata(id);
        return { track, info: null, originalTrackUrl: null };
    }

    async getStreamUrl(id, quality) {
        return buildStreamDescriptor(id, quality);
    }

    async getVideoStreamUrl(_id) {
        // Music videos play as audio tracks in v1 (contract decision 5).
        return null;
    }

    /** Search box completions (`/search/suggest/`). */
    async searchSuggestions(query, options = {}) {
        if (!query?.trim()) return [];
        try {
            const response = await this.fetchWithRetry(`/search/suggest/?q=${encodeURIComponent(query)}`, options);
            const json = await response.json();
            return Array.isArray(json.data) ? json.data : [];
        } catch (error) {
            if (error.name === 'AbortError') throw error;
            return [];
        }
    }

    /** Plain-text YouTube Music lyrics: `{text, source}` or null. */
    async getLyrics(id) {
        const cached = await this.cache.get('lyrics', id);
        if (cached) return cached;
        try {
            const response = await this.fetchWithRetry(`/lyrics/?id=${encodeURIComponent(id)}`);
            const { data } = await response.json();
            const lyrics = data?.text ? { text: data.text, source: data.source || 'YouTube Music' } : null;
            if (lyrics) await this.cache.set('lyrics', id, lyrics);
            return lyrics;
        } catch {
            return null;
        }
    }

    /** YouTube Music home feed: `{sections: [{type, title, items}]}`. */
    async getHomeFeed() {
        const cached = await this.cache.get('home', 'feed');
        if (cached) return cached;
        const response = await this.fetchWithRetry('/home/');
        const data = await response.json();
        await this.cache.set('home', 'feed', data);
        return data;
    }

    /** Moods & genres grids: `{categories: [{title, genres: [{id, name, color}]}]}`. */
    async getExplore() {
        const cached = await this.cache.get('home', 'explore');
        if (cached) return cached;
        const response = await this.fetchWithRetry('/explore/');
        const data = await response.json();
        await this.cache.set('home', 'explore', data);
        return data;
    }

    async getExploreGenre(id) {
        const response = await this.fetchWithRetry(`/explore/genre/?id=${encodeURIComponent(id)}`);
        return response.json();
    }

    getCoverUrl(id, size = '320') {
        if (!id) {
            return 'images/monochrome_logo.svg';
        }
        return isDirectImage(id) ? resizeImage(id, size) : 'images/monochrome_logo.svg';
    }

    getCoverSrcset() {
        // Covers are single volna:// URLs; a srcset would override `src`.
        return '';
    }

    getArtistPictureUrl(id, size = '320') {
        return this.getCoverUrl(id, size);
    }

    getArtistPictureSrcset() {
        return '';
    }

    getVideoCoverUrl(imageId, size = '1280') {
        return isDirectImage(imageId) ? resizeImage(imageId, size) : null;
    }

    async clearCache() {
        await this.cache.clear();
    }

    getCacheStats() {
        return this.cache.getCacheStats();
    }
}
