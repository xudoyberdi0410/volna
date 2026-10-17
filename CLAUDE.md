# Volna

Неофициальный десктоп-плеер YouTube Music с «Моей волной». Tauri v2 + Rust, без своего
бэкенда, Linux и Windows. Интерфейс — форк Monochrome (Apache-2.0). Спека — `PLAN.md`.

## Где что

- `ui/` — фронт (форк Monochrome, JS/Vite). `ui-upstream/` — нетронутый оригинал для сравнения.
- `crates/core` — `volna-core`: всё, что вызывает UI, кроме десктопной обвязки: команды
  (`dispatch.rs`, `api/`, `wave.rs`, `network.rs`), настройки, стримы, логика плеера
  (`player/`: хуки, журнал, команды `player_*`, мост к плееру платформы), C ABI для мобильных
  (`ffi.rs`, `include/volna_core.h`). Сборка под Android/iOS — `docs/mobile-build.md`.
- `crates/daemon` — бинарник `volna`: хост для `volna-core` без GTK/WebKit (сокет `ipc.rs`,
  GStreamer, трей, MPRIS, окно, `volna://api/`); старые пути модулей реэкспортирует из ядра.
  `api_emu/` (эмуляция TIDAL hifi-api) лежит в `src-tauri/src/api_emu` и подключён в демон через `#[path]`.
- `src-tauri/` — бинарник `volna-window`: окно (Tauri), всё пересылает демону по сокету
  (`shell.rs`); `yandex.rs` (вход в Яндекс и выгрузка библиотеки; сопоставление с YTM —
  `ui/js/yandex-import.js`) работает в окне. Схема процессов — `docs/native-player.md` §9.
- `crates/player` — движок плеера (очередь, EQ, визуализатор) над `Output`; GStreamer — фича `gstreamer`.
- `crates/ytm` — свой InnerTube-клиент YouTube Music (WEB_REMIX).
- `crates/net` — reqwest-клиент с прокси (Direct / URL / VLESS через xray).
- `crates/streams` — резолв аудио через yt-dlp и локальный аудиосервер с Range.
- `crates/wave` — движок «Моей волны», SQLite.
- `crates/scrobble` — скробблеры (Last.fm, Libre.fm, ListenBrainz, Maloja) с офлайн-очередью
  в SQLite; команды `scrobble_*` — `crates/core/src/api/scrobble.rs`. По умолчанию выключены,
  пока их скробблит старый UI.
- `ios/` — iOS-приложение (SwiftUI, xtool на Linux) поверх `volna-core`; `VolnaKit` (мост к
  ядру, модели, логика плеера) тестируется на Linux: `cd ios && swift test`. Сборка и
  установка — `docs/mobile-build.md` («The iOS app»).
- `docs/` — `STRUCTURE.md`, `hifi-api-contract.md` (формат ответов для фронта), `streams.md`.
- `packaging/arch/PKGBUILD` — пакет pacman. Windows собирается в CI (`.github/workflows/build.yml`).

## Команды

```sh
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cd ui && bun run build                 # фронт
cargo build -p volna-daemon && (cd ui && bun run tauri dev)   # dev: окно само запустит target/debug/volna
cd packaging/arch && makepkg -f        # пакет; ставить: sudo pacman -U volna-*.pkg.tar.zst
# живые тесты через реальный YouTube, как настроено приложение:
VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081 VOLNA_TEST_COOKIES=chrome \
  cargo test -p volna-daemon api_live -- --ignored --nocapture --test-threads=1
```

При выпуске нового пакета увеличивать `pkgrel` в PKGBUILD.

## Сеть: почему так, а не иначе

- Если music.youtube.com недоступен в вашей стране («not available in your area»),
  метаданные нужно пускать через зарубежный прокси.
- С IP датацентра без cookies yt-dlp получает бот-проверку «Sign in to confirm you're not a bot».
- Рабочая схема: настройки → «Сеть» = Proxy URL (например, `socks5h://127.0.0.1:1081` от
  `ssh -D` до своего сервера) + cookies из браузера. Настройки: `~/.config/uz.khudoberdi.volna/settings.json`.
- Вне KDE/GNOME (например, под Hyprland) yt-dlp не угадывает связку ключей Chrome, поэтому
  `streams` сам дописывает `+kwallet6` (или gnomekeyring) для Chromium-браузеров.
- Ссылки клиента web_embedded googlevideo часто отбивает 403 (эксперимент с PO token,
  привязанным к видео). Поэтому резолв идёт сначала через `player_client=web_music`, и нужен
  провайдер PO token: AUR `bgutil-ytdlp-pot-provider`, пользовательский сервис `bgutil-pot`
  на 127.0.0.1:4416.
- Яндекс Музыка ходит напрямую, без прокси: у выхода прокси сервис может быть недоступен.
  Токен берётся из сессии Яндекса в браузере: yt-dlp выгружает cookies, `oauth.yandex.<tld>/authorize`
  сразу отдаёт токен (домен — тот, где есть сессия: `.ru`, `.uz` и т.д.). Живой тест:
  `VOLNA_TEST_COOKIES=chrome cargo test -p volna-daemon yandex_live -- --ignored --nocapture`.
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
