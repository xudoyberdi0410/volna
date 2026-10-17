/// Screenshots of every settings section (light, plus a few dark) in
/// `test/goldens/`. Regenerate with `flutter test --update-goldens test/settings_golden_test.dart`.
@Tags(['golden'])
library;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:volna/ui/settings/settings_screen.dart';

import 'support/fake_transport_settings.dart';
import 'support/settings_harness.dart';

void main() {
  setUpAll(loadFonts);
  setUp(() => SharedPreferences.setMockInitialValues({}));

  const heights = {
    'network': 1250.0,
    'wave': 1300.0,
    'quality': 760.0,
    'appearance': 560.0,
    'scrobbling': 2050.0,
    'yandex': 1000.0,
    'library': 560.0,
    'ytdlp': 620.0,
    'about': 640.0,
  };

  for (final s in settingsSections) {
    testWidgets('golden ${s.id}', (tester) async {
      setWindow(tester, height: heights[s.id] ?? 900);
      final h = Harness(FakeSettingsTransport(sampleReplies()));
      await tester.pumpWidget(h.app(SettingsScreen(section: s.id)));
      await tester.pumpAndSettle();
      if (s.id == 'network') {
        await tester.tap(find.text('Проверить'));
        await tester.pumpAndSettle();
      }
      await expectLater(find.byType(SettingsScreen), matchesGoldenFile('goldens/${s.id}.png'));
    });
  }

  for (final id in ['network', 'scrobbling', 'yandex']) {
    testWidgets('golden $id dark', (tester) async {
      setWindow(tester, height: heights[id]!);
      final h = Harness(FakeSettingsTransport(sampleReplies()));
      await tester.pumpWidget(h.app(SettingsScreen(section: id), mode: ThemeMode.dark));
      await tester.pumpAndSettle();
      await expectLater(find.byType(SettingsScreen), matchesGoldenFile('goldens/${id}_dark.png'));
    });
  }

  testWidgets('golden yandex import running and done', (tester) async {
    setWindow(tester, height: 1250);
    final t = FakeSettingsTransport(sampleReplies());
    final h = Harness(t);
    await tester.pumpWidget(h.app(const SettingsScreen(section: 'yandex')));
    await tester.pumpAndSettle();
    t.emit('yandex_import_progress', {
      'stage': 'tracks',
      'done': 412,
      'total': 1180,
      'matched': 398,
      'current': 'Земфира — Хочешь?',
    });
    await tester.pump();
    await tester.pump();
    await expectLater(find.byType(SettingsScreen), matchesGoldenFile('goldens/yandex_running.png'));
    t.emit('yandex_import_done', {
      'ok': true,
      'cancelled': false,
      'needLogin': false,
      'error': null,
      'summary': {'likedTracks': 1043, 'playlists': 12, 'albums': 37, 'artists': 54, 'matched': 1121, 'total': 1180},
      'misses': [
        {'title': 'Хочешь?', 'artist': 'Земфира'},
        {'title': 'Спокойная ночь', 'artist': 'Кино'},
        {'title': 'Some Remix (Live)', 'artist': 'Unknown DJ'},
      ],
    });
    await tester.pumpAndSettle();
    await tester.tap(find.textContaining('Не найдено на YouTube Music'));
    await tester.pumpAndSettle();
    await expectLater(find.byType(SettingsScreen), matchesGoldenFile('goldens/yandex_done.png'));
  });

  testWidgets('golden narrow menu', (tester) async {
    setWindow(tester, width: 480, height: 640);
    final h = Harness(FakeSettingsTransport(sampleReplies()));
    await tester.pumpWidget(h.app(const SettingsScreen()));
    await tester.pumpAndSettle();
    await expectLater(find.byType(SettingsScreen), matchesGoldenFile('goldens/menu_narrow.png'));
  });
}
