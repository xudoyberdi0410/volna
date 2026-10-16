// Settings → «Сеть»: network mode (direct / proxy URL / VLESS), health check,
// xray log and yt-dlp version/update. Talks to the Rust commands in
// src-tauri/src/network.rs and commands.rs; formatting lives in
// network-settings-model.js.

import { invoke } from '@tauri-apps/api/core';
import {
    buildConfig,
    describeHealth,
    describeStatus,
    describeYtDlp,
    formFromConfig,
    precheck,
} from './network-settings-model.js';
import '../network-settings.css';

const $ = (id) => document.getElementById(id);

function inTauri() {
    return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

function errorText(e) {
    return typeof e === 'string' ? e : (e?.message ?? String(e));
}

class NetworkSettings {
    constructor() {
        this.el = {
            status: $('net-status'),
            mode: $('net-mode-select'),
            urlField: $('net-url-field'),
            url: $('net-url-input'),
            vlessField: $('net-vless-field'),
            vless: $('net-vless-input'),
            message: $('net-message'),
            check: $('net-check-btn'),
            apply: $('net-apply-btn'),
            health: $('net-health'),
            xrayGroup: $('net-xray-group'),
            logs: $('net-logs'),
            logsBtn: $('net-logs-btn'),
            ytdlp: $('ytdlp-version'),
            ytdlpUpdate: $('ytdlp-update-btn'),
        };
        this.loaded = false;
    }

    bind() {
        const { el } = this;
        el.mode.addEventListener('change', () => this.showFields());
        el.apply.addEventListener('click', () => void this.apply());
        el.check.addEventListener('click', () => void this.check());
        el.logsBtn.addEventListener('click', () => void this.refreshLogs());
        el.ytdlpUpdate.addEventListener('click', () => void this.updateYtDlp());
        // Load lazily, the first time the tab is opened (and again on later opens).
        document
            .querySelector('.settings-tab[data-tab="network"]')
            ?.addEventListener('click', () => void this.refresh());
        if ($('settings-tab-network')?.classList.contains('active')) void this.refresh();
    }

    showFields() {
        const mode = this.el.mode.value;
        this.el.urlField.hidden = mode !== 'url';
        this.el.vlessField.hidden = mode !== 'vless';
    }

    setMessage(text, tone = '') {
        this.el.message.textContent = text;
        this.el.message.dataset.tone = tone;
    }

    setBusy(busy) {
        for (const b of [this.el.apply, this.el.check, this.el.mode]) b.disabled = busy;
    }

    async refresh() {
        try {
            const view = await invoke('net_settings');
            // Keep what the user is typing if the form was already filled once.
            if (!this.loaded) {
                const form = formFromConfig(view.config);
                this.el.mode.value = form.mode;
                this.el.url.value = form.url;
                this.el.vless.value = form.vless;
                this.showFields();
                this.loaded = true;
            }
            this.renderStatus(view.status);
            if (view.startup_error) {
                this.setMessage(`Сохранённая настройка не применилась при запуске: ${view.startup_error}`, 'bad');
            }
        } catch (e) {
            this.el.status.textContent = `Ошибка: ${errorText(e)}`;
        }
        void this.refreshYtDlp();
    }

    renderStatus(status) {
        this.el.status.textContent = describeStatus(status);
        const vless = status.mode === 'vless';
        this.el.xrayGroup.hidden = !vless && this.el.logs.textContent === '';
        if (vless) void this.refreshLogs();
    }

    currentConfig() {
        return buildConfig(this.el.mode.value, { url: this.el.url.value, vless: this.el.vless.value });
    }

    async apply() {
        const config = this.currentConfig();
        const problem = precheck(config);
        if (problem) return this.setMessage(problem, 'bad');
        this.setBusy(true);
        this.setMessage(config.mode === 'vless' ? 'Запускаю xray…' : 'Применяю…');
        try {
            const status = await invoke('net_apply', { config });
            this.renderStatus(status);
            this.setMessage('Применено. Проверяю соединение…');
            const tone = await this.check({ busy: false });
            this.setMessage(
                tone === 'bad' ? 'Применено, но соединение не работает (подробности ниже)' : 'Применено',
                tone === 'bad' ? 'bad' : 'good'
            );
        } catch (e) {
            this.setMessage(`Не удалось применить: ${errorText(e)}`, 'bad');
            if (config.mode === 'vless') {
                this.el.xrayGroup.hidden = false;
                void this.refreshLogs();
            }
        } finally {
            this.setBusy(false);
        }
    }

    /**
     * Health check of the *active* connection (apply first to test a new one).
     * Resolves to the report's tone ('good' | 'warn' | 'bad').
     */
    async check({ busy = true } = {}) {
        if (busy) this.setBusy(true);
        this.el.health.hidden = false;
        this.el.health.dataset.tone = '';
        this.el.health.textContent = 'Проверяю music.youtube.com…';
        try {
            const audio = invoke('net_audio_check').catch((e) => ({ ok: false, kind: 'other', message: errorText(e) }));
            const report = await invoke('net_health');
            if (busy) this.setMessage('');
            if (!report.ok) return this.renderHealth(report);
            this.renderHealth(report, 'pending');
            return this.renderHealth(report, await audio);
        } catch (e) {
            this.el.health.textContent = `Ошибка проверки: ${errorText(e)}`;
            this.el.health.dataset.tone = 'bad';
            return 'bad';
        } finally {
            if (busy) this.setBusy(false);
        }
    }

    renderHealth(report, audio = null) {
        const { tone, rows } = describeHealth(report, audio);
        const dl = document.createElement('dl');
        for (const [k, v] of rows) {
            const dt = document.createElement('dt');
            dt.textContent = k;
            const dd = document.createElement('dd');
            dd.textContent = v;
            dl.append(dt, dd);
        }
        this.el.health.replaceChildren(dl);
        this.el.health.dataset.tone = tone;
        return tone;
    }

    async refreshLogs() {
        try {
            const lines = await invoke('net_xray_logs');
            this.el.logs.textContent = lines.length ? lines.slice(-200).join('\n') : '(пусто)';
            this.el.logs.scrollTop = this.el.logs.scrollHeight;
        } catch (e) {
            this.el.logs.textContent = errorText(e);
        }
    }

    async refreshYtDlp() {
        try {
            this.el.ytdlp.textContent = describeYtDlp(await invoke('ytdlp_version'));
        } catch (e) {
            this.el.ytdlp.textContent = `Не найден: ${errorText(e)}`;
            this.el.ytdlpUpdate.disabled = true;
        }
    }

    async updateYtDlp() {
        const btn = this.el.ytdlpUpdate;
        btn.disabled = true;
        this.el.ytdlp.textContent = 'Обновляю…';
        try {
            const r = await invoke('ytdlp_update');
            const tail = r.updated
                ? `обновлён ${r.version_before} → ${r.version_after}`
                : r.origin === 'system'
                  ? 'системный бинарь обновляется пакетным менеджером'
                  : 'уже последняя версия';
            this.el.ytdlp.textContent = `${describeYtDlp({ version: r.version_after, origin: r.origin })} · ${tail}`;
            this.el.ytdlp.title = r.message;
        } catch (e) {
            this.el.ytdlp.textContent = `Не удалось обновить: ${errorText(e)}`;
        } finally {
            btn.disabled = false;
        }
    }
}

function init() {
    if (!$('settings-tab-network')) return;
    if (!inTauri()) {
        // Plain browser (vite preview): there is no Rust side to talk to.
        document.querySelector('.settings-tab[data-tab="network"]')?.remove();
        return;
    }
    new NetworkSettings().bind();
}

if (typeof document !== 'undefined') {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init, { once: true });
    else init();
}
