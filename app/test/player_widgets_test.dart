import 'package:flutter/material.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:volna/ui/player/lyrics_view.dart';
import 'package:volna/api/lyrics.dart';

import 'support/fake_transport.dart';
import 'support/test_app.dart';

Map<String, Object?> _waveSnapshot({String status = 'paused'}) => snapshotJson(
      items: [
        trackJson('u1', title: 'Первый', artists: ['Альфа'], origin: 'wave'),
        trackJson('u2', title: 'Второй', artists: ['Бета'], origin: 'wave'),
        trackJson('u3', title: 'Третий', artists: ['Гамма'], origin: 'wave'),
      ],
      index: 0,
      wave: true,
      continuation: 'wave',
      state: stateJson(status: status, uid: 'u1', position: 30, at: kTestNow.toDouble()),
    );

void main() {
  testWidgets('idle wave starts the wave', (tester) async {
    final t = FakeTransport();
    final p = FakePlayer(t);
    t.on('player_wave_start', (_) {
      p.snapshot = _waveSnapshot();
      t.emit('player:wave', {'active': true});
      t.emit('player:queue', queueChangedJson(rev: 2, order: ['u1', 'u2', 'u3'], added: [
        for (final i in (p.snapshot['queue'] as Map)['items'] as List) i as Map<String, Object?>,
      ]));
      t.emit('player:state', stateJson(status: 'playing', uid: 'u1', at: kTestNow.toDouble()));
      return {'rev': 2};
    });
    await pumpApp(tester, t);
    expect(find.text('Моя волна'), findsOneWidget);
    await tester.tap(find.text('Запустить волну'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 600));
    expect(t.callsOf('player_wave_start'), hasLength(1));
    // The track shows on the wave screen and in the mini player.
    expect(find.text('Первый'), findsNWidgets(2));
  });

  testWidgets('wave buttons: dislike, skip, like, play/pause', (tester) async {
    final t = FakeTransport();
    FakePlayer(t, snapshot: _waveSnapshot());
    t.on('library_is_liked', (_) => {'vid_u1': false});
    t.on('library_like', (a) => a['on']);
    await pumpApp(tester, t);
    await tester.pump(const Duration(milliseconds: 600));

    await tester.tap(find.byTooltip('Не нравится — пропустить'));
    await tester.pump();
    expect(t.callsOf('player_wave_dislike'), hasLength(1));

    await tester.tap(find.byTooltip('Следующий').first);
    await tester.pump();
    expect(t.callsOf('player_next'), hasLength(1));

    await tester.tap(find.byTooltip('Нравится').first);
    await tester.pump(const Duration(milliseconds: 500));
    final like = t.callsOf('library_like').single.args;
    expect(like['kind'], 'track');
    expect(like['on'], true);
    expect((like['item'] as Map)['video_id'], 'vid_u1');
    expect(find.byTooltip('Убрать из любимых'), findsWidgets);

    await tester.tap(find.byTooltip('Играть (пробел)').first);
    await tester.pump();
    expect(t.callsOf('player_toggle'), hasLength(1));

    await tester.tap(find.text('Остановить'));
    await tester.pump();
    expect(t.callsOf('player_wave_stop'), hasLength(1));
  });

  testWidgets('keyboard: space toggles, arrows seek', (tester) async {
    final t = FakeTransport();
    FakePlayer(t, snapshot: _waveSnapshot());
    await pumpApp(tester, t);
    await tester.pump(const Duration(milliseconds: 600));
    await tester.sendKeyEvent(LogicalKeyboardKey.space);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
    await tester.pump();
    expect(t.callsOf('player_toggle'), hasLength(1));
    expect(t.callsOf('player_seek').single.args['position'], closeTo(35, 0.01));
  });

  testWidgets('mini player opens the fullscreen player; queue remove', (tester) async {
    final t = FakeTransport();
    FakePlayer(t, snapshot: _waveSnapshot());
    t.on('lyrics_get', (_) => null);
    await pumpApp(tester, t, location: '/search');
    await tester.pump(const Duration(milliseconds: 600));
    expect(find.text('Первый'), findsOneWidget); // mini player only
    await tester.tap(find.byTooltip('Очередь'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 600));
    expect(find.text('Сейчас играет'.toUpperCase()), findsOneWidget);
    expect(find.text('Второй'), findsOneWidget);

    final row = find.ancestor(of: find.text('Второй'), matching: find.byType(MouseRegion)).first;
    final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await gesture.addPointer(location: tester.getCenter(row));
    await tester.pump();
    await tester.tap(find.byTooltip('Убрать из очереди'));
    await tester.pump();
    expect(t.callsOf('player_remove').single.args, {'uid': 'u2'});
    expect(find.text('Второй'), findsNothing);
    await gesture.removePointer();

    // Esc closes the player.
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 600));
    expect(find.text('Сейчас играет'.toUpperCase()), findsNothing);
  });

  testWidgets('lyrics: active line follows position; tap seeks', (tester) async {
    final t = FakeTransport();
    FakePlayer(t, snapshot: _waveSnapshot());
    t.on('lyrics_get', (_) => {
          'source': 'LRCLIB',
          'synced': 'line',
          'lines': [
            {'startMs': 10000, 'endMs': 20000, 'text': 'раз'},
            {'startMs': 25000, 'endMs': 35000, 'text': 'два'},
            {'startMs': 40000, 'endMs': 50000, 'text': 'три'},
          ],
        });
    await pumpApp(tester, t, location: '/player?tab=lyrics');
    await tester.pump(const Duration(milliseconds: 800));
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('два'), findsOneWidget);
    expect(find.text('Источник: LRCLIB'), findsOneWidget);
    // Position 30 s → line "два" is active (opaque).
    Color colorOf(String s) {
      final style = tester.widget<AnimatedDefaultTextStyle>(
          find.ancestor(of: find.text(s), matching: find.byType(AnimatedDefaultTextStyle)).first);
      return style.style.color!;
    }

    expect(colorOf('два').a, closeTo(1, 0.01));
    expect(colorOf('три').a, lessThan(0.5));
    await tester.tap(find.text('три'));
    await tester.pump();
    expect(t.callsOf('player_seek').single.args['position'], 40);
  });

  test('activeLineAt', () {
    const lines = [LyricLine(1000, 2000, 'a'), LyricLine(3000, 4000, 'b'), LyricLine(5000, 6000, 'c')];
    expect(activeLineAt(lines, 0), -1);
    expect(activeLineAt(lines, 1000), 0);
    expect(activeLineAt(lines, 4500), 1);
    expect(activeLineAt(lines, 99999), 2);
  });
}
