import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:volna/api/player_models.dart';
import 'package:volna/ipc/transport.dart';
import 'package:volna/state/connection.dart';
import 'package:volna/state/player.dart';

import 'support/fake_transport.dart';

Future<void> _flush() => Future<void>.delayed(Duration.zero);

void main() {
  late FakeTransport t;
  late FakePlayer daemon;
  late ProviderContainer c;

  setUp(() {
    t = FakeTransport();
    daemon = FakePlayer(t,
        snapshot: snapshotJson(
          items: [trackJson('a', title: 'A'), trackJson('b', title: 'B')],
          index: 0,
          rev: 5,
          state: stateJson(status: 'playing', uid: 'a', position: 10, at: 1000),
        ));
    c = ProviderContainer(overrides: [
      transportProvider.overrideWithValue(t),
      clockProvider.overrideWithValue(() => 3000),
    ]);
    addTearDown(c.dispose);
    c.listen(playerViewProvider, (_, _) {});
  });

  test('applies the snapshot on start', () async {
    await _flush();
    await _flush();
    final v = c.read(playerViewProvider);
    expect(v.ready, isTrue);
    expect(v.items.map((e) => e.title), ['A', 'B']);
    expect(v.current?.uid, 'a');
    expect(v.rev, 5);
    expect(v.upNext.single.uid, 'b');
  });

  test('position interpolates between ticks and stops when paused', () async {
    await _flush();
    await _flush();
    // Sample 10 s at t=1000 ms, now 3000 ms → 12 s.
    expect(c.read(positionProvider).positionAt(3000), closeTo(12, 1e-9));
    t.emit('player:position',
        {'uid': 'a', 'position': 20.0, 'duration': 200.0, 'rate': 1.0, 'at': 2900.0, 'playing': false});
    await _flush();
    expect(c.read(positionProvider).positionAt(3000), 20);
    // A stale sample does not run away.
    const s = PositionSample(position: 5, at: 1, playing: true, duration: 300);
    expect(s.positionAt(60000), 7);
  });

  test('queue events merge, a rev gap resyncs', () async {
    await _flush();
    await _flush();
    t.emit('player:queue', queueChangedJson(rev: 6, order: ['b', 'a'], index: 1));
    await _flush();
    expect(c.read(playerViewProvider).items.map((e) => e.uid), ['b', 'a']);
    expect(t.callsOf('player_snapshot'), hasLength(1));

    t.emit('player:queue',
        queueChangedJson(rev: 7, order: ['b', 'a', 'c'], index: 1, added: [trackJson('c', title: 'C')]));
    await _flush();
    expect(c.read(playerViewProvider).items.map((e) => e.title), ['B', 'A', 'C']);

    // Unknown uid → snapshot.
    daemon.snapshot = snapshotJson(items: [trackJson('z', title: 'Z')], index: 0, rev: 20);
    t.emit('player:queue', queueChangedJson(rev: 20, order: ['z']));
    await _flush();
    await _flush();
    expect(t.callsOf('player_snapshot'), hasLength(2));
    expect(c.read(playerViewProvider).items.single.title, 'Z');
  });

  test('state, wave and error events', () async {
    await _flush();
    await _flush();
    final messages = <String>[];
    c.read(playerMessagesProvider).stream.listen(messages.add);
    t.emit('player:state', stateJson(status: 'paused', uid: 'b'));
    t.emit('player:wave', {'active': true});
    t.emit('player:error', {
      'uid': 'b',
      'trackId': 'vid_b',
      'kind': 'unavailable',
      'message': 'x',
      'action': 'skipped',
      'streak': 1
    });
    await _flush();
    final v = c.read(playerViewProvider);
    expect(v.state.status, PlayStatus.paused);
    expect(v.current?.uid, 'b');
    expect(v.waveActive, isTrue);
    expect(messages.single, contains('пропущен'));
  });

  test('commands reach the daemon; move and remove are optimistic', () async {
    await _flush();
    await _flush();
    final ctl = c.read(playerViewProvider.notifier);
    await ctl.toggle();
    await ctl.move('b', 0);
    expect(c.read(playerViewProvider).items.map((e) => e.uid), ['b', 'a']);
    await ctl.remove('a');
    expect(c.read(playerViewProvider).items.map((e) => e.uid), ['b']);
    await ctl.cycleRepeat();
    await ctl.waveStart();
    expect(t.calls.map((e) => e.cmd), containsAllInOrder([
      'player_toggle',
      'player_move',
      'player_remove',
      'player_set_repeat',
      'player_wave_start',
    ]));
    expect(t.callsOf('player_move').single.args, {'uid': 'b', 'to': 0});
    expect(t.callsOf('player_set_repeat').single.args, {'mode': 'all'});
  });

  test('reconnect resyncs', () async {
    await _flush();
    await _flush();
    t.setStatus(TransportStatus.disconnected);
    t.setStatus(TransportStatus.connected);
    await _flush();
    await _flush();
    expect(t.callsOf('player_snapshot'), hasLength(2));
  });
}
