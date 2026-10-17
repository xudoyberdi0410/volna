import { invoke } from '@tauri-apps/api/core';
import { LastFMScrobbler } from './lastfm.js';
import { ListenBrainzScrobbler } from './listenbrainz.js';
import { MalojaScrobbler } from './maloja.js';
import { LibreFmScrobbler } from './librefm.js';

export class MultiScrobbler {
    constructor() {
        this.lastfm = new LastFMScrobbler();
        this.listenbrainz = new ListenBrainzScrobbler();
        this.maloja = new MalojaScrobbler();
        this.librefm = new LibreFmScrobbler();
        // Services the daemon scrobbles itself (`scrobble_status`): skipped
        // here so a play is not sent twice.
        this.daemonActive = new Set();
        void this.refreshDaemon();
    }

    async refreshDaemon() {
        try {
            const status = await invoke('scrobble_status');
            this.daemonActive = new Set((status?.services || []).filter((s) => s.active).map((s) => s.service));
        } catch {
            this.daemonActive = new Set();
        }
    }

    /** The services this window still scrobbles to. */
    _own(names = ['lastfm', 'listenbrainz', 'maloja', 'librefm']) {
        return names.filter((n) => !this.daemonActive.has(n)).map((n) => this[n]);
    }

    // Proxy method for Last.fm specific usage (auth flow)
    getLastFM() {
        return this.lastfm;
    }

    // Proxy method for Libre.fm specific usage (auth flow)
    getLibreFm() {
        return this.librefm;
    }

    isAuthenticated() {
        // Return true if any service is configured, so events.js will proceed to call updateNowPlaying
        // Individual services check their own enabled/auth state internally
        return (
            this.lastfm.isAuthenticated() ||
            this.listenbrainz.isEnabled() ||
            this.maloja.isEnabled() ||
            this.librefm.isAuthenticated()
        );
    }

    async updateNowPlaying(track) {
        await this.refreshDaemon();
        await Promise.allSettled(this._own().map((s) => s.updateNowPlaying(track).catch(console.error)));
    }

    /** A finished play from the player journal; `unixSeconds` is when it started. */
    async scrobbleAt(track, unixSeconds) {
        await this.refreshDaemon();
        await Promise.allSettled(this._own().map((s) => s.scrobbleAt(track, unixSeconds).catch(console.error)));
    }

    async onTrackChange(track) {
        await this.refreshDaemon();
        await Promise.allSettled(this._own().map((s) => s.onTrackChange(track).catch(console.error)));
    }

    onPlaybackStop() {
        this.lastfm.onPlaybackStop();
        this.listenbrainz.onPlaybackStop();
        this.maloja.onPlaybackStop();
        this.librefm.onPlaybackStop();
    }

    // Love/Like tracks on all services that support it
    async loveTrack(track) {
        await this.refreshDaemon();
        await Promise.allSettled(
            this._own(['lastfm', 'librefm', 'listenbrainz']).map((s) => s.loveTrack(track).catch(console.error))
        );
        // Maloja feedback could be added here when supported
    }
}
