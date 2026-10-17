/// A [Transport] for widget tests of the settings: canned replies per
/// command, a log of the calls, and events pushed by the test.
library;

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:volna/ipc/transport.dart';

typedef Reply = FutureOr<Object?> Function(Map<String, Object?> args);

class FakeSettingsTransport implements Transport {
  final Map<String, Reply> replies;
  final calls = <(String, Map<String, Object?>)>[];
  final _events = StreamController<DaemonEvent>.broadcast();
  final _status = StreamController<TransportStatus>.broadcast();

  FakeSettingsTransport([Map<String, Reply>? replies]) : replies = {...?replies};

  /// Args of the calls of [cmd], oldest first.
  List<Map<String, Object?>> callsOf(String cmd) => [for (final (c, a) in calls) if (c == cmd) a];

  void emit(String name, Object? payload) => _events.add(DaemonEvent(name, jsonDecode(jsonEncode(payload))));

  @override
  TransportStatus get status => TransportStatus.connected;

  @override
  Stream<TransportStatus> get statusChanges => _status.stream;

  @override
  String? get streamBase => 'http://127.0.0.1:41234/';

  @override
  Future<Object?> call(String cmd, [Map<String, Object?> args = const {}]) async {
    calls.add((cmd, args));
    final r = replies[cmd];
    if (r == null) throw DaemonException('error', 'unknown command $cmd');
    // Through JSON, so maps arrive as `Map<String, dynamic>` like from the socket.
    return jsonDecode(jsonEncode(await r(args)));
  }

  @override
  Future<ProtoResponse> proto(String method, String uri, {List<Header> headers = const [], Uint8List? body}) async =>
      ProtoResponse(404, const [], Uint8List(0));

  @override
  Stream<DaemonEvent> get events => _events.stream;

  @override
  Stream<Uint8List> get vis => const Stream.empty();

  @override
  Stream<void> get show => const Stream.empty();

  @override
  Stream<void> get quit => const Stream.empty();

  @override
  Future<void> close() async {
    await _events.close();
    await _status.close();
  }
}

/// Replies of a daemon in a typical state, shaped as the Rust handlers answer.
Map<String, Reply> sampleReplies() => {
      'net_settings': (_) => {
            'config': {'mode': 'url', 'value': 'socks5h://127.0.0.1:1081'},
            'status': {'mode': 'url', 'proxy': 'socks5h://127.0.0.1:1081', 'xray_running': null, 'xray_pid': null},
            'startup_error': null,
            'cookies': {'mode': 'browser', 'value': 'chrome'},
          },
      'net_validate': (a) {
        final v = ((a['config'] as Map)['value'] as String?) ?? '';
        if (v.contains('bad')) throw const DaemonException('error', 'invalid proxy URL: bad port');
        return null;
      },
      'net_apply': (a) => {'mode': (a['config'] as Map)['mode'], 'proxy': null, 'xray_running': null, 'xray_pid': null},
      'net_health': (_) => {
            'ok': true,
            'latency_ms': 412,
            'exit_ip': '203.0.113.7',
            'ytm_available': true,
            'error': null,
          },
      'net_audio_check': (_) => {'ok': true, 'kind': null, 'message': null, 'latency_ms': 2380},
      'net_set_cookies': (_) => null,
      'net_xray_logs': (_) => <String>[],
      'ytdlp_version': (_) => {
            'binary': '/usr/lib/volna/yt-dlp',
            'origin': 'sidecar',
            'version': '2026.10.10',
          },
      'ytdlp_update': (_) => {
            'binary': '/usr/lib/volna/yt-dlp',
            'origin': 'sidecar',
            'version_before': '2026.10.10',
            'version_after': '2026.10.15',
            'updated': true,
            'message': 'Updated yt-dlp to stable@2026.10.15',
          },
      'player_snapshot': (_) => {
            'protocol': 1,
            'state': {},
            'queue': {},
            'settings': {
              'volume': 0.7,
              'quality': 'high',
              'crossfade': {'enabled': true, 'seconds': 6},
              'gapless': true,
            },
            'dsp': {},
          },
      'player_set_settings': (a) => {
            'volume': 0.7,
            'quality': 'high',
            'crossfade': {'enabled': true, 'seconds': 6},
            'gapless': true,
            ...(a['patch'] as Map<String, Object?>),
          },
      'wave_status': (_) => {
            'cold': true,
            'active': false,
            'settings': {
              'character': 'unfamiliar',
              'mood': 'calm',
              'activity': {'title': 'Focus', 'params': 'p-focus'},
              'language': 'any',
            },
          },
      'wave_set_settings': (a) => {'cold': true, 'active': false, 'settings': a['settings']},
      'wave_activities': (_) => {
            'activities': [
              {'title': 'Workout', 'label': 'Тренировка', 'params': 'p-workout'},
              {'title': 'Focus', 'label': 'Работа и учёба', 'params': 'p-focus'},
              {'title': 'Commute', 'label': 'В дороге', 'params': 'p-commute'},
              {'title': 'Sleep', 'label': 'Сон', 'params': 'p-sleep'},
              {'title': 'Party', 'label': 'Вечеринка', 'params': 'p-party'},
            ],
            'genres': [
              {'title': 'Hip-hop', 'label': 'Хип-хоп', 'params': 'g1'},
              {'title': 'Pop', 'label': 'Поп', 'params': 'g2'},
              {'title': 'Rock', 'label': 'Рок', 'params': 'g3'},
              {'title': 'Jazz', 'label': 'Джаз', 'params': 'g4'},
              {'title': 'Dance & electronic', 'label': 'Электроника', 'params': 'g5'},
            ],
          },
      'wave_artist_search': (_) => [
            for (final (i, n) in ['Shahzoda', 'Скриптонит', 'Jony', 'The Weeknd', 'Billie Eilish', 'Macan',
              'Dua Lipa', 'Баста'].indexed)
              {'id': 'UC$i', 'name': n, 'picture': null},
          ],
      'wave_related_artists': (_) => [
            {'id': 'UCr1', 'name': 'Rauf & Faik', 'picture': null},
          ],
      'wave_cold_start': (_) => {'cold': false, 'active': false, 'settings': {}},
      'scrobble_status': (_) => {
            'percentage': 50,
            'services': [
              {
                'service': 'lastfm',
                'enabled': true,
                'active': true,
                'canLove': true,
                'loveOnLike': true,
                'usesLogin': true,
                'username': 'khudoberdi',
                'loginPending': false,
                'hasToken': false,
                'useCustomCredentials': false,
                'queue': {'pending': 0, 'nextRetryAt': null, 'lastError': null},
              },
              {
                'service': 'librefm',
                'enabled': false,
                'active': false,
                'canLove': true,
                'loveOnLike': false,
                'usesLogin': true,
                'loginPending': false,
                'hasToken': false,
                'queue': {'pending': 0, 'nextRetryAt': null, 'lastError': null},
              },
              {
                'service': 'listenbrainz',
                'enabled': true,
                'active': true,
                'canLove': true,
                'loveOnLike': false,
                'usesLogin': false,
                'loginPending': false,
                'hasToken': true,
                'queue': {'pending': 3, 'nextRetryAt': 1792260000, 'lastError': 'HTTP 503'},
              },
              {
                'service': 'maloja',
                'enabled': false,
                'active': false,
                'canLove': false,
                'loveOnLike': false,
                'usesLogin': false,
                'loginPending': false,
                'hasToken': false,
                'queue': {'pending': 0, 'nextRetryAt': null, 'lastError': null},
              },
            ],
          },
      'yandex_status': (_) => {'signedIn': false, 'login': null, 'importing': false},
      'legacy_status': (_) => {'imported': true, 'importedAt': 1792238574811},
    };
