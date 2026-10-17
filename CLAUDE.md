# Volna

Неофициальный десктоп-плеер YouTube Music с «Моей волной». Tauri v2 + Rust, без своего
бэкенда, Linux и Windows. Интерфейс — форк Monochrome (Apache-2.0). Спека — `PLAN.md`.

## Где что

- `ui/` — фронт (форк Monochrome, JS/Vite). `ui-upstream/` — нетронутый оригинал для сравнения.
- `src-tauri/` — приложение: `protocol.rs` (схема `volna://`), `api_emu/` (эмуляция TIDAL
  hifi-api, которую ждёт фронт), `network.rs` (настройки сети и cookies), `wave*.rs`, `yandex.rs` (вход в Яндекс
  и выгрузка библиотеки; сопоставление с YTM — `ui/js/yandex-import.js`).
- `crates/ytm` — свой InnerTube-клиент YouTube Music (WEB_REMIX).
- `crates/net` — reqwest-клиент с прокси (Direct / URL / VLESS через xray).
- `crates/streams` — резолв аудио через yt-dlp и локальный аудиосервер с Range.
- `crates/wave` — движок «Моей волны», SQLite.
- `docs/` — `STRUCTURE.md`, `hifi-api-contract.md` (формат ответов для фронта), `streams.md`.
- `packaging/arch/PKGBUILD` — пакет pacman. Windows собирается в CI (`.github/workflows/build.yml`).

## Команды

```sh
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cd ui && bun run build                 # фронт
cd ui && bun run tauri dev             # приложение в dev-режиме
cd packaging/arch && makepkg -f        # пакет; ставить: sudo pacman -U volna-*.pkg.tar.zst
# живые тесты через реальный YouTube, как настроено приложение:
VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081 VOLNA_TEST_COOKIES=chrome \
  cargo test -p volna api_live -- --ignored --nocapture --test-threads=1
```

При выпуске нового пакета увеличивать `pkgrel` в PKGBUILD.

## Сеть: почему так, а не иначе (2026-10-17)

- С узбекского IP music.youtube.com отвечает «not available in your area», поэтому
  метаданные идут только через зарубежный выход.
- С IP Hetzner (203.0.113.10) без cookies yt-dlp получает «Sign in to confirm you're not a bot».
- Рабочая схема: настройки → «Сеть» = Proxy URL `socks5h://127.0.0.1:1081` (пользовательский
  сервис `ssh-socks`) + cookies из Chrome. Настройки: `~/.config/uz.khudoberdi.volna/settings.json`.
- Под Hyprland yt-dlp не угадывает связку ключей Chrome, поэтому `streams` сам дописывает
  `+kwallet6` (или gnomekeyring) для Chromium-браузеров.
- Ссылки клиента web_embedded googlevideo часто отбивает 403 (эксперимент с PO token,
  привязанным к видео). Поэтому резолв идёт сначала через `player_client=web_music`, и нужен
  провайдер PO token: AUR `bgutil-ytdlp-pot-provider`, пользовательский сервис `bgutil-pot`
  на 127.0.0.1:4416.
- Яндекс Музыка ходит напрямую, без прокси (с узбекского IP сервис доступен). Токен берётся
  из сессии Яндекса в Chrome: yt-dlp выгружает cookies, `oauth.yandex.uz/authorize` сразу
  отдаёт токен (сессия на `.yandex.uz`, не `.ru`). Живой тест:
  `VOLNA_TEST_COOKIES=chrome cargo test -p volna yandex_live -- --ignored --nocapture`.
- Тихий звук бывает из-за громкости потока, сохранённой в WirePlumber
  (`~/.local/state/wireplumber/stream-properties`), а не из-за кода.

## Правила

- Webview не ходит в YouTube/Google напрямую: всё через Rust, иначе прокси не применяется,
  а ссылки googlevideo привязаны к IP резолва.
- Скачивание треков вырезано намеренно, не возвращать.
- Системную сеть, VPN и фаервол не трогать.
- Коммиты атомарные, с `Co-Authored-By`.

## Открытые вопросы

- Windows ни разу не собирался и не запускался; встроенному yt-dlp там нужен deno и провайдер PO token.
- Нет длительностей в фиде главной (YTM их не присылает), неполная дискография артистов,
  обложка в MPRIS не грузится (URL вида volna://).
- Два сценария волны не проверены вживую: перезасев после двух пропусков и дозаполнение
  очереди после дослушанного трека.
