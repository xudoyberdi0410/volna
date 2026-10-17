# Volna

Десктопный плеер YouTube Music для Linux и Windows, со своей «Моей волной» по образцу Яндекс Музыки. Написан на Rust и Tauri v2, своего сервера нет: всё работает внутри приложения. Интерфейс взят из [Monochrome](https://github.com/monochrome-music/monochrome) (Apache-2.0, см. `ui/NOTICE`) и переделан под YouTube Music.

Каталог, поиск, тексты и обложки Rust получает через InnerTube, звук достаёт yt-dlp. Вебвью сам в Google не ходит, поэтому выбранный прокси действует на все запросы сразу. Устройство кода описано в `docs/STRUCTURE.md`, план и решения лежат в `PLAN.md`.

## Запуск для разработки

Нужны Rust 1.90+, bun, а на Linux ещё webkit2gtk-4.1 и GStreamer (base, good, для AAC libav). yt-dlp и xray в dev-режиме берутся из `PATH`. Можно указать свои через `VOLNA_YTDLP` и `VOLNA_XRAY`.

```sh
cd ui
bun install
bun run tauri dev
```

Проверки:

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd ui && bun run test:node js/tests/network-settings-model.test.js   # и другие файлы из js/tests
```

## Сборка

```sh
scripts/build.sh                  # Linux: AppImage + deb
scripts/build.sh --bundles deb    # аргументы уходят в tauri build
```

Сначала скрипт вызывает `scripts/fetch-sidecars.sh`, если в `src-tauri/binaries/` ещё нет бинарей. Тот скачивает официальные релизы Xray-core и yt-dlp (версии зафиксированы в начале файла) и сверяет SHA-256 и с файлами контрольных сумм из релиза, и с хешами, записанными в скрипте. Потом идёт `tauri build` с `src-tauri/tauri.bundle.json`: в нём перечислены сайдкары, а обычные dev-сборки и `cargo test` обходятся без них. В пакете бинари называются `volna-xray` и `volna-yt-dlp`, чтобы в `/usr/bin` не пересечься с системными.

Готовые файлы появляются в `target/release/bundle/appimage/` и `target/release/bundle/deb/`. AppImage для Linux требует `patchelf` в `PATH`.

Windows-сборки делает только CI: `.github/workflows/build.yml` собирает NSIS-установщик на `windows-latest`, а заодно AppImage и deb на `ubuntu-22.04`. Ubuntu 22.04 там выбрана ради старой glibc: собранный на ней AppImage запустится почти на любом дистрибутиве, а собранный на CachyOS нет. Файлы скачиваются из артефактов workflow. Кросс-сборку Tauri с Linux на Windows мы не пробуем, она нестабильна.

## Сеть

Режим выбирается в «Настройки → Сеть», применяется на лету и сохраняется в `settings.json` в каталоге конфигурации приложения (на Linux `~/.config/uz.khudoberdi.volna/`, права 0600). При запуске сохранённый режим восстанавливается.

- Напрямую. Системный маршрут, включая системный VPN, если он поднят.
- Прокси URL. `socks5://`, `socks5h://` (DNS резолвит прокси), `http://` или `https://`, при необходимости с `user:pass@`. Подойдёт свой xray на ПК или `ssh -D`.
- VLESS-ссылка. Вставляете `vless://…` из 3x-ui. Приложение само запускает встроенный xray с SOCKS на 127.0.0.1 со случайным портом и паролем и пускает через него всё, включая yt-dlp и звук. Поддерживаются REALITY, TLS, xhttp, ws, grpc и tcp.

Кнопка «Проверить» показывает внешний IP, задержку, открывается ли YouTube Music с этого выхода и играет ли звук (yt-dlp пробует получить тестовый трек). Ниже выводится журнал xray. Ссылки на звук от googlevideo привязаны к IP, поэтому при смене режима кэш ссылок сбрасывается. Там же есть кнопка «Обновить yt-dlp»: она обновляет встроенный yt-dlp, а системный оставляет пакетному менеджеру.

Через IP датацентров (типичный VPS) YouTube почти на каждый трек отвечает yt-dlp «Sign in to confirm you're not a bot». Каталог при этом работает, а звук нет. Помогают cookies YouTube: в «Настройки → Сеть → Cookies YouTube для звука» выберите браузер, в котором вы вошли в YouTube (yt-dlp прочитает его cookies, `--cookies-from-browser`), или файл `cookies.txt` в формате Netscape. Cookies остаются на компьютере и сохраняются в том же `settings.json`. Если трек всё равно не играет, приложение пишет, почему, и переходит к следующему (после трёх неудач подряд останавливается).

## Правовая сторона

Volna неофициальный клиент. С Google, YouTube и Яндексом проект никак не связан, их названия упоминаются только чтобы описать, с чем работает приложение. Треки только проигрываются потоком, скачивания нет и не будет. Пользуясь приложением, вы сами отвечаете за соблюдение условий YouTube и Яндекс Музыки.

## Лицензия

Apache License 2.0, полный текст в `LICENSE`. Интерфейс в `ui/` основан на [Monochrome](https://github.com/monochrome-music/monochrome) под той же лицензией и изменён, подробности в `ui/NOTICE`.
