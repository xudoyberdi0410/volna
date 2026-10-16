import { describe, expect, it, vi } from 'vitest';
import { installLyricsFetchProxy, proxiedLyricsUrl } from '../lyrics-proxy.js';

describe('lyrics-proxy', () => {
    it('proxies allowlisted https hosts only', () => {
        expect(proxiedLyricsUrl('https://lrclib.net/api/get?track_name=a b', { windows: false })).toBe(
            'volna://api/proxy/?u=https%3A%2F%2Flrclib.net%2Fapi%2Fget%3Ftrack_name%3Da%2520b'
        );
        expect(proxiedLyricsUrl('https://lrclib.net/x', { windows: true })).toMatch(
            /^http:\/\/volna\.localhost\/api\/proxy\/\?u=/
        );
        expect(proxiedLyricsUrl('http://lrclib.net/x')).toBe(null);
        expect(proxiedLyricsUrl('https://evil.lrclib.net/x')).toBe(null);
        expect(proxiedLyricsUrl('https://lrclib.net:444/x')).toBe(null);
        expect(proxiedLyricsUrl('https://music.youtube.com/')).toBe(null);
        expect(proxiedLyricsUrl('volna://api/info/')).toBe(null);
    });

    it('rewrites GETs and leaves everything else alone', async () => {
        const calls = [];
        const win = { fetch: vi.fn(async (input, init) => calls.push([input, init])) };
        installLyricsFetchProxy(win);
        installLyricsFetchProxy(win); // idempotent

        await win.fetch('https://lrc.red/search.json?q=x');
        await win.fetch(new URL('https://lrclib.net/api/search?q=y'));
        await win.fetch('https://lrclib.net/api/publish', { method: 'POST' });
        await win.fetch('volna://api/search/?s=x');

        expect(calls[0][0]).toMatch(/proxy\/\?u=https%3A%2F%2Flrc\.red/);
        expect(calls[1][0]).toMatch(/proxy\/\?u=https%3A%2F%2Flrclib\.net/);
        expect(calls[2][0]).toBe('https://lrclib.net/api/publish');
        expect(calls[3][0]).toBe('volna://api/search/?s=x');
    });
});
