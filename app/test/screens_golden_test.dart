// Screenshots of the main screens over a fake daemon. Regenerate with
// `flutter test --update-goldens test/screens_golden_test.dart`; the PNGs
// land in test/goldens/.
@Tags(['golden'])
library;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/fake_transport.dart';
import 'support/test_app.dart';

const _covers = [
  ('https://lh3.example/c1', Color(0xff2b1a5e), Color(0xffe0457b)),
  ('https://lh3.example/c2', Color(0xff0f3d3a), Color(0xff6fd3b8)),
  ('https://lh3.example/c3', Color(0xff5a2a0c), Color(0xfff2b34a)),
  ('https://lh3.example/c4', Color(0xff13294b), Color(0xff4f8cff)),
  ('https://lh3.example/c5', Color(0xff3b0f2b), Color(0xffc879ff)),
];

List<Map<String, Object?>> _tracks() => [
      trackJson('u1',
          title: 'Полуночный экспресс',
          artists: ['Луна', 'Нейромонах Феофан'],
          album: 'Ночь',
          duration: 214,
          cover: _covers[0].$1,
          origin: 'wave'),
      trackJson('u2',
          title: 'Glass Animals of the Sea', artists: ['Tidal Bloom'], cover: _covers[1].$1, origin: 'wave'),
      trackJson('u3', title: 'Самолёты', artists: ['Мумий Тролль'], cover: _covers[2].$1, origin: 'wave'),
      trackJson('u4', title: 'Neon Rain', artists: ['Kavinsky'], cover: _covers[3].$1, origin: 'wave'),
      trackJson('u5', title: 'Звезда', artists: ['Кино'], cover: _covers[4].$1, origin: 'wave'),
    ];

Future<FakeTransport> _daemon({bool wave = true, bool playing = false}) async {
  final t = FakeTransport();
  FakePlayer(t,
      snapshot: wave
          ? snapshotJson(
              items: _tracks(),
              index: 0,
              wave: true,
              continuation: 'wave',
              state: stateJson(
                status: playing ? 'playing' : 'paused',
                uid: 'u1',
                trackId: 'vid_u1',
                position: 79,
                at: kTestNow.toDouble(),
                duration: 214,
              ),
            )
          : snapshotJson());
  for (final c in _covers) {
    await addCover(t, c.$1, c.$2, c.$3);
  }
  t.on('library_is_liked', (a) => {for (final id in a['ids'] as List) id as String: id == 'vid_u1'});
  t.on('lyrics_get', (_) => _lyrics);
  t.on('lyrics_sources', (_) => [
        {'source': 'lrc.red', 'synced': 'word', 'lines': 8},
        {'source': 'LRCLIB', 'synced': 'line', 'lines': 8},
      ]);
  return t;
}

final _lyrics = {
  'source': 'lrc.red',
  'synced': 'word',
  'lines': [
    for (final (i, l) in const [
      'Город засыпает под шорох шин',
      'Мы едем туда, где нет никого',
      'Фонари горят, и мы одни',
      'В полуночном экспрессе домой',
      'Слышишь, как стучит сердце в такт',
      'Колёсам, что не знают пути',
      'Не отпускай мою руку никак',
      'Нам ещё так долго идти',
    ].indexed)
      {
        'startMs': 60000 + i * 6000,
        'endMs': 60000 + i * 6000 + 5500,
        'text': l,
        'words': [
          for (final (j, w) in l.split(' ').indexed)
            {
              'startMs': 60000 + i * 6000 + j * 700,
              'endMs': 60000 + i * 6000 + j * 700 + 650,
              'text': '$w ',
            },
        ],
      },
  ],
};

void main() {
  setUpAll(loadAppFonts);

  Future<void> shot(WidgetTester tester, String name,
      {String location = '/wave',
      bool wave = true,
      ThemeMode theme = ThemeMode.dark,
      Size size = const Size(1440, 900)}) async {
    // Real (blurred) shadows in the screenshot; restored before the
    // binding checks its invariants after the body.
    debugDisableShadows = false;
    try {
      final t = (await tester.runAsync(() => _daemon(wave: wave)))!;
      await pumpApp(tester, t, location: location, theme: theme, size: size);
      await settleImages(tester);
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/$name.png'));
    } finally {
      debugDisableShadows = true;
    }
  }

  testWidgets('wave idle', (tester) => shot(tester, 'wave_idle', wave: false));
  testWidgets('wave playing (dark)', (tester) => shot(tester, 'wave_playing_dark'));
  testWidgets('wave playing (light)',
      (tester) => shot(tester, 'wave_playing_light', theme: ThemeMode.light));
  testWidgets('wave playing (narrow)',
      (tester) => shot(tester, 'wave_playing_narrow', size: const Size(760, 820)));
  testWidgets('player with lyrics', (tester) => shot(tester, 'player_lyrics', location: '/player'));
  testWidgets('player with queue',
      (tester) => shot(tester, 'player_queue', location: '/player?tab=queue'));
  testWidgets('player (light)',
      (tester) => shot(tester, 'player_lyrics_light', location: '/player', theme: ThemeMode.light));
  testWidgets('settings in the shell',
      (tester) => shot(tester, 'shell_settings', location: '/settings/appearance'));
  testWidgets('search placeholder', (tester) => shot(tester, 'shell_search', location: '/search'));
}
