Первая публичная сборка Volna, неофициального десктоп-плеера YouTube Music с «Моей волной».

## Что скачать

- Windows: `Volna_*_x64-setup.exe`, обычный установщик. GStreamer, yt-dlp и xray уже внутри.
- Arch Linux и производные (CachyOS, EndeavourOS, Manjaro): `volna-*-x86_64.pkg.tar.zst`.

Проверить файлы можно по `SHA256SUMS.txt`: `sha256sum -c SHA256SUMS.txt --ignore-missing`.

## Установка на Arch

Пакет зависит от `bgutil-ytdlp-pot-provider` из AUR, поэтому сначала поставьте его, потом сам пакет:

```sh
paru -S bgutil-ytdlp-pot-provider        # или yay
sudo pacman -U volna-*-x86_64.pkg.tar.zst
systemctl --user enable --now bgutil-pot
```

## Перед первым запуском на Windows

Отдельно нужен [Deno](https://deno.com): через него yt-dlp разбирает плеер YouTube, и без него часть треков не играет.

```
winget install DenoLand.Deno
```

## Сеть

Если YouTube Music недоступен в вашей стране, укажите прокси или VLESS-ссылку в «Настройки → Сеть». Если звук не идёт с ошибкой «Sign in to confirm you're not a bot», там же выберите браузер, в котором вы вошли в YouTube: yt-dlp возьмёт его cookies.

## Известные ограничения

Сборку под Windows собрали кросс-компиляцией и ещё не запускали на живой машине, о проблемах пишите в issues. Провайдера PO token для Windows в установщике нет, поэтому часть треков там может не играть. Скачивания треков нет и не будет. Приложение не связано с Google, YouTube и Яндексом.
