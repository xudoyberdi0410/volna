// js/wave.js
// «Моя волна»: the big card on Home, its settings popover, the cold-start
// artist picker, the dislike button in the player and the glue between the
// player and the Rust engine (src-tauri/src/wave.rs, crates/wave).
//
// The player is not modified: plays are observed through capture-phase
// `timeupdate`/`ended` listeners on the document, likes through the
// `favorites-changed` event of db.js. While the wave plays, the queue is
// topped up whenever fewer than three tracks are left.

import { invoke } from '@tauri-apps/api/core';
import { db } from './db.js';
import { showNotification } from './notifications.js';
import { setBeforeAutoSkip } from './playback-failures.js';
import { escapeHtml } from './utils.js';
import {
    applyChoice,
    CHARACTERS,
    DEFAULT_SETTINGS,
    LANGUAGES,
    MOODS,
    needsTopUp,
    normalizeSettings,
    PlayTracker,
    remainingAfter,
    summarize,
    trackForRust,
    upcomingWaveTracks,
} from './wave-logic.js';
import SVG_SETTINGS from '!lucide/sliders-horizontal.svg?svg&icon';
import SVG_THUMBS_DOWN from '!lucide/thumbs-down.svg?svg&icon';
import SVG_CLOSE from '!lucide/x.svg?svg&icon';
import SVG_SEARCH from '!lucide/search.svg?svg&icon';
import SVG_CHECK from '!lucide/check.svg?svg&icon';
import '../wave.css';

const BATCH = 5;
const TOP_UP_AT = 3;
const MIN_ARTISTS = 3;

const PLAY_ICON =
    '<svg viewBox="0 0 24 24" width="30" height="30" aria-hidden="true"><path fill="currentColor" d="M8 5.5v13a1 1 0 0 0 1.52.85l10.4-6.5a1 1 0 0 0 0-1.7L9.52 4.65A1 1 0 0 0 8 5.5z"/></svg>';
const PAUSE_ICON =
    '<svg viewBox="0 0 24 24" width="30" height="30" aria-hidden="true"><rect x="6" y="5" width="4" height="14" rx="1.2" fill="currentColor"/><rect x="14" y="5" width="4" height="14" rx="1.2" fill="currentColor"/></svg>';

class WaveController {
    constructor(player, api) {
        this.player = player;
        this.api = api;
        this.active = false;
        this.status = { cold: true, active: false, settings: { ...DEFAULT_SETTINGS } };
        this.waveIds = new Set();
        this.tracker = new PlayTracker();
        this.activities = null;
        this.toppingUp = null;
        this.starting = false;
    }

    async init() {
        this.renderCard();
        this.installDislikeButton();
        this.hookPlayer();
        this.hookLikes();
        await this.refreshStatus();
        void this.syncLikes();
    }

    // ---- engine calls --------------------------------------------------

    async refreshStatus() {
        try {
            this.status = await invoke('wave_status');
            this.status.settings = normalizeSettings(this.status.settings);
        } catch (e) {
            console.warn('[wave] status failed', e);
        }
        this.updateCard();
    }

    async syncLikes() {
        try {
            const favorites = await db.getFavorites('track');
            const tracks = favorites
                .map((f) => ({ ...trackForRust(f), addedAt: f.addedAt || null }))
                .filter((t) => t.id);
            await invoke('wave_sync_likes', { tracks });
            if (tracks.length && this.status.cold) await this.refreshStatus();
        } catch (e) {
            console.warn('[wave] like sync failed', e);
        }
    }

    prepare(rawTracks) {
        const lossless = this.api.getAPI();
        return (rawTracks || []).map((t) => lossless.prepareTrack(t));
    }

    send(event) {
        const inWave = this.active && this.waveIds.has(String(event.track?.id));
        const payload = { ...event, track: trackForRust(event.track), inWave };
        invoke('wave_event', { event: payload })
            .then((out) => {
                if (out?.reseeded && this.active) void this.requeue();
            })
            .catch((e) => console.warn('[wave] event failed', e));
    }

    // ---- playback --------------------------------------------------------

    async start() {
        if (this.starting) return;
        if (this.status.cold) {
            this.openColdStart();
            return;
        }
        this.starting = true;
        this.setCardBusy(true);
        try {
            const tracks = this.prepare(await invoke('wave_start', { count: BATCH }));
            if (!tracks.length) throw new Error('empty wave');
            this.waveIds = new Set(tracks.map((t) => String(t.id)));
            this.active = true;
            await this.player.setQueue(tracks, 0);
            await this.player.playAtIndex(0);
            window.renderQueueFunction?.();
        } catch (e) {
            console.error('[wave] start failed', e);
            this.active = false;
            const msg = String(e);
            if (msg.includes('no candidates')) {
                this.status.cold = true;
                this.openColdStart();
            } else {
                showNotification('Не удалось запустить волну. Проверьте подключение.');
            }
        } finally {
            this.starting = false;
            this.setCardBusy(false);
            this.updateCard();
            this.updateDislike();
        }
    }

    deactivate() {
        if (!this.active) return;
        this.active = false;
        void invoke('wave_stop').catch(() => {});
        this.updateCard();
        this.updateDislike();
    }

    queueState() {
        const queue = this.player.getCurrentQueue();
        return { queue, index: this.player.currentQueueIndex };
    }

    async topUp() {
        if (!this.active) return;
        if (this.toppingUp) return this.toppingUp;
        const { queue, index } = this.queueState();
        if (!needsTopUp(queue.length, index, TOP_UP_AT)) return;
        this.toppingUp = (async () => {
            try {
                const tracks = this.prepare(await invoke('wave_next', { count: BATCH }));
                if (!this.active || !tracks.length) return;
                tracks.forEach((t) => this.waveIds.add(String(t.id)));
                await this.player.addToQueue(tracks);
                window.renderQueueFunction?.();
            } catch (e) {
                console.warn('[wave] top-up failed', e);
            } finally {
                this.toppingUp = null;
            }
        })();
        return this.toppingUp;
    }

    /** A seed was replaced: swap queued wave tracks for fresh ones. */
    async requeue() {
        if (this.toppingUp) await this.toppingUp;
        const { queue, index } = this.queueState();
        const stale = upcomingWaveTracks(queue, index, this.waveIds);
        if (!stale.length) return this.topUp();
        try {
            for (const { index: i } of stale) await this.player.removeFromQueue(i);
            const fresh = this.prepare(
                await invoke('wave_requeue', { dropped: stale.map((s) => s.id), count: TOP_UP_AT })
            );
            fresh.forEach((t) => this.waveIds.add(String(t.id)));
            if (this.active && fresh.length) await this.player.addToQueue(fresh);
            window.renderQueueFunction?.();
        } catch (e) {
            console.warn('[wave] requeue failed', e);
        }
    }

    hookPlayer() {
        document.addEventListener(
            'timeupdate',
            (e) => {
                if (e.target !== this.player.activeElement) return;
                const el = e.target;
                for (const ev of this.tracker.tick(this.player.currentTrack, el.currentTime, el.duration)) {
                    this.send(ev);
                    if (ev.kind === 'start') this.onTrackStarted(ev.track);
                }
            },
            true
        );
        document.addEventListener(
            'ended',
            (e) => {
                if (e.target !== this.player.activeElement) return;
                for (const ev of this.tracker.ended(this.player.currentTrack)) this.send(ev);
                // Last track and nothing queued: fetch more, then move on
                // (the player itself stops at the end of the queue).
                const { queue, index } = this.queueState();
                if (this.active && remainingAfter(queue.length, index) === 0) {
                    void this.topUp().then(() => this.player.playNext());
                }
            },
            true
        );
        // A new source is loading (skip, or a track whose stream fails and
        // never plays): close the previous play right away. Preloads do not
        // change currentTrack, so they are ignored.
        for (const type of ['loadstart', 'error']) {
            document.addEventListener(
                type,
                (e) => {
                    if (!(e.target instanceof HTMLMediaElement)) return;
                    for (const ev of this.tracker.switchTo(this.player.currentTrack)) this.send(ev);
                    // Top up on every track change, not only once a track
                    // plays: skipped or unplayable tracks never "start".
                    if (type === 'loadstart' && this.active) void this.topUp();
                },
                true
            );
        }
        window.addEventListener('pagehide', () => this.tracker.leave().forEach((ev) => this.send(ev)));
    }

    onTrackStarted(track) {
        if (this.active && !this.waveIds.has(String(track.id))) {
            // The listener started something else: the wave steps aside.
            this.deactivate();
            return;
        }
        this.updateDislike();
        this.updateCard();
        void this.topUp();
    }

    hookLikes() {
        window.addEventListener('favorites-changed', (e) => {
            const d = e.detail;
            // An import adds many likes at once: resync the list instead of one event each.
            if (d?.bulk && d.type === 'track') return void this.syncLikes();
            if (!d || (d.type !== 'track' && d.type !== 'video') || !d.item) return;
            this.send({ kind: 'like', track: d.item, liked: !!d.added });
            if (d.added && this.status.cold) void this.refreshStatus();
        });
        // Keep the card's play/pause icon in sync.
        for (const ev of ['play', 'pause']) {
            document.addEventListener(ev, () => this.updateCard(), true);
        }
    }

    // ---- dislike button ------------------------------------------------------

    installDislikeButton() {
        const like = document.getElementById('now-playing-like-btn');
        if (!like || document.getElementById('wave-dislike-btn')) return;
        const btn = document.createElement('button');
        btn.id = 'wave-dislike-btn';
        btn.className = 'wave-dislike-btn';
        btn.title = 'Не нравится';
        btn.setAttribute('aria-label', 'Не нравится');
        btn.hidden = true;
        btn.innerHTML = SVG_THUMBS_DOWN(20);
        btn.addEventListener('click', () => this.dislikeCurrent());
        like.before(btn);
        this.dislikeBtn = btn;
    }

    updateDislike() {
        if (!this.dislikeBtn) return;
        const t = this.player.currentTrack;
        this.dislikeBtn.hidden = !(this.active && t && this.waveIds.has(String(t.id)));
        // The like button sits next to it and must be visible while the wave plays.
        const like = document.getElementById('now-playing-like-btn');
        if (like && !this.dislikeBtn.hidden) like.style.display = '';
    }

    async dislikeCurrent() {
        const t = this.player.currentTrack;
        if (!t) return;
        // End the play first so the skip is not counted twice.
        const ends = this.tracker.leave();
        ends.forEach((ev) => this.send(ev));
        this.send({ kind: 'dislike', track: t, disliked: true });
        if (await db.isFavorite('track', t.id)) await db.toggleFavorite('track', t);
        showNotification('Больше не поставим этот трек');
        await this.topUp();
        await this.player.playNext();
    }

    // ---- card ------------------------------------------------------------------

    renderCard() {
        const root = document.getElementById('wave-root');
        if (!root) return;
        root.innerHTML = `
            <section class="wave-card" aria-label="Моя волна">
                <div class="wave-card-bg" aria-hidden="true">
                    <span class="wave-blob b1"></span><span class="wave-blob b2"></span><span class="wave-blob b3"></span>
                </div>
                <button class="wave-play" type="button" aria-label="Запустить волну">${PLAY_ICON}</button>
                <div class="wave-text">
                    <h2 class="wave-title">Моя волна</h2>
                    <p class="wave-summary"></p>
                </div>
                <div class="wave-bars" aria-hidden="true"><i></i><i></i><i></i><i></i></div>
                <button class="wave-settings-btn" type="button" title="Настроить волну" aria-haspopup="dialog">
                    ${SVG_SETTINGS(20)}<span>Настроить</span>
                </button>
                <div class="wave-popover" role="dialog" aria-label="Настройки волны" hidden></div>
            </section>`;
        this.card = root.querySelector('.wave-card');
        this.card.querySelector('.wave-play').addEventListener('click', () => this.onCardPlay());
        this.card.querySelector('.wave-settings-btn').addEventListener('click', (e) => {
            e.stopPropagation();
            void this.togglePopover();
        });
        document.addEventListener('click', (e) => {
            const pop = this.card?.querySelector('.wave-popover');
            if (pop && !pop.hidden && !pop.contains(e.target)) pop.hidden = true;
        });
        document.addEventListener('keydown', (e) => {
            if (e.key === 'Escape') {
                const pop = this.card?.querySelector('.wave-popover');
                if (pop) pop.hidden = true;
            }
        });
        this.updateCard();
    }

    onCardPlay() {
        if (this.active) {
            void this.player.handlePlayPause();
        } else {
            void this.start();
        }
    }

    isPlaying() {
        const el = this.player.activeElement;
        return this.active && el && !el.paused && !el.ended && !el.error;
    }

    setCardBusy(busy) {
        this.card?.classList.toggle('is-busy', busy);
    }

    activityLabel() {
        const a = this.status.settings.activity;
        if (!a) return null;
        const all = [...(this.activities?.activities || []), ...(this.activities?.genres || [])];
        return all.find((x) => x.title === a.title)?.label || a.title;
    }

    updateCard() {
        if (!this.card) return;
        const playing = this.isPlaying();
        this.card.classList.toggle('is-active', this.active);
        this.card.classList.toggle('is-playing', playing);
        const play = this.card.querySelector('.wave-play');
        play.innerHTML = playing ? PAUSE_ICON : PLAY_ICON;
        play.setAttribute('aria-label', playing ? 'Пауза' : 'Запустить волну');
        const summary = this.card.querySelector('.wave-summary');
        const t = this.active ? this.player.currentTrack : null;
        if (t) {
            const artist = (t.artists || [t.artist]).filter(Boolean)[0]?.name || '';
            summary.textContent = `${t.title}${artist ? ` — ${artist}` : ''}`;
        } else if (this.status.cold) {
            summary.textContent = 'Выберите любимых исполнителей — и волна подстроится под вас';
        } else {
            summary.textContent = summarize(this.status.settings, this.activityLabel());
        }
    }

    // ---- settings popover ----------------------------------------------------

    async togglePopover() {
        const pop = this.card.querySelector('.wave-popover');
        if (!pop.hidden) {
            pop.hidden = true;
            return;
        }
        pop.hidden = false;
        this.renderPopover();
        if (!this.activities) {
            try {
                this.activities = await invoke('wave_activities');
            } catch (e) {
                console.warn('[wave] activities failed', e);
                this.activities = { activities: [], genres: [] };
            }
            if (!pop.hidden) this.renderPopover();
        }
    }

    renderPopover() {
        const pop = this.card.querySelector('.wave-popover');
        const s = this.status.settings;
        const chips = (group, list, isOn) =>
            list
                .map(
                    (o, i) =>
                        `<button type="button" class="wave-chip${isOn(o) ? ' on' : ''}" data-group="${group}" data-i="${i}">${escapeHtml(o.label)}</button>`
                )
                .join('');
        const acts = this.activities;
        pop.innerHTML = `
            <div class="wave-pop-head">
                <h3>Настройки волны</h3>
                <button type="button" class="wave-pop-close" aria-label="Закрыть">${SVG_CLOSE(18)}</button>
            </div>
            <div class="wave-pop-group"><h4>Характер</h4><div class="wave-chips">${chips('character', CHARACTERS, (o) => s.character === o.value)}</div></div>
            <div class="wave-pop-group"><h4>Настроение</h4><div class="wave-chips">${chips('mood', MOODS, (o) => s.mood === o.value)}</div></div>
            <div class="wave-pop-group"><h4>Занятие</h4><div class="wave-chips">${
                acts
                    ? chips('activity', acts.activities, (o) => s.activity?.title === o.title)
                    : '<span class="wave-muted">Загружаем…</span>'
            }</div></div>
            ${
                acts?.genres?.length
                    ? `<div class="wave-pop-group"><h4>Жанр</h4><div class="wave-chips wave-chips-scroll">${chips('genre', acts.genres, (o) => s.activity?.title === o.title)}</div></div>`
                    : ''
            }
            <div class="wave-pop-group"><h4>Язык</h4><div class="wave-chips">${chips('language', LANGUAGES, (o) => s.language === o.value)}</div></div>
            <button type="button" class="wave-pop-reset btn-secondary">Сбросить</button>`;
        pop.querySelector('.wave-pop-close').addEventListener('click', () => (pop.hidden = true));
        pop.querySelector('.wave-pop-reset').addEventListener('click', () =>
            this.changeSettings({ ...DEFAULT_SETTINGS })
        );
        pop.querySelectorAll('.wave-chip').forEach((chip) =>
            chip.addEventListener('click', (e) => {
                e.stopPropagation();
                const group = chip.dataset.group;
                const i = Number(chip.dataset.i);
                const lists = {
                    character: CHARACTERS,
                    mood: MOODS,
                    language: LANGUAGES,
                    activity: acts?.activities || [],
                    genre: acts?.genres || [],
                };
                const option = lists[group][i];
                const next =
                    group === 'activity' || group === 'genre'
                        ? applyChoice(s, 'activity', option)
                        : applyChoice(s, group, option.value);
                void this.changeSettings(next);
            })
        );
    }

    async changeSettings(settings) {
        const next = normalizeSettings(settings);
        this.status.settings = next;
        this.renderPopover();
        this.updateCard();
        try {
            this.status = await invoke('wave_set_settings', { settings: next });
            this.status.settings = normalizeSettings(this.status.settings);
            // A running wave switches right away: queued tracks get replaced.
            if (this.active) await this.requeue();
        } catch (e) {
            console.warn('[wave] settings failed', e);
            showNotification('Не удалось сохранить настройки волны');
        }
        this.updateCard();
    }

    // ---- cold start ----------------------------------------------------------

    openColdStart() {
        let modal = document.getElementById('wave-coldstart');
        if (!modal) {
            modal = document.createElement('div');
            modal.id = 'wave-coldstart';
            modal.className = 'modal wave-coldstart';
            modal.innerHTML = `
                <div class="modal-overlay"></div>
                <div class="modal-content extra-wide wave-cs-content" role="dialog" aria-labelledby="wave-cs-title">
                    <div class="wave-cs-head">
                        <div>
                            <h3 id="wave-cs-title">Кого вы любите слушать?</h3>
                            <p class="wave-muted">Выберите хотя бы ${MIN_ARTISTS} исполнителей — с них начнётся ваша волна.</p>
                        </div>
                        <button type="button" class="wave-pop-close" aria-label="Закрыть">${SVG_CLOSE(20)}</button>
                    </div>
                    <label class="wave-cs-search">
                        ${SVG_SEARCH(18)}
                        <input type="search" placeholder="Найти исполнителя" autocomplete="off" />
                    </label>
                    <div class="wave-cs-grid" role="list"></div>
                    <div class="wave-cs-foot">
                        <span class="wave-cs-count"></span>
                        <button type="button" class="btn-primary wave-cs-done" disabled>Готово</button>
                    </div>
                </div>`;
            document.body.appendChild(modal);
            this.cs = { modal, picked: new Map(), shown: [], timer: null, query: '' };
            const close = () => modal.classList.remove('active');
            modal.querySelector('.modal-overlay').addEventListener('click', close);
            modal.querySelector('.wave-pop-close').addEventListener('click', close);
            modal.querySelector('input').addEventListener('input', (e) => {
                clearTimeout(this.cs.timer);
                const q = e.target.value;
                this.cs.timer = setTimeout(() => this.loadArtists(q), 300);
            });
            modal.querySelector('.wave-cs-done').addEventListener('click', () => this.finishColdStart());
        }
        modal.classList.add('active');
        this.updateColdStartFoot();
        if (!this.cs.shown.length) void this.loadArtists('');
        setTimeout(() => modal.querySelector('input')?.focus(), 50);
    }

    async loadArtists(query) {
        this.cs.query = query;
        const grid = this.cs.modal.querySelector('.wave-cs-grid');
        grid.classList.add('is-loading');
        try {
            const found = await invoke('wave_artist_search', { query });
            if (this.cs.query !== query) return;
            // Picked artists stay on screen, first.
            const picked = [...this.cs.picked.values()];
            const rest = found.filter((a) => !this.cs.picked.has(a.id));
            this.cs.shown = [...picked, ...rest];
            this.renderArtistGrid();
        } catch (e) {
            console.warn('[wave] artist search failed', e);
            grid.innerHTML = '<p class="wave-muted">Не удалось загрузить исполнителей. Проверьте подключение.</p>';
        } finally {
            grid.classList.remove('is-loading');
        }
    }

    renderArtistGrid() {
        const grid = this.cs.modal.querySelector('.wave-cs-grid');
        grid.innerHTML = this.cs.shown
            .map((a) => {
                const on = this.cs.picked.has(a.id);
                const pic = a.picture
                    ? `<img src="${escapeHtml(a.picture)}" alt="" crossorigin="anonymous" referrerpolicy="no-referrer" />`
                    : `<span class="wave-cs-initial">${escapeHtml((a.name || '?').slice(0, 1))}</span>`;
                return `<button type="button" role="listitem" class="wave-cs-artist${on ? ' on' : ''}" data-id="${escapeHtml(a.id)}" aria-pressed="${on}">
                    <span class="wave-cs-pic">${pic}<span class="wave-cs-check">${SVG_CHECK(18)}</span></span>
                    <span class="wave-cs-name">${escapeHtml(a.name)}</span>
                </button>`;
            })
            .join('');
        grid.querySelectorAll('.wave-cs-artist').forEach((el) =>
            el.addEventListener('click', () => this.toggleArtist(el.dataset.id))
        );
    }

    async toggleArtist(id) {
        const artist = this.cs.shown.find((a) => a.id === id);
        if (!artist) return;
        if (this.cs.picked.has(id)) {
            this.cs.picked.delete(id);
        } else {
            this.cs.picked.set(id, artist);
            // Like Yandex Music: show similar artists right after the pick.
            invoke('wave_related_artists', { id })
                .then((related) => {
                    const known = new Set(this.cs.shown.map((a) => a.id));
                    const extra = related.filter((a) => !known.has(a.id)).slice(0, 6);
                    if (!extra.length) return;
                    const at = this.cs.shown.findIndex((a) => a.id === id) + 1;
                    this.cs.shown.splice(at, 0, ...extra);
                    this.renderArtistGrid();
                })
                .catch(() => {});
        }
        this.renderArtistGrid();
        this.updateColdStartFoot();
    }

    updateColdStartFoot() {
        const n = this.cs.picked.size;
        this.cs.modal.querySelector('.wave-cs-count').textContent =
            n >= MIN_ARTISTS ? `Выбрано: ${n}` : `Выбрано: ${n} из ${MIN_ARTISTS}`;
        this.cs.modal.querySelector('.wave-cs-done').disabled = n < MIN_ARTISTS;
    }

    async finishColdStart() {
        const artists = [...this.cs.picked.values()].map((a) => ({ id: a.id, name: a.name }));
        const btn = this.cs.modal.querySelector('.wave-cs-done');
        btn.disabled = true;
        try {
            this.status = await invoke('wave_cold_start', { artists });
            this.status.settings = normalizeSettings(this.status.settings);
            this.cs.modal.classList.remove('active');
            await this.start();
        } catch (e) {
            console.error('[wave] cold start failed', e);
            showNotification('Не удалось сохранить выбор');
            btn.disabled = false;
        }
    }
}

/** Called once from app.js after the player is ready. */
export async function initWave(player, api) {
    if (!window.__TAURI_INTERNALS__) {
        console.debug('[wave] not running inside Tauri; «Моя волна» is off');
        return null;
    }
    const wave = new WaveController(player, api);
    // An unplayable wave track is skipped: make sure there is a next one.
    setBeforeAutoSkip(() => (wave.active ? wave.topUp() : undefined));
    await wave.init();
    if (import.meta.env.DEV) window.volnaWave = wave;
    return wave;
}
