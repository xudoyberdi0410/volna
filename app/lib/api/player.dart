/// Typed wrappers over `crates/daemon/src/player/commands.rs`. Commands
/// that change the queue return its new `rev`. Visualizer and DSP commands
/// are not wrapped (dropped features).
library;

import '../ipc/transport.dart';
import '_json.dart';
import 'player_models.dart';

enum EnqueueAt { next, end }

class PlayerApi {
  final Transport _t;
  const PlayerApi(this._t);

  Future<Object?> _call(String cmd, [Map<String, Object?> args = const {}]) =>
      _t.call(cmd, args);

  Future<int> _rev(String cmd, [Map<String, Object?> args = const {}]) async =>
      asInt(((await _call(cmd, args)) as Json)['rev']) ?? 0;

  /// Typed player events from the transport's event stream.
  Stream<PlayerEvent> get events => _t.events
      .map((e) => PlayerEvent.parse(e.name, e.payload))
      .where((e) => e != null)
      .cast<PlayerEvent>();

  Future<PlayerSnapshot> snapshot() async =>
      PlayerSnapshot.fromJson((await _call('player_snapshot')) as Json);

  Future<void> play() => _call('player_play');
  Future<void> pause() => _call('player_pause');
  Future<void> toggle() => _call('player_toggle');
  Future<void> stop() => _call('player_stop');
  Future<void> next() => _call('player_next');
  Future<void> prev() => _call('player_prev');

  /// [position] in seconds; [resume] starts playback when paused.
  Future<void> seek(double position, {bool resume = false}) =>
      _call('player_seek', {'position': position, 'resume': resume});

  Future<void> playUid(String uid, {double? position}) =>
      _call('player_play_uid', {'uid': uid, 'position': position});

  Future<int> setQueue(List<TrackInput> tracks,
          {int startIndex = 0, bool play = false, double? position}) =>
      _rev('player_set_queue', {
        'tracks': [for (final t in tracks) t.toJson()],
        'startIndex': startIndex,
        'play': play,
        'position': position,
      });

  Future<int> enqueue(List<TrackInput> tracks, EnqueueAt at) => _rev('player_enqueue', {
        'tracks': [for (final t in tracks) t.toJson()],
        'at': at.name,
      });

  Future<int> remove(String uid) => _rev('player_remove', {'uid': uid});

  Future<int> move(String uid, int to) => _rev('player_move', {'uid': uid, 'to': to});

  Future<int> clear({bool keepCurrent = true}) =>
      _rev('player_clear', {'keepCurrent': keepCurrent});

  Future<int> setShuffle(bool on) => _rev('player_set_shuffle', {'on': on});

  Future<int> setRepeat(Repeat mode) => _rev('player_set_repeat', {'mode': mode.name});

  /// Merged one level deep: nested objects (`crossfade`, …) are replaced whole.
  Future<PlayerSettings> setSettings(Map<String, Object?> patch) async => PlayerSettings.fromJson(
      (await _call('player_set_settings', {'patch': patch})) as Json);

  /// [minutes] null cancels. Returns the end time (unix ms) or null.
  Future<double?> setSleepTimer(double? minutes) async => asDouble(
      ((await _call('player_set_sleep_timer', {'minutes': minutes})) as Json)['endsAt']);

  Future<void> setContinuation(ContinuationKind kind, {List<TrackInput> seeds = const []}) =>
      _call('player_set_continuation', {
        'kind': kind.name,
        'seeds': [for (final t in seeds) t.toJson()],
      });

  /// Journal entries after [after] (a `journalSeq`).
  Future<List<JournalEntry>> journal({int after = 0}) async => [
        for (final e in ((await _call('player_journal', {'after': after})) as List? ?? const []))
          ?JournalEntry.fromJson(e as Json),
      ];

  Future<void> journalAck(int upTo) => _call('player_journal_ack', {'upTo': upTo});

  // ---- wave ----------------------------------------------------------------

  Future<int> waveStart({int count = 5}) => _rev('player_wave_start', {'count': count});
  Future<void> waveStop() => _call('player_wave_stop');
  Future<void> waveDislike() => _call('player_wave_dislike');

  /// Drops the upcoming wave entries and fetches fresh ones.
  Future<int> waveRequeue() => _rev('player_wave_requeue');
}
