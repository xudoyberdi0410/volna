import { describe, expect, it } from 'vitest';
import {
    buildStreamDescriptor,
    isDirectImage,
    isRadioId,
    isVolnaImage,
    resizeImage,
    routeForYoutubeLink,
} from '../volna-media.js';

describe('volna-media', () => {
    it('plays from the loopback audio server', () => {
        const d = buildStreamDescriptor('dQw4w9WgXcQ', 'HI_RES_LOSSLESS', { base: 'http://127.0.0.1:9/t/stream' });
        expect(d.url).toBe('http://127.0.0.1:9/t/stream/dQw4w9WgXcQ');
        expect(d.playbackType).toBe('direct');
        expect(d.url).not.toMatch(/\.mpd|\.m3u8/);
        expect(d.lossless).toBe(false);
        const low = buildStreamDescriptor('a_b-c', 'LOW', { base: 'http://127.0.0.1:9/t/stream' });
        expect(low.url).toBe('http://127.0.0.1:9/t/stream/a_b-c?q=low');
        expect(low.quality).toBe('LOW');
    });

    it('falls back to the volna:// stream route without a server', () => {
        const d = buildStreamDescriptor('abc', 'HIGH', { base: null });
        expect(d.url).toMatch(/^(volna:\/\/stream|http:\/\/volna\.localhost\/stream)\/abc$/);
    });

    it('recognises direct image URLs', () => {
        expect(isDirectImage('volna://img/?u=x')).toBe(true);
        expect(isDirectImage('http://volna.localhost/img/?u=x')).toBe(true);
        expect(isDirectImage('https://i.ytimg.com/x.jpg')).toBe(true);
        expect(isDirectImage('data:image/png;base64,AA')).toBe(true);
        expect(isDirectImage('assets/appicon.png')).toBe(true);
        expect(isDirectImage('/assets/appicon.png')).toBe(true);
        expect(isDirectImage('8ad1e2a3-1b2c-4d5e-8f90-1234567890ab')).toBe(false);
        expect(isDirectImage(null)).toBe(false);
    });

    it('resizes only volna images', () => {
        expect(resizeImage('volna://img/?u=https%3A%2F%2Fa%2Fb&s=544', '80')).toBe(
            'volna://img/?u=https%3A%2F%2Fa%2Fb&s=80'
        );
        expect(resizeImage('http://volna.localhost/img/?u=x', 160)).toBe('http://volna.localhost/img/?u=x&s=160');
        expect(isVolnaImage('https://example.com/img/')).toBe(false);
        expect(resizeImage('https://example.com/a.jpg', '80')).toBe('https://example.com/a.jpg');
        expect(resizeImage('volna://img/?u=x&s=544', 'bogus')).toBe('volna://img/?u=x&s=544');
    });

    it('tells radios from playlists', () => {
        expect(isRadioId('RDAMVMdQw4w9WgXcQ')).toBe(true);
        expect(isRadioId('RDEMabc')).toBe(true);
        expect(isRadioId('RDCLAK5uy_abc')).toBe(false);
        expect(isRadioId('PLabc')).toBe(false);
    });

    it('maps pasted YouTube links to routes', () => {
        expect(routeForYoutubeLink('https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=RDAMVMx')).toBe(
            '/track/dQw4w9WgXcQ'
        );
        expect(routeForYoutubeLink('https://youtu.be/dQw4w9WgXcQ?t=3')).toBe('/track/dQw4w9WgXcQ');
        expect(routeForYoutubeLink('https://www.youtube.com/playlist?list=PLabc')).toBe('/playlist/PLabc');
        expect(routeForYoutubeLink('https://music.youtube.com/playlist?list=RDCLAK5uy_x')).toBe(
            '/playlist/RDCLAK5uy_x'
        );
        expect(routeForYoutubeLink('https://music.youtube.com/playlist?list=OLAK5uy_x')).toBe('/album/OLAK5uy_x');
        expect(routeForYoutubeLink('https://music.youtube.com/playlist?list=RDEMabc')).toBe('/mix/RDEMabc');
        expect(routeForYoutubeLink('https://music.youtube.com/browse/MPREb_abc')).toBe('/album/MPREb_abc');
        expect(routeForYoutubeLink('https://music.youtube.com/channel/UCabc')).toBe('/artist/UCabc');
        expect(routeForYoutubeLink('https://example.com/watch?v=x')).toBe(null);
        expect(routeForYoutubeLink('not a link')).toBe(null);
        expect(routeForYoutubeLink('https://youtube.com/watch?v=<script>')).toBe(null);
    });
});
