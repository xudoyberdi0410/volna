import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../api/scrobble.dart';
import '../../../state/settings.dart';
import '../platform.dart';
import '../widgets.dart';

/// Settings → «Скробблинг»: Last.fm, Libre.fm, ListenBrainz and Maloja. The
/// daemon scrobbles from the player, so it works with the window closed.
class ScrobblingSection extends ConsumerStatefulWidget {
  const ScrobblingSection({super.key});

  @override
  ConsumerState<ScrobblingSection> createState() => _ScrobblingSectionState();
}

class _ScrobblingSectionState extends ConsumerState<ScrobblingSection> {
  ScrobbleStatus? _status;
  String? _loadError;
  double? _pctDrag;
  bool _flushing = false;
  String? _flushMessage;

  ScrobbleApi get _api => ref.read(scrobbleApiProvider);

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    setState(() => _loadError = null);
    try {
      final s = await _api.status();
      if (mounted) setState(() => _status = s);
    } catch (e) {
      if (mounted) setState(() => _loadError = errorText(e));
    }
  }

  void _update(ScrobbleStatus s) {
    if (mounted) setState(() => _status = s);
  }

  Future<void> _flush() async {
    setState(() {
      _flushing = true;
      _flushMessage = null;
    });
    try {
      final s = await _api.flush();
      final left = s.services.fold(0, (n, x) => n + x.queue.pending);
      _update(s);
      if (mounted) setState(() => _flushMessage = left == 0 ? 'Очередь пуста' : 'Ещё в очереди: $left');
    } catch (e) {
      if (mounted) setState(() => _flushMessage = 'Не удалось: ${errorText(e)}');
    } finally {
      if (mounted) setState(() => _flushing = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final st = _status;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SectionHeader('Скробблинг',
            subtitle: 'Volna отправляет прослушанные треки сама, даже при закрытом окне. '
                'Без сети они ждут в очереди и уходят позже.'),
        if (_loadError != null)
          SettingsCard(icon: Icons.history, title: 'Скробблинг', children: [LoadFailed(_loadError!, onRetry: _load)])
        else if (st == null)
          const Padding(padding: EdgeInsets.all(32), child: Center(child: CircularProgressIndicator()))
        else ...[
          _commonCard(st),
          for (final svc in ScrobbleService.values)
            _ServiceCard(key: ValueKey(svc), status: st.of(svc), onStatus: _update),
        ],
      ],
    );
  }

  Widget _commonCard(ScrobbleStatus st) {
    final pct = _pctDrag ?? st.percentage.clamp(10, 100).toDouble();
    final pending = st.services.fold(0, (n, x) => n + x.queue.pending);
    return SettingsCard(
      icon: Icons.tune,
      title: 'Когда засчитывать',
      subtitle: 'Трек уходит в скробблер, когда прослушана эта часть или 4 минуты',
      trailing: pending > 0 ? StatusPill(Tone.warn, 'В очереди: $pending') : const StatusPill(Tone.good, 'Очередь пуста'),
      children: [
        Row(
          children: [
            SizedBox(
              width: 48,
              child: Text('${pct.round()}%',
                  style: Theme.of(context).textTheme.titleMedium?.copyWith(fontWeight: FontWeight.w600)),
            ),
            Expanded(
              child: Slider(
                key: const Key('scrobble-percentage'),
                value: pct,
                min: 10,
                max: 100,
                divisions: 18,
                label: '${pct.round()}%',
                onChanged: (v) => setState(() => _pctDrag = v),
                onChangeEnd: (v) async {
                  try {
                    _update(await _api.set(percentage: v.round()));
                  } catch (_) {}
                  if (mounted) setState(() => _pctDrag = null);
                },
              ),
            ),
          ],
        ),
        if (pending > 0) ...[
          ButtonRow([
            OutlinedButton.icon(
              onPressed: _flushing ? null : _flush,
              icon: _flushing ? const ButtonSpinner() : const Icon(Icons.send_outlined),
              label: const Text('Отправить очередь сейчас'),
            ),
          ]),
        ],
        if (_flushMessage != null) InlineMessage(Tone.neutral, _flushMessage!),
      ],
    );
  }
}

String _when(int unixSeconds) {
  final d = DateTime.fromMillisecondsSinceEpoch(unixSeconds * 1000);
  String two(int n) => n.toString().padLeft(2, '0');
  return '${two(d.hour)}:${two(d.minute)}';
}

String? _urlError(String v, {required bool required}) {
  final s = v.trim();
  if (s.isEmpty) return required ? 'Укажите адрес сервера' : null;
  final u = Uri.tryParse(s);
  if (u == null || !(u.scheme == 'http' || u.scheme == 'https') || u.host.isEmpty) {
    return 'Адрес должен начинаться с http:// или https://';
  }
  return null;
}

class _ServiceCard extends ConsumerStatefulWidget {
  final ServiceStatus status;
  final ValueChanged<ScrobbleStatus> onStatus;
  const _ServiceCard({super.key, required this.status, required this.onStatus});

  @override
  ConsumerState<_ServiceCard> createState() => _ServiceCardState();
}

class _ServiceCardState extends ConsumerState<_ServiceCard> {
  final _token = TextEditingController();
  final _url = TextEditingController();
  final _apiKey = TextEditingController();
  final _apiSecret = TextEditingController();
  bool _busy = false;
  (Tone, String)? _message;
  String? _loginUrl;
  bool _waitingBrowser = false;
  bool _showCustom = false;
  String? _urlFieldError;

  ScrobbleApi get _api => ref.read(scrobbleApiProvider);
  ServiceStatus get s => widget.status;

  @override
  void initState() {
    super.initState();
    _url.text = s.url ?? '';
    _apiKey.text = s.customApiKey ?? '';
    _showCustom = s.useCustomCredentials ?? false;
    _waitingBrowser = s.loginPending;
  }

  @override
  void dispose() {
    _token.dispose();
    _url.dispose();
    _apiKey.dispose();
    _apiSecret.dispose();
    super.dispose();
  }

  Future<void> _run(Future<ScrobbleStatus> Function() f, {String? ok}) async {
    setState(() {
      _busy = true;
      _message = null;
    });
    try {
      widget.onStatus(await f());
      if (ok != null && mounted) setState(() => _message = (Tone.good, ok));
    } catch (e) {
      if (mounted) setState(() => _message = (Tone.bad, errorText(e)));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _loginStart() async {
    setState(() {
      _busy = true;
      _message = null;
    });
    try {
      final url = await _api.loginStart(s.service);
      final opened = await ref.read(urlOpenerProvider)(Uri.parse(url));
      if (!mounted) return;
      setState(() {
        _loginUrl = url;
        _waitingBrowser = true;
        if (!opened) {
          _message = (Tone.warn, 'Не удалось открыть браузер. Скопируйте ссылку и откройте её сами.');
        }
      });
    } catch (e) {
      if (mounted) setState(() => _message = (Tone.bad, 'Не удалось начать вход: ${errorText(e)}'));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _loginFinish() async {
    setState(() {
      _busy = true;
      _message = null;
    });
    try {
      final name = await _api.loginFinish(s.service);
      final st = await _api.status();
      widget.onStatus(st);
      if (!mounted) return;
      setState(() {
        _waitingBrowser = false;
        _loginUrl = null;
        _message = (Tone.good, name == null ? 'Вход выполнен' : 'Вход выполнен: $name');
      });
    } catch (e) {
      if (mounted) {
        setState(() => _message = (
              Tone.bad,
              'Доступ ещё не подтверждён. Нажмите «Разрешить» на странице ${s.service.title} '
                  'и попробуйте снова. (${errorText(e)})'
            ));
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _saveServer() async {
    final needUrl = s.service == ScrobbleService.maloja;
    final err = _urlError(_url.text, required: needUrl);
    if (err != null) {
      setState(() => _urlFieldError = err);
      return;
    }
    final token = _token.text.trim();
    if (token.isEmpty && !s.hasToken) {
      setState(() => _message = (Tone.bad, needUrl ? 'Укажите ключ API Maloja' : 'Укажите токен ListenBrainz'));
      return;
    }
    await _run(
      () => _api.set(
        service: s.service,
        url: _url.text.trim(),
        token: token.isEmpty ? null : token,
        enabled: true,
      ),
      ok: 'Сохранено',
    );
    _token.clear();
  }

  Future<void> _saveCustom() async {
    final key = _apiKey.text.trim();
    final secret = _apiSecret.text.trim();
    if (_showCustom && (key.isEmpty || (secret.isEmpty && s.hasCustomApiSecret != true))) {
      setState(() => _message = (Tone.bad, 'Нужны и ключ, и секрет API'));
      return;
    }
    await _run(
      () => _api.set(
        service: s.service,
        useCustomCredentials: _showCustom,
        customApiKey: _showCustom ? key : null,
        customApiSecret: _showCustom && secret.isNotEmpty ? secret : null,
      ),
      ok: 'Сохранено. Войдите заново: сессия привязана к ключу.',
    );
    _apiSecret.clear();
  }

  (Tone, String) get _pill {
    if (!s.enabled) return (Tone.neutral, 'Выключено');
    if (s.active) return (Tone.good, s.username != null ? s.username! : 'Работает');
    if (s.usesLogin) return (Tone.warn, 'Нужен вход');
    return (Tone.warn, s.service == ScrobbleService.maloja ? 'Нужны адрес и ключ' : 'Нужен токен');
  }

  String get _subtitle => switch (s.service) {
        ScrobbleService.lastfm => 'last.fm: история прослушиваний и рекомендации',
        ScrobbleService.librefm => 'libre.fm: свободная альтернатива Last.fm',
        ScrobbleService.listenbrainz => 'Открытая база прослушиваний MetaBrainz',
        ScrobbleService.maloja => 'Свой сервер Maloja',
      };

  IconData get _icon => switch (s.service) {
        ScrobbleService.lastfm => Icons.radio,
        ScrobbleService.librefm => Icons.album_outlined,
        ScrobbleService.listenbrainz => Icons.hearing,
        ScrobbleService.maloja => Icons.dns_outlined,
      };

  @override
  Widget build(BuildContext context) {
    final (tone, label) = _pill;
    final t = Theme.of(context).textTheme;
    final c = Theme.of(context).colorScheme;
    final loggedIn = s.usesLogin && s.username != null;
    return SettingsCard(
      icon: _icon,
      title: s.service.title,
      subtitle: _subtitle,
      trailing: StatusPill(tone, label),
      children: [
        SwitchRow(
          title: 'Отправлять прослушивания',
          value: s.enabled,
          onChanged: _busy ? null : (v) => _run(() => _api.set(service: s.service, enabled: v)),
        ),
        if (s.canLove)
          SwitchRow(
            title: 'Любимые треки',
            subtitle: 'Лайк в Volna ставит «love» и в ${s.service.title}',
            value: s.loveOnLike,
            onChanged: _busy ? null : (v) => _run(() => _api.set(service: s.service, loveOnLike: v)),
          ),
        const SizedBox(height: 8),
        if (s.usesLogin) ...[
          if (loggedIn && !_waitingBrowser)
            Row(
              children: [
                CircleAvatar(
                  radius: 16,
                  backgroundColor: c.secondaryContainer,
                  child: Icon(Icons.person, size: 18, color: c.onSecondaryContainer),
                ),
                const SizedBox(width: 10),
                Expanded(child: Text('Вы вошли как ${s.username}', style: t.bodyLarge)),
                TextButton(
                  onPressed: _busy ? null : () => _run(() => _api.logout(s.service)),
                  child: const Text('Выйти'),
                ),
              ],
            )
          else if (_waitingBrowser)
            _browserStep()
          else
            ButtonRow([
              FilledButton.icon(
                onPressed: _busy ? null : _loginStart,
                icon: _busy ? const ButtonSpinner() : const Icon(Icons.open_in_browser),
                label: Text('Войти через ${s.service.title}'),
              ),
            ]),
          if (s.service == ScrobbleService.lastfm) _customCredentials(),
        ] else
          _serverForm(),
        if (s.queue.pending > 0 || s.queue.lastError != null)
          InlineMessage(
            s.queue.lastError != null ? Tone.warn : Tone.neutral,
            [
              if (s.queue.pending > 0) 'В очереди: ${s.queue.pending}',
              if (s.queue.nextRetryAt != null) 'следующая попытка в ${_when(s.queue.nextRetryAt!)}',
              if (s.queue.lastError != null) 'последняя ошибка: ${s.queue.lastError}',
            ].join(' · '),
          ),
        if (_message != null) InlineMessage(_message!.$1, _message!.$2),
      ],
    );
  }

  Widget _browserStep() {
    final c = Theme.of(context).colorScheme;
    final t = Theme.of(context).textTheme;
    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: c.secondaryContainer.withValues(alpha: 0.5),
        borderRadius: BorderRadius.circular(14),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('Подтвердите доступ в браузере', style: t.titleSmall?.copyWith(fontWeight: FontWeight.w600)),
          const SizedBox(height: 4),
          Text(
            'На странице ${s.service.title} нажмите «Разрешить» (Allow access), затем вернитесь сюда.',
            style: t.bodyMedium,
          ),
          const SizedBox(height: 12),
          Wrap(
            spacing: 10,
            runSpacing: 8,
            children: [
              FilledButton.icon(
                onPressed: _busy ? null : _loginFinish,
                icon: _busy ? const ButtonSpinner() : const Icon(Icons.check),
                label: const Text('Я разрешил доступ'),
              ),
              if (_loginUrl != null)
                TextButton.icon(
                  onPressed: () => Clipboard.setData(ClipboardData(text: _loginUrl!)),
                  icon: const Icon(Icons.copy, size: 18),
                  label: const Text('Копировать ссылку'),
                ),
              if (_loginUrl == null)
                TextButton(onPressed: _busy ? null : _loginStart, child: const Text('Открыть страницу снова')),
              TextButton(
                onPressed: _busy ? null : () => setState(() => _waitingBrowser = false),
                child: const Text('Отмена'),
              ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _customCredentials() {
    return Theme(
      data: Theme.of(context).copyWith(dividerColor: Colors.transparent),
      child: ExpansionTile(
        tilePadding: EdgeInsets.zero,
        childrenPadding: const EdgeInsets.only(bottom: 8),
        title: Text('Свой ключ API', style: Theme.of(context).textTheme.bodyLarge),
        subtitle: Text(
          s.useCustomCredentials == true ? 'Используется ключ ${s.customApiKey ?? ''}' : 'Необязательно',
          style: Theme.of(context).textTheme.bodySmall,
        ),
        initiallyExpanded: s.useCustomCredentials == true,
        children: [
          SwitchRow(
            title: 'Использовать свой ключ',
            subtitle: 'Ключ и секрет со страницы last.fm/api/account/create',
            value: _showCustom,
            onChanged: (v) => setState(() => _showCustom = v),
          ),
          if (_showCustom) ...[
            const SizedBox(height: 8),
            TextField(controller: _apiKey, decoration: fieldDecoration(label: 'API key')),
            const SizedBox(height: 12),
            TextField(
              controller: _apiSecret,
              obscureText: true,
              decoration: fieldDecoration(
                label: 'Shared secret',
                hint: s.hasCustomApiSecret == true ? 'сохранён; оставьте пустым, чтобы не менять' : null,
              ),
            ),
          ],
          ButtonRow([
            OutlinedButton(onPressed: _busy ? null : _saveCustom, child: const Text('Сохранить ключ')),
          ]),
        ],
      ),
    );
  }

  Widget _serverForm() {
    final maloja = s.service == ScrobbleService.maloja;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SizedBox(height: 4),
        TextField(
          key: Key('${s.service.name}-token'),
          controller: _token,
          obscureText: true,
          decoration: fieldDecoration(
            label: maloja ? 'Ключ API' : 'Токен пользователя',
            hint: s.hasToken ? 'сохранён; введите новый, чтобы заменить' : null,
            helper: maloja ? null : 'Страница listenbrainz.org/settings, поле User token',
            suffix: s.hasToken ? const Icon(Icons.lock_outline, size: 18) : null,
          ),
        ),
        const SizedBox(height: 12),
        TextField(
          key: Key('${s.service.name}-url'),
          controller: _url,
          onChanged: (_) {
            if (_urlFieldError != null) setState(() => _urlFieldError = null);
          },
          decoration: fieldDecoration(
            label: maloja ? 'Адрес сервера' : 'Свой сервер (необязательно)',
            hint: maloja ? 'https://maloja.example.com' : 'https://api.listenbrainz.org',
            error: _urlFieldError,
          ),
        ),
        ButtonRow([
          FilledButton.tonal(
            onPressed: _busy ? null : _saveServer,
            child: _busy ? const ButtonSpinner() : const Text('Сохранить'),
          ),
          if (s.hasToken)
            TextButton(
              onPressed: _busy ? null : () => _run(() => _api.logout(s.service)),
              child: const Text('Забыть токен'),
            ),
        ]),
      ],
    );
  }
}
