// js/music-api.js

import { LosslessAPI } from './api.js';
import { musicProviderSettings } from './storage.js';
import { tracksStreamerAPI, TRACKS_API_BASE_URL } from './tracks-api.js';
import { getCommunityPlaylist } from './community-playlists.js';

/**
 * MusicAPI - Singleton class that provides a unified interface for accessing music streaming services.
 *
 * Supports multiple providers (primarily Tidal) and includes functionality for searching,
 * retrieving metadata, streaming, and managing playlists, artists, albums and tracks.
 *
 * @class MusicAPI
 * @classdesc Manages API interactions with music providers and provides caching mechanisms
 * for cover artwork and video metadata.
 *
 * @example
 * // Initialize the MusicAPI
 * await MusicAPI.initialize(settings);
 *
 * // Get the singleton instance
 * const api = MusicAPI.instance;
 *
 * // Search for tracks
 * const results = await api.search('query');
 *
 * // Get a specific track
 * const track = await api.getTrack('track-id');
 *
 * // Get stream URL
 * const streamUrl = await api.getStreamUrl('track-id', 'HIGH');
 *
 * @property {LosslessAPI} tidalAPI - The Tidal API instance
 * @property {Object} _settings - Configuration settings
 * @property {Map} videoArtworkCache - Cache for video artwork data
 *
 * @throws {Error} Throws if instance is accessed before initialization
 * @throws {Error} Throws if initialize is called more than once
 */
export class MusicAPI {
    static #instance = null;
    /**
     * @type {MusicAPI}
     */
    static get instance() {
        if (!MusicAPI.#instance) {
            throw new Error('MusicAPI not initialized. Call MusicAPI.initialize(settings) first.');
        }
        return MusicAPI.#instance;
    }

    /** @private */
    constructor(settings) {
        this.tidalAPI = new LosslessAPI(settings);
        this.tracksStreamerAPI = tracksStreamerAPI;
        this._settings = settings;
        this.videoArtworkCache = new Map();
        this.videoArtworkRequests = new Map();
        this.tracksTrackCache = new Map();
        this.tracksArtistCache = new Map();
        this.tracksAlbumCache = new Map();
        this.tracksPlaylistCache = new Map();
        this.tracksEntityRequests = new Map();
        this.tracksArtistIds = new Set();
        this.tracksAlbumIds = new Set();
        this.tracksPlaylistIds = new Set();
    }

    static async initialize(settings) {
        if (MusicAPI.#instance) {
            throw new Error('MusicAPI is already initialized');
        }

        const api = new MusicAPI(settings);
        return (MusicAPI.#instance = api);
    }

    getCurrentProvider() {
        return musicProviderSettings.getProvider();
    }

    // Get the appropriate API based on provider
    getAPI() {
        return this.tidalAPI;
    }

    async canPlayLegacyStream(trackInfo) {
        return this.getAPI().canPlayLegacyStream(trackInfo);
    }

    // Search methods
    async search(query, options = {}) {
        try {
            const tracksResults = await this.tracksStreamerAPI.search(query, options);
            if (
                tracksResults &&
                (tracksResults.tracks?.items?.length ||
                    tracksResults.albums?.items?.length ||
                    tracksResults.artists?.items?.length)
            ) {
                return this.cacheTracksResults(tracksResults);
            }
        } catch (error) {
            if (error.name === 'AbortError') throw error;
            if (import.meta.env.DEV) console.warn('[search] Tracks Streamer unavailable, trying fallbacks', error);
        }

        const api = this.getAPI();
        if (typeof api.search === 'function') return api.search(query, options);
        return this.searchWithCurrentProvider(query, options);
    }

    async searchWithCurrentProvider(query, options = {}) {
        const api = this.getAPI();
        const [tracksResult, videosResult, artistsResult, albumsResult, playlistsResult] = await Promise.all([
            api.searchTracks(query, options),
            api.searchVideos ? api.searchVideos(query, options) : Promise.resolve({ items: [] }),
            api.searchArtists(query, options),
            api.searchAlbums(query, options),
            api.searchPlaylists ? api.searchPlaylists(query, options) : Promise.resolve({ items: [] }),
        ]);

        return {
            tracks: tracksResult,
            videos: videosResult,
            artists: artistsResult,
            albums: albumsResult,
            playlists: playlistsResult,
        };
    }

    async searchTracks(query, options = {}) {
        try {
            const result = await this.tracksStreamerAPI.searchTracks(query, options);
            if (result?.items && result.items.length > 0) {
                this.cacheTracks(result.items);
                return result;
            }
        } catch (error) {
            if (error.name === 'AbortError') throw error;
            if (import.meta.env.DEV)
                console.warn('[searchTracks] Tracks Streamer unavailable, trying fallbacks', error);
        }
        return this.getAPI().searchTracks(query, options);
    }

    async searchArtists(query, options = {}) {
        try {
            const result = await this.tracksStreamerAPI.searchArtists(query, options);
            if (result?.items && result.items.length > 0) {
                for (const artist of result.items) {
                    if (artist.tracksArtistId) this.tracksArtistIds.add(String(artist.tracksArtistId));
                }
                return result;
            }
        } catch (error) {
            if (error.name === 'AbortError') throw error;
            if (import.meta.env.DEV)
                console.warn('[searchArtists] Tracks Streamer unavailable, trying fallbacks', error);
        }
        return this.getAPI().searchArtists(query, options);
    }

    async searchAlbums(query, options = {}) {
        try {
            const result = await this.tracksStreamerAPI.searchAlbums(query, options);
            if (result?.items && result.items.length > 0) {
                for (const album of result.items) {
                    if (album.tracksReleaseId) this.tracksAlbumIds.add(String(album.tracksReleaseId));
                }
                return result;
            }
        } catch (error) {
            if (error.name === 'AbortError') throw error;
            if (import.meta.env.DEV)
                console.warn('[searchAlbums] Tracks Streamer unavailable, trying fallbacks', error);
        }
        return this.getAPI().searchAlbums(query, options);
    }

    async searchPlaylists(query, options = {}) {
        return this.tidalAPI.searchPlaylists(query, options);
    }

    async searchVideos(query, options = {}) {
        return this.tidalAPI.searchVideos(query, options);
    }

    async searchSuggestions(query, options = {}) {
        try {
            const suggestions = await this.tracksStreamerAPI.suggestions(query, options);
            if (suggestions && suggestions.length > 0) return suggestions;
        } catch (error) {
            if (error.name === 'AbortError') throw error;
            if (import.meta.env.DEV) console.warn('[searchSuggestions] Tracks Streamer unavailable', error);
        }
        return [];
    }

    // Get methods
    async getTrack(id, quality) {
        if (this.isTracksId(id, 'track') || this.isTracksId(id)) {
            const track = await this.getTrackMetadata(id);
            return { track, info: track, originalTrackUrl: track?.url || null };
        }
        const tracksTrack = this.getCachedTracksTrack(id);
        if (tracksTrack) return { track: tracksTrack, info: tracksTrack, originalTrackUrl: tracksTrack?.url || null };
        const api = this.getAPI();
        const cleanId = this.stripProviderPrefix(id);
        return api.getTrack(cleanId, quality);
    }

    async getTrackMetadata(id) {
        if (this.isTracksId(id, 'track') || this.isTracksId(id)) {
            const cached = this.getCachedTracksTrack(id);
            if (cached) return cached;
            const tracksId = this.getTracksId(id, 'track');
            const placeholder = {
                id: tracksId,
                trackId: tracksId,
                tracksTrackId: tracksId,
                provider: 'tracks',
                type: 'track',
                title: `Track ${tracksId}`,
                duration: 0,
                playable: true,
                url: `${TRACKS_API_BASE_URL}/track/${tracksId}`,
                _href: `/track/${tracksId}`,
            };
            this.cacheTracks([placeholder]);
            return placeholder;
        }
        const tracksTrack = this.getCachedTracksTrack(id);
        if (tracksTrack) return tracksTrack;
        const api = this.getAPI();
        const cleanId = this.stripProviderPrefix(id);
        return api.getTrackMetadata(cleanId);
    }

    async getAlbum(id, provider = null) {
        if (this.isTracksId(id, 'album', provider) || this.tracksAlbumIds.has(String(id))) {
            const tracksId = this.getTracksId(id, 'album');
            if (this.tracksAlbumCache.has(String(tracksId))) return this.tracksAlbumCache.get(String(tracksId));
            const requestKey = `tracks:album:${tracksId}`;
            if (this.tracksEntityRequests.has(requestKey)) return this.tracksEntityRequests.get(requestKey);
            const request = this.tracksStreamerAPI
                .getAlbum(tracksId)
                .then((result) => {
                    this.tracksAlbumIds.add(String(tracksId));
                    this.tracksAlbumCache.set(String(tracksId), result);
                    if (result.tracks) this.cacheTracks(result.tracks);
                    return result;
                })
                .finally(() => this.tracksEntityRequests.delete(requestKey));
            this.tracksEntityRequests.set(requestKey, request);
            return request;
        }
        const api = this.getAPI();
        const cleanId = this.stripProviderPrefix(id);
        return api.getAlbum(cleanId);
    }

    async getArtist(id, provider = null) {
        if (this.isTracksId(id, 'artist', provider) || this.tracksArtistIds.has(String(id))) {
            const tracksId = this.getTracksId(id, 'artist');
            const cached = this.tracksArtistCache.get(String(tracksId));
            if (cached) return cached;
            const requestKey = `tracks:artist:${tracksId}`;
            if (this.tracksEntityRequests.has(requestKey)) return this.tracksEntityRequests.get(requestKey);
            const request = this.tracksStreamerAPI
                .getArtist(tracksId)
                .then((artist) => {
                    this.tracksArtistIds.add(String(tracksId));
                    this.tracksArtistCache.set(String(tracksId), artist);
                    if (artist.tracks) this.cacheTracks(artist.tracks);
                    for (const album of [...(artist.albums || []), ...(artist.eps || [])]) {
                        if (album.tracksReleaseId) this.tracksAlbumIds.add(String(album.tracksReleaseId));
                    }
                    return artist;
                })
                .finally(() => this.tracksEntityRequests.delete(requestKey));
            this.tracksEntityRequests.set(requestKey, request);
            return request;
        }
        const api = this.getAPI();
        const cleanId = this.stripProviderPrefix(id);
        return api.getArtist(cleanId);
    }

    async getArtistBiography(id) {
        if (this.isTracksId(id, 'artist') || this.tracksArtistIds.has(String(id))) {
            const artist = this.tracksArtistCache.get(String(this.getTracksId(id, 'artist')));
            return artist?.biography || null;
        }
        const api = this.getAPI();
        const cleanId = this.stripProviderPrefix(id);
        if (typeof api.getArtistBiography === 'function') {
            return api.getArtistBiography(cleanId);
        }
        return null;
    }

    async getVideo(id) {
        const api = this.getAPI();
        const cleanId = this.stripProviderPrefix(id);
        return api.getVideo(cleanId);
    }

    async getVideoStreamUrl(id) {
        const api = this.getAPI();
        const cleanId = this.stripProviderPrefix(id);
        if (typeof api.getVideoStreamUrl === 'function') {
            return api.getVideoStreamUrl(cleanId);
        }
    }

    async getArtistSocials(artistName) {
        return this.tidalAPI.getArtistSocials(artistName);
    }

    async getPlaylist(id, provider = null) {
        if (id?.startsWith('VL')) {
            return getCommunityPlaylist(id);
        }

        return this.tidalAPI.getPlaylist(id);
    }

    async getMix(id) {
        // Mixes are always Tidal for now
        return this.tidalAPI.getMix(id);
    }

    async getTrackRecommendations(id) {
        const api = this.getAPI();
        const cleanId = this.stripProviderPrefix(id);
        if (typeof api.getTrackRecommendations === 'function') {
            return api.getTrackRecommendations(cleanId);
        }
        return [];
    }

    // Stream methods
    async getStreamUrl(id, quality, options = {}) {
        let track = options?.track || this.getCachedTracksTrack(id);

        if (!track && (this.isTracksId(id) || /^\d{17,20}$/.test(String(id)))) {
            track = await this.getTrackMetadata(id).catch(() => null);
        }

        const stream = await this.tracksStreamerAPI.resolveTrackStream(id, quality, {
            ...options,
            track,
        });
        if (stream?.url) {
            return stream;
        }

        const cleanId = this.getTracksId(id);
        return this.tracksStreamerAPI.getStreamUrl(cleanId, quality, { track });
    }

    usesSingleUsePlaybackUrls() {
        return this.getAPI().usesSingleUsePlaybackUrls?.() === true;
    }

    clearMonochromePlaybackSession() {
        this.getAPI().clearMonochromePlaybackSession?.();
    }

    // Cover/artwork methods
    getCoverUrl(id, size = '320') {
        if (!id) {
            return 'images/monochrome_logo.svg';
        }
        if (typeof id === 'string' && /^(?:https?:|blob:|data:|assets\/)/.test(id)) {
            return id;
        }
        return this.tidalAPI.getCoverUrl(this.stripProviderPrefix(id), size);
    }

    getCoverSrcset(id) {
        if (typeof id === 'string' && id.startsWith('blob:')) {
            return '';
        }
        return this.tidalAPI.getCoverSrcset(this.stripProviderPrefix(id));
    }

    getVideoCoverUrl(imageId, size = '1280') {
        if (!imageId) {
            return null;
        }
        if (typeof imageId === 'string' && imageId.startsWith('blob:')) {
            return imageId;
        }
        return this.tidalAPI.getVideoCoverUrl(this.stripProviderPrefix(imageId), size);
    }

    async getVideoArtwork(_title, _artist) {
        // Animated covers came from Apple Music, which Volna does not use.
        return null;
    }

    getArtistPictureUrl(id, size = '320') {
        if (!id) {
            return 'images/monochrome_logo.svg';
        }
        if (typeof id === 'string' && /^(?:https?:|blob:|data:|assets\/)/.test(id)) return id;
        return this.tidalAPI.getArtistPictureUrl(this.stripProviderPrefix(id), size);
    }

    getArtistPictureSrcset(id) {
        return this.tidalAPI.getArtistPictureSrcset(this.stripProviderPrefix(id));
    }

    async getArtistBanner(artistName) {
        const cacheKey = `banner-${artistName}`.toLowerCase();
        if (this.videoArtworkCache.has(cacheKey)) {
            return this.videoArtworkCache.get(cacheKey);
        }

        try {
            const url = `https://artwork-boidu-dev.samidy.workers.dev/artist?a=${encodeURIComponent(artistName)}`;
            const response = await fetch(url);
            if (!response.ok) return null;
            const data = await response.json();

            let hlsUrl = null;
            if (data.animated) {
                if (typeof data.animated === 'string') {
                    hlsUrl = data.animated;
                } else if (typeof data.animated === 'object') {
                    hlsUrl = data.animated.hls || data.animated.url || data.animated.hlsUrl || data.animated.videoUrl;

                    if (!hlsUrl) {
                        for (const key in data.animated) {
                            if (typeof data.animated[key] === 'string' && data.animated[key].includes('.m3u8')) {
                                hlsUrl = data.animated[key];
                                break;
                            }
                        }
                    }
                }
            }

            const result = {
                hlsUrl: hlsUrl,
            };
            this.videoArtworkCache.set(cacheKey, result);
            return result;
        } catch (error) {
            console.warn('Failed to fetch artist banner:', error);
            return null;
        }
    }

    extractStreamUrlFromManifest(manifest) {
        return this.tidalAPI.extractStreamUrlFromManifest(manifest);
    }

    // Helper methods
    getProviderFromId(id) {
        if (typeof id === 'string') {
            if (id.startsWith('tracks:') || id.startsWith('mono:')) return 'tracks';
            if (id.startsWith('t:')) return 'tidal';
        }
        return null;
    }

    stripProviderPrefix(id) {
        if (typeof id === 'string') {
            if (id.startsWith('q:') || id.startsWith('t:')) {
                return id.slice(2);
            }
        }
        return id;
    }

    isTracksId(id, type = null, provider = null) {
        if (provider === 'tracks' || provider === 'monochrome') return true;
        if (typeof id !== 'string') return false;
        if (id.startsWith(type ? `tracks:${type}:` : 'tracks:')) return true;
        if (id.startsWith('mono:')) return true;
        if (/^\d{17,20}$/.test(id)) {
            if (type === 'album' && this.tracksAlbumIds.has(id)) return true;
            if (type === 'artist' && this.tracksArtistIds.has(id)) return true;
            if (type === 'track' && this.tracksTrackCache.has(id)) return true;
            if (!type) {
                return this.tracksTrackCache.has(id) || this.tracksAlbumIds.has(id) || this.tracksArtistIds.has(id);
            }
        }
        return false;
    }

    getTracksId(id, type = null) {
        if (typeof id !== 'string') return String(id || '');
        const prefix = type ? `tracks:${type}:` : 'tracks:';
        if (id.startsWith(prefix)) return id.slice(prefix.length);
        if (id.startsWith('tracks:')) return id.replace(/^tracks:[^:]+:/, '');
        if (id.startsWith('mono:')) return id.replace(/^mono:[^:]+:/, '');
        return id;
    }

    cacheTracks(tracks = []) {
        for (const track of tracks) {
            if (!track) continue;
            const tid = String(track.tracksTrackId || track.trackId || track.id || '');
            this.tracksTrackCache.set(String(track.id), track);
            if (tid) this.tracksTrackCache.set(tid, track);
            if (track.album?.tracksReleaseId) {
                this.tracksAlbumIds.add(String(track.album.tracksReleaseId));
            }
            if (track.album?.releaseId) {
                this.tracksAlbumIds.add(String(track.album.releaseId));
            }
            if (track.artist?.tracksArtistId) {
                this.tracksArtistIds.add(String(track.artist.tracksArtistId));
            }
            if (track.artist?.artistId) {
                this.tracksArtistIds.add(String(track.artist.artistId));
            }
        }
        return tracks;
    }

    getCachedTracksTrack(id) {
        if (id == null) return null;
        return (
            this.tracksTrackCache.get(String(id)) ||
            this.tracksTrackCache.get(String(this.getTracksId(id, 'track'))) ||
            this.tracksTrackCache.get(String(this.getTracksId(id)))
        );
    }

    cacheTracksResults(results) {
        this.cacheTracks(results.tracks?.items || []);
        for (const album of results.albums?.items || []) {
            if (album.tracksReleaseId) this.tracksAlbumIds.add(String(album.tracksReleaseId));
            if (album.releaseId) this.tracksAlbumIds.add(String(album.releaseId));
        }
        for (const artist of results.artists?.items || []) {
            if (artist.tracksArtistId) this.tracksArtistIds.add(String(artist.tracksArtistId));
            if (artist.artistId) this.tracksArtistIds.add(String(artist.artistId));
        }
        for (const playlist of results.playlists?.items || []) {
            if (playlist.tracksPlaylistId) this.tracksPlaylistIds.add(String(playlist.tracksPlaylistId));
            if (playlist.playlistId) this.tracksPlaylistIds.add(String(playlist.playlistId));
        }
        return results;
    }

    // Similar/recommendation methods
    async getSimilarArtists(artistId) {
        const api = this.getAPI();
        const cleanId = this.stripProviderPrefix(artistId);
        return api.getSimilarArtists(cleanId);
    }

    async getArtistTopTracks(artistId, options = {}) {
        if (this.isTracksId(artistId, 'artist') || this.tracksArtistIds.has(String(artistId))) {
            const artist = await this.getArtist(this.getTracksId(artistId, 'artist'), 'tracks');
            const offset = options.offset || 0;
            const limit = options.limit || 15;
            const tracks = artist?.tracks || [];
            return {
                tracks: tracks.slice(offset, offset + limit),
                offset,
                limit,
                hasMore: offset + limit < tracks.length,
            };
        }
        return this.tidalAPI.getArtistTopTracks(artistId, options);
    }

    async getSimilarAlbums(albumId) {
        const api = this.getAPI();
        const cleanId = this.stripProviderPrefix(albumId);
        return api.getSimilarAlbums(cleanId);
    }

    async getRecommendedTracksForPlaylist(tracks, limit = 20, options = {}) {
        const excluded = new Set([
            ...tracks.map((track) => String(track.id)),
            ...Array.from(options.knownTrackIds || [], (id) => String(id)),
        ]);
        const recommended = tracks.length
            ? await this.tidalAPI.getRecommendedTracksForPlaylist(tracks, limit, options)
            : [];
        const seen = new Set();
        return recommended
            .filter((track) => {
                const id = String(track?.id || '');
                if (!id || excluded.has(id) || seen.has(id)) return false;
                seen.add(id);
                return true;
            })
            .slice(0, limit);
    }

    // Cache methods
    async clearCache() {
        await this.tidalAPI.clearCache();
        this.tracksStreamerAPI.clearCache();
        this.videoArtworkCache.clear();
        this.videoArtworkRequests.clear();
        this.tracksTrackCache.clear();
        this.tracksArtistCache.clear();
        this.tracksAlbumCache.clear();
        this.tracksPlaylistCache.clear();
        this.tracksEntityRequests.clear();
        this.tracksArtistIds.clear();
        this.tracksAlbumIds.clear();
        this.tracksPlaylistIds.clear();
    }

    getCacheStats() {
        return this.tidalAPI.getCacheStats();
    }

    // Settings accessor for compatibility
    get settings() {
        return this._settings;
    }
}

export const musicAPI = new MusicAPI();
