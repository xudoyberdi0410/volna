import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../api/network.dart';
import '../../../state/settings.dart';
import '../network_text.dart';
import '../widgets.dart';

/// Settings → «Сеть»: direct / proxy URL / VLESS, the connection check,
/// YouTube cookies for yt-dlp and the xray log.
class NetworkSection extends ConsumerStatefulWidget {
  const NetworkSection({super.key});

  @override
  ConsumerState<NetworkSection> createState() => _NetworkSectionState();
}

class _NetworkSectionState extends ConsumerState<NetworkSection> {
  NetSettings? _view;
  String? _loadError;

  NetMode _mode = NetMode.direct;
  final _url = TextEditingController();
  final _vless = TextEditingController();
  String? _fieldError;
  Timer? _validateTimer;
  int _validateSeq = 0;

  bool _applying = false;
  (Tone, String)? _applyMessage;

  bool _checking = false;
  HealthReport? _health;
  AudioCheck? _audio;
  bool _audioPending = false;
  String? _checkError;

  String _cookieChoice = 'none';
  final _cookieExtra = TextEditingController();
  bool _savingCookies = false;
  (Tone, String)? _cookiesMessage;

  List<String>? _logs;
  bool _showLogs = false;

  NetworkApi get _api => ref.read(networkApiProvider);

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    _validateTimer?.cancel();
    _url.dispose();
    _vless.dispose();
    _cookieExtra.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    setState(() => _loadError = null);
    try {
      final v = await _api.settings();
      if (!mounted) return;
      final firstLoad = _view == null;
      setState(() {
        _view = v;
        if (firstLoad) {
          _mode = v.config.mode;
          _url.text = v.config.mode == NetMode.url ? v.config.value : '';
          _vless.text = v.config.mode == NetMode.vless ? v.config.value : '';
          final (choice, extra) = cookiesForm(v.cookies);
          _cookieChoice = choice;
          _cookieExtra.text = extra;
        }
      });
      if (v.status.mode == 'vless') unawaited(_refreshLogs());
    } catch (e) {
      if (mounted) setState(() => _loadError = errorText(e));
    }
  }

  ProxyConfig get _config => switch (_mode) {
        NetMode.direct => const ProxyConfig.direct(),
        NetMode.url => ProxyConfig(NetMode.url, _url.text),
        NetMode.vless => ProxyConfig(NetMode.vless, _vless.text),
      };

  /// Inline validation: the quick local check at once, the daemon's
  /// (`net_validate`, the real parser) after a pause in typing.
  void _onFieldChanged() {
    _validateTimer?.cancel();
    final config = _config;
    final local = precheckProxy(config);
    setState(() {
      _fieldError = config.value.trim().isEmpty ? null : local;
      _applyMessage = null;
    });
    if (local != null) return;
    final seq = ++_validateSeq;
    _validateTimer = Timer(const Duration(milliseconds: 400), () async {
      String? err;
      try {
        await _api.validate(config);
      } catch (e) {
        err = errorText(e);
      }
      if (mounted && seq == _validateSeq) setState(() => _fieldError = err);
    });
  }

  Future<void> _apply() async {
    final config = _config;
    final problem = precheckProxy(config);
    if (problem != null) {
      setState(() => _fieldError = problem);
      return;
    }
    setState(() {
      _applying = true;
      _applyMessage = (Tone.busy, config.mode == NetMode.vless ? 'Запускаю xray…' : 'Применяю…');
    });
    try {
      final status = await _api.apply(config);
      if (!mounted) return;
      setState(() {
        _view = NetSettings(config, status, null, _view?.cookies ?? const CookiesConfig.none());
        _applyMessage = (Tone.busy, 'Применено. Проверяю соединение…');
      });
      final tone = await _check();
      if (!mounted) return;
      setState(() => _applyMessage = tone == Tone.bad
          ? (Tone.bad, 'Применено, но соединение не работает (подробности в проверке ниже)')
          : (Tone.good, 'Применено'));
      if (config.mode == NetMode.vless) unawaited(_refreshLogs());
    } catch (e) {
      if (!mounted) return;
      setState(() => _applyMessage = (Tone.bad, 'Не удалось применить: ${errorText(e)}'));
      if (config.mode == NetMode.vless) {
        _showLogs = true;
        unawaited(_refreshLogs());
      }
    } finally {
      if (mounted) setState(() => _applying = false);
    }
  }

  /// Checks the active connection; resolves to its overall tone.
  Future<Tone> _check() async {
    setState(() {
      _checking = true;
      _checkError = null;
      _health = null;
      _audio = null;
    });
    final audio = _api.audioCheck().catchError(
        (Object e) => AudioCheck(ok: false, kind: FailureKind.other, message: errorText(e)));
    try {
      final report = await _api.health();
      if (!mounted) return Tone.bad;
      setState(() {
        _health = report;
        _audioPending = report.ok;
      });
      if (!report.ok) return describeHealth(report).$1;
      final a = await audio;
      if (!mounted) return Tone.bad;
      setState(() {
        _audio = a;
        _audioPending = false;
      });
      return describeHealth(report, audio: a).$1;
    } catch (e) {
      if (mounted) setState(() => _checkError = 'Ошибка проверки: ${errorText(e)}');
      return Tone.bad;
    } finally {
      if (mounted) setState(() => _checking = false);
    }
  }

  Future<void> _saveCookies() async {
    final cookies = buildCookies(_cookieChoice, _cookieExtra.text);
    if (cookies.mode == CookiesMode.file && cookies.value.isEmpty) {
      setState(() => _cookiesMessage = (Tone.bad, 'Укажите путь к cookies.txt'));
      return;
    }
    setState(() {
      _savingCookies = true;
      _cookiesMessage = (Tone.busy, 'Сохраняю…');
    });
    try {
      await _api.setCookies(cookies);
      if (!mounted) return;
      setState(() => _cookiesMessage = (Tone.busy, 'Сохранено. Проверяю звук…'));
      final check = await _api.audioCheck();
      final (text, tone) = describeAudio(check);
      if (!mounted) return;
      setState(() => _cookiesMessage = check.kind == FailureKind.botCheck && cookies.mode != CookiesMode.none
          ? (
              Tone.bad,
              'Звук не играет: YouTube всё ещё просит подтвердить, что вы не бот. '
                  'Войдите в YouTube в этом браузере или экспортируйте свежий cookies.txt'
            )
          : (tone, 'Сохранено. Звук: $text'));
    } catch (e) {
      if (mounted) setState(() => _cookiesMessage = (Tone.bad, 'Не удалось: ${errorText(e)}'));
    } finally {
      if (mounted) setState(() => _savingCookies = false);
    }
  }

  Future<void> _refreshLogs() async {
    try {
      final lines = await _api.xrayLogs();
      if (mounted) setState(() => _logs = lines);
    } catch (e) {
      if (mounted) setState(() => _logs = [errorText(e)]);
    }
  }

  @override
  Widget build(BuildContext context) {
    final view = _view;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SectionHeader('Сеть',
            subtitle: 'Как Volna выходит в интернет. Каталог, обложки и звук идут через одно и то же '
                'подключение: ссылки на звук привязаны к IP, с которого их получили.'),
        if (_loadError != null)
          SettingsCard(
              icon: Icons.lan_outlined,
              title: 'Подключение',
              children: [LoadFailed(_loadError!, onRetry: _load)])
        else if (view == null)
          const Padding(padding: EdgeInsets.all(32), child: Center(child: CircularProgressIndicator()))
        else ...[
          _connectionCard(view),
          _checkCard(),
          _cookiesCard(),
          if (_showLogs || view.status.mode == 'vless' || _logs != null) _logsCard(),
        ],
      ],
    );
  }

  Widget _connectionCard(NetSettings view) {
    final status = view.status;
    final running = status.mode != 'vless' || status.xrayRunning != false;
    return SettingsCard(
      icon: Icons.lan_outlined,
      title: 'Подключение',
      subtitle: 'Сейчас: ${describeStatus(status)}',
      trailing: StatusPill(running ? Tone.good : Tone.bad, running ? 'Активно' : 'xray остановлен'),
      children: [
        if (view.startupError != null)
          Padding(
            padding: const EdgeInsets.only(bottom: 12),
            child: InlineMessage(
                Tone.bad, 'Сохранённая настройка не применилась при запуске: ${view.startupError}'),
          ),
        SegmentedButton<NetMode>(
          segments: const [
            ButtonSegment(value: NetMode.direct, label: Text('Напрямую'), icon: Icon(Icons.public)),
            ButtonSegment(value: NetMode.url, label: Text('Прокси'), icon: Icon(Icons.swap_horiz)),
            ButtonSegment(value: NetMode.vless, label: Text('VLESS'), icon: Icon(Icons.vpn_lock_outlined)),
          ],
          selected: {_mode},
          showSelectedIcon: false,
          onSelectionChanged: _applying
              ? null
              : (s) {
                  setState(() {
                    _mode = s.first;
                    _applyMessage = null;
                  });
                  _onFieldChanged();
                },
        ),
        const SizedBox(height: 16),
        ...switch (_mode) {
          NetMode.direct => [
              Text(
                'Системный маршрут, включая системный VPN, если он есть. Если YouTube Music пишет '
                '«not available in your area», выберите прокси или VLESS с выходом в другой стране.',
                style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                    color: Theme.of(context).colorScheme.onSurfaceVariant),
              ),
            ],
          NetMode.url => [
              const FieldLabel('Адрес прокси'),
              TextField(
                key: const Key('net-url'),
                controller: _url,
                enabled: !_applying,
                onChanged: (_) => _onFieldChanged(),
                onSubmitted: (_) => _apply(),
                decoration: fieldDecoration(
                  hint: 'socks5h://127.0.0.1:1080',
                  helper: 'socks5://, socks5h://, http:// или https://, можно с user:pass@. '
                      'Например, ssh -D 1080 до своего сервера.',
                  error: _fieldError,
                ),
              ),
            ],
          NetMode.vless => [
              const FieldLabel('Ссылка vless://'),
              TextField(
                key: const Key('net-vless'),
                controller: _vless,
                enabled: !_applying,
                minLines: 2,
                maxLines: 4,
                onChanged: (_) => _onFieldChanged(),
                style: const TextStyle(fontFamily: 'monospace', fontSize: 13),
                decoration: fieldDecoration(
                  hint: 'vless://uuid@host:443?security=reality&…',
                  helper: 'Ссылка из панели вашего сервера. Volna запустит встроенный xray.',
                  error: _fieldError,
                ),
              ),
            ],
        },
        ButtonRow([
          FilledButton.icon(
            onPressed: _applying || _fieldError != null ? null : _apply,
            icon: _applying ? const ButtonSpinner() : const Icon(Icons.check),
            label: const Text('Применить'),
          ),
        ]),
        if (_applyMessage != null) InlineMessage(_applyMessage!.$1, _applyMessage!.$2),
      ],
    );
  }

  Widget _checkCard() {
    final health = _health;
    final (tone, rows) = health == null
        ? (Tone.neutral, const <(String, String, Tone?)>[])
        : describeHealth(health, audio: _audio, audioPending: _audioPending);
    final pill = _checking
        ? const StatusPill(Tone.busy, 'Проверяю')
        : health == null
            ? null
            : StatusPill(tone, switch (tone) {
                Tone.good => 'Всё работает',
                Tone.warn => 'Есть вопросы',
                _ => 'Не работает',
              });
    return SettingsCard(
      icon: Icons.network_check,
      title: 'Проверка соединения',
      subtitle: 'music.youtube.com, внешний IP и пробный трек через yt-dlp',
      trailing: pill,
      children: [
        if (rows.isNotEmpty) KeyValueTable(rows),
        if (_checkError != null) InlineMessage(Tone.bad, _checkError!),
        if (health == null && _checkError == null && _checking)
          const InlineMessage(Tone.busy, 'Проверяю music.youtube.com…'),
        ButtonRow([
          OutlinedButton.icon(
            onPressed: _checking || _applying ? null : _check,
            icon: const Icon(Icons.speed),
            label: const Text('Проверить'),
          ),
        ]),
      ],
    );
  }

  Widget _cookiesCard() {
    final file = _cookieChoice == 'file';
    return SettingsCard(
      icon: Icons.cookie_outlined,
      title: 'Cookies YouTube',
      subtitle: 'С IP датацентра YouTube просит «подтвердить, что вы не бот». '
          'Сессия браузера, где вы вошли в YouTube, помогает пройти проверку.',
      children: [
        const FieldLabel('Откуда брать cookies'),
        DropdownButtonFormField<String>(
          key: const Key('net-cookies'),
          initialValue: _cookieChoice,
          isExpanded: true,
          decoration: fieldDecoration(),
          items: [
            const DropdownMenuItem(value: 'none', child: Text('Не использовать')),
            for (final b in cookieBrowsers) DropdownMenuItem(value: b, child: Text(_browserName(b))),
            const DropdownMenuItem(value: 'file', child: Text('Файл cookies.txt')),
          ],
          onChanged: _savingCookies
              ? null
              : (v) => setState(() {
                    _cookieChoice = v ?? 'none';
                    _cookiesMessage = null;
                  }),
        ),
        if (_cookieChoice != 'none') ...[
          const SizedBox(height: 14),
          FieldLabel(file ? 'Путь к cookies.txt' : 'Профиль браузера'),
          TextField(
            controller: _cookieExtra,
            decoration: fieldDecoration(
              hint: file ? '/home/user/cookies.txt' : 'default-release',
              helper: file
                  ? 'Файл в формате Netscape (расширение «Get cookies.txt» и подобные)'
                  : 'Необязательно: имя профиля, если их несколько',
            ),
          ),
        ],
        ButtonRow([
          FilledButton.tonalIcon(
            onPressed: _savingCookies ? null : _saveCookies,
            icon: _savingCookies ? const ButtonSpinner() : const Icon(Icons.save_outlined),
            label: const Text('Сохранить и проверить звук'),
          ),
        ]),
        if (_cookiesMessage != null) InlineMessage(_cookiesMessage!.$1, _cookiesMessage!.$2),
      ],
    );
  }

  Widget _logsCard() {
    final c = Theme.of(context).colorScheme;
    final logs = _logs;
    return SettingsCard(
      icon: Icons.terminal,
      title: 'Журнал xray',
      subtitle: 'Последние строки встроенного xray: пригодятся, если VLESS не подключается',
      trailing: IconButton(
        tooltip: 'Обновить',
        onPressed: _refreshLogs,
        icon: const Icon(Icons.refresh),
      ),
      children: [
        Container(
          constraints: const BoxConstraints(maxHeight: 260),
          decoration: BoxDecoration(
            color: c.surfaceContainerHighest.withValues(alpha: 0.6),
            borderRadius: BorderRadius.circular(12),
          ),
          padding: const EdgeInsets.all(12),
          child: SingleChildScrollView(
            reverse: true,
            child: SelectableText(
              logs == null ? 'Загружаю…' : (logs.isEmpty ? '(пусто)' : logs.reversed.take(200).toList().reversed.join('\n')),
              style: const TextStyle(fontFamily: 'monospace', fontSize: 12, height: 1.4),
            ),
          ),
        ),
      ],
    );
  }
}

String _browserName(String b) => switch (b) {
      'firefox' => 'Firefox',
      'chrome' => 'Google Chrome',
      'chromium' => 'Chromium',
      'brave' => 'Brave',
      'edge' => 'Microsoft Edge',
      'opera' => 'Opera',
      'vivaldi' => 'Vivaldi',
      _ => b,
    };
