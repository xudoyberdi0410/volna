# Volna — десктопный плеер YouTube Music с «Моей волной»

Tauri v2 + Rust, без своего бэкенда. Linux и Windows. Интерфейс — форк Monochrome
(Apache-2.0, `ui-upstream/`, коммит в `ui-upstream/UPSTREAM_COMMIT`), источник — YouTube Music.

## Архитектура

```
┌─ webview (форк Monochrome, ui/) ────────────────────────────┐
│ fetch("volna://api/search/?s=..")   <audio src="volna://stream/<videoId>">
└──────────────┬──────────────────────────────┬──────────────┘
               │ custom protocol              │ custom protocol (Range)
┌─ src-tauri (Rust) ─────────────────────────────────────────────┐
│ api_emu   — отвечает в формате hifi-api (TIDAL), который ждёт  │
│             Monochrome; данные из ytm                          │
│ ytm       — свой InnerTube-клиент (WEB_REMIX): search, browse  │
│             (artist/album/playlist), next (радио), related,    │
│             lyrics, moods&genres                               │
│ streams   — резолв аудио через yt-dlp (sidecar), кэш по expire │
│ wave      — движок «Моей волны», SQLite: plays/skips/likes     │
│ net       — единый reqwest-клиент с прокси; менеджер xray      │
└────────────────────────────────────────────────────────────────┘
```

Принципы:
- Webview никогда не ходит в YouTube/Google напрямую: все запросы, аудио и обложки идут
  через Rust, поэтому прокси применяется ко всему. Ссылки googlevideo привязаны к IP,
  поэтому поток обязан идти через тот же выход, что и резолв.
- Без логина в Google на первом этапе: вкус копится локально (SQLite). Синк лайков с
  аккаунтом YTM (cookies) — опционально, позже.
- Без скачивания треков (вырезаем downloads из Monochrome).

## Сеть / прокси (настройка в UI)

1. **Напрямую** — системный маршрут (системный VPN, если включён).
2. **Прокси URL** — `socks5://` / `http://` (например, локальный xray на ПК или SSH -D).
3. **VLESS-ссылка** — приложение запускает встроенный xray-sidecar с конфигом из
   `vless://…` (например, REALITY/xhttp), поднимает socks на 127.0.0.1:<random>
   и гонит через него всё. Бинарь xray кладётся в бандл (Linux + Windows).

yt-dlp получает тот же прокси (`--proxy`).

## «Моя волна»

- **Сигналы**: трек дослушан (>70%) = +, пропуск <30 с = сильный −, пропуск позже = слабый −,
  лайк = ++, дизлайк = бан трека и сильный − артисту. Всё с затуханием по времени.
- **Кандидаты**: радио YTM (`next` с `RDAMVM<id>`) от 2–4 сидов (свежие лайки/дослушанные),
  related, топ-треки близких артистов, moods&genres под выбранное настроение.
- **Ранжирование**: аффинити к артисту, новизна (не играл последние N дней), штраф за
  повтор артиста подряд, ε-исследование. Пересчёт после каждого трека: два пропуска
  подряд от одного сида → смена сида.
- **Настройки как в ЯМ**: характер (Любимое / Незнакомое / Популярное), настроение
  (Бодрое / Весёлое / Спокойное / Грустное), занятие (moods&genres YTM), язык.
- **Холодный старт**: экран выбора 3+ артистов → сиды.
- UI: большая кнопка «Моя волна» на главной, бесконечная очередь, лайк/дизлайк в плеере.

## Фазы

0. **Каркас**: Tauri v2 app, workspace, `ui/` = копия Monochrome с вычищенными
   аккаунтами/аналитикой/Sentry/listening party/downloads/Apple Music/podcasts;
   `bun run tauri dev` открывает UI.
1. **ytm + net** (параллельно): InnerTube-клиент с тестами на живом API; прокси-клиент.
2. **streams**: yt-dlp sidecar, протокол `volna://stream/` с Range, кэш, авто-обновление
   yt-dlp.
3. **api_emu + проводка UI**: эндпоинты, которые реально вызывает Monochrome
   (`/search /track /album /artist /artist/similar /album/similar /playlist /mix
   /trackManifests /info`), обложки, правки фронта под не-TIDAL id.
4. **wave**: движок + SQLite + UI-кнопка и настройки.
5. **прокси UI + xray sidecar**.
6. **сборка**: Linux (AppImage/deb) локально; Windows — GitHub Actions (NSIS), т.к. кросс-сборка
   Tauri с Linux нестабильна.

## Риски

- YouTube меняет player/PO token → лечится обновлением yt-dlp (кнопка + авто).
- Формат hifi-api не задокументирован → реверсим по нормализаторам в `ui-upstream/js/api.js`.
- Monochrome местами жёстко завязан на TIDAL (числовые id, resources.tidal.com) → патчим фронт.
