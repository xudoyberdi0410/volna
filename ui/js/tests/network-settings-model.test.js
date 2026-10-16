import { describe, expect, it } from 'vitest';
import {
    buildConfig,
    describeHealth,
    describeStatus,
    describeYtDlp,
    formFromConfig,
    precheck,
} from '../network-settings-model.js';

describe('network-settings-model', () => {
    it('builds ProxyConfig JSON for each mode', () => {
        expect(buildConfig('direct', { url: 'x' })).toEqual({ mode: 'direct' });
        expect(buildConfig('url', { url: ' socks5h://127.0.0.1:1080 ' })).toEqual({
            mode: 'url',
            value: 'socks5h://127.0.0.1:1080',
        });
        expect(buildConfig('vless', { vless: 'vless://a@b:1\n' })).toEqual({ mode: 'vless', value: 'vless://a@b:1' });
        expect(() => buildConfig('ss')).toThrow();
    });

    it('round-trips stored config into form values', () => {
        expect(formFromConfig({ mode: 'vless', value: 'vless://x' })).toEqual({
            mode: 'vless',
            url: '',
            vless: 'vless://x',
        });
        expect(formFromConfig({ mode: 'url', value: 'http://h:1' }).url).toBe('http://h:1');
        expect(formFromConfig(undefined).mode).toBe('direct');
        expect(formFromConfig({ mode: 'weird' }).mode).toBe('direct');
    });

    it('prechecks obvious mistakes', () => {
        expect(precheck({ mode: 'direct' })).toBeNull();
        expect(precheck({ mode: 'url', value: '' })).toMatch(/адрес/);
        expect(precheck({ mode: 'url', value: 'ftp://h:1' })).toMatch(/socks5/);
        expect(precheck({ mode: 'url', value: 'not a url' })).toMatch(/Неверный/);
        expect(precheck({ mode: 'url', value: 'socks5h://u:p@127.0.0.1:1080' })).toBeNull();
        expect(precheck({ mode: 'vless', value: 'vmess://x' })).toMatch(/vless/);
        expect(precheck({ mode: 'vless', value: 'VLESS://id@h:443' })).toBeNull();
    });

    it('describes status', () => {
        expect(describeStatus({ mode: 'direct', proxy: null, xray_running: null })).toBe('Напрямую');
        expect(describeStatus({ mode: 'url', proxy: 'socks5h://***@127.0.0.1:1080' })).toBe(
            'Прокси · socks5h://***@127.0.0.1:1080'
        );
        expect(describeStatus({ mode: 'vless', proxy: 'x', xray_running: true, xray_pid: 42 })).toBe(
            'VLESS (xray) · xray работает (pid 42)'
        );
        expect(describeStatus({ mode: 'vless', xray_running: false })).toMatch(/остановлен/);
    });

    it('describes health reports', () => {
        const good = describeHealth({ ok: true, latency_ms: 613, exit_ip: '1.2.3.4', ytm_available: true });
        expect(good.tone).toBe('good');
        expect(good.rows).toContainEqual(['Задержка', '613 мс']);

        expect(describeHealth({ ok: true, ytm_available: false }).tone).toBe('bad');
        expect(describeHealth({ ok: true, ytm_available: null }).tone).toBe('warn');
        const failed = describeHealth({ ok: false, error: 'timeout' });
        expect(failed.tone).toBe('bad');
        expect(failed.rows).toContainEqual(['Ошибка', 'timeout']);
        expect(failed.rows).toContainEqual(['Внешний IP', '—']);
    });

    it('describes yt-dlp', () => {
        expect(describeYtDlp({ version: '2026.08.19', origin: 'sidecar' })).toBe('2026.08.19 · встроенный');
    });
});
