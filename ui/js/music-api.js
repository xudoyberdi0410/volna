// js/music-api.js

import { LosslessAPI } from './api.js';
import { musicProviderSettings } from './storage.js';
import { buildStreamDescriptor, isDirectImage, resizeImage } from './volna-media.js';

/**
 * MusicAPI - singleton facade the pages call.
 *
 * In Volna there is a single backend: the Rust `volna://api` emulator over
 * YouTube Music, reached through `LosslessAPI`. Ids are YouTube strings
 * (videoId, MPREb_…, UC…, playlist ids); images are absolute URLs.
 *
 * @example
 * await MusicAPI.initialize(settings);
 * const api = MusicAPI.instance;
 * const results = await api.search('query');
 * const stream = await api.getStreamUrl('dQw4w9WgXcQ', 'HIGH');
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
        this._settings = settings;
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

    getAPI() {
        return this.tidalAPI;
    }

    async canPlayLegacyStream() {
        return true;
    }

    // Search methods
    async search(query, options = {}) {
        return this.tidalAPI.search(query, options);
    }

    async searchTracks(query, options = {}) {
        return this.tidalAPI.searchTracks(query, options);
    }

    async searchArtists(query, options = {}) {
        return this.tidalAPI.searchArtists(query, options);
    }

    async searchAlbums(query, options = {}) {
        return this.tidalAPI.searchAlbums(query, options);
    }

    async searchPlaylists(query, options = {}) {
        return this.tidalAPI.searchPlaylists(query, options);
    }

    async searchVideos(query, options = {}) {
        return this.tidalAPI.searchVideos(query, options);
    }

    /** Completed queries from YouTube Music, as `[{kind:'term', searchTerm, displayTerm}]`. */
    async searchSuggestions(query, options = {}) {
        return this.tidalAPI.searchSuggestions(query, options);
    }

    // Get methods
    async getTrack(id, quality) {
        return this.tidalAPI.getTrack(this.stripProviderPrefix(id), quality);
    }

    async getTrackMetadata(id) {
        return this.tidalAPI.getTrackMetadata(this.stripProviderPrefix(id));
    }

    async getAlbum(id) {
        return this.tidalAPI.getAlbum(this.stripProviderPrefix(id));
    }

    async getArtist(id) {
        return this.tidalAPI.getArtist(this.stripProviderPrefix(id));
    }

    async getArtistBiography(id) {
        return this.tidalAPI.getArtistBiography(this.stripProviderPrefix(id));
    }

    async getVideo(id) {
        return this.tidalAPI.getVideo(this.stripProviderPrefix(id));
    }

    async getVideoStreamUrl(id) {
        return this.tidalAPI.getVideoStreamUrl(this.stripProviderPrefix(id));
    }

    async getArtistSocials(artistName) {
        return this.tidalAPI.getArtistSocials(artistName);
    }

    async getPlaylist(id) {
        return this.tidalAPI.getPlaylist(id);
    }

    async getMix(id) {
        return this.tidalAPI.getMix(id);
    }

    async getTrackRecommendations(id) {
        return this.tidalAPI.getTrackRecommendations(this.stripProviderPrefix(id));
    }

    async getLyrics(id) {
        return this.tidalAPI.getLyrics(this.stripProviderPrefix(id));
    }

    // Stream methods
    /**
     * Native playback descriptor: no API round trip; the Rust audio server
     * resolves the videoId through yt-dlp on the first request.
     */
    async getStreamUrl(id, quality) {
        return buildStreamDescriptor(this.stripProviderPrefix(id), quality);
    }

    usesSingleUsePlaybackUrls() {
        return false;
    }

    clearMonochromePlaybackSession() {}

    // Cover/artwork methods: api_emu always sends absolute volna:// image URLs.
    getCoverUrl(id, size = '320') {
        if (!id) {
            return 'images/monochrome_logo.svg';
        }
        return isDirectImage(id) ? resizeImage(id, size) : 'images/monochrome_logo.svg';
    }

    getCoverSrcset() {
        return '';
    }

    getVideoCoverUrl(imageId, size = '1280') {
        return isDirectImage(imageId) ? resizeImage(imageId, size) : null;
    }

    async getVideoArtwork(_title, _artist) {
        // Animated covers came from Apple Music, which Volna does not use.
        return null;
    }

    getArtistPictureUrl(id, size = '320') {
        return this.getCoverUrl(id, size);
    }

    getArtistPictureSrcset() {
        return '';
    }

    async getArtistBanner(_artistName) {
        // The animated banners came from a third-party worker.
        return null;
    }

    extractStreamUrlFromManifest(manifest) {
        return this.tidalAPI.extractStreamUrlFromManifest(manifest);
    }

    // Helper methods
    getProviderFromId() {
        return null;
    }

    stripProviderPrefix(id) {
        if (typeof id === 'string' && (id.startsWith('q:') || id.startsWith('t:'))) {
            return id.slice(2);
        }
        return id;
    }

    isTracksId() {
        return false;
    }

    getTracksId(id) {
        return typeof id === 'string' ? id : String(id || '');
    }

    getCachedTracksTrack() {
        return null;
    }

    // Similar/recommendation methods
    async getSimilarArtists(artistId) {
        return this.tidalAPI.getSimilarArtists(this.stripProviderPrefix(artistId));
    }

    async getArtistTopTracks(artistId, options = {}) {
        return this.tidalAPI.getArtistTopTracks(artistId, options);
    }

    async getSimilarAlbums(albumId) {
        return this.tidalAPI.getSimilarAlbums(this.stripProviderPrefix(albumId));
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

    // Home / explore (YouTube Music feed and moods & genres)
    async getHomeFeed() {
        return this.tidalAPI.getHomeFeed();
    }

    async getExplore() {
        return this.tidalAPI.getExplore();
    }

    async getExploreGenre(id) {
        return this.tidalAPI.getExploreGenre(id);
    }

    // Cache methods
    async clearCache() {
        await this.tidalAPI.clearCache();
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
