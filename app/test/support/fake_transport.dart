/// A scriptable in-memory [Transport] for widget tests: register command
/// handlers, push daemon events, record calls. Plus helpers to build player
/// snapshots and tracks in the daemon's JSON shapes.
library;

import 'dart:async';
import 'dart:typed_data';

import 'package:volna/ipc/transport.dart';

typedef Handler = FutureOr<Object?> Function(Map<String, Object?> args);

class Call {
  final String cmd;
  final Map<String, Object?> args;
  const Call(this.cmd, this.args);
  @override
  String toString() => '$cmd($args)';
}

class FakeTransport implements Transport {
  FakeTransport({TransportStatus status = TransportStatus.connected}) : _status = status; // ignore: prefer_initializing_formals

  /// Command handlers by name. A handler may throw a [DaemonException].
  final handlers = <String, Handler>{};

  /// `volna://` responses by URI (exact), then by [protoFallback].
  final protoResponses = <String, ProtoResponse>{};
  ProtoResponse Function(String uri)? protoFallback;

  /// Every call made, in order.
  final calls = <Call>[];

  /// Commands without a handler answer `null` instead of failing.
  bool lenient = true;

  TransportStatus _status;
  final _statusCtl = StreamController<TransportStatus>.broadcast();
  final _events = StreamController<DaemonEvent>.broadcast();
  final _vis = StreamController<Uint8List>.broadcast();
  final _show = StreamController<void>.broadcast();
  final _quit = StreamController<void>.broadcast();

  void on(String cmd, Handler h) => handlers[cmd] = h;

  /// Pushes a daemon event (`player:state`, `library_changed`, …).
  void emit(String name, Object? payload) => _events.add(DaemonEvent(name, payload));

  void setStatus(TransportStatus s) {
    _status = s;
    _statusCtl.add(s);
  }

  List<Call> callsOf(String cmd) => [for (final c in calls) if (c.cmd == cmd) c];

  @override
  TransportStatus get status => _status;

  @override
  Stream<TransportStatus> get statusChanges => _statusCtl.stream;

  @override
  String? get streamBase => 'http://127.0.0.1:0';

  @override
  Future<Object?> call(String cmd, [Map<String, Object?> args = const {}]) async {
    calls.add(Call(cmd, args));
    final h = handlers[cmd];
    if (h == null) {
      if (lenient) return null;
      throw DaemonException('bad_args', 'unknown command $cmd');
    }
    return await h(args);
  }

  @override
  Future<ProtoResponse> proto(String method, String uri,
      {List<Header> headers = const [], Uint8List? body}) async {
    final r = protoResponses[uri] ?? protoFallback?.call(uri);
    return r ?? ProtoResponse(404, const [], Uint8List(0));
  }

  @override
  Stream<DaemonEvent> get events => _events.stream;

  @override
  Stream<Uint8List> get vis => _vis.stream;

  @override
  Stream<void> get show => _show.stream;

  @override
  Stream<void> get quit => _quit.stream;

  @override
  Future<void> close() async {
    _status = TransportStatus.closed;
  }
}

// ---- daemon JSON builders ------------------------------------------------------

Map<String, Object?> trackJson(
  String uid, {
  String? id,
  String title = 'Track',
  List<String> artists = const ['Artist'],
  String? album,
  double? duration = 200,
  String? cover,
  String origin = 'user',
  bool explicit = false,
}) =>
    {
      'uid': uid,
      'id': id ?? 'vid_$uid',
      'title': title,
      'artists': [
        for (final (i, a) in artists.indexed) {'id': 'UC_${a.hashCode}_$i', 'name': a},
      ],
      'album': album == null ? null : {'id': 'MPREb_$album', 'title': album},
      'duration': duration,
      'cover': cover,
      'explicit': explicit,
      'kind': 'track',
      'origin': origin,
      'raw': null,
      'ui': null,
      'unavailable': false,
    };

Map<String, Object?> stateJson({
  String status = 'paused',
  String? uid,
  String? trackId,
  double position = 0,
  double at = 0,
  double? duration = 200,
  double volume = 0.7,
  bool muted = false,
  bool buffering = false,
}) =>
    {
      'status': status,
      'uid': uid,
      'trackId': trackId,
      'position': position,
      'at': at,
      'duration': duration,
      'rate': 1.0,
      'buffering': buffering,
      'crossfading': false,
      'volume': volume,
      'muted': muted,
      'stream': null,
      'error': null,
    };

Map<String, Object?> settingsJson({double volume = 0.7}) => {
      'volume': volume,
      'muted': false,
      'volumeCurve': 'linear',
      'quality': 'high',
      'crossfade': {'enabled': false, 'seconds': 5},
      'gapless': true,
      'speed': 1.0,
      'autoplay': false,
    };

Map<String, Object?> snapshotJson({
  List<Map<String, Object?>> items = const [],
  int index = -1,
  int rev = 1,
  Map<String, Object?>? state,
  bool wave = false,
  bool shuffle = false,
  String repeat = 'off',
  String continuation = 'none',
  bool fetching = false,
}) =>
    {
      'protocol': 1,
      'state': state ?? stateJson(status: 'idle'),
      'queue': {
        'rev': rev,
        'items': items,
        'original': null,
        'index': index,
        'shuffle': shuffle,
        'repeat': repeat,
        'continuation': {'kind': continuation, 'fetching': fetching},
      },
      'settings': settingsJson(),
      'dsp': const <String, Object?>{},
      'sleepTimer': null,
      'wave': {'active': wave},
      'journalSeq': 0,
    };

Map<String, Object?> queueChangedJson({
  required int rev,
  required List<String> order,
  int index = 0,
  List<Map<String, Object?>> added = const [],
  List<Map<String, Object?>> updated = const [],
  bool shuffle = false,
  String repeat = 'off',
  String continuation = 'none',
}) =>
    {
      'rev': rev,
      'order': order,
      'original': null,
      'index': index,
      'shuffle': shuffle,
      'repeat': repeat,
      'continuation': {'kind': continuation, 'fetching': false},
      'added': added,
      'updated': updated,
    };

/// A fake daemon player: answers `player_snapshot` from [snapshot] and
/// records the rest. Mutate [snapshot] to change what a resync sees.
class FakePlayer {
  final FakeTransport t;
  Map<String, Object?> snapshot;

  FakePlayer(this.t, {Map<String, Object?>? snapshot})
      : snapshot = snapshot ?? snapshotJson() {
    t.on('player_snapshot', (_) => this.snapshot);
    for (final c in const [
      'player_set_shuffle',
      'player_set_repeat',
      'player_move',
      'player_remove',
      'player_wave_start',
      'player_wave_requeue',
      'player_set_queue',
      'player_enqueue',
      'player_clear',
    ]) {
      t.on(c, (_) => {'rev': 1});
    }
    t.on('player_set_settings', (a) => {...settingsJson(), ...(a['patch'] as Map).cast()});
  }
}
