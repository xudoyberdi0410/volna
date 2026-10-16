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

const TONE_RANK = { good: 0, warn: 1, bad: 2 };
const worse = (a, b) => (TONE_RANK[b] > TONE_RANK[a] ? b : a);

/**
 * The «Звук» row: can yt-dlp resolve audio through this connection?
 * `check` is the result of `net_audio_check`, or `'pending'` while it runs.
 */
export function describeAudio(check) {
    if (check === 'pending') return { text: 'проверяю…', tone: 'good' };
    if (!check) return { text: 'неизвестно', tone: 'warn' };
    if (check.ok) return { text: `работает (${check.latency_ms} мс)`, tone: 'good' };
    switch (check.kind) {
        case 'bot_check':
            return {
                text: 'не играет: YouTube требует подтвердить, что вы не бот, для этого IP. Выберите другое подключение или включите cookies ниже',
                tone: 'bad',
            };
        case 'unavailable':
            return { text: 'не удалось проверить (тестовый трек недоступен)', tone: 'warn' };
        case 'no_yt_dlp':
            return { text: 'yt-dlp не найден', tone: 'bad' };
        default:
            return { text: `ошибка: ${check.message ?? 'неизвестная'}`, tone: 'bad' };
    }
}

/** Shortens the reqwest error chain for the most common proxy failures. */
export function humanizeNetError(error) {
    const e = String(error ?? '');
    if (/socks proxy|proxy.*(refused|connect)|tunnel/i.test(e)) return `Прокси не отвечает. ${e}`;
    if (/timed? ?out/i.test(e)) return `Нет ответа (тайм-аут). ${e}`;
    if (/dns|resolve|lookup/i.test(e)) return `Не удалось найти адрес (DNS). ${e}`;
    return e;
}

/**
 * Turns a HealthReport (and, optionally, the audio check) into display rows
 * plus an overall tone.
 */
export function describeHealth(report, audio = null) {
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
    let tone = 'bad';
    if (report.ok && report.ytm_available !== false) tone = report.ytm_available ? 'good' : 'warn';
    // Without a connection the audio check says nothing new.
    if (audio !== null && report.ok) {
        const a = describeAudio(audio);
        rows.push(['Звук', a.text]);
        tone = worse(tone, a.tone);
    }
    if (report.error) rows.push(['Ошибка', humanizeNetError(report.error)]);
    return { tone, rows };
}

/** Text for the yt-dlp line: version and where the binary came from. */
export function describeYtDlp(info) {
    const origin = { sidecar: 'встроенный', system: 'системный', configured: 'из VOLNA_YTDLP' };
    return `${info.version} · ${origin[info.origin] ?? info.origin}`;
}

// ---- yt-dlp cookies (volna_streams::Cookies) --------------------------------

/** Browsers offered in the cookie selector (yt-dlp names). */
export const COOKIE_BROWSERS = ['firefox', 'chrome', 'chromium', 'brave', 'edge', 'opera', 'vivaldi'];

/**
 * Form values -> the Cookies JSON for `net_set_cookies`.
 * `choice` is 'none', 'file' or a browser name; `extra` is the cookies.txt
 * path for 'file', or an optional profile for a browser.
 */
export function buildCookies(choice, extra = '') {
    const value = extra.trim();
    if (choice === 'none') return { mode: 'none' };
    if (choice === 'file') return { mode: 'file', value };
    return { mode: 'browser', value: value ? `${choice}:${value}` : choice };
}

/** Saved Cookies JSON -> form values. */
export function cookiesForm(cookies) {
    if (cookies?.mode === 'file') return { choice: 'file', extra: cookies.value ?? '' };
    if (cookies?.mode === 'browser' && cookies.value) {
        const [name, ...profile] = String(cookies.value).split(':');
        if (COOKIE_BROWSERS.includes(name)) return { choice: name, extra: profile.join(':') };
    }
    return { choice: 'none', extra: '' };
}

/** Client-side check before Rust validates again. Returns an error or null. */
export function precheckCookies(cookies) {
    if (cookies.mode === 'file' && !cookies.value) return 'Укажите путь к cookies.txt';
    return null;
}
