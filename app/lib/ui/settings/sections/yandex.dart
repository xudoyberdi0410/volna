import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../api/yandex.dart';
import '../../../state/settings.dart';
import '../platform.dart';
import '../widgets.dart';

/// Settings → «Импорт из Яндекс Музыки»: likes, playlists, albums and
/// artists, matched to YouTube Music by the daemon.
class YandexSection extends ConsumerStatefulWidget {
  const YandexSection({super.key});

  @override
  ConsumerState<YandexSection> createState() => _YandexSectionState();
}

class _YandexSectionState extends ConsumerState<YandexSection> {
  YandexImportOptions _options = const YandexImportOptions();
  final _paste = TextEditingController();
  bool _showPaste = false;
  bool _signingIn = false;
  String? _pasteError;
  String? _loginMessage;

  YandexImportController get _ctl => ref.read(yandexImportProvider.notifier);

  @override
  void initState() {
    super.initState();
    Future.microtask(_ctl.refresh);
  }

  @override
  void dispose() {
    _paste.dispose();
    super.dispose();
  }

  /// The sign-in window when WebKitGTK is there; else the paste form.
  Future<void> _loginWindow() async {
    setState(() {
      _signingIn = true;
      _loginMessage = null;
    });
    try {
      final url = await ref.read(webLoginProvider).open(yandexAuthorizeUrl(), title: 'Вход в Яндекс');
      if (url == null) {
        if (mounted) setState(() => _loginMessage = 'Окно входа закрыто');
        return;
      }
      await _useToken(url);
    } on PlatformException catch (e) {
      if (mounted) {
        setState(() {
          _showPaste = true;
          _loginMessage = 'Окно входа не открылось (${e.message}). Вставьте токен вручную.';
        });
      }
    } finally {
      if (mounted) setState(() => _signingIn = false);
    }
  }

  Future<void> _useToken(String input) async {
    final wasNeedLogin = ref.read(yandexImportProvider).done?.needLogin ?? false;
    final ok = await _ctl.setToken(input);
    if (!mounted) return;
    if (!ok) {
      setState(() => _pasteError = ref.read(yandexImportProvider).error);
      return;
    }
    setState(() {
      _paste.clear();
      _pasteError = null;
      _showPaste = false;
    });
    // The import stopped for a login: go on with it.
    if (wasNeedLogin) await _ctl.start(_options);
  }

  Future<void> _submitPaste() async {
    final v = _paste.text.trim();
    if (v.isEmpty) {
      setState(() => _pasteError = 'Вставьте адрес страницы или токен');
      return;
    }
    if (v.startsWith('http') && tokenFromRedirect(v) == null) {
      setState(() => _pasteError = 'В адресе нет access_token. Скопируйте его после того, как разрешите доступ');
      return;
    }
    setState(() => _signingIn = true);
    try {
      await _useToken(v);
    } finally {
      if (mounted) setState(() => _signingIn = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final st = ref.watch(yandexImportProvider);
    final webview = ref.watch(webLoginAvailableProvider).value ?? false;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SectionHeader('Импорт из Яндекс Музыки',
            subtitle: 'Перенесёт лайки, плейлисты, альбомы и артистов в библиотеку Volna. '
                'Каждый трек ищется на YouTube Music; не найденные будут в списке в конце.'),
        _accountCard(st, webview),
        _importCard(st),
        if (st.done != null) _resultCard(st.done!, webview),
      ],
    );
  }

  Widget _accountCard(YandexImportState st, bool webview) {
    final s = st.status;
    final t = Theme.of(context).textTheme;
    final c = Theme.of(context).colorScheme;
    final pill = s == null
        ? (st.error != null ? const StatusPill(Tone.bad, 'Нет связи') : const StatusPill(Tone.busy, 'Проверяю'))
        : s.signedIn
            ? StatusPill(Tone.good, s.login ?? 'Вход выполнен')
            : const StatusPill(Tone.neutral, 'Не выполнен');
    return SettingsCard(
      icon: Icons.account_circle_outlined,
      title: 'Аккаунт Яндекса',
      subtitle: s?.signedIn == true
          ? 'Токен проверен, можно импортировать'
          : 'Если вы вошли в Яндекс в браузере, Volna возьмёт вход оттуда сама',
      trailing: pill,
      children: [
        if (s?.signedIn != true) ...[
          Wrap(
            spacing: 10,
            runSpacing: 10,
            children: [
              if (webview)
                FilledButton.tonalIcon(
                  key: const Key('yandex-login-window'),
                  onPressed: _signingIn ? null : _loginWindow,
                  icon: _signingIn ? const ButtonSpinner() : const Icon(Icons.login),
                  label: const Text('Войти в окне'),
                ),
              OutlinedButton.icon(
                onPressed: () => setState(() => _showPaste = !_showPaste),
                icon: const Icon(Icons.content_paste),
                label: Text(webview ? 'Вставить токен вручную' : 'Войти по ссылке'),
              ),
            ],
          ),
          if (_showPaste || (!webview && st.done?.needLogin == true)) ...[
            const SizedBox(height: 16),
            Container(
              padding: const EdgeInsets.all(14),
              decoration: BoxDecoration(
                color: c.surfaceContainer,
                borderRadius: BorderRadius.circular(14),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  _step(1, 'Откройте страницу входа и разрешите доступ', [
                    TextButton.icon(
                      onPressed: () => ref.read(urlOpenerProvider)(Uri.parse(yandexAuthorizeUrl())),
                      icon: const Icon(Icons.open_in_new, size: 18),
                      label: const Text('Открыть oauth.yandex.ru'),
                    ),
                  ]),
                  const SizedBox(height: 8),
                  _step(2, 'Скопируйте адрес открывшейся страницы (в нём есть #access_token=) и вставьте сюда', []),
                  const SizedBox(height: 10),
                  TextField(
                    key: const Key('yandex-paste'),
                    controller: _paste,
                    onSubmitted: (_) => _submitPaste(),
                    onChanged: (_) {
                      if (_pasteError != null) setState(() => _pasteError = null);
                    },
                    decoration: fieldDecoration(
                      hint: 'https://music.yandex.ru/#access_token=…',
                      error: _pasteError,
                    ),
                  ),
                  ButtonRow([
                    FilledButton(
                      onPressed: _signingIn ? null : _submitPaste,
                      child: _signingIn ? const ButtonSpinner() : const Text('Проверить и сохранить'),
                    ),
                  ]),
                ],
              ),
            ),
          ],
          if (!webview)
            Padding(
              padding: const EdgeInsets.only(top: 10),
              child: Text(
                'Встроенное окно входа недоступно: нужен WebKitGTK (пакет webkit2gtk-4.1).',
                style: t.bodySmall?.copyWith(color: c.onSurfaceVariant),
              ),
            ),
        ],
        if (_loginMessage != null) InlineMessage(Tone.warn, _loginMessage!),
        if (st.error != null && _pasteError == null) InlineMessage(Tone.bad, st.error!),
      ],
    );
  }

  Widget _step(int n, String text, List<Widget> actions) {
    final c = Theme.of(context).colorScheme;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        CircleAvatar(
          radius: 11,
          backgroundColor: c.primary,
          child: Text('$n', style: TextStyle(fontSize: 12, color: c.onPrimary, fontWeight: FontWeight.w600)),
        ),
        const SizedBox(width: 10),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Padding(padding: const EdgeInsets.only(top: 2), child: Text(text)),
              ...actions,
            ],
          ),
        ),
      ],
    );
  }

  Widget _importCard(YandexImportState st) {
    final running = st.running;
    final p = st.progress;
    final t = Theme.of(context).textTheme;
    final c = Theme.of(context).colorScheme;
    return SettingsCard(
      icon: Icons.move_down,
      title: 'Импорт',
      trailing: running ? const StatusPill(Tone.busy, 'Идёт импорт') : null,
      children: [
        if (!running) ...[
          const FieldLabel('Что перенести'),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              FilterChip(
                label: const Text('Лайки'),
                selected: _options.likes,
                onSelected: (v) => setState(() => _options = _options.copyWith(likes: v)),
              ),
              FilterChip(
                label: const Text('Плейлисты'),
                selected: _options.playlists,
                onSelected: (v) => setState(() => _options = _options.copyWith(playlists: v)),
              ),
              FilterChip(
                label: const Text('Альбомы'),
                selected: _options.albums,
                onSelected: (v) => setState(() => _options = _options.copyWith(albums: v)),
              ),
              FilterChip(
                label: const Text('Артисты'),
                selected: _options.artists,
                onSelected: (v) => setState(() => _options = _options.copyWith(artists: v)),
              ),
            ],
          ),
          const SizedBox(height: 8),
          SwitchRow(
            title: 'Брать вход из браузера',
            subtitle: 'Если токена нет, взять сессию Яндекса из cookies браузера',
            value: _options.browser,
            onChanged: (v) => setState(() => _options = _options.copyWith(browser: v)),
          ),
          ButtonRow([
            FilledButton.icon(
              key: const Key('yandex-start'),
              onPressed: _options.any ? () => _ctl.start(_options) : null,
              icon: const Icon(Icons.play_arrow_rounded),
              label: const Text('Начать импорт'),
            ),
          ]),
          if (!_options.any) const InlineMessage(Tone.warn, 'Выберите хотя бы что-то одно'),
        ] else ...[
          Text(_stageText(p), style: t.titleSmall),
          const SizedBox(height: 10),
          ClipRRect(
            borderRadius: BorderRadius.circular(8),
            child: LinearProgressIndicator(value: p?.fraction, minHeight: 8),
          ),
          const SizedBox(height: 10),
          if (p != null && p.total > 0)
            Text('${p.done} из ${p.total} · найдено треков: ${p.matched}',
                style: t.bodyMedium?.copyWith(color: c.onSurfaceVariant)),
          if (p?.current != null)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(p!.current!,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: t.bodySmall?.copyWith(color: c.onSurfaceVariant)),
            ),
          ButtonRow([
            OutlinedButton.icon(
              onPressed: _ctl.cancel,
              icon: const Icon(Icons.stop_rounded),
              label: const Text('Отменить'),
            ),
          ]),
        ],
      ],
    );
  }

  Widget _resultCard(YandexDone d, bool webview) {
    final s = d.summary;
    final (tone, title) = d.ok
        ? (Tone.good, 'Импорт завершён')
        : d.cancelled
            ? (Tone.neutral, 'Импорт отменён')
            : d.needLogin
                ? (Tone.warn, 'Нужен вход в Яндекс')
                : (Tone.bad, 'Импорт не удался');
    return SettingsCard(
      icon: Icons.fact_check_outlined,
      title: title,
      trailing: IconButton(
        tooltip: 'Скрыть',
        onPressed: _ctl.dismissResult,
        icon: const Icon(Icons.close),
      ),
      children: [
        if (d.needLogin)
          InlineMessage(
              tone,
              webview
                  ? 'Volna не нашла вход в Яндекс в браузере. Нажмите «Войти в окне» выше: '
                      'после входа импорт продолжится сам.'
                  : 'Volna не нашла вход в Яндекс в браузере. Войдите по ссылке выше: '
                      'после входа импорт продолжится сам.')
        else if (d.error != null)
          InlineMessage(Tone.bad, d.error!),
        if (d.ok || s.total > 0) ...[
          Wrap(
            spacing: 10,
            runSpacing: 10,
            children: [
              _stat('${s.matched} из ${s.total}', 'треков найдено'),
              _stat('${s.likedTracks}', 'новых лайков'),
              _stat('${s.playlists}', 'плейлистов'),
              _stat('${s.albums}', 'альбомов'),
              _stat('${s.artists}', 'артистов'),
            ],
          ),
        ],
        if (d.misses.isNotEmpty) _MissesList(d.misses),
      ],
    );
  }

  Widget _stat(String value, String label) {
    final c = Theme.of(context).colorScheme;
    final t = Theme.of(context).textTheme;
    return Container(
      constraints: const BoxConstraints(minWidth: 120),
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
      decoration: BoxDecoration(color: c.surfaceContainer, borderRadius: BorderRadius.circular(12)),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(value, style: t.titleLarge?.copyWith(fontWeight: FontWeight.w700)),
          Text(label, style: t.bodySmall?.copyWith(color: c.onSurfaceVariant)),
        ],
      ),
    );
  }
}

String _stageText(YandexProgress? p) {
  if (p == null) return 'Начинаю…';
  final detail = p.detail == null ? '' : ': ${p.detail}';
  return switch (p.stage) {
    'browser' => 'Ищу вход в Яндекс в браузере…',
    'account' => 'Проверяю аккаунт Яндекса…',
    'likes' => 'Читаю лайки…',
    'playlists' => 'Читаю плейлисты$detail',
    'albums' => 'Читаю альбомы и артистов…',
    'tracks' => 'Ищу треки на YouTube Music',
    'album_match' => 'Ищу альбомы на YouTube Music',
    'artist_match' => 'Ищу артистов на YouTube Music',
    'saving' => 'Сохраняю в библиотеку…',
    final other => other,
  };
}

class _MissesList extends StatefulWidget {
  final List<YandexMiss> misses;
  const _MissesList(this.misses);

  @override
  State<_MissesList> createState() => _MissesListState();
}

class _MissesListState extends State<_MissesList> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    final t = Theme.of(context).textTheme;
    final m = widget.misses;
    return Padding(
      padding: const EdgeInsets.only(top: 14),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Expanded(
                child: TextButton.icon(
                  style: TextButton.styleFrom(alignment: Alignment.centerLeft),
                  onPressed: () => setState(() => _open = !_open),
                  icon: Icon(_open ? Icons.expand_less : Icons.expand_more),
                  label: Text('Не найдено на YouTube Music: ${m.length}'),
                ),
              ),
              IconButton(
                tooltip: 'Скопировать список',
                onPressed: () => Clipboard.setData(
                    ClipboardData(text: m.map((x) => '${x.artist} — ${x.title}').join('\n'))),
                icon: const Icon(Icons.copy, size: 18),
              ),
            ],
          ),
          if (_open)
            Container(
              constraints: const BoxConstraints(maxHeight: 280),
              decoration: BoxDecoration(color: c.surfaceContainer, borderRadius: BorderRadius.circular(12)),
              child: ListView.separated(
                shrinkWrap: true,
                padding: const EdgeInsets.symmetric(vertical: 6),
                itemCount: m.length,
                separatorBuilder: (_, _) => Divider(height: 1, color: c.outlineVariant.withValues(alpha: 0.4)),
                itemBuilder: (_, i) => Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
                  child: Text.rich(TextSpan(children: [
                    TextSpan(text: m[i].title, style: t.bodyMedium),
                    TextSpan(
                        text: m[i].artist.isEmpty ? '' : '  ·  ${m[i].artist}',
                        style: t.bodyMedium?.copyWith(color: c.onSurfaceVariant)),
                  ])),
                ),
              ),
            ),
        ],
      ),
    );
  }
}
