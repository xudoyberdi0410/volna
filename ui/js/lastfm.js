//js/lastfm.js
import { lastFMStorage } from './storage.js';

export class LastFMScrobbler {
    constructor() {
        this.DEFAULT_API_KEY = '85214f5abbc730e78770f27784b9bdf7';
        this.DEFAULT_API_SECRET = '2c2c37fd86739191860db810dd063292';
        this.API_URL = 'https://ws.audioscrobbler.com/2.0/';

        this.sessionKey = null;
        this.username = null;
        this.currentTrack = null;
        this.scrobbleTimer = null;
        this.scrobbleThreshold = 0;
        this.hasScrobbled = false;
        this.isScrobbling = false;

        this.loadCredentials();
        this.loadSession();
    }

    loadCredentials() {
        if (lastFMStorage.useCustomCredentials()) {
            this.API_KEY = lastFMStorage.getCustomApiKey() || this.DEFAULT_API_KEY;
            this.API_SECRET = lastFMStorage.getCustomApiSecret() || this.DEFAULT_API_SECRET;
        } else {
            this.API_KEY = this.DEFAULT_API_KEY;
            this.API_SECRET = this.DEFAULT_API_SECRET;
        }
    }

    reloadCredentials() {
        this.loadCredentials();
    }

    loadSession() {
        try {
            const session = localStorage.getItem('lastfm-session');
            if (session) {
                const data = JSON.parse(session);
                this.sessionKey = data.key;
                this.username = data.name;
            }
        } catch {
            console.error('Failed to load Last.fm session');
        }
    }

    saveSession(sessionKey, username) {
        this.sessionKey = sessionKey;
        this.username = username;
        localStorage.setItem(
            'lastfm-session',
            JSON.stringify({
                key: sessionKey,
                name: username,
            })
        );
    }

    clearSession() {
        this.sessionKey = null;
        this.username = null;
        localStorage.removeItem('lastfm-session');
    }

    isAuthenticated() {
        return !!this.sessionKey && lastFMStorage.isEnabled();
    }

    _getScrobbleArtist(track) {
        if (!track) return 'Unknown Artist';

        // Get the primary artist name
        let artistName = 'Unknown Artist';

        if (track.artist?.name) {
            artistName = track.artist.name;
        } else if (typeof track.artist === 'string') {
            artistName = track.artist;
        } else if (track.artists && track.artists.length > 0) {
            // Only use the FIRST artist (main artist)
            const first = track.artists[0];
            artistName = typeof first === 'string' ? first : first.name || 'Unknown Artist';
        }

        if (typeof artistName !== 'string') return 'Unknown Artist';
        return artistName || 'Unknown Artist';
    }

    async generateSignature(params) {
        const filteredParams = { ...params };
        delete filteredParams.format;
        delete filteredParams.callback;

        const sortedKeys = Object.keys(filteredParams).sort();

        const signatureString = sortedKeys.map((key) => `${key}${filteredParams[key]}`).join('') + this.API_SECRET;

        console.log('Signature string:', signatureString);

        try {
            const { default: md5 } = await import('./md5.js');
            return md5(signatureString);
        } catch (e) {
            console.error('MD5 library not available', e);
            throw new Error('MD5 library required for Last.fm');
        }
    }

    async makeRequest(method, params = {}, requiresAuth = false) {
        const requestParams = {
            method,
            api_key: this.API_KEY,
            ...params,
        };

        if (requiresAuth && this.sessionKey) {
            requestParams.sk = this.sessionKey;
        }

        const signature = await this.generateSignature(requestParams);

        const formData = new URLSearchParams({
            ...requestParams,
            api_sig: signature,
            format: 'json',
        });

        try {
            const response = await fetch(this.API_URL, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/x-www-form-urlencoded',
                },
                body: formData,
            });

            const data = await response.json();

            if (data.error) {
                throw new Error(data.message || 'Last.fm API error');
            }

            return data;
        } catch (error) {
            console.error('Last.fm API request failed:', error);
            throw error;
        }
    }

    async getAuthUrl() {
        try {
            const data = await this.makeRequest('auth.getToken');
            const token = data.token;

            return {
                token,
                url: `https://www.last.fm/api/auth/?api_key=${this.API_KEY}&token=${token}`,
            };
        } catch (error) {
            console.error('Failed to get auth URL:', error);
            throw error;
        }
    }

    async completeAuthentication(token) {
        try {
            const data = await this.makeRequest('auth.getSession', { token });

            if (data.session) {
                this.saveSession(data.session.key, data.session.name);
                return {
                    success: true,
                    username: data.session.name,
                };
            }

            throw new Error('No session returned');
        } catch (error) {
            console.error('Authentication failed:', error);
            throw error;
        }
    }

    async authenticateWithCredentials(username, password) {
        try {
            const params = {
                username: username,
                password: password,
                api_key: this.API_KEY,
                method: 'auth.getMobileSession',
            };

            const signature = await this.generateSignature(params);

            const formData = new URLSearchParams({
                username: username,
                password: password,
                api_key: this.API_KEY,
                method: 'auth.getMobileSession',
                api_sig: signature,
                format: 'json',
            });

            const response = await fetch(this.API_URL, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/x-www-form-urlencoded',
                },
                body: formData,
            });

            const data = await response.json();

            if (data.error) {
                throw new Error(data.message || 'Last.fm authentication error');
            }

            if (data.session) {
                this.saveSession(data.session.key, data.session.name);
                return {
                    success: true,
                    username: data.session.name,
                };
            }

            throw new Error('No session returned');
        } catch (error) {
            console.error('Mobile authentication failed:', error);
            throw error;
        }
    }

    async updateNowPlaying(track) {
        if (!this.isAuthenticated()) return;

        this.currentTrack = track;
        // Only reset hasScrobbled if we're not currently in the middle of scrobbling
        // to prevent race conditions that could cause double scrobbles
        if (!this.isScrobbling) {
            this.hasScrobbled = false;
        }
        this.clearScrobbleTimer();

        try {
            const scrobbleTitle = track.cleanTitle || track.title;
            const params = {
                artist: this._getScrobbleArtist(track),
                track: scrobbleTitle,
            };

            if (track.album?.title) {
                params.album = track.album.title;
            }

            if (track.duration) {
                params.duration = Math.floor(track.duration);
            }

            if (track.trackNumber) {
                params.trackNumber = track.trackNumber;
            }

            await this.makeRequest('track.updateNowPlaying', params, true);

            console.log('Now playing updated:', scrobbleTitle);
            // Scrobbles come from the player journal (js/player-journal.js), with the play's start time.
        } catch (error) {
            console.error('Failed to update now playing:', error);
        }
    }

    clearScrobbleTimer() {
        if (this.scrobbleTimer) {
            clearTimeout(this.scrobbleTimer);
            this.scrobbleTimer = null;
        }
    }

    /** Scrobbles a finished play with the time it started (unix seconds). */
    async scrobbleAt(track, unixSeconds) {
        if (!this.isAuthenticated() || !track) return;
        try {
            const scrobbleTitle = track.cleanTitle || track.title;
            const params = {
                artist: this._getScrobbleArtist(track),
                track: scrobbleTitle,
                timestamp: Math.floor(unixSeconds),
            };
            if (track.album?.title) params.album = track.album.title;
            if (track.duration) params.duration = Math.floor(track.duration);
            if (track.trackNumber) params.trackNumber = track.trackNumber;

            await this.makeRequest('track.scrobble', params, true);
            console.log('Scrobbled:', scrobbleTitle);
        } catch (error) {
            console.error('Failed to scrobble:', error);
        }
    }

    async loveTrack(track) {
        if (!this.isAuthenticated()) return;

        try {
            const params = {
                artist: this._getScrobbleArtist(track),
                track: track.title,
            };

            await this.makeRequest('track.love', params, true);
            console.log('Loved track on Last.fm:', track.title);
        } catch (error) {
            console.error('Failed to love track on Last.fm:', error);
        }
    }

    async onTrackChange(track) {
        if (!this.isAuthenticated()) return;
        await this.updateNowPlaying(track);
    }

    onPlaybackStop() {
        this.clearScrobbleTimer();
    }

    disconnect() {
        this.clearSession();
        this.clearScrobbleTimer();
        this.currentTrack = null;
    }
}
