/// Pure helpers of Settings → «Сеть» (a port of `ui/js/network-settings-model.js`):
/// form checks and the Russian texts of statuses and checks.
library;

import '../../api/network.dart';
import 'widgets.dart';

const proxySchemes = ['socks5', 'socks5h', 'http', 'https'];

/// Browsers offered for cookies (yt-dlp names).
const cookieBrowsers = ['firefox', 'chrome', 'chromium', 'brave', 'edge', 'opera', 'vivaldi'];

/// Quick client-side check of a proxy setting; the daemon validates again.
/// Returns an error message or null.
String? precheckProxy(ProxyConfig config) {
  if (config.mode == NetMode.direct) return null;
  final value = config.value.trim();
  if (value.isEmpty) {
    return config.mode == NetMode.url ? 'Укажите адрес прокси' : 'Вставьте ссылку vless://';
  }
  if (config.mode == NetMode.vless) {
    return value.toLowerCase().startsWith('vless://') ? null : 'Ссылка должна начинаться с vless://';
  }
  final uri = Uri.tryParse(value);
  if (uri == null || !uri.hasScheme || uri.host.isEmpty) {
    return 'Неверный адрес, пример: socks5h://127.0.0.1:1080';
  }
  if (!proxySchemes.contains(uri.scheme.toLowerCase())) {
    return 'Поддерживаются socks5://, socks5h://, http:// и https://';
  }
  return null;
}

const modeLabels = {
  NetMode.direct: 'Напрямую',
  NetMode.url: 'Прокси',
  NetMode.vless: 'VLESS (xray)',
};

/// One line for the active connection, e.g. «VLESS (xray) · xray работает (pid 123)».
String describeStatus(NetStatus s) {
  final mode = NetMode.values.where((m) => m.name == s.mode).firstOrNull;
  final parts = [mode == null ? s.mode : modeLabels[mode]!];
  if (s.proxy != null && s.mode == 'url') parts.add(s.proxy!);
  if (s.xrayRunning == true) {
    parts.add(s.xrayPid != null ? 'xray работает (pid ${s.xrayPid})' : 'xray работает');
  } else if (s.xrayRunning == false) {
    parts.add('xray остановлен');
  }
  return parts.join(' · ');
}

/// The «Звук» row: can yt-dlp resolve audio through this connection?
(String, Tone) describeAudio(AudioCheck? check, {bool pending = false}) {
  if (pending) return ('проверяю…', Tone.busy);
  if (check == null) return ('неизвестно', Tone.warn);
  if (check.ok) return ('работает (${check.latencyMs} мс)', Tone.good);
  return switch (check.kind) {
    FailureKind.botCheck => (
        'не играет: YouTube просит подтвердить, что вы не бот, для этого IP. '
            'Выберите другое подключение или включите cookies ниже',
        Tone.bad
      ),
    FailureKind.unavailable => ('не удалось проверить (тестовый трек недоступен)', Tone.warn),
    FailureKind.noYtDlp => ('yt-dlp не найден', Tone.bad),
    _ => ('ошибка: ${check.message ?? 'неизвестная'}', Tone.bad),
  };
}

/// Shortens the reqwest error chain for the most common proxy failures.
String humanizeNetError(String e) {
  if (RegExp(r'socks proxy|proxy.*(refused|connect)|tunnel', caseSensitive: false).hasMatch(e)) {
    return 'Прокси не отвечает. $e';
  }
  if (RegExp(r'timed? ?out', caseSensitive: false).hasMatch(e)) return 'Нет ответа (тайм-аут). $e';
  if (RegExp(r'dns|resolve|lookup', caseSensitive: false).hasMatch(e)) {
    return 'Не удалось найти адрес (DNS). $e';
  }
  return e;
}

Tone _worse(Tone a, Tone b) {
  const rank = {Tone.good: 0, Tone.neutral: 0, Tone.busy: 0, Tone.warn: 1, Tone.bad: 2};
  return rank[b]! > rank[a]! ? b : a;
}

/// A health report (and the audio check) as rows plus an overall tone.
(Tone, List<(String, String, Tone?)>) describeHealth(HealthReport r,
    {AudioCheck? audio, bool audioPending = false}) {
  final ytm = switch (r.ytmAvailable) {
    true => ('доступна', Tone.good),
    false => ('недоступна в этой стране', Tone.bad),
    null => ('неизвестно', Tone.warn),
  };
  final rows = <(String, String, Tone?)>[
    ('Внешний IP', r.exitIp ?? '—', null),
    ('Задержка', r.latencyMs != null ? '${r.latencyMs} мс' : '—', null),
    ('YouTube Music', ytm.$1, r.ok ? ytm.$2 : null),
  ];
  var tone = Tone.bad;
  if (r.ok && r.ytmAvailable != false) tone = r.ytmAvailable == true ? Tone.good : Tone.warn;
  if (r.ok && (audio != null || audioPending)) {
    final (text, t) = describeAudio(audio, pending: audioPending);
    rows.add(('Звук', text, audioPending ? null : t));
    if (!audioPending) tone = _worse(tone, t);
  }
  if (r.error != null) rows.add(('Ошибка', humanizeNetError(r.error!), Tone.bad));
  return (tone, rows);
}

String originLabel(YtDlpOrigin o) => switch (o) {
      YtDlpOrigin.sidecar => 'встроенный',
      YtDlpOrigin.system => 'системный',
      YtDlpOrigin.configured => 'из VOLNA_YTDLP',
    };

/// Form values → cookies config. [choice] is `none`, `file` or a browser.
CookiesConfig buildCookies(String choice, String extra) {
  final v = extra.trim();
  return switch (choice) {
    'none' => const CookiesConfig.none(),
    'file' => CookiesConfig(CookiesMode.file, v),
    _ => CookiesConfig(CookiesMode.browser, v.isEmpty ? choice : '$choice:$v'),
  };
}

/// Saved cookies config → (choice, extra).
(String, String) cookiesForm(CookiesConfig c) {
  if (c.mode == CookiesMode.file) return ('file', c.value);
  if (c.mode == CookiesMode.browser && c.value.isNotEmpty) {
    final parts = c.value.split(':');
    if (cookieBrowsers.contains(parts.first)) return (parts.first, parts.skip(1).join(':'));
  }
  return ('none', '');
}
