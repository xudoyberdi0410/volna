import { describe, expect, it } from 'vitest';
import { isWindowsWebview, volnaBase, volnaUrl } from '../volna-url.js';

describe('volna-url', () => {
    it('detects WebView2 from the user agent', () => {
        expect(isWindowsWebview('Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/130.0')).toBe(true);
        expect(isWindowsWebview('Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15')).toBe(false);
    });

    it('uses the custom scheme on Linux and localhost on Windows', () => {
        expect(volnaBase('api', { windows: false })).toBe('volna://api');
        expect(volnaBase('stream', { windows: true })).toBe('http://volna.localhost/stream');
    });

    it('builds paths and skips empty query values', () => {
        expect(volnaUrl('api', 'search/', { s: 'daft punk', limit: 10, x: undefined }, { windows: false })).toBe(
            'volna://api/search/?s=daft+punk&limit=10'
        );
        expect(volnaUrl('stream', '/abc123', undefined, { windows: true })).toBe(
            'http://volna.localhost/stream/abc123'
        );
    });

    it('rejects unknown routes', () => {
        expect(() => volnaBase('evil')).toThrow();
    });
});
