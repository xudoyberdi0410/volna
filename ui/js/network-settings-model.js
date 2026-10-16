// Pure helpers for Settings → «Сеть». No DOM, no Tauri: tested in node.
//
// The Rust side stores the setting as {mode:'direct'} or
// {mode:'url'|'vless', value:'...'} (volna_net::ProxyConfig).

export const MODES = ['direct', 'url', 'vless'];

const PROXY_SCHEMES = ['socks5:', 'socks5h:', 'http:', 'https:'];

/** Builds the ProxyConfig sent to `net_apply` from form values. */
export function buildConfig(mode, { url = '', vless = '' } = {}) {
    if (mode === 'direct') return { mode: 'direct' };
    if (mode === 'url') return { mode: 'url', value: url.trim() };
    if (mode === 'vless') return { mode: 'vless', value: vless.trim() };
    throw new Error(`unknown network mode: ${mode}`);
}

/** Splits a stored ProxyConfig back into form values. */
export function formFromConfig(config) {
    const mode = MODES.includes(config?.mode) ? config.mode : 'direct';
    return {
        mode,
        url: mode === 'url' ? (config.value ?? '') : '',
        vless: mode === 'vless' ? (config.value ?? '') : '',
    };
}

/**
 * Quick client-side check before calling Rust, which validates again.
 * Returns an error message or null.
 */
export function precheck(config) {
    if (config.mode === 'direct') return null;
    const value = config.value ?? '';
    if (!value) return config.mode === 'url' ? 'Укажите адрес прокси' : 'Вставьте ссылку vless://';
    if (config.mode === 'vless') {
        return /^vless:\/\//i.test(value) ? null : 'Ссылка должна начинаться с vless://';
    }
    let parsed;
    try {
        parsed = new URL(value);
    } catch {
        return 'Неверный адрес, пример: socks5h://127.0.0.1:1080';
    }
    if (!PROXY_SCHEMES.includes(parsed.protocol)) {
        return 'Поддерживаются socks5://, socks5h://, http:// и https://';
    }
    return null;
}

const MODE_LABELS = { direct: 'Напрямую', url: 'Прокси', vless: 'VLESS (xray)' };

/** One line for the current state, e.g. «VLESS (xray) · xray работает (pid 123)». */
export function describeStatus(status) {
    if (!status) return 'Нет данных';
    const parts = [MODE_LABELS[status.mode] ?? status.mode];
    if (status.proxy && status.mode === 'url') parts.push(status.proxy);
    if (status.xray_running === true) {
        parts.push(status.xray_pid ? `xray работает (pid ${status.xray_pid})` : 'xray работает');
    } else if (status.xray_running === false) {
        parts.push('xray остановлен');
    }
    return parts.join(' · ');
}

/** Turns a HealthReport into display rows plus an overall tone. */
export function describeHealth(report) {
    const ytm =
        report.ytm_available === true
            ? 'доступна'
            : report.ytm_available === false
              ? 'недоступна в этой стране'
              : 'неизвестно';
    const rows = [
        ['Внешний IP', report.exit_ip ?? '—'],
        ['Задержка', report.latency_ms != null ? `${report.latency_ms} мс` : '—'],
        ['YouTube Music', ytm],
    ];
    if (report.error) rows.push(['Ошибка', report.error]);
    let tone = 'bad';
    if (report.ok && report.ytm_available !== false) tone = report.ytm_available ? 'good' : 'warn';
    return { tone, rows };
}

/** Text for the yt-dlp line: version and where the binary came from. */
export function describeYtDlp(info) {
    const origin = { sidecar: 'встроенный', system: 'системный', configured: 'из VOLNA_YTDLP' };
    return `${info.version} · ${origin[info.origin] ?? info.origin}`;
}
