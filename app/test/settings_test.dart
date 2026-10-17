import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:volna/api/network.dart';
import 'package:volna/api/yandex.dart';
import 'package:volna/ui/settings/network_text.dart';
import 'package:volna/ui/settings/settings_screen.dart';
import 'package:volna/ui/settings/widgets.dart';

import 'support/fake_transport_settings.dart';
import 'support/settings_harness.dart';

Future<Harness> pumpSection(WidgetTester tester, String id, {Harness? harness, double height = 1600}) async {
  setWindow(tester, height: height);
  final h = harness ?? Harness(FakeSettingsTransport(sampleReplies()));
  await tester.pumpWidget(h.app(SettingsScreen(section: id)));
  await tester.pumpAndSettle();
  return h;
}

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));

  group('network text', () {
    test('precheck', () {
      expect(precheckProxy(const ProxyConfig.direct()), isNull);
      expect(precheckProxy(const ProxyConfig(NetMode.url, '')), 'Укажите адрес прокси');
      expect(precheckProxy(const ProxyConfig(NetMode.url, 'ftp://x:1')), contains('socks5://'));
      expect(precheckProxy(const ProxyConfig(NetMode.url, 'socks5h://127.0.0.1:1081')), isNull);
      expect(precheckProxy(const ProxyConfig(NetMode.vless, 'http://x')), contains('vless://'));
    });
    test('cookies form round trip', () {
      expect(buildCookies('chrome', 'Profile 1').toJson(), {'mode': 'browser', 'value': 'chrome:Profile 1'});
      expect(cookiesForm(const CookiesConfig(CookiesMode.browser, 'firefox:default')), ('firefox', 'default'));
      expect(cookiesForm(const CookiesConfig(CookiesMode.file, '/c.txt')), ('file', '/c.txt'));
      expect(buildCookies('none', 'x').toJson(), {'mode': 'none'});
    });
    test('health tones', () {
      const bot = AudioCheck(ok: false, kind: FailureKind.botCheck);
      final (tone, rows) = describeHealth(const HealthReport(ok: true, ytmAvailable: true), audio: bot);
      expect(tone, Tone.bad);
      expect(rows.last.$2, contains('не бот'));
      expect(describeHealth(const HealthReport(ok: true, ytmAvailable: false)).$1, Tone.bad);
    });
    test('token from redirect', () {
      expect(tokenFromRedirect('https://music.yandex.ru/#access_token=y0_abc&token_type=bearer'), 'y0_abc');
      expect(tokenFromRedirect('https://music.yandex.ru/'), isNull);
    });
  });

  testWidgets('network: inline validation, apply and check', (tester) async {
    final h = await pumpSection(tester, 'network');
    final field = find.byKey(const Key('net-url'));
    await tester.enterText(field, 'ftp://nope');
    await tester.pump();
    expect(find.text('Поддерживаются socks5://, socks5h://, http:// и https://'), findsOneWidget);

    // The daemon's own parser after a pause in typing.
    await tester.enterText(field, 'socks5h://bad.example:1080');
    await tester.pump(const Duration(milliseconds: 500));
    await tester.pumpAndSettle();
    expect(find.text('invalid proxy URL: bad port'), findsOneWidget);

    await tester.enterText(field, 'socks5h://127.0.0.1:1080');
    await tester.pump(const Duration(milliseconds: 500));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Применить'));
    await tester.pumpAndSettle();
    expect(h.transport.callsOf('net_apply').single['config'], {'mode': 'url', 'value': 'socks5h://127.0.0.1:1080'});
    expect(h.transport.callsOf('net_health'), hasLength(1));
    expect(find.text('Применено'), findsOneWidget);
    expect(find.text('Всё работает'), findsOneWidget);
    expect(find.text('203.0.113.7'), findsOneWidget);
  });

  testWidgets('network: cookies saved, then audio checked', (tester) async {
    final h = await pumpSection(tester, 'network');
    await tester.enterText(find.widgetWithText(TextField, 'default-release'), 'Profile 2');
    await tester.tap(find.text('Сохранить и проверить звук'));
    await tester.pumpAndSettle();
    expect(h.transport.callsOf('net_set_cookies').single['cookies'], {'mode': 'browser', 'value': 'chrome:Profile 2'});
    expect(find.textContaining('Звук: работает'), findsOneWidget);
  });

  testWidgets('scrobbling: Libre.fm browser login', (tester) async {
    final t = FakeSettingsTransport(sampleReplies());
    t.replies['scrobble_login_start'] = (_) => {'url': 'https://libre.fm/api/auth/?token=abc'};
    t.replies['scrobble_login_finish'] = (_) => {'username': 'me'};
    final h = await pumpSection(tester, 'scrobbling', harness: Harness(t), height: 2200);
    await tester.tap(find.text('Войти через Libre.fm'));
    await tester.pumpAndSettle();
    expect(t.callsOf('scrobble_login_start').single, {'service': 'librefm'});
    expect(h.opened.single.toString(), 'https://libre.fm/api/auth/?token=abc');
    await tester.tap(find.text('Я разрешил доступ'));
    await tester.pumpAndSettle();
    expect(t.callsOf('scrobble_login_finish').single, {'service': 'librefm'});
    expect(find.text('Вход выполнен: me'), findsOneWidget);
  });

  testWidgets('scrobbling: Maloja needs a valid URL; ListenBrainz keeps its token', (tester) async {
    final t = FakeSettingsTransport(sampleReplies());
    t.replies['scrobble_set'] = sampleReplies()['scrobble_status']!;
    await pumpSection(tester, 'scrobbling', harness: Harness(t), height: 2200);

    await tester.enterText(find.byKey(const Key('maloja-url')), 'maloja.local');
    await tester.tap(find.text('Сохранить').last);
    await tester.pumpAndSettle();
    expect(find.text('Адрес должен начинаться с http:// или https://'), findsOneWidget);
    expect(t.callsOf('scrobble_set'), isEmpty);

    await tester.enterText(find.byKey(const Key('maloja-url')), 'https://maloja.local');
    await tester.enterText(find.byKey(const Key('maloja-token')), 'key1');
    await tester.tap(find.text('Сохранить').last);
    await tester.pumpAndSettle();
    expect(t.callsOf('scrobble_set').last,
        {'service': 'maloja', 'url': 'https://maloja.local', 'token': 'key1', 'enabled': true});

    // A token is already saved: an empty field does not overwrite it.
    await tester.tap(find.text('Сохранить').first);
    await tester.pumpAndSettle();
    expect(t.callsOf('scrobble_set').last, {'service': 'listenbrainz', 'url': '', 'enabled': true});
  });

  testWidgets('yandex: need login → login window → token → import goes on', (tester) async {
    final t = FakeSettingsTransport(sampleReplies());
    t.replies['yandex_import_start'] = (_) => {'started': true};
    t.replies['yandex_set_token'] = (_) => {'signedIn': true, 'login': 'user', 'importing': false};
    final login = FakeWebLogin(result: 'https://music.yandex.ru/#access_token=y0_tok&token_type=bearer');
    final h = await pumpSection(tester, 'yandex', harness: Harness(t, webLogin: login));

    await tester.tap(find.text('Плейлисты'));
    await tester.pump();
    await tester.tap(find.byKey(const Key('yandex-start')));
    await tester.pump();
    await tester.pump();
    expect(t.callsOf('yandex_import_start').single,
        {'likes': true, 'playlists': false, 'albums': true, 'artists': true, 'browser': true});

    t.emit('yandex_import_progress', {'stage': 'tracks', 'done': 5, 'total': 10, 'matched': 4, 'current': 'A — B'});
    await tester.pump();
    await tester.pump();
    expect(find.text('5 из 10 · найдено треков: 4'), findsOneWidget);

    t.emit('yandex_import_done', {
      'ok': false,
      'cancelled': false,
      'needLogin': true,
      'error': 'нужен вход в Яндекс',
      'summary': {},
      'misses': [],
    });
    await tester.pumpAndSettle();
    expect(find.text('Нужен вход в Яндекс'), findsOneWidget);

    await tester.tap(find.byKey(const Key('yandex-login-window')));
    for (var i = 0; i < 5; i++) {
      await tester.pump();
    }
    expect(h.webLogin.opened.single, yandexAuthorizeUrl());
    expect(t.callsOf('yandex_set_token').single['token'], contains('access_token=y0_tok'));
    expect(t.callsOf('yandex_import_start'), hasLength(2));
  });

  testWidgets('yandex: paste fallback without WebKitGTK', (tester) async {
    final t = FakeSettingsTransport(sampleReplies());
    t.replies['yandex_set_token'] = (_) => {'signedIn': true, 'login': 'user', 'importing': false};
    final h = await pumpSection(tester, 'yandex', harness: Harness(t, webLogin: FakeWebLogin(isAvailable: false)));
    expect(find.byKey(const Key('yandex-login-window')), findsNothing);
    await tester.tap(find.text('Войти по ссылке'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Открыть oauth.yandex.ru'));
    expect(h.opened.single.host, 'oauth.yandex.ru');
    await tester.enterText(find.byKey(const Key('yandex-paste')), 'https://music.yandex.ru/');
    await tester.tap(find.text('Проверить и сохранить'));
    await tester.pumpAndSettle();
    expect(find.textContaining('В адресе нет access_token'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('yandex-paste')), 'y0_plain_token');
    await tester.tap(find.text('Проверить и сохранить'));
    await tester.pumpAndSettle();
    expect(t.callsOf('yandex_set_token').single, {'token': 'y0_plain_token'});
    expect(find.text('user'), findsOneWidget);
  });

  testWidgets('library: import from a file', (tester) async {
    final t = FakeSettingsTransport(sampleReplies());
    t.replies['legacy_import'] = (_) => {'tracks': 120, 'albums': 3, 'playlists': 2, 'userPlaylists': 1};
    final h = Harness(t)..pickedFile = '/home/u/Downloads/volna-library-1.json';
    await pumpSection(tester, 'library', harness: h);
    await tester.tap(find.byKey(const Key('legacy-import')));
    await tester.pumpAndSettle();
    expect(t.callsOf('legacy_import').single, {'path': '/home/u/Downloads/volna-library-1.json'});
    expect(find.textContaining('Треков: 120'), findsOneWidget);
  });

  testWidgets('wave: settings and cold start', (tester) async {
    final t = FakeSettingsTransport(sampleReplies());
    await pumpSection(tester, 'wave', harness: Harness(t));
    await tester.tap(find.text('Грустное'));
    await tester.pumpAndSettle();
    expect((t.callsOf('wave_set_settings').last['settings'] as Map)['mood'], 'sad');
    await tester.tap(find.text('Без занятия'));
    await tester.pumpAndSettle();
    expect((t.callsOf('wave_set_settings').last['settings'] as Map)['activity'], isNull);

    await tester.tap(find.text('Jony'));
    await tester.pumpAndSettle();
    expect(t.callsOf('wave_related_artists').single, {'id': 'UC2'});
    expect(find.text('Rauf & Faik'), findsOneWidget);
    await tester.tap(find.text('Добавить в волну (1)'));
    await tester.pumpAndSettle();
    expect(t.callsOf('wave_cold_start').single, {
      'artists': [
        {'id': 'UC2', 'name': 'Jony'}
      ]
    });
    expect(find.text('Вкус известен'), findsOneWidget);
  });

  testWidgets('quality: switching to low', (tester) async {
    final t = FakeSettingsTransport(sampleReplies());
    await pumpSection(tester, 'quality', harness: Harness(t));
    await tester.tap(find.byKey(const Key('quality-low')));
    await tester.pumpAndSettle();
    expect(t.callsOf('player_set_settings').single, {
      'patch': {'quality': 'low'}
    });
  });

  testWidgets('appearance: theme is saved', (tester) async {
    await pumpSection(tester, 'appearance');
    await tester.tap(find.byKey(const Key('theme-dark')));
    await tester.pumpAndSettle();
    final prefs = await SharedPreferences.getInstance();
    expect(prefs.getString('theme_mode'), 'dark');
  });

  testWidgets('ytdlp: update', (tester) async {
    await pumpSection(tester, 'ytdlp');
    await tester.tap(find.text('Обновить'));
    await tester.pumpAndSettle();
    expect(find.text('Обновлён: 2026.10.10 → 2026.10.15'), findsOneWidget);
  });
}
