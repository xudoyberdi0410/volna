Первая публичная сборка Volna, неофициального десктоп-плеера YouTube Music с «Моей волной».

## Что скачать

- Windows: `Volna_*_x64-setup.exe`, обычный установщик.
- Linux: `Volna_*_amd64.AppImage` запускается на большинстве дистрибутивов без установки (`chmod +x`, потом запуск). Для Debian и Ubuntu есть `.deb`.

Проверить файлы можно по `SHA256SUMS.txt`: `sha256sum -c SHA256SUMS.txt --ignore-missing`.

## Перед первым запуском

yt-dlp и xray уже лежат внутри. Отдельно нужен [Deno](https://deno.com): через него yt-dlp разбирает плеер YouTube, и без него часть треков не играет.

- Windows: `winget install DenoLand.Deno`
- Linux: пакет `deno` из репозитория дистрибутива или `curl -fsSL https://deno.land/install.sh | sh`

Если YouTube Music недоступен в вашей стране, укажите прокси или VLESS-ссылку в «Настройки → Сеть». Если звук не идёт с ошибкой «Sign in to confirm you're not a bot», там же выберите браузер, в котором вы вошли в YouTube: yt-dlp возьмёт его cookies.

## Известные ограничения

Сборку под Windows пока почти не проверяли вживую, о проблемах пишите в issues. Скачивания треков нет и не будет. Приложение не связано с Google, YouTube и Яндексом.
